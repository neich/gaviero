//! A record of every script run started from the TUI's SWARM panel.
//!
//! One folder per run under `<folder>/.gaviero/swarm-runs/<run-id>/`:
//! `run.json` ([`SwarmRunRecord`]: what was run, with which inputs, how it
//! ended, every agent's final state) and `agents/<agent>.json` (each agent's
//! saved output, in whatever form the host writes it).
//!
//! A run is continued from the pipeline's own checkpoint
//! (`<root>/.gaviero/state/<plan-hash>.json`, [`ExecutionState`]). Every run
//! of the same plan writes that one file, so only the newest record of a plan
//! can be continued — see [`resume_checkpoint`].

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::execution_state::{ExecutionState, NodeStatus};
use super::models::AgentStatus;
use crate::types::ModelTier;

/// Runs kept per folder; older ones are pruned when a run starts.
pub const KEEP_RUNS: usize = 50;

const RECORD_VERSION: u32 = 1;
const RECORD_FILE: &str = "run.json";
const AGENTS_DIR: &str = "agents";

/// How a run ended. `Running` on a record no process is executing means the
/// run was interrupted (the editor closed, the machine slept or crashed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunOutcome {
    Running,
    Completed,
    CompletedWithFailures,
    Failed,
    Stopped,
}

impl RunOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::CompletedWithFailures => "completed with failures",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

/// `--tier-client` / `--tier-effort` for one tier, as the run was given them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTierOverride {
    pub tier: String,
    pub client: Option<String>,
    pub effort: Option<String>,
}

/// Everything the script was compiled with, after the host resolved it
/// (`$skill` expanded into the prompt, file vars located). Compiling the same
/// script with these again gives the same plan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInputs {
    /// The folder chosen for the run, before the script anchored it.
    pub form_root: PathBuf,
    pub workflow: Option<String>,
    pub profile: Option<String>,
    pub prompt: Option<String>,
    pub vars: Vec<(String, String)>,
    pub params: Vec<(String, String)>,
    pub agent_tiers: Vec<(String, String)>,
    pub tier_overrides: Vec<RunTierOverride>,
}

/// One agent's final state in a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunAgentRecord {
    pub id: String,
    pub status: AgentStatus,
    pub detail: String,
    pub branch: Option<String>,
    #[serde(default)]
    pub modified_files: Vec<String>,
    pub tier: Option<ModelTier>,
    pub backend: Option<String>,
    pub elapsed_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwarmRunRecord {
    pub version: u32,
    pub id: String,
    pub script_path: PathBuf,
    pub script_label: String,
    /// The run root: where the pipeline ran and wrote its checkpoint.
    pub root: PathBuf,
    pub plan_hash: String,
    /// Unix seconds.
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub outcome: RunOutcome,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub cost_usd: f64,
    /// Times the run was continued after it stopped.
    #[serde(default)]
    pub resumes: u32,
    /// The run's merges were undone: its checkpoint no longer describes the
    /// tree, so it cannot be continued.
    #[serde(default)]
    pub reverted: bool,
    pub inputs: RunInputs,
    #[serde(default)]
    pub agents: Vec<RunAgentRecord>,
}

impl SwarmRunRecord {
    pub fn new(
        id: String,
        script_path: PathBuf,
        script_label: String,
        root: PathBuf,
        plan_hash: String,
        inputs: RunInputs,
    ) -> Self {
        Self {
            version: RECORD_VERSION,
            id,
            script_path,
            script_label,
            root,
            plan_hash,
            started_at: chrono::Utc::now().timestamp(),
            ended_at: None,
            outcome: RunOutcome::Running,
            message: String::new(),
            cost_usd: 0.0,
            resumes: 0,
            reverted: false,
            inputs,
            agents: Vec::new(),
        }
    }

    /// Agents that completed, out of all recorded.
    pub fn completed_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| matches!(a.status, AgentStatus::Completed))
            .count()
    }
}

/// The run records of one folder.
#[derive(Debug, Clone)]
pub struct RunHistory {
    dir: PathBuf,
}

impl RunHistory {
    /// `<folder>/.gaviero/swarm-runs`.
    pub fn for_folder(folder: &Path) -> Self {
        Self {
            dir: folder.join(".gaviero").join("swarm-runs"),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new run id; ids sort in start order.
    pub fn new_run_id() -> String {
        chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f").to_string()
    }

    fn run_dir(&self, run_id: &str) -> PathBuf {
        self.dir.join(safe_name(run_id))
    }

    fn agent_log_path(&self, run_id: &str, agent: &str) -> PathBuf {
        self.run_dir(run_id)
            .join(AGENTS_DIR)
            .join(format!("{}.json", safe_name(agent)))
    }

    /// Write the record (tmp file + rename, so a crash never leaves half).
    pub fn save(&self, record: &SwarmRunRecord) -> Result<()> {
        let dir = self.run_dir(&record.id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let json = serde_json::to_string_pretty(record)?;
        write_atomic(&dir.join(RECORD_FILE), json.as_bytes())
    }

    /// Every readable record, newest first.
    pub fn list(&self) -> Vec<SwarmRunRecord> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut records: Vec<SwarmRunRecord> = entries
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join(RECORD_FILE)).ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();
        records.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        records
    }

    pub fn write_agent_log(&self, run_id: &str, agent: &str, content: &str) -> Result<()> {
        let path = self.agent_log_path(run_id, agent);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&path, content.as_bytes())
    }

    pub fn read_agent_log(&self, run_id: &str, agent: &str) -> Option<String> {
        std::fs::read_to_string(self.agent_log_path(run_id, agent)).ok()
    }

    /// Delete all but the newest `keep` runs. Returns how many went.
    pub fn prune(&self, keep: usize) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.join(RECORD_FILE).is_file())
            .collect();
        // Ids start with the UTC start time, so name order is start order.
        dirs.sort();
        let excess = dirs.len().saturating_sub(keep);
        dirs.into_iter()
            .take(excess)
            .filter(|dir| std::fs::remove_dir_all(dir).is_ok())
            .count()
    }
}

/// The checkpoint `record` can be continued from, if any.
///
/// None when the run completed or was undone, when a newer record of the
/// same plan in the same root exists (its run wrote over the shared
/// checkpoint), or when the checkpoint is gone or has nothing left to run.
/// `records` is the folder's history; `record` may be one of them.
pub fn resume_checkpoint(
    record: &SwarmRunRecord,
    records: &[SwarmRunRecord],
) -> Option<ExecutionState> {
    if record.outcome == RunOutcome::Completed || record.reverted {
        return None;
    }
    let superseded = records.iter().any(|other| {
        other.id != record.id
            && other.root == record.root
            && other.plan_hash == record.plan_hash
            && (other.started_at, &other.id) > (record.started_at, &record.id)
    });
    if superseded {
        return None;
    }
    let state = ExecutionState::load(&record.root, &record.plan_hash).ok()??;
    state
        .node_states
        .values()
        .any(|node| node.status != NodeStatus::Completed)
        .then_some(state)
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
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
    use crate::swarm::execution_state::NodeState;

    fn record(id: &str, root: &Path, hash: &str, started_at: i64) -> SwarmRunRecord {
        let mut record = SwarmRunRecord::new(
            id.to_string(),
            root.join("s.gaviero"),
            "s".to_string(),
            root.to_path_buf(),
            hash.to_string(),
            RunInputs {
                form_root: root.to_path_buf(),
                prompt: Some("the moon".into()),
                vars: vec![("OUT_DIR".into(), "out".into())],
                ..Default::default()
            },
        );
        record.started_at = started_at;
        record
    }

    fn checkpoint(root: &Path, hash: &str, nodes: &[(&str, NodeStatus)]) {
        let mut state = ExecutionState::default();
        for (id, status) in nodes {
            state.node_states.insert(
                id.to_string(),
                NodeState {
                    status: status.clone(),
                    ..Default::default()
                },
            );
        }
        state.save(root, hash).unwrap();
    }

    #[test]
    fn records_round_trip_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let history = RunHistory::for_folder(dir.path());
        let mut old = record("20260101-000000-000", dir.path(), "h", 10);
        old.agents.push(RunAgentRecord {
            id: "writer".into(),
            status: AgentStatus::Failed("limit".into()),
            detail: "limit".into(),
            branch: Some("gaviero/writer".into()),
            modified_files: vec!["a.md".into()],
            tier: Some(ModelTier::Expensive),
            backend: Some("claude:opus".into()),
            elapsed_secs: Some(3),
        });
        let new = record("20260101-000001-000", dir.path(), "h", 20);
        history.save(&old).unwrap();
        history.save(&new).unwrap();

        let listed = history.list();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, new.id);
        assert_eq!(listed[1], old);
        assert!(
            !history.run_dir(&old.id).join("run.json.tmp").exists(),
            "no tmp file left behind"
        );
    }

    #[test]
    fn agent_logs_round_trip_under_a_safe_name() {
        let dir = tempfile::tempdir().unwrap();
        let history = RunHistory::for_folder(dir.path());
        history.write_agent_log("r1", "map[0]/x", "[1,2]").unwrap();
        assert_eq!(history.read_agent_log("r1", "map[0]/x").as_deref(), Some("[1,2]"));
        assert!(history.dir().join("r1/agents/map_0__x.json").is_file());
        assert_eq!(history.read_agent_log("r1", "other"), None);
    }

    #[test]
    fn unreadable_records_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let history = RunHistory::for_folder(dir.path());
        history.save(&record("good", dir.path(), "h", 1)).unwrap();
        let bad = history.dir().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("run.json"), "{ not json").unwrap();
        let ids: Vec<String> = history.list().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["good".to_string()]);
    }

    #[test]
    fn prune_keeps_the_newest() {
        let dir = tempfile::tempdir().unwrap();
        let history = RunHistory::for_folder(dir.path());
        for i in 0..5 {
            history
                .save(&record(&format!("2026010{i}-000000-000"), dir.path(), "h", i))
                .unwrap();
        }
        assert_eq!(history.prune(3), 2);
        let ids: Vec<String> = history.list().into_iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec!["20260104-000000-000", "20260103-000000-000", "20260102-000000-000"]
        );
        assert_eq!(history.prune(3), 0);
    }

    #[test]
    fn an_unfinished_run_resumes_from_its_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        checkpoint(root, "h", &[("a", NodeStatus::Completed), ("b", NodeStatus::Running)]);
        let run = record("r1", root, "h", 1);
        let state = resume_checkpoint(&run, std::slice::from_ref(&run)).expect("resumable");
        assert_eq!(state.status("a"), NodeStatus::Completed);
        assert_eq!(state.status("b"), NodeStatus::Running);
    }

    #[test]
    fn nothing_to_resume_in_these_cases() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        // No checkpoint.
        let run = record("r1", root, "h", 1);
        assert!(resume_checkpoint(&run, &[]).is_none());

        // Every node completed.
        checkpoint(root, "done", &[("a", NodeStatus::Completed)]);
        let run = record("r2", root, "done", 1);
        assert!(resume_checkpoint(&run, &[]).is_none());

        // The run completed.
        checkpoint(root, "h", &[("a", NodeStatus::HardFailure)]);
        let mut run = record("r3", root, "h", 1);
        run.outcome = RunOutcome::Completed;
        assert!(resume_checkpoint(&run, &[]).is_none());

        // The run was undone.
        let mut run = record("r3b", root, "h", 1);
        run.outcome = RunOutcome::CompletedWithFailures;
        assert!(resume_checkpoint(&run, &[]).is_some());
        run.reverted = true;
        assert!(resume_checkpoint(&run, &[]).is_none());

        // A newer run of the same plan in the same root wrote over the
        // checkpoint; a newer one elsewhere or of another plan does not count.
        let older = record("r4", root, "h", 1);
        let newer = record("r5", root, "h", 2);
        let elsewhere = record("r6", &root.join("sub"), "h", 3);
        let other_plan = record("r7", root, "other", 4);
        assert!(resume_checkpoint(&older, &[older.clone(), newer.clone()]).is_none());
        assert!(resume_checkpoint(&newer, &[older.clone(), newer.clone()]).is_some());
        assert!(
            resume_checkpoint(&older, &[older.clone(), elsewhere, other_plan]).is_some()
        );
    }
}
