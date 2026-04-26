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

    /// Claude emitted its session id (first `SystemInit` event of a turn).
    /// The controller stores this on the matching `Conversation` so the
    /// next turn can pass `--resume <session_id>` and avoid re-sending
    /// conversation history + bootstrap context.
    ClaudeSessionStarted {
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
                    Ok(Some(_)) => {} // Ignore other event types
                    Ok(None) => {}    // No event in poll window
                    Err(_) => break,  // spawn_blocking failed
                }
            }
        });
    }

    /// Spawn a file-system watcher on the given paths.
    /// Returns the watcher handle — it must be kept alive for watching to continue.
    pub fn spawn_file_watcher(
        &self,
        paths: &[&Path],
    ) -> notify::Result<notify::RecommendedWatcher> {
        use notify::{RecursiveMode, Watcher, event::ModifyKind};

        let tx = self.tx.clone();
        let mut watcher = notify::RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    match event.kind {
                        notify::EventKind::Modify(ModifyKind::Data(_))
                        | notify::EventKind::Modify(ModifyKind::Any) => {
                            for path in event.paths {
                                let _ = tx.send(Event::FileChanged(path));
                            }
                        }
                        notify::EventKind::Create(_)
                        | notify::EventKind::Remove(_)
                        | notify::EventKind::Modify(ModifyKind::Name(_)) => {
                            let _ = tx.send(Event::FileTreeChanged);
                        }
                        _ => {}
                    }
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
