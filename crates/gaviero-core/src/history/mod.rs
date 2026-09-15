//! Per-turn history: the record model, the single writer, the tolerant
//! reader, and the one token estimator.
//!
//! The problem this module solves: after a turn, the TUI could show the
//! prompt but *not* what actually went into and came out of the turn —
//! tool arguments, tool results, MCP request/response pairs, and the
//! memory call's response were captured nowhere (or dropped on the live
//! path). This module owns the durable artefact that answers "what
//! happened in this turn?" for both the HISTORY panel and the
//! `gaviero history` CLI.
//!
//! # On-disk layout
//!
//! `<workspace>/.gaviero/history/turns.ndjson` — one JSON object per
//! line, tagged by `kind`, schema-versioned per line (`v`, never a
//! header, so truncation or rotation can never invalidate the rest of the
//! file). Size-rotated at [`HISTORY_MAX_BYTES`], keeping exactly one
//! prior generation as `turns.ndjson.1`, through the same
//! [`ndjson::NdjsonAppender`] the MCP telemetry sink uses.
//!
//! # Hard invariants (from the plan)
//!
//! 1. **Write-only.** Nothing in the agent, planner, writer task,
//!    extractor, or MCP server reads this file; a missing, truncated, or
//!    malformed log must never affect a turn. Read failures degrade to a
//!    panel message.
//! 2. **Never written from the memory writer task** — the anti-pattern
//!    documented at `mcp/observer.rs` applies verbatim.
//! 3. **Writing stays cheap**: append-only, no `fsync`, no
//!    pretty-printing; the tool response waits on the callback.
//! 4. **No dual writers**: exactly one [`writer::HistoryRecorder`] per
//!    app instance.
//! 5. **Every estimate is labelled** with its [`tokens::Estimator`]; an
//!    exact count is labelled `usage_source: "provider"`.
//! 6. **Append-only and forward compatible**: new fields are additive
//!    with `#[serde(default)]`; old lines parse forever.
//!
//! # Turn identity
//!
//! `turn_id = "{conv_id}-{millis}"`, the string the TUI already computes
//! at dispatch and already persists on `injection_manifests`. It is
//! reused here rather than invented, so the history log and the memory
//! manifest join on one key.

pub mod ndjson;
pub mod reader;
pub mod record;
pub mod tokens;
pub mod writer;

use std::path::{Path, PathBuf};

pub use ndjson::{NdjsonAppender, rotated_path};
pub use reader::{
    HistoryEvent, ReadOutcome, TurnRecords, TurnStatus, TurnSummary, group_turns, read_records,
    read_turn, summarize,
};
pub use record::{
    Attribution, CaptureMode, HistoryKind, HistoryRecord, McpCall, MemoryInjection, ProviderUsage,
    SCHEMA_VERSION, ToolCall, ToolOutput, TurnEnd, TurnStart,
};
pub use tokens::{
    Estimator, count_words, estimate_json_text_tokens, estimate_json_tokens, estimate_text_tokens,
    words_to_tokens,
};
pub use writer::HistoryRecorder;

/// Directory under the workspace root: `<root>/.gaviero/history`.
pub const HISTORY_DIR: &str = ".gaviero/history";

/// The single NDJSON file name.
pub const HISTORY_FILENAME: &str = "turns.ndjson";

/// Rotation threshold for `turns.ndjson`: 32 MB, keeping one generation.
/// Larger than the MCP sink's 10 MB because tool payloads are big.
pub const HISTORY_MAX_BYTES: u64 = 32 * 1024 * 1024;

/// Cap on the stored prompt. The **estimate** is still computed from the
/// untruncated text.
pub const HISTORY_MAX_PROMPT_BYTES: usize = 256 * 1024;

/// Cap on a stored JSON payload (tool args, tool result, MCP in/out,
/// memory manifest).
pub const HISTORY_MAX_JSON_BYTES: usize = 64 * 1024;

/// Cap on the rendered `<project_memory>` response block.
pub const HISTORY_MAX_BLOCK_BYTES: usize = 64 * 1024;

/// Cap on the stored head of the assistant's output.
pub const HISTORY_MAX_ASSISTANT_BYTES: usize = 8 * 1024;

/// The history log path for a workspace root.
pub fn history_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(HISTORY_DIR)
        .join(HISTORY_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_is_workspace_relative_under_gaviero() {
        let p = history_path(Path::new("C:/w/gaviero"));
        assert!(p.ends_with("turns.ndjson"));
        assert!(p.to_string_lossy().contains(".gaviero"));
    }
}
