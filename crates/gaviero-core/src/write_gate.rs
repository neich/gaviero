use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::diff_engine::compute_hunks;
use crate::observer::WriteGateObserver;
use crate::tree_sitter::{enrich_hunks, language_for_extension};
use crate::types::{
    FileScope, HunkStatus, ProposalStatus, WriteProposal,
};

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
    mode: WriteMode,
    observer: Box<dyn WriteGateObserver>,
    agent_scopes: HashMap<String, FileScope>,
    /// Proposals accumulated in `Deferred` mode, pending batch review.
    deferred_proposals: Vec<WriteProposal>,
}

impl WriteGatePipeline {
    pub fn new(mode: WriteMode, observer: Box<dyn WriteGateObserver>) -> Self {
        Self {
            proposals: HashMap::new(),
            next_id: 1,
            mode,
            observer,
            agent_scopes: HashMap::new(),
            deferred_proposals: Vec::new(),
        }
    }

    /// Register a file scope for an agent.
    pub fn register_agent_scope(&mut self, agent_id: &str, scope: &FileScope) {
        self.agent_scopes.insert(agent_id.to_string(), scope.clone());
    }

    /// Check if an agent is allowed to write to a path.
    pub fn is_scope_allowed(&self, agent_id: &str, path: &str) -> bool {
        match self.agent_scopes.get(agent_id) {
            Some(scope) => scope.is_owned(path),
            None => false, // No scope registered = deny (fail-closed)
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
    pub fn build_proposal(
        id: u64,
        source: &str,
        file_path: &Path,
        original_content: &str,
        proposed_content: &str,
    ) -> WriteProposal {
        let hunks = compute_hunks(original_content, proposed_content);

        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
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
        }
    }

    /// Insert a proposal into the pipeline.
    /// Returns `Some((path, content))` if the mode auto-accepts (caller writes to disk).
    /// Returns `None` if the proposal is queued for interactive review, deferred, or rejected.
    pub fn insert_proposal(&mut self, proposal: WriteProposal) -> Option<(PathBuf, String)> {
        match self.mode {
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
                let result = assemble_final_content(&proposal);
                let path = proposal.file_path.clone();
                self.observer.on_proposal_finalized(&path.to_string_lossy());
                Some((path, result))
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

    /// Whether the pipeline is in deferred (batch review) mode.
    pub fn is_deferred(&self) -> bool {
        self.mode == WriteMode::Deferred
    }

    /// Access the deferred proposals accumulated so far.
    pub fn pending_proposals(&self) -> &[WriteProposal] {
        &self.deferred_proposals
    }

    /// Drain and return all deferred proposals.
    pub fn take_pending_proposals(&mut self) -> Vec<WriteProposal> {
        std::mem::take(&mut self.deferred_proposals)
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

    /// Finalize a proposal: assemble content from accepted hunks, remove from pipeline.
    /// Returns `(path, final_content)` for the caller to write to disk.
    pub fn finalize(&mut self, proposal_id: u64) -> Option<(PathBuf, String)> {
        let proposal = self.proposals.remove(&proposal_id)?;
        let content = assemble_final_content(&proposal);
        let path = proposal.file_path.clone();
        self.observer
            .on_proposal_finalized(&path.to_string_lossy());
        Some((path, content))
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

    pub fn mode(&self) -> &WriteMode {
        &self.mode
    }

    /// Change the write mode at runtime (e.g., switch to Deferred before agent calls).
    pub fn set_mode(&mut self, mode: WriteMode) {
        self.mode = mode;
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
            if proposal.structural_hunks.iter().all(|h| h.status == HunkStatus::Rejected) {
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

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

    fn make_pipeline(mode: WriteMode) -> (WriteGatePipeline, Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicU64>) {
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
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        assert!(result.is_none(), "Interactive mode should not return content");
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
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        assert!(result.is_some(), "AutoAccept should return content");
        let (path, content) = result.unwrap();
        assert_eq!(path, PathBuf::from("src/main.rs"));
        assert!(content.contains("new"));
        assert_eq!(finalized.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn test_reject_all_mode_discards() {
        let (mut pipeline, _c, _u, _f) = make_pipeline(WriteMode::RejectAll);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
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
            Path::new("src/main.rs"),
            "fn old() {}\n",
            "fn new() {}\n",
        );
        let result = pipeline.insert_proposal(proposal);
        assert!(result.is_none(), "Deferred mode should not return content");
        assert_eq!(created.load(Ordering::SeqCst), 0, "No observer notification in deferred mode");
        assert_eq!(pipeline.pending_proposals().len(), 1);

        // Add a second proposal
        let id2 = pipeline.next_id();
        let proposal2 = WriteGatePipeline::build_proposal(
            id2,
            "test-agent",
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
    fn test_accept_reject_hunks() {
        let (mut pipeline, _c, updated, _f) = make_pipeline(WriteMode::Interactive);
        let id = pipeline.next_id();
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
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
            Path::new("src/main.rs"),
            "aaa\nbbb\nccc\n",
            "aaa\nBBB\nccc\n",
        );
        pipeline.insert_proposal(proposal);
        pipeline.accept_all(id);

        let result = pipeline.finalize(id);
        assert!(result.is_some());
        let (_path, content) = result.unwrap();
        assert!(content.contains("BBB"));
        assert!(!content.contains("bbb"));
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
            Path::new("src/main.rs"),
            "aaa\nbbb\nccc\nddd\neee\nfff\n",
            "aaa\nBBB\nccc\nddd\nEEE\nfff\n",
        );
        pipeline.insert_proposal(proposal);

        pipeline.accept_hunk(id, 0); // BBB accepted
        pipeline.reject_hunk(id, 1); // EEE rejected, keep eee

        let result = pipeline.finalize(id).unwrap();
        let content = result.1;
        assert!(content.contains("BBB"));
        assert!(content.contains("eee"));
        assert!(!content.contains("bbb"));
        assert!(!content.contains("EEE"));
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

        let original = "fn foo() {\n    let x = 1;\n    let y = 2;\n}\n\nfn bar() {\n    let z = 3;\n}\n";
        let proposed = "fn foo() {\n    let x = 42;\n    let y = 2;\n}\n\nfn bar() {\n    let z = 99;\n}\n";
        let proposal = WriteGatePipeline::build_proposal(
            id,
            "test-agent",
            Path::new("src/main.rs"),
            original,
            proposed,
        );
        pipeline.insert_proposal(proposal);

        // Accept only hunks in "foo"
        pipeline.accept_node(id, "foo");

        let p = pipeline.get_proposal(id).unwrap();
        // The hunk in foo should be accepted
        let foo_hunks: Vec<_> = p.structural_hunks.iter()
            .filter(|h| h.enclosing_node.as_ref().and_then(|n| n.name.as_deref()) == Some("foo"))
            .collect();
        assert!(foo_hunks.iter().all(|h| h.status == HunkStatus::Accepted));
    }
}
