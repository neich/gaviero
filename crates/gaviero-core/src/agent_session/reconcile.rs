//! Route writes an agent made *outside* the Write Gate through it.
//!
//! Two of gaviero's session patterns hand the model a file-writing surface the
//! host cannot intercept *before* the bytes land:
//!
//! * **Pattern C** (`deepseek:`) runs the tool loop in-process, but its
//!   `Write`/`Edit`/`MultiEdit` tools commit with a plain `tokio::fs::write`
//!   (`tool_agent/tools/write.rs`). That is deliberate — the model's own next
//!   `Read`/`Edit` must see what it just wrote, or a multi-edit turn breaks on
//!   its own second call — so the host only learns about the change afterwards,
//!   from the turn's [`TurnSnapshot`](super::tool_agent::snapshot::TurnSnapshot).
//! * **Pattern D** (`dsh:`) hands the entire loop to a child process that, in
//!   practice, never calls the ACP client `fs` channel at all
//!   (`agent_client_protocol/mod.rs`).
//!
//! Both therefore produce the same user-visible problem: a long turn mutates the
//! tree silently, file by file, with no accept/reject decision anywhere. This
//! module is the single answer to that, so the two patterns cannot drift into
//! two different review models — and so that "what a DeepSeek turn looks like"
//! does not depend on which of the two prefixes the user typed. Each written
//! path is:
//!
//! 1. paired with its **turn-start baseline** — the caller supplies this, from
//!    whichever source it actually has (Pattern C: the turn snapshot; Pattern D:
//!    git plus the pre-turn dirty set);
//! 2. **restored** to that baseline, so the agent's bytes leave the tree;
//! 3. **re-submitted** as a `WriteProposal` carrying the real diff.
//!
//! Afterwards the *gate* owns the change, which is what makes the mode
//! meaningful: `Deferred` accumulates it for batch review and `Interactive` pops
//! a per-file modal, both leaving the tree at its turn-start state until a human
//! accepts; `RejectAll` refuses it outright with no disk write. A path the gate
//! refuses on its own authority (sensitive, out of scope) stays restored — the
//! refusal *is* the rejection.
//!
//! [`AutoAccept`](WriteMode::AutoAccept) is the one mode where the restore is
//! skipped, because the gate's answer is already known to be "write the agent's
//! bytes": the round trip could only lose information (the write-back goes
//! through [`assemble_final_content`](crate::write_gate::assemble_final_content),
//! whose trailing newline follows `original_content` rather than the agent's
//! text) while reviewing nothing. Those paths are reported as still carrying the
//! agent's bytes so the host still resyncs its buffers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use crate::acp::client::{propose_delete, propose_write};
use crate::observer::AcpObserver;
use crate::write_gate::{WriteGatePipeline, WriteMode};

/// Largest file the reconciler will read back for a diff. Beyond this the
/// content is treated as unreviewable and the path is left where the agent put
/// it (and reported as modified, exactly as before this pass existed).
pub const RECONCILE_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// One path an agent wrote during a turn, with its turn-start content.
#[derive(Debug, Clone)]
pub struct DirectWrite {
    /// Workspace-relative path.
    pub rel_path: PathBuf,
    /// Turn-start content; `None` = the path did not exist then.
    pub before: Option<String>,
}

impl DirectWrite {
    pub fn new(rel_path: impl Into<PathBuf>, before: Option<String>) -> Self {
        Self {
            rel_path: rel_path.into(),
            before,
        }
    }
}

impl From<(PathBuf, Option<String>)> for DirectWrite {
    fn from((rel_path, before): (PathBuf, Option<String>)) -> Self {
        Self { rel_path, before }
    }
}

/// What reconciliation did with a turn's out-of-band writes.
#[derive(Debug, Default)]
pub struct ReconcileOutcome {
    /// Paths the gate now owns: restored to their turn-start content and
    /// re-submitted as a proposal.
    pub proposed: Vec<PathBuf>,
    /// Paths still carrying the agent's bytes — because the effective mode is
    /// `AutoAccept`, because the content could not be diffed, or because a
    /// concurrent writer moved underneath the restore.
    pub left_in_place: Vec<PathBuf>,
}

/// Read a file as UTF-8 text, refusing the cases the reconciler cannot diff: a
/// missing path (`Ok(None)`), a non-file, a file over [`RECONCILE_MAX_FILE_BYTES`],
/// or non-UTF-8 bytes.
pub async fn read_text_capped(path: &Path) -> Result<Option<String>> {
    let meta = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("stat of {}", path.display())),
    };
    if !meta.is_file() {
        anyhow::bail!("{} is not a regular file", path.display());
    }
    if meta.len() > RECONCILE_MAX_FILE_BYTES {
        anyhow::bail!(
            "{} is {} bytes, over the {RECONCILE_MAX_FILE_BYTES}-byte review limit",
            path.display(),
            meta.len()
        );
    }
    let bytes = tokio::fs::read(path)
        .await
        .with_context(|| format!("read of {}", path.display()))?;
    let text =
        String::from_utf8(bytes).with_context(|| format!("{} is not UTF-8", path.display()))?;
    Ok(Some(text))
}

/// Restore `path` to `before`: its content, or absence.
pub async fn restore_path(path: &Path, before: Option<&str>) -> Result<()> {
    match before {
        Some(content) => tokio::fs::write(path, content)
            .await
            .with_context(|| format!("restoring {}", path.display())),
        None => match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        },
    }
}

/// Put every [`DirectWrite`] under gate control. See the module docs.
///
/// Never fails: a path that cannot be reconciled is left where the agent put it
/// and reported through [`ReconcileOutcome::left_in_place`], which is what the
/// caller forwards to its host as "these are on disk". A review surface that
/// silently dropped a path would be worse than one that shows an unreviewed
/// change, so the failure mode is always "leave it, report it".
pub async fn reconcile_direct_writes(
    write_gate: &Arc<Mutex<WriteGatePipeline>>,
    observer: &dyn AcpObserver,
    workspace_root: &Path,
    agent_id: &str,
    conv_id: Option<&str>,
    writes: Vec<DirectWrite>,
) -> ReconcileOutcome {
    let mut outcome = ReconcileOutcome::default();

    let mut ordered = writes;
    ordered.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    ordered.dedup_by(|a, b| a.rel_path == b.rel_path);

    // `AutoAccept`: the gate would accept every hunk and write the content
    // straight back, so the restore→re-propose round trip reviews nothing and
    // can only lose bytes. Report the paths instead and touch nothing.
    if *write_gate.lock().await.effective_mode_for_conv(conv_id) == WriteMode::AutoAccept {
        outcome.left_in_place = ordered.into_iter().map(|w| w.rel_path).collect();
        return outcome;
    }

    for DirectWrite {
        rel_path: rel,
        before,
    } in ordered
    {
        let abs = workspace_root.join(&rel);

        let current = match read_text_capped(&abs).await {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    path = %rel.display(),
                    "an out-of-band write cannot be diffed ({e:#}); leaving it in place"
                );
                outcome.left_in_place.push(rel);
                continue;
            }
        };

        // No net change — the agent rewrote identical bytes, or created and
        // removed the path. Nothing to propose and nothing to restore.
        if current == before {
            continue;
        }

        // Drift guard, mirroring the Codex finalizer: if the bytes moved between
        // the read and the restore, a concurrent writer (an editor save) is
        // involved and restoring would clobber it.
        match read_text_capped(&abs).await {
            Ok(again) if again == current => {}
            _ => {
                tracing::warn!(
                    path = %rel.display(),
                    "an out-of-band write drifted while it was being reconciled; \
                     leaving it in place"
                );
                outcome.left_in_place.push(rel);
                continue;
            }
        }

        if let Err(e) = restore_path(&abs, before.as_deref()).await {
            tracing::warn!(
                path = %rel.display(),
                "could not restore an out-of-band write before review: {e:#}"
            );
            outcome.left_in_place.push(rel);
            continue;
        }

        let result = match (&current, &before) {
            (Some(proposed), _) => {
                propose_write(
                    write_gate,
                    observer,
                    workspace_root,
                    agent_id,
                    conv_id,
                    &rel,
                    proposed,
                )
                .await
            }
            (None, Some(prior)) => {
                propose_delete(
                    write_gate,
                    observer,
                    workspace_root,
                    agent_id,
                    conv_id,
                    &rel,
                    prior,
                )
                .await
            }
            (None, None) => Ok(()),
        };
        match result {
            Ok(()) => outcome.proposed.push(rel),
            Err(e) => {
                tracing::warn!(
                    "failed to propose an out-of-band write to {}: {e:#}",
                    rel.display()
                );
                outcome.left_in_place.push(rel);
            }
        }
    }

    tracing::info!(
        proposed = outcome.proposed.len(),
        left_in_place = outcome.left_in_place.len(),
        "routed out-of-band writes through the write gate"
    );

    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::WriteProposal;

    /// The gate needs an observer at construction; these tests assert on the
    /// proposals themselves, not on gate lifecycle callbacks.
    struct NoopGateObserver;

    impl crate::observer::WriteGateObserver for NoopGateObserver {
        fn on_proposal_created(&self, _proposal: &WriteProposal) {}
        fn on_proposal_updated(&self, _proposal_id: u64) {}
        fn on_proposal_finalized(&self, _path: &str) {}
    }

    #[derive(Default)]
    struct Recorder {
        deferred: std::sync::Mutex<Vec<(PathBuf, Option<String>, String)>>,
    }

    impl AcpObserver for Recorder {
        fn on_stream_chunk(&self, _text: &str) {}
        fn on_tool_call_started(&self, _tool_name: &str) {}
        fn on_streaming_status(&self, _status: &str) {}
        fn on_message_complete(&self, _role: &str, _content: &str) {}
        fn on_proposal_deferred(&self, path: &Path, old_content: Option<&str>, new_content: &str) {
            self.deferred.lock().unwrap().push((
                path.to_path_buf(),
                old_content.map(str::to_string),
                new_content.to_string(),
            ));
        }
    }

    fn gate(mode: WriteMode) -> Arc<Mutex<WriteGatePipeline>> {
        Arc::new(Mutex::new(WriteGatePipeline::new(
            mode,
            Box::new(NoopGateObserver),
        )))
    }

    async fn pending(gate: &Arc<Mutex<WriteGatePipeline>>) -> Vec<WriteProposal> {
        gate.lock().await.pending_proposals().to_vec()
    }

    /// One modified file, the shape of almost every real turn.
    #[tokio::test]
    async fn deferred_write_is_restored_and_proposed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "after\n").unwrap();

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            Some("conv-1"),
            vec![DirectWrite::new(
                PathBuf::from("a.txt"),
                Some("before\n".into()),
            )],
        )
        .await;

        assert_eq!(outcome.proposed, vec![PathBuf::from("a.txt")]);
        assert!(outcome.left_in_place.is_empty());
        // Restored: the agent's bytes leave the tree until a human accepts.
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "before\n"
        );
        let proposals = pending(&gate).await;
        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].original_content, "before\n");
        assert_eq!(proposals[0].proposed_content, "after\n");
        assert_eq!(proposals[0].conv_id.as_deref(), Some("conv-1"));
    }

    #[tokio::test]
    async fn created_file_is_removed_and_proposed_as_new() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("new.rs"), "fn main() {}\n").unwrap();

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            Some("conv-1"),
            vec![DirectWrite::new(PathBuf::from("new.rs"), None)],
        )
        .await;

        assert_eq!(outcome.proposed, vec![PathBuf::from("new.rs")]);
        assert!(
            !root.join("new.rs").exists(),
            "created file must be removed"
        );
        let proposals = pending(&gate).await;
        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].original_content.is_empty());
    }

    #[tokio::test]
    async fn deleted_file_is_restored_and_proposed_as_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // The agent removed it during the turn.

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![DirectWrite::new(
                PathBuf::from("gone.txt"),
                Some("was here\n".into()),
            )],
        )
        .await;

        assert_eq!(outcome.proposed, vec![PathBuf::from("gone.txt")]);
        assert_eq!(
            std::fs::read_to_string(root.join("gone.txt")).unwrap(),
            "was here\n",
            "a deletion must be undone until the user accepts it"
        );
        let proposals = pending(&gate).await;
        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].is_deletion);
    }

    #[tokio::test]
    async fn auto_accept_leaves_the_bytes_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "after\n").unwrap();

        let gate = gate(WriteMode::AutoAccept);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![DirectWrite::new(
                PathBuf::from("a.txt"),
                Some("before\n".into()),
            )],
        )
        .await;

        assert_eq!(outcome.left_in_place, vec![PathBuf::from("a.txt")]);
        assert!(outcome.proposed.is_empty());
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "after\n"
        );
        assert!(pending(&gate).await.is_empty());
    }

    #[tokio::test]
    async fn reject_all_removes_the_agents_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "after\n").unwrap();

        let gate = gate(WriteMode::RejectAll);
        let rec = Recorder::default();
        let _ = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![DirectWrite::new(
                PathBuf::from("a.txt"),
                Some("before\n".into()),
            )],
        )
        .await;

        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "before\n"
        );
        assert!(pending(&gate).await.is_empty());
    }

    #[tokio::test]
    async fn unchanged_content_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "same\n").unwrap();

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![DirectWrite::new(
                PathBuf::from("a.txt"),
                Some("same\n".into()),
            )],
        )
        .await;

        assert!(outcome.proposed.is_empty());
        assert!(outcome.left_in_place.is_empty());
        assert!(pending(&gate).await.is_empty());
    }

    #[tokio::test]
    async fn binary_content_is_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("blob.bin"), [0xff_u8, 0xfe, 0x00]).unwrap();

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![DirectWrite::new(
                PathBuf::from("blob.bin"),
                Some("x".into()),
            )],
        )
        .await;

        assert_eq!(outcome.left_in_place, vec![PathBuf::from("blob.bin")]);
        assert!(outcome.proposed.is_empty());
        // Undiffable is not the same as rejected: the bytes stay put, and the
        // caller reports them so they are visible rather than silently kept.
        assert_eq!(
            std::fs::read(root.join("blob.bin")).unwrap(),
            vec![0xff_u8, 0xfe, 0x00]
        );
    }

    #[tokio::test]
    async fn sensitive_path_is_refused_and_stays_restored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".env"), "SECRET=new\n").unwrap();

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let _ = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![DirectWrite::new(
                PathBuf::from(".env"),
                Some("SECRET=old\n".into()),
            )],
        )
        .await;

        assert_eq!(
            std::fs::read_to_string(root.join(".env")).unwrap(),
            "SECRET=old\n"
        );
        assert!(
            pending(&gate).await.is_empty(),
            "no proposal for a sensitive path — the refusal is the rejection"
        );
    }

    #[tokio::test]
    async fn duplicate_paths_collapse_to_one_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "after\n").unwrap();

        let gate = gate(WriteMode::Deferred);
        let rec = Recorder::default();
        let outcome = reconcile_direct_writes(
            &gate,
            &rec,
            root,
            "deepseek",
            None,
            vec![
                DirectWrite::new(PathBuf::from("a.txt"), Some("before\n".into())),
                DirectWrite::new(PathBuf::from("a.txt"), Some("before\n".into())),
            ],
        )
        .await;

        assert_eq!(outcome.proposed, vec![PathBuf::from("a.txt")]);
        assert_eq!(pending(&gate).await.len(), 1);
    }
}
