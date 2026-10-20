use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

// Used by the leaked-report decoder below (`decode_mouse_report_run`), which
// rebuilds the mouse events crossterm's Windows source can no longer deliver.
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use gaviero_core::terminal::TerminalEvent;
use gaviero_core::types::WriteProposal;

#[derive(Debug)]
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
    /// Files or directories were created, deleted, or renamed — refresh the
    /// tree and update the code graph. Carries the non-excluded paths (both
    /// sides of a rename when the platform reports them).
    FileTreeChanged(Vec<PathBuf>),
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
    /// Claude / Cursor / Codex Task/Agent launched in the background.
    BackgroundTaskStarted {
        conv_id: String,
        task_id: String,
        description: String,
    },
    /// Background Task/Agent reached a terminal state (or was killed on parent exit).
    BackgroundTaskFinished {
        conv_id: String,
        task_id: String,
        status: String,
        summary: String,
    },
    MessageComplete {
        conv_id: String,
        role: String,
        content: String,
    },
    /// The provider reported its session model's real context window.
    ContextWindow {
        conv_id: String,
        tokens: u64,
    },
    /// The provider compacted the session's context (`/compact` or its own
    /// automatic compaction).
    ContextCompacted {
        conv_id: String,
        trigger: String,
        pre_tokens: Option<u64>,
        post_tokens: Option<u64>,
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

    /// A tool-agent turn finished; its edits are already on disk.
    ///
    /// `paths` is the full set the turn wrote, which is the only point at
    /// which the editor can learn about files it does not hold open.
    /// Reconciliation is read-only, so nothing is carried to revert to --
    /// undoing a turn is the harness's job, not a per-file editor action.
    ToolAgentEditsPending {
        paths: Vec<std::path::PathBuf>,
    },

    /// A chat turn changed files on disk (host-side turn capture). Its
    /// review is mandatory before the conversation's next prompt.
    /// `overlapped` lists other pending reviews that gained an overlap mark.
    TurnReviewPending {
        review: gaviero_core::turn_capture::PendingReview,
        overlapped: Vec<String>,
    },

    /// A4: writer task enqueued a write. Panel counts events for the
    /// "activity" pulse indicator but does not re-query yet.
    MemoryWriteEnqueued,
    /// A4: writer task committed a write. Triggers a debounced panel
    /// refresh of the "Recently Written" section.
    MemoryWriteCommitted,
    /// A4: writer task failed. Logs to status bar and panel.
    MemoryWriteFailed {
        kind: String,
        error: String,
    },
    /// A4: writer task persisted an `injection_manifests` row. Panel
    /// re-queries the row for the "Injected Now" section.
    MemoryManifestPersisted {
        turn_id: String,
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

    /// HISTORY panel: the log was read off the event loop. Receiver installs
    /// it, keeping the selected turn when it still exists.
    HistoryLoaded(Box<crate::panels::history_panel::HistoryLoad>),

    // Memory
    MemoryReady(Arc<gaviero_core::memory::MemoryStores>),

    // Remote sidecar (Plan A A3/A4): decoded, deduplicated, rate-limited
    // client commands plus connection lifecycle from the RemoteHub.
    RemoteCommand(Box<gaviero_remote::envelope::ClientEnvelope>),
    /// A client (re)connected and needs a full snapshot.
    RemoteSnapshotNeeded,
    RemoteClientConnected,
    RemoteClientDisconnected,
    /// Plan C §2.2: the background bootstrap finished — the sidecar is
    /// listening and this carries its handle plus everything `/remote`
    /// reports. Startup never waits for it (invariant 13).
    RemoteStarted(Box<crate::app::remote_setup::RemoteStarted>),
    /// The bootstrap could not start the sidecar; `/remote` shows why.
    RemoteUnavailable(crate::app::remote_setup::RemoteUnavailable),

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
    //
    // `[` is the exception, and it gets the burst window rather than ZERO: it
    // is the second byte of a leaked SGR report whose ESC the console already
    // ate, and its `<Cb;Cx;CyM` half can land a tick later. Waiting ZERO there
    // handed the pane a lone `[` and left `<35;106;53M` to arrive as a burst
    // with no marker at all. 5 ms on a literal `[` is the entire cost.
    let wait = if first_ch == '[' {
        Duration::from_millis(crate::theme::PASTE_BURST_MS)
    } else {
        Duration::ZERO
    };
    if !event::poll(wait).unwrap_or(false) {
        return vec![first];
    }

    // Drain the burst into one string, keeping the key events themselves so a
    // burst that turns out to be typing can be replayed verbatim. Windows
    // emits a press + release per key; the releases are skipped. The first
    // genuine non-text event ends the burst and is re-emitted after it.
    let mut text = String::from(first_ch);
    let mut keys = vec![first];
    let mut trailing: Option<Event> = None;
    while event::poll(burst_gap(&text)).unwrap_or(false) {
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

    // Leaked mouse reports are neither typing nor a paste (`is_mouse_report_run`):
    // they must not be replayed as keys, or the pane types their parameters.
    // They are the clicks themselves, so they come back as `Event::Mouse`.
    if is_mouse_report_run(&text) {
        return recover_mouse_report_run(&text, trailing);
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

/// Report introducers, longest first. The bytes get eaten one at a time on the
/// injection path this module exists for (see [`is_mouse_report_run`]), so the
/// marker is matched whole (`ESC[<`), half-eaten (`[<`) or gone (`<`).
const SGR_MARKERS: [&str; 3] = ["\x1b[<", "[<", "<"];
const X10_MARKERS: [&str; 2] = ["\x1b[M", "[M"];

/// Split `text` at the first matching marker, returning the marker's byte
/// length and the body after it.
fn strip_marker<'a>(text: &'a str, markers: &[&str]) -> Option<(usize, &'a str)> {
    markers
        .iter()
        .find_map(|m| text.strip_prefix(m).map(|body| (m.len(), body)))
}

/// Byte length of one SGR mouse report at the start of `text` — `ESC[<Cb;Cx;CyM`
/// (release uses `m`, extra modifier params are accepted), with the introducer
/// whole, half-eaten or absent.
fn sgr_report_len(text: &str) -> Option<usize> {
    let (head, body) = strip_marker(text, &SGR_MARKERS)?;
    let mut numbers = 0usize;
    let mut digits = 0usize;
    for (i, c) in body.char_indices() {
        match c {
            '0'..='9' => digits += 1,
            ';' if digits > 0 => {
                numbers += 1;
                digits = 0;
            }
            'M' | 'm' if digits > 0 && numbers >= 2 => return Some(head + i + 1),
            _ => return None,
        }
    }
    None
}

/// Byte length of one legacy X10 mouse report (`ESC[M` + button/x/y).
fn x10_report_len(text: &str) -> Option<usize> {
    let (head, body) = strip_marker(text, &X10_MARKERS)?;
    let mut chars = body.char_indices();
    let (i, c) = (0..3).filter_map(|_| chars.next()).last()?;
    Some(head + i + c.len_utf8())
}

/// A truncated SGR report — the burst window cut it before the `M`/`m`
/// (`[<35;106`). Requires the `[<` marker, `;`-separated all-digit params and
/// at least three digits, so a typed `[<3;4` is still keystrokes.
fn sgr_report_prefix_only(text: &str) -> bool {
    let Some((_, body)) = strip_marker(text, &SGR_MARKERS) else {
        return false;
    };
    let mut digits = 0usize;
    let mut total = 0usize;
    let mut semis = 0usize;
    for c in body.chars() {
        match c {
            '0'..='9' => {
                digits += 1;
                total += 1;
            }
            ';' if digits > 0 => {
                semis += 1;
                digits = 0;
            }
            _ => return false,
        }
    }
    semis >= 1 && total >= 3
}

/// Numeric tail of a report split across two bursts (`53M`, left behind by the
/// `[<35;106;` half of the previous one).
fn report_tail_len(text: &str) -> Option<usize> {
    let mut digits = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '0'..='9' => digits += 1,
            ';' if digits > 0 => digits = 0,
            'M' | 'm' if digits > 0 => return Some(i + 1),
            _ => return None,
        }
    }
    None
}

/// `Cb` → kind + modifiers, byte-for-byte the mapping crossterm uses on the
/// paths where it decodes a report itself (`event::sys::unix::parse::parse_cb`).
/// Mirroring it matters: a report recovered here must be indistinguishable from
/// one crossterm parsed natively, or clicks would land with different buttons /
/// modifiers depending on which transport the host terminal happened to use.
fn parse_cb(cb: u8) -> Option<(MouseEventKind, KeyModifiers)> {
    let button_number = (cb & 0b0000_0011) | ((cb & 0b1100_0000) >> 4);
    let dragging = cb & 0b0010_0000 != 0;

    let kind = match (button_number, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3, true) | (4, true) | (5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
        _ => return None,
    };

    let mut modifiers = KeyModifiers::empty();
    if cb & 0b0000_0100 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if cb & 0b0000_1000 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if cb & 0b0001_0000 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    Some((kind, modifiers))
}

/// A press kind becomes its release counterpart; everything else is unchanged.
/// SGR reports a release as the same `Cb` with a lowercase final byte.
fn released(kind: MouseEventKind) -> MouseEventKind {
    match kind {
        MouseEventKind::Down(b) => MouseEventKind::Up(b),
        other => other,
    }
}

/// Decode one SGR report at the start of `text` → `(event, bytes consumed)`.
fn sgr_mouse_event(text: &str) -> Option<(MouseEvent, usize)> {
    let (head, body) = strip_marker(text, &SGR_MARKERS)?;
    let mut params = [0u32; 3];
    let mut idx = 0usize;
    let mut digits = 0usize;
    for (i, c) in body.char_indices() {
        match c {
            '0'..='9' => {
                if idx < params.len() {
                    params[idx] = params[idx]
                        .saturating_mul(10)
                        .saturating_add(c as u32 - '0' as u32);
                }
                digits += 1;
            }
            ';' if digits > 0 => {
                idx += 1;
                digits = 0;
            }
            // Parameters past the third are ignored, as crossterm does.
            'M' | 'm' if digits > 0 && idx >= 2 => {
                let cb = u8::try_from(params[0]).ok()?;
                let (kind, modifiers) = parse_cb(cb)?;
                let kind = if c == 'm' { released(kind) } else { kind };
                let column = u16::try_from(params[1]).ok()?.saturating_sub(1);
                let row = u16::try_from(params[2]).ok()?.saturating_sub(1);
                return Some((
                    MouseEvent {
                        kind,
                        column,
                        row,
                        modifiers,
                    },
                    head + i + 1,
                ));
            }
            _ => return None,
        }
    }
    None
}

/// Decode one legacy X10 report at the start of `text` → `(event, bytes)`.
/// Coordinates are single bytes offset by 32, so they saturate at 222 — the
/// reason `?1006h` (SGR) is requested in the first place.
fn x10_mouse_event(text: &str) -> Option<(MouseEvent, usize)> {
    let (head, body) = strip_marker(text, &X10_MARKERS)?;
    let mut chars = body.char_indices();
    let (_, cb) = chars.next()?;
    let (_, cx) = chars.next()?;
    let (_, cy) = chars.next()?;
    let payload = cb.len_utf8() + cx.len_utf8() + cy.len_utf8();
    let cb = u8::try_from(cb as u32).ok()?;
    if cb < 32 {
        return None;
    }
    let (kind, modifiers) = parse_cb(cb - 32)?;
    let coord = |c: char| u16::try_from(c as u32).ok().map(|v| v.saturating_sub(33));
    Some((
        MouseEvent {
            kind,
            column: coord(cx)?,
            row: coord(cy)?,
            modifiers,
        },
        head + payload,
    ))
}

/// Turn a run that [`is_mouse_report_run`] accepted back into the
/// `Event::Mouse` values crossterm would have produced had the reports reached
/// it as records.
///
/// Every complete report in the run yields one event; a numeric tail left over
/// from a report split across two bursts (`53M`) and an unfinished final report
/// (`[<35;106`) carry no usable event and are consumed silently. A report whose
/// `Cb` names a button crossterm does not model is skipped rather than guessed.
pub(crate) fn decode_mouse_report_run(text: &str) -> Vec<MouseEvent> {
    let mut rest = text;
    if let Some(n) = report_tail_len(rest) {
        rest = &rest[n..];
    }
    let mut events = Vec::new();
    while let Some((event, n)) = sgr_mouse_event(rest).or_else(|| x10_mouse_event(rest)) {
        events.push(event);
        rest = &rest[n..];
    }
    events
}

/// Whether the whole of `text` is terminal *mouse reporting* that leaked into
/// the key stream instead of arriving as `crossterm::event::Event::Mouse`.
///
/// **Where the leak comes from.** On Windows ConPTY, mouse input can be
/// delivered to a child two ways: as `MOUSE_EVENT` records (what crossterm
/// reads, and what a real `Event::Mouse` needs) or as VT bytes. Under psmux the
/// choice is made per pane by `detect_vt_bridge` → `has_vt_bridge_descendant`,
/// which walks the pane child's **whole process tree** looking for
/// `wsl.exe`/`ssh.exe`; a hit switches the pane to
/// `window_ops::inject_sgr_mouse`, which writes `ESC[<Cb;Cx;CyM` into the
/// child's console input buffer as **KEY_EVENT** records — no `MOUSE_EVENT`
/// record is injected at all. Conhost then hands those bytes to a
/// `ReadConsoleInputW` reader such as crossterm as *keystrokes*, losing the
/// introducer byte by byte. So the moment any descendant of the pane starts
/// `ssh` — gaviero's own terminal pane, nvim's `:terminal` — **every** report
/// for that pane arrives here as text like `[<35;106;53M`, and the pane stops
/// seeing `Event::Mouse` entirely: keys keep working, the mouse goes dead.
/// That is the whole of the "console fills with `[<35;106;53M`" flood *and* the
/// "gaviero ignores the mouse while I'm ssh'd somewhere" report; they are one
/// bug seen from two sides.
///
/// **What this gate is for.** The run must not be replayed as keystrokes (that
/// types its parameters into whatever has focus — a remote shell echoes them
/// back) nor handed to the coalescer's `Event::Paste`. It is instead recovered
/// by [`decode_mouse_report_run`], which restores the clicks, drags and wheel
/// events the pane was starved of.
///
/// Dropping `?1003h` from the Windows request set (see [`crate::platform`]) is
/// what removed the per-pointer-move *volume* of these runs: psmux forwards
/// bare motion only to a pane whose own screen asks for DECSET 1003
/// (`window_ops::pane_wants_bare_motion`), so with the request gone the motion
/// `Cb=35` reports stop being injected at all. It never stopped the leak —
/// press/release/drag/wheel reports take the same KEY_EVENT route.
pub(crate) fn is_mouse_report_run(text: &str) -> bool {
    let mut rest = text;
    if let Some(n) = report_tail_len(rest) {
        rest = &rest[n..];
    }
    let mut complete = 0usize;
    while let Some(n) = sgr_report_len(rest).or_else(|| x10_report_len(rest)) {
        rest = &rest[n..];
        complete += 1;
    }
    if complete == 0 {
        // Nothing complete: only an unmistakable unfinished report.
        return sgr_report_prefix_only(rest);
    }
    // After a complete report any cut-off follow-up counts (`[<0;`), however
    // short — it cannot be typing.
    rest.is_empty() || sgr_report_prefix_only(rest) || unfinished_sgr_report(rest)
}

/// `text` after its complete reports — what [`is_mouse_report_run`] and
/// [`ends_in_unfinished_report`] judge.
fn after_complete_reports(text: &str) -> &str {
    let mut rest = text;
    if let Some(n) = report_tail_len(rest) {
        rest = &rest[n..];
    }
    while let Some(n) = sgr_report_len(rest).or_else(|| x10_report_len(rest)) {
        rest = &rest[n..];
    }
    rest
}

/// An SGR report stopped before its final byte: `ESC[` (the `<` not here yet)
/// or a whole/half marker plus digits and `;` only (`[<35;106`, `ESC[<0;`).
fn unfinished_sgr_report(rest: &str) -> bool {
    if rest == "\x1b[" {
        return true;
    }
    rest.strip_prefix("\x1b[<")
        .or_else(|| rest.strip_prefix("[<"))
        .is_some_and(|body| body.chars().all(|c| c.is_ascii_digit() || c == ';'))
}

/// Whether a drained burst stopped in the middle of a leaked mouse report, so
/// the reader should wait for the rest instead of ending the burst there.
fn ends_in_unfinished_report(text: &str) -> bool {
    unfinished_sgr_report(after_complete_reports(text))
}

/// How long the Windows reader waits for the next record of a burst. Normally
/// `PASTE_BURST_MS`; `MOUSE_REPORT_SPLIT_MS` while a mouse report is cut off,
/// so a report that ConPTY hands over in two batches is still read whole. A
/// typed `[<` costs at most that gap, and only when it already came in a burst.
fn burst_gap(text: &str) -> Duration {
    let ms = if ends_in_unfinished_report(text) {
        crate::theme::MOUSE_REPORT_SPLIT_MS
    } else {
        crate::theme::PASTE_BURST_MS
    };
    Duration::from_millis(ms)
}

/// The events a burst that [`is_mouse_report_run`] accepted stands for: its
/// decoded mouse reports, then the event that ended the burst.
fn recover_mouse_report_run(
    text: &str,
    trailing: Option<crossterm::event::Event>,
) -> Vec<crossterm::event::Event> {
    let mut out: Vec<_> = decode_mouse_report_run(text)
        .into_iter()
        .map(crossterm::event::Event::Mouse)
        .collect();
    out.extend(trailing);
    out
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
    while let Ok(next) = event::read() {
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

        let wait = if saw_start && !text.contains(BP_END) {
            // Still inside a bracketed paste — wait much longer for the next
            // byte / end marker than for a raw key-burst boundary.
            Duration::from_millis(crate::theme::PASTE_BURST_MS.saturating_mul(40).max(100))
        } else {
            burst_gap(&text)
        };
        if !event::poll(wait).unwrap_or(false) {
            break;
        }
    }

    // Same guard on the Esc-led path: an Esc followed by `[<Cb;Cx;CyM` is the
    // host terminal's mouse report, not a paste burst.
    if is_mouse_report_run(&text) {
        return recover_mouse_report_run(&text, trailing);
    }

    let mut out = if let Some(payload) = strip_bracketed_paste(&text) {
        vec![Event::Paste(payload.to_string())]
    } else if !text.starts_with(BP_START) {
        // No start marker: an Esc keypress, possibly followed by ordinary
        // typing the console had already buffered. Replay every drained key —
        // rewriting them as a `Paste` (or returning the lone Esc and dropping
        // the rest) is how typing right after Esc turned into a clipboard
        // dump / vanished characters.
        let rest = &text[text
            .char_indices()
            .nth(1)
            .map(|(i, _)| i)
            .unwrap_or(text.len())..];
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
                        // Forward the visible paths; drop the event entirely
                        // if every reported path is excluded.
                        let visible: Vec<PathBuf> = event
                            .paths
                            .into_iter()
                            .filter(|p| !path_is_excluded(p, &roots, &exclude_patterns))
                            .collect();
                        if !visible.is_empty() {
                            let _ = tx.send(Event::FileTreeChanged(visible));
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
        assert!(path_is_excluded(
            &root.join(".git/objects/abc"),
            &roots,
            &[]
        ));
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
        assert_eq!(
            paste_char(&press(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some('a')
        );
        assert_eq!(
            paste_char(&press(KeyCode::Enter, KeyModifiers::NONE)),
            Some('\n')
        );
        assert_eq!(
            paste_char(&press(KeyCode::Tab, KeyModifiers::NONE)),
            Some('\t')
        );
        // Shift is part of pasted capitals, not a chord.
        assert_eq!(
            paste_char(&press(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Some('A')
        );

        // Ctrl/Alt chords and navigation keys are never pasted text.
        assert_eq!(
            paste_char(&press(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            None
        );
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
            paste_char(&key(
                KeyCode::Char('a'),
                KeyModifiers::NONE,
                KeyEventKind::Release
            )),
            None
        );
    }

    #[test]
    fn strip_bracketed_paste_extracts_payload_including_empty() {
        // Windows Terminal image paste: empty bracketed-paste payload.
        assert_eq!(strip_bracketed_paste("\x1b[200~\x1b[201~"), Some(""));
        assert_eq!(
            strip_bracketed_paste("\x1b[200~hello\x1b[201~"),
            Some("hello")
        );
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
    fn leaked_mouse_reports_are_never_keystrokes() {
        // Windows ConPTY/psmux re-encode the host terminal's SGR reports into
        // the key stream and lose the ESC introducer; replaying them types the
        // parameters into the focused pane (ssh echoes them back).
        assert!(is_mouse_report_run("[<35;106;53M"));
        assert!(is_mouse_report_run("\x1b[<35;106;53M"));
        assert!(is_mouse_report_run("[<35;106;53M[<35;109;53M[<0;30;59M"));
        assert!(is_mouse_report_run("\x1b[<0;30;59m"));
        // Burst boundary cut the report: prefix, numeric tail, or both.
        assert!(is_mouse_report_run("[<35;106;53"));
        assert!(is_mouse_report_run("[<35;106"));
        assert!(is_mouse_report_run("53M[<35;109;53M"));
        assert!(is_mouse_report_run("[<35;106;53M[<35;109"));
        assert!(is_mouse_report_run("53M[<35;106"));
        // Legacy X10 reports: ESC[M + button/x/y.
        assert!(is_mouse_report_run("\x1b[Magb"));

        // Not mouse reporting — typing, pastes, numbers, bracketed paste.
        assert!(!is_mouse_report_run(""));
        assert!(!is_mouse_report_run("cargo test -p gaviero-tui"));
        assert!(!is_mouse_report_run("line one\nline two\n"));
        assert!(!is_mouse_report_run("123"));
        assert!(!is_mouse_report_run("[<3;4"));
        assert!(!is_mouse_report_run("[<"));
        assert!(!is_mouse_report_run("[M"));
        assert!(!is_mouse_report_run("\x1b[200~hello\x1b[201~"));
    }

    /// psmux injects a pane's mouse reports as keystrokes once `ssh.exe` runs
    /// anywhere under it (gaviero's own terminal pane included). Swallowing
    /// the run killed the mouse; it must come back as the clicks it encodes.
    #[test]
    fn leaked_mouse_reports_come_back_as_mouse_events() {
        use crossterm::event::Event as CtEvent;

        let trailing = CtEvent::Key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            KeyModifiers::NONE,
        ));
        let out = recover_mouse_report_run("\x1b[<0;12;5M\x1b[<0;12;5m", Some(trailing.clone()));
        assert_eq!(
            out,
            vec![
                CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 11,
                    row: 4,
                    modifiers: KeyModifiers::NONE,
                }),
                CtEvent::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: 11,
                    row: 4,
                    modifiers: KeyModifiers::NONE,
                }),
                trailing,
            ]
        );

        // ESC already eaten, wheel report.
        assert_eq!(
            recover_mouse_report_run("[<65;3;7M", None),
            vec![CtEvent::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 2,
                row: 6,
                modifiers: KeyModifiers::NONE,
            })]
        );
    }

    /// A report cut by the burst window keeps the reader waiting for its rest
    /// (`burst_gap`); anything else ends the burst at the normal gap.
    #[test]
    fn unfinished_reports_extend_the_burst() {
        assert!(ends_in_unfinished_report("\x1b["));
        assert!(ends_in_unfinished_report("\x1b[<"));
        assert!(ends_in_unfinished_report("[<0;"));
        assert!(ends_in_unfinished_report("[<35;106"));
        assert!(ends_in_unfinished_report("\x1b[<0;1;1M\x1b[<0;1"));
        assert!(ends_in_unfinished_report("53M[<0"));

        assert!(!ends_in_unfinished_report("\x1b"));
        assert!(!ends_in_unfinished_report("["));
        assert!(!ends_in_unfinished_report("[<0;1;1M"));
        assert!(!ends_in_unfinished_report("[<a"));
        assert!(!ends_in_unfinished_report("cargo"));
        assert_eq!(
            burst_gap("hello"),
            Duration::from_millis(crate::theme::PASTE_BURST_MS)
        );
        assert_eq!(
            burst_gap("[<35;106"),
            Duration::from_millis(crate::theme::MOUSE_REPORT_SPLIT_MS)
        );

        // A short cut-off report after a complete one is still part of the run.
        assert!(is_mouse_report_run("[<0;12;5M[<0;"));
        assert!(is_mouse_report_run("\x1b[<0;12;5M\x1b["));
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
