//! The on-disk record model for `<workspace>/.gaviero/history/turns.ndjson`.
//!
//! One JSON object per line, tagged by `kind`, schema-versioned per line
//! (`v`, never a header, so the file stays append-only and greppable).
//! Every record carries `ts` (RFC3339 UTC) and `seq` (per-turn, strictly
//! increasing); `conv_id` / `turn_id` are absent only on records the
//! writer could not attribute to a turn (see [`Attribution`]).
//!
//! Forward compatibility is a hard rule: old lines must keep parsing
//! forever, so new fields are additive and `#[serde(default)]`. Nothing
//! in the agent / planner / writer / MCP paths ever reads this file — it
//! is a write-only audit artefact.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::tokens::Estimator;

/// Truncate `s` to at most `cap` bytes on a char boundary. Returns the
/// (possibly shortened) text and whether anything was dropped.
///
/// Tool results and MCP payloads are stored nowhere else, so truncation is
/// real data loss: every record that can truncate carries a
/// `*_truncated` flag and the panel renders the cap boundary. Estimates
/// are always computed from the *untruncated* text.
pub fn truncate_bytes(s: &str, cap: usize) -> (String, bool) {
    if s.len() <= cap {
        return (s.to_string(), false);
    }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

/// How a finished tool call's result was captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    /// The provider handed us raw args and results.
    Full,
    /// The provider exposes only a one-line summary string.
    SummaryOnly,
}

/// The result of a tool call, as far as the provider exposed it.
///
/// `None` is *not* the same as an empty result: it means the provider
/// never reports results for this tool (or the call was still in flight
/// when the turn ended), and the panel says so explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolOutput {
    /// Raw result text plus the provider's error flag.
    Full { content: String, is_error: bool },
    /// Only a summary string exists (e.g. a rendered tool-call line).
    Summary { text: String },
    /// No result was reported by this provider.
    None,
}

impl ToolOutput {
    /// True when no payload at all was captured.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

/// Whether an MCP record could be tied to a turn.
///
/// `McpCallLogEntry` carries no `conv_id` and the server is shared across
/// turns and conversations, so attribution is inferred: with exactly one
/// streaming conversation at the moment of the call the turn is known;
/// with zero or several it is not, and the record is kept but labelled
/// rather than silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribution {
    /// Exactly one conversation was streaming — this is its turn.
    TurnInferred,
    /// Zero or more than one conversation was streaming.
    Unattributed,
}

/// Exact provider-reported usage for one turn. Mirrors
/// `acp::protocol::TokenUsage` so this module never depends on the ACP
/// parsing types; the call site converts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub input_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub output_tokens: u64,
}

impl ProviderUsage {
    /// Tokens the model was conditioned on at the end of the turn —
    /// the context-window reading.
    pub fn prefix_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }
}

/// One record's payload, tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HistoryKind {
    TurnStart(TurnStart),
    ToolCall(ToolCall),
    McpCall(McpCall),
    MemoryInjection(MemoryInjection),
    TurnEnd(TurnEnd),
}

impl HistoryKind {
    /// The `kind` tag, for readers and the panel.
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::TurnStart(_) => "turn_start",
            Self::ToolCall(_) => "tool_call",
            Self::McpCall(_) => "mcp_call",
            Self::MemoryInjection(_) => "memory_injection",
            Self::TurnEnd(_) => "turn_end",
        }
    }
}

/// A complete line of `turns.ndjson`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryRecord {
    /// Schema version. Present on every line (never a header) so a
    /// truncated file still parses line by line.
    pub v: u8,
    /// RFC3339 UTC capture time.
    pub ts: String,
    /// Owning conversation. `None` only when the writer could not
    /// attribute the record to a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv_id: Option<String>,
    /// `"{conv_id}-{millis}"` — the same string already used for the
    /// memory manifest, so the two artefacts join on one key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// Per-turn monotonically increasing sequence number.
    pub seq: u32,
    #[serde(flatten)]
    pub payload: HistoryKind,
}

/// Current schema version written on new lines.
pub const SCHEMA_VERSION: u8 = 1;

/// The full initial prompt plus turn identity — the only place the
/// verbatim prompt is known before the agent is spawned.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnStart {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conv_title: Option<String>,
    pub workspace_root: String,
    /// The user's prompt, verbatim (truncated only at
    /// `HISTORY_MAX_PROMPT_BYTES`).
    pub prompt: String,
    pub prompt_bytes: usize,
    #[serde(default)]
    pub prompt_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_est: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator: Option<Estimator>,
}

/// One tool call: arguments and result, or an explicit summary-only marker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Raw tool arguments. `None` when the provider exposes only a summary.
    #[serde(default)]
    pub input: Option<Value>,
    pub output: ToolOutput,
    /// The provider's one-line summary, when it produced one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub capture: CaptureMode,
    #[serde(default)]
    pub input_truncated: bool,
    #[serde(default)]
    pub output_truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens_est: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens_est: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator: Option<Estimator>,
}

/// One MCP tool call: request and response, verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpCall {
    pub tool: String,
    pub input: Value,
    pub output: Value,
    pub duration_us: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub empty_result: bool,
    #[serde(default)]
    pub input_truncated: bool,
    #[serde(default)]
    pub output_truncated: bool,
    pub input_tokens_est: usize,
    pub output_tokens_est: usize,
    pub estimator: Estimator,
    pub attribution: Attribution,
}

/// The chat memory call and its rendered response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryInjection {
    pub items_injected: usize,
    pub pool_size: usize,
    pub tokens_used_est: usize,
    pub token_budget: usize,
    pub estimator: Estimator,
    /// The rendered `<project_memory>` block that went into the prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_block: Option<String>,
    #[serde(default)]
    pub response_block_truncated: bool,
    /// The exact manifest payload the writer task persisted, when manifests
    /// are enabled. `None` is valid (counts only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Value>,
}

/// Turn end — exact totals when the provider reports them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnEnd {
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub proposal_count: usize,
    pub assistant_bytes: usize,
    #[serde(default)]
    pub assistant_truncated: bool,
    /// Head of the assistant's output, capped at `HISTORY_MAX_ASSISTANT_BYTES`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_excerpt: Option<String>,
    pub output_tokens_est: usize,
    pub estimator: Estimator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap_tokens_est: Option<usize>,
    /// Present only when the provider reported authoritative usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ProviderUsage>,
    /// `"provider"` when `usage` came from the provider, never a heuristic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_source: Option<String>,
}

impl TurnEnd {
    /// Attach provider usage (and its provenance) if not already present.
    pub fn with_usage(mut self, usage: Option<ProviderUsage>) -> Self {
        if let Some(u) = usage {
            self.usage = Some(u);
            self.usage_source = Some("provider".to_string());
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_kind_tag_and_round_trips() {
        let rec = HistoryRecord {
            v: SCHEMA_VERSION,
            ts: "2026-09-15T18:43:44.512Z".to_string(),
            conv_id: Some("c1".into()),
            turn_id: Some("c1-1".into()),
            seq: 0,
            payload: HistoryKind::TurnStart(TurnStart {
                provider: "claude".into(),
                model: "sonnet".into(),
                conv_title: Some("history panel".into()),
                workspace_root: "C:/w".into(),
                prompt: "do the thing".into(),
                prompt_bytes: 12,
                prompt_truncated: false,
                input_tokens_est: Some(3),
                estimator: Some(Estimator::WordsX13),
            }),
        };
        let line = serde_json::to_string(&rec).unwrap();
        // Tagged flat, not nested under a `payload` key.
        assert!(line.contains("\"kind\":\"turn_start\""));
        assert!(line.contains("\"prompt\":\"do the thing\""));
        let back: HistoryRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn unparsable_future_field_is_tolerated_on_old_lines() {
        // A missing `ts` is fatal (required), but unknown extra keys are not —
        // that is what keeps the format forward compatible.
        let line = r#"{"v":1,"kind":"turn_end","ts":"t","seq":3,"cancelled":false,
            "proposal_count":0,"assistant_bytes":0,"output_tokens_est":0,
            "estimator":"words_x13","future_field":{"a":1}}"#;
        let rec: HistoryRecord = serde_json::from_str(line).unwrap();
        assert_eq!(rec.payload.kind_str(), "turn_end");
    }

    #[test]
    fn tool_output_variants_serialize_with_kind_tag() {
        let full = ToolOutput::Full {
            content: "ok".into(),
            is_error: false,
        };
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"kind":"full","content":"ok","is_error":false}"#
        );
        assert_eq!(
            serde_json::to_string(&ToolOutput::None).unwrap(),
            r#"{"kind":"none"}"#
        );
    }

    #[test]
    fn truncate_is_char_boundary_safe() {
        let (out, cut) = truncate_bytes("hello", 10);
        assert_eq!(out, "hello");
        assert!(!cut);
        let (out, cut) = truncate_bytes("hello", 3);
        assert_eq!(out, "hel");
        assert!(cut);
        // Multi-byte: 'é' is 2 bytes, so a 1-byte cap keeps nothing.
        let (out, cut) = truncate_bytes("é", 1);
        assert_eq!(out, "");
        assert!(cut);
    }

    #[test]
    fn usage_prefix_sums_cache_buckets() {
        let u = ProviderUsage {
            input_tokens: 10,
            cache_creation_input_tokens: 20,
            cache_read_input_tokens: 30,
            output_tokens: 40,
        };
        assert_eq!(u.prefix_tokens(), 60);
    }

    #[test]
    fn turn_end_with_usage_labels_provenance() {
        let end = TurnEnd {
            cancelled: false,
            error: None,
            proposal_count: 0,
            assistant_bytes: 0,
            assistant_truncated: false,
            assistant_excerpt: None,
            output_tokens_est: 0,
            estimator: Estimator::WordsX13,
            bootstrap_tokens_est: None,
            usage: None,
            usage_source: None,
        };
        let end = end.with_usage(Some(ProviderUsage::default()));
        assert_eq!(end.usage_source.as_deref(), Some("provider"));
    }
}
