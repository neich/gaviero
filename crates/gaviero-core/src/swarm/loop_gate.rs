//! Deterministic loop gates: running them, describing their failures,
//! and skipping the ones that cannot have changed their answer.
//!
//! A `loop { until … }` block can be decided three ways. A judge agent
//! is an LLM call and lives in `pipeline.rs`; the other two —
//! `until command "…"` and `until { compile … test … }` — are
//! deterministic probes, and this module owns them end to end.
//!
//! Two properties matter and neither was true before:
//!
//! * **A broken gate is not a failing gate.** A probe that cannot be
//!   executed at all used to read as "not converged yet" and burn every
//!   remaining iteration in silence. It is now an error.
//! * **A failing gate says why.** Output used to go to `Stdio::null()`,
//!   so the next iteration's agents were told to try again with no idea
//!   what broke. It is now captured, bounded, and fed back.
//!
//! [`ProbeDedup`] then skips a probe whose answer cannot have changed —
//! same command, same workspace, and it failed last time. This is what
//! keeps a ten-iteration loop from running `cargo test` ten times over
//! an unchanged tree.

use anyhow::{Context, Result};

use super::workspace_snapshot::WorkspaceSnapshot;

/// Cap on captured gate output fed back into the next iteration (H-OD1).
///
/// Head-keep: the first lines of a compiler or test failure name the
/// problem; the tail is usually a summary line the agent can re-derive.
pub(crate) const GATE_OUTPUT_CAP: usize = 4000;

/// Placeholder an author can put in a loop agent's prompt to choose
/// where the previous gate failure lands.
pub(crate) const GATE_FEEDBACK_PLACEHOLDER: &str = "{{GATE_FEEDBACK}}";

/// Why a deterministic loop gate refused to pass.
///
/// A failing `until command` / `until { … }` probe used to yield a bare
/// `Continue`, so the next iteration's agents were told to try again
/// with no idea what broke. Carrying the probe, its exit status, and its
/// bounded output turns that into the corrective-feedback shape the
/// `produces` contract already uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GateFailure {
    /// The probe as executed — already `{{ITER}}`-substituted — or a
    /// description of the verification check that failed.
    pub(crate) probe: String,
    /// Human-readable outcome, e.g. `exit status: 1`.
    pub(crate) status: String,
    /// Combined stdout+stderr, already truncated to [`GATE_OUTPUT_CAP`].
    pub(crate) output: String,
}

impl GateFailure {
    /// Render for injection into an agent prompt.
    pub(crate) fn render(&self) -> String {
        let mut out = format!("Gate: `{}`\nResult: {}", self.probe, self.status);
        let trimmed = self.output.trim_end();
        if !trimmed.is_empty() {
            out.push_str("\nOutput:\n```\n");
            out.push_str(trimmed);
            out.push_str("\n```");
        }
        out
    }
}

/// Dedup key standing in for a `Verify` block's command string.
///
/// A verify block has no command to expand and its config is fixed for
/// the life of the loop, so the snapshot alone distinguishes passes.
pub(crate) const VERIFY_DEDUP_KEY: &str = "<verify>";

/// Status line reported when a gate was skipped as pointless.
pub(crate) const UNCHANGED_GATE_STATUS: &str = "not rerun — workspace unchanged since the previous failure. Edit source files, \
     tests, or a workspace artefact before attempting to finish again.";

/// The last failing deterministic gate and the workspace it left behind.
struct ProbeMemo {
    /// The probe as expanded when it failed. Part of the key because
    /// `{{ITER}}`-substituted probes address a *different* target each
    /// iteration — `git show gaviero/foo-iter{{ITER}}:path` inspects a
    /// new branch every pass, so an unchanged workspace says nothing
    /// about whether the probe would still fail.
    command: String,
    /// Workspace state immediately *after* the failing run, so a gate
    /// with side effects (a formatter, a build dir) cannot make the next
    /// pass look changed.
    snapshot: WorkspaceSnapshot,
    /// What the probe reported, replayed when the gate is skipped.
    failure: GateFailure,
}

impl ProbeMemo {
    /// The failure to report in place of a skipped run.
    ///
    /// Keeps the original output: the agent still needs to know what is
    /// failing, and it has not changed since the probe last ran.
    fn skipped(&self) -> GateFailure {
        GateFailure {
            probe: self.failure.probe.clone(),
            status: UNCHANGED_GATE_STATUS.to_string(),
            output: self.failure.output.clone(),
        }
    }
}

/// Skips a deterministic gate when re-running it cannot tell us anything
/// new: same probe, same workspace, and it failed last time.
///
/// In-memory and per pipeline invocation only. Artefact-based resume
/// re-evaluates from disk (`loop_resume.rs`), so there is nothing to
/// persist. Judges get no dedup — their input is the whole conversation,
/// not the filesystem.
#[derive(Default)]
pub(crate) struct ProbeDedup {
    /// One memo per condition slot in an `until … and …` composition.
    ///
    /// Keyed by *position*, not by the dedup key string: two `verify`
    /// blocks in one composition both key on [`VERIFY_DEDUP_KEY`] and
    /// would otherwise alias into a single memo, letting one condition's
    /// result suppress the other's probe.
    per_condition: std::collections::HashMap<usize, ProbeMemo>,
}

impl ProbeDedup {
    /// The memoized failure, when `key` and `snapshot` both match.
    ///
    /// A missing snapshot (capture failed) never matches: "we don't
    /// know" must not be read as "unchanged".
    fn memoized(
        &self,
        index: usize,
        key: &str,
        snapshot: Option<&WorkspaceSnapshot>,
    ) -> Option<&ProbeMemo> {
        let memo = self.per_condition.get(&index)?;
        let snapshot = snapshot?;
        (memo.command == key && &memo.snapshot == snapshot).then_some(memo)
    }

    /// Record a failing probe. Without a snapshot there is nothing to
    /// compare against next pass, so drop the memo entirely rather than
    /// leave one that could match by accident.
    fn remember(
        &mut self,
        index: usize,
        key: String,
        snapshot: Option<WorkspaceSnapshot>,
        failure: GateFailure,
    ) {
        match snapshot {
            Some(snapshot) => {
                self.per_condition.insert(
                    index,
                    ProbeMemo {
                        command: key,
                        snapshot,
                        failure,
                    },
                );
            }
            None => {
                self.per_condition.remove(&index);
            }
        }
    }

    fn clear(&mut self, index: usize) {
        self.per_condition.remove(&index);
    }
}

/// Capture a snapshot for dedup purposes, downgrading failure to `None`.
///
/// A capture error disables dedup for this pass, so the probe runs. That
/// is the safe direction: a needless re-run costs one probe, whereas a
/// wrongly skipped gate hides a real failure.
fn capture_workspace_snapshot(root: &std::path::Path) -> Option<WorkspaceSnapshot> {
    match WorkspaceSnapshot::capture(root) {
        Ok(snapshot) => Some(snapshot),
        Err(e) => {
            tracing::warn!(
                "workspace snapshot of {} failed ({:#}); loop gate dedup disabled for this pass",
                root.display(),
                e
            );
            None
        }
    }
}

/// Run a deterministic gate unless the previous failure still stands.
///
/// `run` is only awaited when the gate actually needs evaluating, which
/// is the point: it may be `cargo test`.
pub(crate) async fn evaluate_deterministic_gate<F, Fut>(
    dedup: &mut ProbeDedup,
    index: usize,
    key: &str,
    workspace_root: &std::path::Path,
    run: F,
) -> Result<Option<GateFailure>>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Option<GateFailure>>>,
{
    let before = capture_workspace_snapshot(workspace_root);

    if let Some(memo) = dedup.memoized(index, key, before.as_ref()) {
        tracing::info!(
            "Loop gate `{}` not rerun: workspace unchanged since the previous failure",
            memo.failure.probe
        );
        return Ok(Some(memo.skipped()));
    }

    match run().await? {
        None => {
            dedup.clear(index);
            Ok(None)
        }
        Some(failure) => {
            let after = capture_workspace_snapshot(workspace_root);
            dedup.remember(index, key.to_string(), after, failure.clone());
            Ok(Some(failure))
        }
    }
}

/// Run one `until command` probe and classify the result.
///
/// `Ok(None)` = the gate passed. `Ok(Some(_))` = it failed, carrying the
/// detail to feed into the next iteration. `Err` = the probe could not be
/// executed at all.
///
/// Note what does *not* produce `Err`: the probe is user-authored shell,
/// run through `sh -c` (or `pwsh -Command`), so a missing binary is the
/// shell exiting 127 — an ordinary failing gate whose "command not
/// found" text now reaches the agent through the captured output. `Err`
/// is reserved for the shell itself being unspawnable, e.g. an
/// unreadable working directory. That case used to be `unwrap_or(false)`
/// and burned every remaining iteration in silence.
pub(crate) async fn run_command_probe(
    expanded: &str,
    workspace_root: &std::path::Path,
) -> Result<Option<GateFailure>> {
    // User-authored probe — POSIX shell when available, pwsh otherwise
    // (Tier W1 / PR-4, W-D5). Output is captured rather than discarded:
    // it is the only evidence the next iteration's agents get.
    let output = crate::util::spawn::shell_command_lenient(expanded)
        .current_dir(workspace_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .with_context(|| {
            format!("loop `until command` probe could not be executed: `{expanded}`")
        })?;

    if output.status.success() {
        return Ok(None);
    }

    let combined = merge_command_output(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    );

    Ok(Some(GateFailure {
        probe: expanded.to_string(),
        status: describe_exit_status(&output.status),
        output: truncate_gate_output(&combined, GATE_OUTPUT_CAP),
    }))
}

/// Human-readable exit status.
///
/// `ExitStatus`'s `Display` differs across platforms; a probe's status
/// ends up in an agent prompt, so it gets a stable phrasing.
pub(crate) fn describe_exit_status(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status: {code}"),
        None => "terminated by signal".to_string(),
    }
}

/// Combine a process's stdout and stderr into one feedback blob.
///
/// Interleaving is lost either way — the two streams are captured
/// separately — so stderr is appended whole, which keeps a compiler's
/// diagnostics contiguous rather than shuffled into its progress output.
pub(crate) fn merge_command_output(stdout: &str, stderr: &str) -> String {
    let mut combined = stdout.to_string();
    if !stderr.trim().is_empty() {
        if !combined.is_empty() && !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push_str(stderr);
    }
    combined
}

/// Keep the first `cap` bytes and say how much was dropped.
///
/// Truncates on a char boundary so the result is always valid UTF-8;
/// mirrors the bounded capture the cargo validation gate performs.
pub(crate) fn truncate_gate_output(raw: &str, cap: usize) -> String {
    if raw.len() <= cap {
        return raw.to_string();
    }
    let mut end = cap;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[… {} chars truncated]", &raw[..end], raw.len() - end)
}

#[cfg(test)]
mod tests {
    use super::*;
    // ── H0 / PR-2: gate failure feedback ─────────────────────────────

    fn gate_failure(probe: &str, output: &str) -> GateFailure {
        GateFailure {
            probe: probe.to_string(),
            status: "exit status: 1".to_string(),
            output: output.to_string(),
        }
    }

    #[test]
    fn truncate_gate_output_keeps_the_head_and_reports_the_drop() {
        let raw = "x".repeat(50);
        let out = truncate_gate_output(&raw, 10);

        assert!(out.starts_with(&"x".repeat(10)));
        assert!(out.contains("[… 40 chars truncated]"));
        assert!(!out.contains(&"x".repeat(11)));
    }

    #[test]
    fn truncate_gate_output_leaves_short_output_alone() {
        assert_eq!(truncate_gate_output("short", 4000), "short");
    }

    #[test]
    fn truncate_gate_output_never_splits_a_multibyte_char() {
        // The cap lands inside the 3-byte '€', so the cut must back off.
        let raw = format!("{}€bbbb", "a".repeat(9));
        let out = truncate_gate_output(&raw, 10);

        assert!(out.starts_with(&"a".repeat(9)));
        assert!(!out.contains('€'));
        assert!(out.contains("[… 7 chars truncated]"));
    }

    #[test]
    fn gate_failure_renders_probe_status_and_output() {
        let rendered = gate_failure("cargo test --quiet", "3 tests failed").render();

        assert!(rendered.contains("cargo test --quiet"));
        assert!(rendered.contains("exit status: 1"));
        assert!(rendered.contains("3 tests failed"));
    }

    #[test]
    fn gate_failure_omits_the_output_block_when_empty() {
        let rendered = gate_failure("cargo check", "").render();

        assert!(rendered.contains("cargo check"));
        assert!(!rendered.contains("Output:"));
    }

    #[tokio::test]
    async fn command_probe_passes_on_exit_zero() {
        let dir = tempfile::tempdir().unwrap();

        let result = run_command_probe("exit 0", dir.path()).await.unwrap();

        assert!(result.is_none(), "exit 0 means the gate passed");
    }

    #[tokio::test]
    async fn command_probe_captures_output_when_the_gate_fails() {
        let dir = tempfile::tempdir().unwrap();

        let failure = run_command_probe("echo gate-diagnostic; exit 3", dir.path())
            .await
            .unwrap()
            .expect("non-zero exit is a failing gate");

        assert_eq!(failure.probe, "echo gate-diagnostic; exit 3");
        assert_eq!(failure.status, "exit status: 3");
        assert!(
            failure.output.contains("gate-diagnostic"),
            "probe output must be captured, got {:?}",
            failure.output
        );
    }

    #[tokio::test]
    async fn command_probe_missing_binary_is_a_failing_gate_not_an_error() {
        // DRIFT from the plan's acceptance wording: probes run through
        // `sh -c` / `pwsh -Command`, so a missing binary is the *shell*
        // exiting non-zero, not a spawn failure. That is the better
        // outcome — "command not found" now reaches the agent as gate
        // feedback instead of aborting the run.
        let dir = tempfile::tempdir().unwrap();

        let result = run_command_probe("/nonexistent-binary-xyz-12345", dir.path()).await;

        let failure = result
            .expect("the shell spawns fine, so this is not a hard error")
            .expect("the shell exits non-zero, so the gate failed");
        assert_ne!(failure.status, "exit status: 0");
    }

    #[tokio::test]
    async fn command_probe_that_cannot_be_spawned_is_a_hard_error() {
        // An unusable working directory stops the shell itself from
        // starting. This used to be `unwrap_or(false)` — an ordinary
        // "not converged", which burned every remaining iteration in
        // silence.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-directory");

        let err = run_command_probe("exit 0", &missing)
            .await
            .expect_err("an unspawnable probe must be a hard error");

        assert!(
            format!("{err:#}").contains("could not be executed"),
            "error must name the failure mode, got {err:#}"
        );
    }

    #[test]
    fn describe_exit_status_reports_the_code() {
        let status = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" })
            .args(if cfg!(windows) {
                vec!["/C", "exit 7"]
            } else {
                vec!["-c", "exit 7"]
            })
            .status()
            .unwrap();

        assert_eq!(describe_exit_status(&status), "exit status: 7");
    }

    // ── H0 / PR-3: deterministic gate dedup ──────────────────────────

    /// Drive one gate evaluation, counting how often the probe actually
    /// ran. The counter lives in the test process, not the workspace, so
    /// it can never perturb the snapshot it is measuring.
    async fn run_gate(
        dedup: &mut ProbeDedup,
        key: &str,
        root: &std::path::Path,
        runs: &mut u32,
        outcome: Option<GateFailure>,
    ) -> Option<GateFailure> {
        run_gate_at(dedup, 0, key, root, runs, outcome).await
    }

    /// `run_gate`, for a specific condition slot in an `All`.
    async fn run_gate_at(
        dedup: &mut ProbeDedup,
        index: usize,
        key: &str,
        root: &std::path::Path,
        runs: &mut u32,
        outcome: Option<GateFailure>,
    ) -> Option<GateFailure> {
        evaluate_deterministic_gate(dedup, index, key, root, || {
            *runs += 1;
            async move { Ok(outcome) }
        })
        .await
        .expect("probe runner does not fail in these tests")
    }

    fn workspace_with_a_file() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("src.rs"), "fn main() {}\n").unwrap();
        dir
    }

    #[tokio::test]
    async fn unchanged_workspace_skips_the_second_probe() {
        let dir = workspace_with_a_file();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("cargo test", "1 test failed");

        let first = run_gate(
            &mut dedup,
            "cargo test",
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        let second = run_gate(
            &mut dedup,
            "cargo test",
            dir.path(),
            &mut runs,
            Some(failure),
        )
        .await;

        assert_eq!(runs, 1, "the probe must run once, not twice");
        assert_eq!(first.unwrap().status, "exit status: 1");

        let second = second.expect("a skipped gate still reports failure");
        assert_eq!(second.status, UNCHANGED_GATE_STATUS);
        assert!(
            second.output.contains("1 test failed"),
            "the original diagnosis is replayed so the agent still sees it"
        );
    }

    #[tokio::test]
    async fn editing_any_workspace_file_reruns_the_probe() {
        let dir = workspace_with_a_file();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("cargo test", "1 test failed");

        run_gate(
            &mut dedup,
            "cargo test",
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        std::fs::write(dir.path().join("src.rs"), "fn main() { changed(); }\n").unwrap();
        run_gate(
            &mut dedup,
            "cargo test",
            dir.path(),
            &mut runs,
            Some(failure),
        )
        .await;

        assert_eq!(runs, 2, "an edited workspace must re-run the gate");
    }

    #[tokio::test]
    async fn an_iteration_aware_probe_reruns_even_when_nothing_changed() {
        // C-2 regression: `git show gaviero/foo-iter{{ITER}}:path`
        // expands differently every pass and inspects a different
        // branch, so an unchanged workspace says nothing about it.
        let dir = workspace_with_a_file();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("git show", "missing");

        run_gate(
            &mut dedup,
            "git show gaviero/foo-iter2:report.md",
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        run_gate(
            &mut dedup,
            "git show gaviero/foo-iter3:report.md",
            dir.path(),
            &mut runs,
            Some(failure),
        )
        .await;

        assert_eq!(runs, 2, "a different expanded probe is a different gate");
    }

    #[tokio::test]
    async fn a_document_mode_artefact_write_reruns_the_probe() {
        // Document mode has no git repo, so the snapshot is files only.
        // An agent whose sole output is an OUT_DIR artefact must still
        // count as progress.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("out")).unwrap();
        std::fs::write(dir.path().join("out/report-v1.md"), "draft\n").unwrap();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("test -f out/report-v2.md", "missing");

        run_gate(
            &mut dedup,
            "test -f out/report-v2.md",
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        std::fs::write(dir.path().join("out/report-v2.md"), "second pass\n").unwrap();
        run_gate(
            &mut dedup,
            "test -f out/report-v2.md",
            dir.path(),
            &mut runs,
            Some(failure),
        )
        .await;

        assert_eq!(runs, 2, "a new OUT_DIR artefact must re-run the gate");
    }

    #[tokio::test]
    async fn a_passing_gate_clears_the_memo() {
        let dir = workspace_with_a_file();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("cargo test", "1 test failed");

        // Fail, then the agent edits something and the gate passes.
        // (An unchanged workspace could never reach the pass — it would
        // be skipped, which is the whole point of the dedup.)
        run_gate(
            &mut dedup,
            "cargo test",
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        std::fs::write(dir.path().join("src.rs"), "fn main() { fixed(); }\n").unwrap();
        run_gate(&mut dedup, "cargo test", dir.path(), &mut runs, None).await;

        assert_eq!(runs, 2);
        assert!(dedup.per_condition.is_empty(), "a pass clears the memo");

        // Nothing changed since the pass, but there is no standing
        // failure to skip on, so the gate is evaluated again.
        run_gate(
            &mut dedup,
            "cargo test",
            dir.path(),
            &mut runs,
            Some(failure),
        )
        .await;
        assert_eq!(runs, 3, "a cleared memo must not suppress the next run");
    }

    #[tokio::test]
    async fn an_unreadable_workspace_never_dedups() {
        // Capture fails, so we cannot know whether anything changed.
        // "Don't know" must run the probe, never skip it.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-directory");
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("cargo test", "1 test failed");

        run_gate(
            &mut dedup,
            "cargo test",
            &missing,
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        run_gate(&mut dedup, "cargo test", &missing, &mut runs, Some(failure)).await;

        assert_eq!(runs, 2, "an unusable snapshot must disable dedup");
        assert!(
            dedup.per_condition.is_empty(),
            "a memo with no snapshot could match by accident later"
        );
    }

    #[tokio::test]
    async fn each_condition_slot_keeps_its_own_memo() {
        // PR-4: two conditions in one `until … and …`. Slot 0 has
        // already failed and its workspace is unchanged, so it is
        // skipped — but slot 1 has never run and must still be probed,
        // even though both would key on the same string if `verify`
        // blocks shared one memo.
        let dir = workspace_with_a_file();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;
        let failure = gate_failure("<verify>", "cargo check failed");

        // Slot 0 fails, then is skipped on the next pass.
        run_gate_at(
            &mut dedup,
            0,
            VERIFY_DEDUP_KEY,
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        let skipped = run_gate_at(
            &mut dedup,
            0,
            VERIFY_DEDUP_KEY,
            dir.path(),
            &mut runs,
            Some(failure.clone()),
        )
        .await;
        assert_eq!(runs, 1, "slot 0's second evaluation must be skipped");
        assert_eq!(skipped.unwrap().status, UNCHANGED_GATE_STATUS);

        // Slot 1 shares the key but not the memo.
        run_gate_at(
            &mut dedup,
            1,
            VERIFY_DEDUP_KEY,
            dir.path(),
            &mut runs,
            Some(failure),
        )
        .await;
        assert_eq!(runs, 2, "slot 1 must run despite slot 0's standing memo");
    }

    #[tokio::test]
    async fn a_gate_with_side_effects_does_not_look_changed() {
        // The probe writes into the workspace (a formatter, a build
        // artefact). Storing the post-run snapshot means that write is
        // already accounted for and does not fake progress.
        let dir = workspace_with_a_file();
        let root = dir.path().to_path_buf();
        let mut dedup = ProbeDedup::default();
        let mut runs = 0;

        for _ in 0..2 {
            let root = root.clone();
            let failure = gate_failure("cargo fmt --check", "reformatted");
            let result =
                evaluate_deterministic_gate(&mut dedup, 0, "cargo fmt --check", &root, || {
                    runs += 1;
                    let root = root.clone();
                    async move {
                        std::fs::write(root.join("side-effect.txt"), "written by the gate\n")
                            .unwrap();
                        Ok(Some(failure))
                    }
                })
                .await
                .unwrap();
            assert!(result.is_some());
        }

        assert_eq!(runs, 1, "the gate's own write must not count as change");
    }
}
