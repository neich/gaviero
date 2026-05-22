use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::diff_engine::compute_hunks;
use crate::observer::WriteGateObserver;
use crate::tree_sitter::{enrich_hunks, language_for_extension};
use crate::types::{
    DiffHunk, FileScope, HunkStatus, HunkType, ProposalStatus, StructuralHunk, WriteProposal,
};

/// Action returned by `insert_proposal` / `finalize` when the gate is in
/// AutoAccept mode. The caller performs the disk I/O outside the lock.
#[derive(Clone, Debug)]
pub enum AutoAcceptAction {
    /// Write `content` to `path`.
    Write { path: PathBuf, content: String },
    /// Remove `path` from disk.
    Delete { path: PathBuf },
}

/// Controls how proposals are handled.
#[derive(Clone, Debug, PartialEq)]
pub enum WriteMode {
    /// Show diff overlay, user reviews each hunk.
    Interactive,
    /// Accept all hunks immediately, write to disk.
    AutoAccept,
    /// Reject all proposals.
    RejectAll,
    /// Collect proposals without writing to disk. Used for batch review mode.
    Deferred,
}

/// The write gate pipeline. Manages proposals, scope validation, and hunk review.
///
/// IMPORTANT: Never hold this across async I/O. Lock only for HashMap operations.
pub struct WriteGatePipeline {
    proposals: HashMap<u64, WriteProposal>,
    next_id: u64,
    /// Fallback mode used when a proposal has no `conv_id` or no per-conv
    /// override is registered. `set_mode` / `mode()` operate on this field
    /// so legacy callers continue to work.
    default_mode: WriteMode,
    /// Per-conversation mode overrides. When a proposal carries a `conv_id`
    /// listed here, the effective mode is `conv_modes[conv_id]` instead of
    /// `default_mode`. This isolates concurrent providers: one chat task
    /// finishing and flipping to `Interactive` no longer pops a single-proposal
    /// modal for another conversation that is still streaming in `Deferred`.
    conv_modes: HashMap<String, WriteMode>,
    observer: Box<dyn WriteGateObserver>,
    agent_scopes: HashMap<String, FileScope>,
    /// Proposals accumulated in `Deferred` mode, pending batch review.
    deferred_proposals: Vec<WriteProposal>,
    /// Proposals handed to the TUI batch-review inbox but still tracked for
    /// same-path conflict detection until the user resolves or dismisses them.
    review_hold: Vec<WriteProposal>,
}

impl WriteGatePipeline {
    pub fn new(mode: WriteMode, observer: Box<dyn WriteGateObserver>) -> Self {
        Self {
            proposals: HashMap::new(),
            next_id: 1,
            default_mode: mode,
            conv_modes: HashMap::new(),
            observer,
            agent_scopes: HashMap::new(),
            deferred_proposals: Vec::new(),
            review_hold: Vec::new(),
        }
    }

    /// Effective mode for a proposal: per-conv override if present, else
    /// `default_mode`. `conv_id = None` always resolves to `default_mode`.
    fn effective_mode(&self, conv_id: Option<&str>) -> &WriteMode {
        conv_id
            .and_then(|id| self.conv_modes.get(id))
            .unwrap_or(&self.default_mode)
    }

    /// Register a file scope for an agent.
    pub fn register_agent_scope(&mut self, agent_id: &str, scope: &FileScope) {
        self.agent_scopes
            .insert(agent_id.to_string(), scope.clone());
    }

    /// Check if an agent is allowed to write to a path.
    ///
    /// Scopes are restriction lists used by swarm agents to limit which files
    /// they can touch. A normal (non-swarm) prompt has no registered scope, which
    /// means no restrictions — all paths are allowed.
    pub fn is_scope_allowed(&self, agent_id: &str, path: &str) -> bool {
        match self.agent_scopes.get(agent_id) {
            Some(scope) => scope.is_owned(path),
            None => true, // No scope registered = no restrictions (swarm agents always register one)
        }
    }

    /// Allocate the next proposal ID.
    pub fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Build a WriteProposal from original + proposed content.
    /// Computes diff hunks and enriches with structural info.
    /// This does NOT hold self — call it before inserting.
    ///
    /// `conv_id` tags the proposal with the originating conversation so the
    /// gate can drain / mode-switch per conversation. `None` means the
    /// proposal came from a non-conversational path (CLI batch ops, internal
    /// tooling) and the gate's `default_mode` applies.
    pub fn build_proposal(
        id: u64,
        source: &str,
        conv_id: Option<&str>,
        file_path: &Path,
        original_content: &str,
        proposed_content: &str,
    ) -> WriteProposal {
        let hunks = compute_hunks(original_content, proposed_content);

        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let structural_hunks = match language_for_extension(ext) {
            Some(lang) => enrich_hunks(hunks, original_content, lang),
            None => hunks
                .into_iter()
                .map(|h| {
                    let desc = format!(
                        "{} lines {}-{}",
                        match h.hunk_type {
                            crate::types::HunkType::Added => "Add",
                            crate::types::HunkType::Removed => "Remove",
                            crate::types::HunkType::Modified => "Modify",
                        },
                        h.original_range.0 + 1,
                        h.original_range.1,
                    );
                    crate::types::StructuralHunk {
                        diff_hunk: h,
                        enclosing_node: None,
                        description: desc,
                        status: HunkStatus::Pending,
                    }
                })
                .collect(),
        };

        WriteProposal {
            id,
            source: source.to_string(),
            file_path: file_path.to_path_buf(),
            original_content: original_content.to_string(),
            proposed_content: proposed_content.to_string(),
            structural_hunks,
            status: ProposalStatus::Pending,
            is_deletion: false,
            conv_id: conv_id.map(str::to_string),
            conflicts_with: Vec::new(),
        }
    }

    /// Build a deletion proposal: original content is the file as it existed
    /// before the tool removed it; proposed content is empty. The hunk list
    /// renders as a single full-file removal so reviewers see exactly what
    /// they're losing. `is_deletion` distinguishes "delete the file" from
    /// "write an empty file" at finalize time.
    pub fn build_delete_proposal(
        id: u64,
        source: &str,
        conv_id: Option<&str>,
        file_path: &Path,
        original_content: &str,
    ) -> WriteProposal {
        let hunk = if original_content.is_empty() {
            None
        } else {
            let original_lines = original_content.lines().count().max(1);
            let diff_hunk = DiffHunk {
                original_range: (0, original_lines),
                proposed_range: (0, 0),
                original_text: original_content.to_string(),
                proposed_text: String::new(),
                hunk_type: HunkType::Removed,
            };
            Some(StructuralHunk {
                diff_hunk,
                enclosing_node: None,
                description: format!("Delete file ({} lines)", original_lines),
                status: HunkStatus::Pending,
            })
        };

        WriteProposal {
            id,
            source: source.to_string(),
            file_path: file_path.to_path_buf(),
            original_content: original_content.to_string(),
            proposed_content: String::new(),
            structural_hunks: hunk.into_iter().collect(),
            status: ProposalStatus::Pending,
            is_deletion: true,
            conv_id: conv_id.map(str::to_string),
            conflicts_with: Vec::new(),
        }
    }

    /// Insert a proposal into the pipeline.
    /// Returns `Some(action)` if the mode auto-accepts (caller performs disk I/O).
    /// Returns `None` if the proposal is queued for interactive review, deferred, or rejected.
    ///
    /// The effective `WriteMode` is the per-conv override for `proposal.conv_id`
    /// if registered, otherwise `default_mode`. This keeps concurrent providers
    /// isolated.
    pub fn insert_proposal(&mut self, proposal: WriteProposal) -> Option<AutoAcceptAction> {
        let mode = self.effective_mode(proposal.conv_id.as_deref()).clone();
        match mode {
            WriteMode::Interactive => {
                self.observer.on_proposal_created(&proposal);
                self.proposals.insert(proposal.id, proposal);
                None
            }
            WriteMode::AutoAccept => {
                let mut proposal = proposal;
                // Accept all hunks
                for hunk in &mut proposal.structural_hunks {
                    hunk.status = HunkStatus::Accepted;
                }
                proposal.status = ProposalStatus::Accepted;
                let path = proposal.file_path.clone();
                self.observer.on_proposal_finalized(&path.to_string_lossy());
                if proposal.is_deletion {
                    Some(AutoAcceptAction::Delete { path })
                } else {
                    Some(AutoAcceptAction::Write {
                        path,
                        content: assemble_final_content(&proposal),
                    })
                }
            }
            WriteMode::RejectAll => {
                // Silently discard
                None
            }
            WriteMode::Deferred => {
                // Accumulate for batch review — no observer notification, no disk write
                self.deferred_proposals.push(proposal);
                None
            }
        }
    }

    /// Whether the pipeline default is `Deferred`. Per-conv overrides are
    /// inspected via [`Self::is_deferred_for_conv`].
    pub fn is_deferred(&self) -> bool {
        self.default_mode == WriteMode::Deferred
    }

    /// Whether the effective mode for `conv_id` is `Deferred`. `None`
    /// resolves to `default_mode`.
    pub fn is_deferred_for_conv(&self, conv_id: Option<&str>) -> bool {
        *self.effective_mode(conv_id) == WriteMode::Deferred
    }

    /// Access the deferred proposals accumulated so far.
    pub fn pending_proposals(&self) -> &[WriteProposal] {
        &self.deferred_proposals
    }

    /// Drain and return all deferred proposals.
    ///
    /// Prefer [`Self::take_pending_proposals_for_conv`] when multiple
    /// conversations share a single gate — drain-all sweeps up another
    /// conversation's still-pending proposals.
    pub fn take_pending_proposals(&mut self) -> Vec<WriteProposal> {
        let drained = std::mem::take(&mut self.deferred_proposals);
        self.hold_for_review(&drained);
        drained
    }

    /// Drain and return deferred proposals owned by `conv_id`. `None` drains
    /// the global bucket (proposals with no `conv_id`); `Some(id)` drains only
    /// proposals whose `conv_id` matches. Proposals owned by other
    /// conversations stay queued so a finishing chat task does not sweep up
    /// a concurrent conversation's pending work.
    pub fn take_pending_proposals_for_conv(
        &mut self,
        conv_id: Option<&str>,
    ) -> Vec<WriteProposal> {
        let mut keep = Vec::new();
        let mut drained = Vec::new();
        for proposal in std::mem::take(&mut self.deferred_proposals) {
            let matches = match (conv_id, proposal.conv_id.as_deref()) {
                (None, None) => true,
                (Some(a), Some(b)) => a == b,
                _ => false,
            };
            if matches {
                drained.push(proposal);
            } else {
                keep.push(proposal);
            }
        }
        self.deferred_proposals = keep;
        self.hold_for_review(&drained);
        drained
    }

    /// Track proposals currently in the TUI batch-review inbox so
    /// `conflict_candidates_for_path` still sees them after deferred drain.
    pub fn hold_for_review(&mut self, proposals: &[WriteProposal]) {
        for proposal in proposals {
            if self
                .review_hold
                .iter()
                .any(|held| held.id == proposal.id)
            {
                continue;
            }
            self.review_hold.push(proposal.clone());
        }
    }

    /// Stop tracking the given proposal IDs (user applied, rejected, or
    /// dismissed the corresponding batch-review rows).
    pub fn release_review_hold_ids(&mut self, ids: &[u64]) {
        if ids.is_empty() {
            return;
        }
        self.review_hold.retain(|p| !ids.contains(&p.id));
    }

    /// Drop every held proposal (batch review cancelled or fully finalized).
    pub fn release_review_hold_all(&mut self) {
        self.review_hold.clear();
    }

    /// Lookup a proposal across active, deferred, and review-hold buckets.
    pub fn proposal_by_id(&self, proposal_id: u64) -> Option<&WriteProposal> {
        self.proposals
            .get(&proposal_id)
            .or_else(|| {
                self.deferred_proposals
                    .iter()
                    .find(|p| p.id == proposal_id)
            })
            .or_else(|| self.review_hold.iter().find(|p| p.id == proposal_id))
    }

    /// Fields the TUI batch inbox mirrors when `ProposalUpdated` fires.
    pub fn review_proposal_fields(&self, proposal_id: u64) -> Option<(Vec<u64>, ProposalStatus)> {
        self.proposal_by_id(proposal_id)
            .map(|p| (p.conflicts_with.clone(), p.status.clone()))
    }

    /// Accept a specific hunk by index.
    pub fn accept_hunk(&mut self, proposal_id: u64, hunk_index: usize) {
        if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
            if let Some(hunk) = proposal.structural_hunks.get_mut(hunk_index) {
                hunk.status = HunkStatus::Accepted;
            }
            update_proposal_status(proposal);
            self.observer.on_proposal_updated(proposal_id);
        }
    }

    /// Reject a specific hunk by index.
    pub fn reject_hunk(&mut self, proposal_id: u64, hunk_index: usize) {
        if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
            if let Some(hunk) = proposal.structural_hunks.get_mut(hunk_index) {
                hunk.status = HunkStatus::Rejected;
            }
            update_proposal_status(proposal);
            self.observer.on_proposal_updated(proposal_id);
        }
    }

    /// Accept all hunks whose enclosing node matches the given name.
    pub fn accept_node(&mut self, proposal_id: u64, node_name: &str) {
        if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
            for hunk in &mut proposal.structural_hunks {
                if let Some(ref node) = hunk.enclosing_node {
                    if node.name.as_deref() == Some(node_name) {
                        hunk.status = HunkStatus::Accepted;
                    }
                }
            }
            update_proposal_status(proposal);
            self.observer.on_proposal_updated(proposal_id);
        }
    }

    /// Accept all hunks in a proposal.
    pub fn accept_all(&mut self, proposal_id: u64) {
        if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
            for hunk in &mut proposal.structural_hunks {
                hunk.status = HunkStatus::Accepted;
            }
            proposal.status = ProposalStatus::Accepted;
            self.observer.on_proposal_updated(proposal_id);
        }
    }

    /// Reject all hunks in a proposal.
    pub fn reject_all(&mut self, proposal_id: u64) {
        if let Some(proposal) = self.proposals.get_mut(&proposal_id) {
            for hunk in &mut proposal.structural_hunks {
                hunk.status = HunkStatus::Rejected;
            }
            proposal.status = ProposalStatus::Rejected;
            self.observer.on_proposal_updated(proposal_id);
        }
    }

    /// Finalize a proposal: remove from pipeline and emit the disk action.
    /// For writes, assembles content from accepted hunks. For deletions, the
    /// caller removes the file. Returns `None` if the proposal id is unknown.
    pub fn finalize(&mut self, proposal_id: u64) -> Option<AutoAcceptAction> {
        let proposal = self.proposals.remove(&proposal_id)?;
        // Mark each conflicting peer as Superseded so the panel can render it
        // as "⊘ superseded by N" and the accept path refuses to write it.
        let peers = proposal.conflicts_with.clone();
        for peer_id in &peers {
            let mut updated = false;
            if let Some(peer) = self.proposals.get_mut(peer_id) {
                peer.status = ProposalStatus::Superseded;
                updated = true;
            }
            if let Some(peer) = self
                .deferred_proposals
                .iter_mut()
                .find(|p| p.id == *peer_id)
            {
                peer.status = ProposalStatus::Superseded;
                updated = true;
            }
            if let Some(peer) = self
                .review_hold
                .iter_mut()
                .find(|p| p.id == *peer_id)
            {
                peer.status = ProposalStatus::Superseded;
                updated = true;
            }
            if updated {
                self.observer.on_proposal_updated(*peer_id);
            }
        }
        let path = proposal.file_path.clone();
        self.observer.on_proposal_finalized(&path.to_string_lossy());
        if proposal.is_deletion {
            Some(AutoAcceptAction::Delete { path })
        } else {
            Some(AutoAcceptAction::Write {
                path,
                content: assemble_final_content(&proposal),
            })
        }
    }

    /// Get an immutable reference to a proposal.
    pub fn get_proposal(&self, proposal_id: u64) -> Option<&WriteProposal> {
        self.proposals.get(&proposal_id)
    }

    /// Get active proposal for a given file path.
    pub fn proposal_for_path(&self, path: &Path) -> Option<&WriteProposal> {
        self.proposals.values().find(|p| p.file_path == path)
    }

    /// Get all active proposal IDs.
    pub fn active_proposal_ids(&self) -> Vec<u64> {
        self.proposals.keys().copied().collect()
    }

    /// Returns the default mode (fallback for proposals with no `conv_id` or
    /// no per-conv override). Use [`Self::effective_mode_for_conv`] to inspect
    /// the resolved mode for a specific conversation.
    pub fn mode(&self) -> &WriteMode {
        &self.default_mode
    }

    /// Returns the effective mode for `conv_id`: per-conv override if set,
    /// otherwise `default_mode`.
    pub fn effective_mode_for_conv(&self, conv_id: Option<&str>) -> &WriteMode {
        self.effective_mode(conv_id)
    }

    /// Change the *default* write mode at runtime. Per-conv overrides
    /// continue to take precedence.
    pub fn set_mode(&mut self, mode: WriteMode) {
        self.default_mode = mode;
    }

    /// Register a per-conversation `WriteMode` override. A proposal whose
    /// `conv_id` matches will use this mode instead of `default_mode` until
    /// [`Self::clear_conv_mode`] is called.
    pub fn set_conv_mode(&mut self, conv_id: impl Into<String>, mode: WriteMode) {
        self.conv_modes.insert(conv_id.into(), mode);
    }

    /// Remove a per-conversation `WriteMode` override. Future proposals from
    /// that conversation fall back to `default_mode`.
    pub fn clear_conv_mode(&mut self, conv_id: &str) {
        self.conv_modes.remove(conv_id);
    }

    /// Collect the IDs of every pending or deferred proposal that targets
    /// `abs_path`. Used by the propose-write seam to detect concurrent
    /// providers proposing changes to the same file so the conflict can be
    /// surfaced in the review UX instead of silently dropping the later
    /// proposal.
    pub fn conflict_candidates_for_path(&self, abs_path: &Path) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .proposals
            .values()
            .filter(|p| p.file_path == abs_path)
            .map(|p| p.id)
            .collect();
        ids.extend(
            self.deferred_proposals
                .iter()
                .filter(|p| p.file_path == abs_path)
                .map(|p| p.id),
        );
        ids.extend(
            self.review_hold
                .iter()
                .filter(|p| p.file_path == abs_path)
                .map(|p| p.id),
        );
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Link a freshly-inserted proposal `new_id` with `peers` as a conflict
    /// set. Each peer gets `new_id` appended to its `conflicts_with`, and
    /// observers see one `on_proposal_updated` per touched peer. Has no
    /// effect on the new proposal itself — the caller is expected to seed
    /// its `conflicts_with` field with `peers` before calling
    /// `insert_proposal`.
    pub fn mark_conflict_pair(&mut self, new_id: u64, peers: &[u64]) {
        for peer_id in peers {
            let mut updated = false;
            if let Some(peer) = self.proposals.get_mut(peer_id)
                && !peer.conflicts_with.contains(&new_id)
            {
                peer.conflicts_with.push(new_id);
                updated = true;
            }
            if let Some(peer) = self
                .deferred_proposals
                .iter_mut()
                .find(|p| p.id == *peer_id)
                && !peer.conflicts_with.contains(&new_id)
            {
                peer.conflicts_with.push(new_id);
                updated = true;
            }
            if let Some(peer) = self
                .review_hold
                .iter_mut()
                .find(|p| p.id == *peer_id)
                && !peer.conflicts_with.contains(&new_id)
            {
                peer.conflicts_with.push(new_id);
                updated = true;
            }
            if updated {
                self.observer.on_proposal_updated(*peer_id);
            }
        }
    }
}

/// Assemble the final file content by applying accepted hunks to the original.
pub fn assemble_final_content(proposal: &WriteProposal) -> String {
    let original_lines: Vec<&str> = proposal.original_content.lines().collect();
    let mut result = String::new();
    let mut orig_idx = 0;

    for hunk in &proposal.structural_hunks {
        let dh = &hunk.diff_hunk;

        // Copy unchanged lines before this hunk
        while orig_idx < dh.original_range.0 {
            if orig_idx < original_lines.len() {
                result.push_str(original_lines[orig_idx]);
                result.push('\n');
            }
            orig_idx += 1;
        }

        match hunk.status {
            HunkStatus::Accepted => {
                // Use proposed text
                result.push_str(&dh.proposed_text);
                orig_idx = dh.original_range.1;
            }
            HunkStatus::Rejected | HunkStatus::Pending => {
                // Keep original text
                result.push_str(&dh.original_text);
                orig_idx = dh.original_range.1;
            }
        }
    }

    // Copy remaining original lines
    while orig_idx < original_lines.len() {
        result.push_str(original_lines[orig_idx]);
        result.push('\n');
        orig_idx += 1;
    }

    // Handle trailing newline: match original
    if proposal.original_content.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    if !proposal.original_content.ends_with('\n') && result.ends_with('\n') {
        result.truncate(result.len() - 1);
    }

    result
}

fn update_proposal_status(proposal: &mut WriteProposal) {
    let (mut any_accepted, mut any_non_accepted) = (false, false);
    for h in &proposal.structural_hunks {
        if h.status == HunkStatus::Accepted {
            any_accepted = true;
        } else {
            any_non_accepted = true;
        }
        if any_accepted && any_non_accepted {
            break; // early exit — already know it's partial
        }
    }

    proposal.status = match (any_accepted, any_non_accepted) {
        (true, false) => ProposalStatus::Accepted,
        (false, true) => {
            // Check if all rejected or some still pending
            if proposal
                .structural_hunks
                .iter()
                .all(|h| h.status == HunkStatus::Rejected)
            {
                ProposalStatus::Rejected
            } else {
                ProposalStatus::Pending
            }
        }
        (true, true) => ProposalStatus::PartiallyAccepted,
        (false, false) => ProposalStatus::Pending, // empty hunks
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::WriteGateObserver;
    use crate::types::WriteProposal;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestObserver {
        created_count: Arc<AtomicU64>,
        updated_count: Arc<AtomicU64>,
        finalized_count: Arc<AtomicU64>,
    }

    impl TestObserver {
        fn new() -> (Self, Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicU64>) {
            let c = Arc::new(AtomicU64::new(0));
            let u = Arc::new(AtomicU64::new(0));
            let f = Arc::new(AtomicU64::new(0));
            (
                Self {
                    created_count: c.clone(),
                    updated_count: u.clone(),
                    finalized_count: f.clone(),
                },
                c,
                u,
                f,
            )
        }
    }

    impl WriteGateObserver for TestObserver {
        fn on_proposal_created(&self, _proposal: &WriteProposal) {
            self.created_count.fetch_add(1, Ordering::SeqCst);
        }
        fn on_proposal_updated(&self, _proposal_id: u64) {
            self.updated_count.fetch_add(1, Ordering::SeqCst);
        }
        fn on_proposal_finalized(&self, _path: &str) {
            self.finalized_count.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn make_pipeline(
        mode: WriteMode,
    ) -> (
        WriteGatePipeline,
        Arc<AtomicU64>,
        Arc<AtomicU64>,
        Arc<AtomicU64>,
    ) {
        let (obs, c, u, f) = TestObserver::new();
        let pipeline = WriteGatePipeline::new(mode, Box::new(obs));
        (pipeline, c, u, f)
    }

    #[test]
    fn test_interactive_mode_queues_proposal() {
        let (mut pipeline, created, _updated, _finalized) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        assert!(
            result.is_none(),
            "Interactive mode should not return content"
        );
        assert_eq!(created.load(Ordering::SeqCst), 1);
        assert!(pipeline.get_proposal(id).is_some());
    }

    #[test]
    fn test_auto_accept_mode_returns_content() {
        let (mut pipeline, _created, _updated, finalized) = make_pipeline(WriteMode::AutoAccept);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        let action = result.expect("AutoAccept should return an action");
        match action {
            AutoAcceptAction::Write { path, content } => {
                assert_eq!(path, PathBuf::from("src/main.rs"));
                assert!(content.contains("new"));
            }
            AutoAcceptAction::Delete { .. } => panic!("expected Write, got Delete"),
        }
        assert_eq!(finalized.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_reject_all_mode_discards() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::RejectAll);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        assert!(result.is_none());
    }

    #[test]
    fn test_deferred_mode_accumulates_proposals() {
        let (mut pipeline, created, _updated, _finalized) = make_pipeline(WriteMode::Deferred);
        assert!(pipeline.is_deferred());

        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        assert!(result.is_none(), "Deferred mode should not return content");
        assert_eq!(
            created.load(Ordering::SeqCst),
            0,
            "No observer notification in deferred mode"
        );
        assert_eq!(pipeline.pending_proposals().len(), 1);

        // Add a second proposal
        let id2 = pipeline.next_id();
        let proposal2 = WriteGatePipeline::build_proposal(
            id2,
            "test-agent",
            None,
            Path::new("src/lib.rs"),
            "fn lib_old() {}\n",
            "fn lib_new() {}\n",
        );
        pipeline.insert_proposal(proposal2);
        assert_eq!(pipeline.pending_proposals().len(), 2);

        // Take drains the vec
        let taken = pipeline.take_pending_proposals();
        assert_eq!(taken.len(), 2);
        assert!(pipeline.pending_proposals().is_empty());
    }

    #[test]
    fn test_take_pending_proposals_for_conv_filters_by_owner() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::Deferred);

        let id_a = pipeline.next_id();
        let proposal_a = WriteGatePipeline::build_proposal(
            id_a,
            "agent-a",
            Some("conv-A"),
            Path::new("src/a.rs"),
            "",
            "fn a() {}\n",
        );
        pipeline.insert_proposal(proposal_a);

        let id_b = pipeline.next_id();
        let proposal_b = WriteGatePipeline::build_proposal(
            id_b,
            "agent-b",
            Some("conv-B"),
            Path::new("src/b.rs"),
            "",
            "fn b() {}\n",
        );
        pipeline.insert_proposal(proposal_b);

        let id_none = pipeline.next_id();
        let proposal_none = WriteGatePipeline::build_proposal(
            id_none,
            "external",
            None,
            Path::new("src/c.rs"),
            "",
            "fn c() {}\n",
        );
        pipeline.insert_proposal(proposal_none);

        assert_eq!(pipeline.pending_proposals().len(), 3);

        let drained_a = pipeline.take_pending_proposals_for_conv(Some("conv-A"));
        assert_eq!(drained_a.len(), 1);
        assert_eq!(drained_a[0].id, id_a);
        assert_eq!(pipeline.pending_proposals().len(), 2);

        // conv-B's proposal must still be there
        assert!(
            pipeline
                .pending_proposals()
                .iter()
                .any(|p| p.conv_id.as_deref() == Some("conv-B"))
        );

        // None-keyed bucket is independent
        let drained_none = pipeline.take_pending_proposals_for_conv(None);
        assert_eq!(drained_none.len(), 1);
        assert!(drained_none[0].conv_id.is_none());
        assert_eq!(pipeline.pending_proposals().len(), 1);
    }

    #[test]
    fn test_review_hold_keeps_path_visible_after_drain() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::Deferred);

        let id_a = pipeline.next_id();
        let proposal_a = WriteGatePipeline::build_proposal(
            id_a,
            "agent-a",
            Some("conv-A"),
            Path::new("src/shared.rs"),
            "old\n",
            "from-a\n",
        );
        pipeline.insert_proposal(proposal_a);

        let drained = pipeline.take_pending_proposals_for_conv(Some("conv-A"));
        assert_eq!(drained.len(), 1);
        assert!(pipeline.pending_proposals().is_empty());

        // A is in review_hold — a later provider must still see the collision.
        let peers = pipeline.conflict_candidates_for_path(Path::new("src/shared.rs"));
        assert_eq!(peers, vec![id_a]);

        pipeline.release_review_hold_ids(&[id_a]);
        assert!(pipeline.conflict_candidates_for_path(Path::new("src/shared.rs")).is_empty());
    }

    #[test]
    fn test_finalize_marks_conflict_peers_superseded() {
        let (mut pipeline, _c, updated, _f) = make_pipeline(WriteMode::Interactive);

        let id_a = pipeline.next_id();
        let mut proposal_a = WriteGatePipeline::build_proposal(
            id_a,
            "agent-a",
            Some("conv-A"),
            Path::new("src/shared.rs"),
            "old\n",
            "from-a\n",
        );
        // Pre-stage the conflict link as the runner would.
        let id_b = pipeline.next_id() + 1; // Reserve next id below; pretend B's id.
        proposal_a.conflicts_with = vec![id_b];
        pipeline.insert_proposal(proposal_a);

        let mut proposal_b = WriteGatePipeline::build_proposal(
            id_b,
            "agent-b",
            Some("conv-B"),
            Path::new("src/shared.rs"),
            "old\n",
            "from-b\n",
        );
        proposal_b.conflicts_with = vec![id_a];
        pipeline.insert_proposal(proposal_b);

        let before = updated.load(Ordering::SeqCst);
        // Accept A.
        pipeline.accept_all(id_a);
        let _ = pipeline.finalize(id_a);

        // B must now be Superseded with one extra observer update.
        let b_after = pipeline.get_proposal(id_b).unwrap();
        assert_eq!(b_after.status, ProposalStatus::Superseded);
        // accept_all fired one update for A; finalize fired one update for B.
        assert!(updated.load(Ordering::SeqCst) > before);
    }

    #[test]
    fn test_mark_conflict_pair_links_proposals_and_fires_updated() {
        let (mut pipeline, _c, updated, _f) = make_pipeline(WriteMode::Interactive);

        // First proposal lands in Interactive mode → `proposals` HashMap.
        let id_a = pipeline.next_id();
        let proposal_a = WriteGatePipeline::build_proposal(
            id_a,
            "agent-a",
            Some("conv-A"),
            Path::new("src/shared.rs"),
            "fn old() {}\n",
            "fn from_a() {}\n",
        );
        pipeline.insert_proposal(proposal_a);
        // Reset updated counter — `on_proposal_created` did not bump it.
        let before = updated.load(Ordering::SeqCst);

        // Second proposal targets the same file from a different conv.
        let peers = pipeline.conflict_candidates_for_path(Path::new("src/shared.rs"));
        assert_eq!(peers, vec![id_a]);

        let id_b = pipeline.next_id();
        let mut proposal_b = WriteGatePipeline::build_proposal(
            id_b,
            "agent-b",
            Some("conv-B"),
            Path::new("src/shared.rs"),
            "fn old() {}\n",
            "fn from_b() {}\n",
        );
        proposal_b.conflicts_with = peers.clone();
        pipeline.insert_proposal(proposal_b);
        pipeline.mark_conflict_pair(id_b, &peers);

        // A learned about B (one `on_proposal_updated`).
        assert_eq!(updated.load(Ordering::SeqCst), before + 1);
        let a_after = pipeline.get_proposal(id_a).unwrap();
        assert_eq!(a_after.conflicts_with, vec![id_b]);
        // B carries A from construction.
        let b_after = pipeline.get_proposal(id_b).unwrap();
        assert_eq!(b_after.conflicts_with, vec![id_a]);
    }

    #[test]
    fn test_per_conv_write_mode_overrides_default() {
        let (mut pipeline, created, _u, finalized) = make_pipeline(WriteMode::Interactive);
        // Conv A → Deferred (override). Conv B → no override (default
        // Interactive). Conv C → AutoAccept (override).
        pipeline.set_conv_mode("conv-A", WriteMode::Deferred);
        pipeline.set_conv_mode("conv-C", WriteMode::AutoAccept);

        // Default is still Interactive.
        assert_eq!(*pipeline.mode(), WriteMode::Interactive);
        assert!(pipeline.is_deferred_for_conv(Some("conv-A")));
        assert!(!pipeline.is_deferred_for_conv(Some("conv-B")));

        // Conv A insert lands in the deferred bucket without firing
        // `on_proposal_created`.
        let id_a = pipeline.next_id();
        let proposal_a = WriteGatePipeline::build_proposal(
            id_a,
            "agent-a",
            Some("conv-A"),
            Path::new("src/a.rs"),
            "",
            "fn a() {}\n",
        );
        assert!(pipeline.insert_proposal(proposal_a).is_none());
        assert_eq!(pipeline.pending_proposals().len(), 1);
        assert_eq!(created.load(Ordering::SeqCst), 0);

        // Conv B insert hits the default Interactive path: observer fires,
        // proposal lands in `proposals`, no deferred bucket.
        let id_b = pipeline.next_id();
        let proposal_b = WriteGatePipeline::build_proposal(
            id_b,
            "agent-b",
            Some("conv-B"),
            Path::new("src/b.rs"),
            "",
            "fn b() {}\n",
        );
        assert!(pipeline.insert_proposal(proposal_b).is_none());
        assert_eq!(created.load(Ordering::SeqCst), 1);
        assert!(pipeline.get_proposal(id_b).is_some());
        assert_eq!(pipeline.pending_proposals().len(), 1);

        // Conv C insert hits AutoAccept and returns a Write action.
        let id_c = pipeline.next_id();
        let proposal_c = WriteGatePipeline::build_proposal(
            id_c,
            "agent-c",
            Some("conv-C"),
            Path::new("src/c.rs"),
            "",
            "fn c() {}\n",
        );
        match pipeline.insert_proposal(proposal_c) {
            Some(AutoAcceptAction::Write { .. }) => {}
            other => panic!("expected AutoAccept Write, got {:?}", other),
        }
        assert_eq!(finalized.load(Ordering::SeqCst), 1);

        // Clearing the conv-A override drops it back to Interactive.
        pipeline.clear_conv_mode("conv-A");
        assert!(!pipeline.is_deferred_for_conv(Some("conv-A")));
    }

    #[test]
    fn test_accept_reject_hunks() {
        let (mut pipeline, _c, updated, _f) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "aaa\nbbb\nccc\nddd\neee\nfff\n",
            "aaa\nBBB\nccc\nddd\nEEE\nfff\n",
        );
        pipeline.insert_proposal(proposal);

        let p = pipeline.get_proposal(id).unwrap();
        assert_eq!(p.structural_hunks.len(), 2);

        pipeline.accept_hunk(id, 0);
        pipeline.reject_hunk(id, 1);
        assert_eq!(updated.load(Ordering::SeqCst), 2);

        let p = pipeline.get_proposal(id).unwrap();
        assert_eq!(p.structural_hunks[0].status, HunkStatus::Accepted);
        assert_eq!(p.structural_hunks[1].status, HunkStatus::Rejected);
        assert_eq!(p.status, ProposalStatus::PartiallyAccepted);
    }

    #[test]
    fn test_accept_all_and_finalize() {
        let (mut pipeline, _c, _u, finalized) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "aaa\nbbb\nccc\n",
            "aaa\nBBB\nccc\n",
        );
        pipeline.insert_proposal(proposal);
        pipeline.accept_all(id);

        let action = pipeline.finalize(id).expect("finalize returned None");
        match action {
            AutoAcceptAction::Write { content, .. } => {
                assert!(content.contains("BBB"));
                assert!(!content.contains("bbb"));
            }
            AutoAcceptAction::Delete { .. } => panic!("expected Write, got Delete"),
        }
        assert_eq!(finalized.load(Ordering::SeqCst), 1);
        assert!(pipeline.get_proposal(id).is_none());
    }

    #[test]
    fn test_finalize_partial_keeps_original_for_rejected() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            "aaa\nbbb\nccc\nddd\neee\nfff\n",
            "aaa\nBBB\nccc\nddd\nEEE\nfff\n",
        );
        pipeline.insert_proposal(proposal);

        pipeline.accept_hunk(id, 0); // BBB accepted
        pipeline.reject_hunk(id, 1); // EEE rejected, keep eee

        let action = pipeline.finalize(id).unwrap();
        let content = match action {
            AutoAcceptAction::Write { content, .. } => content,
            AutoAcceptAction::Delete { .. } => panic!("expected Write, got Delete"),
        };
        assert!(content.contains("BBB"));
        assert!(content.contains("eee"));
        assert!(!content.contains("bbb"));
        assert!(!content.contains("EEE"));
    }

    #[test]
    fn test_build_delete_proposal_round_trip() {
        let (mut pipeline, _c, _u, finalized) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_delete_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/stale.rs"),
            "fn doomed() {}\n",
        );
        assert!(proposal.is_deletion);
        assert_eq!(proposal.proposed_content, "");
        assert_eq!(proposal.structural_hunks.len(), 1);
        assert_eq!(
            proposal.structural_hunks[0].diff_hunk.hunk_type,
            HunkType::Removed
        );

        pipeline.insert_proposal(proposal);
        pipeline.accept_all(id);

        match pipeline.finalize(id) {
            Some(AutoAcceptAction::Delete { path }) => {
                assert_eq!(path, PathBuf::from("src/stale.rs"));
            }
            other => panic!("expected Delete action, got {:?}", other),
        }
        assert_eq!(finalized.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_auto_accept_delete_returns_delete_action() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::AutoAccept);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_delete_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/stale.rs"),
            "fn doomed() {}\n",
        );
        match pipeline.insert_proposal(proposal) {
            Some(AutoAcceptAction::Delete { path }) => {
                assert_eq!(path, PathBuf::from("src/stale.rs"));
            }
            other => panic!("expected Delete action, got {:?}", other),
        }
    }

    #[test]
    fn test_scope_validation() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::Interactive);
        let scope = FileScope {
            owned_paths: vec!["src/editor/".into()],
            read_only_paths: vec![],
            interface_contracts: std::collections::HashMap::new(),
        };
        pipeline.register_agent_scope("agent-1", &scope);

        assert!(pipeline.is_scope_allowed("agent-1", "src/editor/buffer.rs"));
        assert!(!pipeline.is_scope_allowed("agent-1", "src/main.rs"));
        assert!(pipeline.is_scope_allowed("unknown-agent", "anything")); // no scope = allowed
    }

    #[test]
    fn test_accept_node() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();

        let original =
            "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n\nfn bar() {\n    let z = 3;\n}\n";
        let proposed =
            "fn foo() {\n    let x = 42;\n    let y = 2;\n}\n\nfn bar() {\n    let z = 99;\n}\n";
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            None,
            Path::new("src/main.rs"),
            original,
            proposed,
        );
        pipeline.insert_proposal(proposal);

        // Accept only hunks in "foo"
        pipeline.accept_node(id, "foo");

        let p = pipeline.get_proposal(id).unwrap();
        // The hunk in foo should be accepted
        let foo_hunks: Vec<_> = p
            .structural_hunks
            .iter()
            .filter(|h| h.enclosing_node.as_ref().and_then(|n| n.name.as_deref()) == Some("foo"))
            .collect();
        assert!(foo_hunks.iter().all(|h| h.status == HunkStatus::Accepted));
    }
}
