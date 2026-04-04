use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;

use gaviero_core::memory::MemoryStore;
use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::swarm::models::{AgentStatus, SwarmResult, WorkUnit};

/// Claude model to use for `--task` and `--work-units` modes.
#[derive(clap::ValueEnum, Clone, Debug, Default)]
enum CliModel {
    #[default]
    Sonnet,
    Opus,
    Haiku,
}

impl CliModel {
    fn as_str(&self) -> &'static str {
        match self {
            CliModel::Sonnet => "sonnet",
            CliModel::Opus => "opus",
            CliModel::Haiku => "haiku",
        }
    }
}

impl std::fmt::Display for CliModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

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

    /// Claude model to use.
    #[arg(long, default_value_t = CliModel::Sonnet)]
    model: CliModel,

    /// Override the write namespace (default: from settings or folder name).
    #[arg(long)]
    namespace: Option<String>,

    /// Additional namespaces to read from (can be specified multiple times).
    #[arg(long = "read-ns")]
    read_ns: Vec<String>,

    /// Output format.
    #[arg(long, default_value_t = OutputFormat::Text)]
    format: OutputFormat,

    /// Use coordinated tier routing (Opus plans, Sonnet/Haiku/local execute).
    /// Requires --task. Opus decomposes the task into a tier-annotated DAG.
    #[arg(long)]
    coordinated: bool,

    /// Resume a previous run that was interrupted. Loads the checkpoint from
    /// `.gaviero/state/<plan-hash>.json` and skips already-completed nodes.
    #[arg(long)]
    resume: bool,

    /// Write structured JSON trace logs to this file (enables DEBUG-level tracing).
    #[arg(long)]
    trace: Option<PathBuf>,
}

/// CLI observer that prints agent events to stderr.
struct CliAcpObserver;

impl AcpObserver for CliAcpObserver {
    fn on_stream_chunk(&self, _text: &str) {
        // Suppress streaming output in CLI mode
    }
    fn on_tool_call_started(&self, tool_name: &str) {
        eprintln!("  [tool] {}", tool_name);
    }
    fn on_message_complete(&self, role: &str, _content: &str) {
        if role == "assistant" {
            eprintln!("  [done]");
        }
    }
    fn on_proposal_deferred(&self, path: &std::path::Path, _old_content: Option<&str>, _new_content: &str) {
        eprintln!("  [deferred] {}", path.display());
    }
    fn on_streaming_status(&self, _status: &str) {
        // CLI doesn't show streaming status
    }
    fn on_validation_result(&self, gate: &str, passed: bool, message: Option<&str>) {
        if passed {
            eprintln!("  [validation] {}: pass", gate);
        } else {
            eprintln!("  [validation] {}: fail — {}", gate, message.unwrap_or(""));
        }
    }
    fn on_validation_retry(&self, attempt: u8, max_retries: u8) {
        eprintln!("  [retry] attempt {}/{} — feeding error to agent", attempt, max_retries);
    }
}

/// CLI observer for swarm events.
struct CliSwarmObserver;

impl SwarmObserver for CliSwarmObserver {
    fn on_phase_changed(&self, phase: &str) {
        eprintln!("[phase] {}", phase);
    }
    fn on_agent_state_changed(&self, work_unit_id: &str, status: &AgentStatus, detail: &str) {
        eprintln!("[agent:{}] {:?} {}", work_unit_id, status, detail);
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
        eprintln!("[coordinator] planning: {}...", &prompt[..prompt.len().min(80)]);
    }
    fn on_coordination_complete(&self, dag: &gaviero_core::swarm::coordinator::TaskDAG) {
        eprintln!("[coordinator] planned {} units: {}", dag.units.len(), dag.plan_summary);
    }
    fn on_tier_dispatch(&self, unit_id: &str, tier: gaviero_core::types::ModelTier, backend: &str) {
        eprintln!("[dispatch] {}  tier={:?}  backend={}", unit_id, tier, backend);
    }
    fn on_cost_update(&self, estimate: &gaviero_core::swarm::verify::CostEstimate) {
        eprintln!("[cost] ~${:.4}", estimate.estimated_usd);
    }
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

    // Load workspace for settings
    let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.clone());
    workspace.ensure_settings();

    // Resolve namespaces: CLI flags override settings, which override folder name
    let write_ns = cli.namespace.clone()
        .unwrap_or_else(|| workspace.resolve_namespace(None));
    let mut read_nss = workspace.resolve_read_namespaces(None);
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

    eprintln!("[namespace] write={}, read=[{}]", write_ns, read_nss.join(", "));

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
    let plan = if let Some(ref script_path) = cli.script {
        let source = std::fs::read_to_string(script_path)
            .with_context(|| format!("reading script: {}", script_path.display()))?;
        let filename = script_path.display().to_string();
        gaviero_dsl::compile(&source, &filename, None, None)
            .map_err(|report| {
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
            model: Some(cli.model.as_str().to_string()),
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
        }];
        gaviero_core::swarm::plan::CompiledPlan::from_work_units(units, None)
    } else if let Some(ref json) = cli.work_units {
        let units = serde_json::from_str::<Vec<WorkUnit>>(json)
            .context("parsing --work-units JSON")?;
        gaviero_core::swarm::plan::CompiledPlan::from_work_units(units, None)
    } else {
        anyhow::bail!("Either --task, --work-units, or --script is required");
    };

    // Execute via swarm pipeline
    // plan.max_parallel overrides the CLI flag when declared.
    let swarm_observer = CliSwarmObserver;
    let config = gaviero_core::swarm::pipeline::SwarmConfig {
        max_parallel: cli.max_parallel,
        workspace_root: repo,
        model: cli.model.as_str().to_string(),
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
        let task = cli.task.as_deref()
            .ok_or_else(|| anyhow::anyhow!("--coordinated requires --task"))?;
        let coord_config = gaviero_core::swarm::coordinator::CoordinatorConfig {
            model: "opus".into(),
            ..Default::default()
        };
        eprintln!("[mode] coordinated — planning DSL (Opus)");
        let dsl_text = gaviero_core::swarm::pipeline::plan_coordinated(
            task,
            &config,
            coord_config,
            memory,
            &swarm_observer,
            |_agent_id| Box::new(CliAcpObserver) as Box<dyn gaviero_core::observer::AcpObserver>,
        ).await?;

        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let plan_path = config.workspace_root.join("tmp").join(format!("gaviero_plan_{}.gaviero", timestamp));
        std::fs::create_dir_all(plan_path.parent().unwrap())
            .context("creating tmp/ directory")?;
        std::fs::write(&plan_path, &dsl_text)
            .context("writing plan file")?;

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
                let completed = state.node_states.values()
                    .filter(|s| s.status == gaviero_core::swarm::execution_state::NodeStatus::Completed)
                    .count();
                eprintln!("[resume] loaded checkpoint: {}/{} nodes already completed",
                    completed, state.node_states.len());
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
        |_agent_id| Box::new(CliAcpObserver) as Box<dyn gaviero_core::observer::AcpObserver>,
    ).await?;

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
                println!("{}: {} ({})", m.work_unit_id, status_str,
                    m.modified_files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "));
            }
        }
    }

    if result.success {
        Ok(())
    } else {
        std::process::exit(1);
    }
}
