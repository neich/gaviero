//! Agent chat panel — conversation history + input for provider-backed interaction.

use ratatui::{
    buffer::Buffer as RataBuf,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

use crate::app::collapse_file_blocks;
use crate::theme;
use crate::theme::Theme;
use crate::widgets::render_utils::write_text;
use crate::widgets::text_input::TextInput;

const CHAT_RENDER_TRACE_ENV: &str = "GAVIERO_CHAT_RENDER_TRACE";
const CHAT_RENDER_TRACE_MS_ENV: &str = "GAVIERO_CHAT_RENDER_TRACE_MS";
const DEFAULT_CHAT_RENDER_TRACE_MS: u128 = 16;

// ── Data types ──────────────────────────────────────────────────

/// Clickable close control for one attachment badge.
#[derive(Debug, Clone)]
pub struct AttachmentCloseHit {
    /// Inclusive start column of the `x` glyph.
    pub x: u16,
    /// Row of the attachment bar.
    pub y: u16,
    /// Index into [`AgentChatState::attachments`] at render time.
    pub index: usize,
}

/// Type of file attachment.
#[derive(Debug, Clone, PartialEq)]
pub enum AttachmentKind {
    /// Text/code file — contents included in prompt context.
    Text,
    /// Image file — passed to providers that support file attachments.
    Image,
}

/// A file attached to the next chat message.
#[derive(Debug, Clone)]
pub struct Attachment {
    /// Absolute path to the file on disk.
    pub path: PathBuf,
    /// Display name (filename component).
    pub display_name: String,
    /// Type of attachment.
    pub kind: AttachmentKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    /// Monotonic per-conversation identity (Plan A §2.6), assigned from
    /// `Conversation::next_message_seq` when the message is pushed. NOT
    /// reset by `/compact` or `/reset` — surviving messages keep their
    /// `seq`, so a stale remote cursor resolves to an empty range instead
    /// of different content. Distinct from the wire envelope's `seq`.
    pub seq: u64,
    pub role: ChatRole,
    pub content: String,
    pub tool_calls: Vec<String>,
}

fn chat_render_trace_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(CHAT_RENDER_TRACE_ENV)
            .map(|value| {
                let value = value.trim();
                !value.is_empty()
                    && value != "0"
                    && !value.eq_ignore_ascii_case("false")
                    && !value.eq_ignore_ascii_case("off")
            })
            .unwrap_or(false)
    })
}

fn chat_render_trace_threshold_ms() -> u128 {
    static THRESHOLD_MS: OnceLock<u128> = OnceLock::new();
    *THRESHOLD_MS.get_or_init(|| {
        std::env::var(CHAT_RENDER_TRACE_MS_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u128>().ok())
            .unwrap_or(DEFAULT_CHAT_RENDER_TRACE_MS)
    })
}

// ── Chat state ──────────────────────────────────────────────────

/// What the autocomplete popup is completing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutocompleteMode {
    /// `@path` references inside the prompt body — workspace files only,
    /// inserts `@path ` on accept.
    FileRef,
    /// `/attach <path>` argument — workspace files plus filesystem listing
    /// when the partial starts with `/` or `~`, inserts the bare path.
    AttachPath,
    /// `/detach <name|all>` argument — current attachment display names
    /// plus `all`, inserts the bare token.
    DetachName,
    /// `$skill` invocation — catalog-backed name completion.
    SkillRef,
    /// `/model <provider:model>` argument — provider prefix and model names.
    ModelSpec,
}

/// Autocomplete state for @file references or /attach|/detach arguments.
#[derive(Debug)]
pub struct FileAutocomplete {
    /// Whether the autocomplete popup is visible.
    pub active: bool,
    /// The partial text being matched (excludes any leading `@`).
    pub query: String,
    /// Byte offset where the inserted path will be anchored. For [`FileRef`]
    /// this points at the `@`; for [`AttachPath`]/[`DetachName`] it points
    /// at the first character of the argument (after `/attach ` / `/detach `).
    ///
    /// [`FileRef`]: AutocompleteMode::FileRef
    /// [`AttachPath`]: AutocompleteMode::AttachPath
    /// [`DetachName`]: AutocompleteMode::DetachName
    pub at_pos: usize,
    /// What kind of completion is active.
    pub mode: AutocompleteMode,
    /// Filtered file paths matching the query.
    pub matches: Vec<String>,
    /// Currently selected index in the matches list.
    pub selected: usize,
}

impl FileAutocomplete {
    fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            at_pos: 0,
            mode: AutocompleteMode::FileRef,
            matches: Vec::new(),
            selected: 0,
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.query.clear();
        self.at_pos = 0;
        self.mode = AutocompleteMode::FileRef;
        self.matches.clear();
        self.selected = 0;
    }
}

/// Word-wrap `text` to `width` columns, prefixing the first row with
/// `first_indent` and every continuation row with `cont_indent`.
///
/// Overlay rows are written straight into the buffer, so each returned row
/// already carries its indent and fits inside `width` columns.
fn wrap_indented(text: &str, width: usize, first_indent: &str, cont_indent: &str) -> Vec<String> {
    let indent_w = UnicodeWidthStr::width(cont_indent).max(UnicodeWidthStr::width(first_indent));
    let body_w = width.saturating_sub(indent_w).max(1);
    crate::widgets::render_utils::word_wrap(text, body_w)
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let indent = if i == 0 { first_indent } else { cont_indent };
            format!("{indent}{line}")
        })
        .collect()
}

/// Process-unique id for a pending permission (Plan A §2.2): the remote
/// client answers by `request_id`, and first-writer-wins races are decided
/// by whether the id still names a parked request.
fn next_request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("req-{millis:x}-{n:x}")
}

/// A pending permission request from the agent subprocess.
/// Held in `Conversation::pending_permission` while the user decides.
pub struct PendingPermission {
    /// Stable id the remote protocol addresses this request by. The first
    /// valid local or remote answer takes the oneshot sender and wins.
    pub request_id: String,
    pub tool_name: String,
    pub description: String,
    /// Original tool input (needed to echo `updatedInput` on allow, and to
    /// drive the AskUserQuestion multi-choice UI).
    pub input: serde_json::Value,
    /// Send allow/deny (with optional updated input for AskUserQuestion).
    pub respond: tokio::sync::oneshot::Sender<gaviero_core::observer::PermissionDecision>,
    /// Interactive AskUserQuestion state. `None` for plain y/n tools.
    pub ask: Option<AskUserQuestionState>,
    /// First visible body row of the overlay (PgUp/PgDn). The request text is
    /// word-wrapped, so a long question or command can still outgrow the
    /// overlay even after it auto-grows.
    pub scroll: usize,
}

impl PendingPermission {
    pub fn new(
        tool_name: String,
        description: String,
        input: serde_json::Value,
        respond: tokio::sync::oneshot::Sender<gaviero_core::observer::PermissionDecision>,
    ) -> Self {
        let ask = if tool_name == "AskUserQuestion" {
            AskUserQuestionState::from_input(&input)
        } else {
            None
        };
        Self {
            request_id: next_request_id(),
            tool_name,
            description,
            input,
            respond,
            ask,
            scroll: 0,
        }
    }

    pub fn is_ask_user_question(&self) -> bool {
        self.ask.is_some()
    }

    /// Word-wrapped body rows of the overlay: `(text, is_selected_option)`.
    /// `width` is the overlay's total column count; the header and key-hint
    /// rows are not included.
    pub fn overlay_rows(&self, width: usize) -> Vec<(String, bool)> {
        match self.ask.as_ref() {
            Some(ask) => ask.body_rows(width),
            None => wrap_indented(&self.description, width, " ", " ")
                .into_iter()
                .map(|line| (line, false))
                .collect(),
        }
    }
}

/// One multiple-choice question from Claude's `AskUserQuestion` tool.
#[derive(Debug, Clone)]
pub struct AskQuestion {
    pub question: String,
    pub header: String,
    pub multi_select: bool,
    pub options: Vec<(String, String)>, // (label, description)
}

/// UI state while answering an `AskUserQuestion` prompt.
#[derive(Debug, Clone)]
pub struct AskUserQuestionState {
    pub questions: Vec<AskQuestion>,
    /// Selected option indices per question (multi-select uses a bitset via Vec).
    pub selected: Vec<Vec<usize>>,
    pub focus_q: usize,
}

impl AskUserQuestionState {
    pub fn from_input(input: &serde_json::Value) -> Option<Self> {
        let arr = input.get("questions")?.as_array()?;
        if arr.is_empty() {
            return None;
        }
        let mut questions = Vec::new();
        for q in arr {
            let question = q.get("question")?.as_str()?.to_string();
            let header = q
                .get("header")
                .and_then(|h| h.as_str())
                .unwrap_or("")
                .to_string();
            let multi_select = q
                .get("multiSelect")
                .and_then(|m| m.as_bool())
                .unwrap_or(false);
            let options = q
                .get("options")
                .and_then(|o| o.as_array())
                .map(|opts| {
                    opts.iter()
                        .filter_map(|o| {
                            Some((
                                o.get("label")?.as_str()?.to_string(),
                                o.get("description")
                                    .and_then(|d| d.as_str())
                                    .unwrap_or("")
                                    .to_string(),
                            ))
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if options.is_empty() {
                continue;
            }
            questions.push(AskQuestion {
                question,
                header,
                multi_select,
                options,
            });
        }
        if questions.is_empty() {
            return None;
        }
        let selected = vec![Vec::new(); questions.len()];
        Some(Self {
            questions,
            selected,
            focus_q: 0,
        })
    }

    /// Word-wrapped rows for the focused question: `(text, is_selected)`.
    /// Continuation rows hang under the text they belong to.
    pub fn body_rows(&self, width: usize) -> Vec<(String, bool)> {
        let mut rows = Vec::new();
        let Some(q) = self.questions.get(self.focus_q) else {
            return rows;
        };
        let title = if q.header.is_empty() {
            format!("{}. {}", self.focus_q + 1, q.question)
        } else {
            format!("{}. [{}] {}", self.focus_q + 1, q.header, q.question)
        };
        rows.extend(
            wrap_indented(&title, width, " ", "    ")
                .into_iter()
                .map(|line| (line, false)),
        );
        for (oi, (label, desc)) in q.options.iter().enumerate() {
            let marked = self.selected[self.focus_q].contains(&oi);
            let mark = if marked { "●" } else { "○" };
            let text = if desc.is_empty() {
                format!("{mark} {}. {label}", oi + 1)
            } else {
                format!("{mark} {}. {label} — {desc}", oi + 1)
            };
            rows.extend(
                wrap_indented(&text, width, "  ", "       ")
                    .into_iter()
                    .map(|line| (line, marked)),
            );
        }
        rows
    }

    pub fn toggle_option(&mut self, opt_idx: usize) {
        let Some(q) = self.questions.get(self.focus_q) else {
            return;
        };
        if opt_idx >= q.options.len() {
            return;
        }
        let sel = &mut self.selected[self.focus_q];
        if q.multi_select {
            if let Some(pos) = sel.iter().position(|&i| i == opt_idx) {
                sel.remove(pos);
            } else {
                sel.push(opt_idx);
            }
        } else {
            *sel = vec![opt_idx];
        }
    }

    pub fn all_answered(&self) -> bool {
        self.selected.iter().all(|s| !s.is_empty())
    }

    /// Build the `answers` map Claude expects (question text → label).
    pub fn answers_map(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut answers = serde_json::Map::new();
        for (qi, q) in self.questions.iter().enumerate() {
            let labels: Vec<String> = self.selected[qi]
                .iter()
                .filter_map(|&oi| q.options.get(oi).map(|(l, _)| l.clone()))
                .collect();
            let value = if q.multi_select {
                serde_json::Value::String(labels.join(", "))
            } else {
                serde_json::Value::String(labels.first().cloned().unwrap_or_default())
            };
            answers.insert(q.question.clone(), value);
        }
        answers
    }
}

/// Data for the `permission_closed` projection after an answer.
#[derive(Debug, Clone)]
pub struct PermissionClosedInfo {
    pub conv_id: String,
    pub request_id: String,
    pub allowed: bool,
}

/// Why a permission answer was refused (the request stays parked on
/// `Invalid`; `NoPending` means the id no longer names a live request —
/// the other side answered first).
#[derive(Debug, Clone)]
pub enum PermissionAnswerError {
    NoPending,
    Invalid(String),
}

/// Who issued a slash command. Policy and desktop-only UI affordances
/// branch on this; mutation semantics never do (Plan A §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlashOrigin {
    Desktop,
    Remote,
}

impl std::fmt::Debug for PendingPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingPermission")
            .field("tool_name", &self.tool_name)
            .field("description", &self.description)
            .field("ask", &self.ask.is_some())
            .finish_non_exhaustive()
    }
}

/// Whether the next first-turn dispatch should re-inline the visible chat
/// transcript into the spilled prompt.
///
/// `is_first_turn` (from `SessionLedger::is_first_turn`) gates *bootstrap
/// context* — graph + memory blocks the agent needs because Claude has no
/// server-side session yet. That signal alone used to also gate transcript
/// inlining, which is what made `/reset` ineffective: the cached
/// `claude_session_id` got dropped, but the next turn cheerfully re-sent
/// every prior user/assistant turn, defeating the user's intent. This flag
/// splits the two concerns:
///
/// * `Auto` — default. Inline the transcript when it's a first turn (e.g.
///   on app launch or after rehydrate-from-disk where `--resume` may not
///   work yet).
/// * `Suppress` — skip transcript inlining on the next first turn, even
///   when bootstrap context is needed. Set by `/reset` and `/clear`.
///   Cleared back to `Auto` once Claude opens a fresh session
///   (handled in the SystemInit event path) so subsequent resets behave
///   the same way.
/// * `Force` — reserved for callers that always want the transcript in;
///   not wired up today, but the explicit variant keeps the semantics
///   readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptInlineMode {
    Auto,
    Suppress,
    /// Reserved variant: callers that want to *guarantee* transcript
    /// inlining regardless of `is_first_turn`. No call site sets this
    /// today; the variant exists so the read site in `side_panel.rs`
    /// has a stable shape and a future "include transcript anyway" toggle
    /// is a one-line change rather than a refactor.
    #[allow(dead_code)]
    Force,
}

impl Default for TranscriptInlineMode {
    fn default() -> Self {
        Self::Auto
    }
}

/// One conversation (tab) in the chat panel.
#[derive(Debug)]
pub struct Conversation {
    pub id: String,
    /// Per-entity freshness token (Plan A §4.8): bumped whenever
    /// summary-visible state changes (title, model/effort/namespace,
    /// auto-approve, streaming transitions). `rename_conversation` /
    /// `reset_conversation` remote commands must present the current value.
    pub conv_revision: u64,
    /// Next `ChatMessage.seq` to assign — monotonic for the life of the
    /// conversation, never reset by `/compact` or `/reset` (§2.6).
    pub next_message_seq: u64,
    pub title: String,
    pub messages: Vec<ChatMessage>,
    /// Per-conversation model override (None = use global default).
    pub model_override: Option<String>,
    /// Per-conversation effort level override (None = use global default).
    pub effort_override: Option<String>,
    /// Per-conversation memory namespace override (None = use workspace default).
    pub namespace_override: Option<String>,
    /// Whether this conversation is currently streaming a response.
    pub is_streaming: bool,
    /// Current activity description shown during streaming (e.g. "Thinking...", "Reading src/main.rs").
    pub streaming_status: String,
    /// When streaming started, for elapsed time display.
    pub streaming_started_at: Option<Instant>,
    /// Persistent auto-approve for this conversation (toggled via `/autoapprove`).
    pub auto_approve: bool,
    /// Pending permission request waiting for user approval (y/n).
    pub pending_permission: Option<PendingPermission>,
    /// Turn id assigned when the user message is dispatched. The same id
    /// ties prompt-time injection manifests to the completion extractor.
    pub pending_turn_id: Option<String>,
    /// Module path captured at dispatch time for scoped memory writes.
    pub pending_module_path: Option<String>,
    /// Workspace folder root the planner used for this dispatched turn —
    /// derived from the active buffer's path when one is open. The
    /// controller reads this back to hash `repo_id` against the same
    /// folder, so memory reads (planner) and writes (post-turn) land in
    /// the same per-folder DB. `None` means the active buffer was outside
    /// every workspace folder (or no buffer was open) and the primary
    /// workspace root was used.
    pub pending_focused_folder: Option<std::path::PathBuf>,
    /// One-shot flag toggled by `/workspace`. When `true`, the next
    /// dispatched turn ignores the focused folder and falls back to the
    /// workspace-wide default scope (`app.graph_workspace_root`). The
    /// flag self-clears after `send_chat_message` consumes it. Useful when
    /// the user knows the prompt is genuinely cross-folder and the
    /// active-buffer heuristic would narrow incorrectly.
    pub workspace_wide_next: bool,
    /// One-shot flag toggled by `/lite`. When `true`, the next dispatched
    /// turn skips outline, memory, and impact; keeps `<repo_topology>`.
    /// Self-clears after `send_chat_message` consumes it.
    pub lite_next: bool,
    /// One-shot flag toggled by `/no-inject`. Suppresses every bootstrap
    /// layer on the next dispatch (including topology).
    pub no_inject_next: bool,
    /// Accumulated per-layer arms from `/inject <layer>` before the next
    /// send. Merged across multiple `/inject` calls; consumed on dispatch.
    pub inject_arms_next: gaviero_core::context_planner::BootstrapArms,
    /// Per-conversation override of `agent.context.bootstrap` from settings.
    pub context_mode_override: Option<gaviero_core::context_planner::BootstrapMode>,
    /// Claude's session id, captured from the first turn's `SystemInit` event.
    /// Subsequent turns pass this back via `--resume <id>` so Claude keeps
    /// conversation memory server-side and we don't re-send history.
    pub claude_session_id: Option<String>,
    /// Per-conversation planner ledger (V9 §4 `SessionLedger`).
    ///
    /// **Lazily initialized** on the first send: the model isn't known until
    /// the user sends, and the `ProviderProfile` factory needs the model.
    /// Cleared on `reset_conversation` along with `claude_session_id`.
    /// M4 persists this across restarts via `pending_persisted_ledger`.
    pub session_ledger: Option<gaviero_core::context_planner::SessionLedger>,
    /// M4: on restore from disk, the persisted ledger lands here. The
    /// first send rehydrates [`session_ledger`] by calling
    /// `SessionLedger::from_persisted` with the current `ProviderProfile`,
    /// then runs `invalidate_if_fingerprint_changed` to drop the handle
    /// if the model/toolset has changed since the save. Consumed (taken)
    /// once rehydration runs.
    pub pending_persisted_ledger: Option<gaviero_core::context_planner::ledger::PersistedLedger>,
    /// Controls whether the visible chat transcript is re-inlined into the
    /// next first-turn prompt. See [`TranscriptInlineMode`].
    pub transcript_inline_mode: TranscriptInlineMode,
    /// Latest server-reported token usage for this conversation (Claude
    /// `result.usage` today). Updated once per turn from
    /// `Event::TurnTokenUsage`. `None` before the first turn completes,
    /// or for providers that don't surface usage.
    pub last_token_usage: Option<gaviero_core::acp::protocol::TokenUsage>,
    /// Accumulated API cost for the last turn (`Event::TurnCostUpdate`).
    pub last_turn_cost_usd: f64,
    /// Gaviero bootstrap measured on the last send (`TurnBootstrapMeasured`):
    /// topology, graph outline, memory block, `@file` refs. Zero on follow-up
    /// turns that skip bootstrap.
    pub last_bootstrap_tokens: usize,
    /// Bootstrap arms that produced [`last_bootstrap_tokens`].
    pub last_bootstrap_arms: gaviero_core::context_planner::BootstrapArms,
    /// Memory injection size from the last `ChatMemoryInjected` event.
    pub last_memory_injection_tokens: usize,
}

impl Conversation {
    /// Push a message, assigning its monotonic `seq` (§2.6). The single
    /// construction path for live messages — direct `ChatMessage {}`
    /// literals outside restore/compact keep-list handling are a bug.
    pub fn push_message(&mut self, role: ChatRole, content: String, tool_calls: Vec<String>) {
        let seq = self.next_message_seq;
        self.next_message_seq += 1;
        self.messages.push(ChatMessage {
            seq,
            role,
            content,
            tool_calls,
        });
    }

    /// Bump the per-entity freshness token after a summary-visible change
    /// (§4.8). Callers that project `conversation_state_changed` read the
    /// bumped value in the same transition.
    pub fn bump_revision(&mut self) {
        self.conv_revision += 1;
    }
}

/// Context-window pressure shown in the status bar and `/context`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextBarSource {
    /// `last_token_usage.prefix_tokens()` from the provider (Claude/Cursor).
    ProviderPrefix,
    /// Transcript + last bootstrap + hidden provider overhead (Codex exec, pre-result).
    CompositeEstimate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextPressure {
    pub tokens: usize,
    pub pct: usize,
    pub source: ContextBarSource,
    pub transcript_tokens: usize,
    pub bootstrap_tokens: usize,
    pub hidden_overhead_tokens: usize,
}

impl ContextPressure {
    pub fn is_approximate(self) -> bool {
        self.source == ContextBarSource::CompositeEstimate
    }
}

/// Global agent settings read from workspace settings.
#[derive(Debug, Clone)]
pub struct AgentSettings {
    pub model: String,
    pub effort: String,
    pub max_tokens: u32,
    pub ollama_base_url: String,
    /// The namespace to write memories to.
    pub write_namespace: String,
    /// Namespaces to search when reading (always includes write_namespace).
    pub read_namespaces: Vec<String>,
    /// Token budget for graph-based source-code context injection. 0 disables graph context.
    pub graph_budget_tokens: usize,
    /// PUSH→PULL Phase 1 thin-anchor outline budget for strong providers
    /// (`agent.anchorBudgetTokens`). The default first turn injects an outline
    /// at this budget and lets the model pull bodies via the MCP tools.
    pub anchor_budget_tokens: usize,
    /// Default chat bootstrap mode (`agent.context.bootstrap`).
    pub bootstrap_mode: gaviero_core::context_planner::BootstrapMode,
    /// PUSH→PULL Phase 4 tier override (`agent.bootstrapTier`). Empty = derive
    /// the tier from provider capabilities; `"strong"` / `"smalllocal"` force it.
    pub bootstrap_tier_override: String,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            model: "claude:sonnet".to_string(),
            effort: "off".to_string(),
            max_tokens: 16384,
            ollama_base_url: "http://localhost:11434".to_string(),
            write_namespace: "default".to_string(),
            read_namespaces: vec!["default".to_string()],
            graph_budget_tokens: 12_000,
            anchor_budget_tokens: 1_200,
            bootstrap_mode: gaviero_core::context_planner::BootstrapMode::Auto,
            bootstrap_tier_override: String::new(),
        }
    }
}

#[derive(Debug)]
pub struct AgentChatState {
    /// All conversations for this workspace.
    pub conversations: Vec<Conversation>,
    /// Index of the active conversation.
    pub active_conv: usize,

    pub text_input: TextInput,
    /// User-resized input area height (0 = auto-size from content).
    pub input_area_rows: u16,
    pub scroll_offset: usize,
    /// When true, the next render pass will snap scroll to the bottom.
    pub scroll_pinned_to_bottom: bool,
    /// Current position in history (None = new input, Some(idx) = browsing user messages).
    pub history_index: Option<usize>,
    /// Stashed current input when browsing history.
    pub history_stash: String,
    /// When true, the input field is editing the conversation title instead of a chat message.
    pub renaming: bool,
    pub autocomplete: FileAutocomplete,
    /// Files attached to the next message.
    pub attachments: Vec<Attachment>,
    /// Screen hit regions for attachment-bar close (`x`) buttons, rebuilt
    /// each render. Absolute terminal coordinates.
    pub attachment_close_hits: Vec<AttachmentCloseHit>,

    /// Global agent settings (from workspace config).
    pub agent_settings: AgentSettings,

    /// When true, user is browsing messages to copy content.
    pub browse_mode: bool,
    /// Index of the currently highlighted message (into active conversation's messages).
    pub browsed_msg: usize,
    /// Cached model options discovered from provider tooling (lazily populated).
    cli_model_options: Option<Vec<String>>,
    /// Tick counter for spinner animation (incremented on each Event::Tick while streaming).
    pub tick_count: u64,

    /// Cached rendered line texts (set during render) for mouse text selection.
    /// Each entry is (text, message_index) where message_index groups lines from the same message.
    pub rendered_lines_cache: Vec<(String, Option<usize>)>,
    /// Cached conversation area rect (set during render).
    pub conv_area_cache: Option<Rect>,
    /// Cached prompt-input area rect (set during render) for mouse wheel hit-testing.
    pub input_area_cache: Option<Rect>,
    /// Text selection anchor: (rendered_line_index, char_index).
    pub text_sel_anchor: Option<(usize, usize)>,
    /// Text selection end: (rendered_line_index, char_index).
    pub text_sel_end: Option<(usize, usize)>,
    /// Whether mouse is currently dragging to select chat text.
    pub chat_dragging: bool,
    /// Whether mouse is dragging a selection inside the prompt input.
    pub input_dragging: bool,
    /// Keyboard cursor line index into rendered_lines_cache, for Shift+Arrow selection.
    pub chat_output_kb_cursor: Option<usize>,
    /// When true, the user has manually scrolled during streaming, so auto-scroll is paused.
    pub user_scrolled_during_stream: bool,
    /// When true, the next prompt will be sent with `--dangerously-skip-permissions`.
    /// Toggled with Alt+Y. Resets to false after the message is sent.
    pub auto_approve_next: bool,
}

impl AgentChatState {
    pub fn new() -> Self {
        let conv = Conversation {
            id: gaviero_core::session_state::new_conversation_id(),
            conv_revision: 1,
            next_message_seq: 1,
            title: "New Chat".to_string(),
            messages: Vec::new(),
            model_override: None,
            effort_override: None,
            namespace_override: None,
            is_streaming: false,
            streaming_status: String::new(),
            streaming_started_at: None,
            auto_approve: false,
            pending_permission: None,
            pending_turn_id: None,
            pending_module_path: None,
            pending_focused_folder: None,
            workspace_wide_next: false,
            lite_next: false,
            no_inject_next: false,
            inject_arms_next: gaviero_core::context_planner::BootstrapArms::none(),
            context_mode_override: None,
            claude_session_id: None,
            session_ledger: None,
            pending_persisted_ledger: None,
            transcript_inline_mode: TranscriptInlineMode::Auto,
            last_token_usage: None,
            last_turn_cost_usd: 0.0,
            last_bootstrap_tokens: 0,
            last_bootstrap_arms: gaviero_core::context_planner::BootstrapArms::none(),
            last_memory_injection_tokens: 0,
        };
        Self {
            conversations: vec![conv],
            active_conv: 0,
            text_input: TextInput::new(),
            input_area_rows: 0,
            scroll_offset: 0,
            scroll_pinned_to_bottom: false,
            history_index: None,
            history_stash: String::new(),
            autocomplete: FileAutocomplete::new(),
            attachments: Vec::new(),
            attachment_close_hits: Vec::new(),
            renaming: false,
            agent_settings: AgentSettings::default(),
            browse_mode: false,
            browsed_msg: 0,
            cli_model_options: None,
            tick_count: 0,
            rendered_lines_cache: Vec::new(),
            conv_area_cache: None,
            input_area_cache: None,
            text_sel_anchor: None,
            text_sel_end: None,
            chat_dragging: false,
            input_dragging: false,
            chat_output_kb_cursor: None,
            user_scrolled_during_stream: false,
            auto_approve_next: false,
        }
    }

    /// Get model options from provider tooling (cached after first call).
    pub(crate) fn model_options(&mut self) -> &[String] {
        if self.cli_model_options.is_none() {
            let mut options = gaviero_core::acp::session::discover_model_options();
            // Always offer the canonical Claude aliases, even when the CLI is
            // absent or its `--help` text drifts and discovery comes back
            // empty. Merged (not replacing) so discovered full model names
            // still surface.
            for alias in gaviero_core::swarm::backend::shared::CLAUDE_MODEL_ALIASES {
                let spec = format!("claude:{alias}");
                if !options.iter().any(|opt| opt == &spec) {
                    options.push(spec);
                }
            }
            // Codex has no CLI discovery — pin the documented aliases so
            // `/model` Available and Tab completion always offer them.
            for alias in gaviero_core::swarm::backend::shared::CODEX_MODEL_ALIASES {
                let spec = format!("codex:{alias}");
                if !options.iter().any(|opt| opt == &spec) {
                    options.push(spec);
                }
            }
            for cursor_model in gaviero_core::acp::session::discover_cursor_model_options() {
                if !options.iter().any(|opt| opt == &cursor_model) {
                    options.push(cursor_model);
                }
            }
            for deepseek_model in gaviero_core::swarm::backend::shared::DEEPSEEK_API_MODELS {
                let spec = format!("deepseek:{deepseek_model}");
                if !options.iter().any(|opt| opt == &spec) {
                    options.push(spec);
                }
            }
            let ollama_example = "ollama:qwen2.5-coder:7b".to_string();
            if !options.iter().any(|opt| opt == &ollama_example) {
                options.push(ollama_example);
            }
            self.cli_model_options = Some(options);
        }
        self.cli_model_options.as_deref().unwrap_or(&[])
    }

    /// Active conversation accessor guarded by the state invariant:
    /// `active_conv` must always point to an existing conversation.
    pub fn active_conversation(&self) -> &Conversation {
        self.conversations
            .get(self.active_conv)
            .expect("active_conv out of bounds; invariant broken (no active conversation at index)")
    }

    /// Mutable active conversation accessor guarded by the same invariant.
    pub fn active_conversation_mut(&mut self) -> &mut Conversation {
        self.conversations
            .get_mut(self.active_conv)
            .expect("active_conv out of bounds; invariant broken (no active conversation at index)")
    }

    /// ID of the active conversation.
    pub fn active_conversation_id(&self) -> &str {
        &self.active_conversation().id
    }

    /// Is the active conversation currently streaming?
    pub fn active_conv_streaming(&self) -> bool {
        self.active_conversation().is_streaming
    }

    /// Is the active conversation waiting for a permission decision?
    pub fn active_conv_pending_permission(&self) -> bool {
        self.active_conversation().pending_permission.is_some()
    }

    /// Store a pending permission request on the named conversation.
    pub fn set_pending_permission(&mut self, conv_id: &str, perm: PendingPermission) {
        if let Some(idx) = self.find_conv_idx(conv_id) {
            self.conversations[idx].pending_permission = Some(perm);
        }
    }

    // NOTE: there is deliberately no `respond_active_permission` helper.
    // Answering must also project `permission_closed`, so the only desktop
    // entry point is `app::remote::desktop_answer_active_permission`, which
    // wraps `respond_permission_at` and emits the frame (invariant 3: one
    // implementation of each mutation's semantics).

    /// Conversation index holding the pending permission with `request_id`.
    pub fn find_permission_conv(&self, request_id: &str) -> Option<usize> {
        self.conversations.iter().position(|c| {
            c.pending_permission
                .as_ref()
                .is_some_and(|p| p.request_id == request_id)
        })
    }

    /// Answer the pending permission on conversation `idx` and clear it.
    ///
    /// `answers` is the remote path's per-question selected option indices
    /// (§5.3): validated against the parked `AskUserQuestion`, then the tool
    /// input is rebuilt through the SAME `answers_map` path the desktop
    /// uses — remote and desktop produce byte-identical `updated_input`.
    /// A permission decision can never carry a different tool input than
    /// the one displayed. `None` uses the ask state's current UI
    /// selections (desktop path).
    pub fn respond_permission_at(
        &mut self,
        idx: usize,
        allow: bool,
        answers: Option<&[Vec<u32>]>,
        deny_message: Option<&str>,
    ) -> Result<PermissionClosedInfo, PermissionAnswerError> {
        let Some(conv) = self.conversations.get_mut(idx) else {
            return Err(PermissionAnswerError::NoPending);
        };
        let Some(perm) = conv.pending_permission.as_mut() else {
            return Err(PermissionAnswerError::NoPending);
        };

        // §5.3 validation happens BEFORE the request is consumed, so an
        // invalid remote answer leaves the request parked for a retry.
        if allow {
            match (perm.ask.as_mut(), answers) {
                (None, Some(_)) => {
                    return Err(PermissionAnswerError::Invalid(
                        "answers are only valid for AskUserQuestion permissions".into(),
                    ));
                }
                (Some(ask), Some(answers)) => {
                    if answers.len() != ask.questions.len() {
                        return Err(PermissionAnswerError::Invalid(format!(
                            "expected {} answer group(s), got {}",
                            ask.questions.len(),
                            answers.len()
                        )));
                    }
                    for (qi, (question, selection)) in
                        ask.questions.iter().zip(answers.iter()).enumerate()
                    {
                        if selection
                            .iter()
                            .any(|&i| i as usize >= question.options.len())
                        {
                            return Err(PermissionAnswerError::Invalid(format!(
                                "answer index out of range for question {}",
                                qi + 1
                            )));
                        }
                        if !question.multi_select && selection.len() != 1 {
                            return Err(PermissionAnswerError::Invalid(format!(
                                "question {} requires exactly one selection",
                                qi + 1
                            )));
                        }
                        if selection.is_empty() {
                            return Err(PermissionAnswerError::Invalid(format!(
                                "question {} has no selection",
                                qi + 1
                            )));
                        }
                    }
                    for (sel, provided) in ask.selected.iter_mut().zip(answers.iter()) {
                        *sel = provided.iter().map(|&i| i as usize).collect();
                    }
                }
                (Some(ask), None) => {
                    if !ask.all_answered() {
                        return Err(PermissionAnswerError::Invalid(
                            "not every question has a selection".into(),
                        ));
                    }
                }
                (None, None) => {}
            }
        }

        let perm = conv
            .pending_permission
            .take()
            .expect("pending permission checked above");
        let decision = if allow {
            if let Some(ask) = perm.ask.as_ref() {
                let mut updated = perm.input.clone();
                if let Some(obj) = updated.as_object_mut() {
                    obj.insert(
                        "answers".into(),
                        serde_json::Value::Object(ask.answers_map()),
                    );
                }
                gaviero_core::observer::PermissionDecision::Allow {
                    updated_input: Some(updated),
                }
            } else {
                gaviero_core::observer::PermissionDecision::Allow {
                    updated_input: Some(perm.input.clone()),
                }
            }
        } else {
            gaviero_core::observer::PermissionDecision::deny_with_message(
                deny_message.unwrap_or("Denied by user"),
            )
        };
        let _ = perm.respond.send(decision);
        Ok(PermissionClosedInfo {
            conv_id: conv.id.clone(),
            request_id: perm.request_id,
            allowed: allow,
        })
    }

    /// Whether every `AskUserQuestion` question has a selection — the
    /// desktop Enter gate before answering through the shared reducer.
    pub fn active_ask_is_answered(&self) -> bool {
        self.active_conversation()
            .pending_permission
            .as_ref()
            .and_then(|p| p.ask.as_ref())
            .map(|a| a.all_answered())
            .unwrap_or(false)
    }

    /// Toggle an option on the focused AskUserQuestion (1-based digit).
    pub fn ask_toggle_option(&mut self, digit: u8) -> bool {
        let conv = self.active_conversation_mut();
        let Some(perm) = conv.pending_permission.as_mut() else {
            return false;
        };
        let Some(ask) = perm.ask.as_mut() else {
            return false;
        };
        if digit == 0 {
            return false;
        }
        ask.toggle_option((digit - 1) as usize);
        true
    }

    pub fn ask_next_question(&mut self) {
        let Some(perm) = self.active_conversation_mut().pending_permission.as_mut() else {
            return;
        };
        let Some(ask) = perm.ask.as_mut() else {
            return;
        };
        if ask.focus_q + 1 >= ask.questions.len() {
            return;
        }
        ask.focus_q += 1;
        perm.scroll = 0;
    }

    pub fn ask_prev_question(&mut self) {
        let Some(perm) = self.active_conversation_mut().pending_permission.as_mut() else {
            return;
        };
        let Some(ask) = perm.ask.as_mut() else {
            return;
        };
        if ask.focus_q == 0 {
            return;
        }
        ask.focus_q -= 1;
        perm.scroll = 0;
    }

    /// Scroll the pending request overlay by `delta` rows (PgUp/PgDn).
    /// `width` is the overlay's column count — the render pass clamps again
    /// against the rows it can actually paint.
    pub fn scroll_pending_permission(&mut self, delta: isize, width: usize) {
        let Some(perm) = self.active_conversation_mut().pending_permission.as_mut() else {
            return;
        };
        let max = perm.overlay_rows(width).len().saturating_sub(1);
        perm.scroll = perm.scroll.saturating_add_signed(delta).min(max);
    }

    /// Rows the pending request overlay wants: header + wrapped body + hints.
    /// `None` when no request is pending.
    fn pending_overlay_height(&self, width: u16) -> Option<u16> {
        let perm = self.conversations[self.active_conv]
            .pending_permission
            .as_ref()?;
        let body = u16::try_from(perm.overlay_rows(width as usize).len()).unwrap_or(u16::MAX);
        Some(body.saturating_add(2))
    }

    /// Toggle the one-shot auto-approve flag for the next prompt (Alt+Y).
    pub fn toggle_auto_approve(&mut self) {
        self.auto_approve_next = !self.auto_approve_next;
    }

    /// Whether auto-approve is effective (persistent conversation flag OR one-shot).
    pub fn effective_auto_approve(&self) -> bool {
        self.active_conversation().auto_approve || self.auto_approve_next
    }

    /// Get the effective model for the active conversation.
    pub fn effective_model(&self) -> &str {
        self.active_conversation()
            .model_override
            .as_deref()
            .unwrap_or(&self.agent_settings.model)
    }

    /// Get the effective effort level for the active conversation.
    pub fn effective_effort(&self) -> &str {
        self.active_conversation()
            .effort_override
            .as_deref()
            .unwrap_or(&self.agent_settings.effort)
    }

    pub fn effective_bootstrap_mode(&self) -> gaviero_core::context_planner::BootstrapMode {
        self.active_conversation()
            .context_mode_override
            .unwrap_or(self.agent_settings.bootstrap_mode)
    }

    // ── Indexed effective-setting accessors (shared reducers, §2.2) ────

    pub fn effective_model_at(&self, idx: usize) -> &str {
        self.conversations[idx]
            .model_override
            .as_deref()
            .unwrap_or(&self.agent_settings.model)
    }

    pub fn effective_effort_at(&self, idx: usize) -> &str {
        self.conversations[idx]
            .effort_override
            .as_deref()
            .unwrap_or(&self.agent_settings.effort)
    }

    pub fn effective_write_namespace_at(&self, idx: usize) -> &str {
        self.conversations[idx]
            .namespace_override
            .as_deref()
            .unwrap_or(&self.agent_settings.write_namespace)
    }

    pub fn effective_bootstrap_mode_at(
        &self,
        idx: usize,
    ) -> gaviero_core::context_planner::BootstrapMode {
        self.conversations[idx]
            .context_mode_override
            .unwrap_or(self.agent_settings.bootstrap_mode)
    }

    /// Budget-only bootstrap estimate for slash commands that lack workspace cache access.
    pub fn fallback_bootstrap_estimate_context(
        &self,
    ) -> gaviero_core::context_planner::BootstrapEstimateContext {
        let conv = self.active_conversation();
        gaviero_core::context_planner::BootstrapEstimateContext {
            budgets: gaviero_core::context_planner::BootstrapBudgets {
                topology: 600,
                outline: self.agent_settings.graph_budget_tokens,
                anchor: self.agent_settings.anchor_budget_tokens,
                memory: 1_000,
                impact: self.agent_settings.graph_budget_tokens.min(4_000),
                impact_summary: 150,
            },
            hints: gaviero_core::context_planner::BootstrapEstimateHints {
                memory_tokens: if conv.last_memory_injection_tokens > 0 {
                    Some(conv.last_memory_injection_tokens)
                } else {
                    None
                },
                ..Default::default()
            },
        }
    }

    fn format_bootstrap_layers(arms: gaviero_core::context_planner::BootstrapArms) -> String {
        let mut parts = Vec::new();
        if arms.topology {
            parts.push("topology");
        }
        if arms.outline {
            parts.push("outline");
        }
        if arms.memory {
            parts.push("memory");
        }
        if arms.impact {
            parts.push("impact");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join(", ")
        }
    }

    fn conversation_is_first_turn(conv: &Conversation) -> bool {
        conv.session_ledger
            .as_ref()
            .map(|l| l.is_first_turn())
            .unwrap_or(true)
    }

    fn has_pending_bootstrap_override(conv: &Conversation) -> bool {
        conv.lite_next
            || conv.no_inject_next
            || (conv.inject_arms_next.explicit && conv.inject_arms_next.any_layer())
    }

    /// Bootstrap arms that will apply on the next dispatched prompt.
    pub fn resolve_next_bootstrap_arms(&self) -> gaviero_core::context_planner::BootstrapArms {
        let conv = self.active_conversation();
        let one_shot = if conv.lite_next {
            Some(gaviero_core::context_planner::BootstrapOneShot::Lite)
        } else if conv.no_inject_next {
            Some(gaviero_core::context_planner::BootstrapOneShot::NoInject)
        } else {
            None
        };
        gaviero_core::context_planner::resolve_chat_bootstrap_arms(
            self.effective_bootstrap_mode(),
            Self::conversation_is_first_turn(conv),
            one_shot,
            conv.inject_arms_next,
        )
    }

    fn projected_bootstrap_tokens(
        &self,
        ctx: &gaviero_core::context_planner::BootstrapEstimateContext,
    ) -> usize {
        let conv = self.active_conversation();
        let next_arms = self.resolve_next_bootstrap_arms();
        if next_arms == conv.last_bootstrap_arms && conv.last_bootstrap_tokens > 0 {
            return conv.last_bootstrap_tokens;
        }
        let projected = gaviero_core::context_planner::estimate_bootstrap_tokens(
            next_arms,
            &ctx.budgets,
            &ctx.hints,
        );
        // Between dispatch and provider `TurnTokenUsage`, `is_first_turn` may
        // already be false while prefix is still unknown — don't drop the
        // measured bootstrap from the composite bar in that window.
        if projected == 0
            && conv.last_bootstrap_tokens > 0
            && conv.last_bootstrap_arms.any_layer()
            && (conv.is_streaming || conv.last_token_usage.is_none())
        {
            return conv.last_bootstrap_tokens;
        }
        projected
    }

    fn pending_bootstrap_summary(&self) -> String {
        let conv = self.active_conversation();
        let mut lines = Vec::new();
        if conv.lite_next {
            lines.push("armed: /lite (topology only)".to_string());
        }
        if conv.no_inject_next {
            lines.push("armed: /no-inject (suppress all)".to_string());
        }
        if conv.inject_arms_next.explicit && conv.inject_arms_next.any_layer() {
            lines.push(format!(
                "armed: /inject ({})",
                Self::format_bootstrap_layers(conv.inject_arms_next)
            ));
        }
        if lines.is_empty() {
            "armed: none".to_string()
        } else {
            lines.join("; ")
        }
    }

    /// Per-layer projected bootstrap token ceilings for the next send.
    fn format_bootstrap_layer_breakdown(
        arms: gaviero_core::context_planner::BootstrapArms,
        budgets: &gaviero_core::context_planner::BootstrapBudgets,
        hints: &gaviero_core::context_planner::BootstrapEstimateHints,
    ) -> String {
        if !arms.any_layer() {
            return "  (none)".to_string();
        }

        let mut lines = Vec::new();
        if arms.topology {
            let projected = hints
                .topology_chars
                .map(|c| c.div_ceil(4).min(budgets.topology))
                .unwrap_or(budgets.topology);
            lines.push(format!(
                "  topology: ≈{} tok (ceiling {})",
                projected, budgets.topology
            ));
        }
        if arms.outline {
            // PUSH→PULL Phase 1: the default first turn uses the thin anchor;
            // an explicit /inject outline|all uses the full push. Mirror
            // estimate_bootstrap_tokens so the breakdown total stays consistent.
            let (projected, ceiling) = if arms.explicit {
                (
                    hints.outline_tokens.unwrap_or(budgets.outline),
                    budgets.outline,
                )
            } else {
                (budgets.anchor, budgets.anchor)
            };
            lines.push(format!(
                "  outline: ≈{} tok (ceiling {})",
                projected, ceiling
            ));
        }
        if arms.memory {
            let projected = hints.memory_tokens.unwrap_or(budgets.memory);
            lines.push(format!(
                "  memory: ≈{} tok (ceiling {})",
                projected, budgets.memory
            ));
        }
        if arms.impact {
            let projected = hints
                .impact_chars
                .map(|c| c.div_ceil(4).min(budgets.impact))
                .unwrap_or(budgets.impact);
            lines.push(format!(
                "  impact: ≈{} tok (ceiling {})",
                projected, budgets.impact
            ));
        }
        let total = gaviero_core::context_planner::estimate_bootstrap_tokens(arms, budgets, hints);
        lines.push(format!("  total bootstrap: ≈{} tok", total));
        lines.join("\n")
    }

    /// User-facing summary after `/reset`: what the next first turn will inject.
    fn reset_post_bootstrap_message(
        &self,
        estimate_ctx: &gaviero_core::context_planner::BootstrapEstimateContext,
    ) -> String {
        let arms = self.resolve_next_bootstrap_arms();
        let breakdown = Self::format_bootstrap_layer_breakdown(
            arms,
            &estimate_ctx.budgets,
            &estimate_ctx.hints,
        );
        let bootstrap_total = gaviero_core::context_planner::estimate_bootstrap_tokens(
            arms,
            &estimate_ctx.budgets,
            &estimate_ctx.hints,
        );
        let model = self.effective_model();
        let hidden = hidden_provider_overhead_tokens(&model);
        let composite = bootstrap_total.saturating_add(hidden);

        let mut msg = format!(
            "Context cleared. Chat history stays in the panel but won't be re-sent.\n\
             Next send is a first turn — server session starts fresh.\n\n\
             Projected bootstrap injection:\n{breakdown}"
        );

        if arms.any_layer() && (arms.outline || arms.memory || arms.impact) {
            msg.push_str(
                "\n\nTip: /lite drops outline + memory + impact (keeps topology only) \
                 for the next send. /lite survives /reset if armed first.",
            );
        }
        if model.starts_with("cursor:") {
            msg.push_str(&format!(
                "\n\nCursor composite estimate: ~{composite} tok \
                 (~{bootstrap_total} bootstrap + ~{hidden} system/tools allowance)."
            ));
        }
        msg
    }

    /// Process slash commands in input. Returns true if a command was handled.
    ///
    /// A leading `//` (double slash) is the explicit "send raw to agent"
    /// marker — used for Claude Code skills like `//init` or custom commands
    /// in `~/.claude/commands/`. The marker is stripped before forwarding
    /// so the agent sees the canonical single-slash form.
    ///
    /// Single-slash unknown commands still produce a local "Unknown command"
    /// error so typos don't silently leak to the agent.
    pub fn process_slash_command(&mut self) -> bool {
        let input = self.text_input.text.trim().to_string();
        if !input.starts_with('/') {
            return false;
        }

        // Explicit pass-through: `//foo bar` → forward `/foo bar` verbatim.
        if let Some(rest) = input.strip_prefix("//") {
            let forwarded = format!("/{}", rest);
            self.text_input.text = forwarded;
            self.text_input.cursor = self.text_input.char_count();
            return false;
        }

        // Clear the draft up-front (every handled arm used to do this at
        // its end); `/rename`'s bare form re-fills it via `start_rename`.
        self.text_input.text.clear();
        self.text_input.cursor = 0;
        let idx = self.active_conv;
        self.apply_slash_line(idx, &input, SlashOrigin::Desktop)
    }

    /// Shared slash reducer (Plan A §2.2): one implementation of each
    /// command's semantics, applied to the conversation at `idx`. Desktop
    /// keyboard input and `Event::RemoteCommand` both land here. `origin`
    /// exists only for policy and desktop-only UI affordances (interactive
    /// rename) — never to fork a mutation.
    pub fn apply_slash_line(&mut self, idx: usize, input: &str, origin: SlashOrigin) -> bool {
        if idx >= self.conversations.len() {
            return false;
        }
        let parts: Vec<&str> = input.splitn(2, ' ').collect();
        let cmd = parts[0];
        let arg = parts.get(1).map(|s| s.trim()).unwrap_or("");

        // /rename is a pure UI op (mutates the tab title) and must never
        // appear in the transcript — otherwise add_user_message would
        // auto-title a fresh conversation to the slash command text and
        // mangle the rename target.
        if cmd != "/rename" {
            self.add_user_message_at(idx, input);
        }

        match cmd {
            "/model" => {
                if arg.is_empty() {
                    let current = self.effective_model_at(idx).to_string();
                    let options = self.model_options().to_vec();
                    let list = if options.is_empty() {
                        "claude:fable, claude:sonnet, claude:opus, claude:haiku, \
                         claude:opusplan, claude:sonnet[1m], claude:opus[1m], \
                         codex:gpt-5.6-sol, codex:gpt-5.6-luna, ollama:qwen2.5-coder:7b"
                            .to_string()
                    } else {
                        options.join(", ")
                    };
                    self.add_system_message_at(
                        idx,
                        &format!(
                            "Current model: {}\nAvailable: {}\nUsage: /model <provider:model>\n\
                         Specs require a provider prefix: `claude:`, `codex:`, `cursor:`, \
                         `deepseek:`, `ollama:`, or `local:`.",
                            current, list
                        ),
                    );
                } else {
                    let model = normalize_model_spec(arg);
                    if let Err(err) =
                        gaviero_core::swarm::backend::shared::validate_model_spec(&model)
                    {
                        self.add_system_message_at(idx, &format!("Invalid model spec: {err:#}"));
                    } else {
                        self.conversations[idx].model_override = Some(model.clone());
                        self.conversations[idx].bump_revision();
                        self.add_system_message_at(idx, &format!("Model set to: {}", model));
                    }
                }
                true
            }
            "/thinking" | "/effort" => {
                if arg.is_empty() {
                    let current = self.effective_effort_at(idx);
                    self.add_system_message_at(
                        idx,
                        &format!(
                            "Effort level: {}.\n\
                         Usage: /effort <off|auto|low|medium|high|xhigh|max|ultra>\n\
                         Applies to Claude and Codex sessions (Ollama ignores it).\n\
                         `xhigh` applies on Opus 4.7 (falls back to `high` on older \
                         Claude models). On Codex: `xhigh`/`max`/`ultra` forward for \
                         GPT-5.6 Sol/Terra; Luna caps at `max`; older models at `xhigh`.\n\
                         `off`/`auto` omit the reasoning hint entirely.\n\
                         `max`/`ultra` are session-only on Claude.",
                            current
                        ),
                    );
                } else {
                    let level = match arg {
                        "off" | "0" | "none" => "off",
                        "auto" => "auto",
                        "low" | "l" => "low",
                        "medium" | "med" | "m" => "medium",
                        "high" | "h" => "high",
                        "xhigh" | "xh" => "xhigh",
                        "max" => "max",
                        "ultra" | "u" => "ultra",
                        _ => {
                            self.add_system_message_at(
                                idx,
                                "Invalid effort level. Use: off, auto, low, medium, high, xhigh, max, ultra",
                            );
                            return true;
                        }
                    };
                    self.conversations[idx].effort_override = Some(level.to_string());
                    self.conversations[idx].bump_revision();
                    self.add_system_message_at(idx, &format!("Effort level set to: {}", level));
                }
                true
            }
            "/compact" => {
                let keep = if arg.is_empty() {
                    theme::DEFAULT_COMPACT_KEEP
                } else {
                    arg.parse::<usize>().unwrap_or(theme::DEFAULT_COMPACT_KEEP)
                };
                let conv = &mut self.conversations[idx];
                let total = conv.messages.len();
                if total <= keep {
                    self.add_system_message_at(
                        idx,
                        &format!("Nothing to compact ({} messages, keeping {})", total, keep),
                    );
                } else {
                    let removed = total - keep;
                    // Summarize removed messages into a single system note.
                    // The summary is a NEW message with a new seq (§2.6);
                    // kept messages retain theirs, so remote cursors into
                    // the removed range resolve to "no such range".
                    let summary = format!("[{} earlier messages compacted]", removed);
                    let kept: Vec<ChatMessage> = conv.messages.split_off(total - keep);
                    conv.messages.clear();
                    let summary_seq = conv.next_message_seq;
                    conv.next_message_seq += 1;
                    conv.messages.push(ChatMessage {
                        seq: summary_seq,
                        role: ChatRole::System,
                        content: summary,
                        tool_calls: Vec::new(),
                    });
                    conv.messages.extend(kept);

                    // The context estimate reads the ACTIVE conversation's
                    // draft/usage state; skip the percentage for a
                    // background target rather than report wrong numbers.
                    if idx == self.active_conv {
                        let (_tokens, pct) =
                            self.estimate_context(&self.fallback_bootstrap_estimate_context());
                        self.add_system_message_at(
                            idx,
                            &format!(
                                "Compacted: removed {} messages, kept {}. Context: ~{}% of limit",
                                removed, keep, pct
                            ),
                        );
                    } else {
                        self.add_system_message_at(
                            idx,
                            &format!("Compacted: removed {} messages, kept {}.", removed, keep),
                        );
                    }
                }
                true
            }
            "/context" => {
                if let Some(mode_arg) = arg.strip_prefix("mode") {
                    let mode_str = mode_arg.trim();
                    let conv = &mut self.conversations[idx];
                    if mode_str.is_empty() {
                        self.add_system_message_at(
                            idx,
                            &format!(
                                "Bootstrap mode: {} (workspace default: {})\n\
                             Usage: /context mode auto|minimal|manual|none\n\
                             /context mode reset — clear per-conversation override",
                                self.effective_bootstrap_mode_at(idx).as_str(),
                                self.agent_settings.bootstrap_mode.as_str(),
                            ),
                        );
                    } else if mode_str == "reset" {
                        conv.context_mode_override = None;
                        self.add_system_message_at(
                            idx,
                            &format!(
                                "Bootstrap mode reset to workspace default: {}",
                                self.agent_settings.bootstrap_mode.as_str()
                            ),
                        );
                    } else if let Some(mode) =
                        gaviero_core::context_planner::BootstrapMode::parse(mode_str)
                    {
                        conv.context_mode_override = Some(mode);
                        self.add_system_message_at(
                            idx,
                            &format!(
                                "Bootstrap mode set to: {} (this conversation)",
                                mode.as_str()
                            ),
                        );
                    } else {
                        self.add_system_message_at(
                            idx,
                            "Invalid mode. Use: auto, minimal, manual, none, or reset",
                        );
                    }
                    return true;
                }

                if idx != self.active_conv {
                    // The rich report below reads active-conversation draft
                    // and estimate state; give a background target an
                    // honest per-conversation subset instead.
                    let conv = &self.conversations[idx];
                    let mut msg = format!(
                        "Model: {} — context limit {} tokens",
                        self.effective_model_at(idx),
                        self.context_limit_tokens_for(self.effective_model_at(idx)),
                    );
                    if let Some(u) = &conv.last_token_usage {
                        msg.push_str(&format!(
                            "\nProvider usage (last turn): prefix {} | output {}",
                            u.prefix_tokens(),
                            u.output_tokens
                        ));
                    }
                    self.add_system_message_at(idx, &msg);
                    return true;
                }

                let limit = self.context_limit_tokens();
                let estimate_ctx = self.fallback_bootstrap_estimate_context();
                let pressure = self.context_pressure(&estimate_ctx);
                let (input_words, output_words) = self.count_transcript_words();
                let source_label = match pressure.source {
                    ContextBarSource::ProviderPrefix => "provider prefix (authoritative)",
                    ContextBarSource::CompositeEstimate => "composite estimate",
                };
                let mode = self.effective_bootstrap_mode();
                let pending = self.pending_bootstrap_summary();
                let next_arms = self.resolve_next_bootstrap_arms();
                let layer_breakdown = Self::format_bootstrap_layer_breakdown(
                    next_arms,
                    &estimate_ctx.budgets,
                    &estimate_ctx.hints,
                );
                let conv = self.active_conversation();
                let mut msg = format!(
                    "Status bar: {} tokens — {} (~{}% of {} limit)\n\
                     Composite parts:\n  transcript: {} tok (visible chat × 1.3)\n  \
                     bootstrap: {} tok (projected next injection)\n  \
                     hidden overhead: {} tok (provider system/tools allowance)\n\
                     Transcript words: {} input | {} output\n\n\
                     Bootstrap policy:\n  mode: {} (workspace default: {})\n  {}\n\n\
                     Next-send bootstrap breakdown:\n{layer_breakdown}",
                    pressure.tokens,
                    source_label,
                    pressure.pct,
                    limit,
                    pressure.transcript_tokens,
                    pressure.bootstrap_tokens,
                    pressure.hidden_overhead_tokens,
                    input_words,
                    output_words,
                    mode.as_str(),
                    self.agent_settings.bootstrap_mode.as_str(),
                    pending,
                );
                if conv.last_bootstrap_tokens > 0 {
                    msg.push_str(&format!(
                        "\n\nLast measured bootstrap (prior send): {} tok ({})",
                        conv.last_bootstrap_tokens,
                        Self::format_bootstrap_layers(conv.last_bootstrap_arms),
                    ));
                }
                if self.effective_model().starts_with("codex:") {
                    msg.push_str(
                        "\n  codex exec replays history client-side; use /inject memory \
                         or /inject all on later turns when mode is manual.",
                    );
                }
                if let Some(u) = self
                    .conversations
                    .get(self.active_conv)
                    .and_then(|c| c.last_token_usage.as_ref())
                {
                    let prefix = u.prefix_tokens() as usize;
                    let total =
                        u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens;
                    let cache_hit_pct = if total > 0 {
                        (u.cache_read_input_tokens * 100 / total) as usize
                    } else {
                        0
                    };
                    msg.push_str(&format!(
                        "\n\nProvider usage (last turn):\n  prefix: {} | output: {}\n  \
                         input: {} | cache_creation: {} | cache_read: {} | cache hit: {}%",
                        prefix,
                        u.output_tokens,
                        u.input_tokens,
                        u.cache_creation_input_tokens,
                        u.cache_read_input_tokens,
                        cache_hit_pct,
                    ));
                }
                self.add_system_message(&msg);
                true
            }
            "/inject" => {
                let conv = &mut self.conversations[idx];
                let layer = arg.to_ascii_lowercase();
                if layer.is_empty() {
                    self.add_system_message_at(
                        idx,
                        "Inject bootstrap layers on the next prompt:\n\
                         /inject memory    — <project_memory>\n\
                         /inject outline   — <repo_outline> (alias: graph)\n\
                         /inject topology  — <repo_topology>\n\
                         /inject impact    — buffer-seeded impact slice\n\
                         /inject all       — all layers (works on follow-up turns)\n\
                         Layers merge if you run multiple /inject commands before sending.\n\
                         /no-inject        — suppress all bootstrap on next prompt",
                    );
                } else if layer == "all" {
                    conv.inject_arms_next = gaviero_core::context_planner::BootstrapArms {
                        explicit: true,
                        ..gaviero_core::context_planner::BootstrapArms::all()
                    };
                    self.add_system_message_at(
                        idx,
                        "Inject all: ARMED for next prompt (topology, outline, memory, impact).",
                    );
                } else {
                    let known = match layer.as_str() {
                        "memory" => {
                            conv.inject_arms_next.memory = true;
                            true
                        }
                        "outline" | "graph" => {
                            conv.inject_arms_next.outline = true;
                            true
                        }
                        "topology" => {
                            conv.inject_arms_next.topology = true;
                            true
                        }
                        "impact" => {
                            conv.inject_arms_next.impact = true;
                            true
                        }
                        _ => false,
                    };
                    if known {
                        let layers = {
                            conv.inject_arms_next.explicit = true;
                            Self::format_bootstrap_layers(conv.inject_arms_next)
                        };
                        self.add_system_message_at(
                            idx,
                            &format!("Inject armed for next prompt: {layers}"),
                        );
                    } else {
                        self.add_system_message_at(
                            idx,
                            "Unknown layer. Use: memory, outline, topology, impact, or all",
                        );
                    }
                }
                true
            }
            "/no-inject" => {
                let conv = &mut self.conversations[idx];
                conv.no_inject_next = !conv.no_inject_next;
                let msg = if conv.no_inject_next {
                    "No-inject: ARMED for next prompt — suppresses all bootstrap layers \
                     (including topology). Run /no-inject again to cancel."
                } else {
                    "No-inject: cleared."
                };
                self.add_system_message_at(idx, msg);
                true
            }
            "/reset" | "/clear" => {
                self.reset_conversation_at(idx);
                true
            }
            "/rename" => {
                if arg.is_empty() {
                    match origin {
                        SlashOrigin::Desktop => {
                            // Bare /rename → interactive rename (same as F2).
                            self.start_rename();
                        }
                        SlashOrigin::Remote => {
                            // Interactive rename is a desktop focus
                            // affordance; remote must not open it.
                            self.add_system_message_at(idx, "Usage: /rename <new title>");
                        }
                    }
                } else {
                    let old = self.conversations[idx].title.clone();
                    self.conversations[idx].title = arg.to_string();
                    self.conversations[idx].bump_revision();
                    self.add_system_message_at(
                        idx,
                        &format!("Renamed conversation: \"{}\" → \"{}\"", old, arg),
                    );
                }
                true
            }
            "/namespace" | "/ns" => {
                if arg.is_empty() {
                    let write = self.effective_write_namespace_at(idx).to_string();
                    let mut read = vec![write.clone()];
                    for ns in &self.agent_settings.read_namespaces {
                        if !read.contains(ns) {
                            read.push(ns.clone());
                        }
                    }
                    let read_str = read
                        .iter()
                        .map(|ns| {
                            if *ns == write {
                                format!("{} (write)", ns)
                            } else {
                                ns.clone()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.add_system_message_at(
                        idx,
                        &format!(
                            "Write namespace: {}\nRead namespaces: [{}]",
                            write, read_str
                        ),
                    );
                } else {
                    self.conversations[idx].namespace_override = Some(arg.to_string());
                    self.conversations[idx].bump_revision();
                    self.add_system_message_at(
                        idx,
                        &format!("Write namespace set to: {} (for this conversation)", arg),
                    );
                }
                true
            }
            "/autoapprove" | "/yolo" => {
                let conv = &mut self.conversations[idx];
                conv.auto_approve = !conv.auto_approve;
                conv.bump_revision();
                let state = if conv.auto_approve { "ON" } else { "OFF" };
                self.add_system_message_at(
                    idx,
                    &format!("Auto-approve: {} for this conversation", state),
                );
                true
            }
            "/workspace" | "/ws" => {
                // Per-turn one-shot. Mirrors `auto_approve_next`: toggling
                // the flag arms the next dispatched turn to use workspace-
                // wide planner scope, then `send_chat_message` clears it.
                // Calling `/workspace` again before sending toggles back
                // off, so the user can change their mind without a separate
                // command.
                let conv = &mut self.conversations[idx];
                conv.workspace_wide_next = !conv.workspace_wide_next;
                let msg = if conv.workspace_wide_next {
                    "Workspace-wide scope: ARMED for next prompt. \
                     The planner will ignore the focused buffer's folder and use \
                     the workspace primary scope. (Run /workspace again to cancel.)"
                } else {
                    "Workspace-wide scope: cleared. Next prompt will use the \
                     focused folder default again."
                };
                self.add_system_message_at(idx, msg);
                true
            }
            "/lite" | "/minimal" => {
                // Per-turn one-shot. Same toggle pattern as `/workspace`:
                // arms the next dispatched turn to skip every bootstrap
                // context block (graph, memory, impact). Self-clears on
                // dispatch. Use this to shrink token spend; Cursor first
                // turns that exceed the OS argv budget now spill to a
                // tempfile instead of failing (see cursor_argv_limit).
                let conv = &mut self.conversations[idx];
                conv.lite_next = !conv.lite_next;
                let msg = if conv.lite_next {
                    "Minimal context: ARMED for next prompt. \
                     Skips ranked graph (<repo_outline>), memory \
                     (<project_memory>), and impact on the first turn; \
                     keeps shallow folder map (<repo_topology>). \
                     (Run /lite again to cancel.)"
                } else {
                    "Minimal context: cleared. Next prompt will include \
                     full bootstrap context normally."
                };
                self.add_system_message_at(idx, msg);
                true
            }
            "/help" => {
                self.add_system_message_at(idx,
                    "Available commands:\n\n\
                     Conversation:\n\
                     /model <provider:model>  — Set model. Examples: claude:fable, claude:sonnet, claude:opus, claude:haiku, claude:opusplan, claude:sonnet[1m], claude:opus[1m], codex:<model>, ollama:<model>\n\
                     /effort <level>          — Set effort/reasoning level for Claude + Codex (off, auto, low, medium, high, xhigh, max, ultra). Alias: /thinking\n\
                     /namespace <name>        — Set memory namespace (or show current). Alias: /ns\n\
                     /autoapprove             — Toggle auto-approve for this conversation. Alias: /yolo\n\
                     /workspace               — Arm workspace-wide planner scope for the next prompt only (multi-folder workspaces). Default scope follows the active buffer's folder; use this when the prompt genuinely spans folders. Alias: /ws\n\
                     /lite                    — Arm minimal-context for the next prompt: skips <repo_outline>, <project_memory>, and impact; keeps <repo_topology>. Alias: /minimal\n\
                     /inject <layer|all>      — Arm bootstrap layers for the next prompt (memory, outline, topology, impact, all)\n\
                     /no-inject               — Arm zero bootstrap on the next prompt\n\
                     /context                 — Show context usage + bootstrap policy\n\
                     /context mode <mode>     — Set bootstrap mode for this conversation (auto|minimal|manual|none)\n\
                     /rename [new title]      — Rename the active conversation tab (bare form starts interactive rename, same as F2)\n\
                     /reset                   — Clear agent context (keeps visible chat history). Alias: /clear\n\
                     /compact [N]             — Keep last N messages (default 6), discard older\n\n\
                     Files & scripts:\n\
                     /attach <path>           — Attach a file (text or image)\n\
                     /attach                  — List current attachments\n\
                     /detach <name|all>       — Remove attachment(s) (Tab completes names)\n\
                     /run <path>              — Execute a .gaviero DSL script (supports `client { effort ... extra { ... } }` and top-level `tier <name> <client>` aliases)\n\n\
                     Swarm:\n\
                     /swarm <task>            — Plan and execute a multi-agent swarm\n\
                     /cswarm <task>           — Coordinated swarm (provider-aware coordinator planning)\n\
                     /undo-swarm              — Revert all changes from the last /cswarm run\n\n\
                     Memory:\n\
                     /remember <text>         — Store a memory at the default scope\n\
                     /remember-here <text>    — Store at run scope (dies with session)\n\
                     /remember-module <text>  — Store at module scope (current file's dir)\n\
                     /remember-workspace <text>\n                              — Store at workspace scope\n\
                     /remember-global <text>  — Store at global scope\n\
                     /consolidate-session     — Run end-of-session consolidator over the active conversation\n\
                     /consolidate history [n] — List recent consolidator runs and whether they were rolled back\n\
                     /consolidate rollback <run_id>\n                              — Undo a consolidator run's memory changes\n\
                     /sleep [--dry-run]       — Trigger the sleeptime maintenance pass\n\
                     /reembed                 — Re-embed every memory under the configured embedder (takes a backup first)\n\
                     /forget <query>          — Soft-delete records matching a fuzzy query (never history)\n\
                     /forget-scope <path>     — Soft-delete every row at a scope (e.g. workspace, repo:<id>)\n\
                     /forget-type <type>      — Soft-delete by type (factual|procedural|decision|pattern|gotcha|...)\n\
                     /forget-source <source>  — Soft-delete by source (user_remember|llm_extracted|llm_consolidated|...)\n\
                     \u{00a0}\u{00a0}(append --dry-run to preview, --yes to confirm, --reason \"<text>\" to annotate the audit)\n\
                     /forget-history <id>     — Preview a history row\n\
                     /forget-history --confirm <id> [REDACT <reason>]\n                              — Redact a history row (one-way; tombstone replaces the transcript)\n\
                     /restore <deletion-id>   — Replay a soft-deleted row through the dedup pipeline\n\
                     /restore --since <N minutes|N hours|N days>\n                              — Replay every soft-deletion in the window\n\n\
                     Help:\n\
                     /help                    — Show this help\n\n\
                     Pass-through to agent:\n\
                     //<command>              — Forward `/<command>` verbatim to the agent\n\
                     \u{00a0}\u{00a0}(use this for Claude Code skills like `//init`, or commands\n\
                     \u{00a0}\u{00a0}defined in ~/.claude/commands/)\n\n\
                     Keyboard shortcuts:\n\
                     F2                       — Rename active conversation tab\n\
                     Ctrl+T                   — New conversation tab\n\
                     Ctrl+C                   — Cancel streaming / enter browse mode\n\
                     Ctrl+V                   — Paste text, or attach clipboard image\n\
                     Alt+V                    — Same (use if Ctrl+V is swallowed by the terminal)\n\
                     Alt+Enter                — Insert newline in input\n\
                     PageUp / PageDown        — Scroll prompt input when it overflows, else chat history\n\
                     Esc                      — Clear input / return to editor\n\n\
                     Use @filename to reference workspace files in your prompt.\n\
                     Use /attach to attach files from outside the workspace.",
                );
                true
            }
            _ => {
                self.add_system_message_at(
                    idx,
                    &format!(
                        "Unknown command: {}. Type /help for available commands. \
                     Prefix with `//` to send a slash command directly to the agent \
                     (e.g. `//init` for Claude Code skills).",
                        cmd
                    ),
                );
                true
            }
        }
    }

    /// Context-window pressure for the status bar and `/context`.
    ///
    /// ## Provider paths
    ///
    /// | Provider | Authoritative source | Notes |
    /// |----------|---------------------|-------|
    /// | `claude:` | `TokenUsage::prefix_tokens()` after each turn | Last API iteration's `input + cache_creation + cache_read` (not the billing sum). |
    /// | `cursor:` | `input_tokens` from `result` event | No prompt-cache breakdown today; `prefix_tokens()` == `input_tokens`. |
    /// | `codex:` | Composite only | `codex exec` emits no token counts; estimate replays transcript client-side. |
    /// | `ollama:` / `local:` | Composite only | Chat path does not surface `prompt_eval_count` yet. |
    ///
    /// ## Composite estimate (no authoritative prefix yet)
    ///
    /// `transcript` (visible chat × 1.3, plus draft input) + `projected bootstrap`
    /// (from `/lite` / `/inject` arms + cache hints) + `hidden overhead`
    /// (flat provider allowance for system prompt, tool schemas, auto-loaded docs).
    ///
    /// When a resumed session has `/inject` or `/lite` armed, projected bootstrap
    /// is added on top of the provider prefix (delta for the upcoming send).
    pub fn context_pressure(
        &self,
        estimate_ctx: &gaviero_core::context_planner::BootstrapEstimateContext,
    ) -> ContextPressure {
        let conv = &self.conversations[self.active_conv];
        let limit = self.context_limit_tokens();
        let transcript_tokens = self.transcript_token_estimate();
        let bootstrap_tokens = self.projected_bootstrap_tokens(estimate_ctx);
        let hidden_overhead_tokens = hidden_provider_overhead_tokens(self.effective_model());

        let composite = transcript_tokens
            .saturating_add(bootstrap_tokens)
            .saturating_add(hidden_overhead_tokens);

        let (tokens, source) = if let Some(u) = &conv.last_token_usage {
            let prefix = u.prefix_tokens() as usize;
            if prefix > 0 {
                if Self::has_pending_bootstrap_override(conv) && bootstrap_tokens > 0 {
                    // Resumed sessions omit armed bootstrap until the next send
                    // (e.g. codex `/inject memory` on a follow-up turn).
                    (
                        prefix.saturating_add(bootstrap_tokens),
                        ContextBarSource::CompositeEstimate,
                    )
                } else {
                    (prefix, ContextBarSource::ProviderPrefix)
                }
            } else {
                (composite, ContextBarSource::CompositeEstimate)
            }
        } else {
            (composite, ContextBarSource::CompositeEstimate)
        };

        let pct = if limit > 0 {
            (tokens * 100 / limit).min(100)
        } else {
            0
        };

        ContextPressure {
            tokens,
            pct,
            source,
            transcript_tokens,
            bootstrap_tokens,
            hidden_overhead_tokens,
        }
    }

    /// Back-compat tuple for call sites that only need tokens + percent.
    pub fn estimate_context(
        &self,
        estimate_ctx: &gaviero_core::context_planner::BootstrapEstimateContext,
    ) -> (usize, usize) {
        let p = self.context_pressure(estimate_ctx);
        (p.tokens, p.pct)
    }

    fn transcript_token_estimate(&self) -> usize {
        let (input_words, output_words) = self.count_transcript_words();
        let draft_words = if self.renaming {
            0
        } else {
            count_words(self.text_input.text.trim())
        };
        words_to_tokens(
            input_words
                .saturating_add(output_words)
                .saturating_add(draft_words),
        )
    }

    /// Word counts for the active conversation's visible transcript,
    /// split by role. User-role content counts as input; assistant-role
    /// content plus its tool-call payloads count as output. System
    /// messages (slash-command echoes, error banners) are panel chatter
    /// and aren't re-sent to the model, so they're excluded.
    ///
    /// After `/reset` (`TranscriptInlineMode::Suppress`) the transcript
    /// remains visible but isn't re-inlined into the next prompt — this
    /// mirrors that and returns `(0, 0)` so the indicator tracks what
    /// will actually be sent.
    pub fn count_transcript_words(&self) -> (usize, usize) {
        let conv = &self.conversations[self.active_conv];
        if conv.transcript_inline_mode == TranscriptInlineMode::Suppress {
            return (0, 0);
        }
        let mut input = 0usize;
        let mut output = 0usize;
        for msg in &conv.messages {
            match msg.role {
                ChatRole::User => input += count_words(&msg.content),
                ChatRole::Assistant => {
                    // Tool names are already inlined in `content` as `[tool]`
                    // markers — do not count `tool_calls` again.
                    output += count_words(&msg.content);
                }
                ChatRole::System => {}
            }
        }
        (input, output)
    }

    /// Context window size in tokens for the effective model.
    pub fn context_limit_tokens(&self) -> usize {
        self.context_limit_tokens_for(self.effective_model())
    }

    /// Context window size for an explicit model spec.
    pub fn context_limit_tokens_for(&self, model: &str) -> usize {
        // Trailing `[1m]` (e.g. `claude:sonnet[1m]`, `claude:claude-opus-4-7[1m]`)
        // selects the 1M-token extended context variant — see Claude Code
        // model-config docs. Strip the suffix before matching the base alias.
        if model.ends_with("[1m]") {
            return 1_000_000;
        }
        let provider = model.split_once(':').map(|(p, _)| p).unwrap_or("claude");
        match provider {
            "ollama" | "local" => 8_192,
            "claude" | "codex" | "cursor" => 200_000,
            _ => 200_000,
        }
    }

    pub fn add_system_message(&mut self, content: &str) {
        let idx = self.active_conv;
        self.add_system_message_at(idx, content);
    }

    /// System message on a specific conversation (remote reducers target
    /// non-active tabs).
    pub fn add_system_message_at(&mut self, idx: usize, content: &str) {
        let Some(conv) = self.conversations.get_mut(idx) else {
            return;
        };
        conv.push_message(ChatRole::System, content.to_string(), Vec::new());
        if idx == self.active_conv {
            self.scroll_to_bottom();
        }
    }

    // ── Active conversation helpers ─────────────────────────────

    fn messages(&self) -> &Vec<ChatMessage> {
        &self.active_conversation().messages
    }

    // NOTE: no `messages_mut`. Mutating the message list must go through
    // `Conversation::push_message` so every message gets its monotonic
    // `seq` (§2.6); a raw `&mut Vec<ChatMessage>` made it easy to push one
    // without an id.

    /// Create a new conversation and switch to it.
    /// Start renaming the active conversation. Puts current title into the input field.
    pub fn start_rename(&mut self) {
        let title = self.active_conversation().title.clone();
        self.text_input.text = title;
        self.text_input.cursor = self.text_input.char_count();
        self.renaming = true;
    }

    /// Confirm the rename — apply input as the new title.
    pub fn confirm_rename(&mut self) {
        let new_title = self.text_input.text.trim().to_string();
        if !new_title.is_empty() {
            self.active_conversation_mut().title = new_title;
        }
        self.text_input.text.clear();
        self.text_input.cursor = 0;
        self.renaming = false;
    }

    /// Cancel the rename — restore input field.
    pub fn cancel_rename(&mut self) {
        self.text_input.text.clear();
        self.text_input.cursor = 0;
        self.renaming = false;
    }

    pub fn new_conversation(&mut self) {
        let conv = Conversation {
            id: gaviero_core::session_state::new_conversation_id(),
            conv_revision: 1,
            next_message_seq: 1,
            title: "New Chat".to_string(),
            messages: Vec::new(),
            model_override: None,
            effort_override: None,
            namespace_override: None,
            is_streaming: false,
            streaming_status: String::new(),
            streaming_started_at: None,
            auto_approve: false,
            pending_permission: None,
            pending_turn_id: None,
            pending_module_path: None,
            pending_focused_folder: None,
            workspace_wide_next: false,
            lite_next: false,
            no_inject_next: false,
            inject_arms_next: gaviero_core::context_planner::BootstrapArms::none(),
            context_mode_override: None,
            claude_session_id: None,
            session_ledger: None,
            pending_persisted_ledger: None,
            transcript_inline_mode: TranscriptInlineMode::Auto,
            last_token_usage: None,
            last_turn_cost_usd: 0.0,
            last_bootstrap_tokens: 0,
            last_bootstrap_arms: gaviero_core::context_planner::BootstrapArms::none(),
            last_memory_injection_tokens: 0,
        };
        self.conversations.push(conv);
        self.active_conv = self.conversations.len() - 1;
        self.scroll_offset = 0;
        self.text_input.text.clear();
        self.text_input.cursor = 0;
    }

    /// Reset the agent context: drop the session + planner ledger so the next
    /// turn bootstraps fresh. Visible chat history, attachments, scroll, and
    /// input are preserved. Indexed because `/reset` can arrive from the
    /// phone for a conversation that is not the active tab (shared reducer,
    /// §2.2).
    pub fn reset_conversation_at(&mut self, idx: usize) {
        let Some(conv) = self.conversations.get_mut(idx) else {
            return;
        };
        conv.claude_session_id = None;
        conv.session_ledger = None;
        conv.pending_persisted_ledger = None;
        conv.pending_turn_id = None;
        conv.pending_module_path = None;
        conv.pending_focused_folder = None;
        // T1: the server-side session is being dropped — the stored usage
        // belongs to that session, not the next one. Clear it so the
        // indicator falls back to the char estimate until the first
        // post-reset turn reports fresh numbers.
        conv.last_token_usage = None;
        conv.last_bootstrap_tokens = 0;
        conv.last_bootstrap_arms = gaviero_core::context_planner::BootstrapArms::none();
        conv.last_memory_injection_tokens = 0;
        // NOTE: `lite_next` / `no_inject_next` / `inject_arms_next` are
        // deliberately NOT cleared here. They are forward-looking one-shot
        // arms for the *next* dispatch, orthogonal to dropping past session
        // state — exactly like `workspace_wide_next`, which reset already
        // leaves alone. Clearing them was a footgun: `/reset` re-arms a
        // fresh first turn whose full bootstrap (`<repo_outline>` + impact
        // + memory) is large, and `/lite` is the documented token-saving
        // hatch (see the `/lite` handler). Wiping `lite_next` on `/reset`
        // meant arming `/lite` then `/reset` silently disarmed that hatch.
        // Suppress the visible transcript on the next first-turn dispatch.
        // Bootstrap context (graph + memory) still flows; only the
        // re-inlining of prior user/assistant turns is skipped, matching
        // the user-facing meaning of "/reset". The SystemInit handler in
        // controller.rs flips this back to `Auto` once Claude opens the
        // fresh session, so subsequent /reset invocations behave the same.
        conv.transcript_inline_mode = TranscriptInlineMode::Suppress;
        conv.bump_revision();
        if idx == self.active_conv {
            self.text_input.text.clear();
            self.text_input.cursor = 0;
            let estimate_ctx = self.fallback_bootstrap_estimate_context();
            let msg = self.reset_post_bootstrap_message(&estimate_ctx);
            self.add_system_message(&msg);
        } else {
            // The bootstrap breakdown reads active-conversation estimate
            // state; a background reset gets the plain confirmation.
            self.add_system_message_at(idx, "Context reset — next turn bootstraps fresh.");
        }
    }

    /// Close the active conversation. If it's the last one, replace it with a fresh one.
    pub fn close_conversation(&mut self) {
        if self.conversations.len() <= 1 {
            self.conversations.clear();
            self.active_conv = 0;
            self.new_conversation();
            return;
        }
        self.conversations.remove(self.active_conv);
        if self.active_conv >= self.conversations.len() {
            self.active_conv = self.conversations.len() - 1;
        }
        self.scroll_offset = 0;
        self.scroll_pinned_to_bottom = true;
        self.text_input.text.clear();
        self.text_input.cursor = 0;
    }

    /// Hit-test chat conversation tabs. Returns `Some(index)` for a tab click,
    /// or `Some(self.conversations.len())` for the "+" button.
    pub fn conv_tab_at_x(&self, click_x: u16, area_x: u16) -> Option<usize> {
        let mut x = area_x;
        for (i, conv) in self.conversations.iter().enumerate() {
            let is_active = i == self.active_conv;
            let title: String = conv.title.chars().take(15).collect();
            let label_len = if is_active {
                format!(" [{}] ", title).len() as u16
            } else {
                format!("  {}  ", title).len() as u16
            };
            let tab_width = label_len + 1; // +1 for separator │
            if click_x >= x && click_x < x + tab_width {
                return Some(i);
            }
            x += tab_width;
        }
        // Check "+" button (space + '+')
        if click_x >= x && click_x < x + 2 {
            return Some(self.conversations.len());
        }
        None
    }

    /// Switch to conversation by index.
    pub fn switch_conversation(&mut self, idx: usize) {
        if idx < self.conversations.len() {
            self.active_conv = idx;
            self.scroll_pinned_to_bottom = true; // scroll to bottom
        }
    }

    /// Cycle to the next conversation.
    pub fn next_conversation(&mut self) {
        if !self.conversations.is_empty() {
            self.active_conv = (self.active_conv + 1) % self.conversations.len();
            self.scroll_pinned_to_bottom = true;
            self.history_index = None;
            self.history_stash.clear();
        }
    }

    /// Cycle to the previous conversation.
    pub fn prev_conversation(&mut self) {
        if !self.conversations.is_empty() {
            self.active_conv = if self.active_conv == 0 {
                self.conversations.len() - 1
            } else {
                self.active_conv - 1
            };
            self.scroll_pinned_to_bottom = true;
            self.history_index = None;
            self.history_stash.clear();
        }
    }

    /// Messages for multi-turn context, for the shared prompt-dispatch
    /// core (§2.2). Indexed because a remote prompt can target a
    /// conversation that is not the active tab.
    pub fn context_messages_at(&self, idx: usize) -> Vec<(&str, &str)> {
        self.conversations[idx]
            .messages
            .iter()
            .filter(|m| m.role == ChatRole::User || m.role == ChatRole::Assistant)
            .map(|m| {
                let role = match m.role {
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                    _ => "system",
                };
                (role, m.content.as_str())
            })
            .collect()
    }

    // ── Input editing ──────────────────────────────────────────

    // ── Editing (delegates to TextInput + autocomplete) ────────

    pub fn insert_char(&mut self, ch: char) {
        self.text_input.insert_char(ch);
        self.update_autocomplete();
    }

    pub fn insert_str(&mut self, text: &str) {
        // Normalize line endings: \r in a ratatui buffer cell is sent as a
        // literal carriage-return to the terminal, which jumps the cursor to
        // column 0 and overwrites other TUI panels (Explorer, Editor).
        if text.contains('\r') {
            let normalized = crate::editor::normalize_paste_newlines(text);
            self.text_input.insert_str(&normalized);
        } else {
            self.text_input.insert_str(text);
        }
        self.update_autocomplete();
    }

    pub fn backspace(&mut self) {
        self.text_input.backspace();
        self.update_autocomplete();
    }

    pub fn delete_word_back(&mut self) {
        self.text_input.delete_word_back();
        self.update_autocomplete();
    }

    // ── Input layout + vertical cursor movement ─────────────────

    /// Prompt label shown to the left of the input text (must match `render_input`).
    pub fn input_prompt_label(&self) -> &'static str {
        if self.renaming {
            "Rename: "
        } else if self.active_conv_streaming() {
            "Ctrl+C to cancel"
        } else if self.effective_auto_approve() {
            "[auto-approve] > "
        } else {
            "> "
        }
    }

    /// Side-panel content width → `(first_line_text_width, full_line_width)`.
    ///
    /// `panel_content_width` is the rect passed to `AgentChatState::render`
    /// (the side-panel content area). The chat block draws a left border, so
    /// the usable inner width is one column narrower.
    pub fn input_layout_widths(&self, panel_content_width: u16) -> (usize, usize) {
        self.input_layout_widths_for_inner(panel_content_width.saturating_sub(1))
    }

    fn input_layout_widths_for_inner(&self, inner_width: u16) -> (usize, usize) {
        let full_w = inner_width as usize;
        let prompt_len = self.input_prompt_label().chars().count();
        (full_w.saturating_sub(prompt_len), full_w)
    }

    /// Auto-grow the prompt box from wrapped visual lines, capped at `max_input`.
    ///
    /// Empty input stays at the 3-row minimum so the hint + cursor have room.
    pub(crate) fn auto_input_height(&self, inner_width: u16, max_input: u16) -> u16 {
        let (first_w, full_w) = self.input_layout_widths_for_inner(inner_width);
        let visual_lines = if self.text_input.text.is_empty() {
            1usize
        } else {
            self.build_visual_lines(first_w, full_w).len()
        };
        (visual_lines as u16).clamp(3, max_input.max(3))
    }

    /// Whether Up/Down should move within the input before history / chat scroll.
    pub fn input_has_multiple_visual_lines(&self, panel_content_width: u16) -> bool {
        if self.text_input.text.is_empty() {
            return false;
        }
        let (first_w, full_w) = self.input_layout_widths(panel_content_width);
        self.build_visual_lines(first_w, full_w).len() > 1
    }

    /// Visual line count of the current prompt at `panel_content_width`.
    pub fn input_visual_line_count(&self, panel_content_width: u16) -> usize {
        if self.text_input.text.is_empty() {
            return 1;
        }
        let (first_w, full_w) = self.input_layout_widths(panel_content_width);
        self.build_visual_lines(first_w, full_w).len()
    }

    /// True when the prompt has more visual lines than the cached input viewport.
    pub fn input_overflows_viewport(&self, panel_content_width: u16) -> bool {
        let Some(area) = self.input_area_cache else {
            return false;
        };
        if area.height == 0 || self.text_input.text.is_empty() {
            return false;
        }
        self.input_visual_line_count(panel_content_width) > area.height as usize
    }

    /// Move the input cursor by `delta` visual lines (negative = up).
    ///
    /// Used by mouse wheel / PageUp/PageDown when the prompt exceeds the input
    /// box; `render_input` keeps the cursor line visible via scroll-follow.
    /// Returns `true` when the cursor moved at least once.
    pub fn scroll_input_by_visual_lines(&mut self, delta: i32, panel_content_width: u16) -> bool {
        if delta == 0 || self.text_input.text.is_empty() {
            return false;
        }
        let before = self.text_input.cursor;
        if delta < 0 {
            for _ in 0..(-delta as usize) {
                if !self.cursor_up_in_input(panel_content_width) {
                    break;
                }
            }
        } else {
            for _ in 0..(delta as usize) {
                if !self.cursor_down_in_input(panel_content_width) {
                    break;
                }
            }
        }
        self.text_input.cursor != before
    }

    /// Move the cursor up within the input. Returns `true` when the cursor moved.
    pub fn cursor_up_in_input(&mut self, panel_content_width: u16) -> bool {
        if !self.input_has_multiple_visual_lines(panel_content_width) {
            return false;
        }
        let (first_w, full_w) = self.input_layout_widths(panel_content_width);
        self.text_input.sel_anchor = None;
        self.move_up_visual(first_w, full_w)
    }

    /// Move the cursor down within the input. Returns `true` when the cursor moved.
    pub fn cursor_down_in_input(&mut self, panel_content_width: u16) -> bool {
        if !self.input_has_multiple_visual_lines(panel_content_width) {
            return false;
        }
        let (first_w, full_w) = self.input_layout_widths(panel_content_width);
        self.text_input.sel_anchor = None;
        self.move_down_visual(first_w, full_w)
    }

    /// Extend the input selection upward by one visual line.
    pub fn select_up_in_input(&mut self, panel_content_width: u16) -> bool {
        if !self.input_has_multiple_visual_lines(panel_content_width) {
            return false;
        }
        let (first_w, full_w) = self.input_layout_widths(panel_content_width);
        self.text_input.ensure_anchor();
        self.move_up_visual(first_w, full_w)
    }

    /// Extend the input selection downward by one visual line.
    pub fn select_down_in_input(&mut self, panel_content_width: u16) -> bool {
        if !self.input_has_multiple_visual_lines(panel_content_width) {
            return false;
        }
        let (first_w, full_w) = self.input_layout_widths(panel_content_width);
        self.text_input.ensure_anchor();
        self.move_down_visual(first_w, full_w)
    }

    /// Scroll the conversation pane up by one rendered line.
    pub fn scroll_chat_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    /// Scroll the conversation pane down by one rendered line.
    pub fn scroll_chat_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    /// Move cursor up one visual line given the rendering widths.
    fn move_up_visual(&mut self, first_line_width: usize, full_width: usize) -> bool {
        let lines = self.build_visual_lines(first_line_width, full_width);
        let cursor_char_pos = self.text_input.cursor;
        let (cur_vline, cur_col) = Self::find_cursor_in_visual_lines(&lines, cursor_char_pos);
        if cur_vline == 0 {
            return false;
        }
        let (prev_start, prev_len) = lines[cur_vline - 1];
        self.text_input.cursor = prev_start + cur_col.min(prev_len);
        true
    }

    /// Move cursor down one visual line given the rendering widths.
    fn move_down_visual(&mut self, first_line_width: usize, full_width: usize) -> bool {
        let lines = self.build_visual_lines(first_line_width, full_width);
        let cursor_char_pos = self.text_input.cursor;
        let (cur_vline, cur_col) = Self::find_cursor_in_visual_lines(&lines, cursor_char_pos);
        if cur_vline >= lines.len() - 1 {
            return false;
        }
        let (next_start, next_len) = lines[cur_vline + 1];
        self.text_input.cursor = next_start + cur_col.min(next_len);
        true
    }

    /// Build visual lines as (start_char_idx, char_count) for the current input.
    pub(crate) fn build_visual_lines(
        &self,
        first_line_width: usize,
        full_width: usize,
    ) -> Vec<(usize, usize)> {
        let mut lines = Vec::new();
        let mut pos = 0;
        for (logical_idx, logical_line) in self.text_input.text.split('\n').enumerate() {
            let line_char_count = logical_line.chars().count();
            let avail = if lines.is_empty() {
                first_line_width
            } else {
                full_width
            };
            if line_char_count == 0 {
                lines.push((pos, 0));
            } else {
                let mut col = 0;
                let first_visual = lines.len();
                while col < line_char_count {
                    let w = if lines.is_empty() {
                        first_line_width
                    } else if lines.len() == first_visual {
                        avail
                    } else {
                        full_width
                    };
                    let w = w.max(1);
                    let take = w.min(line_char_count - col);
                    lines.push((pos + col, take));
                    col += take;
                }
            }
            pos += line_char_count;
            if logical_idx < self.text_input.text.matches('\n').count() {
                pos += 1;
            }
        }
        if lines.is_empty() {
            lines.push((0, 0));
        }
        lines
    }

    /// Find which visual line the cursor is on and the column within it.
    fn find_cursor_in_visual_lines(
        lines: &[(usize, usize)],
        cursor_char_pos: usize,
    ) -> (usize, usize) {
        for (i, &(start, len)) in lines.iter().enumerate() {
            if cursor_char_pos >= start && cursor_char_pos <= start + len {
                if cursor_char_pos < start + len || i == lines.len() - 1 {
                    return (i, cursor_char_pos - start);
                }
            }
            if i + 1 < lines.len() && cursor_char_pos == lines[i + 1].0 {
                return (i + 1, 0);
            }
        }
        if let Some(&(start, _)) = lines.last() {
            (lines.len() - 1, cursor_char_pos.saturating_sub(start))
        } else {
            (0, 0)
        }
    }

    /// Take the input text (for sending), clear the input field.
    pub fn take_input(&mut self) -> String {
        let text = self.text_input.text.clone();
        self.history_index = None;
        self.history_stash.clear();
        self.text_input.clear();
        self.autocomplete.reset();
        text
    }

    /// Get user messages from the active conversation (chronological, owned).
    fn active_user_messages(&self) -> Vec<String> {
        self.active_conversation()
            .messages
            .iter()
            .filter(|m| m.role == ChatRole::User)
            .map(|m| m.content.clone())
            .collect()
    }

    /// Navigate history upward (older). Called when Up is pressed with empty input.
    pub fn history_up(&mut self) {
        let user_msgs = self.active_user_messages();
        if user_msgs.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.history_stash = self.text_input.text.clone();
                let idx = user_msgs.len() - 1;
                self.history_index = Some(idx);
                self.text_input.text = user_msgs[idx].clone();
            }
            Some(idx) if idx > 0 => {
                let new_idx = idx - 1;
                self.history_index = Some(new_idx);
                self.text_input.text = user_msgs[new_idx].clone();
            }
            _ => {}
        }
        self.text_input.cursor = self.text_input.char_count();
    }

    /// Navigate history downward (newer). Called when Down is pressed while browsing history.
    pub fn history_down(&mut self) {
        let Some(idx) = self.history_index else {
            return;
        };
        let user_msgs = self.active_user_messages();
        if idx + 1 < user_msgs.len() {
            let new_idx = idx + 1;
            self.history_index = Some(new_idx);
            self.text_input.text = user_msgs[new_idx].clone();
        } else {
            self.history_index = None;
            self.text_input.text = std::mem::take(&mut self.history_stash);
        }
        self.text_input.cursor = self.text_input.char_count();
    }

    // ── Browse mode (copy from chat) ──────────────────────────

    /// Enter browse mode, selecting the last message.
    pub fn enter_browse_mode(&mut self) {
        let msg_count = self.active_conversation().messages.len();
        if msg_count == 0 {
            return;
        }
        self.browse_mode = true;
        self.browsed_msg = msg_count - 1;
    }

    /// Exit browse mode.
    pub fn exit_browse_mode(&mut self) {
        self.browse_mode = false;
    }

    // ── Mouse text selection ─────────────────────────────────────

    /// Map screen coordinates to a char index in the prompt input.
    ///
    /// Uses the same wrap + cursor-follow scroll as `render_input`, so hit
    /// testing matches what the user sees.
    pub fn screen_to_input_char(&self, col: u16, row: u16) -> Option<usize> {
        let area = self.input_area_cache?;
        if row < area.y || row >= area.y + area.height || col < area.x || col >= area.x + area.width
        {
            return None;
        }
        if self.text_input.text.is_empty() {
            return Some(0);
        }

        let prompt_len = self.input_prompt_label().chars().count();
        let text_width = (area.width as usize).saturating_sub(prompt_len);
        if text_width == 0 {
            return Some(0);
        }

        let lines = self.build_visual_lines(text_width, area.width as usize);
        let (cursor_line, _) = Self::find_cursor_in_visual_lines(&lines, self.text_input.cursor);
        let total_rows = area.height as usize;
        let scroll = if cursor_line >= total_rows {
            cursor_line - total_rows + 1
        } else {
            0
        };

        let viewport_row = (row - area.y) as usize;
        let line_idx = scroll + viewport_row;
        if line_idx >= lines.len() {
            return Some(self.text_input.char_count());
        }

        let (start, len) = lines[line_idx];
        let x_start = if line_idx == 0 {
            area.x.saturating_add(prompt_len as u16)
        } else {
            area.x
        };
        if col <= x_start {
            return Some(start);
        }

        let col_in_line = (col - x_start) as usize;
        let offset = col_in_line.min(len);
        Some((start + offset).min(self.text_input.char_count()))
    }

    /// Begin a mouse selection in the prompt (click / drag start).
    pub fn start_input_mouse_selection(&mut self, char_idx: usize) {
        let ci = char_idx.min(self.text_input.char_count());
        self.text_input.sel_anchor = Some(ci);
        self.text_input.cursor = ci;
        self.input_dragging = true;
        self.clear_text_selection();
    }

    /// Extend the prompt mouse selection to `char_idx`.
    pub fn extend_input_mouse_selection(&mut self, char_idx: usize) {
        if !self.input_dragging {
            return;
        }
        if self.text_input.sel_anchor.is_none() {
            self.text_input.sel_anchor = Some(self.text_input.cursor);
        }
        self.text_input.cursor = char_idx.min(self.text_input.char_count());
    }

    /// End prompt mouse selection; collapse a zero-width (plain click) selection.
    pub fn end_input_mouse_selection(&mut self) {
        self.input_dragging = false;
        if self.text_input.sel_anchor == Some(self.text_input.cursor) {
            self.text_input.sel_anchor = None;
        }
    }

    /// Map screen coordinates to a position in the rendered lines cache.
    /// Returns (line_index, char_index) where char_index is the character offset.
    pub fn screen_to_text_pos(&self, col: u16, row: u16) -> Option<(usize, usize)> {
        let area = self.conv_area_cache?;
        if row < area.y || row >= area.y + area.height {
            return None;
        }
        let viewport_row = (row - area.y) as usize;
        let line_idx = self.scroll_offset + viewport_row;
        if line_idx >= self.rendered_lines_cache.len() {
            // Clamp to last line
            let last = self.rendered_lines_cache.len().saturating_sub(1);
            let char_count = self
                .rendered_lines_cache
                .get(last)
                .map(|(l, _)| l.chars().count())
                .unwrap_or(0);
            return Some((last, char_count));
        }
        let target_col = col.saturating_sub(area.x) as usize;
        let line = &self.rendered_lines_cache[line_idx].0;
        let mut current_col = 0usize;
        let mut char_idx = 0usize;
        for ch in line.chars() {
            let w = UnicodeWidthChar::width(ch).unwrap_or(1);
            if current_col + w > target_col {
                return Some((line_idx, char_idx));
            }
            current_col += w;
            char_idx += 1;
        }
        Some((line_idx, char_idx))
    }

    /// Start a text selection at the given position.
    pub fn start_text_selection(&mut self, line_idx: usize, char_idx: usize) {
        self.text_sel_anchor = Some((line_idx, char_idx));
        self.text_sel_end = Some((line_idx, char_idx));
        self.chat_dragging = true;
        self.chat_output_kb_cursor = None;
        // Exit browse mode if active
        self.browse_mode = false;
    }

    /// Extend the text selection to the given position.
    pub fn extend_text_selection(&mut self, line_idx: usize, char_idx: usize) {
        self.text_sel_end = Some((line_idx, char_idx));
    }

    /// Clear the text selection.
    pub fn clear_text_selection(&mut self) {
        self.text_sel_anchor = None;
        self.text_sel_end = None;
        self.chat_dragging = false;
        self.chat_output_kb_cursor = None;
    }

    /// Check if there is an active text selection in the chat output.
    pub fn has_text_selection(&self) -> bool {
        matches!((self.text_sel_anchor, self.text_sel_end), (Some(a), Some(e)) if a != e)
    }

    /// Get the ordered selection range: (start_line, start_char, end_line, end_char).
    fn text_selection_range(&self) -> Option<(usize, usize, usize, usize)> {
        let (al, ac) = self.text_sel_anchor?;
        let (el, ec) = self.text_sel_end?;
        if al < el || (al == el && ac <= ec) {
            Some((al, ac, el, ec))
        } else {
            Some((el, ec, al, ac))
        }
    }

    /// Check if a character at (line_idx, char_idx) is within the selection.
    fn is_char_selected(&self, line_idx: usize, char_idx: usize) -> bool {
        let Some((sl, sc, el, ec)) = self.text_selection_range() else {
            return false;
        };
        if sl == el && sc == ec {
            return false;
        }
        if line_idx < sl || line_idx > el {
            return false;
        }
        if line_idx == sl && line_idx == el {
            return char_idx >= sc && char_idx < ec;
        }
        if line_idx == sl {
            return char_idx >= sc;
        }
        if line_idx == el {
            return char_idx < ec;
        }
        true
    }

    /// Extract the selected text from the cached rendered lines.
    /// Each rendered line is separated by a newline, matching the visual layout
    /// as the user sees it on screen.
    pub fn selected_chat_text(&self) -> Option<String> {
        let (sl, sc, el, ec) = self.text_selection_range()?;
        if sl == el && sc == ec {
            return None;
        }
        let mut result = String::new();
        for line_idx in sl..=el {
            if line_idx >= self.rendered_lines_cache.len() {
                break;
            }
            let (ref line, _msg_idx) = self.rendered_lines_cache[line_idx];
            let chars: Vec<char> = line.chars().collect();
            let start_c = if line_idx == sl {
                sc.min(chars.len())
            } else {
                0
            };
            let end_c = if line_idx == el {
                ec.min(chars.len())
            } else {
                chars.len()
            };

            if line_idx > sl {
                result.push('\n');
            }

            let selected: String = chars[start_c..end_c].iter().collect();
            result.push_str(&selected);
        }
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    /// Move to the previous message in browse mode.
    pub fn browse_up(&mut self) {
        if self.browsed_msg > 0 {
            self.browsed_msg -= 1;
        }
    }

    /// Move to the next message in browse mode.
    pub fn browse_down(&mut self) {
        let msg_count = self.active_conversation().messages.len();
        if self.browsed_msg + 1 < msg_count {
            self.browsed_msg += 1;
        }
    }

    /// Get the content of the currently browsed message.
    pub fn browsed_message_content(&self) -> Option<String> {
        self.active_conversation()
            .messages
            .get(self.browsed_msg)
            .map(|m| m.content.clone())
    }

    // ── Keyboard selection in chat output ─────────────────────────

    /// Extend selection upward by one line in chat output.
    pub fn select_up_in_output(&mut self) {
        let total = self.rendered_lines_cache.len();
        if total == 0 {
            return;
        }
        let viewport = self
            .conv_area_cache
            .map(|a| a.height as usize)
            .unwrap_or(20);
        let cursor = self
            .chat_output_kb_cursor
            .get_or_insert_with(|| (self.scroll_offset + viewport).min(total).saturating_sub(1));
        if self.text_sel_anchor.is_none() {
            self.text_sel_anchor = Some((*cursor, 0));
        }
        if *cursor > 0 {
            *cursor -= 1;
        }
        let col = self
            .rendered_lines_cache
            .get(*cursor)
            .map(|(l, _)| l.chars().count())
            .unwrap_or(0);
        self.text_sel_end = Some((*cursor, col));
        if *cursor < self.scroll_offset {
            self.scroll_offset = *cursor;
        }
    }

    /// Extend selection downward by one line in chat output.
    pub fn select_down_in_output(&mut self) {
        let total = self.rendered_lines_cache.len();
        if total == 0 {
            return;
        }
        let viewport = self
            .conv_area_cache
            .map(|a| a.height as usize)
            .unwrap_or(20);
        let cursor = self
            .chat_output_kb_cursor
            .get_or_insert_with(|| (self.scroll_offset + viewport).min(total).saturating_sub(1));
        if self.text_sel_anchor.is_none() {
            self.text_sel_anchor = Some((*cursor, 0));
        }
        if *cursor + 1 < total {
            *cursor += 1;
        }
        let col = self
            .rendered_lines_cache
            .get(*cursor)
            .map(|(l, _)| l.chars().count())
            .unwrap_or(0);
        self.text_sel_end = Some((*cursor, col));
        if *cursor >= self.scroll_offset + viewport {
            self.scroll_offset = (*cursor + 1).saturating_sub(viewport);
        }
    }

    // ── @file autocomplete ─────────────────────────────────────

    /// Check if cursor is inside an @reference or a `/attach` argument and
    /// update autocomplete state accordingly. The actual match list is
    /// populated by the caller (which holds the workspace file list).
    fn update_autocomplete(&mut self) {
        let byte_pos = self.text_input.cursor_byte_offset();
        let text = &self.text_input.text;
        let before_cursor = &text[..byte_pos];

        // ── /model <provider:model> completion ───────────────────────────
        if let Some(rest) = before_cursor.strip_prefix("/model ") {
            if !rest.contains('\n') {
                self.autocomplete.active = true;
                self.autocomplete.mode = AutocompleteMode::ModelSpec;
                self.autocomplete.at_pos = "/model ".len();
                self.autocomplete.query = rest.to_string();
                self.autocomplete.selected = 0;
                return;
            }
        }

        // ── /attach <path> completion ────────────────────────────────────
        //
        // Only kicks in on a single-line input starting with `/attach `
        // and when the cursor is in the argument span (so `/attach foo`
        // with the cursor at the very end completes `foo`).
        if let Some(rest) = before_cursor.strip_prefix("/attach ") {
            // Bail if the argument spans a newline (e.g. user moved the
            // cursor into a multi-line prompt body).
            if !rest.contains('\n') {
                let arg_start = "/attach ".len();
                self.autocomplete.active = true;
                self.autocomplete.mode = AutocompleteMode::AttachPath;
                self.autocomplete.at_pos = arg_start;
                self.autocomplete.query = rest.to_string();
                self.autocomplete.selected = 0;
                return;
            }
        }

        // ── /detach <name|all> completion ────────────────────────────────
        if let Some(rest) = before_cursor.strip_prefix("/detach ") {
            if !rest.contains('\n') {
                self.autocomplete.active = true;
                self.autocomplete.mode = AutocompleteMode::DetachName;
                self.autocomplete.at_pos = "/detach ".len();
                self.autocomplete.query = rest.to_string();
                self.autocomplete.selected = 0;
                return;
            }
        }

        // ── $skill invocation completion ────────────────────────────────
        let dollar_pos = before_cursor.rfind('$');
        if let Some(pos) = dollar_pos {
            if pos > 0 {
                let prev_byte = text.as_bytes()[pos - 1];
                if prev_byte == b'\\' {
                    // escaped — fall through to @ completion
                } else if prev_byte == b' ' || prev_byte == b'\n' || prev_byte == b'\t' {
                    let query = &before_cursor[pos + 1..];
                    if !query.contains(' ') && query.len() >= 2 {
                        self.autocomplete.active = true;
                        self.autocomplete.mode = AutocompleteMode::SkillRef;
                        self.autocomplete.at_pos = pos;
                        self.autocomplete.query = query.to_string();
                        self.autocomplete.selected = 0;
                        return;
                    }
                }
            } else {
                let query = &before_cursor[pos + 1..];
                if !query.contains(' ') && query.len() >= 2 {
                    self.autocomplete.active = true;
                    self.autocomplete.mode = AutocompleteMode::SkillRef;
                    self.autocomplete.at_pos = pos;
                    self.autocomplete.query = query.to_string();
                    self.autocomplete.selected = 0;
                    return;
                }
            }
        }

        // ── @path reference completion ──────────────────────────────────
        let at_pos = before_cursor.rfind('@');
        match at_pos {
            Some(pos) => {
                if pos > 0 {
                    let prev_byte = text.as_bytes()[pos - 1];
                    if prev_byte != b' ' && prev_byte != b'\n' && prev_byte != b'\t' {
                        self.autocomplete.reset();
                        return;
                    }
                }
                let query = &before_cursor[pos + 1..];
                if query.contains(' ') {
                    self.autocomplete.reset();
                    return;
                }
                self.autocomplete.active = true;
                self.autocomplete.mode = AutocompleteMode::FileRef;
                self.autocomplete.at_pos = pos;
                self.autocomplete.query = query.to_string();
                self.autocomplete.selected = 0;
            }
            None => {
                self.autocomplete.reset();
            }
        }
    }

    /// Update `$skill` autocomplete matches from the catalog.
    pub fn update_skill_autocomplete_matches(
        &mut self,
        catalog: &gaviero_core::skills::SkillCatalog,
        active_repo_id: Option<&str>,
    ) {
        if !self.autocomplete.active || self.autocomplete.mode != AutocompleteMode::SkillRef {
            return;
        }
        let prefix = &self.autocomplete.query;
        let hits = catalog.complete(prefix, active_repo_id);
        let mut by_name: std::collections::HashMap<&str, Vec<&gaviero_core::skills::Skill>> =
            std::collections::HashMap::new();
        for skill in &hits {
            by_name.entry(&skill.name).or_default().push(*skill);
        }
        self.autocomplete.matches = hits
            .iter()
            .map(|skill| {
                let siblings = by_name
                    .get(skill.name.as_str())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                skill_autocomplete_insert(catalog, skill, siblings)
            })
            .take(10)
            .collect();
        if self.autocomplete.selected >= self.autocomplete.matches.len() {
            self.autocomplete.selected = 0;
        }
    }

    /// Update `/model` autocomplete matches from discovered provider specs.
    pub fn update_model_autocomplete_matches(&mut self, discovered: &[String]) {
        if !self.autocomplete.active || self.autocomplete.mode != AutocompleteMode::ModelSpec {
            return;
        }
        self.autocomplete.matches = gaviero_core::swarm::backend::shared::model_spec_completions(
            &self.autocomplete.query,
            discovered,
        );
        if self.autocomplete.selected >= self.autocomplete.matches.len() {
            self.autocomplete.selected = 0;
        }
    }

    /// Update autocomplete matches from a list of workspace file paths.
    pub fn update_autocomplete_matches(&mut self, all_files: &[String]) {
        if !self.autocomplete.active
            || self.autocomplete.mode == AutocompleteMode::SkillRef
            || self.autocomplete.mode == AutocompleteMode::ModelSpec
            || self.autocomplete.mode == AutocompleteMode::DetachName
        {
            return;
        }
        let query_lower = self.autocomplete.query.to_lowercase();
        self.autocomplete.matches = all_files
            .iter()
            .filter(|f| {
                let f_lower = f.to_lowercase();
                if query_lower.is_empty() {
                    true // Show all when just '@'
                } else {
                    // Match anywhere in the path, or fuzzy on filename
                    f_lower.contains(&query_lower)
                }
            })
            .take(10) // Limit to 10 suggestions
            .cloned()
            .collect();
        if self.autocomplete.selected >= self.autocomplete.matches.len() {
            self.autocomplete.selected = 0;
        }
    }

    /// Accept the currently selected autocomplete match.
    pub fn accept_autocomplete(&mut self) {
        if !self.autocomplete.active || self.autocomplete.matches.is_empty() {
            return;
        }
        let selected = self
            .autocomplete
            .selected
            .min(self.autocomplete.matches.len() - 1);
        let path = self.autocomplete.matches[selected].clone();
        let anchor = self.autocomplete.at_pos;

        let cursor_byte = self.text_input.cursor_byte_offset();
        let after_cursor = if self.autocomplete.mode == AutocompleteMode::ModelSpec {
            // A model spec is a single whitespace-free token spanning to the
            // end of the line. Accepting with the cursor mid-token must
            // replace the *whole* token, so drop the remainder of it that
            // sits right of the cursor — otherwise an existing tail like
            // `cursor:composer-2.5` gets concatenated after the accepted
            // `claude:`, yielding the doubly-prefixed `claude:cursor:...`.
            let raw = &self.text_input.text[cursor_byte..];
            match raw.find(char::is_whitespace) {
                Some(ws) => raw[ws..].to_string(),
                None => String::new(),
            }
        } else {
            self.text_input.text[cursor_byte..].to_string()
        };
        self.text_input.text.truncate(anchor);

        match self.autocomplete.mode {
            AutocompleteMode::FileRef => {
                // Replace `@<query>` with `@<path> ` (trailing space keeps
                // typing fluid after accept).
                self.text_input.text.push('@');
                self.text_input.text.push_str(&path);
                self.text_input.text.push(' ');
            }
            AutocompleteMode::AttachPath | AutocompleteMode::DetachName => {
                // Replace just the argument; no trailing space because
                // `/attach` / `/detach` take a single token.
                self.text_input.text.push_str(&path);
            }
            AutocompleteMode::SkillRef => {
                self.text_input.text.push_str(&path);
                self.text_input.text.push(' ');
            }
            AutocompleteMode::ModelSpec => {
                self.text_input.text.push_str(&path);
            }
        }
        self.text_input.cursor = self.text_input.char_count();
        self.text_input.text.push_str(&after_cursor);

        self.autocomplete.reset();
    }

    /// Move autocomplete selection up.
    pub fn autocomplete_up(&mut self) {
        if self.autocomplete.active && !self.autocomplete.matches.is_empty() {
            self.autocomplete.selected = self.autocomplete.selected.saturating_sub(1);
        }
    }

    /// Move autocomplete selection down.
    pub fn autocomplete_down(&mut self) {
        if self.autocomplete.active && !self.autocomplete.matches.is_empty() {
            self.autocomplete.selected =
                (self.autocomplete.selected + 1).min(self.autocomplete.matches.len() - 1);
        }
    }

    // ── Attachments ────────────────────────────────────────────

    /// Add a file attachment for the next message.
    pub fn add_attachment(&mut self, path: PathBuf, kind: AttachmentKind) {
        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        // Avoid duplicates by path
        if !self.attachments.iter().any(|a| a.path == path) {
            self.attachments.push(Attachment {
                path,
                display_name,
                kind,
            });
        }
    }

    /// Remove an attachment by display name. Returns true if found.
    pub fn remove_attachment(&mut self, name: &str) -> bool {
        let before = self.attachments.len();
        self.attachments.retain(|a| a.display_name != name);
        self.attachments.len() < before
    }

    /// Remove an attachment by index. Returns the display name if removed.
    pub fn remove_attachment_at(&mut self, index: usize) -> Option<String> {
        if index < self.attachments.len() {
            let removed = self.attachments.remove(index);
            Some(removed.display_name)
        } else {
            None
        }
    }

    /// Hit-test a screen coordinate against attachment-bar close (`x`) buttons.
    pub fn attachment_close_at(&self, col: u16, row: u16) -> Option<usize> {
        self.attachment_close_hits
            .iter()
            .find(|h| h.y == row && h.x == col)
            .map(|h| h.index)
    }

    /// Take attachments for sending (clears the list).
    pub fn take_attachments(&mut self) -> Vec<Attachment> {
        std::mem::take(&mut self.attachments)
    }

    /// Populate `/detach` autocomplete matches from current attachments.
    pub fn update_detach_autocomplete_matches(&mut self) {
        if !self.autocomplete.active || self.autocomplete.mode != AutocompleteMode::DetachName {
            return;
        }
        let q = self.autocomplete.query.to_lowercase();
        let mut matches: Vec<String> = Vec::new();
        // Always offer `all` first when it matches the partial (prefix or
        // substring), so `/detach a` still suggests clearing everything.
        if q.is_empty() || "all".contains(&q) {
            matches.push("all".to_string());
        }
        for a in &self.attachments {
            if q.is_empty() || a.display_name.to_lowercase().contains(&q) {
                if !matches.iter().any(|m| m == &a.display_name) {
                    matches.push(a.display_name.clone());
                }
            }
        }
        self.autocomplete.matches = matches;
        if self.autocomplete.selected >= self.autocomplete.matches.len() {
            self.autocomplete.selected = 0;
        }
    }

    // ── Message management ─────────────────────────────────────

    pub fn add_user_message(&mut self, text: &str) {
        let idx = self.active_conv;
        self.add_user_message_at(idx, text);
    }

    /// User message on a specific conversation (shared by the desktop path
    /// and the remote prompt reducer).
    pub fn add_user_message_at(&mut self, idx: usize, text: &str) {
        let Some(conv) = self.conversations.get_mut(idx) else {
            return;
        };
        // Auto-title: set title from first user message (truncated)
        if conv.title == "New Chat" && conv.messages.is_empty() {
            let title: String = text.chars().take(30).collect();
            conv.title = if text.chars().count() > 30 {
                format!("{}...", title)
            } else {
                title
            };
            conv.bump_revision();
        }
        conv.push_message(ChatRole::User, text.to_string(), Vec::new());
        if idx == self.active_conv {
            self.scroll_to_bottom();
        }
    }

    // NOTE: message finalization is `finalize_message_to(conv_id, …)`.
    // There is no active-conversation variant: a turn can finish for a
    // background conversation, and the remote mirror projects the
    // finalized message by id.

    fn scroll_to_bottom(&mut self) {
        // Will be recalculated during render
        self.scroll_pinned_to_bottom = true;
        // Exit browse mode so auto-scroll takes precedence during streaming
        self.browse_mode = false;
        // Re-enable auto-scroll during streaming
        self.user_scrolled_during_stream = false;
    }

    fn auto_scroll_during_stream(&mut self) {
        self.browse_mode = false;
        if !self.user_scrolled_during_stream {
            self.scroll_pinned_to_bottom = true;
        }
    }

    // ── Conversation-ID-targeted methods (for parallel streaming) ──

    pub fn find_conv_idx(&self, conv_id: &str) -> Option<usize> {
        self.conversations.iter().position(|c| c.id == conv_id)
    }

    pub fn append_stream_chunk_to(&mut self, conv_id: &str, text: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else {
            return;
        };
        self.conversations[idx].streaming_status = "Writing...".to_string();
        let clean = crate::widgets::render_utils::strip_ansi(text);
        let conv = &mut self.conversations[idx];
        if let Some(last) = conv.messages.last_mut() {
            if last.role == ChatRole::Assistant {
                last.content.push_str(&clean);
                if idx == self.active_conv {
                    self.auto_scroll_during_stream();
                }
                return;
            }
        }
        conv.push_message(ChatRole::Assistant, clean, Vec::new());
        if idx == self.active_conv {
            self.auto_scroll_during_stream();
        }
    }

    pub fn add_tool_call_to(&mut self, conv_id: &str, tool_name: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else {
            return;
        };
        let conv = &mut self.conversations[idx];

        // Inline tool call markers into the message content so they appear
        // in chronological order alongside text (not grouped at the end).
        let marker = format!("\n[{}]", tool_name);
        if let Some(last) = conv.messages.last_mut() {
            if last.role == ChatRole::Assistant {
                last.content.push_str(&marker);
                last.tool_calls.push(tool_name.to_string());
                return;
            }
        }
        conv.push_message(ChatRole::Assistant, marker, vec![tool_name.to_string()]);
    }

    /// Append a compact file proposal summary to the assistant's current streaming message.
    /// Displayed as `[wrote path/to/file.rs +N -M]` inline in the chat output.
    pub fn append_deferred_summary(
        &mut self,
        conv_id: &str,
        path: &std::path::Path,
        additions: usize,
        deletions: usize,
    ) {
        let Some(idx) = self.find_conv_idx(conv_id) else {
            return;
        };
        let rel = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let summary = format!("\n[wrote {} +{} -{}]", rel, additions, deletions);
        let conv = &mut self.conversations[idx];
        if let Some(last) = conv.messages.last_mut() {
            if last.role == ChatRole::Assistant {
                last.content.push_str(&summary);
                if idx == self.active_conv {
                    self.auto_scroll_during_stream();
                }
                return;
            }
        }
        // No assistant message yet — create one with just the summary
        conv.push_message(ChatRole::Assistant, summary, Vec::new());
        if idx == self.active_conv {
            self.auto_scroll_during_stream();
        }
    }

    pub fn finalize_message_to(&mut self, conv_id: &str, role: &str, content: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else {
            return;
        };
        let chat_role = match role {
            "user" => ChatRole::User,
            "assistant" => ChatRole::Assistant,
            _ => ChatRole::System,
        };

        let clean = crate::widgets::render_utils::strip_ansi(content);
        let msgs = &mut self.conversations[idx].messages;

        // Assistant finalize must rewrite the last Assistant bubble even when
        // a System message was appended after streaming (disk-drift warnings,
        // etc.). Otherwise the streamed text — including a visible
        // `<turn_annotations>` sidecar — stays on screen above a new bubble.
        // Always apply `clean` (even when empty) so an annotations-only reply
        // cannot leave the streamed sidecar behind.
        if chat_role == ChatRole::Assistant {
            if let Some(msg) = msgs
                .iter_mut()
                .rev()
                .find(|m| m.role == ChatRole::Assistant)
            {
                msg.content = clean;
            } else if !clean.is_empty() {
                self.conversations[idx].push_message(chat_role, clean, Vec::new());
            }
            self.conversations[idx].is_streaming = false;
            self.conversations[idx].streaming_status.clear();
            self.conversations[idx].streaming_started_at = None;
            if idx == self.active_conv {
                self.scroll_to_bottom();
            }
            return;
        }

        if let Some(last) = msgs.last_mut() {
            if last.role == chat_role {
                if !clean.is_empty() {
                    last.content = clean.clone();
                }
                self.conversations[idx].is_streaming = false;
                self.conversations[idx].streaming_status.clear();
                self.conversations[idx].streaming_started_at = None;
                if idx == self.active_conv {
                    self.scroll_to_bottom();
                }
                return;
            }
        }

        if !clean.is_empty() {
            self.conversations[idx].push_message(chat_role, clean, Vec::new());
        }

        self.conversations[idx].is_streaming = false;
        self.conversations[idx].streaming_status.clear();
        self.conversations[idx].streaming_started_at = None;
        if idx == self.active_conv {
            self.scroll_to_bottom();
        }
    }

    /// Replace `<file>` blocks in the last assistant message with short summaries.
    pub fn collapse_file_blocks_in(&mut self, conv_id: &str) {
        let Some(idx) = self.find_conv_idx(conv_id) else {
            return;
        };
        let msgs = &mut self.conversations[idx].messages;
        if let Some(last) = msgs.last_mut() {
            if last.role == ChatRole::Assistant && last.content.contains("<file path=\"") {
                last.content = collapse_file_blocks(&last.content);
            }
        }
    }

    // ── Persistence ─────────────────────────────────────────────

    /// Load all conversations for a workspace from disk.
    pub fn load_conversations(&mut self, workspace_key: &std::path::Path) {
        use gaviero_core::session_state as ss;

        let index = ss::load_conversation_index(workspace_key);

        self.conversations.clear();
        self.active_conv = 0;
        for summary in &index.conversations {
            if let Some(stored) = ss::load_conversation(workspace_key, &summary.id) {
                let messages: Vec<ChatMessage> = stored
                    .messages
                    .into_iter()
                    .enumerate()
                    .map(|(i, m)| ChatMessage {
                        seq: i as u64 + 1,
                        role: match m.role.as_str() {
                            "user" => ChatRole::User,
                            "assistant" => ChatRole::Assistant,
                            _ => ChatRole::System,
                        },
                        content: m.content,
                        tool_calls: m.tool_calls,
                    })
                    .collect();
                // V9 §11 M4: restore the planner ledger and legacy
                // `claude_session_id` from disk so the first post-restart
                // turn can attempt resume. Fingerprint validation happens
                // lazily in `side_panel::send_chat_message` when the
                // current `ProviderProfile` is known — a model change
                // between sessions invalidates the handle there.
                //
                // Priority: prefer `session_ledger` (full state incl.
                // fingerprint + turn_count); fall back to bare
                // `continuity_handle` if an older reader/writer produced
                // only that. No handle at all → fresh bootstrap next turn.
                let claude_session_id = match (&stored.session_ledger, &stored.continuity_handle) {
                    (Some(l), _) => match &l.continuity_handle {
                        Some(gaviero_core::context_planner::ContinuityHandle::ClaudeSessionId(
                            id,
                        )) => Some(id.clone()),
                        _ => None,
                    },
                    (
                        None,
                        Some(gaviero_core::context_planner::ContinuityHandle::ClaudeSessionId(id)),
                    ) => Some(id.clone()),
                    _ => None,
                };
                // The full ledger is reconstructed on demand at send time
                // (needs the current ProviderProfile — unavailable here).
                // For now stash the persisted bytes back onto the conv so
                // the send path can call `SessionLedger::from_persisted`.
                let pending_ledger = stored.session_ledger;
                // T1: rehydrate last-known token usage so the context
                // indicator reflects the resumed session size immediately,
                // not zero-until-next-turn.
                let pending_usage = stored.last_token_usage.map(Into::into);

                // Restored messages are renumbered sequentially; the
                // counter continues past them. Remote clients resnapshot
                // on connect, so pre-restart cursors never survive anyway.
                let next_message_seq = messages
                    .last()
                    .map(|m: &ChatMessage| m.seq + 1)
                    .unwrap_or(1);
                self.conversations.push(Conversation {
                    id: stored.id,
                    conv_revision: 1,
                    next_message_seq,
                    title: stored.title,
                    messages,
                    model_override: stored.model_override,
                    effort_override: stored.effort_override,
                    namespace_override: None,
                    is_streaming: false,
                    streaming_status: String::new(),
                    streaming_started_at: None,
                    auto_approve: false,
                    pending_permission: None,
                    pending_turn_id: None,
                    pending_module_path: None,
                    pending_focused_folder: None,
                    workspace_wide_next: false,
                    lite_next: false,
                    no_inject_next: false,
                    inject_arms_next: gaviero_core::context_planner::BootstrapArms::none(),
                    context_mode_override: None,
                    claude_session_id,
                    // M4: in-memory ledger is rehydrated at send time from
                    // `pending_persisted_ledger` once the ProviderProfile
                    // is known. Initially None (lazy-init still triggers).
                    session_ledger: None,
                    pending_persisted_ledger: pending_ledger,
                    // Rehydrate-from-disk: this is exactly the case where
                    // re-inlining the transcript is load-bearing — Claude's
                    // server-side session may be gone, and `--resume` may
                    // refuse the stale id. Default `Auto` preserves that.
                    transcript_inline_mode: TranscriptInlineMode::Auto,
                    last_token_usage: pending_usage,
                    last_turn_cost_usd: 0.0,
                    last_bootstrap_tokens: 0,
                    last_bootstrap_arms: gaviero_core::context_planner::BootstrapArms::none(),
                    last_memory_injection_tokens: 0,
                });
            }
        }

        // Set active conversation
        if let Some(ref active_id) = index.active_id {
            if let Some(idx) = self.conversations.iter().position(|c| c.id == *active_id) {
                self.active_conv = idx;
            }
        }

        // Ensure at least one conversation exists
        if self.conversations.is_empty() {
            self.new_conversation();
        } else if self.active_conv >= self.conversations.len() {
            // Persisted `active_id` may be missing/stale; force a valid index.
            self.active_conv = 0;
        }

        self.scroll_pinned_to_bottom = true;
        self.history_index = None;
        self.history_stash.clear();
    }

    /// Save all conversations for a workspace to disk.
    pub fn save_conversations(&self, workspace_key: &std::path::Path) {
        use gaviero_core::session_state as ss;

        let mut summaries = Vec::new();
        for conv in &self.conversations {
            let stored = ss::StoredConversation {
                id: conv.id.clone(),
                title: conv.title.clone(),
                messages: conv
                    .messages
                    .iter()
                    .map(|m| ss::StoredMessage {
                        role: match m.role {
                            ChatRole::User => "user".to_string(),
                            ChatRole::Assistant => "assistant".to_string(),
                            ChatRole::System => "system".to_string(),
                        },
                        content: m.content.clone(),
                        tool_calls: m.tool_calls.clone(),
                        timestamp: 0,
                    })
                    .collect(),
                created: 0,
                updated: ss::now_unix(),
                model_override: conv.model_override.clone(),
                effort_override: conv.effort_override.clone(),
                // V9 §11 M4: persist planner ledger so first post-restart
                // turn can attempt resume and recover read-file cache +
                // thinking state. Fingerprint is part of the ledger and
                // invalidates the handle on model/toolset change.
                session_ledger: conv.session_ledger.as_ref().map(|l| l.to_persisted()),
                continuity_handle: conv
                    .session_ledger
                    .as_ref()
                    .and_then(|l| l.continuity_handle.clone()),
                last_token_usage: conv.last_token_usage.as_ref().map(Into::into),
            };

            summaries.push(ss::ConversationSummary {
                id: conv.id.clone(),
                title: conv.title.clone(),
                updated: stored.updated,
                message_count: conv.messages.len(),
            });

            if let Err(e) = ss::save_conversation(workspace_key, &stored) {
                tracing::warn!("Failed to save conversation {}: {}", conv.id, e);
            }
        }

        let active_id = self
            .conversations
            .get(self.active_conv)
            .map(|c| c.id.clone());
        let index = ss::ConversationIndex {
            conversations: summaries,
            active_id,
        };
        if let Err(e) = ss::save_conversation_index(workspace_key, &index) {
            tracing::warn!("Failed to save conversation index: {}", e);
        }
    }

    // ── Rendering ──────────────────────────────────────────────

    pub fn render(&mut self, area: Rect, buf: &mut RataBuf, focused: bool, theme: &Theme) {
        let border_style = if focused {
            Style::default().fg(theme::ACCENT)
        } else {
            Style::default().fg(theme::TEXT_DIM)
        };

        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width < 4 || inner.height < 5 {
            return;
        }

        // Conversation tabs (1 line at the top)
        let tab_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        self.render_conv_tabs(tab_area, buf);

        // Split remaining: conversation area + [attachments] + input area (bottom)
        let attach_height: u16 = if self.attachments.is_empty() { 0 } else { 1 };
        let remaining_y = inner.y + 1;
        let remaining_h = inner.height.saturating_sub(1);

        // Input height: a pending permission / question overlay claims what its
        // word-wrapped text needs, leaving the separator plus two conversation
        // rows visible. Otherwise user override, else auto-grow from *visual*
        // wrapped lines (not only explicit newlines — a long pasted paragraph
        // must enlarge the box the same way multi-line text does).
        let max_input = (inner.height / 2).max(3);
        let max_overlay = remaining_h.saturating_sub(attach_height + 3).max(3);
        let auto_height = self.auto_input_height(inner.width, max_input);
        let input_height: u16 = if let Some(rows) = self.pending_overlay_height(inner.width) {
            rows.clamp(3, max_overlay)
        } else if self.input_area_rows > 0 {
            self.input_area_rows.clamp(3, max_input)
        } else {
            auto_height
        };
        let conv_height = remaining_h.saturating_sub(input_height + 1 + attach_height); // +1 for separator
        let conv_area = Rect {
            x: inner.x,
            y: remaining_y,
            width: inner.width,
            height: conv_height,
        };
        let sep_y = remaining_y + conv_height;
        let attach_y = sep_y + 1;
        let input_area = Rect {
            x: inner.x,
            y: attach_y + attach_height,
            width: inner.width,
            height: input_height,
        };
        self.input_area_cache = Some(input_area);

        // Render conversation
        self.render_conversation(conv_area, buf, theme);

        // Render separator
        let sep_style = Style::default().fg(theme::BORDER_DIM);
        for x in 0..inner.width {
            let cx = inner.x + x;
            if cx < buf.area().right() && sep_y < buf.area().bottom() {
                buf[(cx, sep_y)].set_char('─').set_style(sep_style);
            }
        }

        // Render attachment bar (if any)
        if !self.attachments.is_empty() {
            self.render_attachments(
                Rect {
                    x: inner.x,
                    y: attach_y,
                    width: inner.width,
                    height: attach_height,
                },
                buf,
            );
        }

        // Render input area
        self.render_input(input_area, buf, focused, theme);

        // Render autocomplete popup above the input area
        if self.autocomplete.active && !self.autocomplete.matches.is_empty() {
            let popup_height = self.autocomplete.matches.len().min(8) as u16;
            let popup_y = sep_y.saturating_sub(popup_height);
            let popup_area = Rect {
                x: inner.x,
                y: popup_y,
                width: inner.width.min(50),
                height: popup_height,
            };
            self.render_autocomplete(popup_area, buf);
        }
    }

    fn render_conv_tabs(&self, area: Rect, buf: &mut RataBuf) {
        let bg = theme::TAB_BG;
        let fg_active = theme::TEXT_BRIGHT;
        let fg_inactive = theme::TEXT_DIM;

        // Clear tab bar
        for x in 0..area.width {
            let cx = area.x + x;
            if cx < buf.area().right() && area.y < buf.area().bottom() {
                buf[(cx, area.y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(bg));
            }
        }

        let mut x = area.x;
        for (i, conv) in self.conversations.iter().enumerate() {
            let is_active = i == self.active_conv;
            let style = if is_active {
                Style::default()
                    .fg(fg_active)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(fg_inactive).bg(bg)
            };

            // Truncate title to fit
            let title: String = conv.title.chars().take(15).collect();
            let label = if is_active {
                format!(" [{}] ", title)
            } else {
                format!("  {}  ", title)
            };

            let bullet_style = Style::default().fg(theme::SUCCESS).bg(bg);
            for (idx, ch) in label.chars().enumerate() {
                let (out_ch, out_style) = if idx == 0 && conv.is_streaming {
                    ('●', bullet_style)
                } else {
                    (ch, style)
                };
                if x < area.x + area.width && x < buf.area().right() {
                    buf[(x, area.y)].set_char(out_ch).set_style(out_style);
                    x += 1;
                }
            }

            // Separator
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, area.y)]
                    .set_char('│')
                    .set_style(Style::default().fg(theme::BORDER_DIM).bg(bg));
                x += 1;
            }
        }

        // "+" button for new conversation
        if x + 3 < area.x + area.width {
            let plus_style = Style::default().fg(theme::ACCENT).bg(bg);
            buf[(x, area.y)].set_char(' ').set_style(plus_style);
            x += 1;
            buf[(x, area.y)].set_char('+').set_style(plus_style);
        }
    }

    fn render_conversation(&mut self, area: Rect, buf: &mut RataBuf, theme: &Theme) {
        // Cache the conversation area for mouse hit-testing
        self.conv_area_cache = Some(area);

        if area.height == 0 {
            return;
        }

        // Subtract 1 to leave room for the scrollbar column on the right edge.
        let width = area.width.saturating_sub(1) as usize;
        let browse_bg = theme::BROWSE_BG; // highlight bg for browsed message
        let trace_enabled = chat_render_trace_enabled();
        let trace_started_at = trace_enabled.then(Instant::now);
        let (trace_message_count, trace_content_bytes) = if trace_enabled {
            let messages = self.messages();
            (
                messages.len(),
                messages
                    .iter()
                    .map(|message| message.content.len())
                    .sum::<usize>(),
            )
        } else {
            (0, 0)
        };
        let trace_streaming_status = if trace_enabled && self.active_conv_streaming() {
            Some(
                self.conversations[self.active_conv]
                    .streaming_status
                    .clone(),
            )
        } else {
            None
        };

        // Build rendered lines from messages: (segments, message_index).
        // Each rendered visual line is a sequence of styled segments so that
        // inline markdown styling (bold, italic, code, links) can be preserved
        // across the whole line — not just per-line.
        let mut lines: Vec<(
            Vec<crate::panels::chat_markdown::StyledSegment>,
            Option<usize>,
        )> = Vec::new();

        for (msg_idx, msg) in self.messages().iter().enumerate() {
            let (prefix, base_style) = match msg.role {
                ChatRole::User => ("You: ", Style::default().fg(theme::ACCENT)),
                ChatRole::Assistant => ("Assistant: ", Style::default().fg(theme::TEXT_FG)),
                ChatRole::System => ("System: ", Style::default().fg(theme::WARNING)),
            };

            // Filter <file> blocks and strip `<turn_annotations>` from display
            // (defense in depth — finalize also strips for storage/memory).
            let display_content = if msg.role == ChatRole::Assistant {
                filter_assistant_for_display(&msg.content)
            } else {
                filter_file_blocks_for_display(&msg.content)
            };

            if msg.role == ChatRole::Assistant && !display_content.is_empty() {
                // Render assistant messages with markdown formatting
                lines.push((
                    vec![crate::panels::chat_markdown::StyledSegment {
                        text: prefix.to_string(),
                        style: base_style,
                    }],
                    Some(msg_idx),
                ));
                let md_lines = crate::panels::chat_markdown::format_chat_markdown(
                    &display_content,
                    width,
                    base_style,
                );
                for cl in md_lines {
                    lines.push((cl.segments, Some(msg_idx)));
                }
            } else {
                // User/System: simple word-wrap (plain, no inline styling)
                let full_text = format!("{}{}", prefix, display_content);
                for line in crate::widgets::render_utils::word_wrap(&full_text, width) {
                    lines.push((
                        vec![crate::panels::chat_markdown::StyledSegment {
                            text: line,
                            style: base_style,
                        }],
                        Some(msg_idx),
                    ));
                }
            }

            // Blank line between messages
            lines.push((Vec::new(), None));
        }

        // Streaming indicator with animated spinner
        if self.active_conv_streaming() {
            let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            // Advance every ~6 ticks (~200ms at 33ms/tick)
            let frame = spinner_frames[(self.tick_count / 6) as usize % spinner_frames.len()];
            let conv = &self.conversations[self.active_conv];
            let status = &conv.streaming_status;
            let label = if status.is_empty() {
                "Thinking..."
            } else {
                status.as_str()
            };
            let elapsed_str = conv
                .streaming_started_at
                .map(|t| {
                    let secs = t.elapsed().as_secs();
                    if secs < 60 {
                        format!(" ({}s)", secs)
                    } else {
                        format!(" ({}m{}s)", secs / 60, secs % 60)
                    }
                })
                .unwrap_or_default();
            let stream_style = Style::default().fg(theme::ACCENT);
            lines.push((
                vec![crate::panels::chat_markdown::StyledSegment {
                    text: format!("{} {}{}", frame, label, elapsed_str),
                    style: stream_style,
                }],
                None,
            ));
        }

        // Cache rendered line texts + message index for mouse text selection
        self.rendered_lines_cache = lines
            .iter()
            .map(|(segments, mi)| {
                let text: String = segments.iter().map(|s| s.text.as_str()).collect();
                (text, *mi)
            })
            .collect();

        // In browse mode, scroll to keep the browsed message visible
        let total = lines.len();
        let viewport = area.height as usize;
        if self.browse_mode {
            // Find first and last line belonging to the browsed message
            let first_line = lines
                .iter()
                .position(|(_, mi)| *mi == Some(self.browsed_msg));
            let last_line = lines
                .iter()
                .rposition(|(_, mi)| *mi == Some(self.browsed_msg));
            if let (Some(first), Some(last)) = (first_line, last_line) {
                if first < self.scroll_offset {
                    self.scroll_offset = first;
                } else if last >= self.scroll_offset + viewport {
                    self.scroll_offset = last.saturating_sub(viewport - 1);
                }
            }
        } else if self.active_conv_streaming() && !self.user_scrolled_during_stream {
            // During streaming, keep the bottom visible unless the user
            // manually scrolled away to read earlier output.
            self.scroll_offset = total.saturating_sub(viewport);
        } else if self.active_conv_streaming() && self.user_scrolled_during_stream {
            // User scrolled away during streaming — if they've scrolled back
            // near the bottom, re-engage auto-scroll.
            if self.scroll_offset + viewport >= total {
                self.user_scrolled_during_stream = false;
                self.scroll_offset = total.saturating_sub(viewport);
            }
        } else if self.scroll_pinned_to_bottom || self.scroll_offset + viewport >= total {
            // Auto-scroll to bottom when pinned or already near the end
            self.scroll_offset = total.saturating_sub(viewport);
            self.scroll_pinned_to_bottom = false;
        }

        // Render visible lines
        let default_style = theme.default_style();
        for row in 0..viewport {
            let line_idx = self.scroll_offset + row;
            let y = area.y + row as u16;

            let is_browsed = self.browse_mode
                && line_idx < lines.len()
                && lines[line_idx].1 == Some(self.browsed_msg);

            let row_bg = if is_browsed {
                browse_bg
            } else {
                default_style.bg.unwrap_or(Color::Reset)
            };

            // Clear row
            let clear_style = if is_browsed {
                Style::default().bg(browse_bg)
            } else {
                default_style
            };
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() && y < buf.area().bottom() {
                    buf[(cx, y)].set_char(' ').set_style(clear_style);
                }
            }

            if line_idx < lines.len() {
                let (ref segments, _) = lines[line_idx];
                let sel_style = Style::default().fg(theme::TAB_BG).bg(theme::ACCENT);
                let mut cx = area.x;
                let mut char_idx = 0usize;
                for seg in segments {
                    let seg_style = if is_browsed {
                        seg.style.bg(row_bg)
                    } else {
                        seg.style
                    };
                    for ch in seg.text.chars() {
                        if ch == '\r' {
                            char_idx += 1;
                            continue;
                        }
                        let display_ch = if ch == '\t' { ' ' } else { ch };
                        let ch_width = UnicodeWidthChar::width(display_ch).unwrap_or(1) as u16;
                        let final_style = if self.is_char_selected(line_idx, char_idx) {
                            sel_style
                        } else {
                            seg_style
                        };
                        if cx + ch_width <= area.x + area.width
                            && cx < buf.area().right()
                            && y < buf.area().bottom()
                        {
                            buf[(cx, y)].set_char(display_ch).set_style(final_style);
                        }
                        cx += ch_width;
                        char_idx += 1;
                    }
                }
            }
        }

        // Browse mode hint
        if self.browse_mode {
            let hint = " [BROWSE] ↑↓ nav  Ctrl+C copy  Esc exit ";
            let hint_style = Style::default().fg(theme::TAB_BG).bg(theme::ACCENT);
            let hint_y = area.y;
            let hint_display_w = UnicodeWidthStr::width(hint) as u16;
            let hint_x = area.x + area.width.saturating_sub(hint_display_w);
            let mut cx = hint_x;
            for ch in hint.chars() {
                let ch_w = UnicodeWidthChar::width(ch).unwrap_or(1) as u16;
                if cx + ch_w <= area.x + area.width
                    && cx < buf.area().right()
                    && hint_y < buf.area().bottom()
                {
                    buf[(cx, hint_y)].set_char(ch).set_style(hint_style);
                }
                cx += ch_w;
            }
        }

        // Scrollbar
        crate::widgets::scrollbar::render_scrollbar(area, buf, total, viewport, self.scroll_offset);

        if let Some(started_at) = trace_started_at {
            let elapsed_ms = started_at.elapsed().as_millis();
            if elapsed_ms >= chat_render_trace_threshold_ms() {
                tracing::warn!(
                    target: "agent_chat_render",
                    elapsed_ms,
                    message_count = trace_message_count,
                    content_bytes = trace_content_bytes,
                    rendered_line_count = total,
                    width,
                    height = viewport,
                    streaming = self.active_conv_streaming(),
                    streaming_status = %trace_streaming_status.as_deref().unwrap_or(""),
                    "slow agent chat render"
                );
            }
        }
    }

    /// Paint word-wrapped overlay rows between the header row and the hint row,
    /// starting at `scroll`. Returns how many rows stay hidden below.
    fn render_overlay_body(
        area: Rect,
        buf: &mut RataBuf,
        rows: &[(String, bool)],
        scroll: usize,
        text_style: Style,
        mark_style: Style,
    ) -> usize {
        let capacity = area.height.saturating_sub(2) as usize;
        if capacity == 0 {
            return rows.len();
        }
        let scroll = scroll.min(rows.len().saturating_sub(capacity));
        let x_max = area.x + area.width;
        for (i, (text, marked)) in rows.iter().skip(scroll).take(capacity).enumerate() {
            let style = if *marked { mark_style } else { text_style };
            write_text(buf, area.x, area.y + 1 + i as u16, x_max, text, style);
        }
        rows.len().saturating_sub(scroll + capacity)
    }

    fn render_input(&self, area: Rect, buf: &mut RataBuf, focused: bool, _theme: &Theme) {
        let bg = theme::INPUT_BG;
        let fg = theme::TEXT_BRIGHT;
        let style = Style::default().fg(fg).bg(bg);

        // Clear input area
        for row in 0..area.height {
            for col in 0..area.width {
                let cx = area.x + col;
                let cy = area.y + row;
                if cx < buf.area().right() && cy < buf.area().bottom() {
                    buf[(cx, cy)].set_char(' ').set_style(style);
                }
            }
        }

        // Permission / AskUserQuestion overlay: replaces normal input area
        if let Some(ref perm) = self.conversations[self.active_conv].pending_permission {
            let warn_style = Style::default()
                .fg(theme::WARNING)
                .bg(bg)
                .add_modifier(Modifier::BOLD);
            let text_style = Style::default().fg(theme::TEXT_BRIGHT).bg(bg);
            let key_style = Style::default()
                .fg(theme::ACCENT)
                .bg(bg)
                .add_modifier(Modifier::BOLD);
            let muted = Style::default().fg(theme::TEXT_DIM).bg(bg);

            let x_max = area.x + area.width;
            let hint_y = area.y + area.height.saturating_sub(1);

            // Body rows are word-wrapped to the overlay width, so a long
            // question or command stays readable instead of being cut off.
            let rows = perm.overlay_rows(area.width as usize);
            let hidden =
                Self::render_overlay_body(area, buf, &rows, perm.scroll, text_style, key_style);
            let scroll_hint = if hidden > 0 {
                format!(" PgDn ▾ +{hidden} ")
            } else if perm.scroll > 0 {
                " PgUp ▴ ".to_string()
            } else {
                String::new()
            };

            if let Some(ref ask) = perm.ask {
                write_text(
                    buf,
                    area.x,
                    area.y,
                    x_max,
                    " ❓ Agent question ",
                    warn_style,
                );
                let hint = if ask.questions.len() > 1 {
                    format!(
                        " [1-9] select  ↑↓ question ({}/{})  Enter submit  n deny ",
                        ask.focus_q + 1,
                        ask.questions.len()
                    )
                } else {
                    " [1-9] select  Enter submit  n deny ".to_string()
                };
                let hx = write_text(buf, area.x, hint_y, x_max, &hint, muted);
                write_text(buf, hx, hint_y, x_max, &scroll_hint, muted);
            } else {
                let header = format!(" ⚠ Permission: {} ", perm.tool_name);
                write_text(buf, area.x, area.y, x_max, &header, warn_style);

                // Last line: key hints, with y/n accented.
                let mut hx = write_text(buf, area.x, hint_y, x_max, " [", text_style);
                hx = write_text(buf, hx, hint_y, x_max, "y", key_style);
                hx = write_text(buf, hx, hint_y, x_max, "] Allow  [", text_style);
                hx = write_text(buf, hx, hint_y, x_max, "n", key_style);
                hx = write_text(buf, hx, hint_y, x_max, "] Deny ", text_style);
                write_text(buf, hx, hint_y, x_max, &scroll_hint, muted);
            }
            return;
        }

        let prompt = self.input_prompt_label();
        let prompt_style = Style::default().fg(theme::ACCENT).bg(bg);

        let mut x = area.x;
        for ch in prompt.chars() {
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, area.y)].set_char(ch).set_style(prompt_style);
                x += 1;
            }
        }

        // Input text — wraps across available rows
        let prompt_len = (x - area.x) as usize;
        let text_width = (area.width as usize).saturating_sub(prompt_len);
        let total_rows = area.height as usize;

        if self.text_input.text.is_empty() && !self.active_conv_streaming() {
            // Hint text
            let hint = "Type a message, Enter to send";
            let hint_style = Style::default().fg(theme::TEXT_DIM).bg(bg);
            for (i, ch) in hint.chars().enumerate() {
                let hx = x + i as u16;
                if hx < area.x + area.width && hx < buf.area().right() {
                    buf[(hx, area.y)].set_char(ch).set_style(hint_style);
                }
            }
            // Show cursor at prompt position even with empty input
            if focused {
                let cursor_style = Style::default().fg(bg).bg(theme::TEXT_FG);
                if x < area.x + area.width && x < buf.area().right() && area.y < buf.area().bottom()
                {
                    buf[(x, area.y)].set_style(cursor_style);
                }
            }
        } else if text_width > 0 {
            let input_chars: Vec<char> = self.text_input.text.chars().collect();
            let lines = self.build_visual_lines(text_width, area.width as usize);
            let (cursor_line, cursor_col) =
                Self::find_cursor_in_visual_lines(&lines, self.text_input.cursor);

            // Scroll so cursor line is visible
            let scroll = if cursor_line >= total_rows {
                cursor_line - total_rows + 1
            } else {
                0
            };

            // Render visible lines
            for row in 0..total_rows {
                let line_idx = scroll + row;
                if line_idx >= lines.len() {
                    break;
                }
                let (start, len) = lines[line_idx];
                let y = area.y + row as u16;
                let x_start = if line_idx == 0 { x } else { area.x };

                // Get selection range for highlighting
                let sel_range = self.text_input.selection_range();

                for i in 0..len {
                    let ch_idx = start + i;
                    if ch_idx < input_chars.len() {
                        let ch = input_chars[ch_idx];
                        if ch == '\n' || ch == '\r' {
                            continue;
                        }
                        let cx = x_start + i as u16;
                        if cx < area.x + area.width
                            && cx < buf.area().right()
                            && y < buf.area().bottom()
                        {
                            let ch_style =
                                if sel_range.map_or(false, |(s, e)| ch_idx >= s && ch_idx < e) {
                                    Style::default().fg(fg).bg(theme::SELECTION_BG)
                                } else {
                                    style
                                };
                            buf[(cx, y)].set_char(ch).set_style(ch_style);
                        }
                    }
                }
            }

            // Position cursor
            if focused && !self.active_conv_streaming() {
                let visible_cursor_line = cursor_line.saturating_sub(scroll);
                if visible_cursor_line < total_rows {
                    let y = area.y + visible_cursor_line as u16;
                    let x_start = if cursor_line == 0 { x } else { area.x };
                    let cursor_x = x_start + cursor_col as u16;
                    if cursor_x < area.x + area.width
                        && cursor_x < buf.area().right()
                        && y < buf.area().bottom()
                    {
                        let cursor_style = Style::default().fg(bg).bg(theme::TEXT_FG);
                        buf[(cursor_x, y)].set_style(cursor_style);
                    }
                }
            }

            // Scrollbar when the prompt exceeds the input viewport.
            if lines.len() > total_rows {
                crate::widgets::scrollbar::render_scrollbar(
                    area,
                    buf,
                    lines.len(),
                    total_rows,
                    scroll,
                );
            }
        }
    }

    fn render_autocomplete(&self, area: Rect, buf: &mut RataBuf) {
        let bg = theme::INPUT_BG;
        let fg = theme::TEXT_FG;
        let selected_bg = theme::SELECTION_BG;

        for (row, (i, path)) in self
            .autocomplete
            .matches
            .iter()
            .enumerate()
            .take(area.height as usize)
            .enumerate()
        {
            let y = area.y + row as u16;
            let is_selected = i == self.autocomplete.selected;
            let style = if is_selected {
                Style::default().fg(Color::White).bg(selected_bg)
            } else {
                Style::default().fg(fg).bg(bg)
            };

            // Clear row
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() && y < buf.area().bottom() {
                    buf[(cx, y)].set_char(' ').set_style(style);
                }
            }

            // Prefix only relevant for @file refs — the /attach popup shows
            // bare paths so they line up with what gets inserted on accept.
            let display = match self.autocomplete.mode {
                AutocompleteMode::FileRef => format!(" @{}", path),
                AutocompleteMode::AttachPath
                | AutocompleteMode::DetachName
                | AutocompleteMode::ModelSpec => {
                    format!(" {}", path)
                }
                AutocompleteMode::SkillRef => path.clone(),
            };
            for (ci, ch) in display.chars().enumerate() {
                let cx = area.x + ci as u16;
                if cx < area.x + area.width && cx < buf.area().right() && y < buf.area().bottom() {
                    buf[(cx, y)].set_char(ch).set_style(style);
                }
            }
        }
    }

    /// Render attachment badges in a single-line bar and record close-button
    /// hit regions. Each badge is ` name x ` — clicking the `x` removes it.
    fn render_attachments(&mut self, area: Rect, buf: &mut RataBuf) {
        self.attachment_close_hits.clear();

        let bg = theme::INPUT_BG;
        let badge_fg = theme::TEXT_BRIGHT;
        let badge_bg = theme::BADGE_BG;
        let img_badge_bg = theme::IMAGE_BADGE_BG;
        let close_style = Style::default().fg(theme::ERROR).bg(theme::BADGE_BG);
        let close_img_style = Style::default().fg(theme::ERROR).bg(theme::IMAGE_BADGE_BG);
        let label_style = Style::default().fg(theme::TEXT_DIM).bg(bg);

        let y = area.y;
        if y >= buf.area().bottom() {
            return;
        }

        for col in 0..area.width {
            let cx = area.x + col;
            if cx < buf.area().right() {
                buf[(cx, y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(bg));
            }
        }

        let mut x = area.x;
        let label = " Attached: ";
        for ch in label.chars() {
            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, y)].set_char(ch).set_style(label_style);
                x += 1;
            }
        }

        for index in 0..self.attachments.len() {
            let display_name = self.attachments[index].display_name.clone();
            let kind = self.attachments[index].kind.clone();
            let this_bg = if kind == AttachmentKind::Image {
                img_badge_bg
            } else {
                badge_bg
            };
            let style = Style::default().fg(badge_fg).bg(this_bg);
            let x_style = if kind == AttachmentKind::Image {
                close_img_style
            } else {
                close_style
            };

            let name_part = format!(" {} ", display_name);
            for ch in name_part.chars() {
                if x < area.x + area.width && x < buf.area().right() {
                    buf[(x, y)].set_char(ch).set_style(style);
                    x += 1;
                }
            }

            if x < area.x + area.width && x < buf.area().right() {
                self.attachment_close_hits
                    .push(AttachmentCloseHit { x, y, index });
                buf[(x, y)].set_char('x').set_style(x_style);
                x += 1;
            }

            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, y)].set_char(' ').set_style(style);
                x += 1;
            }

            if x < area.x + area.width && x < buf.area().right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(bg));
                x += 1;
            }
        }
    }
}

/// Hidden provider context (built-in system prompt, tool schemas, auto-loaded
/// project docs) not visible in the chat panel. Used only in the composite
/// estimate when `prefix_tokens()` is unavailable.
fn hidden_provider_overhead_tokens(model: &str) -> usize {
    let provider = model.split_once(':').map(|(p, _)| p).unwrap_or("claude");
    match provider {
        "ollama" | "local" => 512,
        "codex" | "cursor" => 12_000,
        _ => 18_000,
    }
}

/// Count words by splitting on Unicode whitespace. Empty/whitespace-only
/// input yields 0.
fn count_words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Convert a word count to an approximate token count using the rule of
/// thumb that an LLM token is ~30% smaller than an English word:
/// `tokens ≈ words × 1.3`. Integer math: `(words × 13) / 10`.
fn words_to_tokens(words: usize) -> usize {
    words.saturating_mul(13) / 10
}

/// Normalize a user-typed model spec to canonical `provider:model` form.
///
/// Specs that pass [`gaviero_core::swarm::backend::shared::validate_model_spec`]
/// are stored verbatim. Bare names are auto-prefixed with `claude:` so old
/// muscle memory (`/model opus`) keeps working, but the stored value is
/// always the canonical form.
fn normalize_model_spec(arg: &str) -> String {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if gaviero_core::swarm::backend::shared::validate_model_spec(trimmed).is_ok() {
        return trimmed.to_string();
    }
    // Already provider-prefixed but invalid (e.g. `deepseek:deepseek-v4`) —
    // return verbatim so validation surfaces the real error instead of
    // rewriting to `claude:deepseek:…`.
    if let Some((prefix, _)) = trimmed.split_once(':') {
        if gaviero_core::swarm::backend::shared::SUPPORTED_PROVIDER_PREFIXES.contains(&prefix) {
            return trimmed.to_string();
        }
    }
    // Back-compat aliases for the legacy bare/dashed shorthand.
    let canonical = match trimmed {
        "claude-sonnet" => "sonnet",
        "claude-opus" => "opus",
        "claude-haiku" => "haiku",
        "claude-fable" => "fable",
        other => other,
    };
    format!("claude:{}", canonical)
}

/// Parse `$skill` invocations from chat text.
///
/// Returns `(cleaned_text, resolved_skills, warnings)`. Unknown invocations
/// stay verbatim in the cleaned text with a non-fatal warning.
pub fn parse_skill_invocations(
    text: &str,
    catalog: &gaviero_core::skills::SkillCatalog,
    active_repo_id: Option<&str>,
) -> (
    String,
    Vec<gaviero_core::skills::ResolvedSkill>,
    Vec<gaviero_core::skills::SkillWarning>,
) {
    let mut out = String::new();
    let mut resolved = Vec::new();
    let mut warnings = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] != b'$' {
            out.push(text[i..].chars().next().unwrap());
            i += text[i..].chars().next().unwrap().len_utf8();
            continue;
        }

        let (bs_count, _) = count_backslashes(bytes, i);
        if bs_count % 2 == 1 {
            out.push('$');
            i += 1;
            continue;
        }
        for _ in 0..bs_count / 2 {
            out.push('\\');
        }

        let rest = &text[i + 1..];
        if rest.starts_with('{') || rest.starts_with('(') {
            out.push('$');
            i += 1;
            continue;
        }

        if let Some(parsed) = try_parse_invocation(text, i) {
            let ParsedInvocation {
                qualifier,
                name,
                args,
                raw_args,
                consumed,
            } = parsed;
            if let Some(skill) = catalog.resolve(qualifier.as_deref(), name, active_repo_id) {
                let rendered = skill.render(&args, &raw_args);
                resolved.push(gaviero_core::skills::ResolvedSkill {
                    name: skill.name.clone(),
                    scope_level: skill.scope_level,
                    rendered_body: rendered,
                });
                i += consumed;
                continue;
            }
            warnings.push(gaviero_core::skills::SkillWarning {
                name: name.to_string(),
                message: "unknown skill".to_string(),
            });
            out.push_str(&text[i..i + consumed]);
            i += consumed;
            continue;
        }

        out.push('$');
        i += 1;
    }

    (out, resolved, warnings)
}

fn count_backslashes(bytes: &[u8], dollar_pos: usize) -> (usize, usize) {
    let mut count = 0usize;
    let mut pos = dollar_pos;
    while pos > 0 && bytes[pos - 1] == b'\\' {
        count += 1;
        pos -= 1;
    }
    (count, pos)
}

fn is_name_char(c: char, first: bool) -> bool {
    if first {
        c.is_ascii_alphabetic() || c == '_'
    } else {
        c.is_ascii_alphanumeric() || c == '_' || c == '-'
    }
}

fn parse_identifier(s: &str) -> Option<(&str, usize)> {
    let mut chars = s.char_indices();
    let (start, first) = chars.next()?;
    if !is_name_char(first, true) {
        return None;
    }
    let mut end = start + first.len_utf8();
    for (_, ch) in chars {
        if is_name_char(ch, false) {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    Some((&s[start..end], end))
}

fn parse_paren_args(s: &str) -> Option<(Vec<String>, String, usize)> {
    if !s.starts_with('(') {
        return None;
    }
    let close = s.find(')')?;
    let inner = s[1..close].trim();
    let args: Vec<String> = if inner.is_empty() {
        Vec::new()
    } else {
        inner
            .split(',')
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect()
    };
    let raw = inner.to_string();
    Some((args, raw, close + 1))
}

fn line_start_space_args(
    text: &str,
    dollar: usize,
    after_name_byte: usize,
) -> Option<(Vec<String>, String, usize)> {
    let line_start = text[..dollar].rfind('\n').map(|p| p + 1).unwrap_or(0);
    if !text[line_start..dollar].trim().is_empty() {
        return None;
    }
    let rest = &text[after_name_byte..];
    if rest.starts_with('(') {
        return None;
    }
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let tail = rest[..line_end].trim();
    if tail.is_empty() {
        return Some((Vec::new(), String::new(), 0));
    }
    let args: Vec<String> = tail.split_whitespace().map(|s| s.to_string()).collect();
    let raw = tail.to_string();
    let leading_ws = rest.len() - rest.trim_start().len();
    Some((args, raw, leading_ws + line_end))
}

struct ParsedInvocation<'a> {
    qualifier: Option<String>,
    name: &'a str,
    args: Vec<String>,
    raw_args: String,
    consumed: usize,
}

fn try_parse_invocation(text: &str, dollar: usize) -> Option<ParsedInvocation<'_>> {
    let after = &text[dollar + 1..];
    let (first, first_len) = parse_identifier(after)?;

    let (qualifier, name, rest_offset) = if let Some(tail) = after.get(first_len..) {
        if let Some(stripped) = tail.strip_prefix('/') {
            let (skill_name, name_len) = parse_identifier(stripped)?;
            (
                Some(first.to_string()),
                skill_name,
                first_len + 1 + name_len,
            )
        } else {
            (None, first, first_len)
        }
    } else {
        (None, first, first_len)
    };

    let rest = &after[rest_offset..];
    let after_name_byte = dollar + 1 + rest_offset;
    let (args, raw_args, arg_len) = if let Some((a, r, l)) = parse_paren_args(rest) {
        (a, r, l)
    } else if rest.starts_with('(') {
        return None;
    } else if let Some((a, r, l)) = line_start_space_args(text, dollar, after_name_byte) {
        (a, r, l)
    } else {
        (Vec::new(), String::new(), 0)
    };

    let consumed = 1 + rest_offset + arg_len;
    Some(ParsedInvocation {
        qualifier,
        name,
        args,
        raw_args,
        consumed,
    })
}

/// Build the insertion text for a skill autocomplete selection.
pub fn skill_autocomplete_insert(
    catalog: &gaviero_core::skills::SkillCatalog,
    skill: &gaviero_core::skills::Skill,
    all_with_name: &[&gaviero_core::skills::Skill],
) -> String {
    let bare = format!("${}", skill.name);
    if all_with_name.len() <= 1 {
        if skill.arguments.is_empty() {
            return bare;
        }
        return format!("{}()", bare);
    }
    let label = catalog.source_label(skill);
    let qualified = format!("${}/{}", label, skill.name);
    if skill.arguments.is_empty() {
        qualified
    } else {
        format!("{}()", qualified)
    }
}

/// Parse `@path/to/file` references from input text.
/// Returns a list of relative file paths referenced.
pub fn parse_file_references(text: &str) -> Vec<String> {
    let mut refs = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'@' {
            // Check that @ is at start or preceded by whitespace
            if i > 0 && bytes[i - 1] != b' ' && bytes[i - 1] != b'\n' && bytes[i - 1] != b'\t' {
                i += 1;
                continue;
            }
            // Collect the reference (non-whitespace chars after @)
            let start = i + 1;
            let mut end = start;
            while end < len && bytes[end] != b' ' && bytes[end] != b'\n' && bytes[end] != b'\t' {
                end += 1;
            }
            if end > start {
                let path = &text[start..end];
                refs.push(path.to_string());
            }
            i = end;
        } else {
            i += 1;
        }
    }

    refs
}

/// Filter `<file>` blocks for display: collapse complete blocks and hide in-progress ones.
fn filter_file_blocks_for_display(text: &str) -> String {
    use crate::app::collapse_file_blocks;

    // First collapse any complete <file ...>...</file> blocks
    let collapsed = collapse_file_blocks(text);

    // Then handle any in-progress (unclosed) <file block from streaming
    if let Some(tag_start) = collapsed.rfind("<file path=\"") {
        // Check if there's a closing </file> after this opening tag
        if collapsed[tag_start..].find("</file>").is_none() {
            // In-progress block — extract path if available and truncate
            let after_attr = tag_start + "<file path=\"".len();
            let label = if let Some(quote_end) = collapsed[after_attr..].find('"') {
                let path = &collapsed[after_attr..after_attr + quote_end];
                format!("[writing {}...]", path)
            } else {
                "[writing file...]".to_string()
            };
            let mut result = collapsed[..tag_start].to_string();
            result.push_str(&label);
            return result;
        }
    }

    collapsed
}

/// Assistant display filter: collapse `<file>` blocks and strip a trailing
/// `<turn_annotations>` sidecar so the memory JSON never reaches the user,
/// even if finalize missed the bubble (interleaved system messages).
fn filter_assistant_for_display(text: &str) -> String {
    let collapsed = filter_file_blocks_for_display(text);
    gaviero_core::memory::parse_and_strip(&collapsed).stripped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_skill_catalog(
        name: &str,
        body: &str,
        description: &str,
    ) -> (gaviero_core::skills::SkillCatalog, tempfile::TempDir) {
        test_skill_catalog_with_args(name, body, description, &[])
    }

    fn test_skill_catalog_with_args(
        name: &str,
        body: &str,
        description: &str,
        arguments: &[&str],
    ) -> (gaviero_core::skills::SkillCatalog, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let skill_dir = tmp.path().join(".gaviero").join("skills").join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        let args_line = if arguments.is_empty() {
            String::new()
        } else {
            format!(
                "arguments: [{}]\n",
                arguments
                    .iter()
                    .map(|a| format!("{a}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\ndescription: {description}\n{args_line}---\n{body}\n"),
        )
        .unwrap();
        let ws = gaviero_core::workspace::Workspace::single_folder(tmp.path().to_path_buf());
        let (catalog, _) =
            gaviero_core::skills::SkillCatalog::scan(&ws, std::path::Path::new("/nonexistent"));
        (catalog, tmp)
    }

    #[test]
    fn parse_skill_invocations_strips_known_skill() {
        let (catalog, _tmp) =
            test_skill_catalog("lint", "Lint the code.", "Run the linter on changed files");
        let (cleaned, resolved, warnings) = parse_skill_invocations("please $lint", &catalog, None);
        assert!(warnings.is_empty());
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].name, "lint");
        assert_eq!(cleaned, "please ");
    }

    #[test]
    fn parse_skill_invocations_parens_inline_mid_sentence() {
        let (catalog, _tmp) = test_skill_catalog(
            "migrate",
            "Migrate $0 from $1 to $2.",
            "Migrate a UI component between frameworks",
        );
        let (cleaned, resolved, _) = parse_skill_invocations(
            "fix it with $migrate(SearchBar, React, Vue) then ship",
            &catalog,
            None,
        );
        assert_eq!(cleaned, "fix it with  then ship");
        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].rendered_body.contains("SearchBar"));
    }

    #[test]
    fn parse_skill_invocations_unknown_stays_verbatim_with_warning() {
        let (catalog, _tmp) =
            test_skill_catalog("known", "body", "known skill for verbatim unknown test");
        let (cleaned, resolved, warnings) =
            parse_skill_invocations("use $PATH here", &catalog, None);
        assert!(resolved.is_empty());
        assert_eq!(warnings.len(), 1);
        assert_eq!(cleaned, "use $PATH here");
    }

    #[test]
    fn parse_skill_invocations_dollar_amount_not_invocation() {
        let (catalog, _tmp) =
            test_skill_catalog("x", "body", "unused skill for dollar amount test");
        let (cleaned, resolved, warnings) =
            parse_skill_invocations("cost is $5.00 today", &catalog, None);
        assert!(resolved.is_empty());
        assert!(warnings.is_empty());
        assert_eq!(cleaned, "cost is $5.00 today");
    }

    #[test]
    fn parse_skill_invocations_escape_is_literal() {
        let (catalog, _tmp) =
            test_skill_catalog("lint", "body", "lint skill for escape literal test");
        let (cleaned, resolved, warnings) =
            parse_skill_invocations(r"run \$lint(x)", &catalog, None);
        assert!(resolved.is_empty());
        assert!(warnings.is_empty());
        assert_eq!(cleaned, r"run \$lint(x)");
    }

    #[test]
    fn skill_autocomplete_insert_unique_with_args_adds_parens() {
        let (catalog, _tmp) = test_skill_catalog_with_args(
            "review",
            "Review $path",
            "review code at a path",
            &["path"],
        );
        let skill = catalog.resolve(None, "review", None).unwrap();
        let insert = skill_autocomplete_insert(&catalog, skill, &[skill]);
        assert_eq!(insert, "$review()");
    }

    #[test]
    fn parse_file_references_extracts_multiple_paths() {
        let refs = parse_file_references("check @src/main.rs and\n\t@docs/readme.md");
        assert_eq!(refs, vec!["src/main.rs", "docs/readme.md"]);
    }

    #[test]
    fn parse_file_references_ignores_embedded_at_symbols() {
        let refs =
            parse_file_references("mail me at user@example.com or foo@bar but inspect @src/lib.rs");
        assert_eq!(refs, vec!["src/lib.rs"]);
    }

    #[test]
    fn effective_model_prefers_conversation_override() {
        let mut state = AgentChatState::new();
        state.agent_settings.model = "claude:sonnet".to_string();
        state.conversations[state.active_conv].model_override =
            Some("ollama:qwen2.5-coder:7b".to_string());

        assert_eq!(state.effective_model(), "ollama:qwen2.5-coder:7b");
    }

    #[test]
    fn normalize_model_spec_keeps_prefixed_specs_unchanged() {
        for spec in [
            "claude:opus",
            "claude:sonnet[1m]",
            "codex:gpt-5.5",
            "cursor:auto",
            "cursor:composer-2.5",
            "cursor:claude-4.6-opus-high-thinking",
            "ollama:qwen2.5-coder:7b",
            "local:qwen2.5-coder:14b",
            "deepseek:deepseek-v4-pro",
            "deepseek:deepseek-v4-flash",
        ] {
            assert_eq!(normalize_model_spec(spec), spec);
        }
    }

    #[test]
    fn normalize_model_spec_auto_prefixes_bare_claude_aliases() {
        assert_eq!(normalize_model_spec("opus"), "claude:opus");
        assert_eq!(normalize_model_spec("sonnet"), "claude:sonnet");
        assert_eq!(normalize_model_spec("haiku"), "claude:haiku");
        assert_eq!(normalize_model_spec("opusplan"), "claude:opusplan");
        assert_eq!(normalize_model_spec("fable"), "claude:fable");
        // Dashed legacy aliases collapse to canonical Claude alias.
        assert_eq!(normalize_model_spec("claude-opus"), "claude:opus");
        assert_eq!(normalize_model_spec("claude-sonnet"), "claude:sonnet");
        assert_eq!(normalize_model_spec("claude-haiku"), "claude:haiku");
        assert_eq!(normalize_model_spec("claude-fable"), "claude:fable");
    }

    #[test]
    fn process_slash_command_model_sets_override_and_clears_input() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/model ollama:qwen2.5-coder:7b".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert_eq!(
            state.conversations[state.active_conv]
                .model_override
                .as_deref(),
            Some("ollama:qwen2.5-coder:7b")
        );
        assert!(state.text_input.text.is_empty());
    }

    #[test]
    fn process_slash_command_model_rejects_unknown_deepseek_model() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/model deepseek:deepseek-v4".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert!(
            state.conversations[state.active_conv]
                .model_override
                .is_none()
        );
        let last = state.conversations[state.active_conv]
            .messages
            .last()
            .expect("system error message");
        assert!(last.content.contains("Invalid model spec"));
    }

    #[test]
    fn process_slash_command_model_accepts_deepseek_v4_pro() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/model deepseek:deepseek-v4-pro".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert_eq!(
            state.conversations[state.active_conv]
                .model_override
                .as_deref(),
            Some("deepseek:deepseek-v4-pro")
        );
    }

    #[test]
    fn process_slash_command_model_preserves_cursor_prefix_verbatim() {
        // Regression: `/model cursor:composer-2.5` previously fell through
        // `normalize_model_spec`'s unknown-prefix branch and was stored
        // as `claude:cursor:composer-2.5`. Pass-through now delegates to
        // core's `validate_model_spec` so new providers need no TUI guard.
        let mut state = AgentChatState::new();
        state.text_input.text = "/model cursor:composer-2.5".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert_eq!(
            state.conversations[state.active_conv]
                .model_override
                .as_deref(),
            Some("cursor:composer-2.5")
        );
    }

    #[test]
    fn process_slash_command_rename_with_arg_renames_active_tab() {
        let mut state = AgentChatState::new();
        let original = state.active_conversation().title.clone();
        state.text_input.text = "/rename my refactor branch".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert_eq!(state.active_conversation().title, "my refactor branch");
        assert_ne!(state.active_conversation().title, original);
        assert!(state.text_input.text.is_empty());
        assert!(!state.renaming);
    }

    #[test]
    fn process_slash_command_rename_bare_starts_interactive_rename() {
        let mut state = AgentChatState::new();
        let title = state.active_conversation().title.clone();
        state.text_input.text = "/rename".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert!(state.renaming);
        // Interactive rename pre-fills the input with the current title (same as F2).
        assert_eq!(state.text_input.text, title);
    }

    #[test]
    fn process_slash_command_double_slash_forwards_stripped_to_agent() {
        // `//foo bar` is the explicit "send raw to agent" marker.
        // process_slash_command must return false (so the caller falls
        // through to send_chat_message) AND rewrite the input buffer
        // to the canonical single-slash form. send_chat_message owns
        // add_user_message on the forwarded path, so nothing may be
        // appended to the conversation here.
        let mut state = AgentChatState::new();
        state.text_input.text = "//init please".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(!handled);
        assert_eq!(state.text_input.text, "/init please");
        assert!(state.conversations[state.active_conv].messages.is_empty());
    }

    #[test]
    fn process_slash_command_unknown_single_slash_reports_error() {
        // Single-slash unknown commands must NOT silently leak to the
        // agent — typos like `/modle` should produce a local error.
        let mut state = AgentChatState::new();
        state.text_input.text = "/modle".to_string();
        state.text_input.cursor = state.text_input.text.len();

        let handled = state.process_slash_command();

        assert!(handled);
        assert!(state.text_input.text.is_empty());
        let messages = &state.conversations[state.active_conv].messages;
        assert!(
            messages
                .iter()
                .any(|m| m.role == ChatRole::System && m.content.contains("Unknown command")),
            "expected an Unknown-command system message, got {:?}",
            messages
        );
    }

    #[test]
    fn process_slash_command_returns_false_for_plain_text() {
        let mut state = AgentChatState::new();
        state.text_input.text = "hello world".to_string();

        assert!(!state.process_slash_command());
        assert!(state.conversations[state.active_conv].messages.is_empty());
    }

    #[test]
    fn process_slash_command_workspace_arms_one_shot_flag() {
        let mut state = AgentChatState::new();
        // Default: not armed.
        assert!(!state.conversations[state.active_conv].workspace_wide_next);

        state.text_input.text = "/workspace".to_string();
        state.text_input.cursor = state.text_input.text.len();
        let handled = state.process_slash_command();

        assert!(handled);
        assert!(state.text_input.text.is_empty());
        assert!(state.conversations[state.active_conv].workspace_wide_next);
        // ARMED message landed in the transcript so the user sees the state.
        let messages = &state.conversations[state.active_conv].messages;
        assert!(
            messages
                .iter()
                .any(|m| m.role == ChatRole::System && m.content.contains("ARMED")),
            "expected ARMED system message, got {:?}",
            messages
        );
    }

    #[test]
    fn process_slash_command_workspace_toggles_off_on_second_invocation() {
        // Mirror of /autoapprove: a second invocation before dispatch flips
        // the flag back to off so the user can change their mind without a
        // separate command.
        let mut state = AgentChatState::new();
        state.text_input.text = "/workspace".to_string();
        state.text_input.cursor = state.text_input.text.len();
        state.process_slash_command();
        assert!(state.conversations[state.active_conv].workspace_wide_next);

        state.text_input.text = "/workspace".to_string();
        state.text_input.cursor = state.text_input.text.len();
        let handled = state.process_slash_command();

        assert!(handled);
        assert!(!state.conversations[state.active_conv].workspace_wide_next);
        let last = state.conversations[state.active_conv]
            .messages
            .last()
            .expect("at least one message after toggle-off");
        assert_eq!(last.role, ChatRole::System);
        assert!(
            last.content.contains("cleared"),
            "expected 'cleared' message on toggle-off, got {:?}",
            last.content
        );
    }

    #[test]
    fn process_slash_command_ws_alias_matches_workspace() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/ws".to_string();
        state.text_input.cursor = state.text_input.text.len();
        let handled = state.process_slash_command();

        assert!(handled);
        assert!(state.conversations[state.active_conv].workspace_wide_next);
    }

    #[test]
    fn process_slash_command_lite_arms_one_shot_flag() {
        let mut state = AgentChatState::new();
        assert!(!state.conversations[state.active_conv].lite_next);

        state.text_input.text = "/lite".to_string();
        state.text_input.cursor = state.text_input.text.len();
        let handled = state.process_slash_command();

        assert!(handled);
        assert!(state.text_input.text.is_empty());
        assert!(state.conversations[state.active_conv].lite_next);
        let messages = &state.conversations[state.active_conv].messages;
        assert!(
            messages
                .iter()
                .any(|m| m.role == ChatRole::System && m.content.contains("ARMED")),
            "expected ARMED system message, got {:?}",
            messages
        );
    }

    #[test]
    fn process_slash_command_lite_toggles_off_on_second_invocation() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/lite".to_string();
        state.text_input.cursor = state.text_input.text.len();
        state.process_slash_command();
        assert!(state.conversations[state.active_conv].lite_next);

        state.text_input.text = "/lite".to_string();
        state.text_input.cursor = state.text_input.text.len();
        let handled = state.process_slash_command();

        assert!(handled);
        assert!(!state.conversations[state.active_conv].lite_next);
        let last = state.conversations[state.active_conv]
            .messages
            .last()
            .expect("at least one message after toggle-off");
        assert_eq!(last.role, ChatRole::System);
        assert!(
            last.content.contains("cleared"),
            "expected 'cleared' message on toggle-off, got {:?}",
            last.content
        );
    }

    #[test]
    fn process_slash_command_minimal_alias_matches_lite() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/minimal".to_string();
        state.text_input.cursor = state.text_input.text.len();
        let handled = state.process_slash_command();

        assert!(handled);
        assert!(state.conversations[state.active_conv].lite_next);
    }

    #[test]
    fn reset_conversation_suppresses_transcript_inlining_until_next_session() {
        let mut state = AgentChatState::new();
        // Pre-condition: a fresh conversation defaults to Auto so the
        // very first dispatch (when there's no transcript anyway) and
        // any rehydrate-from-disk case retain the historical behaviour.
        assert_eq!(
            state.conversations[state.active_conv].transcript_inline_mode,
            TranscriptInlineMode::Auto
        );

        // Simulate a few prior turns so context_messages() has something
        // to inline.
        state.add_user_message("first user msg");
        let conv_id = state.active_conversation_id().to_string();
        state.finalize_message_to(&conv_id, "assistant", "first assistant reply");

        state.text_input.text = "/reset".to_string();
        assert!(state.process_slash_command());

        // Post-/reset: transcript inlining is suppressed even though
        // session_ledger was cleared (so is_first_turn would be true again).
        assert_eq!(
            state.conversations[state.active_conv].transcript_inline_mode,
            TranscriptInlineMode::Suppress
        );
        // Visible chat history is preserved in the panel.
        assert!(
            state.conversations[state.active_conv]
                .messages
                .iter()
                .any(|m| m.role == ChatRole::User && m.content == "first user msg"),
            "panel transcript must survive /reset"
        );
        // Session handle was dropped so bootstrap context will fire.
        assert!(
            state.conversations[state.active_conv]
                .claude_session_id
                .is_none()
        );
        assert!(
            state.conversations[state.active_conv]
                .session_ledger
                .is_none()
        );
    }

    #[test]
    fn reset_message_includes_bootstrap_projection() {
        let mut state = AgentChatState::new();
        state.agent_settings.graph_budget_tokens = 8_000;
        state.agent_settings.model = "cursor:composer".to_string();

        // PUSH→PULL Phase 1: the default first turn projects the thin anchor
        // (default 1200), not the full graph budget. cursor:composer is a
        // strong, tool-capable provider, so the auto first turn pulls bodies
        // on demand rather than pushing the full outline.
        assert_eq!(state.agent_settings.anchor_budget_tokens, 1_200);

        state.text_input.text = "/reset".to_string();
        assert!(state.process_slash_command());

        let last = state.conversations[state.active_conv]
            .messages
            .last()
            .expect("reset emits a system message");
        assert_eq!(last.role, ChatRole::System);
        assert!(
            last.content.contains("Projected bootstrap injection"),
            "reset should explain upcoming bootstrap: {:?}",
            last.content
        );
        assert!(
            last.content.contains("outline: ≈1200 tok"),
            "default first turn should project the thin anchor, not the full push: {:?}",
            last.content
        );
        assert!(
            last.content.contains("/lite"),
            "should mention /lite escape hatch: {:?}",
            last.content
        );
        assert!(
            last.content.contains("Cursor composite estimate"),
            "cursor model should show composite breakdown: {:?}",
            last.content
        );
    }

    #[test]
    fn reset_message_with_lite_shows_topology_only_projection() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/lite".to_string();
        assert!(state.process_slash_command());

        state.text_input.text = "/reset".to_string();
        assert!(state.process_slash_command());

        let last = state.conversations[state.active_conv]
            .messages
            .last()
            .expect("reset emits a system message");
        assert!(
            last.content.contains("topology: ≈600 tok"),
            "armed /lite should project topology-only bootstrap: {:?}",
            last.content
        );
        assert!(
            !last.content.contains("outline:"),
            "armed /lite should not project outline layer: {:?}",
            last.content
        );
    }

    #[test]
    fn reset_preserves_armed_lite_one_shot() {
        // Regression: `/lite` is a one-shot bootstrap arm, orthogonal to
        // `/reset` session-state clearing. Arming `/lite` then `/reset`
        // must NOT disarm it — otherwise the post-reset first turn
        // silently re-injects the full bootstrap.
        let mut state = AgentChatState::new();

        state.text_input.text = "/lite".to_string();
        state.text_input.cursor = state.text_input.text.len();
        assert!(state.process_slash_command());
        assert!(state.conversations[state.active_conv].lite_next);

        state.text_input.text = "/reset".to_string();
        state.text_input.cursor = state.text_input.text.len();
        assert!(state.process_slash_command());

        assert!(
            state.conversations[state.active_conv].lite_next,
            "an armed /lite must survive /reset so the post-reset bootstrap stays minimal"
        );
        // /reset still drops session state regardless.
        assert!(
            state.conversations[state.active_conv]
                .session_ledger
                .is_none()
        );
    }

    #[test]
    fn reset_preserves_armed_no_inject_and_inject_arms() {
        // Same orthogonality contract as `/lite`: `/no-inject` and
        // `/inject <layer>` are forward-looking one-shot arms for the next
        // dispatch, not past session state, so `/reset` leaves them alone.
        let mut state = AgentChatState::new();

        state.text_input.text = "/no-inject".to_string();
        state.text_input.cursor = state.text_input.text.len();
        assert!(state.process_slash_command());
        assert!(state.conversations[state.active_conv].no_inject_next);

        state.text_input.text = "/reset".to_string();
        state.text_input.cursor = state.text_input.text.len();
        assert!(state.process_slash_command());
        assert!(
            state.conversations[state.active_conv].no_inject_next,
            "an armed /no-inject must survive /reset"
        );

        state.text_input.text = "/inject memory".to_string();
        state.text_input.cursor = state.text_input.text.len();
        assert!(state.process_slash_command());
        assert!(
            state.conversations[state.active_conv]
                .inject_arms_next
                .memory
        );

        state.text_input.text = "/reset".to_string();
        state.text_input.cursor = state.text_input.text.len();
        assert!(state.process_slash_command());
        assert!(
            state.conversations[state.active_conv]
                .inject_arms_next
                .memory,
            "an armed /inject layer must survive /reset"
        );
    }

    #[test]
    fn update_autocomplete_recognises_model_argument() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/model deepseek:deep".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();

        assert!(state.autocomplete.active);
        assert_eq!(state.autocomplete.mode, AutocompleteMode::ModelSpec);
        assert_eq!(state.autocomplete.query, "deepseek:deep");
        assert_eq!(state.autocomplete.at_pos, "/model ".len());
    }

    #[test]
    fn update_autocomplete_model_accept_replaces_argument_only() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/model deepseek:deep".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();
        state.autocomplete.matches = vec!["deepseek:deepseek-v4-pro".to_string()];
        state.autocomplete.selected = 0;
        state.accept_autocomplete();

        assert_eq!(state.text_input.text, "/model deepseek:deepseek-v4-pro");
        assert!(!state.autocomplete.active);
    }

    #[test]
    fn update_autocomplete_model_accept_midspec_replaces_whole_token() {
        // Regression: cursor sits right after `/model ` while an old spec
        // (`cursor:composer-2.5`) still trails to the right. Accepting a new
        // provider must replace the whole spec token, not concatenate after
        // it — the old behaviour produced `claude:cursor:composer-2.5`.
        let mut state = AgentChatState::new();
        state.text_input.text = "/model cursor:composer-2.5".to_string();
        // Place the cursor immediately after "/model " (char index 7).
        state.text_input.cursor = "/model ".chars().count();
        state.update_autocomplete();

        assert_eq!(state.autocomplete.mode, AutocompleteMode::ModelSpec);
        assert_eq!(state.autocomplete.query, "");
        state.autocomplete.matches = vec!["claude:".to_string()];
        state.autocomplete.selected = 0;
        state.accept_autocomplete();

        assert_eq!(state.text_input.text, "/model claude:");
        assert!(!state.autocomplete.active);
    }

    #[test]
    fn update_autocomplete_recognises_attach_argument() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/attach pat".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();

        assert!(state.autocomplete.active);
        assert_eq!(state.autocomplete.mode, AutocompleteMode::AttachPath);
        assert_eq!(state.autocomplete.query, "pat");
        assert_eq!(state.autocomplete.at_pos, "/attach ".len());
    }

    #[test]
    fn update_autocomplete_attach_accept_replaces_argument_only() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/attach scre".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();
        state.autocomplete.matches = vec!["/tmp/screenshot.png".to_string()];
        state.autocomplete.selected = 0;
        state.accept_autocomplete();

        assert_eq!(state.text_input.text, "/attach /tmp/screenshot.png");
        assert!(!state.autocomplete.active);
    }

    #[test]
    fn update_autocomplete_recognises_detach_argument() {
        let mut state = AgentChatState::new();
        state.add_attachment(PathBuf::from("/tmp/shot.png"), AttachmentKind::Image);
        state.text_input.text = "/detach sh".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();
        state.update_detach_autocomplete_matches();

        assert!(state.autocomplete.active);
        assert_eq!(state.autocomplete.mode, AutocompleteMode::DetachName);
        assert_eq!(state.autocomplete.query, "sh");
        assert_eq!(state.autocomplete.matches, vec!["shot.png".to_string()]);

        state.autocomplete.query.clear();
        state.update_detach_autocomplete_matches();
        assert_eq!(
            state.autocomplete.matches,
            vec!["all".to_string(), "shot.png".to_string()]
        );
    }

    #[test]
    fn detach_autocomplete_accept_fills_name() {
        let mut state = AgentChatState::new();
        state.text_input.text = "/detach sh".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();
        state.autocomplete.matches = vec!["shot.png".to_string()];
        state.autocomplete.selected = 0;
        state.accept_autocomplete();
        assert_eq!(state.text_input.text, "/detach shot.png");
    }

    #[test]
    fn remove_attachment_at_removes_by_index() {
        let mut state = AgentChatState::new();
        state.add_attachment(PathBuf::from("/a/one.txt"), AttachmentKind::Text);
        state.add_attachment(PathBuf::from("/a/two.txt"), AttachmentKind::Text);
        assert_eq!(state.remove_attachment_at(0).as_deref(), Some("one.txt"));
        assert_eq!(state.attachments.len(), 1);
        assert_eq!(state.attachments[0].display_name, "two.txt");
    }

    #[test]
    fn update_autocomplete_still_handles_at_references() {
        let mut state = AgentChatState::new();
        state.text_input.text = "check @src/lib".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();

        assert!(state.autocomplete.active);
        assert_eq!(state.autocomplete.mode, AutocompleteMode::FileRef);
        assert_eq!(state.autocomplete.query, "src/lib");
        assert_eq!(state.autocomplete.at_pos, "check ".len());
    }

    #[test]
    fn update_autocomplete_at_accept_keeps_at_prefix_and_trailing_space() {
        let mut state = AgentChatState::new();
        state.text_input.text = "@src/li".to_string();
        state.text_input.cursor = state.text_input.text.chars().count();
        state.update_autocomplete();
        state.autocomplete.matches = vec!["src/lib.rs".to_string()];
        state.accept_autocomplete();

        assert_eq!(state.text_input.text, "@src/lib.rs ");
    }

    #[test]
    fn finalize_message_to_rewrites_assistant_even_with_trailing_system() {
        let mut state = AgentChatState::new();
        let conv_id = state.active_conversation_id().to_string();
        state.append_stream_chunk_to(
            &conv_id,
            "Visible reply.\n\n<turn_annotations>\n{\"v\":1,\"flags\":[]}\n</turn_annotations>",
        );
        state.add_system_message("⚠ Disk drifted on foo.rs — skipping revert.");

        let stripped = gaviero_core::memory::parse_and_strip(
            "Visible reply.\n\n<turn_annotations>\n{\"v\":1,\"flags\":[]}\n</turn_annotations>",
        )
        .stripped;
        state.finalize_message_to(&conv_id, "assistant", &stripped);

        let msgs = &state.conversations[state.active_conv].messages;
        let assistant = msgs
            .iter()
            .rev()
            .find(|m| m.role == ChatRole::Assistant)
            .expect("assistant message");
        assert_eq!(assistant.content.trim_end(), "Visible reply.");
        assert!(
            !assistant.content.contains("<turn_annotations>"),
            "sidecar must be stripped from stored assistant content"
        );
        assert!(
            msgs.iter().any(|m| m.role == ChatRole::System),
            "interleaved system message must remain"
        );
        // Must not push a second assistant bubble.
        assert_eq!(
            msgs.iter()
                .filter(|m| m.role == ChatRole::Assistant)
                .count(),
            1
        );
    }

    #[test]
    fn filter_assistant_for_display_strips_turn_annotations() {
        let raw = "Answer text.\n\n<turn_annotations>\n{\"v\":1,\"flags\":[]}\n</turn_annotations>";
        let shown = filter_assistant_for_display(raw);
        assert_eq!(shown.trim_end(), "Answer text.");
        assert!(!shown.contains("<turn_annotations>"));
    }

    #[test]
    fn count_words_splits_on_whitespace() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   "), 0);
        assert_eq!(count_words("hello"), 1);
        assert_eq!(count_words("hello world"), 2);
        assert_eq!(count_words("  hello   world  "), 2);
        assert_eq!(count_words("foo\tbar\nbaz"), 3);
    }

    #[test]
    fn words_to_tokens_applies_one_point_three_multiplier() {
        assert_eq!(words_to_tokens(0), 0);
        assert_eq!(words_to_tokens(10), 13);
        assert_eq!(words_to_tokens(100), 130);
        // 7 × 1.3 = 9.1 → integer floor 9
        assert_eq!(words_to_tokens(7), 9);
    }

    // ── Plan A A3 reducer-boundary tests ────────────────────────────

    fn ask_permission_with(
        respond: tokio::sync::oneshot::Sender<gaviero_core::observer::PermissionDecision>,
    ) -> PendingPermission {
        PendingPermission::new(
            "AskUserQuestion".to_string(),
            "The agent is asking".to_string(),
            serde_json::json!({
                "questions": [
                    {
                        "question": "Which serializer?",
                        "header": "Serializer",
                        "multiSelect": false,
                        "options": [
                            { "label": "serde_json", "description": "" },
                            { "label": "simd-json", "description": "" }
                        ]
                    },
                    {
                        "question": "Which suites?",
                        "header": "Tests",
                        "multiSelect": true,
                        "options": [
                            { "label": "unit", "description": "" },
                            { "label": "integration", "description": "" },
                            { "label": "fuzz", "description": "" }
                        ]
                    }
                ]
            }),
            respond,
        )
    }

    #[test]
    fn message_seq_is_monotonic_across_compact_and_reset() {
        let mut state = AgentChatState::new();
        for i in 0..8 {
            state.add_user_message(&format!("msg {i}"));
        }
        let before: Vec<u64> = state
            .active_conversation()
            .messages
            .iter()
            .map(|m| m.seq)
            .collect();
        assert_eq!(before, (1..=8).collect::<Vec<u64>>());

        // Compact keeps the last 3; kept messages retain their seq and the
        // summary takes a NEW seq (§2.6).
        state.text_input.text = "/compact 3".to_string();
        state.process_slash_command();
        let conv = state.active_conversation();
        let seqs: Vec<u64> = conv.messages.iter().map(|m| m.seq).collect();
        // Layout: [summary(10), kept 7, 8, /compact echo 9, confirmation 11]
        assert_eq!(seqs[0], 10, "summary is a new message with a new seq");
        assert_eq!(&seqs[1..4], &[7, 8, 9], "kept messages retain their seq");
        assert_eq!(seqs[4], 11, "the confirmation takes the next fresh seq");

        // Reset does not rewind the counter.
        let idx = state.active_conv;
        state.reset_conversation_at(idx);
        state.add_user_message("after reset");
        let last = state.active_conversation().messages.last().unwrap().seq;
        assert!(last > 10, "seq keeps growing after /reset, got {last}");
    }

    #[test]
    fn remote_answer_targets_non_active_conversation_by_request_id() {
        let mut state = AgentChatState::new();
        let background_id = state.active_conversation_id().to_string();
        let (tx, mut rx) = tokio::sync::oneshot::channel();
        state.set_pending_permission(&background_id, ask_permission_with(tx));
        let request_id = state
            .active_conversation()
            .pending_permission
            .as_ref()
            .unwrap()
            .request_id
            .clone();

        // Desktop switches to a NEW tab; the request stays on the old one.
        state.new_conversation();
        assert_ne!(state.active_conversation_id(), background_id);

        let idx = state
            .find_permission_conv(&request_id)
            .expect("request findable by id");
        let info = state
            .respond_permission_at(idx, true, Some(&[vec![1], vec![0, 2]]), None)
            .expect("valid answers accepted");
        assert_eq!(
            info.conv_id, background_id,
            "answered the conversation OWNING the request, not the active tab"
        );
        assert!(info.allowed);

        // The rebuilt input went through the same answers_map path the
        // desktop uses: labels, not indices.
        let decision = rx.try_recv().expect("decision delivered");
        match decision {
            gaviero_core::observer::PermissionDecision::Allow {
                updated_input: Some(v),
            } => {
                let answers = v.get("answers").expect("answers inserted");
                assert_eq!(answers.get("Which serializer?").unwrap(), "simd-json");
                assert_eq!(answers.get("Which suites?").unwrap(), "unit, fuzz");
            }
            other => panic!("expected Allow with updated_input, got {other:?}"),
        }
    }

    #[test]
    fn permission_answer_validation_rejects_without_consuming() {
        let mut state = AgentChatState::new();
        let conv_id = state.active_conversation_id().to_string();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.set_pending_permission(&conv_id, ask_permission_with(tx));
        let idx = state.active_conv;

        // answers on a wrong shape: count mismatch, out-of-range index,
        // multi-select violation — all rejected, request stays parked.
        for bad in [
            vec![vec![0u32]],
            vec![vec![9], vec![0]],
            vec![vec![0, 1], vec![0]],
        ] {
            let err = state.respond_permission_at(idx, true, Some(&bad), None);
            assert!(
                matches!(err, Err(PermissionAnswerError::Invalid(_))),
                "{bad:?}"
            );
            assert!(state.active_conversation().pending_permission.is_some());
        }

        // First valid answer wins; the second finds nothing (stale_request).
        state
            .respond_permission_at(idx, true, Some(&[vec![0], vec![1]]), None)
            .unwrap();
        let second = state.respond_permission_at(idx, true, Some(&[vec![0], vec![1]]), None);
        assert!(matches!(second, Err(PermissionAnswerError::NoPending)));
    }

    #[test]
    fn answers_on_plain_permission_are_rejected() {
        let mut state = AgentChatState::new();
        let conv_id = state.active_conversation_id().to_string();
        let (tx, _rx) = tokio::sync::oneshot::channel();
        state.set_pending_permission(
            &conv_id,
            PendingPermission::new(
                "Bash".to_string(),
                "Run: ls".to_string(),
                serde_json::json!({"command": "ls"}),
                tx,
            ),
        );
        let idx = state.active_conv;
        let err = state.respond_permission_at(idx, true, Some(&[vec![0]]), None);
        assert!(
            matches!(err, Err(PermissionAnswerError::Invalid(_))),
            "a permission decision must not be able to smuggle answers into a non-ask tool"
        );
        assert!(state.active_conversation().pending_permission.is_some());
    }

    #[test]
    fn slash_reducer_mutates_target_conversation_and_bumps_revision() {
        let mut state = AgentChatState::new();
        let target = 0;
        state.new_conversation(); // active becomes 1
        assert_eq!(state.active_conv, 1);

        let rev_before = state.conversations[target].conv_revision;
        assert!(state.apply_slash_line(target, "/model claude:opus", SlashOrigin::Remote));
        assert_eq!(
            state.conversations[target].model_override.as_deref(),
            Some("claude:opus"),
            "the TARGET conversation got the override"
        );
        assert!(
            state.conversations[1].model_override.is_none(),
            "the active tab is untouched"
        );
        assert!(
            state.conversations[target].conv_revision > rev_before,
            "summary change bumped conv_revision"
        );

        // Remote bare /rename must not start the desktop interactive
        // rename (focus affordance).
        assert!(state.apply_slash_line(target, "/rename", SlashOrigin::Remote));
        assert!(!state.renaming);
    }

    #[test]
    fn rename_and_reset_bump_the_entity_revision_exactly_once() {
        let mut state = AgentChatState::new();
        let idx = state.active_conv;
        let base = state.conversations[idx].conv_revision;
        state.apply_slash_line(idx, "/rename Parser work", SlashOrigin::Desktop);
        assert_eq!(state.conversations[idx].conv_revision, base + 1);
        state.reset_conversation_at(idx);
        assert_eq!(state.conversations[idx].conv_revision, base + 2);
    }

    /// Push a fully-formed assistant message. `finalize_message` only
    /// rewrites an existing trailing assistant message (set up by the
    /// streaming pipeline), so tests that need a synthetic transcript
    /// push directly.
    fn push_assistant(state: &mut AgentChatState, content: &str, tool_calls: Vec<String>) {
        state.conversations[state.active_conv].push_message(
            ChatRole::Assistant,
            content.to_string(),
            tool_calls,
        );
    }

    #[test]
    fn count_transcript_words_splits_input_and_output_by_role() {
        let mut state = AgentChatState::new();
        state.add_user_message("hello there friend");
        push_assistant(&mut state, "ok done", Vec::new());
        // System messages must not contribute (panel chatter, not re-sent).
        state.add_system_message("ignore me");

        let (input, output) = state.count_transcript_words();
        assert_eq!(input, 3);
        assert_eq!(output, 2);
    }

    #[test]
    fn count_transcript_words_ignores_orphan_tool_calls_vec() {
        let mut state = AgentChatState::new();
        state.add_user_message("edit the file");
        // Synthetic push without inline `[tool]` marker — only `content` counts.
        push_assistant(&mut state, "done it", vec!["Write src/main.rs".to_string()]);

        let (input, output) = state.count_transcript_words();
        assert_eq!(input, 3);
        assert_eq!(output, 2);
    }

    #[test]
    fn count_transcript_words_returns_zero_when_inline_suppressed() {
        let mut state = AgentChatState::new();
        state.add_user_message("first user msg");
        push_assistant(&mut state, "first assistant reply", Vec::new());
        state.reset_conversation_at(state.active_conv);

        // Visible transcript stays, but after /reset it isn't re-inlined.
        assert!(
            !state.conversations[state.active_conv].messages.is_empty(),
            "panel transcript should survive /reset"
        );
        let (input, output) = state.count_transcript_words();
        assert_eq!(input, 0);
        assert_eq!(output, 0);
    }

    #[test]
    fn context_pressure_composite_includes_transcript_bootstrap_and_hidden() {
        let mut state = AgentChatState::new();
        state.add_user_message("one two three");
        push_assistant(&mut state, "four five", Vec::new());
        state.conversations[state.active_conv].last_bootstrap_tokens = 12_000;
        state.conversations[state.active_conv].last_bootstrap_arms =
            gaviero_core::context_planner::BootstrapArms::all();

        let ctx = state.fallback_bootstrap_estimate_context();
        let p = state.context_pressure(&ctx);
        assert_eq!(p.source, ContextBarSource::CompositeEstimate);
        let expected = words_to_tokens(5)
            .saturating_add(12_000)
            .saturating_add(hidden_provider_overhead_tokens("claude:sonnet"));
        assert_eq!(p.tokens, expected);
    }

    #[test]
    fn context_pressure_uses_provider_prefix_when_reported() {
        let mut state = AgentChatState::new();
        state.add_user_message("a b c d e");
        state.conversations[state.active_conv].last_token_usage =
            Some(gaviero_core::acp::protocol::TokenUsage {
                input_tokens: 500,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 87_500,
                output_tokens: 42,
            });

        let ctx = state.fallback_bootstrap_estimate_context();
        let p = state.context_pressure(&ctx);
        assert_eq!(p.source, ContextBarSource::ProviderPrefix);
        assert_eq!(p.tokens, 88_000);
        assert!(!p.is_approximate());
    }

    #[test]
    fn context_pressure_drops_when_lite_armed() {
        let mut state = AgentChatState::new();
        let ctx = state.fallback_bootstrap_estimate_context();
        let full = state.context_pressure(&ctx).bootstrap_tokens;

        state.conversations[state.active_conv].lite_next = true;
        let lite = state.context_pressure(&ctx).bootstrap_tokens;

        assert!(full > lite);
        assert_eq!(lite, 600);
    }

    #[test]
    fn context_pressure_holds_measured_bootstrap_while_streaming() {
        let mut state = AgentChatState::new();
        let ctx = state.fallback_bootstrap_estimate_context();
        state.conversations[state.active_conv].last_bootstrap_tokens = 9_500;
        state.conversations[state.active_conv].last_bootstrap_arms =
            gaviero_core::context_planner::BootstrapArms::all();
        state.conversations[state.active_conv].is_streaming = true;

        assert_eq!(state.context_pressure(&ctx).bootstrap_tokens, 9_500);
    }

    #[test]
    fn transcript_estimate_includes_draft_input() {
        let mut state = AgentChatState::new();
        state.text_input.text = "hello draft message".to_string();
        let with_draft = state.transcript_token_estimate();
        state.text_input.text.clear();
        let without = state.transcript_token_estimate();
        assert!(with_draft > without);
    }

    #[test]
    fn context_pressure_rises_when_inject_memory_armed_on_follow_up() {
        use gaviero_core::context_planner::{
            ModelSpec, PlannerFingerprint, RuntimeConfig, SessionLedger, build_provider_profile,
        };

        let mut state = AgentChatState::new();
        let profile = build_provider_profile(
            &ModelSpec::parse("claude:sonnet"),
            &RuntimeConfig::default(),
        );
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        ledger.record_turn_dispatched();
        state.conversations[state.active_conv].session_ledger = Some(ledger);

        let ctx = state.fallback_bootstrap_estimate_context();
        assert_eq!(state.context_pressure(&ctx).bootstrap_tokens, 0);

        state.conversations[state.active_conv].inject_arms_next =
            gaviero_core::context_planner::BootstrapArms {
                memory: true,
                explicit: true,
                ..gaviero_core::context_planner::BootstrapArms::none()
            };
        assert_eq!(state.context_pressure(&ctx).bootstrap_tokens, 1_000);
    }

    #[test]
    fn count_transcript_words_no_double_count_tool_calls_in_content() {
        let mut state = AgentChatState::new();
        state.add_user_message("edit the file");
        push_assistant(
            &mut state,
            "done it\n[Write src/main.rs]",
            vec!["Write src/main.rs".to_string()],
        );

        let (input, output) = state.count_transcript_words();
        assert_eq!(input, 3);
        // Content only: "done it" + "[Write src/main.rs]" → 4 words (not 6).
        assert_eq!(output, 4);
    }

    #[test]
    fn input_layout_widths_account_for_prompt_and_border() {
        let mut state = AgentChatState::new();
        // panel content 41 → inner 40 after left border; prompt "> " is 2 chars.
        let (first, full) = state.input_layout_widths(41);
        assert_eq!(full, 40);
        assert_eq!(first, 38);

        state.conversations[state.active_conv].auto_approve = true;
        let (first_auto, full_auto) = state.input_layout_widths(41);
        assert_eq!(full_auto, 40);
        assert_eq!(first_auto, 23); // "[auto-approve] > " is 17 chars
    }

    #[test]
    fn build_visual_lines_wraps_long_single_line() {
        let mut state = AgentChatState::new();
        state.text_input.text = "abcdefghijklmnopqrstuvwxyz".to_string();
        let lines = state.build_visual_lines(10, 20);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], (0, 10));
        assert_eq!(lines[1], (10, 16));
    }

    #[test]
    fn auto_input_height_grows_for_wrapped_text_without_newlines() {
        let mut state = AgentChatState::new();
        // 80 chars, inner width 20 → first line 18 ("> " prompt), then full 20s.
        // 80 = 18 + 20 + 20 + 20 + 2 → 5 visual lines → height 5.
        state.text_input.text = "a".repeat(80);
        assert_eq!(state.auto_input_height(20, 10), 5);
        // Cap at max_input.
        assert_eq!(state.auto_input_height(20, 3), 3);
        // Newline-only growth previously: 0 newlines → height 3; wrapping must exceed that.
        assert!(state.auto_input_height(20, 10) > 3);
    }

    #[test]
    fn scroll_input_by_visual_lines_moves_cursor_through_overflow() {
        let mut state = AgentChatState::new();
        state.text_input.text = "abcdefghijklmnopqrstuvwxyz".to_string();
        state.text_input.cursor = state.text_input.char_count();
        // Viewport shorter than wrapped lines.
        state.input_area_cache = Some(Rect {
            x: 0,
            y: 0,
            width: 15,
            height: 1,
        });
        let panel_w = 15;
        assert!(state.input_overflows_viewport(panel_w));
        assert!(state.scroll_input_by_visual_lines(-1, panel_w));
        assert_eq!(state.text_input.cursor, 12);
        // At the top — further up does not move.
        state.text_input.cursor = 0;
        assert!(!state.scroll_input_by_visual_lines(-1, panel_w));
    }

    #[test]
    fn screen_to_input_char_maps_click_into_wrapped_prompt() {
        let mut state = AgentChatState::new();
        // "> " prompt (2) + inner width 20 → first line holds 18 chars.
        state.text_input.text = "abcdefghijklmnopqrstuvwxyz".to_string();
        state.text_input.cursor = 0;
        state.input_area_cache = Some(Rect {
            x: 10,
            y: 20,
            width: 20,
            height: 3,
        });

        // First visual line starts after the "> " prompt at x=12.
        assert_eq!(state.screen_to_input_char(12, 20), Some(0));
        assert_eq!(state.screen_to_input_char(14, 20), Some(2));
        // Second visual line starts at area.x (no prompt).
        assert_eq!(state.screen_to_input_char(10, 21), Some(18));
        assert_eq!(state.screen_to_input_char(12, 21), Some(20));
        // Outside the input area.
        assert_eq!(state.screen_to_input_char(10, 19), None);
    }

    #[test]
    fn input_mouse_selection_drag_selects_and_click_collapses() {
        let mut state = AgentChatState::new();
        state.text_input.text = "hello world".to_string();
        state.start_input_mouse_selection(0);
        assert!(state.input_dragging);
        state.extend_input_mouse_selection(5);
        assert_eq!(state.text_input.selection_range(), Some((0, 5)));
        assert_eq!(state.text_input.selected_text(), Some("hello"));
        state.end_input_mouse_selection();
        assert!(!state.input_dragging);
        assert!(state.text_input.has_selection());

        // Plain click (no drag) clears selection.
        state.start_input_mouse_selection(3);
        state.end_input_mouse_selection();
        assert!(!state.text_input.has_selection());
        assert_eq!(state.text_input.cursor, 3);
    }

    #[test]
    fn cursor_up_in_input_moves_within_wrapped_text_before_history() {
        let mut state = AgentChatState::new();
        state.text_input.text = "abcdefghijklmnopqrstuvwxyz".to_string();
        state.text_input.cursor = state.text_input.char_count();
        state.add_user_message("prior");
        let panel_w = 15; // inner 14, first line 12 → two visual lines (12 + 14)

        assert!(state.input_has_multiple_visual_lines(panel_w));
        assert!(state.cursor_up_in_input(panel_w));
        assert_eq!(state.text_input.cursor, 12);
        assert!(state.history_index.is_none());
    }

    #[test]
    fn cursor_up_at_first_visual_line_does_not_move() {
        let mut state = AgentChatState::new();
        state.text_input.text = "abcdefghijklmnopqrstuvwxyz".to_string();
        state.text_input.cursor = 5;

        assert!(!state.cursor_up_in_input(15));
    }

    #[test]
    fn cursor_down_in_input_moves_within_multiline_text() {
        let mut state = AgentChatState::new();
        state.text_input.text = "line one\nline two".to_string();
        state.text_input.cursor = 0;

        assert!(state.cursor_down_in_input(41));
        assert_eq!(state.text_input.cursor, 9);
    }

    #[test]
    fn select_up_in_input_extends_selection_across_visual_lines() {
        let mut state = AgentChatState::new();
        state.text_input.text = "abcdefghijklmnopqrstuvwxyz".to_string();
        state.text_input.cursor = 15;

        assert!(state.select_up_in_input(15));
        assert_eq!(state.text_input.cursor, 3);
        assert_eq!(state.text_input.sel_anchor, Some(15));
    }

    fn ask_permission() -> PendingPermission {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        PendingPermission::new(
            "AskUserQuestion".to_string(),
            "ask".to_string(),
            serde_json::json!({
                "questions": [{
                    "question": "Which retry policy should the dispatcher use when the upstream service keeps answering 503 for several minutes?",
                    "header": "Retries",
                    "multiSelect": false,
                    "options": [
                        {
                            "label": "Exponential backoff",
                            "description": "Wait longer after every failure, capped at one minute per attempt."
                        },
                        { "label": "Fail fast", "description": "Surface the error immediately." }
                    ]
                }]
            }),
            tx,
        )
    }

    fn joined_rows(rows: &[(String, bool)]) -> String {
        rows.iter()
            .map(|(text, _)| text.trim())
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn ask_overlay_rows_wrap_question_and_options_to_width() {
        let perm = ask_permission();
        let width = 40;
        let rows = perm.overlay_rows(width);

        for (text, _) in &rows {
            assert!(
                UnicodeWidthStr::width(text.as_str()) <= width,
                "row wider than overlay: {text:?}"
            );
        }
        let joined = joined_rows(&rows);
        assert!(
            joined.contains("answering 503 for several minutes?"),
            "question truncated: {joined}"
        );
        assert!(
            joined.contains("capped at one minute per attempt."),
            "option description truncated: {joined}"
        );
    }

    #[test]
    fn ask_overlay_rows_flag_selected_option() {
        let mut perm = ask_permission();
        perm.ask.as_mut().unwrap().toggle_option(1);
        let rows = perm.overlay_rows(60);

        let selected: Vec<&str> = rows
            .iter()
            .filter(|(_, marked)| *marked)
            .map(|(text, _)| text.as_str())
            .collect();
        assert!(!selected.is_empty(), "no row flagged as selected");
        assert!(selected[0].contains("● 2. Fail fast"), "got {selected:?}");
    }

    #[test]
    fn plain_permission_description_wraps_instead_of_truncating() {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        let cmd = "cargo test -p gaviero-tui --all-features -- --nocapture overlay_rows_wrap";
        let perm = PendingPermission::new(
            "Bash".to_string(),
            cmd.to_string(),
            serde_json::json!({ "command": cmd }),
            tx,
        );

        let rows = perm.overlay_rows(30);
        assert!(rows.len() > 1, "description was not wrapped: {rows:?}");
        for (text, _) in &rows {
            assert!(
                UnicodeWidthStr::width(text.as_str()) <= 30,
                "row wider than overlay: {text:?}"
            );
        }
        assert!(joined_rows(&rows).contains("--nocapture"));
    }

    #[test]
    fn pending_overlay_height_covers_wrapped_rows() {
        let mut state = AgentChatState::new();
        assert!(state.pending_overlay_height(40).is_none());

        let conv_id = state.active_conversation_id().to_string();
        state.set_pending_permission(&conv_id, ask_permission());

        let narrow = state.pending_overlay_height(40).unwrap();
        let wide = state.pending_overlay_height(100).unwrap();
        let rows = state.conversations[state.active_conv]
            .pending_permission
            .as_ref()
            .unwrap()
            .overlay_rows(40)
            .len() as u16;

        // header + body + hint row
        assert_eq!(narrow, rows + 2);
        assert!(narrow > wide, "narrow={narrow} wide={wide}");
    }

    fn row_text(buf: &RataBuf, y: u16, width: u16) -> String {
        (0..width).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn render_overlay_body_paints_wrapped_rows_and_reports_hidden() {
        let perm = ask_permission();
        // header + 4 body rows + hint
        let area = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 6,
        };
        let mut buf = RataBuf::empty(area);
        let rows = perm.overlay_rows(area.width as usize);

        let hidden = AgentChatState::render_overlay_body(
            area,
            &mut buf,
            &rows,
            0,
            Style::default(),
            Style::default(),
        );
        assert_eq!(hidden, rows.len() - 4);

        for (i, expected) in rows.iter().take(4).enumerate() {
            let painted = row_text(&buf, 1 + i as u16, area.width);
            assert_eq!(painted.trim_end(), expected.0.trim_end());
        }

        // Scrolling past the end clamps to the last window, so the tail of the
        // question is reachable instead of lost.
        let hidden = AgentChatState::render_overlay_body(
            area,
            &mut buf,
            &rows,
            rows.len(),
            Style::default(),
            Style::default(),
        );
        assert_eq!(hidden, 0);
        assert_eq!(
            row_text(&buf, 4, area.width).trim_end(),
            rows.last().unwrap().0.trim_end()
        );
    }

    #[test]
    fn scroll_pending_permission_clamps_to_row_count() {
        let mut state = AgentChatState::new();
        let conv_id = state.active_conversation_id().to_string();
        state.set_pending_permission(&conv_id, ask_permission());
        let rows = state.conversations[state.active_conv]
            .pending_permission
            .as_ref()
            .unwrap()
            .overlay_rows(40)
            .len();

        let scroll_of = |state: &AgentChatState| {
            state
                .active_conversation()
                .pending_permission
                .as_ref()
                .unwrap()
                .scroll
        };

        state.scroll_pending_permission(1_000, 40);
        assert_eq!(scroll_of(&state), rows - 1);

        state.scroll_pending_permission(-1_000, 40);
        assert_eq!(scroll_of(&state), 0);
    }
}
