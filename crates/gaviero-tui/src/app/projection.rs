//! Outbound projection (Plan A unit A4): converts TUI state into
//! remote-owned wire DTOs (invariant 5 — never serialize core domain types
//! directly), builds bounded snapshots (§4.5), and pumps frames to the
//! `RemoteHub` with `try_send` only (invariant 11 — the event loop never
//! blocks on the sidecar; a full channel marks snapshot-dirty and moves on).

use gaviero_remote::dto as rdto;
use gaviero_remote::envelope as renv;
use gaviero_remote::envelope::ServerFrame;
use gaviero_remote::server::HubInput;

use crate::app::remote::CommandFailure;
use crate::app::App;
use crate::event::Event;
use crate::panels::agent_chat::{ChatMessage, ChatRole, Conversation, PendingPermission};

/// §4.5 caps.
const SNAPSHOT_TAIL_MESSAGES: usize = 100;
const SNAPSHOT_TAIL_BYTES: usize = 512 * 1024;
const PAGE_BYTES: usize = 512 * 1024;
const MESSAGE_BYTES: usize = 128 * 1024;
const HUNK_SIDE_BYTES: usize = 64 * 1024;
const PREVIEW_CHARS: usize = 120;

// ── DTO conversion (invariant 5) ────────────────────────────────────

fn role_dto(role: &ChatRole) -> rdto::Role {
    match role {
        ChatRole::User => rdto::Role::User,
        ChatRole::Assistant => rdto::Role::Assistant,
        ChatRole::System => rdto::Role::System,
    }
}

/// Truncate to a UTF-8 boundary at or below `max` bytes.
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Per-message cap (§4.5): over 128 KiB the head is sent with
/// `truncated: true` and `full_bytes` — a message larger than the page cap
/// must still be deliverable. `code_blocks` stays empty until A7 populates
/// spans (fields exist in the schema from A1).
pub(crate) fn message_dto(msg: &ChatMessage) -> rdto::Message {
    let full = msg.content.len();
    let truncated = full > MESSAGE_BYTES;
    rdto::Message {
        seq: msg.seq,
        role: role_dto(&msg.role),
        content: truncate_utf8(&msg.content, MESSAGE_BYTES).to_string(),
        truncated,
        full_bytes: truncated.then_some(full as u64),
        tool_calls: msg.tool_calls.clone(),
        code_blocks: Vec::new(),
    }
}

fn context_pressure_dto(app: &App, idx: usize) -> Option<rdto::ContextPressure> {
    let conv = &app.chat_state.conversations[idx];
    let usage = conv.last_token_usage.as_ref()?;
    let used = usage.prefix_tokens();
    if used == 0 {
        return None;
    }
    let model = app.chat_state.effective_model_at(idx);
    Some(rdto::ContextPressure {
        used_tokens: used,
        max_tokens: app.chat_state.context_limit_tokens_for(model) as u64,
    })
}

fn preview_of(conv: &Conversation) -> Option<String> {
    conv.messages
        .iter()
        .rev()
        .find(|m| m.role != ChatRole::System)
        .map(|m| {
            let head: String = m.content.chars().take(PREVIEW_CHARS).collect();
            if m.content.chars().count() > PREVIEW_CHARS {
                format!("{head}…")
            } else {
                head
            }
        })
}

pub(crate) fn conversation_summary_dto(app: &App, idx: usize) -> rdto::ConversationSummary {
    let conv = &app.chat_state.conversations[idx];
    rdto::ConversationSummary {
        conv_id: conv.id.clone(),
        conv_revision: conv.conv_revision,
        title: conv.title.clone(),
        // Overrides only (§4.5); defaults ride `RemoteSettings`.
        model: conv.model_override.clone(),
        effort: conv.effort_override.clone(),
        namespace: conv.namespace_override.clone(),
        is_streaming: conv.is_streaming,
        pending_turn_id: conv.pending_turn_id.clone(),
        context_pressure: context_pressure_dto(app, idx),
        auto_approve: conv.auto_approve,
        last_message_preview: preview_of(conv),
    }
}

pub(crate) fn permission_request_dto(
    conv_id: &str,
    perm: &PendingPermission,
) -> rdto::PermissionRequest {
    let ask = perm.ask.as_ref().map(|state| rdto::Ask {
        questions: state
            .questions
            .iter()
            .map(|q| rdto::AskQuestion {
                question: q.question.clone(),
                header: q.header.clone(),
                multi_select: q.multi_select,
                options: q
                    .options
                    .iter()
                    .map(|(label, description)| rdto::AskOption {
                        label: label.clone(),
                        description: description.clone(),
                    })
                    .collect(),
            })
            .collect(),
    });
    rdto::PermissionRequest {
        conv_id: conv_id.to_string(),
        request_id: perm.request_id.clone(),
        tool_name: perm.tool_name.clone(),
        description: perm.description.clone(),
        input: perm.input.clone(),
        ask,
    }
}

fn hunk_status_dto(status: &gaviero_core::types::HunkStatus) -> rdto::HunkStatus {
    match status {
        gaviero_core::types::HunkStatus::Pending => rdto::HunkStatus::Pending,
        gaviero_core::types::HunkStatus::Accepted => rdto::HunkStatus::Accepted,
        gaviero_core::types::HunkStatus::Rejected => rdto::HunkStatus::Rejected,
    }
}

fn hunk_type_dto(kind: &gaviero_core::types::HunkType) -> rdto::HunkType {
    match kind {
        gaviero_core::types::HunkType::Added => rdto::HunkType::Added,
        gaviero_core::types::HunkType::Removed => rdto::HunkType::Removed,
        gaviero_core::types::HunkType::Modified => rdto::HunkType::Modified,
    }
}

fn proposal_status_dto(status: &gaviero_core::types::ProposalStatus) -> rdto::ProposalStatus {
    match status {
        gaviero_core::types::ProposalStatus::Pending => rdto::ProposalStatus::Pending,
        gaviero_core::types::ProposalStatus::PartiallyAccepted => {
            rdto::ProposalStatus::PartiallyAccepted
        }
        gaviero_core::types::ProposalStatus::Accepted => rdto::ProposalStatus::Accepted,
        gaviero_core::types::ProposalStatus::Rejected => rdto::ProposalStatus::Rejected,
        gaviero_core::types::ProposalStatus::Superseded => rdto::ProposalStatus::Superseded,
    }
}

/// Workspace-relative display path (§4.7): never leak the absolute root.
pub(crate) fn relative_path(app: &App, path: &std::path::Path) -> String {
    for root in app.workspace.roots() {
        if let Ok(rel) = path.strip_prefix(root) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "<outside workspace>".to_string())
}

/// Full proposal DTO from the EXISTING structural hunks (§4.7 — never
/// rediffed; the gate mutates by index into this exact vector). Hunk sides
/// over 64 KiB are elided with `truncated: true` — still fully reviewable
/// because the server assembles content from its own copy.
pub(crate) fn proposal_dto(
    app: &App,
    proposal: &gaviero_core::types::WriteProposal,
    proposal_revision: u64,
) -> rdto::Proposal {
    rdto::Proposal {
        proposal_id: proposal.id,
        proposal_revision,
        conv_id: proposal.conv_id.clone(),
        source: proposal.source.clone(),
        path: relative_path(app, &proposal.file_path),
        status: proposal_status_dto(&proposal.status),
        is_deletion: proposal.is_deletion,
        conflicts_with: proposal.conflicts_with.clone(),
        hunks: proposal
            .structural_hunks
            .iter()
            .enumerate()
            .map(|(index, hunk)| {
                let original_over = hunk.diff_hunk.original_text.len() > HUNK_SIDE_BYTES;
                let proposed_over = hunk.diff_hunk.proposed_text.len() > HUNK_SIDE_BYTES;
                let truncated = original_over || proposed_over;
                rdto::Hunk {
                    index: index as u32,
                    original_range: rdto::LineRange {
                        start_line: hunk.diff_hunk.original_range.0 as u64,
                        end_line: hunk.diff_hunk.original_range.1 as u64,
                    },
                    proposed_range: rdto::LineRange {
                        start_line: hunk.diff_hunk.proposed_range.0 as u64,
                        end_line: hunk.diff_hunk.proposed_range.1 as u64,
                    },
                    original_text: if truncated {
                        String::new()
                    } else {
                        hunk.diff_hunk.original_text.clone()
                    },
                    proposed_text: if truncated {
                        String::new()
                    } else {
                        hunk.diff_hunk.proposed_text.clone()
                    },
                    truncated,
                    hunk_type: hunk_type_dto(&hunk.diff_hunk.hunk_type),
                    description: hunk.description.clone(),
                    status: hunk_status_dto(&hunk.status),
                }
            })
            .collect(),
    }
}

/// Snapshot form (§4.5): never carries hunk text.
pub(crate) fn proposal_summary_dto(
    app: &App,
    proposal: &gaviero_core::types::WriteProposal,
    proposal_revision: u64,
) -> rdto::ProposalSummary {
    let added: u64 = proposal
        .structural_hunks
        .iter()
        .map(|h| h.diff_hunk.proposed_text.lines().count() as u64)
        .sum();
    let removed: u64 = proposal
        .structural_hunks
        .iter()
        .map(|h| h.diff_hunk.original_text.lines().count() as u64)
        .sum();
    rdto::ProposalSummary {
        proposal_id: proposal.id,
        proposal_revision,
        conv_id: proposal.conv_id.clone(),
        source: proposal.source.clone(),
        path: relative_path(app, &proposal.file_path),
        status: proposal_status_dto(&proposal.status),
        is_deletion: proposal.is_deletion,
        conflicts_with: proposal.conflicts_with.clone(),
        hunk_count: proposal.structural_hunks.len() as u32,
        added_lines: added,
        removed_lines: removed,
        hunks: proposal
            .structural_hunks
            .iter()
            .enumerate()
            .map(|(index, hunk)| rdto::HunkSummary {
                index: index as u32,
                hunk_type: hunk_type_dto(&hunk.diff_hunk.hunk_type),
                description: hunk.description.clone(),
                status: hunk_status_dto(&hunk.status),
            })
            .collect(),
    }
}

// ── request_messages / request_proposal (§4.5) ──────────────────────

/// `request_messages` reducer: messages with `seq < before_seq`,
/// newest-first request semantics, `limit` clamped to 1–200, response
/// capped at 512 KiB. Returns messages in ascending order plus the cursor.
pub fn build_message_page(
    app: &App,
    conv_id: &str,
    before_seq: u64,
    limit: u32,
) -> Result<renv::MessagePage, CommandFailure> {
    let Some(idx) = app.chat_state.find_conv_idx(conv_id) else {
        return Err(CommandFailure::new(
            rdto::ErrorCode::UnknownConversation,
            "unknown conversation",
        ));
    };
    let limit = limit.clamp(1, 200) as usize;
    let conv = &app.chat_state.conversations[idx];
    let mut selected: Vec<rdto::Message> = Vec::new();
    let mut bytes = 0usize;
    for msg in conv.messages.iter().rev().filter(|m| m.seq < before_seq) {
        let dto = message_dto(msg);
        bytes += dto.content.len();
        if !selected.is_empty() && bytes > PAGE_BYTES {
            break;
        }
        selected.push(dto);
        if selected.len() >= limit {
            break;
        }
    }
    selected.reverse();
    let oldest_seq = selected.first().map(|m| m.seq).unwrap_or(before_seq);
    let has_older = conv.messages.iter().any(|m| m.seq < oldest_seq);
    Ok(renv::MessagePage {
        conv_id: conv_id.to_string(),
        messages: selected,
        oldest_seq,
        has_older_messages: has_older,
    })
}

/// `request_proposal` reducer: full hunk text on demand (§4.5). Resolves
/// the same owner order as review actions: open overlay copy first, then
/// the gate.
pub fn build_proposal_detail(
    app: &App,
    proposal_id: u64,
) -> Result<renv::ProposalEvent, CommandFailure> {
    let revision = app.remote.proposal_revision(proposal_id);
    if let Some(review) = app.diff_review.as_ref()
        && review.proposal.id == proposal_id
    {
        return Ok(renv::ProposalEvent {
            proposal: proposal_dto(app, &review.proposal, revision),
        });
    }
    let Ok(gate) = app.write_gate.try_lock() else {
        return Err(CommandFailure::new(
            rdto::ErrorCode::InternalError,
            "write gate busy — retry",
        ));
    };
    let Some(proposal) = gate.get_proposal(proposal_id) else {
        return Err(CommandFailure::new(
            rdto::ErrorCode::UnknownProposal,
            "unknown proposal",
        ));
    };
    let dto = proposal_dto(app, proposal, revision);
    drop(gate);
    Ok(renv::ProposalEvent { proposal: dto })
}

// ── Snapshot builder (§4.5, §6.3) ───────────────────────────────────

/// Compact, immutable, allow-listed snapshot DTO built in the event loop;
/// the hub serializes it. Never contains the absolute root, raw settings,
/// secrets, or terminal/editor state.
pub fn build_snapshot(app: &App) -> renv::Snapshot {
    let chat = &app.chat_state;
    let conversations: Vec<rdto::ConversationSummary> = (0..chat.conversations.len())
        .map(|idx| conversation_summary_dto(app, idx))
        .collect();
    let active_idx = chat.active_conv.min(chat.conversations.len().saturating_sub(1));
    let active = &chat.conversations[active_idx];

    // Bounded active tail: latest 100 messages / 512 KiB encoded.
    let mut tail: Vec<rdto::Message> = Vec::new();
    let mut bytes = 0usize;
    for msg in active.messages.iter().rev().take(SNAPSHOT_TAIL_MESSAGES) {
        let dto = message_dto(msg);
        bytes += dto.content.len();
        if !tail.is_empty() && bytes > SNAPSHOT_TAIL_BYTES {
            break;
        }
        tail.push(dto);
    }
    tail.reverse();
    let oldest_seq = tail.first().map(|m| m.seq).unwrap_or(0);
    let has_older = active.messages.iter().any(|m| m.seq < oldest_seq);

    let open_permissions: Vec<rdto::PermissionRequest> = chat
        .conversations
        .iter()
        .filter_map(|conv| {
            conv.pending_permission
                .as_ref()
                .map(|perm| permission_request_dto(&conv.id, perm))
        })
        .collect();

    // Open proposals: the gate's pending set, with the desktop overlay's
    // local copy taking precedence for the open id (§2.5 — only one owner
    // was ever mutated).
    let mut open_proposals: Vec<rdto::ProposalSummary> = Vec::new();
    if let Ok(gate) = app.write_gate.try_lock() {
        for proposal in gate.pending_proposals() {
            let overlay = app
                .diff_review
                .as_ref()
                .filter(|r| r.proposal.id == proposal.id);
            let source = overlay.map(|r| &r.proposal).unwrap_or(proposal);
            open_proposals.push(proposal_summary_dto(
                app,
                source,
                app.remote.proposal_revision(proposal.id),
            ));
        }
    }
    // The overlay may hold a proposal the gate no longer lists.
    if let Some(review) = app.diff_review.as_ref()
        && !open_proposals
            .iter()
            .any(|p| p.proposal_id == review.proposal.id)
    {
        open_proposals.push(proposal_summary_dto(
            app,
            &review.proposal,
            app.remote.proposal_revision(review.proposal.id),
        ));
    }

    renv::Snapshot {
        revision: app.remote.revision,
        conversations,
        active_id: active.id.clone(),
        active_conversation: rdto::ConversationState {
            summary: conversation_summary_dto(app, active_idx),
            messages: tail,
            oldest_seq,
            has_older_messages: has_older,
        },
        open_permissions,
        open_proposals,
        // Explicit allow-list DTO (§4.5): exactly these fields, never raw
        // workspace settings. Additions are minor-version bumps and must be
        // added to the field-level test.
        settings: rdto::RemoteSettings {
            default_model: Some(chat.agent_settings.model.clone()),
            default_effort: Some(chat.agent_settings.effort.clone()),
        },
    }
}

// ── Hot-event projection (invariant 12) ─────────────────────────────

/// How each TUI event reaches the remote mirror. The match is exhaustive
/// with NO wildcard arm — adding an `Event` variant is a compile error
/// here until someone classifies it (fail-closed projection).
pub fn project_hot_event(app: &App, event: &Event) -> Option<ServerFrame> {
    let turn_of = |conv_id: &str| -> String {
        app.chat_state
            .find_conv_idx(conv_id)
            .and_then(|idx| app.chat_state.conversations[idx].pending_turn_id.clone())
            .unwrap_or_default()
    };
    match event {
        // Streaming events carry complete wire data (§2.1 path 1).
        Event::StreamChunk { conv_id, text } => {
            if conv_id.starts_with("swarm-") {
                return None; // Swarm activity is not mirrored (scope: chat panel only).
            }
            Some(ServerFrame::StreamChunk(renv::StreamChunk {
                conv_id: conv_id.clone(),
                turn_id: turn_of(conv_id),
                text: text.clone(),
            }))
        }
        Event::ToolCallStarted { conv_id, tool_name } => {
            if conv_id.starts_with("swarm-") {
                return None;
            }
            Some(ServerFrame::ToolCallStarted(renv::ToolCallStarted {
                conv_id: conv_id.clone(),
                turn_id: turn_of(conv_id),
                tool_name: tool_name.clone(),
            }))
        }
        Event::StreamingStatus { conv_id, status } => {
            if conv_id.starts_with("swarm-") {
                return None;
            }
            Some(ServerFrame::StreamingStatus(renv::StreamingStatus {
                conv_id: conv_id.clone(),
                turn_id: turn_of(conv_id),
                status: status.clone(),
            }))
        }
        Event::TurnTokenUsage { conv_id, usage } => {
            Some(ServerFrame::TokenUsage(renv::TokenUsageEvent {
                conv_id: conv_id.clone(),
                turn_id: turn_of(conv_id),
                usage: rdto::TokenUsage {
                    input_tokens: usage.input_tokens,
                    cache_creation_input_tokens: usage.cache_creation_input_tokens,
                    cache_read_input_tokens: usage.cache_read_input_tokens,
                    output_tokens: usage.output_tokens,
                },
            }))
        }
        Event::TurnCostUpdate { conv_id, cost_usd } => {
            Some(ServerFrame::CostUpdate(renv::CostUpdate {
                conv_id: conv_id.clone(),
                turn_id: turn_of(conv_id),
                usd: *cost_usd,
            }))
        }
        // Post-reducer projection (§2.1 path 2): these mutate state the
        // controller owns; the pump's revision sweep and the controller's
        // explicit frame pushes cover them.
        Event::ProposalCreated(_)
        | Event::ProposalUpdated(_)
        | Event::BatchProposalSynced { .. }
        | Event::ProposalFinalized(_)
        | Event::MessageComplete { .. }
        | Event::FileProposalDeferred { .. }
        | Event::PermissionRequest { .. }
        | Event::AcpTaskCompleted { .. }
        | Event::AgentTurnFinished { .. } => None,
        // Internal / desktop-only events: not mirrored.
        Event::Key(_)
        | Event::Mouse(_)
        | Event::Paste(_)
        | Event::Resize(_, _)
        | Event::TerminalFocus(_)
        | Event::FileChanged(_)
        | Event::FileTreeChanged
        | Event::Terminal(_)
        | Event::ClaudeSessionStarted { .. }
        | Event::CursorSessionStarted { .. }
        | Event::ChatMemoryInjected { .. }
        | Event::TurnBootstrapMeasured { .. }
        | Event::ToolAgentEditCaptured { .. }
        | Event::ToolAgentEditsPending { .. }
        | Event::MemoryWriteEnqueued { .. }
        | Event::MemoryWriteCommitted { .. }
        | Event::MemoryWriteFailed { .. }
        | Event::MemoryManifestPersisted { .. }
        | Event::McpToolCall { .. }
        | Event::MemorySearchResults { .. }
        | Event::MemoryHistoryRows { .. }
        | Event::MemorySelectedItems { .. }
        | Event::MemoryManifestReady { .. }
        | Event::MemoryScopeSummary { .. }
        | Event::MemoryDeletionsLoaded { .. }
        | Event::SwarmPhaseChanged(_)
        | Event::SwarmAgentStateChanged { .. }
        | Event::SwarmTierStarted { .. }
        | Event::SwarmCompleted(_)
        | Event::SwarmMergeConflict { .. }
        | Event::SwarmCoordinationStarted(_)
        | Event::SwarmCoordinationComplete { .. }
        | Event::SwarmTierDispatch { .. }
        | Event::SwarmCostUpdate(_)
        | Event::SwarmDslPlanReady(_)
        | Event::MemoryReady(_)
        | Event::RemoteCommand(_)
        | Event::RemoteSnapshotNeeded
        | Event::RemoteClientConnected
        | Event::RemoteClientDisconnected
        | Event::Tick => None,
    }
}

// ── The pump (§6.3, invariant 11) ───────────────────────────────────

/// Drain reducer-produced frames and summary changes to the hub. Called at
/// the end of every `handle_event` pass. `try_send` only: a full hub
/// channel sets snapshot-dirty and the loop continues.
pub fn pump_remote(app: &mut App) {
    // Emit conversation_state_changed for every conversation whose
    // revision moved since the last pump, and conversation_removed for
    // ones that disappeared — this single sweep covers rename, reset,
    // model/effort/namespace, auto-approve, streaming transitions, and
    // active-tab changes, no matter which reducer or key handler made
    // them (§2.1 path 2).
    let active_id = app.chat_state.active_conversation_id().to_string();
    let mut summary_frames: Vec<ServerFrame> = Vec::new();
    for idx in 0..app.chat_state.conversations.len() {
        let (conv_id, revision) = {
            let conv = &app.chat_state.conversations[idx];
            (conv.id.clone(), conv.conv_revision)
        };
        let last = app.remote.projected_conv_revisions.get(&conv_id).copied();
        if last != Some(revision) {
            summary_frames.push(ServerFrame::ConversationStateChanged(
                renv::ConversationStateChanged {
                    conversation: conversation_summary_dto(app, idx),
                    active_id: active_id.clone(),
                },
            ));
            app.remote
                .projected_conv_revisions
                .insert(conv_id, revision);
        }
    }
    let live_ids: std::collections::HashSet<String> = app
        .chat_state
        .conversations
        .iter()
        .map(|c| c.id.clone())
        .collect();
    let removed: Vec<String> = app
        .remote
        .projected_conv_revisions
        .keys()
        .filter(|id| !live_ids.contains(*id))
        .cloned()
        .collect();
    for conv_id in removed {
        app.remote.projected_conv_revisions.remove(&conv_id);
        summary_frames.push(ServerFrame::ConversationRemoved(renv::ConversationRemoved {
            conv_id,
            active_id: active_id.clone(),
        }));
    }
    // Active-tab change without a revision change (switch_conversation).
    if app.remote.projected_active_id.as_deref() != Some(active_id.as_str()) {
        app.remote.projected_active_id = Some(active_id.clone());
        if let Some(idx) = app.chat_state.find_conv_idx(&active_id) {
            summary_frames.push(ServerFrame::ConversationStateChanged(
                renv::ConversationStateChanged {
                    conversation: conversation_summary_dto(app, idx),
                    active_id: active_id.clone(),
                },
            ));
        }
    }
    app.remote.pending_frames.extend(summary_frames);

    let Some(handle) = app.remote.handle.clone() else {
        // No sidecar running: drop the frames, keep revisions coherent.
        app.remote.pending_frames.clear();
        app.remote.snapshot_dirty = false;
        return;
    };

    let revision = app.remote.revision;
    for frame in std::mem::take(&mut app.remote.pending_frames) {
        if handle
            .try_send(HubInput::Event { revision, frame })
            .is_err()
        {
            // Invariant 11: never await. The hub, on resuming, sends a
            // fresh snapshot instead of the lost deltas.
            app.remote.snapshot_dirty = true;
            break;
        }
    }
    if app.remote.snapshot_dirty {
        let snapshot = build_snapshot(app);
        if handle
            .try_send(HubInput::Snapshot(Box::new(snapshot)))
            .is_ok()
        {
            app.remote.snapshot_dirty = false;
        }
    }
}
