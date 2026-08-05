//! Envelopes and frames (PROTOCOL.md §Envelopes, §Client frames, §Server
//! frames). `type` + `payload` are adjacently tagged; unknown frame types
//! are surfaced as [`ClientDecode::UnknownType`] / [`ServerDecode::UnknownType`]
//! so peers can ignore (client) or answer `unknown_type` (server) instead
//! of crashing — the minor-version forward-compat contract.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::*;
use crate::version::ProtocolVersion;

// ── Client frames ────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientFrame {
    ClientHello(ClientHello),
    SendPrompt(SendPrompt),
    Slash(Slash),
    PermissionDecision(PermissionDecision),
    ReviewAction(ReviewAction),
    NewConversation {},
    SwitchConversation(SwitchConversation),
    RenameConversation(RenameConversation),
    ResetConversation(ResetConversation),
    Interrupt(Interrupt),
    RequestSnapshot {},
    RequestMessages(RequestMessages),
    RequestProposal(RequestProposal),
}

impl ClientFrame {
    /// Every wire `type` string this version understands.
    pub const TYPE_NAMES: &'static [&'static str] = &[
        "client_hello",
        "send_prompt",
        "slash",
        "permission_decision",
        "review_action",
        "new_conversation",
        "switch_conversation",
        "rename_conversation",
        "reset_conversation",
        "interrupt",
        "request_snapshot",
        "request_messages",
        "request_proposal",
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SendPrompt {
    pub conv_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Slash {
    pub conv_id: String,
    pub line: String,
    /// Must be true for commands listed in `hello.confirm_required`.
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PermissionDecision {
    pub request_id: String,
    pub allow: bool,
    /// Per question, selected option indices. Only valid for
    /// `AskUserQuestion` permissions; never a tool-input document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<Vec<Vec<u32>>>,
    /// Free text on deny; size-limited, control-character filtered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReviewAction {
    pub proposal_id: u64,
    pub proposal_revision: u64,
    pub action: ReviewActionKind,
    /// Required for `accept_hunk` / `reject_hunk`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunk_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SwitchConversation {
    pub conv_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RenameConversation {
    pub conv_id: String,
    pub conv_revision: u64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ResetConversation {
    pub conv_id: String,
    pub conv_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Interrupt {
    pub conv_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestMessages {
    pub conv_id: String,
    pub before_seq: u64,
    /// Clamped server-side to 1–200.
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RequestProposal {
    pub proposal_id: u64,
}

// ── Server frames ────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello(Hello),
    Snapshot(Snapshot),
    ConversationStateChanged(ConversationStateChanged),
    ConversationRemoved(ConversationRemoved),
    MessagePage(MessagePage),
    StreamChunk(StreamChunk),
    StreamingStatus(StreamingStatus),
    StreamingEnded(StreamingEnded),
    ToolCallStarted(ToolCallStarted),
    MessageComplete(MessageComplete),
    PermissionRequest(PermissionRequest),
    PermissionClosed(PermissionClosed),
    ProposalCreated(ProposalEvent),
    ProposalUpdated(ProposalEvent),
    ProposalDetail(ProposalEvent),
    ProposalFinalized(ProposalFinalized),
    TokenUsage(TokenUsageEvent),
    CostUpdate(CostUpdate),
    CommandResult(CommandResult),
    CommandError(CommandError),
}

impl ServerFrame {
    /// Every wire `type` string this version emits.
    pub const TYPE_NAMES: &'static [&'static str] = &[
        "hello",
        "snapshot",
        "conversation_state_changed",
        "conversation_removed",
        "message_page",
        "stream_chunk",
        "streaming_status",
        "streaming_ended",
        "tool_call_started",
        "message_complete",
        "permission_request",
        "permission_closed",
        "proposal_created",
        "proposal_updated",
        "proposal_detail",
        "proposal_finalized",
        "token_usage",
        "cost_update",
        "command_result",
        "command_error",
    ];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub revision: u64,
    pub conversations: Vec<ConversationSummary>,
    pub active_id: String,
    pub active_conversation: ConversationState,
    pub open_permissions: Vec<PermissionRequest>,
    pub open_proposals: Vec<ProposalSummary>,
    pub settings: RemoteSettings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationStateChanged {
    pub conversation: ConversationSummary,
    pub active_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationRemoved {
    pub conv_id: String,
    pub active_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessagePage {
    pub conv_id: String,
    /// Newest-first request semantics; page capped at 512 KiB.
    pub messages: Vec<Message>,
    /// When `messages` is empty this echoes the requested cursor.
    pub oldest_seq: u64,
    pub has_older_messages: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamChunk {
    pub conv_id: String,
    pub turn_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamingStatus {
    pub conv_id: String,
    pub turn_id: String,
    /// Free-form display string.
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StreamingEnded {
    pub conv_id: String,
    pub turn_id: String,
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub proposal_count: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCallStarted {
    pub conv_id: String,
    pub turn_id: String,
    pub tool_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageComplete {
    pub conv_id: String,
    pub message: Message,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PermissionClosed {
    pub conv_id: String,
    pub request_id: String,
    pub outcome: PermissionOutcome,
    pub answered_by: AnsweredBy,
}

/// Shared payload of `proposal_created` / `proposal_updated` /
/// `proposal_detail` — one shape, three lifecycle meanings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProposalEvent {
    pub proposal: Proposal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProposalFinalized {
    pub proposal_id: u64,
    pub path: String,
    pub outcome: ProposalOutcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TokenUsageEvent {
    pub conv_id: String,
    pub turn_id: String,
    pub usage: TokenUsage,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostUpdate {
    pub conv_id: String,
    pub turn_id: String,
    pub usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandResult {
    pub command_id: String,
    pub status: CommandStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    pub command_id: String,
    pub code: ErrorCode,
    pub message: String,
}

// ── Envelopes ────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClientEnvelope {
    pub version: ProtocolVersion,
    /// Always present on the wire. `None` (serialized as literal `null`)
    /// exactly on `client_hello` — the only frame sent before the server's
    /// `hello` delivers the id. Every later frame echoes it.
    pub instance_id: Option<String>,
    /// Client-generated, unique per command. The server deduplicates
    /// repeats within a bounded recent-ID cache.
    pub command_id: String,
    #[serde(flatten)]
    pub frame: ClientFrame,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerEnvelope {
    pub version: ProtocolVersion,
    pub instance_id: String,
    /// Monotonic per `instance_id`, assigned at emission, continues across
    /// socket generations. A gap ⇒ `request_snapshot`.
    pub seq: u64,
    /// Global snapshot generation. Staleness display only — never a
    /// command precondition.
    pub revision: u64,
    #[serde(flatten)]
    pub frame: ServerFrame,
}

// ── Forward-compatible decoding ──────────────────────────────────

/// Envelope skeleton used to classify frames that fail full decoding.
#[derive(Debug, Deserialize)]
struct RawFrame {
    #[serde(rename = "type")]
    frame_type: String,
    #[serde(default)]
    command_id: Option<String>,
}

/// Result of decoding a frame received *by the client*.
#[derive(Debug)]
pub enum ServerDecode {
    Frame(Box<ServerEnvelope>),
    /// A frame type this version does not know. Ignore it (log a warning);
    /// this is how minor-version additions stay compatible.
    UnknownType { frame_type: String },
}

/// Result of decoding a frame received *by the server*.
#[derive(Debug)]
pub enum ClientDecode {
    Frame(Box<ClientEnvelope>),
    /// Unknown command type: answer `command_error { unknown_type }` using
    /// the envelope's `command_id` when it parsed.
    UnknownType {
        frame_type: String,
        command_id: Option<String>,
    },
}

pub fn decode_server_frame(json: &str) -> Result<ServerDecode, serde_json::Error> {
    match serde_json::from_str::<ServerEnvelope>(json) {
        Ok(env) => Ok(ServerDecode::Frame(Box::new(env))),
        Err(err) => {
            if let Ok(raw) = serde_json::from_str::<RawFrame>(json)
                && !ServerFrame::TYPE_NAMES.contains(&raw.frame_type.as_str())
            {
                return Ok(ServerDecode::UnknownType {
                    frame_type: raw.frame_type,
                });
            }
            Err(err)
        }
    }
}

pub fn decode_client_frame(json: &str) -> Result<ClientDecode, serde_json::Error> {
    match serde_json::from_str::<ClientEnvelope>(json) {
        Ok(env) => Ok(ClientDecode::Frame(Box::new(env))),
        Err(err) => {
            if let Ok(raw) = serde_json::from_str::<RawFrame>(json)
                && !ClientFrame::TYPE_NAMES.contains(&raw.frame_type.as_str())
            {
                return Ok(ClientDecode::UnknownType {
                    frame_type: raw.frame_type,
                    command_id: raw.command_id,
                });
            }
            Err(err)
        }
    }
}
