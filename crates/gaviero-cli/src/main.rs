use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;

use gaviero_core::memory::MemoryStore;
use gaviero_core::observer::{AcpObserver, SwarmObserver};
use gaviero_core::swarm::models::{AgentStatus, SwarmResult, WorkUnit};

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

    /// Auto-accept all changes (no interactive review).
    #[arg(long)]
    auto_accept: bool,

    /// Maximum parallel agents (reserved for M3b, currently sequential).
    #[arg(long, default_value = "1")]
    max_parallel: usize,

    /// Claude model to use.
    #[arg(long, default_value = "sonnet")]
    model: String,

    /// Override the write namespace (default: from settings or folder name).
    #[arg(long)]
    namespace: Option<String>,

    /// Additional namespaces to read from (can be specified multiple times).
    #[arg(long = "read-ns")]
    read_ns: Vec<String>,

    /// Output format: text or json.
    #[arg(long, default_value = "text")]
    format: String,
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
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::WARN)
        .init();

    let cli = Cli::parse();

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
    let work_units = if let Some(ref task) = cli.task {
        vec![WorkUnit {
            id: "task-0".to_string(),
            description: task.clone(),
            scope: gaviero_core::types::FileScope {
                owned_paths: vec![".".to_string()],
                read_only_paths: Vec::new(),
                interface_contracts: std::collections::HashMap::new(),
            },
            depends_on: Vec::new(),
            backend: Default::default(),
            model: Some(cli.model.clone()),
        }]
    } else if let Some(ref json) = cli.work_units {
        serde_json::from_str::<Vec<WorkUnit>>(json)
            .context("parsing --work-units JSON")?
    } else {
        anyhow::bail!("Either --task or --work-units is required");
    };

    // Execute via swarm pipeline
    let swarm_observer = CliSwarmObserver;
    let config = gaviero_core::swarm::pipeline::SwarmConfig {
        max_parallel: cli.max_parallel,
        workspace_root: repo,
        model: cli.model.clone(),
        use_worktrees: cli.max_parallel > 1,
        read_namespaces: read_nss,
        write_namespace: write_ns,
    };

    let result = gaviero_core::swarm::pipeline::execute(
        work_units,
        &config,
        memory,
        &swarm_observer,
        |_agent_id| Box::new(CliAcpObserver) as Box<dyn gaviero_core::observer::AcpObserver>,
    ).await?;

    // Output results
    match cli.format.as_str() {
        "json" => {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        _ => {
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
