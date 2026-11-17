//! A `gaviero-cli` run, published while it runs so the phone app can show it
//! the way it shows the TUI SWARM panel's run.
//!
//! One folder per run under the machine folder
//! `~/.gaviero/remote/cli-runs/<run-id>/` ([`CliRunStore::machine_default`];
//! `GAVIERO_CLI_RUNS_DIR` overrides it): `run.json` ([`LiveRun`] — the header
//! and one row per agent) and `agents/<agent>.json` (that agent's output
//! lines). The CLI writes it through a [`LiveRunHandle`]: at most once a
//! second when something changed, and at least every [`HEARTBEAT`] while
//! running. A record still `running` whose last write is older than
//! [`STALE_AFTER_SECS`] belongs to a process that went away, and is reported
//! as `interrupted`.
//!
//! Any TUI running the remote listener on the machine serves the folder as
//! `GET /v1/cli-runs` ([`CliRunStore::list_json`]) and
//! `GET /v1/cli-runs/<id>` ([`CliRunStore::detail_json`], the same shape as the
//! `request_swarm` reply). Neither body carries an absolute path.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::models::{AgentStatus, SwarmResult};
use crate::types::ModelTier;

/// Records kept on the machine; older finished ones go when a run starts.
pub const KEEP_CLI_RUNS: usize = 20;
/// A `running` record not written for this long is `interrupted`.
pub const STALE_AFTER_SECS: i64 = 90;
/// A running record is rewritten at least this often, changed or not.
pub const HEARTBEAT: Duration = Duration::from_secs(30);
/// How often the flusher looks for changes.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);
/// Overrides the machine folder (tests, and users who want it elsewhere).
pub const DIR_ENV: &str = "GAVIERO_CLI_RUNS_DIR";

/// Output kept per agent, as the TUI SWARM panel keeps it.
const MAX_ACTIVITY_LINES: usize = 500;
const MAX_ACTIVITY_BYTES: usize = 256 * 1024;
/// Most output bytes one detail reply carries: the newest ones.
const OUTPUT_TAIL_BYTES: usize = 64 * 1024;

const RECORD_VERSION: u32 = 1;
const RECORD_FILE: &str = "run.json";
const AGENTS_DIR: &str = "agents";

// ── Model ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    Text,
    ToolCall,
    Status,
    FileChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiveLine {
    pub kind: LineKind,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveAgent {
    pub id: String,
    pub status: AgentStatus,
    pub detail: String,
    pub tier: Option<ModelTier>,
    pub backend: Option<String>,
    /// Unix milliseconds the current pass started; None before it ran.
    pub started_at_ms: Option<i64>,
    /// Run time, frozen once the agent finished.
    pub elapsed_ms: Option<u64>,
    pub branch: Option<String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
    /// Bumped on every output change; names the saved output.
    pub revision: u64,
    /// Kept in `agents/<id>.json`, not in `run.json`.
    #[serde(skip)]
    pub activity: Vec<LiveLine>,
    #[serde(skip)]
    saw_text: bool,
}

impl LiveAgent {
    fn new(id: String, status: AgentStatus, detail: String) -> Self {
        let started_at_ms = matches!(status, AgentStatus::Running).then(now_ms);
        Self {
            id,
            status,
            detail,
            tier: None,
            backend: None,
            started_at_ms,
            elapsed_ms: None,
            branch: None,
            modified_files: Vec::new(),
            revision: 0,
            activity: Vec::new(),
            saw_text: false,
        }
    }

    fn is_finished(&self) -> bool {
        matches!(self.status, AgentStatus::Completed | AgentStatus::Failed(_))
    }

    fn freeze_elapsed(&mut self) {
        if self.elapsed_ms.is_none() {
            self.elapsed_ms = self
                .started_at_ms
                .map(|start| (now_ms() - start).max(0) as u64);
        }
    }

    fn push_activity(&mut self, kind: LineKind, text: String) {
        self.activity.push(LiveLine { kind, text });
        self.trim_activity();
    }

    fn trim_activity(&mut self) {
        self.revision += 1;
        if self.activity.len() > MAX_ACTIVITY_LINES {
            let excess = self.activity.len() - MAX_ACTIVITY_LINES;
            self.activity.drain(..excess);
        }
        let mut total: usize = self.activity.iter().map(|l| l.text.len()).sum();
        while total > MAX_ACTIVITY_BYTES {
            let over = total - MAX_ACTIVITY_BYTES;
            if self.activity.len() > 1 && self.activity[0].text.len() <= over {
                total -= self.activity.remove(0).text.len();
            } else {
                let first = &mut self.activity[0].text;
                let mut cut = over.min(first.len());
                while !first.is_char_boundary(cut) {
                    cut += 1;
                }
                first.drain(..cut);
                total -= cut;
            }
        }
    }
}

/// An agent known from the compiled plan before anything has run.
#[derive(Debug, Clone)]
pub struct PlannedAgent {
    pub id: String,
    pub tier: ModelTier,
    pub model: Option<String>,
}

/// One CLI run: what `run.json` holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveRun {
    pub version: u32,
    /// Folder name and the id the phone addresses the run by.
    pub id: String,
    /// Numeric id for the reply's `run.run_id` and output tokens: the start
    /// time in Unix milliseconds.
    pub run_id: u64,
    pub script: String,
    /// Name of the run root's folder — never a path.
    pub folder: String,
    pub pid: u32,
    /// Unix seconds.
    pub started_at: i64,
    /// Unix seconds of the last write; the heartbeat.
    pub updated_at: i64,
    pub phase: String,
    pub running: bool,
    pub tier_current: usize,
    pub tier_total: usize,
    pub cost_usd: f64,
    /// Run-wide note: the failure, gate results, loop passes.
    pub message: String,
    /// The agent the output follows: whichever started last, unless the one
    /// followed is still running.
    pub selected: Option<String>,
    pub agents: Vec<LiveAgent>,
}

/// One change to a run, from the pipeline's observers.
#[derive(Debug)]
pub enum LiveUpdate {
    Phase(String),
    Agent {
        id: String,
        status: AgentStatus,
        detail: String,
    },
    Tier {
        current: usize,
        total: usize,
    },
    Dispatch {
        id: String,
        tier: ModelTier,
        backend: String,
    },
    Text {
        id: String,
        text: String,
    },
    Tool {
        id: String,
        summary: String,
    },
    Status {
        id: String,
        status: String,
    },
    File {
        id: String,
        path: String,
        additions: usize,
        deletions: usize,
    },
    /// An agent's whole answer; kept only if it never streamed any text.
    Message {
        id: String,
        content: String,
    },
    Notice(String),
    Cost(f64),
    Completed(Box<SwarmResult>),
    Failed(String),
}

impl LiveRun {
    /// A run that has just started, every planned agent queued.
    pub fn new(id: String, script: String, folder: String, planned: Vec<PlannedAgent>) -> Self {
        let now = now_ms();
        Self {
            version: RECORD_VERSION,
            id,
            run_id: now.max(0) as u64,
            script,
            folder,
            pid: std::process::id(),
            started_at: now / 1000,
            updated_at: now / 1000,
            phase: "starting".to_string(),
            running: true,
            tier_current: 0,
            tier_total: 0,
            cost_usd: 0.0,
            message: String::new(),
            selected: None,
            agents: planned
                .into_iter()
                .map(|p| {
                    let mut agent = LiveAgent::new(p.id, AgentStatus::Pending, "queued".into());
                    agent.tier = Some(p.tier);
                    agent.backend = p.model;
                    agent
                })
                .collect(),
        }
    }

    pub fn apply(&mut self, update: LiveUpdate) {
        match update {
            LiveUpdate::Phase(phase) => self.phase = phase,
            LiveUpdate::Agent { id, status, detail } => self.update_agent(&id, status, detail),
            LiveUpdate::Tier { current, total } => {
                self.tier_current = current;
                self.tier_total = total;
            }
            LiveUpdate::Dispatch { id, tier, backend } => {
                if let Some(agent) = self.agents.iter_mut().find(|a| a.id == id) {
                    agent.tier = Some(tier);
                    agent.backend = Some(backend);
                }
            }
            LiveUpdate::Text { id, text } => {
                let clean = strip_ansi(&text);
                let agent = self.agent_or_insert(&id);
                agent.saw_text = true;
                match agent.activity.last_mut() {
                    Some(last) if last.kind == LineKind::Text => {
                        last.text.push_str(&clean);
                        agent.trim_activity();
                    }
                    _ => agent.push_activity(LineKind::Text, clean),
                }
            }
            LiveUpdate::Tool { id, summary } => {
                let text = if summary.starts_with('[') {
                    summary
                } else {
                    format!("[{summary}]")
                };
                let agent = self.agent_or_insert(&id);
                agent.detail = text.clone();
                agent.push_activity(LineKind::ToolCall, text);
            }
            LiveUpdate::Status { id, status } => self.agent_or_insert(&id).detail = status,
            LiveUpdate::File {
                id,
                path,
                additions,
                deletions,
            } => {
                let line = format!("[wrote {path} +{additions} -{deletions}]");
                let agent = self.agent_or_insert(&id);
                agent.detail = line.clone();
                agent.push_activity(LineKind::FileChange, line);
            }
            LiveUpdate::Message { id, content } => {
                let agent = self.agent_or_insert(&id);
                if !agent.saw_text && !content.trim().is_empty() {
                    agent.saw_text = true;
                    agent.push_activity(LineKind::Text, strip_ansi(&content));
                }
            }
            LiveUpdate::Notice(message) => self.message = message,
            LiveUpdate::Cost(usd) => self.cost_usd = usd,
            LiveUpdate::Completed(result) => self.finish_with_result(&result),
            LiveUpdate::Failed(message) => {
                self.end_run("failed");
                self.message = message;
            }
        }
    }

    fn agent_or_insert(&mut self, id: &str) -> &mut LiveAgent {
        let index = match self.agents.iter().position(|a| a.id == id) {
            Some(index) => index,
            None => {
                self.agents
                    .push(LiveAgent::new(id.to_string(), AgentStatus::Running, String::new()));
                self.agents.len() - 1
            }
        };
        &mut self.agents[index]
    }

    fn update_agent(&mut self, id: &str, status: AgentStatus, detail: String) {
        let agent = self.agent_or_insert(id);
        let changed = std::mem::discriminant(&agent.status) != std::mem::discriminant(&status);
        let now_running = matches!(status, AgentStatus::Running);
        if now_running && (changed || agent.started_at_ms.is_none()) {
            // A loop runs the same agent again: the clock restarts each pass.
            agent.started_at_ms = Some(now_ms());
            agent.elapsed_ms = None;
            agent.saw_text = false;
        }
        if matches!(status, AgentStatus::Completed | AgentStatus::Failed(_)) {
            agent.freeze_elapsed();
        }
        if changed {
            let line = match &status {
                AgentStatus::Pending => format!("Queued: {detail}"),
                AgentStatus::Running => format!("Running: {detail}"),
                AgentStatus::Completed => format!("Completed: {detail}"),
                AgentStatus::Failed(e) => format!("Failed: {e}"),
            };
            agent.push_activity(LineKind::Status, line);
        }
        agent.status = status;
        agent.detail = detail;
        if now_running && changed {
            self.follow(id);
        }
    }

    fn follow(&mut self, id: &str) {
        let followed_running = self.selected.as_deref().is_some_and(|sel| {
            sel != id
                && self
                    .agents
                    .iter()
                    .any(|a| a.id == sel && matches!(a.status, AgentStatus::Running))
        });
        if !followed_running {
            self.selected = Some(id.to_string());
        }
    }

    /// Close out a run that ended without a result.
    fn end_run(&mut self, phase: &str) {
        self.running = false;
        self.phase = phase.to_string();
        for agent in &mut self.agents {
            if !agent.is_finished() {
                agent.freeze_elapsed();
                agent.status = AgentStatus::Failed(phase.to_string());
                agent.detail = phase.to_string();
            }
        }
    }

    fn finish_with_result(&mut self, result: &SwarmResult) {
        self.running = false;
        self.phase = if result.success {
            "completed".to_string()
        } else {
            "completed with failures".to_string()
        };
        for manifest in &result.manifests {
            let Some(agent) = self
                .agents
                .iter_mut()
                .find(|a| a.id == manifest.work_unit_id)
            else {
                continue;
            };
            agent.status = manifest.status.clone();
            agent.freeze_elapsed();
            agent.branch = manifest.branch.clone();
            agent.modified_files = manifest
                .modified_files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let count = agent.modified_files.len();
            agent.detail = if count > 0 {
                format!("Modified {count} file{}", if count == 1 { "" } else { "s" })
            } else {
                manifest
                    .summary
                    .clone()
                    .unwrap_or_else(|| agent.detail.clone())
            };
        }
        // An agent the run never reached has no manifest.
        for agent in &mut self.agents {
            if matches!(agent.status, AgentStatus::Pending | AgentStatus::Running) {
                agent.freeze_elapsed();
                agent.status = AgentStatus::Failed("not run".to_string());
                agent.detail = "not run".to_string();
            }
        }
    }

    /// Still `running`, but its process stopped writing.
    pub fn is_interrupted(&self, now_secs: i64) -> bool {
        self.running && now_secs - self.updated_at > STALE_AFTER_SECS
    }

    /// `running`, `completed`, `completed_with_failures`, `failed` or
    /// `interrupted`.
    pub fn state(&self, now_secs: i64) -> &'static str {
        if self.is_interrupted(now_secs) {
            "interrupted"
        } else if self.running {
            "running"
        } else {
            match self.phase.as_str() {
                "completed" => "completed",
                "completed with failures" => "completed_with_failures",
                _ => "failed",
            }
        }
    }
}

// ── Store ───────────────────────────────────────────────────────

/// The machine folder of CLI run records.
#[derive(Debug, Clone)]
pub struct CliRunStore {
    dir: PathBuf,
}

impl CliRunStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `$GAVIERO_CLI_RUNS_DIR` when set and non-empty, else
    /// `~/.gaviero/remote/cli-runs`. None without a home directory.
    pub fn machine_default() -> Option<Self> {
        if let Some(dir) = std::env::var_os(DIR_ENV).filter(|v| !v.is_empty()) {
            return Some(Self::new(PathBuf::from(dir)));
        }
        crate::workspace::remote_machine_state_dir().map(|dir| Self::new(dir.join("cli-runs")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new run id: start time then pid, so ids sort in start order and two
    /// runs started in the same millisecond still differ.
    pub fn new_run_id() -> String {
        format!(
            "{}-{}",
            chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f"),
            std::process::id()
        )
    }

    /// Ids are only ever ours: letters, digits, `-`, `_`. Anything else (a
    /// path separator, `..`) names no run.
    fn run_dir(&self, id: &str) -> Option<PathBuf> {
        let valid = !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        valid.then(|| self.dir.join(id))
    }

    fn agent_path(run_dir: &Path, agent: &str) -> PathBuf {
        let name: String = agent
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        run_dir.join(AGENTS_DIR).join(format!("{name}.json"))
    }

    /// Write `run.json` and the given agents' output (tmp file + rename).
    pub fn save(&self, run: &LiveRun, outputs: &[(String, Vec<LiveLine>)]) -> Result<()> {
        let dir = self
            .run_dir(&run.id)
            .with_context(|| format!("invalid run id {:?}", run.id))?;
        std::fs::create_dir_all(dir.join(AGENTS_DIR))
            .with_context(|| format!("creating {}", dir.display()))?;
        for (agent, lines) in outputs {
            write_atomic(&Self::agent_path(&dir, agent), &serde_json::to_vec(lines)?)?;
        }
        write_atomic(&dir.join(RECORD_FILE), &serde_json::to_vec_pretty(run)?)
    }

    pub fn load(&self, id: &str) -> Option<LiveRun> {
        let dir = self.run_dir(id)?;
        let text = std::fs::read_to_string(dir.join(RECORD_FILE)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn read_output(&self, id: &str, agent: &str) -> Vec<LiveLine> {
        self.run_dir(id)
            .and_then(|dir| std::fs::read_to_string(Self::agent_path(&dir, agent)).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Every readable record, newest first.
    pub fn list(&self) -> Vec<LiveRun> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut runs: Vec<LiveRun> = entries
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join(RECORD_FILE)).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
        runs.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        runs
    }

    /// Delete all but the newest `keep` records. A run still going (running
    /// and not stale) is never deleted. Returns how many went.
    pub fn prune(&self, keep: usize) -> usize {
        let now = now_secs();
        self.list()
            .into_iter()
            .skip(keep)
            .filter(|run| !run.running || run.is_interrupted(now))
            .filter_map(|run| self.run_dir(&run.id))
            .filter(|dir| std::fs::remove_dir_all(dir).is_ok())
            .count()
    }

    /// `GET /v1/cli-runs`: every record, running ones first, then newest.
    pub fn list_json(&self, host: &str) -> serde_json::Value {
        let now = now_secs();
        let mut runs = self.list();
        // Stable: newest first within each group.
        runs.sort_by_key(|run| run.state(now) != "running");
        let runs: Vec<_> = runs
            .iter()
            .map(|run| {
                let state = run.state(now);
                serde_json::json!({
                    "id": run.id,
                    "script": run.script,
                    "folder": run.folder,
                    "state": state,
                    "phase": if state == "interrupted" { "interrupted" } else { run.phase.as_str() },
                    "running": state == "running",
                    "started_at": rfc3339(run.started_at),
                    "updated_at": rfc3339(run.updated_at),
                    "cost_usd": run.cost_usd,
                    "counts": counts_json(run, state == "interrupted"),
                    "agents": run.agents.len(),
                })
            })
            .collect();
        serde_json::json!({
            "host": host,
            "generated_at": rfc3339(now),
            "runs": runs,
        })
    }

    /// `GET /v1/cli-runs/<id>`: the `request_swarm` reply shape for one run,
    /// plus `id`, `folder`, `state`, `started_at` in `run`. None for an
    /// unknown id.
    pub fn detail_json(
        &self,
        id: &str,
        agent_id: Option<&str>,
        output_token: Option<&str>,
    ) -> Option<serde_json::Value> {
        let run = self.load(id)?;
        let now = now_secs();
        let state = run.state(now);
        let interrupted = state == "interrupted";
        let now_ms = now_ms();
        let agents: Vec<_> = run
            .agents
            .iter()
            .map(|agent| {
                let (status, error) = shown_status(agent, interrupted);
                let elapsed = agent.elapsed_ms.or_else(|| {
                    let start = agent.started_at_ms?;
                    // An interrupted run stopped at its last write.
                    let end = if interrupted { run.updated_at * 1000 } else { now_ms };
                    (matches!(agent.status, AgentStatus::Running)).then(|| (end - start).max(0) as u64)
                });
                serde_json::json!({
                    "id": agent.id,
                    "status": status,
                    "error": error,
                    "detail": if interrupted && status == "failed" && error.as_deref() == Some("interrupted") {
                        "interrupted".to_string()
                    } else {
                        agent.detail.replace(['\n', '\r'], " ")
                    },
                    "tier": agent.tier.map(|t| match t {
                        ModelTier::Cheap => "cheap",
                        ModelTier::Expensive => "expensive",
                    }),
                    "backend": agent.backend,
                    "elapsed_ms": elapsed,
                    "branch": agent.branch,
                    "modified_files": agent.modified_files,
                })
            })
            .collect();
        let run_json = serde_json::json!({
            "id": run.id,
            "run_id": run.run_id,
            "script": run.script,
            "folder": run.folder,
            "state": state,
            "started_at": rfc3339(run.started_at),
            "phase": if interrupted { "interrupted" } else { run.phase.as_str() },
            "running": state == "running",
            "tier_current": run.tier_current,
            "tier_total": run.tier_total,
            "cost_usd": run.cost_usd,
            "message": if interrupted && run.message.is_empty() {
                "gaviero-cli stopped without finishing this run.".to_string()
            } else {
                run.message.clone()
            },
            "counts": counts_json(&run, interrupted),
            "agents": agents,
        });

        let selected = agent_id
            .and_then(|id| run.agents.iter().find(|a| a.id == id))
            .or_else(|| {
                run.selected
                    .as_deref()
                    .and_then(|id| run.agents.iter().find(|a| a.id == id))
            })
            .or_else(|| run.agents.first());
        let output = selected.map(|agent| {
            let token = format!("{}:{}", run.run_id, agent.revision);
            if output_token == Some(token.as_str()) {
                return serde_json::json!({
                    "agent_id": agent.id,
                    "token": token,
                    "unchanged": true,
                });
            }
            let lines = self.read_output(&run.id, &agent.id);
            let (lines, truncated) = output_tail(&lines, OUTPUT_TAIL_BYTES);
            serde_json::json!({
                "agent_id": agent.id,
                "token": token,
                "unchanged": false,
                "lines": lines,
                "truncated": truncated,
            })
        });
        Some(serde_json::json!({
            "run": run_json,
            "selected_id": selected.map(|a| a.id.clone()),
            "output": output,
        }))
    }
}

/// Wire status and error of an agent; an interrupted run's unfinished
/// agents read as failed, `interrupted`.
fn shown_status(agent: &LiveAgent, interrupted: bool) -> (&'static str, Option<String>) {
    match &agent.status {
        AgentStatus::Pending | AgentStatus::Running if interrupted => {
            ("failed", Some("interrupted".to_string()))
        }
        AgentStatus::Pending => ("queued", None),
        AgentStatus::Running => ("running", None),
        AgentStatus::Completed => ("completed", None),
        AgentStatus::Failed(e) => ("failed", Some(e.clone())),
    }
}

fn counts_json(run: &LiveRun, interrupted: bool) -> serde_json::Value {
    let (mut running, mut queued, mut done, mut failed) = (0, 0, 0, 0);
    for agent in &run.agents {
        match shown_status(agent, interrupted).0 {
            "running" => running += 1,
            "queued" => queued += 1,
            "completed" => done += 1,
            _ => failed += 1,
        }
    }
    serde_json::json!({ "running": running, "queued": queued, "done": done, "failed": failed })
}

/// The newest lines that fit in `budget` bytes, oldest first. The oldest
/// kept line loses its head when it does not fit whole.
fn output_tail(lines: &[LiveLine], budget: usize) -> (Vec<serde_json::Value>, bool) {
    let mut out = Vec::new();
    let mut left = budget;
    let mut truncated = false;
    for line in lines.iter().rev() {
        if left == 0 {
            truncated = true;
            break;
        }
        let text = if line.text.len() <= left {
            line.text.as_str()
        } else {
            truncated = true;
            let mut cut = line.text.len() - left;
            while !line.text.is_char_boundary(cut) {
                cut += 1;
            }
            &line.text[cut..]
        };
        left -= text.len();
        out.push(serde_json::json!({ "kind": line.kind, "text": text }));
    }
    out.reverse();
    (out, truncated)
}

// ── Publishing ──────────────────────────────────────────────────

struct Published {
    run: LiveRun,
    dirty: bool,
    /// Output revision last written, per agent.
    saved: HashMap<String, u64>,
    last_write: Instant,
}

/// The publishing side, shared by a run's observers. Updates only touch
/// memory under the lock; files are written outside it.
#[derive(Clone)]
pub struct LiveRunHandle {
    store: CliRunStore,
    state: Arc<Mutex<Published>>,
}

impl LiveRunHandle {
    /// Prune old records and write the first one. A failed write is logged
    /// and later writes are still tried.
    pub fn start(store: CliRunStore, run: LiveRun) -> Self {
        store.prune(KEEP_CLI_RUNS.saturating_sub(1));
        let handle = Self {
            store,
            state: Arc::new(Mutex::new(Published {
                run,
                dirty: true,
                saved: HashMap::new(),
                last_write: Instant::now(),
            })),
        };
        handle.flush(true);
        handle
    }

    pub fn id(&self) -> String {
        self.state.lock().map(|s| s.run.id.clone()).unwrap_or_default()
    }

    pub fn update(&self, update: LiveUpdate) {
        if let Ok(mut state) = self.state.lock() {
            state.run.apply(update);
            state.dirty = true;
        }
    }

    /// Write what changed; with `force`, rewrite the header even when
    /// nothing did (the heartbeat).
    pub fn flush(&self, force: bool) {
        let (run, outputs) = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if !state.dirty && !force {
                return;
            }
            state.dirty = false;
            state.last_write = Instant::now();
            state.run.updated_at = now_secs();
            let mut outputs = Vec::new();
            let mut header = state.run.clone();
            for agent in &mut header.agents {
                let activity = std::mem::take(&mut agent.activity);
                if state.saved.get(&agent.id) != Some(&agent.revision) {
                    outputs.push((agent.id.clone(), activity));
                }
            }
            for (id, _) in &outputs {
                let revision = header
                    .agents
                    .iter()
                    .find(|a| &a.id == id)
                    .map(|a| a.revision)
                    .unwrap_or_default();
                state.saved.insert(id.clone(), revision);
            }
            (header, outputs)
        };
        if let Err(e) = self.store.save(&run, &outputs) {
            tracing::warn!("cli run record {}: {e:#}", run.id);
            if let Ok(mut state) = self.state.lock() {
                // Write everything again next time.
                state.saved.clear();
                state.dirty = true;
            }
        }
    }

    fn heartbeat_due(&self) -> bool {
        self.state
            .lock()
            .map(|s| s.last_write.elapsed() >= HEARTBEAT)
            .unwrap_or(false)
    }

    /// Flush every [`FLUSH_INTERVAL`] until [`LiveRunFlusher::stop`]; the
    /// heartbeat keeps a quiet run from reading as interrupted.
    pub fn spawn_flusher(&self) -> LiveRunFlusher {
        let handle = self.clone();
        let stop = Arc::new(tokio::sync::Notify::new());
        let stopped = stop.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(FLUSH_INTERVAL) => {}
                    _ = stopped.notified() => break,
                }
                // Awaited outside the select: a stop waits for this write.
                let h = handle.clone();
                let force = h.heartbeat_due();
                let _ = tokio::task::spawn_blocking(move || h.flush(force)).await;
            }
        });
        LiveRunFlusher { stop, task }
    }

    /// End the run (`Completed` or `Failed`) and write it at once. Stop the
    /// flusher first, or an older snapshot it is still writing can land
    /// after this one.
    pub fn finish(&self, outcome: LiveUpdate) {
        self.update(outcome);
        self.flush(true);
    }
}

/// The background writer of one run ([`LiveRunHandle::spawn_flusher`]).
pub struct LiveRunFlusher {
    stop: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl LiveRunFlusher {
    /// Stop, waiting for a write in progress, so nothing it holds can
    /// overwrite the final record.
    pub async fn stop(self) {
        // `notify_one` keeps the permit when the task is mid-write.
        self.stop.notify_one();
        let _ = self.task.await;
    }
}

// ── Helpers ─────────────────────────────────────────────────────

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn rfc3339(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Agent output may carry terminal escapes; the phone shows plain text.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            out.push(ch);
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                for c in chars.by_ref() {
                    if c == '\x07' || c == '\x1b' {
                        break;
                    }
                }
            }
            _ => {
                chars.next();
            }
        }
    }
    out
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::models::AgentManifest;

    fn planned(ids: &[&str]) -> Vec<PlannedAgent> {
        ids.iter()
            .map(|id| PlannedAgent {
                id: id.to_string(),
                tier: ModelTier::Cheap,
                model: Some("claude:haiku".into()),
            })
            .collect()
    }

    fn running(id: &str) -> LiveUpdate {
        LiveUpdate::Agent {
            id: id.into(),
            status: AgentStatus::Running,
            detail: "go".into(),
        }
    }

    fn sample_run(id: &str) -> LiveRun {
        LiveRun::new(id.into(), "deep_explore".into(), "proj".into(), planned(&["a", "b"]))
    }

    #[test]
    fn updates_follow_the_dashboard_rules() {
        let mut run = sample_run("r1");
        run.apply(running("a"));
        run.apply(LiveUpdate::Text { id: "a".into(), text: "hel".into() });
        run.apply(LiveUpdate::Text { id: "a".into(), text: "\x1b[1mlo\x1b[0m".into() });
        run.apply(LiveUpdate::Tool { id: "a".into(), summary: "Read x".into() });
        // A whole message after streamed text is not repeated.
        run.apply(LiveUpdate::Message { id: "a".into(), content: "hello".into() });
        let a = &run.agents[0];
        assert_eq!(
            a.activity.iter().map(|l| (l.kind, l.text.as_str())).collect::<Vec<_>>(),
            vec![
                (LineKind::Status, "Running: go"),
                (LineKind::Text, "hello"),
                (LineKind::ToolCall, "[Read x]"),
            ]
        );
        assert_eq!(a.detail, "[Read x]");
        assert_eq!(run.selected.as_deref(), Some("a"));

        // b starting while a still runs does not move the selection.
        run.apply(running("b"));
        assert_eq!(run.selected.as_deref(), Some("a"));

        run.apply(LiveUpdate::Completed(Box::new(SwarmResult {
            manifests: vec![AgentManifest {
                work_unit_id: "a".into(),
                status: AgentStatus::Completed,
                modified_files: vec!["x.rs".into()],
                branch: Some("gaviero/a".into()),
                summary: None,
                output: None,
                cost_usd: 0.0,
            }],
            merge_results: vec![],
            success: false,
            pre_swarm_sha: String::new(),
        })));
        assert!(!run.running);
        assert_eq!(run.phase, "completed with failures");
        assert_eq!(run.agents[0].detail, "Modified 1 file");
        assert_eq!(run.agents[1].status, AgentStatus::Failed("not run".into()));
        assert!(run.agents[0].elapsed_ms.is_some());
    }

    #[test]
    fn a_published_run_round_trips_with_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let store = CliRunStore::new(dir.path());
        let handle = LiveRunHandle::start(store.clone(), sample_run("r1"));
        handle.update(running("a"));
        handle.update(LiveUpdate::Text { id: "a".into(), text: "working".into() });
        handle.flush(false);

        let detail = store.detail_json("r1", None, None).unwrap();
        assert_eq!(detail["run"]["script"], "deep_explore");
        assert_eq!(detail["run"]["folder"], "proj");
        assert_eq!(detail["run"]["state"], "running");
        assert_eq!(detail["run"]["counts"]["running"], 1);
        assert_eq!(detail["selected_id"], "a");
        assert_eq!(detail["output"]["lines"][1]["text"], "working");
        assert_eq!(detail["output"]["lines"][1]["kind"], "text");

        // The same token again: nothing new to send.
        let token = detail["output"]["token"].as_str().unwrap().to_string();
        let again = store.detail_json("r1", None, Some(&token)).unwrap();
        assert_eq!(again["output"]["unchanged"], true);
        assert!(again["output"].get("lines").is_none());

        // Another agent's output on request; an unknown run is None.
        let b = store.detail_json("r1", Some("b"), Some(&token)).unwrap();
        assert_eq!(b["output"]["agent_id"], "b");
        assert_eq!(b["output"]["unchanged"], false);
        assert!(store.detail_json("nope", None, None).is_none());
        assert!(store.detail_json("../r1", None, None).is_none());

        handle.finish(LiveUpdate::Failed("boom".into()));
        let list = store.list_json("desk");
        assert_eq!(list["host"], "desk");
        assert_eq!(list["runs"][0]["id"], "r1");
        assert_eq!(list["runs"][0]["state"], "failed");
        assert_eq!(list["runs"][0]["running"], false);
        let detail = store.detail_json("r1", None, None).unwrap();
        assert_eq!(detail["run"]["message"], "boom");
    }

    #[test]
    fn a_running_record_that_stopped_writing_is_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let store = CliRunStore::new(dir.path());
        let mut run = sample_run("r1");
        run.apply(running("a"));
        run.updated_at = now_secs() - STALE_AFTER_SECS - 5;
        store.save(&run, &[]).unwrap();

        let detail = store.detail_json("r1", None, None).unwrap();
        assert_eq!(detail["run"]["state"], "interrupted");
        assert_eq!(detail["run"]["phase"], "interrupted");
        assert_eq!(detail["run"]["running"], false);
        assert_eq!(detail["run"]["agents"][0]["status"], "failed");
        assert_eq!(detail["run"]["agents"][0]["error"], "interrupted");
        assert_eq!(detail["run"]["counts"]["failed"], 2);
        assert_eq!(store.list_json("h")["runs"][0]["state"], "interrupted");
    }

    #[test]
    fn running_records_list_first_then_newest() {
        let dir = tempfile::tempdir().unwrap();
        let store = CliRunStore::new(dir.path());
        let mut old_running = sample_run("a-old");
        old_running.started_at -= 100;
        let mut new_done = sample_run("b-new");
        new_done.apply(LiveUpdate::Failed("x".into()));
        store.save(&old_running, &[]).unwrap();
        store.save(&new_done, &[]).unwrap();
        let ids: Vec<_> = store.list_json("h")["runs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids, vec!["a-old", "b-new"]);
    }

    #[test]
    fn prune_keeps_the_newest_and_every_live_run() {
        let dir = tempfile::tempdir().unwrap();
        let store = CliRunStore::new(dir.path());
        for i in 0..5 {
            let mut run = sample_run(&format!("r{i}"));
            run.started_at += i;
            if i != 0 {
                run.apply(LiveUpdate::Failed("x".into()));
            }
            store.save(&run, &[]).unwrap();
        }
        // r0 is the oldest but still running: kept.
        assert_eq!(store.prune(2), 2);
        let mut ids: Vec<_> = store.list().into_iter().map(|r| r.id).collect();
        ids.sort();
        assert_eq!(ids, vec!["r0", "r3", "r4"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_flusher_writes_changes_and_stops_before_the_final_write() {
        let dir = tempfile::tempdir().unwrap();
        let store = CliRunStore::new(dir.path());
        let handle = LiveRunHandle::start(store.clone(), sample_run("r1"));
        let flusher = handle.spawn_flusher();
        handle.update(running("a"));
        handle.update(LiveUpdate::Text { id: "a".into(), text: "tick".into() });
        let deadline = Instant::now() + Duration::from_secs(5);
        while store.read_output("r1", "a").len() < 2 {
            assert!(Instant::now() < deadline, "the flusher never wrote");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        flusher.stop().await;
        handle.finish(LiveUpdate::Failed("boom".into()));
        // Nothing is left running that could write an older snapshot.
        tokio::time::sleep(FLUSH_INTERVAL + Duration::from_millis(200)).await;
        let run = store.load("r1").unwrap();
        assert!(!run.running);
        assert_eq!(run.phase, "failed");
    }

    #[test]
    fn output_tail_keeps_the_newest_bytes() {
        let lines = vec![
            LiveLine { kind: LineKind::Text, text: "aaaa".into() },
            LiveLine { kind: LineKind::Status, text: "bbbb".into() },
        ];
        let (tail, truncated) = output_tail(&lines, 6);
        assert!(truncated);
        assert_eq!(tail[0]["text"], "aa");
        assert_eq!(tail[1]["kind"], "status");
    }
}
