use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use gaviero_core::terminal::TerminalEvent;
use gaviero_core::types::WriteProposal;

#[derive(Debug)]
#[allow(dead_code)] // Swarm variants are wired but not yet launched from TUI
pub enum Event {
    // Terminal input
    Key(crossterm::event::KeyEvent),
    Mouse(crossterm::event::MouseEvent),
    Paste(String),
    Resize(u16, u16),
    /// Host terminal gained or lost OS window focus (crossterm focus-change).
    TerminalFocus(bool),

    // Filesystem
    FileChanged(PathBuf),
    /// A file or directory was created, deleted, or renamed — refresh the tree.
    FileTreeChanged,
    /// Terminal event from the bounded PTY channel.
    Terminal(TerminalEvent),

    // Write gate events — proposal carries the full data so the TUI never
    // needs to lock the write gate to read it.
    ProposalCreated(Box<WriteProposal>),
    ProposalUpdated(u64),
    /// Gate pushed new `conflicts_with` / supersede state for a proposal
    /// already visible in batch review (deferred when the gate lock was busy).
    BatchProposalSynced {
        id: u64,
        conflicts_with: Vec<u64>,
        superseded: bool,
    },
    ProposalFinalized(String), // file path

    // ACP agent events (conv_id identifies which conversation)
    StreamChunk {
        conv_id: String,
        text: String,
    },
    ToolCallStarted {
        conv_id: String,
        tool_name: String,
    },
    StreamingStatus {
        conv_id: String,
        status: String,
    },
    MessageComplete {
        conv_id: String,
        role: String,
        content: String,
    },

    /// A file proposal was deferred (batch review mode) — show compact summary in chat.
    FileProposalDeferred {
        conv_id: String,
        path: PathBuf,
        additions: usize,
        deletions: usize,
    },

    /// The agent subprocess needs user approval to run a tool.
    /// The pipeline is suspended until the `respond` channel is used.
    PermissionRequest {
        conv_id: String,
        tool_name: String,
        description: String,
        /// Send `true` to allow, `false` (or drop) to deny.
        respond: tokio::sync::oneshot::Sender<bool>,
    },

    /// All file proposals from an agent response are ready for batch review.
    /// Fired when streaming ends and there are pending deferred proposals.
    AcpTaskCompleted {
        conv_id: String,
        proposals: Vec<WriteProposal>,
    },

    /// Chat agent turn fully finished (success, error, or user cancel).
    /// Fired once per `send_chat_message` spawn after the session closes and
    /// deferred proposals are drained — model-agnostic completion hook.
    AgentTurnFinished {
        conv_id: String,
        cancelled: bool,
        error: Option<String>,
        proposal_count: usize,
    },

    /// Claude emitted its session id (first `SystemInit` event of a turn).
    /// The controller stores this on the matching `Conversation` so the
    /// next turn can pass `--resume <session_id>` and avoid re-sending
    /// conversation history + bootstrap context.
    ClaudeSessionStarted {
        conv_id: String,
        session_id: String,
    },

    /// Cursor emitted its chat / thread id (first `system.init` event of a
    /// turn). The controller stores it on the `Conversation`'s
    /// `SessionLedger` as a `ContinuityHandle::CursorThreadId` so the next
    /// turn can pass `--resume <session_id>` and avoid re-sending
    /// conversation history.
    CursorSessionStarted {
        conv_id: String,
        session_id: String,
    },

    /// Fired once per chat turn after `retrieve_for_chat` selects the memories
    /// that will be spliced into the prompt. Summary is surfaced in the
    /// status bar and (Tier A4) the memory panel. Mirrors S4's manifest data
    /// at a coarser granularity — keeps the per-candidate pool off the UI
    /// event path.
    ChatMemoryInjected {
        conv_id: String,
        items_injected: usize,
        pool_size: usize,
        tokens_used: usize,
        token_budget: usize,
    },

    /// Measured gaviero bootstrap tokens for the turn about to be sent
    /// (topology + graph outline + memory injection + `@file` refs).
    /// Drives the status-bar composite estimate until provider usage arrives.
    TurnBootstrapMeasured {
        conv_id: String,
        tokens: usize,
        arms: gaviero_core::context_planner::BootstrapArms,
    },

    /// Fired once per chat turn with the provider's authoritative token
    /// usage (Claude `result.usage` today). The controller stores the
    /// latest reading on the matching conversation so the status bar can
    /// show real context-window pressure (`prefix_tokens()`).
    TurnTokenUsage {
        conv_id: String,
        usage: gaviero_core::acp::protocol::TokenUsage,
    },

    /// Accumulated USD cost for an in-process tool-agent turn (DeepSeek).
    TurnCostUpdate {
        conv_id: String,
        cost_usd: f64,
    },

    /// Option-B write tool snapshotted a path mid-turn (before the watcher fires).
    ToolAgentEditCaptured {
        path: std::path::PathBuf,
        pre_turn_content: Option<String>,
    },

    /// In-process tool-agent (DeepSeek) finished a turn with on-disk edits.
    /// The controller opens external-change review for the first touched file
    /// and stores pre-turn snapshots for revert-on-reject.
    ToolAgentEditsPending {
        conv_id: String,
        edits: Vec<gaviero_core::observer::ToolAgentEdit>,
    },

    /// A4: writer task enqueued a write. Panel counts events for the
    /// "activity" pulse indicator but does not re-query yet.
    MemoryWriteEnqueued {
        kind: String,
    },
    /// A4: writer task committed a write. Triggers a debounced panel
    /// refresh of the "Recently Written" section.
    MemoryWriteCommitted {
        kind: String,
    },
    /// A4: writer task failed. Logs to status bar and panel.
    MemoryWriteFailed {
        kind: String,
        error: String,
    },
    /// A4: writer task persisted an `injection_manifests` row. Panel
    /// re-queries the row for the "Injected Now" section.
    MemoryManifestPersisted {
        turn_id: String,
        session_id: String,
    },
    /// A5: read-only MCP tool activity from the in-process server.
    McpToolCall {
        tool_name: String,
        duration_ms: u64,
        error: Option<String>,
    },

    /// A4: live-search results from the panel's spawned query. Receiver
    /// overwrites `MemoryPanelState::search_results` and resets the
    /// cursor to 0.
    MemorySearchResults {
        rows: Vec<crate::panels::memory_panel::MemoryRow>,
    },

    /// A4: history overlay fill — last N manifests across all sessions.
    MemoryHistoryRows {
        rows: Vec<gaviero_core::memory::store::InjectionManifestRow>,
    },

    /// A4: resolved `selected_ids` for the current manifest, loaded
    /// from the memories table. Populates "Injected Now" section body.
    MemorySelectedItems {
        rows: Vec<crate::panels::memory_panel::MemoryRow>,
    },

    /// A4: current manifest row re-fetched after `MemoryManifestPersisted`.
    MemoryManifestReady {
        row: gaviero_core::memory::store::InjectionManifestRow,
    },

    /// A4: per-scope counts + last-write timestamps for Section 3.
    MemoryScopeSummary {
        rows: Vec<crate::panels::memory_panel::ScopeSummaryRow>,
    },

    /// C2.6: loaded audit rows for the Deletions tab. Receiver
    /// overwrites `MemoryPanelState::deletions_rows` and resets the
    /// cursor to 0.
    MemoryDeletionsLoaded {
        rows: Vec<crate::panels::memory_panel::DeletionRow>,
    },

    // Swarm events (constructed by TuiSwarmObserver when swarm is launched)
    SwarmPhaseChanged(String),
    SwarmAgentStateChanged {
        id: String,
        status: gaviero_core::swarm::models::AgentStatus,
        detail: String,
    },
    SwarmTierStarted {
        current: usize,
        total: usize,
    },
    SwarmCompleted(Box<gaviero_core::swarm::models::SwarmResult>),
    SwarmMergeConflict {
        branch: String,
        files: Vec<String>,
    },

    // Coordination lifecycle events
    SwarmCoordinationStarted(String),
    SwarmCoordinationComplete {
        unit_count: usize,
        summary: String,
    },
    SwarmTierDispatch {
        unit_id: String,
        tier: gaviero_core::types::ModelTier,
        backend: String,
    },
    SwarmCostUpdate(gaviero_core::swarm::verify::CostEstimate),
    /// Coordinator produced a `.gaviero` DSL plan file ready for user review.
    /// The path is absolute. The user should review/edit it, then `/run` it.
    SwarmDslPlanReady(PathBuf),

    // Memory
    MemoryReady(Arc<gaviero_core::memory::MemoryStores>),

    // Internal
    Tick,
}

/// Path components that are always dropped on the file-watcher path,
/// regardless of `files.exclude`. These are directories whose contents are
/// virtually never useful as editor signals (build artefacts, VCS internals,
/// gaviero's own swarm worktrees) and would otherwise flood the unbounded
/// event channel during a build.
const ALWAYS_SKIP_COMPONENTS: &[&str] = &["target", "node_modules", ".git"];

/// Decide whether a notify event path should be dropped before it reaches
/// the main loop. Skips paths under one of `ALWAYS_SKIP_COMPONENTS`,
/// gaviero's own `.gaviero/worktrees/` subtree, or any user `files.exclude`
/// pattern. Paths outside every workspace root pass through unchanged —
/// notify can deliver `~/.cache/...` events on Linux and we don't want to
/// silently swallow them.
fn path_is_excluded(path: &Path, roots: &[PathBuf], exclude_patterns: &[String]) -> bool {
    let rel = roots
        .iter()
        .filter_map(|root| path.strip_prefix(root).ok().map(|r| (root, r)))
        .max_by_key(|(root, _)| root.as_os_str().len())
        .map(|(_, rel)| rel);
    let Some(rel) = rel else {
        return false;
    };

    let mut saw_dot_gaviero = false;
    for component in rel.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        let Some(name) = name.to_str() else { continue };
        if ALWAYS_SKIP_COMPONENTS.contains(&name) {
            return true;
        }
        if saw_dot_gaviero && name == "worktrees" {
            return true;
        }
        saw_dot_gaviero = name == ".gaviero";
    }

    if !exclude_patterns.is_empty() {
        // `matches_exclude` is a leaf matcher: a pattern like `build/` only
        // matches the literal `build` path, not `build/output.txt`. Walk the
        // relative path up so a watcher event under an excluded directory
        // still gets dropped. Cheap because rel paths inside a workspace are
        // shallow (a handful of components at most).
        let mut current: Option<&Path> = Some(rel);
        while let Some(p) = current {
            if !p.as_os_str().is_empty() {
                let s = p.to_string_lossy();
                if crate::app::matches_exclude(&s, exclude_patterns) {
                    return true;
                }
            }
            current = p.parent();
        }
    }

    false
}

pub struct EventLoop {
    tx: mpsc::UnboundedSender<Event>,
    rx: Option<mpsc::UnboundedReceiver<Event>>,
}

impl EventLoop {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx: Some(rx) }
    }

    pub fn tx(&self) -> mpsc::UnboundedSender<Event> {
        self.tx.clone()
    }

    pub fn take_rx(&mut self) -> mpsc::UnboundedReceiver<Event> {
        self.rx
            .take()
            .expect("EventLoop::take_rx called more than once")
    }

    /// Spawn a background task that reads crossterm events and sends them.
    pub fn spawn_crossterm_reader(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                // Poll crossterm events in a blocking thread
                let event = tokio::task::spawn_blocking(|| {
                    if crossterm::event::poll(Duration::from_millis(
                        crate::theme::CROSSTERM_POLL_MS,
                    ))
                    .unwrap_or(false)
                    {
                        crossterm::event::read().ok()
                    } else {
                        None
                    }
                })
                .await;

                match event {
                    Ok(Some(crossterm::event::Event::Key(key))) => {
                        if tx.send(Event::Key(key)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(crossterm::event::Event::Mouse(mouse))) => {
                        if tx.send(Event::Mouse(mouse)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(crossterm::event::Event::Resize(w, h))) => {
                        if tx.send(Event::Resize(w, h)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(crossterm::event::Event::Paste(text))) => {
                        if tx.send(Event::Paste(text)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(crossterm::event::Event::FocusGained)) => {
                        if tx.send(Event::TerminalFocus(true)).is_err() {
                            break;
                        }
                    }
                    Ok(Some(crossterm::event::Event::FocusLost)) => {
                        if tx.send(Event::TerminalFocus(false)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {} // No event in poll window
                    Err(_) => break,  // spawn_blocking failed
                }
            }
        });
    }

    /// Spawn a file-system watcher on the given paths.
    ///
    /// `exclude_patterns` is the resolved `files.exclude` set (gitignore-style
    /// patterns). Any event whose path matches a pattern, or whose path
    /// contains one of the always-skip components below, is dropped before it
    /// reaches the unified event channel. Without this filter, a single
    /// `cargo test` writes thousands of files under `target/` and floods the
    /// main loop into apparent freeze.
    ///
    /// Returns the watcher handle — it must be kept alive for watching to
    /// continue.
    pub fn spawn_file_watcher(
        &self,
        paths: &[&Path],
        exclude_patterns: Vec<String>,
    ) -> notify::Result<notify::RecommendedWatcher> {
        use notify::{RecursiveMode, Watcher, event::ModifyKind};

        let tx = self.tx.clone();
        let roots: Vec<PathBuf> = paths.iter().map(|p| p.to_path_buf()).collect();
        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                let Ok(event) = res else { return };
                match event.kind {
                    notify::EventKind::Modify(ModifyKind::Data(_))
                    | notify::EventKind::Modify(ModifyKind::Any) => {
                        for path in event.paths {
                            if path_is_excluded(&path, &roots, &exclude_patterns) {
                                continue;
                            }
                            let _ = tx.send(Event::FileChanged(path));
                        }
                    }
                    notify::EventKind::Create(_)
                    | notify::EventKind::Remove(_)
                    | notify::EventKind::Modify(ModifyKind::Name(_)) => {
                        for path in &event.paths {
                            if gaviero_core::skills::SkillCatalog::needs_rebuild(path) {
                                let _ = tx.send(Event::FileChanged(path.clone()));
                            }
                        }
                        // FileTreeChanged carries no path, so coalesce: drop the
                        // event entirely if every reported path is excluded.
                        let any_visible = event
                            .paths
                            .iter()
                            .any(|p| !path_is_excluded(p, &roots, &exclude_patterns));
                        if any_visible {
                            let _ = tx.send(Event::FileTreeChanged);
                        }
                    }
                    _ => {}
                }
            },
            notify::Config::default(),
        )?;

        for path in paths {
            if let Err(e) = watcher.watch(path, RecursiveMode::Recursive) {
                tracing::warn!("failed to watch {}: {e}", path.display());
            }
        }

        Ok(watcher)
    }

    /// Spawn a bridge that forwards terminal events into the unified TUI event channel.
    pub fn spawn_terminal_bridge(
        &self,
        mut terminal_rx: tokio::sync::mpsc::Receiver<TerminalEvent>,
    ) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            while let Some(term_event) = terminal_rx.recv().await {
                if tx.send(Event::Terminal(term_event)).is_err() {
                    break;
                }
            }
        });
    }

    /// Spawn a tick timer (~30fps).
    pub fn spawn_tick_timer(&self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_millis(crate::theme::TICK_INTERVAL_MS));
            loop {
                interval.tick().await;
                if tx.send(Event::Tick).is_err() {
                    break;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_excluded_skips_target_dir() {
        let root = PathBuf::from("/ws");
        let roots = vec![root.clone()];
        assert!(path_is_excluded(
            &root.join("target/debug/build/foo.rmeta"),
            &roots,
            &[],
        ));
        assert!(path_is_excluded(
            &root.join("crates/x/target/release/x"),
            &roots,
            &[],
        ));
    }

    #[test]
    fn path_is_excluded_skips_dot_git_and_node_modules() {
        let root = PathBuf::from("/ws");
        let roots = vec![root.clone()];
        assert!(path_is_excluded(&root.join(".git/objects/abc"), &roots, &[]));
        assert!(path_is_excluded(
            &root.join("node_modules/foo/index.js"),
            &roots,
            &[],
        ));
    }

    #[test]
    fn path_is_excluded_skips_gaviero_worktrees_only() {
        let root = PathBuf::from("/ws");
        let roots = vec![root.clone()];
        // gaviero's own swarm worktrees → drop.
        assert!(path_is_excluded(
            &root.join(".gaviero/worktrees/abc/src/lib.rs"),
            &roots,
            &[],
        ));
        // Other .gaviero contents (settings, code_graph.db) are real signals.
        assert!(!path_is_excluded(
            &root.join(".gaviero/settings.json"),
            &roots,
            &[],
        ));
    }

    #[test]
    fn path_is_excluded_honors_user_patterns() {
        let root = PathBuf::from("/ws");
        let roots = vec![root.clone()];
        let patterns = vec!["**/*.log".to_string(), "build/".to_string()];
        assert!(path_is_excluded(&root.join("a/b/c.log"), &roots, &patterns));
        assert!(path_is_excluded(
            &root.join("build/output.txt"),
            &roots,
            &patterns,
        ));
        assert!(!path_is_excluded(
            &root.join("src/main.rs"),
            &roots,
            &patterns,
        ));
    }

    #[test]
    fn path_is_excluded_passes_through_paths_outside_roots() {
        let roots = vec![PathBuf::from("/ws")];
        // Notify on Linux can deliver `~/.cache/...` events; we don't want
        // to silently drop them just because they're not under a root.
        assert!(!path_is_excluded(
            &PathBuf::from("/home/u/.cache/gaviero/log"),
            &roots,
            &[],
        ));
    }

    #[test]
    fn path_is_excluded_passes_through_normal_source_files() {
        let root = PathBuf::from("/ws");
        let roots = vec![root.clone()];
        assert!(!path_is_excluded(
            &root.join("crates/foo/src/lib.rs"),
            &roots,
            &[],
        ));
        assert!(!path_is_excluded(&root.join("README.md"), &roots, &[]));
    }
}
