use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;

use gaviero_core::memory::MemoryStore;
use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::swarm::models::{AgentStatus, SwarmResult, WorkUnit};

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Text => f.write_str("text"),
            OutputFormat::Json => f.write_str("json"),
        }
    }
}

/// Output format for results.
#[derive(clap::ValueEnum, Clone, Debug, Default)]
enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Parser)]
#[command(name = "gaviero-cli", about = "Headless AI agent task runner")]
struct Cli {
    /// Path to the repository / workspace root.
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Single task description (creates one WorkUnit with full repo scope).
    #[arg(long, conflicts_with = "work_units")]
    task: Option<String>,

    /// JSON array of WorkUnit definitions.
    #[arg(long, conflicts_with = "task")]
    work_units: Option<String>,

    /// Path to a .gaviero DSL script file.
    #[arg(long, conflicts_with_all = ["task", "work_units"])]
    script: Option<PathBuf>,

    /// Auto-accept all changes (no interactive review).
    #[arg(long)]
    auto_accept: bool,

    /// Maximum parallel agents (reserved for M3b, currently sequential).
    #[arg(long, default_value = "1")]
    max_parallel: usize,

    /// Model spec to use for synthetic task execution and as the default runtime model.
    /// Examples: sonnet, opus, haiku, claude:sonnet, codex:gpt-5-codex, ollama:qwen2.5-coder:7b.
    /// Defaults to workspace agent.model, then sonnet.
    #[arg(long)]
    model: Option<String>,

    /// Model spec to use for coordinated planning.
    /// Defaults to --model, then workspace agent.coordinator.model, then sonnet.
    #[arg(long)]
    coordinator_model: Option<String>,

    /// Override the Ollama base URL.
    /// Defaults to workspace agent.ollamaBaseUrl, then http://localhost:11434.
    #[arg(long)]
    ollama_base_url: Option<String>,

    /// Override the write namespace (default: from settings or folder name).
    #[arg(long)]
    namespace: Option<String>,

    /// Additional namespaces to read from (can be specified multiple times).
    #[arg(long = "read-ns")]
    read_ns: Vec<String>,

    /// Output format.
    #[arg(long, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    /// Use coordinated planning: emits a .gaviero plan file for review and exits.
    /// Requires --task. Produces a .gaviero DSL file for review before execution.
    #[arg(long)]
    coordinated: bool,

    /// Resume a previous run that was interrupted. Loads the checkpoint from
    /// `.gaviero/state/<plan-hash>.json` and skips already-completed nodes.
    #[arg(long)]
    resume: bool,

    /// Maximum inner-loop retries per attempt (iteration mode).
    #[arg(long, default_value = "5")]
    max_retries: u32,

    /// Number of independent attempts for BestOfN strategy.
    #[arg(long, default_value = "1")]
    attempts: u32,

    /// Generate failing tests before the edit loop (TDD red phase).
    #[arg(long)]
    test_first: bool,

    /// Disable iteration — single pass only (overrides --max-retries).
    #[arg(long)]
    no_iterate: bool,

    /// Write structured JSON trace logs to this file (enables DEBUG-level tracing).
    #[arg(long)]
    trace: Option<PathBuf>,

    /// Output path for the generated .gaviero DSL plan file (--coordinated only).
    /// Defaults to tmp/gaviero_plan_<timestamp>.gaviero inside the repo.
    #[arg(long, requires = "coordinated")]
    output: Option<PathBuf>,

    /// Build or update the code knowledge graph, print stats, and exit.
    /// Does not run any agents. Useful after major codebase changes.
    #[arg(long)]
    graph: bool,
}

/// CLI observer that prints agent events to stderr, mirroring agent chat output.
struct CliAcpObserver;

impl CliAcpObserver {
    fn new() -> Self {
        Self
    }
}

impl AcpObserver for CliAcpObserver {
    fn on_stream_chunk(&self, text: &str) {
        eprint!("{}", text);
        let _ = std::io::stderr().flush();
    }

    fn on_tool_call_started(&self, tool_name: &str) {
        eprintln!("\n  ⚙ {}", tool_name);
    }

    fn on_message_complete(&self, role: &str, _content: &str) {
        if role == "assistant" {
            eprintln!(); // newline after streamed text
        }
    }

    fn on_proposal_deferred(
        &self,
        path: &std::path::Path,
        _old_content: Option<&str>,
        _new_content: &str,
    ) {
        eprintln!("  ✎ {}", path.display());
    }

    fn on_streaming_status(&self, status: &str) {
        eprintln!("  … {}", status);
    }

    fn on_validation_result(&self, gate: &str, passed: bool, message: Option<&str>) {
        if passed {
            eprintln!("  ✓ {}", gate);
        } else {
            eprintln!("  ✗ {} — {}", gate, message.unwrap_or(""));
        }
    }

    fn on_validation_retry(&self, attempt: u8, max_retries: u8) {
        eprintln!("  ↺ retry {}/{}", attempt, max_retries);
    }
}

/// CLI observer for swarm events.
struct CliSwarmObserver;

impl SwarmObserver for CliSwarmObserver {
    fn on_phase_changed(&self, phase: &str) {
        eprintln!("[phase] {}", phase);
    }
    fn on_agent_state_changed(&self, work_unit_id: &str, status: &AgentStatus, detail: &str) {
        match status {
            AgentStatus::Running => {
                eprintln!("\n── agent: {} ─────────────────────────────", work_unit_id)
            }
            AgentStatus::Completed => {
                eprintln!("── done: {} ──────────────────────────────", work_unit_id)
            }
            AgentStatus::Failed(_) => eprintln!("── failed: {} {}", work_unit_id, detail),
            _ => eprintln!("[agent:{}] {:?} {}", work_unit_id, status, detail),
        }
    }
    fn on_tier_started(&self, current: usize, total: usize) {
        eprintln!("[tier] {}/{}", current, total);
    }
    fn on_merge_conflict(&self, branch: &str, files: &[String]) {
        eprintln!("[conflict] branch={} files={}", branch, files.join(", "));
    }
    fn on_completed(&self, result: &SwarmResult) {
        eprintln!("[completed] success={}", result.success);
    }
    fn on_coordination_started(&self, prompt: &str) {
        eprintln!(
            "[coordinator] planning: {}...",
            &prompt[..prompt.len().min(80)]
        );
    }
    fn on_coordination_complete(&self, dag: &gaviero_core::swarm::coordinator::TaskDAG) {
        eprintln!(
            "[coordinator] planned {} units: {}",
            dag.units.len(),
            dag.plan_summary
        );
    }
    fn on_tier_dispatch(&self, unit_id: &str, tier: gaviero_core::types::ModelTier, backend: &str) {
        eprintln!(
            "[dispatch] {}  tier={:?}  backend={}",
            unit_id, tier, backend
        );
    }
    fn on_cost_update(&self, estimate: &gaviero_core::swarm::verify::CostEstimate) {
        eprintln!("[cost] ~${:.4}", estimate.estimated_usd);
    }
}

fn resolve_model_spec(spec: &str, label: &str) -> Result<String> {
    let trimmed = spec.trim();
    gaviero_core::swarm::backend::shared::validate_model_spec(trimmed)
        .with_context(|| format!("invalid {} model spec '{}'", label, trimmed))?;
    Ok(trimmed.to_string())
}

fn workspace_setting_string(
    workspace: &gaviero_core::workspace::Workspace,
    key: &str,
) -> Option<String> {
    workspace
        .resolve_setting(key, None)
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn resolve_execution_model(
    cli: &Cli,
    workspace: &gaviero_core::workspace::Workspace,
) -> Result<String> {
    let candidate = cli
        .model
        .clone()
        .or_else(|| {
            workspace_setting_string(workspace, gaviero_core::workspace::settings::AGENT_MODEL)
        })
        .unwrap_or_else(|| "sonnet".to_string());
    resolve_model_spec(&candidate, "execution")
}

fn resolve_coordinator_model(
    cli: &Cli,
    workspace: &gaviero_core::workspace::Workspace,
    execution_model: &str,
) -> Result<String> {
    let candidate = cli
        .coordinator_model
        .clone()
        .or_else(|| {
            workspace_setting_string(
                workspace,
                gaviero_core::workspace::settings::COORDINATOR_MODEL,
            )
        })
        .unwrap_or_else(|| execution_model.to_string());
    resolve_model_spec(&candidate, "coordinator")
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing: JSON to file when --trace is set, human-readable to stderr otherwise
    if let Some(ref trace_path) = cli.trace {
        let file = std::fs::File::create(trace_path)
            .with_context(|| format!("creating trace file: {}", trace_path.display()))?;
        tracing_subscriber::fmt()
            .json()
            .with_writer(file)
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(tracing::Level::WARN)
            .init();
    }

    let repo = std::fs::canonicalize(&cli.repo)
        .with_context(|| format!("resolving repo path: {}", cli.repo.display()))?;

    // ── --graph: build/update code knowledge graph and exit ──────
    if cli.graph {
        eprintln!(
            "[graph] building code knowledge graph for {}...",
            repo.display()
        );
        let (store, result) = gaviero_core::repo_map::graph_builder::build_graph(&repo)
            .context("building code knowledge graph")?;
        let (nodes, edges) = store.stats()?;
        eprintln!("[graph] done");
        eprintln!("  files scanned:   {}", result.files_scanned);
        eprintln!("  files changed:   {}", result.files_changed);
        eprintln!("  files unchanged: {}", result.files_unchanged);
        eprintln!("  files removed:   {}", result.files_removed);
        eprintln!("  total nodes:     {}", nodes);
        eprintln!("  total edges:     {}", edges);
        return Ok(());
    }

    // Load workspace for settings
    let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.clone());
    workspace.ensure_settings();

    // Resolve namespaces: CLI flags override settings, which override folder name
    let write_ns = cli
        .namespace
        .clone()
        .unwrap_or_else(|| workspace.resolve_namespace(None));
    let mut read_nss = workspace.resolve_read_namespaces(None);
    let execution_model = resolve_execution_model(&cli, &workspace)?;
    let coordinator_model = if cli.coordinated {
        Some(resolve_coordinator_model(
            &cli,
            &workspace,
            &execution_model,
        )?)
    } else {
        None
    };
    let ollama_base_url = cli.ollama_base_url.clone().or_else(|| {
        workspace_setting_string(
            &workspace,
            gaviero_core::workspace::settings::AGENT_OLLAMA_BASE_URL,
        )
    });
    // Merge CLI --read-ns flags
    for ns in &cli.read_ns {
        if !read_nss.contains(ns) {
            read_nss.push(ns.clone());
        }
    }
    // Ensure write namespace is in the read list
    if !read_nss.contains(&write_ns) {
        read_nss.insert(0, write_ns.clone());
    }

    eprintln!(
        "[namespace] write={}, read=[{}]",
        write_ns,
        read_nss.join(", ")
    );
    if let Some(ref coord_model) = coordinator_model {
        eprintln!(
            "[model] execution={}, coordinator={}",
            execution_model, coord_model
        );
    } else {
        eprintln!("[model] execution={}", execution_model);
    }

    // Initialize memory store (graceful if it fails — offline, corrupt model, etc.)
    let memory: Option<Arc<MemoryStore>> =
        match tokio::task::spawn_blocking(|| gaviero_core::memory::init(None)).await {
            Ok(Ok(store)) => {
                eprintln!("[memory] ready");
                Some(store)
            }
            Ok(Err(e)) => {
                eprintln!("[memory] disabled: {}", e);
                None
            }
            Err(e) => {
                eprintln!("[memory] init panicked: {}", e);
                None
            }
        };

    // Parse work units
    let mut plan = if let Some(ref script_path) = cli.script {
        let source = std::fs::read_to_string(script_path)
            .with_context(|| format!("reading script: {}", script_path.display()))?;
        let filename = script_path.display().to_string();
        gaviero_dsl::compile(&source, &filename, None, None).map_err(|report| {
            eprintln!("{:?}", report);
            anyhow::anyhow!("DSL compilation failed")
        })?
    } else if let Some(ref task) = cli.task {
        let units = vec![WorkUnit {
            id: "task-0".to_string(),
            description: task.clone(),
            scope: gaviero_core::types::FileScope {
                owned_paths: vec![".".to_string()],
                read_only_paths: Vec::new(),
                interface_contracts: std::collections::HashMap::new(),
            },
            depends_on: Vec::new(),
            #[allow(deprecated)]
            backend: Default::default(),
            model: Some(execution_model.clone()),
            effort: None,
            extra: Vec::new(),
            tier: Default::default(),
            privacy: Default::default(),
            coordinator_instructions: String::new(),
            estimated_tokens: 0,
            max_retries: 1,
            escalation_tier: None,
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: Vec::new(),
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
        }];
        gaviero_core::swarm::plan::CompiledPlan::from_work_units(units, None)
    } else if let Some(ref json) = cli.work_units {
        let units =
            serde_json::from_str::<Vec<WorkUnit>>(json).context("parsing --work-units JSON")?;
        gaviero_core::swarm::plan::CompiledPlan::from_work_units(units, None)
    } else {
        anyhow::bail!("Either --task, --work-units, or --script is required");
    };

    // Apply iteration CLI flags (override DSL / defaults).
    {
        use gaviero_core::iteration::Strategy;
        if cli.no_iterate {
            plan.iteration_config.strategy = Strategy::SinglePass;
        } else if cli.attempts > 1 {
            plan.iteration_config.strategy = Strategy::BestOfN { n: cli.attempts };
        }
        plan.iteration_config.max_retries = cli.max_retries;
        plan.iteration_config.test_first = cli.test_first;
    }

    // Execute via swarm pipeline
    // plan.max_parallel overrides the CLI flag when declared.
    let swarm_observer = CliSwarmObserver;
    let config = gaviero_core::swarm::pipeline::SwarmConfig {
        max_parallel: cli.max_parallel,
        workspace_root: repo,
        model: execution_model.clone(),
        ollama_base_url: ollama_base_url.clone(),
        use_worktrees: cli.max_parallel > 1,
        read_namespaces: read_nss,
        write_namespace: write_ns,
        context_files: vec![],
    };

    // --coordinated: produce a DSL plan file for review, then exit.
    // The user reviews the file and runs it with: gaviero --script <path>
    if cli.coordinated {
        if cli.script.is_some() {
            anyhow::bail!("--coordinated requires --task, not --script");
        }
        let task = cli
            .task
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("--coordinated requires --task"))?;
        let coord_config = gaviero_core::swarm::coordinator::CoordinatorConfig {
            model: coordinator_model
                .clone()
                .unwrap_or_else(|| execution_model.clone()),
            ollama_base_url: ollama_base_url,
            ..Default::default()
        };
        eprintln!("[mode] coordinated — planning DSL ({})", coord_config.model);
        let dsl_text = gaviero_core::swarm::pipeline::plan_coordinated(
            task,
            &config,
            coord_config,
            memory,
            &swarm_observer,
            |_| Box::new(CliAcpObserver::new()) as Box<dyn gaviero_core::observer::AcpObserver>,
        )
        .await?;

        let plan_path = if let Some(ref out) = cli.output {
            if out.is_absolute() {
                out.clone()
            } else {
                config.workspace_root.join(out)
            }
        } else {
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            config
                .workspace_root
                .join("tmp")
                .join(format!("gaviero_plan_{}.gaviero", timestamp))
        };
        if let Some(parent) = plan_path.parent() {
            std::fs::create_dir_all(parent).context("creating output directory")?;
        }
        std::fs::write(&plan_path, &dsl_text).context("writing plan file")?;

        // Validate immediately so the user gets early feedback on syntax errors.
        let plan_filename = plan_path.display().to_string();
        match gaviero_dsl::compile(&dsl_text, &plan_filename, None, None) {
            Ok(compiled) => {
                let agent_count = compiled.graph.node_count();
                let units = compiled.work_units_ordered().unwrap_or_default();
                let tier_count = gaviero_core::swarm::validation::dependency_tiers(&units)
                    .map(|t| t.len())
                    .unwrap_or(1);
                eprintln!(
                    "[plan] valid — {} agents, {} tiers",
                    agent_count, tier_count
                );
            }
            Err(report) => {
                eprintln!("{:?}", report);
                eprintln!("[plan] DSL has errors — edit before running with --script");
            }
        }

        eprintln!("[plan] saved to {}", plan_path.display());
        eprintln!("[plan] review it, then run with:");
        eprintln!("         gaviero --script {}", plan_path.display());
        println!("{}", plan_path.display());
        return Ok(());
    }

    // Load checkpoint for --resume
    let initial_state = if cli.resume {
        let hash = plan.hash();
        match gaviero_core::swarm::execution_state::ExecutionState::load(&hash) {
            Ok(Some(state)) => {
                let completed = state
                    .node_states
                    .values()
                    .filter(|s| {
                        s.status == gaviero_core::swarm::execution_state::NodeStatus::Completed
                    })
                    .count();
                eprintln!(
                    "[resume] loaded checkpoint: {}/{} nodes already completed",
                    completed,
                    state.node_states.len()
                );
                Some(state)
            }
            Ok(None) => {
                eprintln!("[resume] no checkpoint found for this plan (starting fresh)");
                None
            }
            Err(e) => {
                eprintln!("[resume] failed to load checkpoint: {} (starting fresh)", e);
                None
            }
        }
    } else {
        None
    };

    let result = gaviero_core::swarm::pipeline::execute(
        &plan,
        &config,
        initial_state,
        memory,
        &swarm_observer,
        |_| Box::new(CliAcpObserver::new()) as Box<dyn gaviero_core::observer::AcpObserver>,
    )
    .await?;

    // Output results
    match cli.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        OutputFormat::Text => {
            for m in &result.manifests {
                let status_str = match &m.status {
                    AgentStatus::Completed => "OK".to_string(),
                    AgentStatus::Failed(e) => format!("FAIL: {}", e),
                    other => format!("{:?}", other),
                };
                println!(
                    "{}: {} ({})",
                    m.work_unit_id,
                    status_str,
                    m.modified_files
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }

    if result.success {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_workspace(name: &str, settings_json: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("gaviero-cli-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join(".gaviero")).unwrap();
        fs::write(base.join(".gaviero/settings.json"), settings_json).unwrap();
        base
    }

    #[test]
    fn cli_accepts_provider_aware_model_flags() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--model",
            "ollama:qwen2.5-coder:7b",
            "--coordinator-model",
            "claude:sonnet",
            "--ollama-base-url",
            "http://localhost:11434",
        ])
        .unwrap();

        assert_eq!(cli.model.as_deref(), Some("ollama:qwen2.5-coder:7b"));
        assert_eq!(cli.coordinator_model.as_deref(), Some("claude:sonnet"));
        assert_eq!(
            cli.ollama_base_url.as_deref(),
            Some("http://localhost:11434")
        );
    }

    #[test]
    fn resolve_model_spec_rejects_unknown_provider_prefix() {
        let err = resolve_model_spec("openai:gpt-4.1", "execution").unwrap_err();
        assert!(err.to_string().contains("invalid execution model spec"));
    }

    #[test]
    fn resolve_execution_model_prefers_cli_over_workspace() {
        let root = temp_workspace(
            "execution-model",
            r#"{
              "agent": {
                "model": "opus"
              }
            }"#,
        );
        let workspace = gaviero_core::workspace::Workspace::single_folder(root.clone());
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--model",
            "ollama:qwen2.5-coder:7b",
        ])
        .unwrap();

        let resolved = resolve_execution_model(&cli, &workspace).unwrap();
        assert_eq!(resolved, "ollama:qwen2.5-coder:7b");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn resolve_coordinator_model_uses_workspace_before_execution_fallback() {
        let root = temp_workspace(
            "coordinator-model",
            r#"{
              "agent": {
                "model": "haiku",
                "coordinator": {
                  "model": "claude:sonnet"
                }
              }
            }"#,
        );
        let workspace = gaviero_core::workspace::Workspace::single_folder(root.clone());
        let cli =
            Cli::try_parse_from(["gaviero-cli", "--task", "plan it", "--coordinated"]).unwrap();

        let execution = resolve_execution_model(&cli, &workspace).unwrap();
        let coordinator = resolve_coordinator_model(&cli, &workspace, &execution).unwrap();

        assert_eq!(execution, "haiku");
        assert_eq!(coordinator, "claude:sonnet");

        let _ = fs::remove_dir_all(root);
    }
}
