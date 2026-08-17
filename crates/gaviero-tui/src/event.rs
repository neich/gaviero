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

    /// The agent subprocess needs user approval to run a tool, or is asking
    /// clarifying questions via `AskUserQuestion`.
    /// The pipeline is suspended until the `respond` channel is used.
    PermissionRequest {
        conv_id: String,
        tool_name: String,
        description: String,
        input: serde_json::Value,
        /// Allow / deny (optionally with updated tool input for AskUserQuestion).
        respond: tokio::sync::oneshot::Sender<gaviero_core::observer::PermissionDecision>,
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
    SwarmLoopGateFailed {
        probe: String,
        status: String,
        output: String,
    },
    SwarmCostUpdate(gaviero_core::swarm::verify::CostEstimate),
    /// Coordinator produced a `.gaviero` DSL plan file ready for user review.
    /// The path is absolute. The user should review/edit it, then `/run` it.
    SwarmDslPlanReady(PathBuf),

    // Memory
    MemoryReady(Arc<gaviero_core::memory::MemoryStores>),

    // Remote sidecar (Plan A A3/A4): decoded, deduplicated, rate-limited
    // client commands plus connection lifecycle from the RemoteHub.
    RemoteCommand(Box<gaviero_remote::envelope::ClientEnvelope>),
    /// A client (re)connected and needs a full snapshot.
    RemoteSnapshotNeeded,
    RemoteClientConnected,
    RemoteClientDisconnected,

    // Internal
    Tick,
}

/// Path components that are always dropped on the file-watcher path,
/// regardless of `files.exclude`. These are directories whose contents are
/// virtually never useful as editor signals (build artefacts, VCS internals,
/// gaviero's own swarm worktrees) and would otherwise flood the unbounded
/// event channel during a build.
pub(crate) const ALWAYS_SKIP_COMPONENTS: &[&str] = &["target", "node_modules", ".git"];

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
        // matches the literal `build` path, not `build/output.txt`. Walk
        // every ancestor prefix of the relative path so a watcher event under
        // an excluded directory still gets dropped. Cheap because rel paths
        // inside a workspace are shallow (a handful of components at most).
        //
        // Candidates are built from components joined with '/': on Windows,
        // notify delivers native `\` separators, which the '/'-literal
        // pattern grammar would never match.
        let comps: Vec<&str> = rel
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .collect();
        for end in (1..=comps.len()).rev() {
            if crate::app::matches_exclude(&comps[..end].join("/"), exclude_patterns) {
                return true;
            }
        }
    }

    false
}

/// Read the crossterm events available after `poll` has reported one ready.
///
/// On Unix a paste already arrives as a single `Event::Paste`, so this is one
/// `read()`. On Windows, crossterm's console event source can never surface
/// `Event::Paste` (it builds only Key/Mouse/Resize/Focus events from console
/// input records — see `platform::set_bracketed_paste`). Pastes arrive as:
///
/// 1. **Bracketed-paste VT markers** (`ESC[200~…ESC[201~`) once we advertise
///    `?2004h` — Windows Terminal uses an *empty* payload for image-only
///    clipboards; we must recognize that and emit `Event::Paste("")` so the
///    chat path can attach the image.
/// 2. **Raw key bursts** (legacy / no BP) — newlines would otherwise submit
///    the chat on the first line, so we coalesce them into one `Event::Paste`.
///
/// Runtime-gated (`cfg!`) rather than compile-time-gated so the coalescer
/// builds and its tests run on every platform.
fn read_crossterm_batch() -> Vec<crossterm::event::Event> {
    use crossterm::event::{self, Event, KeyEventKind};

    let Ok(first) = event::read() else {
        return Vec::new();
    };
    if !cfg!(windows) {
        return vec![first];
    }

    // Bracketed-paste sequences begin with Esc (KeyCode::Esc). A lone Esc
    // (no follow-up within a short burst window) is a real keypress and must
    // pass through. Use PASTE_BURST_MS rather than ZERO — ConPTY often
    // delivers the `[200~` bytes a tick after Esc.
    if is_escape_press(&first) {
        return coalesce_bracketed_paste(first);
    }

    // Only an unmodified text key can begin a raw paste burst.
    let Some(first_ch) = paste_char(&first) else {
        return vec![first];
    };

    // A console paste is injected as a batch, so the *next* event is already
    // queued and a zero-wait poll succeeds. An isolated keystroke usually
    // leaves the queue empty and passes straight through — but a queued event
    // is only a hint, never proof of a paste: the key's own release, a mouse
    // move, or anything typed while the UI thread was busy sits in the same
    // console buffer. What the burst turns out to hold decides
    // (`looks_like_raw_paste`), not the fact that it was drainable.
    if !event::poll(Duration::ZERO).unwrap_or(false) {
        return vec![first];
    }

    // Drain the burst into one string, keeping the key events themselves so a
    // burst that turns out to be typing can be replayed verbatim. Windows
    // emits a press + release per key; the releases are skipped. The first
    // genuine non-text event ends the burst and is re-emitted after it.
    let mut text = String::from(first_ch);
    let mut keys = vec![first];
    let mut trailing: Option<Event> = None;
    while event::poll(Duration::from_millis(crate::theme::PASTE_BURST_MS)).unwrap_or(false) {
        let Ok(next) = event::read() else {
            break;
        };
        if let Some(ch) = paste_char(&next) {
            text.push(ch);
            keys.push(next);
        } else if matches!(&next, Event::Key(k) if k.kind == KeyEventKind::Release) {
            continue;
        } else {
            trailing = Some(next);
            break;
        }
    }

    let mut out = if looks_like_raw_paste(&text) {
        vec![Event::Paste(text)]
    } else {
        // Ordinary typing the console happened to hand us in one batch — every
        // drained key must still be delivered as a key (dropping them here ate
        // keystrokes; rewriting them as a `Paste` inserted the clipboard).
        keys
    };
    out.extend(trailing);
    out
}

/// Whether a drained raw key burst should be rewritten as one `Event::Paste`.
///
/// "Arrived back-to-back" does **not** mean "injected by a paste". The console
/// input buffer holds every record typed while the UI thread was busy, so a
/// fast two-key roll during a redraw reaches us with the same zero gap as a
/// console-injected paste, and key auto-repeat delivers a long run the same
/// way. Coalescing those is not harmless: the paste path arms the debounce and
/// `WINDOWS_PASTE_SETTLE_MS` (swallowing the next keystrokes) and, on Windows,
/// used to substitute the whole system clipboard for the burst.
///
/// Rewriting only *buys* anything for bursts long enough that replaying them
/// key-by-key misbehaves (an embedded newline submitting the chat mid-paste),
/// so require a length no typing roll reaches and reject a single repeated
/// character. Everything below that replays as keys, which types the identical
/// text. Bracketed paste (`ESC[200~…`) is unaffected — it is self-delimiting
/// and handled by [`coalesce_bracketed_paste`].
fn looks_like_raw_paste(text: &str) -> bool {
    let mut rest = text.chars();
    let Some(first) = rest.next() else {
        return false;
    };
    text.chars().count() >= crate::theme::RAW_PASTE_MIN_CHARS && rest.any(|c| c != first)
}

const BP_START: &str = "\x1b[200~";
const BP_END: &str = "\x1b[201~";

/// After an Esc press, drain a possible `ESC[200~…ESC[201~` burst into
/// `Event::Paste`. Empty payload (image-only clipboard under Windows Terminal)
/// is a successful paste — callers attach the clipboard image.
fn coalesce_bracketed_paste(first: crossterm::event::Event) -> Vec<crossterm::event::Event> {
    use crossterm::event::{self, Event, KeyEventKind};

    // Wait briefly for `[200~…` — ConPTY may not have the next record ready
    // at ZERO, and returning lone Esc would drop the image-paste signal.
    if !event::poll(Duration::from_millis(crate::theme::PASTE_BURST_MS)).unwrap_or(false) {
        return vec![first];
    }

    let mut text = String::from('\x1b');
    let mut keys = vec![first];
    let mut trailing: Option<Event> = None;
    // Once we see the start marker, keep draining until the end marker even
    // across brief gaps — large text pastes still arrive as one burst, but
    // the end marker must not be lost to a tight timeout. ConPTY can stall
    // tens of ms mid-paste under load; giving up early splits one gesture
    // into multiple Event::Paste chunks (see paste_text cursor note).
    let mut saw_start = false;
    loop {
        let Ok(next) = event::read() else {
            break;
        };
        if let Some(ch) = paste_or_escape_char(&next) {
            text.push(ch);
            keys.push(next);
            if !saw_start && text.starts_with(BP_START) {
                saw_start = true;
            }
            if saw_start && text.contains(BP_END) {
                while event::poll(Duration::ZERO).unwrap_or(false) {
                    let Ok(extra) = event::read() else {
                        break;
                    };
                    if matches!(&extra, Event::Key(k) if k.kind == KeyEventKind::Release) {
                        continue;
                    }
                    trailing = Some(extra);
                    break;
                }
                break;
            }
        } else if matches!(&next, Event::Key(k) if k.kind == KeyEventKind::Release) {
            // keep draining
        } else if saw_start {
            // Inside a bracketed-paste payload: ignore non-text keys (focus,
            // mouse noise, odd VK mappings for `~` on some layouts) rather
            // than aborting — aborting early left payloads truncated at the
            // first `~` and armed the Windows settle window that then dropped
            // the remaining characters as stragglers.
            continue;
        } else {
            trailing = Some(next);
            break;
        }

        let wait_ms = if saw_start && !text.contains(BP_END) {
            // Still inside a bracketed paste — wait much longer for the next
            // byte / end marker than for a raw key-burst boundary.
            crate::theme::PASTE_BURST_MS.saturating_mul(40).max(100)
        } else {
            crate::theme::PASTE_BURST_MS
        };
        if !event::poll(Duration::from_millis(wait_ms)).unwrap_or(false) {
            break;
        }
    }

    let mut out = if let Some(payload) = strip_bracketed_paste(&text) {
        vec![Event::Paste(payload.to_string())]
    } else if !text.starts_with(BP_START) {
        // No start marker: an Esc keypress, possibly followed by ordinary
        // typing the console had already buffered. Replay every drained key —
        // rewriting them as a `Paste` (or returning the lone Esc and dropping
        // the rest) is how typing right after Esc turned into a clipboard
        // dump / vanished characters.
        let rest = &text[text.char_indices().nth(1).map(|(i, _)| i).unwrap_or(text.len())..];
        if looks_like_raw_paste(rest) {
            keys.truncate(1);
            keys.push(Event::Paste(rest.to_string()));
        }
        keys
    } else {
        // Start marker seen but the end marker never arrived (mid-burst
        // timeout) — the payload is still a genuine paste.
        let inner = text.strip_prefix(BP_START).unwrap_or(&text);
        vec![Event::Paste(inner.to_string())]
    };
    out.extend(trailing);
    out
}

/// If `raw` is a complete bracketed-paste sequence, return the inner payload
/// (may be empty). Used by the Windows coalescer and unit-tested directly.
fn strip_bracketed_paste(raw: &str) -> Option<&str> {
    let rest = raw.strip_prefix(BP_START)?;
    let end = rest.find(BP_END)?;
    if end + BP_END.len() != rest.len() {
        // Trailing junk after the end marker — still accept the payload.
        return Some(&rest[..end]);
    }
    Some(&rest[..end])
}

fn is_escape_press(event: &crossterm::event::Event) -> bool {
    use crossterm::event::{Event, KeyCode, KeyEventKind};
    matches!(
        event,
        Event::Key(k) if k.kind != KeyEventKind::Release && k.code == KeyCode::Esc
    )
}

/// Like [`paste_char`], but also maps Esc → `\x1b` so bracketed-paste markers
/// can be assembled from the key burst Windows Terminal injects.
fn paste_or_escape_char(event: &crossterm::event::Event) -> Option<char> {
    if let Some(c) = paste_char(event) {
        return Some(c);
    }
    if is_escape_press(event) {
        return Some('\x1b');
    }
    None
}

/// Character a key event contributes to a pasted burst, or `None` if it cannot
/// be part of a paste. Enter → `\n`, Tab → `\t`, plain chars → themselves;
/// releases, real Ctrl/Alt/Super chords, and navigation keys map to `None`.
/// Shift is allowed so pasted capitals survive.
///
/// **AltGr (Windows):** reported as `CONTROL|ALT`. ConPTY paste injection of
/// characters that are AltGr-produced on the active layout (notably `~` on
/// many European layouts) arrives with those modifiers. Rejecting them ended
/// the raw-paste coalescer at the first `~` (e.g. `topology: ~600` →
/// `topology: `), after which `WINDOWS_PASTE_SETTLE_MS` dropped the rest.
fn paste_char(event: &crossterm::event::Event) -> Option<char> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key
        .modifiers
        .intersects(KeyModifiers::SUPER | KeyModifiers::META)
    {
        return None;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    // AltGr == Ctrl+Alt on Windows. Printable non-lowercase chars with that
    // chord are paste text (same rule psmux uses); lowercase C-M-x is a real
    // chord and must not be swallowed into a paste burst.
    let altgr = ctrl && alt;

    match key.code {
        KeyCode::Char(c) if altgr => {
            if c.is_ascii_lowercase() {
                None
            } else {
                Some(c)
            }
        }
        KeyCode::Char(_) | KeyCode::Enter | KeyCode::Tab if ctrl || alt => None,
        KeyCode::Char(c) => Some(c),
        KeyCode::Enter => Some('\n'),
        KeyCode::Tab => Some('\t'),
        _ => None,
    }
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
            'outer: loop {
                // Poll + read in a blocking thread. On Windows the read step
                // coalesces paste bursts into a synthetic `Event::Paste`
                // (see `read_crossterm_batch`), so one poll can yield several
                // logical events; on Unix the batch holds a single event.
                let batch = tokio::task::spawn_blocking(|| {
                    if crossterm::event::poll(Duration::from_millis(
                        crate::theme::CROSSTERM_POLL_MS,
                    ))
                    .unwrap_or(false)
                    {
                        read_crossterm_batch()
                    } else {
                        Vec::new()
                    }
                })
                .await;

                let batch = match batch {
                    Ok(batch) => batch,
                    Err(_) => break, // spawn_blocking failed
                };

                for event in batch {
                    match event {
                        crossterm::event::Event::Key(key) => {
                            // Windows crossterm emits Press AND Release for
                            // every keystroke — forwarding both double-fires
                            // all input (Tier W1 / PR-5). Keep Repeat: held
                            // keys must still repeat in editor/terminal
                            // panes. Unix emits Press only; unaffected.
                            if key.kind == crossterm::event::KeyEventKind::Release {
                                continue;
                            }
                            if tx.send(Event::Key(key)).is_err() {
                                break 'outer;
                            }
                        }
                        crossterm::event::Event::Mouse(mouse) => {
                            if tx.send(Event::Mouse(mouse)).is_err() {
                                break 'outer;
                            }
                        }
                        crossterm::event::Event::Resize(w, h) => {
                            if tx.send(Event::Resize(w, h)).is_err() {
                                break 'outer;
                            }
                        }
                        crossterm::event::Event::Paste(text) => {
                            if tx.send(Event::Paste(text)).is_err() {
                                break 'outer;
                            }
                        }
                        crossterm::event::Event::FocusGained => {
                            if tx.send(Event::TerminalFocus(true)).is_err() {
                                break 'outer;
                            }
                        }
                        crossterm::event::Event::FocusLost => {
                            if tx.send(Event::TerminalFocus(false)).is_err() {
                                break 'outer;
                            }
                        }
                    }
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
            // After a UI stall, resume with at most one pending tick instead
            // of tokio's default burst of catch-up ticks — missed heartbeats
            // carry no information and only flood the unbounded channel.
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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

    /// Real notify events on Windows carry native `\` separators (paths are
    /// rebuilt from FILE_NOTIFY_INFORMATION); exclude patterns must still
    /// match them.
    #[cfg(windows)]
    #[test]
    fn path_is_excluded_matches_windows_native_separators() {
        let root = PathBuf::from(r"C:\ws");
        let roots = vec![root.clone()];
        let patterns = vec!["**/*.log".to_string(), "build/".to_string()];
        assert!(path_is_excluded(
            &PathBuf::from(r"C:\ws\a\b\c.log"),
            &roots,
            &patterns,
        ));
        assert!(path_is_excluded(
            &PathBuf::from(r"C:\ws\build\output.txt"),
            &roots,
            &patterns,
        ));
        assert!(!path_is_excluded(
            &PathBuf::from(r"C:\ws\src\main.rs"),
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
    fn paste_char_maps_text_keys_and_rejects_chords() {
        use crossterm::event::{
            Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
        };

        let key = |code, mods, kind| {
            Event::Key(KeyEvent {
                code,
                modifiers: mods,
                kind,
                state: KeyEventState::NONE,
            })
        };
        let press = |code, mods| key(code, mods, KeyEventKind::Press);

        // Text-bearing keys contribute their character; Enter/Tab normalize.
        assert_eq!(paste_char(&press(KeyCode::Char('a'), KeyModifiers::NONE)), Some('a'));
        assert_eq!(paste_char(&press(KeyCode::Enter, KeyModifiers::NONE)), Some('\n'));
        assert_eq!(paste_char(&press(KeyCode::Tab, KeyModifiers::NONE)), Some('\t'));
        // Shift is part of pasted capitals, not a chord.
        assert_eq!(paste_char(&press(KeyCode::Char('A'), KeyModifiers::SHIFT)), Some('A'));

        // Ctrl/Alt chords and navigation keys are never pasted text.
        assert_eq!(paste_char(&press(KeyCode::Char('v'), KeyModifiers::CONTROL)), None);
        assert_eq!(paste_char(&press(KeyCode::Enter, KeyModifiers::ALT)), None);
        assert_eq!(paste_char(&press(KeyCode::Left, KeyModifiers::NONE)), None);
        // AltGr (Ctrl+Alt) printable chars — including `~` on many layouts —
        // MUST be paste text; rejecting them truncated pastes at the first tilde.
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(paste_char(&press(KeyCode::Char('~'), altgr)), Some('~'));
        assert_eq!(paste_char(&press(KeyCode::Char('@'), altgr)), Some('@'));
        assert_eq!(paste_char(&press(KeyCode::Char('\\'), altgr)), Some('\\'));
        // Real Ctrl+Alt+letter chords stay rejected.
        assert_eq!(paste_char(&press(KeyCode::Char('c'), altgr)), None);
        // Release halves of pasted keys must not contribute a character.
        assert_eq!(
            paste_char(&key(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Release)),
            None
        );
    }

    #[test]
    fn strip_bracketed_paste_extracts_payload_including_empty() {
        // Windows Terminal image paste: empty bracketed-paste payload.
        assert_eq!(strip_bracketed_paste("\x1b[200~\x1b[201~"), Some(""));
        assert_eq!(strip_bracketed_paste("\x1b[200~hello\x1b[201~"), Some("hello"));
        assert_eq!(
            strip_bracketed_paste("\x1b[200~a\nb\x1b[201~"),
            Some("a\nb")
        );
        // Payload may contain `~` — only the full `ESC[201~` end marker closes.
        assert_eq!(
            strip_bracketed_paste("\x1b[200~hello~world\x1b[201~"),
            Some("hello~world")
        );
        assert_eq!(
            strip_bracketed_paste("\x1b[200~path=~/foo\x1b[201~"),
            Some("path=~/foo")
        );
        assert_eq!(
            strip_bracketed_paste(
                "\x1b[200~  topology: ~600 tok (ceiling 600)\n  outline: ~1200 tok\x1b[201~"
            ),
            Some("  topology: ~600 tok (ceiling 600)\n  outline: ~1200 tok")
        );
        // Not a BP sequence.
        assert_eq!(strip_bracketed_paste("hello"), None);
        assert_eq!(strip_bracketed_paste("\x1b[200~no-end"), None);
        assert_eq!(strip_bracketed_paste(""), None);
    }

    #[test]
    fn looks_like_raw_paste_rejects_typing_and_key_repeat() {
        // A fast roll (or keys buffered while the UI was busy) arrives with
        // the same zero gap as an injected paste — length is the only signal.
        assert!(!looks_like_raw_paste("ab"));
        assert!(!looks_like_raw_paste("the quick"));
        // Enter typed at the end of a roll must still submit, not paste.
        assert!(!looks_like_raw_paste("ok\n"));
        // Key auto-repeat: one character, however long the run.
        assert!(!looks_like_raw_paste(&"a".repeat(64)));
        assert!(!looks_like_raw_paste(""));

        // A real legacy (non-bracketed) paste.
        assert!(looks_like_raw_paste("cargo test -p gaviero-tui"));
        assert!(looks_like_raw_paste("line one\nline two\n"));
    }

    #[test]
    fn paste_or_escape_char_maps_esc() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(paste_or_escape_char(&esc), Some('\x1b'));
        assert!(is_escape_press(&esc));
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
