//! Remote-owned wire DTOs (PROTOCOL.md §DTOs). Never serialize
//! `gaviero-core` domain types directly — the projection layer converts.
//!
//! Serialization rules: snake_case everywhere; optional fields are omitted
//! when absent (never `null`), with the single envelope-level exception
//! documented in [`crate::envelope`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::version::ProtocolVersion;

// ── Handshake ────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    /// Hex workspace identity (first 16 lowercase hex chars of the
    /// canonical-root SHA-256). Never an absolute path.
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Limits {
    pub max_frame_bytes: u64,
    pub max_prompt_bytes: u64,
    pub command_rate_per_second: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hello {
    pub protocol_version: ProtocolVersion,
    pub instance_id: String,
    pub tui_version: String,
    pub workspace: WorkspaceInfo,
    /// Frozen shape, empty in 1.0. Clients ignore unknown entries.
    pub capabilities: Vec<String>,
    pub confirm_required: Vec<String>,
    pub allowed_slash_commands: Vec<String>,
    pub limits: Limits,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClientHello {
    pub protocol_version: ProtocolVersion,
    pub client_name: String,
    pub client_version: String,
}

// ── Conversations ────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ContextPressure {
    pub used_tokens: u64,
    pub max_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationSummary {
    pub conv_id: String,
    pub conv_revision: u64,
    pub title: String,
    /// Full `provider:model` spec.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub is_streaming: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_pressure: Option<ContextPressure>,
    pub auto_approve: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConversationState {
    pub summary: ConversationSummary,
    /// At most the latest 100 messages / 512 KiB encoded.
    pub messages: Vec<Message>,
    /// When `messages` is empty this echoes the requested cursor.
    pub oldest_seq: u64,
    pub has_older_messages: bool,
}

// ── Messages and highlighting ────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
    System,
}

/// All offsets are UTF-8 byte offsets into `Message.content`, ends
/// exclusive; spans are absolute (not block-relative). See PROTOCOL.md
/// §Highlighting offsets.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Span {
    pub start_byte: u64,
    pub end_byte: u64,
    /// Semantic tree-sitter capture name (`keyword`, `string`, …).
    pub class: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CodeBlock {
    /// Range of the entire fenced block, fences included.
    pub start_byte: u64,
    pub end_byte: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// True ⇒ block over 256 KiB: no spans, render plain.
    pub truncated: bool,
    pub spans: Vec<Span>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    /// Monotonic per-conversation id. Survives /compact and /reset.
    /// NOT the envelope `seq`.
    pub seq: u64,
    pub role: Role,
    pub content: String,
    /// True ⇒ `content` is the head of a >128 KiB message.
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_bytes: Option<u64>,
    pub tool_calls: Vec<String>,
    pub code_blocks: Vec<CodeBlock>,
}

// ── Permissions ──────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskOption {
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskQuestion {
    pub question: String,
    pub header: String,
    pub multi_select: bool,
    pub options: Vec<AskOption>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ask {
    pub questions: Vec<AskQuestion>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PermissionRequest {
    pub conv_id: String,
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    /// Display-only. The client never sends tool input back.
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOutcome {
    Allowed,
    Denied,
    Superseded,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnsweredBy {
    Desktop,
    Remote,
    System,
}

// ── Proposals ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HunkType {
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HunkStatus {
    Pending,
    Accepted,
    Rejected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Pending,
    PartiallyAccepted,
    Accepted,
    Rejected,
    Superseded,
}

/// 0-indexed line numbers, display-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LineRange {
    pub start_line: u64,
    pub end_line: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hunk {
    /// Position in the gate's `structural_hunks` at proposal creation;
    /// stable for the proposal lifetime; echoed by `review_action`.
    pub index: u32,
    pub original_range: LineRange,
    pub proposed_range: LineRange,
    pub original_text: String,
    pub proposed_text: String,
    /// True ⇒ text elided (over 64 KiB per side). Still reviewable: the
    /// server assembles from its own copy.
    pub truncated: bool,
    pub hunk_type: HunkType,
    pub description: String,
    pub status: HunkStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Proposal {
    pub proposal_id: u64,
    pub proposal_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv_id: Option<String>,
    pub source: String,
    /// Workspace-relative.
    pub path: String,
    pub status: ProposalStatus,
    pub is_deletion: bool,
    pub conflicts_with: Vec<u64>,
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HunkSummary {
    pub index: u32,
    pub hunk_type: HunkType,
    pub description: String,
    pub status: HunkStatus,
}

/// Snapshot form: never carries hunk text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProposalSummary {
    pub proposal_id: u64,
    pub proposal_revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv_id: Option<String>,
    pub source: String,
    pub path: String,
    pub status: ProposalStatus,
    pub is_deletion: bool,
    pub conflicts_with: Vec<u64>,
    pub hunk_count: u32,
    pub added_lines: u64,
    pub removed_lines: u64,
    pub hunks: Vec<HunkSummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProposalOutcome {
    Accepted,
    PartiallyAccepted,
    Rejected,
}

// ── Settings, usage ──────────────────────────────────────────────

/// Explicit allow-list DTO — never raw workspace settings. Additions are
/// minor-version bumps and must be added to the A4 field-level test.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RemoteSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
}

/// Mirrors core's `TokenUsage` field-for-field (remote-owned copy).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

// ── Command plumbing ─────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// Terminal: validated and started; outcome arrives as lifecycle
    /// events keyed by ids in `result`. Never followed by `completed`.
    Accepted,
    Completed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidPayload,
    UnknownType,
    UnknownConversation,
    UnknownRequest,
    UnknownProposal,
    InvalidHunk,
    StaleRequest,
    StaleProposal,
    StaleConversation,
    ConversationStreaming,
    SlashNotAllowed,
    ConfirmRequired,
    TooLarge,
    RateLimited,
    DuplicateCommand,
    InternalError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewActionKind {
    AcceptHunk,
    RejectHunk,
    AcceptAll,
    RejectAll,
    Finalize,
}
