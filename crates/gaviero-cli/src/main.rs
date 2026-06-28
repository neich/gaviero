use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use clap::Parser;

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

#[derive(Parser, Debug)]
#[command(name = "gaviero-cli", about = "Headless AI agent task runner")]
struct Cli {
    /// Path to the git repository root (`execution repo` workflows).
    #[arg(long, default_value = ".", conflicts_with = "workspace")]
    repo: PathBuf,

    /// Data directory for `execution document` workflows (no git lifecycle).
    /// Defaults to the plan file's directory when `--var PLAN_FILE=...` is set.
    #[arg(long, conflicts_with = "repo")]
    workspace: Option<PathBuf>,

    /// Single task description (creates one WorkUnit with full repo scope).
    #[arg(long, conflicts_with = "work_units")]
    task: Option<String>,

    /// JSON array of WorkUnit definitions.
    #[arg(long, conflicts_with = "task")]
    work_units: Option<String>,

    /// Path to a .gaviero DSL script file.
    #[arg(long, conflicts_with_all = ["task", "work_units"])]
    script: Option<PathBuf>,

    /// Inline value to substitute for every `{{PROMPT}}` placeholder in the
    /// DSL script. Also becomes the full prompt for any agent without a
    /// `prompt` field. Mutually exclusive with `--prompt-file` (use that
    /// when the text is long enough to warrant its own file).
    /// Only valid with `--script`.
    #[arg(long, requires = "script", conflicts_with_all = ["task", "work_units", "prompt_file"])]
    prompt: Option<String>,

    /// Path to a file whose contents replace every `{{PROMPT}}` placeholder in
    /// the DSL script. Also becomes the full prompt for any agent without a
    /// `prompt` field. Only valid with `--script`.
    #[arg(long, requires = "script", conflicts_with_all = ["task", "work_units"])]
    prompt_file: Option<PathBuf>,

    /// Pick a workflow `<name>` declared in the DSL script. Required when
    /// the script defines more than one workflow; for a single-workflow
    /// script (or no workflows at all) the compiler auto-selects.
    /// Only valid with `--script`.
    #[arg(long, requires = "script", conflicts_with_all = ["task", "work_units"])]
    workflow: Option<String>,

    /// Auto-accept all changes (no interactive review).
    #[arg(long)]
    auto_accept: bool,

    /// Maximum parallel agents (reserved for M3b, currently sequential).
    #[arg(long, default_value = "1")]
    max_parallel: usize,

    /// Model spec to use for synthetic task execution and as the default runtime model.
    /// Canonical form is `provider:model` — e.g. `claude:sonnet`, `claude:opus`,
    /// `codex:gpt-5.5`, `ollama:qwen2.5-coder:7b`. Defaults to workspace agent.model,
    /// then `claude:sonnet`.
    #[arg(long)]
    model: Option<String>,

    /// Model spec to use for coordinated planning.
    /// Defaults to --model, then workspace agent.coordinator.model, then `claude:sonnet`.
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

    /// Enable INFO-level logging to stderr. Use twice (-vv) for DEBUG.
    #[arg(long = "verbose", short = 'v', action = clap::ArgAction::Count)]
    verbose: u8,

    /// Output path for the generated .gaviero DSL plan file (--coordinated only).
    /// Defaults to tmp/gaviero_plan_<timestamp>.gaviero inside the repo.
    #[arg(long, requires = "coordinated")]
    output: Option<PathBuf>,

    /// Build or update the code knowledge graph, print stats, and exit.
    /// Pass `--enrich` to run rustdoc symbol enrichment (S2.1) after the
    /// graph scan. Requires a successful `cargo build` and a nightly
    /// toolchain for `cargo rustdoc --output-format json` (`rustup toolchain install nightly`).
    #[arg(long)]
    graph: bool,

    /// With `--graph`, run rustdoc JSON enrichment into `symbol_docs`
    /// (S2.1 / S2.2). Never runs at workspace-open — explicit only.
    #[arg(long, requires = "graph")]
    enrich: bool,

    /// With `--graph --enrich`, skip embedding vectors (signatures/docs only).
    #[arg(long, requires = "enrich")]
    enrich_no_embed: bool,

    /// Delete local branches matching `gaviero/*` (left over from prior
    /// swarm runs — agent worktrees and stacked-loop iteration branches).
    /// Dry-run by default: prints the matching branches and exits.
    /// Pass `--force` to actually delete. The currently checked-out branch
    /// is always skipped, and `git worktree prune` is run first.
    #[arg(long = "cleanup-branches")]
    cleanup_branches: bool,

    /// With `--cleanup-branches`, actually delete the matched branches
    /// (without this flag the command only previews).
    #[arg(long, requires = "cleanup_branches")]
    force: bool,

    /// Folder name or glob pattern to exclude from repo-map scanning.
    /// Can be specified multiple times and/or as a comma-separated list
    /// (e.g. `--exclude node_modules,docs/**`). A bare name like `node_modules`
    /// matches any directory with that basename at any depth; patterns
    /// containing `/` are glob-matched against paths relative to `--repo`
    /// (e.g. `docs/**`).
    #[arg(long = "exclude", value_delimiter = ',')]
    exclude: Vec<String>,

    /// Override a `vars {}` entry in a DSL script. Format: KEY=VALUE.
    /// Can be specified multiple times (e.g. `--var LOG_DIR=out --var FOO=bar`).
    /// CLI overrides beat script-level vars but not agent-level vars.
    /// Only valid with `--script`.
    #[arg(long = "var", requires = "script")]
    vars: Vec<String>,

    /// Supply a workflow `param <name>` declared in a DSL script. Format:
    /// `NAME=VALUE`. For roster params, the value is a comma-separated list
    /// of `id=provider:model[@effort]` entries
    /// (e.g. `--param roster=claude=claude:opus@max,codex=codex:gpt-5.5@high`).
    /// Required params without a default fail compilation when absent.
    /// Can be specified multiple times. Only valid with `--script`.
    #[arg(long = "param", requires = "script")]
    params: Vec<String>,

    /// Tier profile for a DSL script: a `.gaviero` file containing only
    /// `tier <role> <client>` bindings. Overrides tier lines from the script
    /// and its `include`s (e.g. `profiles/doc-default.gaviero`). Agents must
    /// `include "clients.gaviero"` (or equivalent) so named clients exist.
    /// Only valid with `--script`.
    #[arg(long = "tiers-file", requires = "script")]
    tiers_file: Option<PathBuf>,

    /// Print the N most recent retrieval manifests (Tier S / S4) and exit.
    /// Useful for auditing what memory was injected into recent chat turns.
    #[arg(long = "manifest-last")]
    manifest_last: Option<usize>,

    /// Print the manifest(s) for a specific turn id and exit.
    /// Pairs with the `turn_id` logged by the chat path on injection.
    #[arg(long = "manifest-turn")]
    manifest_turn: Option<String>,

    /// Run the Tier 1 retrieval smoke test against the workspace
    /// `memory.db` using the given JSONL fixture and exit. Prints
    /// recall@1/5/10 and MRR. Exit code is non-zero when recall@5
    /// drops more than `--eval-tolerance` against the baseline (if a
    /// baseline file exists at `<fixture>.baseline.json`).
    #[arg(long = "eval-fixture")]
    eval_fixture: Option<PathBuf>,

    /// Recall@5 regression tolerance for `--eval-fixture` (default 0.02).
    #[arg(long = "eval-tolerance", default_value = "0.02")]
    eval_tolerance: f32,

    /// Write the fresh report to this path. With no path supplied,
    /// writes to `<fixture>.last.json`. Has no effect without
    /// `--eval-fixture`.
    #[arg(long = "eval-report-out")]
    eval_report_out: Option<PathBuf>,

    /// Update the baseline at `<fixture>.baseline.json` to the result
    /// of this run. Use after intentional improvements to lock the
    /// regression gate at the new value.
    #[arg(long = "eval-update-baseline")]
    eval_update_baseline: bool,

    /// Tier B / T0: when set, a missing baseline is **not** an error —
    /// the run prints metrics and exits 0. Off by default so CI fails
    /// loudly the first time a fixture lands without a baseline; turn
    /// it on for ad-hoc local runs against a fresh fixture.
    #[arg(long = "eval-allow-missing-baseline")]
    eval_allow_missing_baseline: bool,

    /// Tier B / B2f ablation: run the fixture twice — once with the
    /// reranker enabled, once with it disabled — and print recall@K /
    /// MRR deltas. Uses the workspace's configured rerank model
    /// (or `gte-reranker-modernbert-base` if none) and the workspace's
    /// `RerankConfig` settings. Mutually exclusive with
    /// `--eval-update-baseline` (ablation never updates the baseline;
    /// run the off-mode separately if you want that).
    #[arg(long = "eval-rerank-ablation", conflicts_with = "eval_update_baseline")]
    eval_rerank_ablation: bool,

    /// KB-efficiency S1.1 / B1g: embedder ablation. Backs up
    /// `.gaviero/memory.db`, then for each of `nomic` and
    /// `gte-modernbert` wipes the workspace DB, seeds the fixture's
    /// `gold_must` File paths, and scores at `run` scope. Restores the
    /// backup afterward. Report-only — does not flip defaults.
    #[arg(long = "eval-embedder-ablation", conflicts_with = "eval_update_baseline")]
    eval_embedder_ablation: bool,

    /// Tier B / T0 rescore mode: replay the fixture against the most
    /// recent N persisted `injection_manifests`. No embedder, no
    /// reranker, no LLM — cheap regression replay for scoring-formula
    /// changes (B4, B3 scope multipliers). Cases whose query never
    /// appeared in a manifest are counted as misses (`absent`).
    #[arg(long = "eval-from-manifests")]
    eval_from_manifests: Option<usize>,

    /// Tier B / T0 bootstrap: read the most recent N injection
    /// manifests from the workspace memory.db and emit a JSONL fixture
    /// (one EvalCase per turn that selected a memory). Hand-prune /
    /// re-tag before checking it in. Combine with `--eval-fixture
    /// <path>` to set the output file.
    #[arg(long = "eval-bootstrap-from-manifests")]
    eval_bootstrap_from_manifests: Option<usize>,

    /// Tier T1 / T1.3 scope-matrix runner: re-run `--eval-fixture`
    /// against multiple scope hints (default: repo, module, run) and
    /// print Recall@K / Precision@K / blast_leakage per scope. Answers
    /// "does narrower scope improve Precision@K?". Each scope swaps
    /// into every case's `scope` before retrieval; gold-set fields on
    /// the case populate the new T1.3 metrics (Precision@5/10,
    /// NDCG@5/10, over/under retrieval, forbid hit rate, blast
    /// leakage). Cases without gold sets contribute 0 to those.
    #[arg(long = "eval-scope-matrix")]
    eval_scope_matrix: bool,

    /// Override the scope chain probed by `--eval-scope-matrix`.
    /// Comma-separated; defaults to `repo,module,run`.
    #[arg(long = "eval-scope-matrix-scopes", default_value = "repo,module,run")]
    eval_scope_matrix_scopes: String,

    /// KB-efficiency S1.3: sweep `memory.chatInjection.maxItems` {3,5,8}
    /// and `agent.graphBudgetTokens` {4k,8k,12k}; print token/quality
    /// tables and write JSON to `--eval-report-out` (default:
    /// `<fixture>.s13-sweep.json`).
    #[arg(long = "eval-budget-sweep", requires = "eval_fixture")]
    eval_budget_sweep: bool,

    /// Tier T1 / T2 corpus seeding: walk every `gold_must` File entry
    /// in the supplied `--eval-fixture` and write one Record memory
    /// per file to the workspace store. Each memory's content is the
    /// repo-relative path plus the file's leading rustdoc and the
    /// names of its top-level `pub` symbols, so substring-matching
    /// gold refs (File / Symbol) actually surface. Repeatable: a
    /// second run dedupes against existing rows. Use to make
    /// --eval-scope-matrix produce non-zero Recall@K against a fresh
    /// workspace memory.db without weeks of organic usage.
    #[arg(long = "seed-corpus-from-paths")]
    seed_corpus_from_paths: bool,

    /// Maximum chars taken from each file's leading rustdoc when
    /// seeding via `--seed-corpus-from-paths`. Default 480.
    #[arg(long = "seed-corpus-doc-chars", default_value = "480")]
    seed_corpus_doc_chars: usize,

    /// Tier B / B5: run the sleeptime hygiene pass against the
    /// workspace `memory.db` and exit. Combine with `--sleep-dry-run`
    /// to see what *would* happen without writing.
    #[arg(long = "sleep")]
    sleep: bool,

    /// Tier B / B5: dry-run flag for `--sleep`. No destructive writes;
    /// audit rows still land with `dry_run = 1` for review.
    #[arg(long = "sleep-dry-run")]
    sleep_dry_run: bool,

    /// Tier B / B6: print top-N most / least utilised memories at the
    /// given scope level (0=Global, 1=Workspace, 2=Repo, 3=Module,
    /// 4=Run) and exit.
    #[arg(long = "utilization-scope")]
    utilization_scope: Option<i32>,

    #[arg(long = "utilization-top", default_value = "20")]
    utilization_top: usize,

    #[arg(long = "utilization-asc")]
    utilization_asc: bool,

    /// Tier C / C1: accept the typed-stores schema migration on first
    /// post-upgrade run. Headless contexts cannot prompt the user
    /// interactively, so an explicit opt-in is required when any
    /// reachable `memory.db` is at a pre-v10 schema. Without this flag
    /// the run aborts and prints the affected files plus the proposed
    /// backup path. The TUI (`gaviero`) prompts on stdin instead.
    #[arg(long = "accept-c1-migration")]
    accept_c1_migration: bool,

    /// Tier C / C2.2: list the most recent N audit-table deletions and
    /// exit. Useful to find the audit id to feed `--restore-id`.
    #[arg(long = "deletions-last")]
    deletions_last: Option<usize>,

    /// Tier C / C2.2: restore a single soft-deleted memory by audit id
    /// and exit. Replays the captured row through the dedup pipeline.
    /// Refused for `user_redaction` rows (one-way per the plan).
    #[arg(long = "restore-id")]
    restore_id: Option<i64>,

    /// Tier C / C2.2: restore every still-pending deletion newer than
    /// the given duration (e.g. `2 hours`, `7 days`, `30 minutes`) and
    /// exit. `user_redaction` rows are skipped silently.
    #[arg(long = "restore-since")]
    restore_since: Option<String>,

    /// Tier C / C2.3: bulk-forget filter — fuzzy match against memory
    /// content. Records and Summaries only; History is never matched.
    #[arg(long = "forget-query")]
    forget_query: Option<String>,

    /// Tier C / C2.3: bulk-forget every memory at the given canonical
    /// scope path (`global`, `workspace`, `repo:<id>`,
    /// `repo:<id>/module:<path>`, `repo:<id>/run:<id>`).
    #[arg(long = "forget-scope")]
    forget_scope: Option<String>,

    /// Tier C / C2.3: bulk-forget every memory of the given type
    /// (factual|procedural|decision|pattern|gotcha|convention|invariant
    /// |preference|lesson|error).
    #[arg(long = "forget-type")]
    forget_type: Option<String>,

    /// Tier C / C2.3: bulk-forget every memory whose write source
    /// matches (e.g. `llm_extracted` for a factory-reset of LLM
    /// extractions; `user_remember` for /remember writes).
    #[arg(long = "forget-source")]
    forget_source: Option<String>,

    /// Tier C / C2.3: dry-run flag for any `--forget-*`. Prints the
    /// matched candidate count + breakdowns and exits without
    /// touching any row.
    #[arg(long = "forget-dry-run")]
    forget_dry_run: bool,

    /// Tier C / C2.3: confirmation flag for any `--forget-*`. Without
    /// this, the CLI defaults to dry-run so unscripted invocations
    /// can't silently drop data.
    #[arg(long = "forget-yes")]
    forget_yes: bool,

    /// Tier C / C2.3: optional reason text written to every audit row
    /// produced by a `--forget-*` invocation. Stored in
    /// `deletions.reason` and surfaced by the panel.
    #[arg(long = "forget-reason")]
    forget_reason: Option<String>,

    /// Tier C / C2.4: redact a single history row in place. The
    /// transcript is replaced with a tombstone (sha + timestamp +
    /// reason); the row continues to exist for provenance. **One-way:
    /// not undoable.** Requires `--redact-confirm` (literal `REDACT`)
    /// and `--redact-reason "<text>"` to actually fire.
    #[arg(long = "forget-history-id")]
    forget_history_id: Option<i64>,

    /// Tier C / C2.4: literal-string confirmation for
    /// `--forget-history-id`. Must equal `REDACT` (uppercase) for the
    /// CLI to dispatch the redaction; without it the run aborts with
    /// a preview of the row.
    #[arg(long = "redact-confirm")]
    redact_confirm: Option<String>,

    /// Tier C / C2.4: required reason text for `--forget-history-id`.
    /// Stored verbatim in the tombstone marker and audit row. Must be
    /// non-empty.
    #[arg(long = "redact-reason")]
    redact_reason: Option<String>,

    /// Disable MCP config synthesis and the in-process Gaviero MCP server
    /// for this run. Extra servers from workspace settings are skipped too.
    #[arg(long = "no-mcp")]
    no_mcp: bool,

    /// Extra remote MCP server for every agent worktree. Format: `name=url`
    /// (repeatable). Merged into `.mcp.json` / `.cursor/mcp.json` and
    /// Codex `config.toml` alongside Gaviero's built-in servers. CLI
    /// entries override workspace `mcp.extraServers` with the same name.
    #[arg(long = "mcp-url", value_name = "NAME=URL")]
    mcp_url: Vec<String>,

    /// Extra stdio MCP server. Format: `name=command,arg1,arg2` (repeatable).
    #[arg(long = "mcp-stdio", value_name = "NAME=COMMAND[,ARGS...]")]
    mcp_stdio: Vec<String>,

    /// Codex project trust for synthesized `.codex/config.toml`.
    /// Use `granted` in CI so Codex agents see MCP servers without a TUI prompt.
    #[arg(long = "mcp-codex-trust", value_name = "granted|denied|unknown")]
    mcp_codex_trust: Option<String>,

    /// Skip MCP preflight (shim on PATH, URL shape, optional HTTP probe).
    #[arg(long = "skip-mcp-preflight")]
    skip_mcp_preflight: bool,

    /// KB-efficiency Phase 1: read the MCP tool-call telemetry sink and
    /// print per-tool intrinsic metrics (call count, p50/p95 latency,
    /// error rate, empty-result rate), then exit. Reads
    /// `<repo>/.gaviero/mcp_calls.ndjson` unless `--mcp-stats-path`
    /// overrides it.
    #[arg(long = "mcp-stats")]
    mcp_stats: bool,

    /// Override the NDJSON path read by `--mcp-stats`.
    #[arg(long = "mcp-stats-path", value_name = "PATH", requires = "mcp_stats")]
    mcp_stats_path: Option<PathBuf>,

    /// Tier A / A2: write a `/remember`-style memory from headless
    /// mode and exit. Goes through the writer task (single-consumer
    /// invariant) — opens [`MemoryServices`] under the hood. Pair with
    /// `--remember-scope` to override the default Repo scope.
    #[arg(long = "remember")]
    remember: Option<String>,

    /// Tier A / A2: scope override for `--remember`. One of `run`,
    /// `module`, `repo`, `workspace`, `global`. Defaults to `repo`
    /// (the plan's recommended default — Run-scoped writes die with
    /// the session).
    #[arg(long = "remember-scope", default_value = "repo")]
    remember_scope: String,
}

/// CLI observer that prints agent events to stderr.
///
/// Every line is prefixed with `[{agent_id}]` so concurrent agents stay legible.
/// When `verbose` is false (script mode), raw streamed text is suppressed;
/// only tool calls, status updates, file writes, and validation results are shown.
/// In verbose mode, streamed chunks are line-buffered so each emitted line is
/// prefixed with the agent id rather than spliced into other agents' output.
struct CliAcpObserver {
    agent_id: String,
    verbose: bool,
    stream_buffer: Mutex<String>,
}

impl CliAcpObserver {
    fn new(agent_id: impl Into<String>, verbose: bool) -> Self {
        Self {
            agent_id: agent_id.into(),
            verbose,
            stream_buffer: Mutex::new(String::new()),
        }
    }

    fn flush_stream_lines(&self, force_final: bool) {
        let Ok(mut buf) = self.stream_buffer.lock() else {
            return;
        };
        while let Some(idx) = buf.find('\n') {
            let line = buf[..idx].to_string();
            buf.drain(..=idx);
            eprintln!("  [{}] {}", self.agent_id, line);
        }
        if force_final && !buf.is_empty() {
            let line = std::mem::take(&mut *buf);
            eprintln!("  [{}] {}", self.agent_id, line);
        }
    }
}

impl AcpObserver for CliAcpObserver {
    fn on_stream_chunk(&self, text: &str) {
        if !self.verbose {
            return;
        }
        if let Ok(mut buf) = self.stream_buffer.lock() {
            buf.push_str(text);
        }
        self.flush_stream_lines(false);
        let _ = std::io::stderr().flush();
    }

    fn on_tool_call_started(&self, tool_summary: &str) {
        self.flush_stream_lines(true);
        eprintln!("  [{}] ⚙ {}", self.agent_id, tool_summary);
    }

    fn on_message_complete(&self, role: &str, content: &str) {
        if role != "assistant" {
            return;
        }
        self.flush_stream_lines(true);
        if self.verbose {
            return;
        }
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return;
        }
        for line in trimmed.lines() {
            eprintln!("  [{}] ▸ {}", self.agent_id, line);
        }
    }

    fn on_proposal_deferred(
        &self,
        path: &std::path::Path,
        _old_content: Option<&str>,
        _new_content: &str,
    ) {
        self.flush_stream_lines(true);
        eprintln!("  [{}] ✎ {}", self.agent_id, path.display());
    }

    fn on_streaming_status(&self, status: &str) {
        self.flush_stream_lines(true);
        eprintln!("  [{}] … {}", self.agent_id, status);
    }

    fn on_validation_result(&self, gate: &str, passed: bool, message: Option<&str>) {
        self.flush_stream_lines(true);
        if passed {
            eprintln!("  [{}] ✓ {}", self.agent_id, gate);
        } else {
            eprintln!(
                "  [{}] ✗ {} — {}",
                self.agent_id,
                gate,
                message.unwrap_or("")
            );
        }
    }

    fn on_validation_retry(&self, attempt: u8, max_retries: u8) {
        self.flush_stream_lines(true);
        eprintln!("  [{}] ↺ retry {}/{}", self.agent_id, attempt, max_retries);
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
                if detail.is_empty() {
                    eprintln!("[{}] starting", work_unit_id);
                } else {
                    eprintln!("[{}] starting — {}", work_unit_id, detail);
                }
            }
            AgentStatus::Completed => {
                let summary = detail.trim();
                if summary.is_empty() {
                    eprintln!("[{}] ✓ done", work_unit_id);
                } else {
                    eprintln!("[{}] ✓ done — {}", work_unit_id, summary);
                }
            }
            AgentStatus::Failed(err) => {
                eprintln!("[{}] ✗ FAILED — {}", work_unit_id, err);
            }
            AgentStatus::Pending => {
                eprintln!("[{}] queued", work_unit_id);
            }
        }
    }

    fn on_tier_started(&self, current: usize, total: usize) {
        eprintln!("[tier] {}/{}", current, total);
    }

    fn on_merge_conflict(&self, branch: &str, files: &[String]) {
        eprintln!("[conflict] branch={}  files={}", branch, files.join(", "));
    }

    fn on_completed(&self, result: &SwarmResult) {
        let n_ok = result
            .manifests
            .iter()
            .filter(|m| matches!(m.status, AgentStatus::Completed))
            .count();
        let n_fail = result.manifests.len() - n_ok;
        if n_fail == 0 {
            eprintln!("[done] all {} agent(s) succeeded", n_ok);
        } else {
            eprintln!("[done] {}/{} failed", n_fail, result.manifests.len());
        }
    }

    fn on_coordination_started(&self, prompt: &str) {
        eprintln!(
            "[coordinator] planning: {}…",
            &prompt[..prompt.len().min(80)]
        );
    }

    fn on_coordination_complete(&self, dag: &gaviero_core::swarm::coordinator::TaskDAG) {
        eprintln!(
            "[coordinator] planned {} unit(s): {}",
            dag.units.len(),
            dag.plan_summary
        );
    }

    fn on_tier_dispatch(&self, unit_id: &str, tier: gaviero_core::types::ModelTier, backend: &str) {
        eprintln!(
            "[dispatch] {}  →  {}  ({})",
            unit_id,
            backend,
            format!("{:?}", tier).to_lowercase()
        );
    }

    fn on_loop_iteration_started(&self, current: u32, max: u32, agents: &[String]) {
        eprintln!(
            "[loop] iteration {}/{}  agents=[{}]",
            current,
            max,
            agents.join(", ")
        );
    }

    fn on_loop_verdict(&self, passed: bool, consecutive: u32, stability: u32) {
        if passed {
            if consecutive >= stability {
                eprintln!(
                    "[loop] verdict PASS — converged (streak {}/{})",
                    consecutive, stability
                );
            } else {
                eprintln!(
                    "[loop] verdict PASS — streak {}/{}, continuing for stability",
                    consecutive, stability
                );
            }
        } else {
            eprintln!("[loop] verdict FAIL — streak reset");
        }
    }

    fn on_cost_update(&self, estimate: &gaviero_core::swarm::verify::CostEstimate) {
        eprintln!("[cost] ~${:.4}", estimate.estimated_usd);
    }
}

fn parse_var_overrides(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|s| {
            let (k, v) = s
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--var `{}`: expected KEY=VALUE format", s))?;
            Ok((k.to_string(), v.to_string()))
        })
        .collect()
}

fn cli_repo_is_default(cli: &Cli) -> bool {
    cli.repo.as_os_str() == std::ffi::OsStr::new(".")
}

/// Resolve a path from `--var` before the workspace root is finalised.
fn resolve_host_path_early(cwd: &std::path::Path, repo: &std::path::Path, value: &str) -> Option<std::path::PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        return None;
    }
    let host = if std::path::Path::new(trimmed).is_absolute() {
        std::path::PathBuf::from(trimmed)
    } else {
        let under_repo = repo.join(trimmed);
        if under_repo.is_file() {
            under_repo
        } else {
            cwd.join(trimmed)
        }
    };
    host.is_file().then_some(host)
}

fn set_var(vars: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some((_, v)) = vars.iter_mut().find(|(k, _)| k == key) {
        *v = value;
    } else {
        vars.push((key.to_string(), value));
    }
}

fn apply_out_dir_default_from_plan(
    vars: &mut Vec<(String, String)>,
    repo: &std::path::Path,
    plan_host: &std::path::Path,
) {
    if vars.iter().any(|(k, _)| k == "OUT_DIR") {
        return;
    }
    let Some(plan_dir) = plan_host.parent() else {
        return;
    };
    let out = path_spec_for_worktree(repo, plan_dir);
    set_var(vars, "OUT_DIR", out);
}

/// When running a DSL script with `PLAN_FILE`, anchor the workspace to the plan's
/// directory (if `--repo` is still the default `.`) and default `OUT_DIR` there.
struct SwarmWorkspacePrep {
    repo_path: PathBuf,
    /// Set when `--script` is present (vars parsed, plan defaults applied).
    override_vars: Option<Vec<(String, String)>>,
}

fn prepare_swarm_workspace(
    cli: &Cli,
    cwd: &std::path::Path,
    execution_mode: gaviero_core::swarm::plan::ExecutionMode,
) -> Result<SwarmWorkspacePrep> {
    use gaviero_core::swarm::plan::ExecutionMode;

    if cli.script.is_none() {
        return Ok(SwarmWorkspacePrep {
            repo_path: cli
                .workspace
                .clone()
                .unwrap_or_else(|| cli.repo.clone()),
            override_vars: None,
        });
    }

    if execution_mode == ExecutionMode::Document && !cli_repo_is_default(cli) {
        anyhow::bail!(
            "workflow declares `execution_mode document`: use --workspace or --var PLAN_FILE=... \
             (omit --repo)"
        );
    }
    if execution_mode == ExecutionMode::Repo && cli.workspace.is_some() {
        anyhow::bail!(
            "workflow declares `execution_mode repo`: --workspace is not allowed (use --repo)"
        );
    }

    let mut vars = parse_var_overrides(&cli.vars)?;
    let mut repo_path = cli
        .workspace
        .clone()
        .unwrap_or_else(|| cli.repo.clone());

    if execution_mode == ExecutionMode::Document {
        if let Some(ws) = &cli.workspace {
            repo_path = ws.clone();
            eprintln!(
                "[execution] document workspace: {}",
                repo_path.display()
            );
        } else if let Some(plan_value) = vars
            .iter()
            .find(|(k, _)| k == "PLAN_FILE")
            .map(|(_, v)| v.as_str())
        {
            let Some(plan_host) = resolve_host_path_early(cwd, &repo_path, plan_value) else {
                anyhow::bail!(
                    "--var PLAN_FILE={plan_value}: file not found (pass an existing path)"
                );
            };
            let plan_dir = plan_host
                .parent()
                .ok_or_else(|| anyhow::anyhow!("--var PLAN_FILE has no parent directory"))?;
            let plan_dir = std::fs::canonicalize(plan_dir)
                .with_context(|| format!("resolving plan directory {}", plan_dir.display()))?;
            if cli_repo_is_default(cli) {
                repo_path = plan_dir.clone();
                let plan_rel = path_spec_for_worktree(&plan_dir, &plan_host);
                set_var(&mut vars, "PLAN_FILE", plan_rel);
                eprintln!(
                    "[execution] document workspace: {} (PLAN_FILE={})",
                    plan_dir.display(),
                    vars.iter()
                        .find(|(k, _)| k == "PLAN_FILE")
                        .map(|(_, v)| v.as_str())
                        .unwrap_or("?")
                );
            }
            apply_out_dir_default_from_plan(&mut vars, &repo_path, &plan_host);
        } else {
            anyhow::bail!(
                "execution document requires --workspace <dir> or --var PLAN_FILE=<path>"
            );
        }
    } else if let Some(plan_value) = vars
        .iter()
        .find(|(k, _)| k == "PLAN_FILE")
        .map(|(_, v)| v.as_str())
    {
        if let Some(plan_host) = resolve_host_path_early(cwd, &repo_path, plan_value) {
            let repo_canon =
                std::fs::canonicalize(&repo_path).unwrap_or_else(|_| repo_path.clone());
            apply_out_dir_default_from_plan(&mut vars, &repo_canon, &plan_host);
        }
    }

    if execution_mode == ExecutionMode::Document {
        if let Some(out) = vars.iter().find(|(k, _)| k == "OUT_DIR").map(|(_, v)| v.as_str()) {
            eprintln!("[execution] OUT_DIR={out} (versioned artefacts; not for commit)");
        }
    }

    Ok(SwarmWorkspacePrep {
        repo_path,
        override_vars: Some(vars),
    })
}

fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating parent directory {}", parent.display()))?;
    }
    Ok(())
}

/// Resolve a `--var` value to an existing file under or outside `--repo`.
fn resolve_var_file_path(repo: &std::path::Path, value: &str) -> Option<std::path::PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        return None;
    }
    let host = if std::path::Path::new(trimmed).is_absolute() {
        std::path::PathBuf::from(trimmed)
    } else {
        repo.join(trimmed)
    };
    host.is_file().then_some(host)
}

fn path_spec_for_worktree(repo: &std::path::Path, host: &std::path::Path) -> String {
    let normalize_rel = |rel: &std::path::Path| {
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() { ".".to_string() } else { s }
    };
    if let Ok(rel) = host.strip_prefix(repo) {
        return normalize_rel(rel);
    }
    let ws = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let host_canon = std::fs::canonicalize(host).unwrap_or_else(|_| host.to_path_buf());
    if let Ok(rel) = host_canon.strip_prefix(&ws) {
        normalize_rel(rel.as_ref())
    } else {
        host.to_string_lossy().to_string()
    }
}

/// Paths from the CLI that may need worktree injection (`--var` files, `--prompt-file`).
fn collect_cli_worktree_context_paths(
    repo: &std::path::Path,
    cwd: &std::path::Path,
    override_vars: &[(String, String)],
    prompt_file: Option<&std::path::Path>,
) -> Vec<String> {
    use std::collections::HashSet;

    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    if let Some(p) = prompt_file {
        let host = if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        };
        if host.is_file() {
            let spec = path_spec_for_worktree(repo, &host);
            if seen.insert(spec.clone()) {
                paths.push(spec);
            }
        }
    }

    for (_key, value) in override_vars {
        if let Some(host) = resolve_var_file_path(repo, value) {
            let spec = path_spec_for_worktree(repo, &host);
            if seen.insert(spec.clone()) {
                paths.push(spec);
            }
        }
    }

    paths
}

/// When a `--var` points at a file outside `--repo`, copy it under
/// `.gaviero/injected/` and rewrite the var so agents and worktrees can read it.
fn materialize_external_vars_for_repo(
    repo: &std::path::Path,
    vars: &mut Vec<(String, String)>,
) -> Result<()> {
    let injected_dir = repo.join(".gaviero/injected");
    let ws = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());

    for (key, value) in vars.iter_mut() {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(host) = resolve_var_file_path(repo, trimmed) else {
            continue;
        };
        let host_canon = std::fs::canonicalize(&host).unwrap_or(host.clone());
        if host_canon.starts_with(&ws) {
            continue;
        }
        std::fs::create_dir_all(&injected_dir)
            .with_context(|| format!("creating {}", injected_dir.display()))?;
        let dest = injected_dir.join(
            host.file_name()
                .ok_or_else(|| anyhow::anyhow!("path has no filename: {}", host.display()))?,
        );
        std::fs::copy(&host, &dest)
            .with_context(|| format!("copying external file into {}", dest.display()))?;
        let rel = dest
            .strip_prefix(repo)
            .with_context(|| "injected file must stay under --repo")?
            .to_string_lossy()
            .replace('\\', "/");
        eprintln!(
            "[vars] {}: file outside --repo copied to {} (was {})",
            key,
            rel,
            host.display()
        );
        *value = rel;
    }
    Ok(())
}

/// Normalize remote MCP URLs (some hosts 308-redirect unless the path ends with `/`).
fn normalize_remote_mcp_url(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("https://mcp.wuilder.com/") && !url.ends_with('/') {
        format!("{url}/")
    } else {
        url.to_string()
    }
}

/// Register `SEMANTIC_SCHOLAR_MCP_URL` / `SEMANTIC_SCHOLAR_SERVER` script vars as MCP URLs.
fn extra_mcp_urls_from_script_vars(vars: &[(String, String)]) -> Vec<(String, String)> {
    let Some(url) = vars
        .iter()
        .find(|(k, _)| k == "SEMANTIC_SCHOLAR_MCP_URL")
        .map(|(_, v)| v.as_str())
        .filter(|v| !v.trim().is_empty())
    else {
        return Vec::new();
    };
    let name = vars
        .iter()
        .find(|(k, _)| k == "SEMANTIC_SCHOLAR_SERVER")
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "semantic-scholar".to_string());
    vec![(name, normalize_remote_mcp_url(url))]
}

fn mcp_overrides_from_cli(
    cli: &Cli,
    script_vars: &[(String, String)],
) -> Result<gaviero_core::mcp::McpConfigOverrides> {
    use gaviero_core::mcp::{parse_mcp_codex_trust_flag, parse_mcp_stdio_flag, parse_mcp_url_flag};

    let mut overrides = gaviero_core::mcp::McpConfigOverrides::default();
    if cli.no_mcp {
        overrides.enabled = Some(false);
    }
    for raw in &cli.mcp_url {
        let (name, url) = parse_mcp_url_flag(raw)?;
        overrides
            .extra_urls
            .push((name, normalize_remote_mcp_url(&url)));
    }
    for (name, url) in extra_mcp_urls_from_script_vars(script_vars) {
        if overrides.extra_urls.iter().any(|(n, _)| n == &name) {
            continue;
        }
        eprintln!("[mcp] {name}: remote server from SEMANTIC_SCHOLAR_MCP_URL");
        overrides.extra_urls.push((name, url));
    }
    for raw in &cli.mcp_stdio {
        let (name, command, args) = parse_mcp_stdio_flag(raw)?;
        overrides.extra_stdio.push((name, command, args));
    }
    if let Some(ref trust) = cli.mcp_codex_trust {
        overrides.codex_trust = Some(parse_mcp_codex_trust_flag(trust)?);
    }
    Ok(overrides)
}

/// Start the in-process MCP server (when memory is available), synthesize
/// per-worktree provider configs at the repo root, and return the synth
/// struct for the swarm pipeline (cloned per agent worktree).
fn prepare_mcp_for_swarm(
    repo: &std::path::Path,
    workspace: &gaviero_core::workspace::Workspace,
    cli: &Cli,
    script_vars: &[(String, String)],
    memory: &Option<Arc<gaviero_core::memory::MemoryStores>>,
) -> Result<(Option<gaviero_core::mcp::McpConfigSynth>, Option<gaviero_core::mcp::McpServerHandle>)> {
    use std::sync::Arc;

    use gaviero_core::mcp::{
        GavieroMcpServer, NdjsonTelemetrySink, PreflightOpts, preflight_mcp,
        resolve_mcp_config_synth, spawn_mcp_server, synthesize_for_worktree,
    };

    let mut overrides = mcp_overrides_from_cli(cli, script_vars)?;
    for (name, url) in gaviero_core::mcp::extra_urls_from_project_mcp_json(repo) {
        if overrides.extra_urls.iter().any(|(n, _)| n == &name) {
            continue;
        }
        eprintln!("[mcp] {name}: remote server from project mcp.json");
        overrides
            .extra_urls
            .push((name, normalize_remote_mcp_url(&url)));
    }
    if memory.is_none() {
        overrides.gaviero_enabled = Some(false);
    }
    let socket_path = repo.join(".gaviero/mcp.sock");
    let synth = resolve_mcp_config_synth(workspace, repo, socket_path, &overrides);
    if !synth.enabled {
        return Ok((None, None));
    }

    if !cli.skip_mcp_preflight {
        preflight_mcp(&synth, PreflightOpts::default())?;
    }

    let mut handle = None;
    if synth.gaviero_enabled {
        if let Some(stores) = memory {
            let retrieval_cfg = workspace.resolve_retrieval_config(Some(repo));
            let rerank_cfg = workspace.resolve_rerank_config(Some(repo));
            let specificity = workspace.resolve_specificity_config(Some(repo));
            let edge_weights = workspace.resolve_all_edge_weights(Some(repo));
            let server = GavieroMcpServer::new(
                stores.clone(),
                repo.to_path_buf(),
                // Phase 1: persist tool-call telemetry to
                // `<repo>/.gaviero/mcp_calls.ndjson` (read back by
                // `--mcp-stats`). Size-rotated, never via the writer.
                Arc::new(NdjsonTelemetrySink::for_workspace(repo)),
                retrieval_cfg,
                rerank_cfg,
                None,
            )
            .with_specificity(specificity)
            .with_edge_weights(edge_weights)
            .with_permissions(synth.permissions.clone())
            .with_symbol_enrichment(
                workspace
                    .resolve_setting(
                        gaviero_core::workspace::settings::REPO_MAP_SYMBOL_ENRICHMENT_ENABLED,
                        Some(repo),
                    )
                    .as_bool()
                    .unwrap_or(false),
            );
            // Phase 1: warm the blast_radius graph cache in the
            // background so the first agent tool call doesn't pay the
            // cold build. (No reranker in the CLI path → graph only.)
            let warm = server.clone();
            tokio::spawn(async move { warm.warmup().await });
            let h = spawn_mcp_server(server, &synth.socket_path)
                .with_context(|| format!("starting gaviero MCP server at {}", synth.socket_path.display()))?;
            eprintln!(
                "[mcp] gaviero server listening on {}",
                h.socket_path.display()
            );
            handle = Some(h);
        }
    }

    if !synth.extra_servers.is_empty() {
        eprintln!(
            "[mcp] {} extra server(s): {}",
            synth.extra_servers.len(),
            synth
                .extra_servers
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        for extra in &synth.extra_servers {
            if let gaviero_core::mcp::ExtraMcpTransport::Url { url } = &extra.transport {
                eprintln!(
                    "[mcp] cursor: invoke MCP tools as {:?} (not display names); url={url}",
                    extra.name
                );
            }
        }
        eprintln!(
            "[mcp] cursor: if MCP tools still fail, set in ~/.cursor/cli-config.json: \
             {{\"approvalMode\":\"unrestricted\",\"sandbox\":{{\"mode\":\"disabled\",\"networkAccess\":\"allow_all\"}}}}",
        );
        if matches!(synth.codex_trust, gaviero_core::mcp::TrustConsent::Granted) {
            eprintln!(
                "[mcp] codex agents will run with --dangerously-bypass-approvals-and-sandbox \
                 (required for MCP tool calls in `codex exec`; sandboxed by per-agent git \
                 worktree + Write Gate). Extras carry `required = true` + \
                 startup_timeout_sec=60 / tool_timeout_sec=60.",
            );
        }
    }

    let paths = synthesize_for_worktree(&synth)
        .context("synthesizing MCP provider config under execution root")?;
    gaviero_core::mcp::validate_synthesized_cursor_remote_mcp(&synth)
        .context("validating Cursor MCP config for remote URL server(s)")?;
    if !paths.is_empty() {
        eprintln!(
            "[mcp] synthesized config at {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    Ok((Some(synth), handle))
}

fn parse_param_overrides(raw: &[String]) -> Result<Vec<(String, String)>> {
    raw.iter()
        .map(|s| {
            let (k, v) = s
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--param `{}`: expected NAME=VALUE format", s))?;
            Ok((k.to_string(), v.to_string()))
        })
        .collect()
}

fn resolve_model_spec(spec: &str, label: &str) -> Result<String> {
    let trimmed = spec.trim();
    gaviero_core::swarm::backend::shared::validate_model_spec(trimmed)
        .with_context(|| format!("invalid {} model spec '{}'", label, trimmed))?;
    Ok(trimmed.to_string())
}

/// Pretty-print manifest rows to stdout. Used by --manifest-last /
/// --manifest-turn (Tier S / S4). Payload is pretty-printed JSON so the
/// developer can eyeball the candidate pool and selection trace without
/// external tooling.
fn print_manifests(rows: &[gaviero_core::memory::store::InjectionManifestRow]) {
    if rows.is_empty() {
        println!("(no manifests)");
        return;
    }
    for r in rows {
        println!(
            "─── manifest id={} turn={} session={} channel={} at={}",
            r.id, r.turn_id, r.session_id, r.source_channel, r.created_at
        );
        match serde_json::from_str::<serde_json::Value>(&r.payload) {
            Ok(v) => match serde_json::to_string_pretty(&v) {
                Ok(s) => println!("{}", s),
                Err(_) => println!("{}", r.payload),
            },
            Err(_) => println!("{}", r.payload),
        }
    }
}

/// Tier B / B5: drive the sleeptime hygiene pass from the CLI. Reuses
/// the same `run_sleeptime` engine the writer task invokes, so the
/// output matches what the TUI would surface during interactive use.
/// Tier A / A2 + Tier S / S2: headless `/remember`. Bootstraps a
/// full [`MemoryServices`] (multi-DB stores + writer task), enqueues
/// `WriterMessage::UserRemember` through the writer (single-consumer
/// invariant), waits for the ack with a 500 ms timeout, prints the
/// outcome, and exits cleanly.
///
/// `scope` is one of `run | module | repo | workspace | global`. The
/// CLI defaults to `repo` (the plan's recommended default — Run-scoped
/// writes die with the session, which is wrong for `/remember`).
async fn run_remember_cli(
    repo: &std::path::Path,
    text: &str,
    scope: &str,
) -> Result<()> {
    use gaviero_core::memory::scope::WriteScope;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--remember: empty text");
    }
    // The CLI is headless — it has no focused buffer (so no Module
    // scope) and no chat session (so no Run scope). Restrict to the
    // three persistent levels callers can express without extra
    // context. The TUI handles `module` / `run` via /remember-here.
    let derived = match scope.to_ascii_lowercase().as_str() {
        "repo" => WriteScope::Repo {
            repo_id: gaviero_core::memory::hash_path(repo),
        },
        "workspace" => WriteScope::Workspace,
        "global" => WriteScope::Global,
        other @ ("run" | "module") => anyhow::bail!(
            "--remember-scope {other} requires session/file context only the TUI supplies; \
             use the `/remember-here` or `/remember-module` chat commands instead",
        ),
        other => anyhow::bail!(
            "--remember-scope: expected repo|workspace|global, got {other}"
        ),
    };

    let repo_buf = repo.to_path_buf();
    let services = tokio::task::spawn_blocking({
        let repo_buf = repo_buf.clone();
        move || -> anyhow::Result<std::sync::Arc<gaviero_core::memory::MemoryServices>> {
            let workspace = gaviero_core::workspace::Workspace::single_folder(repo_buf.clone());
            gaviero_core::memory::MemoryServices::open(
                &repo_buf,
                &workspace,
                gaviero_core::memory::ServicesOpts::default(),
            )
        }
    })
    .await
    .context("init MemoryServices (remember)")??;

    let result = services
        .writer
        .user_remember_scoped(derived, trimmed.to_string())
        .await
        .context("user_remember_scoped enqueue/ack")?;
    println!(
        "[gaviero-remember] {} (scope={}, len={})",
        match result {
            gaviero_core::memory::WriteResult::Inserted(id) => format!("inserted id={id}"),
            gaviero_core::memory::WriteResult::Deduplicated(id) => format!("deduplicated id={id}"),
            gaviero_core::memory::WriteResult::AlreadyCovered =>
                "already covered by broader scope".to_string(),
            gaviero_core::memory::WriteResult::Skipped => "skipped".to_string(),
        },
        scope,
        trimmed.len(),
    );
    Ok(())
}

async fn run_sleeptime_cli(repo: &std::path::Path, dry_run: bool) -> Result<()> {
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (sleeptime)")??;

    let mut cfg = gaviero_core::memory::SleeptimeConfig::default();
    cfg.dry_run = dry_run;
    eprintln!(
        "[gaviero-sleep] {} pass against {}",
        if dry_run { "dry-run" } else { "live" },
        repo.display()
    );
    let report = gaviero_core::memory::run_sleeptime(&store, &cfg, None).await?;
    println!("─── Sleeptime report ────────────────────────────────");
    println!("run_id          : {}", report.run_id);
    println!("dry_run         : {}", report.dry_run);
    println!("decay_flagged   : {}", report.decay_flagged);
    println!("near_dup_merged : {}", report.near_dup_merged);
    println!("promoted        : {}", report.promoted);
    println!("trust_adjusted  : {}", report.trust_adjusted);
    println!("telemetry_pruned: {}", report.telemetry_pruned);
    Ok(())
}

/// Tier C / C2.2: list the N most recent audit rows from the
/// workspace memory.db. Output mirrors the columns the TUI Deletions
/// tab will surface (C2.6); use it to pick an audit id for
/// `--restore-id`.
async fn run_deletions_last_cli(repo: &std::path::Path, n: usize) -> Result<()> {
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (deletions list)")??;
    let rows = store
        .recent_deletions(n)
        .await
        .context("reading deletions audit")?;
    if rows.is_empty() {
        println!("[gaviero-deletions] no audit rows.");
        return Ok(());
    }
    println!(
        "{:>4}  {:>6}  {:<14}  {:<8}  {:<19}  {}",
        "id", "mem_id", "deleted_by", "kind", "deleted_at", "reason"
    );
    for r in rows {
        println!(
            "{:>4}  {:>6}  {:<14}  {:<8}  {:<19}  {}",
            r.id,
            r.memory_id,
            r.deleted_by,
            r.memory_kind,
            r.deleted_at,
            r.reason.as_deref().unwrap_or("—"),
        );
    }
    Ok(())
}

/// Tier C / C2.2: restore a single audit row and print the outcome.
/// Replays the captured row through `MemoryStore::store_scoped` so the
/// dedup pipeline decides whether the row reinserts cleanly, dedups
/// against a newer row, or is already covered.
async fn run_restore_id_cli(repo: &std::path::Path, audit_id: i64) -> Result<()> {
    use gaviero_core::memory::RestoreOutcome;
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (restore)")??;
    let outcome = store
        .restore_deletion(audit_id)
        .await
        .with_context(|| format!("restoring audit {audit_id}"))?;
    match outcome {
        RestoreOutcome::Inserted {
            deletion_id,
            new_memory_id,
        } => println!(
            "[gaviero-restore] audit {deletion_id} reinstated as new memory id {new_memory_id}"
        ),
        RestoreOutcome::Deduplicated {
            deletion_id,
            surviving_memory_id,
        } => println!(
            "[gaviero-restore] audit {deletion_id} merged into existing memory \
             {surviving_memory_id} (dedup hit)"
        ),
        RestoreOutcome::AlreadyCovered { deletion_id } => println!(
            "[gaviero-restore] audit {deletion_id} already covered at a broader scope; \
             nothing new written"
        ),
        RestoreOutcome::Refused {
            deletion_id,
            reason,
        } => {
            eprintln!("[gaviero-restore] refused for audit {deletion_id}: {reason}");
            std::process::exit(2);
        }
    }
    Ok(())
}

/// Tier C / C2.2: restore every pending deletion newer than the given
/// human-readable duration (e.g. `2 hours`, `7 days`).
async fn run_restore_since_cli(repo: &std::path::Path, window: &str) -> Result<()> {
    use gaviero_core::memory::RestoreOutcome;
    let since_offset = parse_restore_since_window(window)?;
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (restore-since)")??;
    let outcomes = store
        .restore_deletions_since(&since_offset)
        .await
        .with_context(|| format!("restoring deletions since {since_offset}"))?;
    let mut inserted = 0u32;
    let mut deduped = 0u32;
    let mut covered = 0u32;
    let mut refused = 0u32;
    for o in &outcomes {
        match o {
            RestoreOutcome::Inserted { .. } => inserted += 1,
            RestoreOutcome::Deduplicated { .. } => deduped += 1,
            RestoreOutcome::AlreadyCovered { .. } => covered += 1,
            RestoreOutcome::Refused { .. } => refused += 1,
        }
    }
    println!(
        "[gaviero-restore-since] {} processed (inserted {inserted}, deduped {deduped}, \
         covered {covered}, refused {refused})",
        outcomes.len()
    );
    Ok(())
}

/// Translate `"2 hours"` / `"7 days"` / `"30 minutes"` (singular / plural)
/// into the SQLite relative-datetime string the store API expects.
fn parse_restore_since_window(spec: &str) -> Result<String> {
    let s = spec.trim();
    if s.is_empty() {
        return Err(anyhow::anyhow!(
            "missing duration (e.g. `2 hours`, `7 days`, `30 minutes`)"
        ));
    }
    let mut it = s.split_whitespace();
    let n: u32 = it
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing count"))?
        .parse()
        .map_err(|_| anyhow::anyhow!("count must be a positive integer"))?;
    let unit_raw = it
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing unit"))?;
    let unit = match unit_raw.trim_end_matches('s') {
        "minute" | "min" => "minutes",
        "hour" | "hr" => "hours",
        "day" => "days",
        other => {
            return Err(anyhow::anyhow!(
                "unsupported unit `{other}` (use minutes / hours / days)"
            ));
        }
    };
    Ok(format!("-{n} {unit}"))
}

/// Tier C / C2.3: drive a bulk-forget from the CLI. Reuses the
/// store's `bulk_forget` so the audit trail matches what the TUI and
/// MCP paths produce. Defaults to dry-run unless `--forget-yes` is
/// set, so an accidental `gaviero-cli --forget-source llm_extracted`
/// can't silently flatten the workspace.
async fn run_forget_cli(
    repo: &std::path::Path,
    filter: gaviero_core::memory::ForgetFilter,
    dry_run: bool,
    reason: Option<&str>,
) -> Result<()> {
    use gaviero_core::memory::deletions::DeletedBy;
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (forget)")??;
    let report = store
        .bulk_forget(&filter, dry_run, reason, DeletedBy::UserCommand)
        .await
        .context("running bulk forget")?;
    if report.candidates.is_empty() {
        println!("[gaviero-forget] no matches.");
        return Ok(());
    }
    println!(
        "[gaviero-forget] {} ({} candidates)",
        if dry_run { "dry-run" } else { "live" },
        report.candidates.len(),
    );
    for (k, n) in &report.kind_breakdown {
        println!("  kind   {k:<10}  {n}");
    }
    for (s, n) in &report.scope_breakdown {
        println!("  scope  {s:<32} {n}");
    }
    if !dry_run {
        println!("  deleted: {} (audit rows written)", report.deleted);
    } else {
        println!("  re-run with --forget-yes to confirm the soft-delete.");
    }
    Ok(())
}

/// Tier C / C2.4: drive `/forget-history` from the CLI. Two-step
/// confirmation is enforced via the `--redact-confirm REDACT` literal
/// + a non-empty `--redact-reason`. Without both, the call aborts
/// with a preview of the row.
async fn run_forget_history_cli(
    repo: &std::path::Path,
    memory_id: i64,
    redact_confirm: Option<&str>,
    redact_reason: Option<&str>,
) -> Result<()> {
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (forget-history)")??;

    let body = store
        .read_history_content(memory_id)
        .await
        .context("reading history row")?;
    let Some(body) = body else {
        eprintln!("[gaviero-redact] no history row at id {memory_id}.");
        std::process::exit(2);
    };

    let confirmed = redact_confirm.map(|s| s == "REDACT").unwrap_or(false);
    let reason = redact_reason.unwrap_or("").trim();
    if !confirmed || reason.is_empty() {
        println!(
            "[gaviero-redact] preview of row {memory_id}:\n  {}",
            body.lines().take(10).collect::<Vec<_>>().join("\n  ")
        );
        eprintln!(
            "Re-run with `--forget-history-id {memory_id} --redact-confirm REDACT \
             --redact-reason \"<text>\"` to redact. \
             Redaction is one-way and CANNOT be undone."
        );
        std::process::exit(2);
    }

    let audit_id = store
        .redact_history_row(memory_id, reason)
        .await
        .with_context(|| format!("redacting history row {memory_id}"))?;
    println!("[gaviero-redact] row {memory_id} redacted; audit {audit_id} written.");
    Ok(())
}

/// Tier B / B6: per-scope utilization report from the CLI.
async fn run_utilization_cli(
    repo: &std::path::Path,
    scope_level: i32,
    top: usize,
    ascending: bool,
) -> Result<()> {
    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (utilization)")??;

    let rows = store
        .top_utilization_in_scope(scope_level, ascending, top)
        .await
        .context("computing utilization")?;
    if rows.is_empty() {
        eprintln!(
            "[gaviero-util] no utilisation data at scope_level={scope_level} \
             (run a few chat turns with telemetry enabled first)"
        );
        return Ok(());
    }
    println!("─── Utilization @ scope_level={scope_level} ───────────────────");
    println!(
        "{:>5}  {:>9}  {:>5}  {:>5}  {:>6}  {}",
        "id", "rate", "inj", "used", "unused", "last_used"
    );
    for (id, util) in rows {
        println!(
            "{:>5}  {:>8.2}%  {:>5}  {:>5}  {:>6}  {}",
            id,
            util.utilization_rate * 100.0,
            util.times_injected,
            util.times_used,
            util.times_unused,
            util.last_used_at.unwrap_or_else(|| "—".into())
        );
    }
    Ok(())
}

/// Tier B / T0 bootstrap: dump the most recent N manifests from `repo`'s
/// `memory.db` into a JSONL fixture so the dev can hand-prune it into a
/// real Tier 1 set. Writes to `out` (or stdout if `None`).
async fn bootstrap_eval_fixture(
    repo: &std::path::Path,
    n: usize,
    out: Option<&std::path::Path>,
) -> Result<()> {
    use gaviero_core::memory::eval::{bootstrap_from_manifests, cases_to_jsonl};

    let store = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        move || gaviero_core::memory::init_workspace(&repo)
    })
    .await
    .context("init memory (eval bootstrap)")??;

    let cases = bootstrap_from_manifests(&store, n).await?;
    if cases.is_empty() {
        anyhow::bail!(
            "no usable manifests found at {} (tried last {n}). Run a few chat \
             turns with manifests enabled first.",
            repo.display()
        );
    }
    let body = cases_to_jsonl(&cases)?;
    match out {
        Some(p) => {
            ensure_parent_dir(p)?;
            std::fs::write(p, &body)
                .with_context(|| format!("writing fixture to {}", p.display()))?;
            eprintln!(
                "[gaviero-eval] wrote {} cases to {}",
                cases.len(),
                p.display()
            );
        }
        None => print!("{body}"),
    }
    Ok(())
}

/// Tier B / T0: Run a Tier 1 retrieval smoke test against `repo`'s
/// workspace `memory.db` using the JSONL fixture. Compares against
/// `<fixture>.baseline.json` (if present) and exits non-zero if
/// recall@5 drops more than `cli.eval_tolerance` on any tag or globally.
async fn run_eval_smoke_test(repo: &std::path::Path, fixture: &PathBuf, cli: &Cli) -> Result<()> {
    use gaviero_core::memory::eval::{
        EvalReport, build_report, load_fixture, run_live, worst_recall5_drop,
    };
    use gaviero_core::memory::{MemoryScope, hash_path};

    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        anyhow::bail!("eval fixture {} contained no cases", fixture.display());
    }

    let store = open_eval_store(repo, "eval").await?;

    let scope_ctx = MemoryScope {
        global_db: PathBuf::new(),
        workspace_db: PathBuf::new(),
        repo_db: None,
        workspace_id: hash_path(repo),
        repo_id: Some(hash_path(repo)),
        module_path: None,
        run_id: None,
    };

    // Default retrieval cfg + no reranker for the smoke test path.
    // The B2 ablation gate calls `run_live` with an explicit reranker
    // wrapper around this same harness.
    let report = run_live(&store, &scope_ctx, &cases, None, None, None).await?;
    let _ = build_report; // re-export silencer for unused-import lint
    print_eval_report(&report);

    let report_out = cli
        .eval_report_out
        .clone()
        .unwrap_or_else(|| fixture.with_extension("last.json"));
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        if let Err(e) = ensure_parent_dir(&report_out)
            .and_then(|()| std::fs::write(&report_out, json).map_err(Into::into))
        {
            tracing::warn!(
                "failed to write eval report to {}: {}",
                report_out.display(),
                e
            );
        }
    }

    let baseline_path = fixture.with_extension("baseline.json");
    if cli.eval_update_baseline {
        if let Ok(json) = serde_json::to_string_pretty(&report) {
            std::fs::write(&baseline_path, json)
                .with_context(|| format!("writing baseline to {}", baseline_path.display()))?;
            eprintln!(
                "[gaviero-eval] baseline updated at {}",
                baseline_path.display()
            );
        }
        return Ok(());
    }

    if !baseline_path.exists() {
        let msg = format!(
            "no baseline at {}; pass --eval-update-baseline to create one, or \
             --eval-allow-missing-baseline to skip the gate for this run.",
            baseline_path.display()
        );
        if cli.eval_allow_missing_baseline {
            eprintln!("[gaviero-eval] {msg}");
            return Ok(());
        }
        anyhow::bail!("{msg}");
    }

    let baseline_json = std::fs::read_to_string(&baseline_path)
        .with_context(|| format!("reading baseline {}", baseline_path.display()))?;
    let baseline: EvalReport = serde_json::from_str(&baseline_json)
        .with_context(|| format!("parsing baseline {}", baseline_path.display()))?;
    let drop = worst_recall5_drop(&baseline, &report);
    eprintln!(
        "[gaviero-eval] worst recall@5 drop vs baseline: {:.3} (tolerance {:.3})",
        drop, cli.eval_tolerance
    );
    if drop > cli.eval_tolerance {
        anyhow::bail!(
            "eval regression: recall@5 dropped by {:.3} (tolerance {:.3})",
            drop,
            cli.eval_tolerance
        );
    }
    Ok(())
}

/// Resolve `memory.embedder.model` from the workspace settings cascade.
/// Returns the empty string when the setting is absent or non-string —
/// which `init_with_embedder_name` interprets as "use the legacy default
/// (nomic)" — so eval runs match the same embedder the TUI / writer
/// will use at retrieval time.
fn resolve_eval_embedder(repo: &std::path::Path) -> String {
    let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.to_path_buf());
    workspace.ensure_settings();
    workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_EMBEDDER_MODEL,
            Some(&repo.to_path_buf()),
        )
        .as_str()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// B1 fix: open the eval workspace honouring the configured embedder
/// (`memory.embedder.model`). Pre-fix, the eval CLI always opened with
/// `init_workspace` which silently used the legacy default — so a B1
/// regression test against the new embedder would actually be running
/// the *old* one.
async fn open_eval_store(
    repo: &std::path::Path,
    context: &'static str,
) -> Result<std::sync::Arc<gaviero_core::memory::MemoryStore>> {
    let embedder_name = resolve_eval_embedder(repo);
    tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        let name = embedder_name.clone();
        move || gaviero_core::memory::init_workspace_with_embedder_name(&repo, &name)
    })
    .await
    .with_context(|| format!("init memory ({context})"))?
}

/// Tier T2 corpus seeding. Walks every `GoldRef::File` entry in
/// `fixture`'s `gold_must` set and writes one Record memory per file
/// at repo scope. Memory content format:
///
/// ```text
/// File <repo-relative path>
/// <leading rustdoc, truncated to doc_chars>
/// Symbols: name1, name2, ...
/// ```
///
/// The substring-rich content lets corpus `gold_must` File / Symbol
/// refs actually match the seeded rows when retrieval scores them by
/// content (which is how the metric helpers in memory::eval work).
///
/// Idempotent: re-running dedupes against existing rows via the
/// writer task's normal store_with_options dedup path.
async fn run_seed_corpus_from_paths(
    repo: &std::path::Path,
    fixture: &PathBuf,
    doc_chars: usize,
    embedder_name: Option<&str>,
) -> Result<()> {
    use gaviero_core::memory::eval::{GoldRef, load_fixture};
    use gaviero_core::memory::scope::WriteScope;
    use gaviero_core::memory::writer::WriterMessage;
    use gaviero_core::repo_map::builder::extract_symbols;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        eprintln!(
            "[gaviero-seed] fixture {} has 0 cases; nothing to seed.",
            fixture.display()
        );
        return Ok(());
    }

    // Collect the set of file paths from every case's gold_must File
    // entries. Dedupe so a path referenced by N cases yields one row.
    let mut paths: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for case in &cases {
        for r in &case.gold_must {
            if let GoldRef::File(p) = r {
                if !p.ends_with('/') {
                    paths.insert(p.clone());
                }
            }
        }
    }
    eprintln!(
        "[gaviero-seed] fixture {}: {} cases → {} unique gold_must File paths",
        fixture.display(),
        cases.len(),
        paths.len()
    );
    if paths.is_empty() {
        eprintln!("[gaviero-seed] no File gold refs to seed; exiting.");
        return Ok(());
    }

    // Aggregate counter observer: per-write timing is not interesting,
    // but we want totals for the end-of-run summary because the
    // fire-and-forget enqueue path doesn't return per-call ack.
    #[derive(Default)]
    struct SeedCounters {
        enqueued: AtomicUsize,
        committed: AtomicUsize,
        failed: AtomicUsize,
        last_failure: Mutex<Option<String>>,
    }
    impl gaviero_core::memory::MemoryObserver for SeedCounters {
        fn on_write_enqueued(&self, _kind: &str) {
            self.enqueued.fetch_add(1, Ordering::Relaxed);
        }
        fn on_write_committed(
            &self,
            _kind: &str,
            _result: &gaviero_core::memory::WriteResult,
        ) {
            self.committed.fetch_add(1, Ordering::Relaxed);
        }
        fn on_write_failed(&self, kind: &str, error: &str) {
            self.failed.fetch_add(1, Ordering::Relaxed);
            let mut g = self.last_failure.lock().unwrap();
            *g = Some(format!("{kind}: {error}"));
        }
    }
    let counters = std::sync::Arc::new(SeedCounters::default());

    let repo_buf = repo.to_path_buf();
    let embedder_for_seed = embedder_name.map(str::to_string);
    let observer: std::sync::Arc<dyn gaviero_core::memory::MemoryObserver> = counters.clone();
    let services = tokio::task::spawn_blocking({
        let repo_buf = repo_buf.clone();
        move || -> anyhow::Result<std::sync::Arc<gaviero_core::memory::MemoryServices>> {
            let workspace = gaviero_core::workspace::Workspace::single_folder(repo_buf.clone());
            let opts = gaviero_core::memory::ServicesOpts {
                embedder_name: embedder_for_seed,
                llm: None,
                observer: Some(observer),
                manifest_observer: None,
            };
            gaviero_core::memory::MemoryServices::open(&repo_buf, &workspace, opts)
        }
    })
    .await
    .context("init MemoryServices (seed-corpus)")??;

    let scope = WriteScope::Repo {
        repo_id: gaviero_core::memory::hash_path(repo),
    };

    let mut enqueued_local = 0usize;
    let mut skipped_missing = 0usize;
    let mut skipped_unsupported = 0usize;

    for rel_path in &paths {
        let abs = repo.join(rel_path);
        if !abs.exists() {
            eprintln!("[gaviero-seed]   ⨯ missing on disk: {rel_path}");
            skipped_missing += 1;
            continue;
        }
        let source = match std::fs::read_to_string(&abs) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[gaviero-seed]   ⨯ unreadable {rel_path}: {e}");
                skipped_missing += 1;
                continue;
            }
        };
        let ext = abs
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        if ext.is_empty() {
            skipped_unsupported += 1;
            continue;
        }

        let leading_doc = extract_leading_doc(&source, doc_chars);
        let symbols = extract_symbols(ext, &source);
        let symbol_names: Vec<String> = symbols.iter().map(|s| s.name.clone()).collect();

        let mut content = String::new();
        content.push_str(&format!("File {rel_path}\n"));
        if !leading_doc.is_empty() {
            content.push_str(&leading_doc);
            content.push('\n');
        }
        if !symbol_names.is_empty() {
            content.push_str("Symbols: ");
            content.push_str(&symbol_names.join(", "));
            content.push('\n');
        }

        // Fire-and-forget. ack: None → no per-call timeout. The
        // writer task processes each message at its own pace; we
        // drain after the loop. ONNX embedding can take 100ms+ per
        // message on a cold model and the legacy `user_remember_scoped`
        // path's 500ms ack timeout cannot accommodate that under
        // back-pressure, so we route through the raw enqueue API.
        let msg = WriterMessage::UserRemember {
            namespace: "user_remember".into(),
            key: "user_remember".into(),
            content,
            metadata: None,
            scope: Some(scope.clone()),
            ack: None,
        };
        if let Err(e) = services.writer.enqueue(msg) {
            eprintln!("[gaviero-seed]   ⨯ enqueue failed for {rel_path}: {e}");
            continue;
        }
        enqueued_local += 1;
    }

    eprintln!(
        "[gaviero-seed] enqueued {enqueued_local} writes; draining writer task…"
    );

    // Drain: loop until queue_depth == 0 AND committed+failed >= enqueued.
    let drain_timeout = Duration::from_secs(120);
    let deadline = Instant::now() + drain_timeout;
    loop {
        let depth = services.writer.queue_depth();
        let enq = counters.enqueued.load(Ordering::Relaxed);
        let com = counters.committed.load(Ordering::Relaxed);
        let fail = counters.failed.load(Ordering::Relaxed);
        if depth == 0 && com + fail >= enq {
            break;
        }
        if Instant::now() >= deadline {
            anyhow::bail!(
                "[gaviero-seed] drain timeout: depth={depth} enq={enq} com={com} fail={fail}"
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let enq = counters.enqueued.load(Ordering::Relaxed);
    let com = counters.committed.load(Ordering::Relaxed);
    let fail = counters.failed.load(Ordering::Relaxed);
    let last_fail = counters.last_failure.lock().unwrap().clone();
    eprintln!("[gaviero-seed] done.");
    eprintln!(
        "  enqueued={enq}  committed={com}  failed={fail}  \
         skipped_missing={skipped_missing}  skipped_unsupported={skipped_unsupported}"
    );
    if let Some(msg) = last_fail {
        eprintln!("  last_failure={msg}");
    }
    if com == 0 && fail == 0 {
        eprintln!("  (note: no commits and no failures — observer wiring likely broken)");
    }
    Ok(())
}

/// Extract the leading `//!` rustdoc block from a Rust source file,
/// or an equivalent `///`-prefixed first comment for non-Rust files
/// where the convention applies. Returns up to `max_chars` characters.
fn extract_leading_doc(source: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("//!") {
            out.push_str(rest.trim_start());
            out.push(' ');
        } else if let Some(rest) = trimmed.strip_prefix("///") {
            out.push_str(rest.trim_start());
            out.push(' ');
        } else if trimmed.is_empty() && out.is_empty() {
            // Skip blank header lines before doc starts.
            continue;
        } else {
            break;
        }
        if out.len() >= max_chars {
            break;
        }
    }
    if out.len() > max_chars {
        out.truncate(max_chars);
    }
    out.trim().to_string()
}

/// Tier T1 / T1.3: scope-matrix runner. Loads `fixture`, runs each
/// case against the live store at every scope in `scopes_csv`, and
/// prints a table with Recall / Precision / NDCG / blast metrics so
/// the dev can see whether narrowing scope improves Precision@K.
async fn run_eval_scope_matrix(
    repo: &std::path::Path,
    fixture: &PathBuf,
    scopes_csv: &str,
) -> Result<()> {
    use gaviero_core::memory::eval::{load_fixture, run_scope_matrix};
    use gaviero_core::memory::MemoryScope;
    use gaviero_core::memory::hash_path;

    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        eprintln!(
            "[gaviero-eval] fixture {} has 0 cases; nothing to score.",
            fixture.display()
        );
        return Ok(());
    }
    let scopes: Vec<String> = scopes_csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if scopes.is_empty() {
        anyhow::bail!("--eval-scope-matrix-scopes resolved to empty list");
    }

    let store = open_eval_store(repo, "scope matrix").await?;
    let _ = hash_path(repo);
    let scope_ctx = MemoryScope::from_context(repo, Some(repo), None, None);
    let results = run_scope_matrix(&store, &scope_ctx, &cases, &scopes, None, None, None).await?;
    print_scope_matrix(&scopes, &results);
    Ok(())
}

fn print_scope_matrix(
    scopes: &[String],
    results: &[(String, gaviero_core::memory::eval::EvalReport)],
) {
    println!("─── T1.3 scope matrix ─────────────────────────────────────");
    println!("scopes probed: {}", scopes.join(", "));
    // `r@5` (legacy) keys on `expected_memory_id` and is N/A for gold-set
    // corpora. `1-under` is its gold-set equivalent: 1 - mean(|G_must \ R| / |G_must|).
    println!(
        "{:<10}  {:>5}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}",
        "scope", "n", "r@5", "1-under", "p@5", "p@10", "ndcg5", "ndcg10", "leak", "over", "forbid"
    );
    for (label, r) in results {
        let gold_recall = (1.0 - r.under_retrieval).clamp(0.0, 1.0);
        println!(
            "{:<10}  {:>5}  {:>6.3}  {:>6.3}  {:>6.3}  {:>6.3}  {:>6.3}  {:>6.3}  {:>6.3}  {:>6.3}  {:>6.3}",
            label,
            r.total,
            r.recall_at_5,
            gold_recall,
            r.precision_at_5,
            r.precision_at_10,
            r.ndcg_at_5,
            r.ndcg_at_10,
            r.blast_leakage,
            r.over_retrieval,
            r.forbid_hit_rate,
        );
    }
}

/// Tier B / T0: replay the fixture against persisted manifests. No
/// embedder, no reranker, no LLM — opens the store with a no-op
/// embedder choice (we only read `injection_manifests`).
async fn run_eval_from_manifests(
    repo: &std::path::Path,
    fixture: &PathBuf,
    n: usize,
) -> Result<()> {
    use gaviero_core::memory::eval::{load_fixture, run_from_manifests};
    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        anyhow::bail!("eval fixture {} contained no cases", fixture.display());
    }
    let store = open_eval_store(repo, "eval rescore").await?;
    let report = run_from_manifests(&store, &cases, n).await?;
    print_eval_report(&report);
    Ok(())
}

/// Backup `.gaviero/memory.db` (+ WAL/SHM sidecars) before a destructive
/// eval arm. Restored by [`restore_memory_db`].
struct MemoryDbBackup {
    db: Option<Vec<u8>>,
    wal: Option<Vec<u8>>,
    shm: Option<Vec<u8>>,
}

fn backup_memory_db(repo: &std::path::Path) -> Result<MemoryDbBackup> {
    let dir = repo.join(".gaviero");
    let read = |name: &str| -> Result<Option<Vec<u8>>> {
        let p = dir.join(name);
        if p.exists() {
            Ok(Some(std::fs::read(&p).with_context(|| format!("reading {}", p.display()))?))
        } else {
            Ok(None)
        }
    };
    Ok(MemoryDbBackup {
        db: read("memory.db")?,
        wal: read("memory.db-wal")?,
        shm: read("memory.db-shm")?,
    })
}

fn remove_memory_db(repo: &std::path::Path) -> Result<()> {
    let dir = repo.join(".gaviero");
    for name in ["memory.db", "memory.db-wal", "memory.db-shm"] {
        let p = dir.join(name);
        if p.exists() {
            std::fs::remove_file(&p)
                .with_context(|| format!("removing {}", p.display()))?;
        }
    }
    Ok(())
}

fn restore_memory_db(repo: &std::path::Path, backup: &MemoryDbBackup) -> Result<()> {
    remove_memory_db(repo)?;
    let dir = repo.join(".gaviero");
    std::fs::create_dir_all(&dir)?;
    let write = |name: &str, bytes: &Option<Vec<u8>>| -> Result<()> {
        if let Some(b) = bytes {
            std::fs::write(dir.join(name), b)
                .with_context(|| format!("restoring {name}"))?;
        }
        Ok(())
    };
    write("memory.db", &backup.db)?;
    write("memory.db-wal", &backup.wal)?;
    write("memory.db-shm", &backup.shm)?;
    Ok(())
}

/// One arm of the embedder ablation (S3.1): `(resolve-name, display
/// label, recall/ranking report, (p50, p95, mean) embed latency ms)`.
type EmbedderArm = (
    String,
    String,
    gaviero_core::memory::eval::EvalReport,
    (f64, f64, f64),
);

/// KB-efficiency S1.1 / B1g + S3.1 / PR-4: compare `nomic` (incumbent),
/// `gte-modernbert`, and `jina-code` on the gold-set fixture with a
/// seeded corpus per arm (fair vector space). Reports per-arm recall /
/// ranking metrics (Δ vs the `nomic` incumbent) **and** per-arm CPU
/// embed latency (p50/p95/mean ms/query), since PR-4's flip gate is
/// "code-recall + CPU-latency vs incumbent".
async fn run_eval_embedder_ablation(repo: &std::path::Path, fixture: &PathBuf) -> Result<()> {
    use gaviero_core::memory::eval::{EvalReport, load_fixture, run_scope_matrix};
    use gaviero_core::memory::{
        MemoryScope, Reranker, build_reranker, hash_path, init_workspace_with_embedder_name,
    };

    // Incumbent first; Δ columns are computed against `arms[0]` (`nomic`).
    // Each entry is (resolve-name, short display label).
    const ARMS: &[(&str, &str)] = &[
        ("nomic", "nomic"),
        ("gte-modernbert", "gte-mbert"),
        ("jina-code", "jina-code"),
    ];

    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        anyhow::bail!("eval fixture {} contained no cases", fixture.display());
    }
    let query_texts: Vec<String> = cases.iter().map(|c| c.query.clone()).collect();

    eprintln!(
        "[gaviero-eval] backing up {} before embedder ablation…",
        repo.join(".gaviero/memory.db").display()
    );
    let backup = backup_memory_db(repo)?;

    let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.to_path_buf());
    workspace.ensure_settings();
    let workspace_root = repo.to_path_buf();
    let retrieval_cfg = workspace.resolve_retrieval_config(Some(&workspace_root));
    let mut rerank_cfg = workspace.resolve_rerank_config(Some(&workspace_root));
    rerank_cfg.enabled = true;

    let reranker_arc: Option<std::sync::Arc<dyn Reranker>> = {
        let model_name = workspace
            .resolve_setting(
                gaviero_core::workspace::settings::MEMORY_RERANKER_MODEL,
                Some(&workspace_root),
            )
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "minilm".to_string());
        let threads = rerank_cfg.threads;
        eprintln!("[gaviero-eval] loading reranker `{model_name}` for embedder ablation…");
        let built = tokio::task::spawn_blocking(move || build_reranker(&model_name, threads))
            .await
            .context("loading reranker (embedder ablation)")??;
        built.map(std::sync::Arc::from)
    };
    if let Some(ref rr) = reranker_arc {
        if let Err(e) = rr.warmup().await {
            tracing::warn!(target: "memory_rerank", error = %e, "rerank warmup failed");
        }
    }

    let scope_ctx = MemoryScope {
        global_db: PathBuf::new(),
        workspace_db: PathBuf::new(),
        repo_db: None,
        workspace_id: hash_path(repo),
        repo_id: Some(hash_path(repo)),
        module_path: None,
        run_id: None,
    };

    // (resolve-name, label, recall/ranking report, (p50, p95, mean) ms)
    let mut arms: Vec<EmbedderArm> = Vec::new();
    for &(resolve_name, label) in ARMS {
        eprintln!("[gaviero-eval] embedder arm `{label}` ({resolve_name}): fresh db + seed…");
        remove_memory_db(repo)?;
        run_seed_corpus_from_paths(repo, fixture, 480, Some(resolve_name)).await?;

        let store = tokio::task::spawn_blocking({
            let repo = repo.to_path_buf();
            let model = resolve_name.to_string();
            move || init_workspace_with_embedder_name(&repo, &model)
        })
        .await
        .with_context(|| format!("init memory (embedder ablation, {resolve_name})"))??;

        let matrix = run_scope_matrix(
            &store,
            &scope_ctx,
            &cases,
            &["run".to_string()],
            Some(&retrieval_cfg),
            reranker_arc.as_deref(),
            Some(&rerank_cfg),
        )
        .await?;
        let report = matrix
            .into_iter()
            .next()
            .map(|(_, r)| r)
            .context("scope matrix returned no rows")?;

        // CPU embed-latency probe: a separate, bare embedder load so the
        // timing is pure embed cost (no retrieval / rerank noise).
        let latency = measure_embed_latency(resolve_name, &query_texts).await?;

        arms.push((resolve_name.to_string(), label.to_string(), report, latency));
    }

    eprintln!("[gaviero-eval] restoring workspace memory.db…");
    restore_memory_db(repo, &backup)?;

    println!("─── Embedder ablation (B1g / S1.1 + S3.1 / PR-4) ───────");
    println!("fixture     : {}", fixture.display());
    println!("scope       : run (seeded gold_must File corpus per arm)");
    println!("cases       : {}", cases.len());
    println!(
        "reranker    : {} ({})",
        if reranker_arc.is_some() { "on" } else { "off" },
        workspace
            .resolve_setting(
                gaviero_core::workspace::settings::MEMORY_RERANKER_MODEL,
                Some(&workspace_root),
            )
            .as_str()
            .unwrap_or("minilm")
    );

    // ── recall / ranking (Δ vs the incumbent `arms[0]`) ──
    println!("\nrecall / ranking (Δ vs incumbent `{}`):", arms[0].0);
    print!("{:13}", "metric");
    for (_, label, _, _) in &arms {
        print!("  {label:>18}");
    }
    println!();
    let row = |name: &str, get: fn(&EvalReport) -> f32| {
        print!("{name:13}");
        let base = get(&arms[0].2);
        for (i, (_, _, r, _)) in arms.iter().enumerate() {
            let v = get(r);
            if i == 0 {
                print!("  {v:>18.3}");
            } else {
                print!("  {:>9.3} ({:+.3})", v, v - base);
            }
        }
        println!();
    };
    row("precision@5", |r| r.precision_at_5);
    row("precision@10", |r| r.precision_at_10);
    row("ndcg@5", |r| r.ndcg_at_5);
    row("ndcg@10", |r| r.ndcg_at_10);
    row("blast_leak", |r| r.blast_leakage);
    row("over_ret", |r| r.over_retrieval);
    // under_ret = fraction of gold_must MISSED — disambiguates a low
    // over_ret (tight-and-complete vs tight-but-dropping-gold).
    row("under_ret", |r| r.under_retrieval);

    // ── CPU embed latency ──
    println!(
        "\nCPU embed latency (ms/query over {} fixture queries, intra_threads=1):",
        query_texts.len()
    );
    println!("{:13}  {:>8}  {:>8}  {:>8}", "arm", "p50", "p95", "mean");
    for (_, label, _, (p50, p95, mean)) in &arms {
        println!("{label:13}  {p50:>8.1}  {p95:>8.1}  {mean:>8.1}");
    }

    // ── verdict: jina-code vs the `nomic` incumbent ──
    let incumbent = &arms[0].2;
    if let Some((_, _, jina, jlat)) = arms.iter().find(|(n, _, _, _)| n == "jina-code") {
        let ndcg5_d = jina.ndcg_at_5 - incumbent.ndcg_at_5;
        let p5_d = jina.precision_at_5 - incumbent.precision_at_5;
        println!(
            "\nverdict     : jina-code vs nomic — ndcg@5 Δ = {:+.3}, precision@5 Δ = {:+.3}, \
             p50 latency = {:.1} ms/query",
            ndcg5_d, p5_d, jlat.0
        );
        println!(
            "PR-4 gate   : embedder-flip authority. Band (per-kind recall@5): ≥ +5 pt across \
             all 4 kinds → flip; ≤ +1 pt or any-kind regression → no flip; between → widen \
             the corpus. Latency tiebreak: if recall ≈, prefer the faster arm for symbol \
             vectors (`repoMap.embedder.model`); memory may keep `nomic`."
        );
    } else {
        println!("\nverdict     : jina-code arm absent — check the ARMS table.");
    }
    Ok(())
}

/// CPU embed-latency probe for the embedder ablation (S3.1). Builds the
/// named embedder fresh and times `embed_query` over the fixture queries
/// on CPU (`OnnxEmbedder` builds its session with `intra_threads = 1`).
/// Returns `(p50, p95, mean)` in milliseconds.
async fn measure_embed_latency(model: &str, queries: &[String]) -> Result<(f64, f64, f64)> {
    let embedder = tokio::task::spawn_blocking({
        let model = model.to_string();
        move || gaviero_core::memory::build_embedder_by_name(&model)
    })
    .await
    .with_context(|| format!("building embedder for latency probe ({model})"))??;

    let mut samples_ms = Vec::with_capacity(queries.len());
    for q in queries {
        let started = std::time::Instant::now();
        embedder
            .embed_query(q)
            .await
            .with_context(|| format!("embed_query during latency probe ({model})"))?;
        samples_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(percentiles_ms(samples_ms))
}

/// `(p50, p95, mean)` of a millisecond sample set; zeros on empty input.
fn percentiles_ms(mut samples: Vec<f64>) -> (f64, f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = samples.len();
    let pct = |q: f64| -> f64 {
        let idx = ((q * n as f64).ceil() as usize).saturating_sub(1).min(n - 1);
        samples[idx]
    };
    let mean = samples.iter().sum::<f64>() / n as f64;
    (pct(0.5), pct(0.95), mean)
}

/// Tier B / B2f: rerank ablation. Runs the fixture twice — once with
/// the reranker enabled, once without — and prints recall@K / MRR
/// deltas so the dev can decide whether to flip
/// `memory.reranker.enabled = true`.
///
/// On `build_reranker` failure (no model file, network unavailable),
/// the ablation aborts with a clear message — the off-mode alone is
/// just `--eval-fixture` without this flag, so we don't double-run.
async fn run_eval_rerank_ablation(repo: &std::path::Path, fixture: &PathBuf) -> Result<()> {
    use gaviero_core::memory::eval::{load_fixture, run_live};
    use gaviero_core::memory::{MemoryScope, Reranker, RetrievalConfig, build_reranker, hash_path};

    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        anyhow::bail!("eval fixture {} contained no cases", fixture.display());
    }

    let store = open_eval_store(repo, "rerank ablation").await?;

    let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.to_path_buf());
    workspace.ensure_settings();
    let workspace_root = repo.to_path_buf();
    // Drive the ON-mode config from settings (model / pool_size / threads /
    // blend) so the temp-repo settings.json can sweep configurations; only
    // `enabled` is forced on.
    let mut rerank_cfg = workspace.resolve_rerank_config(Some(&workspace_root));
    rerank_cfg.enabled = true;
    let model_name = workspace
        .resolve_setting(
            gaviero_core::workspace::settings::MEMORY_RERANKER_MODEL,
            Some(&workspace_root),
        )
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "gte-reranker-modernbert-base-int8".to_string());

    eprintln!(
        "[gaviero-eval] loading reranker `{model_name}` (pool={}, threads={})…",
        rerank_cfg.pool_size, rerank_cfg.threads
    );
    let threads = rerank_cfg.threads;
    let reranker = tokio::task::spawn_blocking({
        let model = model_name.clone();
        move || build_reranker(&model, threads)
    })
    .await
    .context("loading reranker (ablation)")??;
    let Some(rr) = reranker else {
        anyhow::bail!(
            "rerank model `{model_name}` resolved to none — set \
             `memory.reranker.model` to a known reranker (e.g. \
             gte-reranker-modernbert-int8) or download the model file first."
        );
    };
    let rr_arc: std::sync::Arc<dyn Reranker> = rr;
    if let Err(e) = rr_arc.warmup().await {
        tracing::warn!(target: "memory_rerank", error = %e, "rerank warmup failed");
    }

    let scope_ctx = MemoryScope {
        global_db: PathBuf::new(),
        workspace_db: PathBuf::new(),
        repo_db: None,
        workspace_id: hash_path(repo),
        repo_id: Some(hash_path(repo)),
        module_path: None,
        run_id: None,
    };
    let retrieval_cfg = RetrievalConfig::default();

    eprintln!("[gaviero-eval] running OFF-mode (composite-only)…");
    let off = run_live(&store, &scope_ctx, &cases, Some(&retrieval_cfg), None, None).await?;
    eprintln!("[gaviero-eval] running ON-mode (rerank enabled)…");
    let on = run_live(
        &store,
        &scope_ctx,
        &cases,
        Some(&retrieval_cfg),
        Some(rr_arc.as_ref()),
        Some(&rerank_cfg),
    )
    .await?;

    println!("─── Rerank ablation (B2f) ────────────────────────────");
    println!("model       : {model_name}");
    println!(
        "pool/threads: {} / {} (0=auto)",
        rerank_cfg.pool_size, rerank_cfg.threads
    );
    println!("cases       : {}", cases.len());
    println!("             {:>10}  {:>10}  {:>10}", "off", "on", "Δ");
    let row = |label: &str, a: f32, b: f32| {
        println!("{:11}  {:>10.3}  {:>10.3}  {:>+10.3}", label, a, b, b - a);
    };
    row("recall@1", off.recall_at_1, on.recall_at_1);
    row("recall@5", off.recall_at_5, on.recall_at_5);
    row("recall@10", off.recall_at_10, on.recall_at_10);
    row("MRR", off.mrr, on.mrr);
    // Gold-set (T1.3) metrics: these are the ones that move on a
    // gold-set fixture like `code_prompts.jsonl`, where every case
    // expresses ground truth via `gold_must`/`gold_neutral` and carries
    // no `expected_memory_id` (so Recall@K / MRR above stay ~0).
    row("precision@5", off.precision_at_5, on.precision_at_5);
    row("precision@10", off.precision_at_10, on.precision_at_10);
    row("ndcg@5", off.ndcg_at_5, on.ndcg_at_5);
    row("ndcg@10", off.ndcg_at_10, on.ndcg_at_10);
    row("blast_leak", off.blast_leakage, on.blast_leakage);
    let r5_delta = on.recall_at_5 - off.recall_at_5;
    let ndcg5_delta = on.ndcg_at_5 - off.ndcg_at_5;
    println!(
        "\nverdict     : recall@5 Δ = {:+.3}  |  ndcg@5 Δ = {:+.3}\n\
         (legacy fixtures gate on recall@5; gold-set fixtures gate on \
         ndcg@5 — B2 plan target ≥ +0.030)",
        r5_delta, ndcg5_delta
    );
    Ok(())
}

fn print_eval_report(r: &gaviero_core::memory::eval::EvalReport) {
    println!("─── Tier 1 retrieval eval ────────────────────────────");
    println!("total cases : {}", r.total);
    println!("recall@1    : {:.3}", r.recall_at_1);
    println!("recall@5    : {:.3}", r.recall_at_5);
    println!("recall@10   : {:.3}", r.recall_at_10);
    println!("MRR         : {:.3}", r.mrr);
    if !r.per_tag.is_empty() {
        println!("per-tag recall@5:");
        let mut tags: Vec<_> = r.per_tag.iter().collect();
        tags.sort_by(|a, b| a.0.cmp(b.0));
        for (tag, stats) in tags {
            println!(
                "  {:20} n={:3} r@5={:.3}",
                tag, stats.total, stats.recall_at_5
            );
        }
    }
    let misses: Vec<_> = r.outcomes.iter().filter(|o| !o.hit_at(5)).collect();
    if !misses.is_empty() {
        println!("misses (rank > 5):");
        for o in misses {
            let rank = match o.rank {
                Some(r) => format!("rank={}", r),
                None => "absent".to_string(),
            };
            let expected = o
                .expected_memory_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "<gold-set>".into());
            println!("  {} expected={} {}", o.id, expected, rank);
        }
    }
}

fn parse_workspace_exclude_patterns(
    workspace: &gaviero_core::workspace::Workspace,
    root: Option<&std::path::Path>,
) -> Vec<String> {
    use gaviero_core::workspace::settings;
    let val = workspace.resolve_setting(settings::FILES_EXCLUDE, root);
    let mut patterns = Vec::new();
    if let Some(obj) = val.as_object() {
        for (pattern, enabled) in obj {
            if enabled.as_bool().unwrap_or(false) {
                patterns.push(pattern.clone());
            }
        }
    }
    patterns
}

/// KB-efficiency S1.3: sweep chat `max_items` and graph budget knobs.
async fn run_eval_budget_sweep(repo: &std::path::Path, fixture: &PathBuf, cli: &Cli) -> Result<()> {
    use gaviero_core::memory::eval::{load_fixture, run_s13_budget_sweep};
    use gaviero_core::memory::{MemoryScope, Reranker, build_reranker, hash_path};
    use gaviero_core::repo_map::RepoMap;

    let cases = load_fixture(fixture).context("loading eval fixture")?;
    if cases.is_empty() {
        anyhow::bail!("eval fixture {} contained no cases", fixture.display());
    }

    let store = open_eval_store(repo, "S1.3 budget sweep").await?;

    let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.to_path_buf());
    workspace.ensure_settings();
    let workspace_root = repo.to_path_buf();
    let excludes = parse_workspace_exclude_patterns(&workspace, Some(&workspace_root));
    let chat_cfg = workspace.resolve_chat_injection_config(Some(&workspace_root));
    let retrieval_cfg = workspace.resolve_retrieval_config(Some(&workspace_root));
    let rerank_cfg = workspace.resolve_rerank_config(Some(&workspace_root));

    let reranker_arc: Option<std::sync::Arc<dyn Reranker>> = if rerank_cfg.enabled {
        let model_name = workspace
            .resolve_setting(
                gaviero_core::workspace::settings::MEMORY_RERANKER_MODEL,
                Some(&workspace_root),
            )
            .as_str()
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "minilm".to_string());
        let threads = rerank_cfg.threads;
        eprintln!(
            "[gaviero-eval] loading reranker `{model_name}` for chat-path sweep…"
        );
        let built = tokio::task::spawn_blocking(move || build_reranker(&model_name, threads))
            .await
            .context("loading reranker (budget sweep)")??;
        built.map(std::sync::Arc::from)
    } else {
        None
    };
    if let Some(ref rr) = reranker_arc {
        if let Err(e) = rr.warmup().await {
            tracing::warn!(target: "memory_rerank", error = %e, "rerank warmup failed");
        }
    }

    eprintln!(
        "[gaviero-eval] building RepoMap for graph budget sweep (may take a minute)…"
    );
    let repo_map = tokio::task::spawn_blocking({
        let repo = repo.to_path_buf();
        let excludes = excludes.clone();
        move || RepoMap::build(&repo, &excludes)
    })
    .await
    .context("RepoMap build (budget sweep)")??;

    let scope_ctx = MemoryScope {
        global_db: PathBuf::new(),
        workspace_db: PathBuf::new(),
        repo_db: None,
        workspace_id: hash_path(repo),
        repo_id: Some(hash_path(repo)),
        module_path: None,
        run_id: None,
    };

    let report = run_s13_budget_sweep(
        &store,
        &scope_ctx,
        &cases,
        &chat_cfg,
        &retrieval_cfg,
        reranker_arc.as_deref(),
        if reranker_arc.is_some() {
            Some(&rerank_cfg)
        } else {
            None
        },
        &repo_map,
        &[4_000, 8_000, 12_000],
        &fixture.to_string_lossy(),
        &repo.to_string_lossy(),
    )
    .await?;

    print_s13_budget_sweep_report(&report);

    let report_out = cli
        .eval_report_out
        .clone()
        .unwrap_or_else(|| fixture.with_extension("s13-sweep.json"));
    if let Ok(json) = serde_json::to_string_pretty(&report) {
        ensure_parent_dir(&report_out)
            .and_then(|()| std::fs::write(&report_out, json).map_err(Into::into))
            .with_context(|| format!("writing S1.3 sweep report to {}", report_out.display()))?;
        eprintln!(
            "[gaviero-eval] S1.3 sweep report written to {}",
            report_out.display()
        );
    }
    Ok(())
}

fn print_s13_budget_sweep_report(r: &gaviero_core::memory::eval::S13BudgetSweepReport) {
    println!("─── S1.3 budget sweep ────────────────────────────────");
    println!("fixture   : {}", r.fixture);
    println!("workspace : {}", r.workspace);
    println!();
    println!("max_items sweep (chat injection path):");
    println!(
        "  {:>9}  {:>10}  {:>8}  {:>8}  {:>8}  {:>8}",
        "max_items", "mem_tok", "p@5", "ndcg@5", "under", "leak"
    );
    for row in &r.max_items_sweep {
        println!(
            "  {:>9}  {:>10.1}  {:>8.3}  {:>8.3}  {:>8.3}  {:>8.3}",
            row.max_items,
            row.mean_memory_tokens,
            row.precision_at_5,
            row.ndcg_at_5,
            row.under_retrieval,
            row.blast_leakage,
        );
    }
    println!();
    println!("graphBudgetTokens sweep (repo outline):");
    println!(
        "  {:>9}  {:>10}  {:>6}  {:>6}  {:>6}  {:>6}",
        "budget", "outline", "files", "path", "sig", "full"
    );
    for row in &r.graph_budget_sweep {
        println!(
            "  {:>9}  {:>10}  {:>6}  {:>6}  {:>6}  {:>6}",
            row.graph_budget_tokens,
            row.outline_tokens,
            row.file_count,
            row.path_only,
            row.signature_only,
            row.full_attach,
        );
    }
    println!();
    println!(
        "recommended: max_items={} graphBudgetTokens={}",
        r.recommended_max_items, r.recommended_graph_budget_tokens
    );
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
        .unwrap_or_else(|| "claude:sonnet".to_string());
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

/// KB-efficiency Phase 1: read the MCP tool-call telemetry sink and
/// print per-tool intrinsic metrics to stdout. Intrinsic only — no
/// task-success correlation. A missing/empty sink is not an error.
fn run_mcp_stats(repo: &std::path::Path, path_override: Option<&std::path::Path>) -> Result<()> {
    let path = path_override
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| gaviero_core::mcp::default_telemetry_path(repo));
    let stats = gaviero_core::mcp::compute_stats(&path)
        .with_context(|| format!("reading MCP telemetry from {}", path.display()))?;
    if stats.is_empty() {
        println!("No MCP tool-call telemetry at {}", path.display());
        return Ok(());
    }
    let total: usize = stats.iter().map(|s| s.calls).sum();
    println!("MCP tool-call stats — {} call(s) at {}", total, path.display());
    println!(
        "{:<16} {:>6} {:>10} {:>10} {:>8} {:>8}",
        "tool", "calls", "p50(ms)", "p95(ms)", "err%", "empty%"
    );
    for s in &stats {
        println!(
            "{:<16} {:>6} {:>10.2} {:>10.2} {:>7.1}% {:>7.1}%",
            s.tool_name,
            s.calls,
            s.p50_ms,
            s.p95_ms,
            s.error_rate * 100.0,
            s.empty_result_rate * 100.0,
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing: JSON to file when --trace is set, human-readable to stderr otherwise.
    // --verbose/-v = INFO, -vv = DEBUG.
    if let Some(ref trace_path) = cli.trace {
        ensure_parent_dir(trace_path)?;
        let file = std::fs::File::create(trace_path)
            .with_context(|| format!("creating trace file: {}", trace_path.display()))?;
        tracing_subscriber::fmt()
            .json()
            .with_writer(file)
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        let level = match cli.verbose {
            0 => tracing::Level::WARN,
            1 => tracing::Level::INFO,
            _ => tracing::Level::DEBUG,
        };
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_max_level(level)
            .init();
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let script_execution_mode = cli
        .script
        .as_ref()
        .map(|p| {
            gaviero_dsl::workflow_execution_mode(p, cli.workflow.as_deref()).map_err(|e| {
                eprintln!("{:?}", e);
                anyhow::anyhow!("failed to read workflow execution_mode from script")
            })
        })
        .transpose()?
        .unwrap_or(gaviero_core::swarm::plan::ExecutionMode::Repo);
    let swarm_workspace = prepare_swarm_workspace(&cli, &cwd, script_execution_mode)?;
    let repo = std::fs::canonicalize(&swarm_workspace.repo_path)
        .with_context(|| format!("resolving repo path: {}", swarm_workspace.repo_path.display()))?;

    // ── KB-efficiency Phase 1: MCP telemetry report ──────────────
    // A pure read of the NDJSON sink. Placed before the C1 migration
    // probe so `--mcp-stats` never forces a migration prompt.
    if cli.mcp_stats {
        return run_mcp_stats(&repo, cli.mcp_stats_path.as_deref());
    }

    // ── Tier C / C1: enforce explicit consent for the typed-stores
    // migration. Headless invocation cannot prompt; require the
    // `--accept-c1-migration` flag if any reachable memory.db is at a
    // pre-v10 schema. Plan §"Anti-patterns to avoid": no silent
    // migration on first run.
    {
        let workspace = gaviero_core::workspace::Workspace::single_folder(repo.clone());
        let pending = gaviero_core::memory::MemoryStores::probe_pending_c1_migrations(
            &repo, &workspace,
        )
        .context("probing for pending C1 typed-stores migration")?;
        if !pending.is_empty() && !cli.accept_c1_migration {
            eprintln!(
                "Gaviero's memory schema requires a one-time typed-stores upgrade (C1)."
            );
            eprintln!("Affected databases:");
            for p in &pending {
                eprintln!(
                    "  - {}  (v{} → v{})",
                    p.db_path.display(),
                    p.current_version,
                    p.target_version
                );
                eprintln!("    backup → {}", p.proposed_backup_path.display());
            }
            eprintln!();
            eprintln!(
                "Re-run with `--accept-c1-migration` to proceed. Each DB will be \
                 snapshotted to the path shown above before migration."
            );
            std::process::exit(2);
        }
    }

    // ── Manifest introspection (Tier S / S4): print and exit ─────
    if cli.manifest_last.is_some() || cli.manifest_turn.is_some() {
        let store = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || gaviero_core::memory::init_workspace(&repo)
        })
        .await
        .context("init memory (manifest introspection)")??;

        if let Some(turn_id) = &cli.manifest_turn {
            let rows = store
                .manifests_for_turn(turn_id)
                .await
                .context("fetching manifests for turn")?;
            print_manifests(&rows);
        }
        if let Some(n) = cli.manifest_last {
            let rows = store
                .recent_manifests(n.max(1))
                .await
                .context("fetching recent manifests")?;
            print_manifests(&rows);
        }
        return Ok(());
    }

    // ── Tier B / T0: bootstrap a fixture from existing manifests ─
    if let Some(n) = cli.eval_bootstrap_from_manifests {
        return bootstrap_eval_fixture(&repo, n, cli.eval_fixture.as_deref()).await;
    }

    // ── Tier B / T0: Tier 1 retrieval smoke test ─────────────────
    if let Some(fixture_path) = cli.eval_fixture.clone() {
        if let Some(n) = cli.eval_from_manifests {
            return run_eval_from_manifests(&repo, &fixture_path, n).await;
        }
        if cli.eval_embedder_ablation {
            return run_eval_embedder_ablation(&repo, &fixture_path).await;
        }
        if cli.eval_rerank_ablation {
            return run_eval_rerank_ablation(&repo, &fixture_path).await;
        }
        if cli.seed_corpus_from_paths {
            return run_seed_corpus_from_paths(
                &repo,
                &fixture_path,
                cli.seed_corpus_doc_chars,
                None,
            )
            .await;
        }
        if cli.eval_scope_matrix {
            return run_eval_scope_matrix(&repo, &fixture_path, &cli.eval_scope_matrix_scopes)
                .await;
        }
        if cli.eval_budget_sweep {
            return run_eval_budget_sweep(&repo, &fixture_path, &cli).await;
        }
        return run_eval_smoke_test(&repo, &fixture_path, &cli).await;
    }

    // ── Tier A / A2: headless `/remember` ────────────────────────
    if let Some(text) = cli.remember.as_deref() {
        return run_remember_cli(&repo, text, &cli.remember_scope).await;
    }

    // ── Tier B / B5: sleeptime hygiene ───────────────────────────
    if cli.sleep {
        return run_sleeptime_cli(&repo, cli.sleep_dry_run).await;
    }

    // ── Tier C / C2.2: deletions list / restore ──────────────────
    if let Some(n) = cli.deletions_last {
        return run_deletions_last_cli(&repo, n.max(1)).await;
    }
    if let Some(audit_id) = cli.restore_id {
        return run_restore_id_cli(&repo, audit_id).await;
    }
    if let Some(window) = cli.restore_since.as_deref() {
        return run_restore_since_cli(&repo, window).await;
    }

    // ── Tier C / C2.4: /forget-history ───────────────────────────
    if let Some(id) = cli.forget_history_id {
        return run_forget_history_cli(
            &repo,
            id,
            cli.redact_confirm.as_deref(),
            cli.redact_reason.as_deref(),
        )
        .await;
    }

    // ── Tier C / C2.3: bulk forget ───────────────────────────────
    {
        use gaviero_core::memory::ForgetFilter;
        use gaviero_core::memory::scope::MemoryType;
        use gaviero_core::memory::trust_defaults::MemorySource;
        let dry_run = cli.forget_dry_run || !cli.forget_yes;
        let reason = cli.forget_reason.as_deref();
        if let Some(q) = cli.forget_query.as_deref() {
            return run_forget_cli(&repo, ForgetFilter::ByQuery(q.to_string()), dry_run, reason)
                .await;
        }
        if let Some(scope_path) = cli.forget_scope.as_deref() {
            let scope_level = if scope_path == "global" {
                gaviero_core::memory::scope::SCOPE_GLOBAL
            } else if scope_path == "workspace" {
                gaviero_core::memory::scope::SCOPE_WORKSPACE
            } else if scope_path.contains("/run:") {
                gaviero_core::memory::scope::SCOPE_RUN
            } else if scope_path.contains("/module:") {
                gaviero_core::memory::scope::SCOPE_MODULE
            } else {
                gaviero_core::memory::scope::SCOPE_REPO
            };
            return run_forget_cli(
                &repo,
                ForgetFilter::ByScope {
                    scope_level,
                    scope_path: scope_path.to_string(),
                },
                dry_run,
                reason,
            )
            .await;
        }
        if let Some(t) = cli.forget_type.as_deref() {
            return run_forget_cli(
                &repo,
                ForgetFilter::ByType(MemoryType::parse_str(&t.to_lowercase())),
                dry_run,
                reason,
            )
            .await;
        }
        if let Some(s) = cli.forget_source.as_deref() {
            return run_forget_cli(
                &repo,
                ForgetFilter::BySource(MemorySource::parse_str(&s.to_lowercase())),
                dry_run,
                reason,
            )
            .await;
        }
    }

    // ── Tier B / B6: per-scope utilization report ────────────────
    if let Some(scope_level) = cli.utilization_scope {
        return run_utilization_cli(
            &repo,
            scope_level,
            cli.utilization_top.max(1),
            cli.utilization_asc,
        )
        .await;
    }

    // ── --cleanup-branches: delete leftover gaviero/* branches and exit ──
    if cli.cleanup_branches {
        let report = gaviero_core::swarm::pipeline::cleanup_gaviero_branches(&repo, !cli.force)
            .context("cleaning up gaviero/* branches")?;

        if report.matched.is_empty() {
            eprintln!("[cleanup] no gaviero/* branches found in {}", repo.display());
            return Ok(());
        }

        if cli.force {
            eprintln!("[cleanup] deleted {} branch(es):", report.deleted.len());
            for b in &report.deleted {
                eprintln!("  - {}", b);
            }
            for b in &report.skipped_current {
                eprintln!("  ! skipped (current branch): {}", b);
            }
        } else {
            eprintln!(
                "[cleanup] would delete {} branch(es) (dry-run; pass --force to execute):",
                report.matched.len() - report.skipped_current.len()
            );
            for b in &report.matched {
                if report.skipped_current.contains(b) {
                    eprintln!("  ! skipped (current branch): {}", b);
                } else {
                    eprintln!("  - {}", b);
                }
            }
        }
        return Ok(());
    }

    // ── --graph: build/update code knowledge graph and exit ──────
    if cli.graph {
        eprintln!(
            "[graph] building code knowledge graph for {}...",
            repo.display()
        );
        let (store, result) =
            gaviero_core::repo_map::graph_builder::build_graph(&repo, &cli.exclude)
                .context("building code knowledge graph")?;
        let (nodes, edges) = store.stats()?;
        eprintln!("[graph] done");
        eprintln!("  files scanned:   {}", result.files_scanned);
        eprintln!("  files changed:   {}", result.files_changed);
        eprintln!("  files unchanged: {}", result.files_unchanged);
        eprintln!("  files removed:   {}", result.files_removed);
        eprintln!("  total nodes:     {}", nodes);
        eprintln!("  total edges:     {}", edges);

        if cli.enrich {
            use gaviero_core::repo_map::symbol_enrichment::{SymbolEnrichOpts, enrich_graph};
            use gaviero_core::workspace::settings;

            let mut workspace = gaviero_core::workspace::Workspace::single_folder(repo.clone());
            workspace.ensure_settings();
            let embedder_setting = workspace
                .resolve_setting(settings::REPO_MAP_EMBEDDER_MODEL, Some(&repo))
                .as_str()
                .map(str::to_string)
                .filter(|s| !s.is_empty() && s != "inherit");
            let embed = !cli.enrich_no_embed;
            let opts = SymbolEnrichOpts {
                embed,
                embedder_name: embedder_setting,
            };
            eprintln!("[graph] enriching symbols (rustdoc JSON)…");
            let enrich_result = enrich_graph(&store, &repo, &opts).await?;
            eprintln!("[graph] enrich done");
            eprintln!("  crates processed: {}", enrich_result.crates_processed);
            eprintln!("  symbols written:  {}", enrich_result.symbols_written);
            eprintln!("  unmatched:        {}", enrich_result.symbols_unmatched);
            eprintln!("  skipped (hash):   {}", enrich_result.symbols_skipped_hash);
            eprintln!(
                "  symbol_docs rows: {}",
                store.symbol_doc_count()?
            );
            if !enrich_result.rustdoc_failures.is_empty() {
                eprintln!("  rustdoc failures:");
                for f in &enrich_result.rustdoc_failures {
                    eprintln!("    - {f}");
                }
            }
        }
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

    // Initialize memory store at `<repo>/.gaviero/memory.db` (graceful if it
    // fails — offline, corrupt model, etc.). The CLI is headless and operates
    // on a single repo argument (no `.gaviero-workspace`), so we wrap the
    // workspace-local single store with `from_single_store` for the registry
    // interface that swarm / pipeline expect. Every other CLI handler (eval,
    // sleep, forget, deletions, remember) already initialises through
    // `init_workspace`; this site previously used `init(None)` which silently
    // routed memory to the global default DB, ignoring `--repo`.
    let memory: Option<Arc<gaviero_core::memory::MemoryStores>> = {
        let repo_for_init = repo.clone();
        match tokio::task::spawn_blocking(move || {
            gaviero_core::memory::init_workspace(&repo_for_init)
        })
        .await
        {
            Ok(Ok(store)) => {
                eprintln!("[memory] ready");
                Some(gaviero_core::memory::MemoryStores::from_single_store(store))
            }
            Ok(Err(e)) => {
                eprintln!("[memory] disabled: {}", e);
                None
            }
            Err(e) => {
                eprintln!("[memory] init panicked: {}", e);
                None
            }
        }
    };

    // Parse work units
    let mut worktree_context_paths = Vec::new();
    let mut plan = if let Some(ref script_path) = cli.script {
        let runtime_prompt = if let Some(ref text) = cli.prompt {
            Some(text.clone())
        } else if let Some(ref p) = cli.prompt_file {
            Some(
                std::fs::read_to_string(p)
                    .with_context(|| format!("reading prompt file: {}", p.display()))?,
            )
        } else {
            None
        };
        let mut override_vars = swarm_workspace
            .override_vars
            .clone()
            .expect("script mode always pre-parses --var overrides");
        materialize_external_vars_for_repo(&repo, &mut override_vars)?;
        worktree_context_paths = collect_cli_worktree_context_paths(
            &repo,
            &cwd,
            &override_vars,
            cli.prompt_file.as_deref(),
        );
        let override_params = parse_param_overrides(&cli.params)?;
        let override_tiers = if let Some(ref tiers_path) = cli.tiers_file {
            let tiers_path = if tiers_path.is_absolute() {
                tiers_path.clone()
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(tiers_path)
            };
            let bindings = gaviero_dsl::load_tier_overrides(&tiers_path).map_err(|report| {
                eprintln!("{:?}", report);
                anyhow::anyhow!("tiers profile failed: {}", tiers_path.display())
            })?;
            eprintln!(
                "[tiers] profile {} ({} binding(s))",
                tiers_path.display(),
                bindings.len()
            );
            bindings
        } else {
            Vec::new()
        };
        // compile_file resolves any `include "..."` directives transitively,
        // so multi-file scripts (shared clients/prompts/etc.) work here.
        gaviero_dsl::compile_file(
            script_path,
            cli.workflow.as_deref(),
            runtime_prompt.as_deref(),
            &override_vars,
            &override_tiers,
            &override_params,
        )
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
            extra_allowed_tools: vec![],
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

    // When a --script is in play, the earlier `[model]` banner is only the
    // fallback for DSL units without a `client`/`tier`. Print the actual
    // per-agent resolution so the user sees what will be dispatched.
    if cli.script.is_some() {
        let ordered = plan
            .work_units_ordered()
            .map_err(|e| anyhow::anyhow!("plan graph error: {}", e))?;
        let name_width = ordered
            .iter()
            .chain(plan.loop_judge_units.iter())
            .map(|u| u.id.len())
            .max()
            .unwrap_or(0);
        let fmt_unit = |u: &WorkUnit| {
            let model = u.model.as_deref().unwrap_or("<fallback>");
            format!(
                "  {:<width$}  {:?}  {}",
                u.id,
                u.tier,
                model,
                width = name_width
            )
        };
        eprintln!("[plan] {} agent(s):", ordered.len());
        for u in &ordered {
            eprintln!("{}", fmt_unit(u));
        }
        if !plan.loop_judge_units.is_empty() {
            eprintln!("[plan] {} loop judge(s):", plan.loop_judge_units.len());
            for u in &plan.loop_judge_units {
                eprintln!("{}", fmt_unit(u));
            }
        }
        for (i, lc) in plan.loop_configs.iter().enumerate() {
            eprintln!(
                "[plan] loop {}  agents=[{}]  max_iterations={}  stability={}  iter_start={}",
                i + 1,
                lc.agent_ids.join(", "),
                lc.max_iterations,
                lc.stability,
                lc.iter_start,
            );
        }
    }

    // MCP: synthesize per-worktree provider configs + optional in-process server.
    let mcp_script_vars = swarm_workspace
        .override_vars
        .as_deref()
        .unwrap_or(&[]);
    let (mcp_config, _mcp_server_guard) =
        prepare_mcp_for_swarm(&repo, &workspace, &cli, mcp_script_vars, &memory)?;
    if let Some(ref synth) = mcp_config {
        gaviero_core::mcp::validate_codex_trust_for_extras(synth, &plan, &execution_model)?;
    }

    // plan.max_parallel overrides the CLI flag when declared.
    let effective_max_parallel = plan.max_parallel.unwrap_or(cli.max_parallel);
    eprintln!("[execution] mode={:?}", plan.execution_mode);
    if plan.execution_mode == gaviero_core::swarm::plan::ExecutionMode::Repo
        && effective_max_parallel > 1
        && cli.max_parallel <= 1
    {
        eprintln!(
            "[parallel] workflow requests max_parallel={effective_max_parallel} \
             (CLI default is 1) — using worktree isolation"
        );
    }
    if !worktree_context_paths.is_empty() {
        eprintln!(
            "[worktree] injecting CLI context file(s): {}",
            worktree_context_paths.join(", ")
        );
    }

    // Execute via swarm pipeline
    let swarm_observer = CliSwarmObserver;
    let specificity = workspace.resolve_specificity_config(Some(&repo));
    let (swarm_extra_tools, _) = workspace.resolve_agent_tools(Some(&repo));
    let config = gaviero_core::swarm::pipeline::SwarmConfig {
        execution_mode: plan.execution_mode,
        max_parallel: effective_max_parallel,
        workspace_root: repo.clone(),
        model: execution_model.clone(),
        ollama_base_url: ollama_base_url.clone(),
        use_worktrees: plan.execution_mode == gaviero_core::swarm::plan::ExecutionMode::Repo
            && effective_max_parallel > 1,
        read_namespaces: read_nss,
        write_namespace: write_ns,
        context_files: vec![],
        worktree_context_paths,
        excludes: cli.exclude.clone(),
        memory_writer: None,
        mcp_config,
        specificity,
        swarm_extra_tools,
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
            |agent_id| {
                Box::new(CliAcpObserver::new(agent_id, true))
                    as Box<dyn gaviero_core::observer::AcpObserver>
            },
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
        |agent_id| {
            let verbose = cli.script.is_none();
            Box::new(CliAcpObserver::new(agent_id, verbose))
                as Box<dyn gaviero_core::observer::AcpObserver>
        },
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
        anyhow::bail!("swarm execution reported failure")
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
    fn cli_accepts_repeated_exclude_flags() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--exclude",
            "node_modules",
            "--exclude",
            "docencia",
            "--exclude",
            "data/**",
        ])
        .unwrap();
        assert_eq!(
            cli.exclude,
            vec![
                "node_modules".to_string(),
                "docencia".to_string(),
                "data/**".to_string(),
            ]
        );
    }

    #[test]
    fn cli_accepts_comma_separated_exclude_values() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--exclude",
            "node_modules,docencia,data/**",
            "--exclude",
            "target,dist",
        ])
        .unwrap();
        assert_eq!(
            cli.exclude,
            vec![
                "node_modules".to_string(),
                "docencia".to_string(),
                "data/**".to_string(),
                "target".to_string(),
                "dist".to_string(),
            ]
        );
    }

    #[test]
    fn cli_accepts_var_with_script() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--script",
            "workflow.gaviero",
            "--var",
            "LOG_DIR=out/log",
            "--var",
            "FOO=bar",
        ])
        .unwrap();
        assert_eq!(
            cli.vars,
            vec!["LOG_DIR=out/log".to_string(), "FOO=bar".to_string()]
        );
    }

    #[test]
    fn parse_var_overrides_splits_on_first_equals() {
        let raw = vec!["KEY=val=ue".to_string(), "FOO=bar".to_string()];
        let pairs = parse_var_overrides(&raw).unwrap();
        assert_eq!(pairs[0], ("KEY".to_string(), "val=ue".to_string()));
        assert_eq!(pairs[1], ("FOO".to_string(), "bar".to_string()));
    }

    #[test]
    fn parse_var_overrides_rejects_missing_equals() {
        let raw = vec!["BADVAR".to_string()];
        assert!(parse_var_overrides(&raw).is_err());
    }

    #[test]
    fn prepare_swarm_workspace_anchors_repo_and_out_dir_to_plan_file() {
        let dir = tempfile::tempdir().unwrap();
        let plan = dir.path().join("draft.md");
        std::fs::write(&plan, "# draft").unwrap();
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--script",
            "workflow.gaviero",
            "--var",
            &format!("PLAN_FILE={}", plan.display()),
        ])
        .unwrap();
        let prep = prepare_swarm_workspace(
            &cli,
            dir.path(),
            gaviero_core::swarm::plan::ExecutionMode::Document,
        )
        .unwrap();
        assert_eq!(
            std::fs::canonicalize(prep.repo_path).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        let vars = prep.override_vars.unwrap();
        assert_eq!(
            vars.iter().find(|(k, _)| k == "OUT_DIR").map(|(_, v)| v.as_str()),
            Some(".")
        );
        assert_eq!(
            vars.iter().find(|(k, _)| k == "PLAN_FILE").map(|(_, v)| v.as_str()),
            Some("draft.md")
        );
    }

    #[test]
    fn cli_accepts_mcp_url_flags() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "x",
            "--mcp-url",
            "semantic-scholar=https://example/mcp",
            "--mcp-codex-trust",
            "granted",
        ])
        .unwrap();
        let overrides = mcp_overrides_from_cli(&cli, &[]).unwrap();
        assert_eq!(overrides.extra_urls.len(), 1);
        assert_eq!(
            overrides.codex_trust,
            Some(gaviero_core::mcp::TrustConsent::Granted)
        );
    }

    #[test]
    fn cli_accepts_prompt_file_with_script() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--script",
            "workflow.gaviero",
            "--prompt-file",
            "prompt.txt",
        ])
        .unwrap();
        assert_eq!(
            cli.prompt_file.as_deref(),
            Some(std::path::Path::new("prompt.txt"))
        );
    }

    #[test]
    fn cli_rejects_prompt_file_without_script() {
        let err = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--prompt-file",
            "prompt.txt",
        ])
        .unwrap_err();
        assert!(err.to_string().contains("--prompt-file"));
    }

    #[test]
    fn cli_accepts_prompt_with_script() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--script",
            "workflow.gaviero",
            "--prompt",
            "Short topic title for logs",
        ])
        .unwrap();
        assert_eq!(cli.prompt.as_deref(), Some("Short topic title for logs"));
        assert!(cli.prompt_file.is_none());
    }

    #[test]
    fn cli_rejects_prompt_without_script() {
        let err = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--prompt",
            "Short topic",
        ])
        .unwrap_err();
        assert!(err.to_string().contains("--prompt"));
    }

    #[test]
    fn cli_rejects_prompt_with_prompt_file() {
        let err = Cli::try_parse_from([
            "gaviero-cli",
            "--script",
            "workflow.gaviero",
            "--prompt",
            "inline",
            "--prompt-file",
            "prompt.txt",
        ])
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("--prompt") && msg.contains("--prompt-file"));
    }

    #[test]
    fn cli_accepts_workflow_with_script() {
        let cli = Cli::try_parse_from([
            "gaviero-cli",
            "--script",
            "workflow.gaviero",
            "--workflow",
            "consensus_review",
        ])
        .unwrap();
        assert_eq!(cli.workflow.as_deref(), Some("consensus_review"));
    }

    #[test]
    fn cli_rejects_workflow_without_script() {
        let err = Cli::try_parse_from([
            "gaviero-cli",
            "--task",
            "fix it",
            "--workflow",
            "consensus_review",
        ])
        .unwrap_err();
        assert!(err.to_string().contains("--workflow"));
    }

    #[test]
    fn ensure_parent_dir_creates_nested_path() {
        let tmp = std::env::temp_dir().join(format!(
            "gaviero-cli-ensure-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let nested = tmp.join("a/b/c/file.json");
        assert!(!tmp.exists());
        ensure_parent_dir(&nested).unwrap();
        assert!(nested.parent().unwrap().is_dir());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn ensure_parent_dir_handles_bare_filename() {
        ensure_parent_dir(std::path::Path::new("foo.json")).unwrap();
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
                "model": "claude:opus"
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
                "model": "claude:haiku",
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

        assert_eq!(execution, "claude:haiku");
        assert_eq!(coordinator, "claude:sonnet");

        let _ = fs::remove_dir_all(root);
    }
}
