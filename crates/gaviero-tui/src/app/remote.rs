//! Remote-control reducers and per-entity freshness state (Plan A units
//! A3/A4). Desktop input handlers and `Event::RemoteCommand` both resolve
//! to the same reducers — one implementation of each mutation's semantics
//! (invariant 3). Reducers validate freshness tokens (§4.8), perform
//! exactly one state transition, bump the affected entity's revision, and
//! push semantic outbound frames into [`RemoteState`] for the projection
//! layer to drain. They never perform network I/O.

use std::collections::HashMap;

use gaviero_remote::dto::{
    AnsweredBy, CommandStatus, ErrorCode, PermissionOutcome, ReviewActionKind,
};
use gaviero_remote::envelope::{
    ClientEnvelope, ClientFrame, CommandError, CommandResult, PermissionClosed, ServerFrame,
};

use crate::app::App;
use crate::panels::agent_chat::{PermissionAnswerError, SlashOrigin};

/// Remote slash-command policy (Plan §5.1). The server owns this
/// allow-list; `hello.allowed_slash_commands` / `hello.confirm_required`
/// are derived from these constants. Unknown or denied commands return
/// `slash_not_allowed` and are never forwarded to the agent.
pub const REMOTE_ALLOWED_SLASH: &[&str] = &[
    "/model",
    "/thinking",
    "/effort",
    "/compact",
    "/context",
    "/inject",
    "/no-inject",
    "/reset",
    "/clear",
    "/rename",
    "/namespace",
    "/ns",
    "/autoapprove",
    "/yolo",
    "/workspace",
    "/ws",
    "/lite",
    "/minimal",
    "/help",
    "/skills",
];

/// Destructive or approval-bypassing commands require `confirmed: true`.
pub const REMOTE_CONFIRM_REQUIRED: &[&str] = &["/autoapprove", "/yolo", "/reset", "/clear"];

/// First whitespace-delimited token of a slash line. Preserves the desktop
/// parser's token boundary — `/runaway` does not match `/run`.
pub fn slash_command_token(line: &str) -> &str {
    line.trim_start()
        .split_whitespace()
        .next()
        .unwrap_or("")
}

/// A rejected command: maps directly onto `command_error`.
#[derive(Debug, Clone)]
pub struct CommandFailure {
    pub code: ErrorCode,
    pub message: String,
}

impl CommandFailure {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Controller-side remote state: the global snapshot generation, per-
/// proposal freshness tokens, and the outbound frame buffer the projection
/// layer drains after each `handle_event` pass (A4).
pub struct RemoteState {
    /// Sidecar hub handle once the server is running (A6 spawns it).
    /// `try_send` only from the event loop (invariant 11).
    pub handle: Option<gaviero_remote::server::RemoteHandle>,
    /// `remote.maxPromptBytes` (§5.2); set from settings when the sidecar
    /// is configured (A6). Default mirrors the wire default.
    pub max_prompt_bytes: usize,
    /// Global snapshot generation (§4.1). Ordering/staleness display only —
    /// never a command precondition.
    pub revision: u64,
    /// Set when outbound delivery must fall back to a fresh snapshot
    /// (hub channel full, or state changed without a per-event frame).
    pub snapshot_dirty: bool,
    /// Per-proposal freshness tokens (§4.8). A proposal not present has
    /// revision 1 (creation).
    proposal_revisions: HashMap<u64, u64>,
    /// Semantic outbound frames produced by reducers this event-loop pass.
    /// Drained (stamped with `revision`, forwarded to the hub) by the A4
    /// projection layer.
    pub pending_frames: Vec<ServerFrame>,
    /// `conv_revision` values as last projected — the pump's change sweep.
    pub projected_conv_revisions: HashMap<String, u64>,
    /// Active conversation id as last projected.
    pub projected_active_id: Option<String>,
    /// Whether a remote client is currently connected (status display).
    pub client_connected: bool,
}

impl Default for RemoteState {
    fn default() -> Self {
        Self {
            handle: None,
            max_prompt_bytes: 128 * 1024,
            revision: 0,
            snapshot_dirty: false,
            proposal_revisions: HashMap::new(),
            pending_frames: Vec::new(),
            projected_conv_revisions: HashMap::new(),
            projected_active_id: None,
            client_connected: false,
        }
    }
}

impl RemoteState {
    /// Current freshness token for a proposal.
    pub fn proposal_revision(&self, proposal_id: u64) -> u64 {
        self.proposal_revisions.get(&proposal_id).copied().unwrap_or(1)
    }

    /// Bump a proposal's token in the same transition that mutated it.
    pub fn bump_proposal_revision(&mut self, proposal_id: u64) -> u64 {
        let entry = self.proposal_revisions.entry(proposal_id).or_insert(1);
        *entry += 1;
        *entry
    }

    /// A proposal left the open set (finalized): its token is retired.
    pub fn retire_proposal(&mut self, proposal_id: u64) {
        self.proposal_revisions.remove(&proposal_id);
    }

    /// Any externally-visible mutation bumps the global generation.
    pub fn bump_global(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    pub fn push_frame(&mut self, frame: ServerFrame) {
        self.pending_frames.push(frame);
    }
}

// ── Command dispatch (§4.4) ─────────────────────────────────────────

/// Route one decoded, deduplicated, rate-limited client command through
/// the shared reducers. Emits exactly one terminal `command_result` or
/// `command_error` into the pending buffer, plus any post-mutation frames.
pub fn handle_remote_command(app: &mut App, envelope: ClientEnvelope, max_prompt_bytes: usize) {
    let command_id = envelope.command_id;
    let outcome: Result<(CommandStatus, Option<serde_json::Value>), CommandFailure> =
        match envelope.frame {
            // The hub already rejects repeated client_hello; defensive.
            ClientFrame::ClientHello(_) => Err(CommandFailure::new(
                ErrorCode::InvalidPayload,
                "unexpected client_hello",
            )),
            ClientFrame::SendPrompt(p) => {
                apply_remote_prompt(app, &p.conv_id, &p.text, max_prompt_bytes).map(|turn_id| {
                    (
                        CommandStatus::Accepted,
                        Some(serde_json::json!({ "turn_id": turn_id })),
                    )
                })
            }
            ClientFrame::Slash(s) => apply_remote_slash(app, &s.conv_id, &s.line, s.confirmed)
                .map(|()| (CommandStatus::Completed, None)),
            ClientFrame::PermissionDecision(d) => apply_permission_decision(
                app,
                &d.request_id,
                d.allow,
                d.answers.as_deref(),
                d.message.as_deref(),
                AnsweredBy::Remote,
            )
            .map(|_| (CommandStatus::Completed, None)),
            ClientFrame::ReviewAction(r) => {
                apply_review_action(app, r.proposal_id, r.proposal_revision, r.action, r.hunk_index)
                    .map(|()| (CommandStatus::Completed, None))
            }
            ClientFrame::NewConversation {} => apply_new_conversation(app).map(|conv_id| {
                (
                    CommandStatus::Completed,
                    Some(serde_json::json!({ "conv_id": conv_id })),
                )
            }),
            ClientFrame::SwitchConversation(c) => apply_switch_conversation(app, &c.conv_id)
                .map(|()| (CommandStatus::Completed, None)),
            ClientFrame::RenameConversation(r) => {
                apply_rename_conversation(app, &r.conv_id, r.conv_revision, &r.title)
                    .map(|()| (CommandStatus::Completed, None))
            }
            ClientFrame::ResetConversation(r) => {
                apply_reset_conversation(app, &r.conv_id, r.conv_revision)
                    .map(|()| (CommandStatus::Completed, None))
            }
            ClientFrame::Interrupt(i) => apply_interrupt(app, &i.conv_id, i.turn_id.as_deref())
                .map(|()| (CommandStatus::Completed, None)),
            ClientFrame::RequestSnapshot {} => {
                app.remote.snapshot_dirty = true;
                Ok((CommandStatus::Completed, None))
            }
            ClientFrame::RequestMessages(r) => {
                crate::app::projection::build_message_page(app, &r.conv_id, r.before_seq, r.limit)
                    .map(|page| {
                        app.remote.push_frame(ServerFrame::MessagePage(page));
                        (CommandStatus::Completed, None)
                    })
            }
            ClientFrame::RequestProposal(r) => {
                crate::app::projection::build_proposal_detail(app, r.proposal_id).map(|event| {
                    app.remote.push_frame(ServerFrame::ProposalDetail(event));
                    (CommandStatus::Completed, None)
                })
            }
        };
    match outcome {
        Ok((status, result)) => {
            app.remote.push_frame(ServerFrame::CommandResult(CommandResult {
                command_id,
                status,
                result,
            }));
        }
        Err(failure) => {
            app.remote.push_frame(ServerFrame::CommandError(CommandError {
                command_id,
                code: failure.code,
                message: failure.message,
            }));
        }
    }
}

// ── Permission decisions (§2.2 / §5.3) ──────────────────────────────

/// Shared permission reducer. Both the desktop y/n keys and remote
/// `permission_decision` land here; first valid writer wins, the loser
/// finds no parked request (`stale_request`), and every close emits one
/// `permission_closed` with `answered_by`.
pub fn apply_permission_decision(
    app: &mut App,
    request_id: &str,
    allow: bool,
    answers: Option<&[Vec<u32>]>,
    message: Option<&str>,
    answered_by: AnsweredBy,
) -> Result<PermissionClosed, CommandFailure> {
    // Deny note hygiene (§5.3): size-limited, control-character filtered,
    // never reaches tool input.
    let message = message.map(|m| {
        m.chars()
            .filter(|c| !c.is_control() || *c == '\n')
            .take(2_000)
            .collect::<String>()
    });

    let Some(idx) = app.chat_state.find_permission_conv(request_id) else {
        return Err(CommandFailure::new(
            ErrorCode::StaleRequest,
            "no pending permission with that id — the other side answered first",
        ));
    };
    match app
        .chat_state
        .respond_permission_at(idx, allow, answers, message.as_deref())
    {
        Ok(info) => {
            let closed = PermissionClosed {
                conv_id: info.conv_id,
                request_id: info.request_id,
                outcome: if info.allowed {
                    PermissionOutcome::Allowed
                } else {
                    PermissionOutcome::Denied
                },
                answered_by,
            };
            app.remote.push_frame(ServerFrame::PermissionClosed(closed.clone()));
            app.remote.bump_global();
            Ok(closed)
        }
        Err(PermissionAnswerError::NoPending) => Err(CommandFailure::new(
            ErrorCode::StaleRequest,
            "no pending permission with that id — the other side answered first",
        )),
        Err(PermissionAnswerError::Invalid(reason)) => {
            Err(CommandFailure::new(ErrorCode::InvalidPayload, reason))
        }
    }
}

/// Desktop wrapper: answer the ACTIVE conversation's pending permission
/// through the shared reducer so the close is projected with
/// `answered_by: desktop`.
pub fn desktop_answer_active_permission(app: &mut App, allow: bool) {
    let Some(request_id) = app
        .chat_state
        .active_conversation()
        .pending_permission
        .as_ref()
        .map(|p| p.request_id.clone())
    else {
        return;
    };
    let _ = apply_permission_decision(
        app,
        &request_id,
        allow,
        None,
        None,
        AnsweredBy::Desktop,
    );
}

// ── Conversation commands (§4.4 / §4.8) ─────────────────────────────

pub fn apply_switch_conversation(app: &mut App, conv_id: &str) -> Result<(), CommandFailure> {
    let Some(idx) = app.chat_state.find_conv_idx(conv_id) else {
        return Err(CommandFailure::new(
            ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    };
    app.chat_state.switch_conversation(idx);
    app.remote.bump_global();
    app.remote.snapshot_dirty = true;
    Ok(())
}

pub fn apply_new_conversation(app: &mut App) -> Result<String, CommandFailure> {
    app.chat_state.new_conversation();
    app.remote.bump_global();
    app.remote.snapshot_dirty = true;
    Ok(app.chat_state.active_conversation_id().to_string())
}

pub fn apply_rename_conversation(
    app: &mut App,
    conv_id: &str,
    conv_revision: u64,
    title: &str,
) -> Result<(), CommandFailure> {
    let Some(idx) = app.chat_state.find_conv_idx(conv_id) else {
        return Err(CommandFailure::new(
            ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    };
    let current = app.chat_state.conversations[idx].conv_revision;
    if conv_revision != current {
        return Err(CommandFailure::new(
            ErrorCode::StaleConversation,
            format!("conversation is at revision {current}"),
        ));
    }
    // Title hygiene (§5.2): no control characters, bounded length.
    let title: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(200)
        .collect();
    if title.trim().is_empty() {
        return Err(CommandFailure::new(
            ErrorCode::InvalidPayload,
            "title must not be empty",
        ));
    }
    app.chat_state.conversations[idx].title = title;
    app.chat_state.conversations[idx].bump_revision();
    app.remote.bump_global();
    app.remote.snapshot_dirty = true;
    Ok(())
}

pub fn apply_reset_conversation(
    app: &mut App,
    conv_id: &str,
    conv_revision: u64,
) -> Result<(), CommandFailure> {
    let Some(idx) = app.chat_state.find_conv_idx(conv_id) else {
        return Err(CommandFailure::new(
            ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    };
    let current = app.chat_state.conversations[idx].conv_revision;
    if conv_revision != current {
        return Err(CommandFailure::new(
            ErrorCode::StaleConversation,
            format!("conversation is at revision {current}"),
        ));
    }
    app.chat_state.reset_conversation_at(idx);
    app.remote.bump_global();
    app.remote.snapshot_dirty = true;
    Ok(())
}

pub fn apply_interrupt(
    app: &mut App,
    conv_id: &str,
    _turn_id: Option<&str>,
) -> Result<(), CommandFailure> {
    if app.chat_state.find_conv_idx(conv_id).is_none() {
        return Err(CommandFailure::new(
            ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    }
    // Targets the NAMED conversation, not the active one — the §2.2
    // parameterization this command exists for.
    crate::app::side_panel::cancel_agent_conv(app, conv_id);
    app.remote.bump_global();
    Ok(())
}

// ── Prompt dispatch (§2.4) ──────────────────────────────────────────

pub fn apply_remote_prompt(
    app: &mut App,
    conv_id: &str,
    text: &str,
    max_prompt_bytes: usize,
) -> Result<String, CommandFailure> {
    if text.len() > max_prompt_bytes {
        return Err(CommandFailure::new(
            ErrorCode::TooLarge,
            format!("prompt exceeds {max_prompt_bytes} bytes"),
        ));
    }
    if text.trim().is_empty() {
        return Err(CommandFailure::new(
            ErrorCode::InvalidPayload,
            "prompt must not be empty",
        ));
    }
    let Some(idx) = app.chat_state.find_conv_idx(conv_id) else {
        return Err(CommandFailure::new(
            ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    };
    if app.chat_state.conversations[idx].is_streaming {
        return Err(CommandFailure::new(
            ErrorCode::ConversationStreaming,
            "conversation is already streaming",
        ));
    }
    // No desktop draft, no attachments, no one-shot auto-approve (§2.4);
    // the shared core still consumes lite/workspace/no-inject one-shots.
    let turn_id = crate::app::side_panel::dispatch_prompt_core(
        app,
        conv_id,
        text.to_string(),
        Vec::new(),
        false,
    )
    .map_err(|reason| CommandFailure::new(ErrorCode::InternalError, reason))?;
    app.remote.bump_global();
    Ok(turn_id)
}

// ── Slash commands (§5.1) ───────────────────────────────────────────

pub fn apply_remote_slash(
    app: &mut App,
    conv_id: &str,
    line: &str,
    confirmed: bool,
) -> Result<(), CommandFailure> {
    let command = slash_command_token(line);
    if !REMOTE_ALLOWED_SLASH.contains(&command) {
        // Denied or unknown — never forwarded to the agent.
        return Err(CommandFailure::new(
            ErrorCode::SlashNotAllowed,
            format!("{command} is not available remotely"),
        ));
    }
    if REMOTE_CONFIRM_REQUIRED.contains(&command) && !confirmed {
        // Redacted audit trail: the command name is policy metadata, the
        // argument tail may hold user content.
        tracing::warn!(command, "remote confirm-required command without confirmation");
        return Err(CommandFailure::new(
            ErrorCode::ConfirmRequired,
            format!("{command} requires confirmed: true"),
        ));
    }
    let Some(idx) = app.chat_state.find_conv_idx(conv_id) else {
        return Err(CommandFailure::new(
            ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    };
    if command == "/skills" {
        // Same catalog listing the desktop's bare `/skills` produces
        // (`side_panel::handle_skills_command`); semantic search stays
        // desktop-only for now (it spawns an embedder query).
        let skills = app.skill_catalog.all_skills();
        let listing = if skills.is_empty() {
            "No skills found. Add skill folders under `.gaviero/skills/<name>/SKILL.md` \
             (repo, workspace, or `~/.gaviero/skills/`)."
                .to_string()
        } else {
            skills
                .iter()
                .map(|s| {
                    format!(
                        "- {} [{}] — {}",
                        s.name,
                        app.skill_catalog.source_label(s),
                        s.description
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        app.chat_state.add_user_message_at(idx, line);
        app.chat_state.add_system_message_at(idx, &listing);
    } else {
        app.chat_state
            .apply_slash_line(idx, line, SlashOrigin::Remote);
    }
    app.remote.bump_global();
    app.remote.snapshot_dirty = true;
    Ok(())
}

// ── Review actions (§2.5 / §4.8) — ownership rule ───────────────────

/// Applies a hunk-level review action, resolving `proposal_id` to exactly
/// one owner: the open `DiffReviewState` if it holds that id, else the
/// gate copy. A remote action on a proposal that is not open on the
/// desktop never opens, closes, or refocuses any overlay.
pub fn apply_review_action(
    app: &mut App,
    proposal_id: u64,
    proposal_revision: u64,
    action: ReviewActionKind,
    hunk_index: Option<u32>,
) -> Result<(), CommandFailure> {
    let current = app.remote.proposal_revision(proposal_id);
    if proposal_revision != current {
        return Err(CommandFailure::new(
            ErrorCode::StaleProposal,
            format!("proposal {proposal_id} is at revision {current}"),
        ));
    }
    if matches!(action, ReviewActionKind::AcceptHunk | ReviewActionKind::RejectHunk)
        && hunk_index.is_none()
    {
        return Err(CommandFailure::new(
            ErrorCode::InvalidPayload,
            "hunk_index is required for per-hunk actions",
        ));
    }

    // Owner 1: the open desktop overlay holds a local copy — mutate that
    // copy only (it is written back to the gate on finalize, preserving
    // the existing desktop contract).
    let overlay_owns = app
        .diff_review
        .as_ref()
        .is_some_and(|r| r.proposal.id == proposal_id);
    if overlay_owns {
        let review = app.diff_review.as_mut().expect("checked above");
        let hunk_count = review.proposal.structural_hunks.len();
        match action {
            ReviewActionKind::AcceptHunk | ReviewActionKind::RejectHunk => {
                let idx = hunk_index.expect("validated above") as usize;
                if idx >= hunk_count {
                    return Err(CommandFailure::new(
                        ErrorCode::InvalidHunk,
                        format!("hunk index {idx} out of range ({hunk_count} hunks)"),
                    ));
                }
                if action == ReviewActionKind::AcceptHunk {
                    review.accept_hunk(idx);
                } else {
                    review.reject_hunk(idx);
                }
            }
            ReviewActionKind::AcceptAll => review.accept_all(),
            ReviewActionKind::RejectAll => review.reject_all(),
            ReviewActionKind::Finalize => {
                // Same outcome as the desktop `f` key — this is the one
                // case where the overlay closes, because the proposal is
                // done (identical to a desktop finalize).
                crate::app::review::finalize_current_review(app);
                app.remote.retire_proposal(proposal_id);
                app.remote.bump_global();
                app.remote.snapshot_dirty = true;
                return Ok(());
            }
        }
        app.remote.bump_proposal_revision(proposal_id);
        app.remote.bump_global();
        app.remote.snapshot_dirty = true;
        return Ok(());
    }

    // Owner 2: the gate copy. Never touch desktop focus or overlays.
    let Ok(mut gate) = app.write_gate.try_lock() else {
        // The gate lock is held only for short synchronous sections; a
        // busy gate here is transient. Fail without mutation — the client
        // may retry with the same (still-valid) revision.
        return Err(CommandFailure::new(
            ErrorCode::InternalError,
            "write gate busy — retry",
        ));
    };
    let Some(proposal) = gate.get_proposal(proposal_id) else {
        return Err(CommandFailure::new(
            ErrorCode::UnknownProposal,
            "unknown proposal",
        ));
    };
    let hunk_count = proposal.structural_hunks.len();
    if proposal.status == gaviero_core::types::ProposalStatus::Superseded {
        return Err(CommandFailure::new(
            ErrorCode::StaleProposal,
            "proposal was superseded by a conflicting peer",
        ));
    }
    match action {
        ReviewActionKind::AcceptHunk | ReviewActionKind::RejectHunk => {
            let idx = hunk_index.expect("validated above") as usize;
            if idx >= hunk_count {
                return Err(CommandFailure::new(
                    ErrorCode::InvalidHunk,
                    format!("hunk index {idx} out of range ({hunk_count} hunks)"),
                ));
            }
            if action == ReviewActionKind::AcceptHunk {
                gate.accept_hunk(proposal_id, idx);
            } else {
                gate.reject_hunk(proposal_id, idx);
            }
            drop(gate);
        }
        ReviewActionKind::AcceptAll => {
            gate.accept_all(proposal_id);
            drop(gate);
        }
        ReviewActionKind::RejectAll => {
            gate.reject_all(proposal_id);
            drop(gate);
        }
        ReviewActionKind::Finalize => {
            // Assemble and apply from the gate's own copy — stale-disk,
            // deletion, and conflict/supersede semantics identical to the
            // desktop finalize path.
            let proposal = proposal.clone();
            drop(gate);
            crate::app::review::finalize_gate_proposal(app, proposal);
            app.remote.retire_proposal(proposal_id);
            app.remote.bump_global();
            app.remote.snapshot_dirty = true;
            return Ok(());
        }
    }
    app.remote.bump_proposal_revision(proposal_id);
    app.remote.bump_global();
    app.remote.snapshot_dirty = true;
    Ok(())
}
