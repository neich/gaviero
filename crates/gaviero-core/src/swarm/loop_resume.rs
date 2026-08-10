//! Artefact-based resume for consensus `loop { }` blocks.
//!
//! A refine loop writes one versioned artefact set per reviewer per iteration
//! under `OUT_DIR` (`<id>-refine-plan-v3.md`, `<id>-conclusion-v3.md`, …).
//! When a run is interrupted — or the operator simply wants another few
//! rounds — restarting the CLI against the same `OUT_DIR` used to overwrite
//! `v1` and throw the earlier panel away.
//!
//! This module scans `OUT_DIR`, finds the newest iteration that *every*
//! reviewer completed, and reports the `iter_start` the loop should pick up
//! from. The pipeline applies it before any agent is dispatched, so round
//! `N+1` reads the round-`N` files exactly as an uninterrupted run would.
//!
//! ## What counts as "complete"
//!
//! Reviewers in a roster are clones of one template agent, so their
//! `scope.owned` globs are index-aligned. For a given version the scanner
//! builds a *coverage vector* per reviewer — how many non-empty files matched
//! each owned glob — and calls the iteration complete only when every
//! reviewer in the loop produced an identical, non-empty vector. A round
//! where one provider crashed after writing its plan but before its summary
//! has a shorter vector than its peers and is therefore rejected, along with
//! every artefact above the last complete round.
//!
//! This is deliberately conservative: rewinding one round costs one extra
//! panel pass, whereas resuming from a half-written round feeds some
//! reviewers peer input their peers never saw.
//!
//! ## Checkpoints vs. artefacts
//!
//! [`super::execution_state::ExecutionState`] checkpoints node completion,
//! not loop progress — nothing inside the per-iteration dispatch saves state.
//! Artefacts on disk are the only durable record a refine loop leaves, which
//! is why resume is derived from them rather than from the checkpoint file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::models::WorkUnit;
use super::plan::LoopConfig;

/// Maximum directory depth walked below `OUT_DIR` when collecting artefacts.
const MAX_SCAN_DEPTH: usize = 8;

/// Trailing `-v<N>` version marker, with or without a file extension.
static VERSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-v(\d+)(?:\.[A-Za-z0-9]+)?$").expect("version regex is valid")
});

/// A resume point derived from artefacts already on disk.
#[derive(Debug, Clone)]
pub struct LoopResume {
    /// `OUT_DIR` as declared by the script, relative to the workspace root.
    pub out_dir: String,
    /// The `iter_start` the loop was compiled with.
    pub original_iter_start: u32,
    /// Newest iteration for which every reviewer produced a full artefact set.
    pub last_complete_iter: u32,
    /// `last_complete_iter + 1` — what the loop's `iter_start` becomes.
    pub resume_iter_start: u32,
    /// Reviewer ids that contributed to `last_complete_iter`, sorted.
    pub reviewers: Vec<String>,
    /// Workspace-relative artefacts of `last_complete_iter`, which the
    /// resumed round reads as its peer input.
    pub reused: Vec<String>,
    /// Workspace-relative artefacts above `last_complete_iter`. These belong
    /// to a partially written round and will be overwritten.
    pub discarded: Vec<String>,
    /// Init-template units (`<id>-init`) whose output the reused artefacts
    /// already contain. The pipeline marks these complete so they do not
    /// rewrite the baseline.
    pub satisfied_init_units: Vec<String>,
    /// Human-readable validation findings (empty files, partial rounds).
    pub notes: Vec<String>,
}

impl LoopResume {
    /// One-line summary suitable for a log line or CLI banner.
    pub fn summary(&self) -> String {
        format!(
            "resuming {} at iteration {} (last complete: v{} across {} reviewer(s))",
            self.out_dir,
            self.resume_iter_start,
            self.last_complete_iter,
            self.reviewers.len()
        )
    }
}

/// One reviewer's artefacts at one version.
#[derive(Debug, Default, Clone)]
struct Coverage {
    /// Non-empty file count per owned-glob index.
    counts: Vec<usize>,
    /// Workspace-relative paths that produced those counts.
    files: Vec<String>,
}

impl Coverage {
    fn total(&self) -> usize {
        self.counts.iter().sum()
    }
}

/// Scan `OUT_DIR` for a resume point for `loop_config`.
///
/// Returns `None` when there is nothing to resume from: no `OUT_DIR`, no
/// artefacts, no iteration completed by the whole panel, or a last complete
/// iteration that does not advance past the configured `iter_start`.
pub fn detect(
    workspace_root: &Path,
    loop_config: &LoopConfig,
    unit_map: &std::collections::HashMap<&str, &WorkUnit>,
) -> Option<LoopResume> {
    let out_dir = loop_config.verdict_output_dir.as_deref()?;
    if loop_config.agent_ids.is_empty() {
        return None;
    }

    let out_root = workspace_root.join(out_dir);
    if !out_root.is_dir() {
        return None;
    }

    // Owned globs per loop agent. An agent with no owned paths cannot be
    // validated, so the whole loop opts out rather than resuming blind.
    let mut owned: BTreeMap<&str, &[String]> = BTreeMap::new();
    for agent_id in &loop_config.agent_ids {
        let unit = unit_map.get(agent_id.as_str())?;
        if unit.scope.owned_paths.is_empty() {
            return None;
        }
        owned.insert(agent_id.as_str(), unit.scope.owned_paths.as_slice());
    }

    let mut notes: Vec<String> = Vec::new();
    let files = collect_files(&out_root, workspace_root, &mut notes);
    if files.is_empty() {
        return None;
    }

    // version → agent id → coverage
    let mut by_version: BTreeMap<u32, BTreeMap<&str, Coverage>> = BTreeMap::new();

    for (rel, len) in &files {
        let Some(version) = parse_version(rel) else {
            continue;
        };
        for (agent_id, globs) in &owned {
            let Some(idx) = globs
                .iter()
                .position(|g| crate::path_pattern::matches(g, rel))
            else {
                continue;
            };
            let entry = by_version
                .entry(version)
                .or_default()
                .entry(agent_id)
                .or_default();
            if entry.counts.len() < globs.len() {
                entry.counts.resize(globs.len(), 0);
            }
            if *len == 0 {
                notes.push(format!("{rel} is empty — iteration v{version} treated as partial"));
            } else {
                entry.counts[idx] += 1;
                entry.files.push(rel.clone());
            }
            // A file is attributed to the first agent whose glob claims it;
            // scope validation already guarantees the panel's owned paths are
            // disjoint, so this only guards against pathological scripts.
            break;
        }
    }

    // Newest version where every reviewer has an identical, non-empty vector.
    let mut last_complete: Option<u32> = None;
    for (version, per_agent) in &by_version {
        if per_agent.len() != loop_config.agent_ids.len() {
            let missing: Vec<&str> = loop_config
                .agent_ids
                .iter()
                .map(String::as_str)
                .filter(|id| !per_agent.contains_key(id))
                .collect();
            notes.push(format!(
                "v{version} incomplete — no artefacts from {}",
                missing.join(", ")
            ));
            continue;
        }
        let mut vectors = per_agent.values().map(|c| &c.counts);
        let Some(first) = vectors.next() else { continue };
        if first.iter().sum::<usize>() == 0 {
            continue;
        }
        if vectors.all(|v| v == first) {
            last_complete = Some(*version);
        } else {
            let detail: Vec<String> = per_agent
                .iter()
                .map(|(id, c)| format!("{id}={}", c.total()))
                .collect();
            notes.push(format!(
                "v{version} incomplete — reviewers produced different artefact sets ({})",
                detail.join(", ")
            ));
        }
    }

    let last_complete = last_complete?;
    let resume_iter_start = last_complete.checked_add(1)?;
    if resume_iter_start <= loop_config.iter_start {
        // Nothing on disk that the configured start would not already skip.
        return None;
    }

    let mut reused: Vec<String> = by_version
        .get(&last_complete)
        .map(|per_agent| {
            per_agent
                .values()
                .flat_map(|c| c.files.iter().cloned())
                .collect()
        })
        .unwrap_or_default();
    reused.sort();

    let mut discarded: Vec<String> = by_version
        .range((last_complete + 1)..)
        .flat_map(|(_, per_agent)| per_agent.values().flat_map(|c| c.files.iter().cloned()))
        .collect();
    discarded.sort();

    let mut reviewers: Vec<String> = by_version
        .get(&last_complete)
        .map(|per_agent| per_agent.keys().map(|id| id.to_string()).collect())
        .unwrap_or_default();
    reviewers.sort();

    // Roster loops name their baseline units `<id>-init` and their loop body
    // `<id>-refine` (see `swarm::validation::expand_loop_groups_with_roster_init`).
    // Resuming past the baseline means those units must not run again.
    let satisfied_init_units: Vec<String> = loop_config
        .agent_ids
        .iter()
        .filter_map(|id| id.strip_suffix("-refine"))
        .map(|prefix| format!("{prefix}-init"))
        .filter(|init_id| unit_map.contains_key(init_id.as_str()))
        .collect();

    Some(LoopResume {
        out_dir: out_dir.to_string(),
        original_iter_start: loop_config.iter_start,
        last_complete_iter: last_complete,
        resume_iter_start,
        reviewers,
        reused,
        discarded,
        satisfied_init_units,
        notes,
    })
}

/// Parse the trailing `-v<N>` marker from a path's file name.
fn parse_version(rel_path: &str) -> Option<u32> {
    let name = rel_path.rsplit('/').next()?;
    VERSION_RE
        .captures(name)?
        .get(1)?
        .as_str()
        .parse::<u32>()
        .ok()
}

/// Collect `(workspace-relative path, byte length)` for every file under
/// `root`, to a bounded depth.
fn collect_files(
    root: &Path,
    workspace_root: &Path,
    notes: &mut Vec<String>,
) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_SCAN_DEPTH {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                notes.push(format!("could not read {}: {e}", dir.display()));
                continue;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push((path, depth + 1));
            } else if meta.is_file()
                && let Some(rel) = rel_path(workspace_root, &path)
            {
                out.push((rel, meta.len()));
            }
        }
    }
    out
}

/// Workspace-relative path with `/` separators, as the glob matcher expects.
fn rel_path(workspace_root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(workspace_root).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::plan::LoopUntilCondition;
    use std::collections::HashMap;

    fn loop_config(agent_ids: &[&str], out_dir: &str, iter_start: u32) -> LoopConfig {
        LoopConfig {
            agent_ids: agent_ids.iter().map(|s| s.to_string()).collect(),
            until: LoopUntilCondition::Agent("judge".into()),
            max_iterations: 5,
            iter_start,
            strict_judge: false,
            stability: 2,
            judge_timeout_secs: 180,
            branch_chain: Default::default(),
            consensus_mode: Default::default(),
            verdict_output_dir: Some(out_dir.to_string()),
            irreconcilable_after: 2,
        }
    }

    /// Every `WorkUnit` field but `id` has a serde default, so a fixture is
    /// far shorter (and less brittle) built through `Deserialize` than by
    /// enumerating ~28 fields.
    fn unit(id: &str, owned: &[&str]) -> WorkUnit {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "scope": { "owned_paths": owned },
        }))
        .expect("work unit fixture")
    }

    fn write(root: &Path, rel: &str, body: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    /// The `plan_refinement.gaviero` shape: one owned glob per reviewer that
    /// covers both the plan and the summary.
    fn plan_refinement_fixture(root: &Path, versions: &[u32], reviewers: &[&str]) {
        for v in versions {
            for r in reviewers {
                write(root, &format!("plans/x/{r}-refine-plan-v{v}.md"), "plan");
                write(root, &format!("plans/x/{r}-refine-summary-v{v}.md"), "sum");
            }
        }
    }

    fn plan_refinement_units() -> Vec<WorkUnit> {
        ["claude", "codex", "cursor"]
            .iter()
            .map(|r| {
                unit(
                    &format!("{r}-refine"),
                    &[&format!("plans/x/{r}-refine-*.md")],
                )
            })
            .collect()
    }

    fn map(units: &[WorkUnit]) -> HashMap<&str, &WorkUnit> {
        units.iter().map(|u| (u.id.as_str(), u)).collect()
    }

    #[test]
    fn resumes_after_last_fully_complete_iteration() {
        let tmp = tempfile::tempdir().unwrap();
        plan_refinement_fixture(tmp.path(), &[1, 2, 3], &["claude", "codex", "cursor"]);
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            1,
        );

        let resume = detect(tmp.path(), &lc, &map(&units)).expect("resume point");
        assert_eq!(resume.last_complete_iter, 3);
        assert_eq!(resume.resume_iter_start, 4);
        assert_eq!(resume.reviewers.len(), 3);
        assert_eq!(resume.reused.len(), 6, "3 reviewers x plan+summary");
        assert!(resume.discarded.is_empty());
    }

    #[test]
    fn rewinds_past_a_partially_written_iteration() {
        let tmp = tempfile::tempdir().unwrap();
        plan_refinement_fixture(tmp.path(), &[1, 2, 3], &["claude", "codex", "cursor"]);
        // v4: claude finished, codex wrote only its plan, cursor never ran.
        write(tmp.path(), "plans/x/claude-refine-plan-v4.md", "plan");
        write(tmp.path(), "plans/x/claude-refine-summary-v4.md", "sum");
        write(tmp.path(), "plans/x/codex-refine-plan-v4.md", "plan");
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            1,
        );

        let resume = detect(tmp.path(), &lc, &map(&units)).expect("resume point");
        assert_eq!(resume.last_complete_iter, 3);
        assert_eq!(resume.resume_iter_start, 4);
        assert_eq!(resume.discarded.len(), 3, "the partial v4 files");
        assert!(resume.discarded.iter().all(|p| p.contains("-v4.md")));
    }

    #[test]
    fn an_empty_artefact_makes_its_iteration_partial() {
        let tmp = tempfile::tempdir().unwrap();
        plan_refinement_fixture(tmp.path(), &[1, 2], &["claude", "codex", "cursor"]);
        plan_refinement_fixture(tmp.path(), &[3], &["claude", "codex", "cursor"]);
        // Truncate one v3 artefact — the round is not trustworthy.
        write(tmp.path(), "plans/x/cursor-refine-summary-v3.md", "");
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            1,
        );

        let resume = detect(tmp.path(), &lc, &map(&units)).expect("resume point");
        assert_eq!(resume.last_complete_iter, 2);
        assert_eq!(resume.resume_iter_start, 3);
        assert!(resume.notes.iter().any(|n| n.contains("empty")));
    }

    /// The `scientific_research.gaviero` shape: three owned globs, where the
    /// baseline round legitimately has no summary.
    #[test]
    fn handles_init_round_without_a_summary_artefact() {
        let tmp = tempfile::tempdir().unwrap();
        for r in ["claude", "codex"] {
            write(tmp.path(), &format!("research/{r}-conclusion-v1.md"), "c");
            write(tmp.path(), &format!("research/{r}-evidence-v1.md"), "e");
            for v in [2, 3] {
                write(tmp.path(), &format!("research/{r}-conclusion-v{v}.md"), "c");
                write(tmp.path(), &format!("research/{r}-summary-v{v}.md"), "s");
                write(tmp.path(), &format!("research/{r}-evidence-v{v}.md"), "e");
            }
        }
        let units: Vec<WorkUnit> = ["claude", "codex"]
            .iter()
            .flat_map(|r| {
                [
                    unit(
                        &format!("{r}-refine"),
                        &[
                            &format!("research/{r}-conclusion-v*.md"),
                            &format!("research/{r}-summary-v*.md"),
                            &format!("research/{r}-evidence-v*.md"),
                        ],
                    ),
                    unit(&format!("{r}-init"), &[&format!("research/{r}-conclusion-v*.md")]),
                ]
            })
            .collect();
        let lc = loop_config(&["claude-refine", "codex-refine"], "research", 2);

        let resume = detect(tmp.path(), &lc, &map(&units)).expect("resume point");
        assert_eq!(resume.last_complete_iter, 3);
        assert_eq!(resume.resume_iter_start, 4);
        assert_eq!(
            resume.satisfied_init_units.len(),
            2,
            "both baseline units are already represented on disk"
        );
    }

    #[test]
    fn no_resume_when_out_dir_is_empty_or_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            1,
        );
        assert!(detect(tmp.path(), &lc, &map(&units)).is_none(), "missing dir");

        std::fs::create_dir_all(tmp.path().join("plans/x")).unwrap();
        assert!(detect(tmp.path(), &lc, &map(&units)).is_none(), "empty dir");
    }

    #[test]
    fn no_resume_when_the_first_round_never_completed() {
        let tmp = tempfile::tempdir().unwrap();
        // Only one of three reviewers produced anything.
        write(tmp.path(), "plans/x/claude-refine-plan-v1.md", "plan");
        write(tmp.path(), "plans/x/claude-refine-summary-v1.md", "sum");
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            1,
        );

        assert!(detect(tmp.path(), &lc, &map(&units)).is_none());
    }

    #[test]
    fn no_resume_when_disk_does_not_advance_past_iter_start() {
        let tmp = tempfile::tempdir().unwrap();
        // A script with `iter_start 2` whose only artefacts are the v1 baseline.
        for r in ["claude", "codex", "cursor"] {
            write(tmp.path(), &format!("plans/x/{r}-refine-plan-v1.md"), "p");
            write(tmp.path(), &format!("plans/x/{r}-refine-summary-v1.md"), "s");
        }
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            2,
        );

        assert!(detect(tmp.path(), &lc, &map(&units)).is_none());
    }

    #[test]
    fn unversioned_files_in_out_dir_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        plan_refinement_fixture(tmp.path(), &[1, 2], &["claude", "codex", "cursor"]);
        write(tmp.path(), "plans/x/plan_log.md", "notes");
        write(tmp.path(), "plans/x/consensus-verdict.json", "{}");
        let units = plan_refinement_units();
        let lc = loop_config(
            &["claude-refine", "codex-refine", "cursor-refine"],
            "plans/x",
            1,
        );

        let resume = detect(tmp.path(), &lc, &map(&units)).expect("resume point");
        assert_eq!(resume.last_complete_iter, 2);
        assert!(resume.reused.iter().all(|p| p.contains("-v2.md")));
    }

    #[test]
    fn parse_version_reads_the_trailing_marker_only() {
        assert_eq!(parse_version("plans/x/claude-refine-plan-v3.md"), Some(3));
        assert_eq!(parse_version("plans/v2/claude-conclusion-v11.md"), Some(11));
        assert_eq!(parse_version("plans/x/plan_log.md"), None);
        assert_eq!(parse_version("plans/x/claude-v2-notes.md"), None);
    }
}
