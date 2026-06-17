//! Tool input/output schemas (Tier A / A5).
//!
//! Exactly three tools — no writes. Each input/output struct derives
//! `schemars::JsonSchema` so rmcp's tool macro can emit the JSON-RPC
//! schema at server-handshake time.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const TOOL_MEMORY_SEARCH: &str = "memory_search";
pub const TOOL_BLAST_RADIUS: &str = "blast_radius";
pub const TOOL_NODE_DOC: &str = "node_doc";

// ── memory_search ─────────────────────────────────────────────────

/// Input schema for the `memory_search` MCP tool.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct MemorySearchInput {
    /// Free-form natural-language query. Same shape as a chat prompt's
    /// retrieval query.
    pub query: String,
    /// Optional scope filter (`"global"`, `"workspace"`, `"repo"`,
    /// `"module"`, `"run"`). When omitted, retrieval merges the
    /// workspace and global scopes via multi-scope hybrid search (RRF),
    /// not a narrow→wide cascade.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_hint: Option<String>,
    /// Maximum results. Server clamps to [1, 20] to protect token
    /// budget in the calling subprocess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Tier C / C1.6: lifecycle-class filter. One of `"record"`
    /// (default), `"history"`, `"summary"`, or `"any"`. Records are
    /// the workhorse facts; History is the immutable raw transcript
    /// log (audit data, not normally injected); Summaries are the
    /// session consolidator's output. Subprocess agents should default
    /// to `record` and only opt into `history` or `any` for
    /// audit/forensic queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// A single `memory_search` result row. Field set is the minimum
/// subprocess agents need to decide whether to quote the memory in
/// their next tool call.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MemorySearchResult {
    pub id: i64,
    pub scope: String,
    #[serde(rename = "type")]
    pub memory_type: String,
    pub text: String,
    pub importance: f32,
    pub trust: f32,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct MemorySearchOutput {
    pub results: Vec<MemorySearchResult>,
}

// ── blast_radius ──────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct BlastRadiusInput {
    /// One or more seed paths (repo-relative). Empty inputs are an
    /// error.
    pub paths: Vec<String>,
    /// Graph traversal depth. Clamped to [1, 5]. Defaults to 2 —
    /// matches the chat-path default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// Relation mode: `"all"` (default), `"impact"`, `"callers"`,
    /// `"tests"`, or `"implementations"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct BlastRadiusRelation {
    pub path: String,
    pub relation: String,
    pub distance: u32,
    /// Populated when the Tier D1 NodeDoc schema lands; empty today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// C4: mode-weighted personalized PageRank score for this file.
    /// Higher = more relevant for the requested `mode`. Optional so
    /// pre-C4 callers ignore it cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// C3: HippoRAG-style file-level node specificity in [0.0, 1.0].
    /// 1.0 = domain-specific, ~0.0 = stop-symbol heavy. Optional so
    /// agents can ignore it without breaking the schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specificity: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct BlastRadiusOutput {
    pub nodes: Vec<BlastRadiusRelation>,
}

// ── node_doc ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema, Default)]
pub struct NodeDocInput {
    pub path: String,
}

/// NodeDoc schema stub (Tier D1 finalizes). MVP returns the
/// signatures we already extract via tree-sitter; `purpose` and
/// `summary` remain empty until Tier D fills them in.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct NodeDoc {
    pub path: String,
    #[serde(default)]
    pub signatures: Vec<String>,
    /// Tier D1 fills this in. Empty string today so the schema is
    /// stable for subprocess agents.
    #[serde(default)]
    pub purpose: String,
    /// Tier D1 fills this in.
    #[serde(default)]
    pub summary: String,
}

/// Clamp `memory_search.limit` to the server-enforced range. Plan
/// §A5 calls for <100ms latency on a populated DB; capping at 20 keeps
/// subprocess token budget tight.
pub const MEMORY_SEARCH_MIN_LIMIT: usize = 1;
pub const MEMORY_SEARCH_MAX_LIMIT: usize = 20;
pub const MEMORY_SEARCH_DEFAULT_LIMIT: usize = 5;

/// Clamp helper shared by the server handler.
pub fn clamp_memory_search_limit(limit: Option<u32>) -> usize {
    let n = limit.unwrap_or(MEMORY_SEARCH_DEFAULT_LIMIT as u32) as usize;
    n.clamp(MEMORY_SEARCH_MIN_LIMIT, MEMORY_SEARCH_MAX_LIMIT)
}

/// Clamp `blast_radius.depth` to [1, 5].
pub fn clamp_blast_depth(depth: Option<u32>) -> u32 {
    depth.unwrap_or(2).clamp(1, 5)
}

/// C1.6: resolve the optional `memory_search.kind` parameter to a
/// concrete filter. Returns:
/// - `Ok(Some(MemoryKind))` to filter by exactly that kind (the
///   common case — `"record"` is the documented default and the
///   strongly-recommended choice).
/// - `Ok(None)` for `"any"` (explicit unfiltered cross-kind search).
/// - `Err(...)` for unknown values so subprocess agents see a clear
///   error rather than silently falling through to the default.
pub fn resolve_memory_search_kind(
    kind: Option<&str>,
) -> std::result::Result<Option<crate::memory::MemoryKind>, String> {
    use std::str::FromStr;
    match kind {
        None | Some("record") => Ok(Some(crate::memory::MemoryKind::Record)),
        Some("any") => Ok(None),
        Some(other) => crate::memory::MemoryKind::from_str(other)
            .map(Some)
            .map_err(|e| {
                format!(
                    "memory_search.kind: unknown value {other:?}; expected \
                     'record' | 'history' | 'summary' | 'any' ({e})"
                )
            }),
    }
}

/// C1.6: default kind constant for documentation / tests.
pub const MEMORY_SEARCH_DEFAULT_KIND: &str = "record";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_search_limit_clamps() {
        assert_eq!(clamp_memory_search_limit(None), MEMORY_SEARCH_DEFAULT_LIMIT);
        assert_eq!(clamp_memory_search_limit(Some(0)), MEMORY_SEARCH_MIN_LIMIT);
        assert_eq!(
            clamp_memory_search_limit(Some(999)),
            MEMORY_SEARCH_MAX_LIMIT
        );
    }

    #[test]
    fn blast_depth_defaults_and_clamps() {
        assert_eq!(clamp_blast_depth(None), 2);
        assert_eq!(clamp_blast_depth(Some(0)), 1);
        assert_eq!(clamp_blast_depth(Some(99)), 5);
    }

    #[test]
    fn memory_search_input_parses_from_minimal_json() {
        let input: MemorySearchInput = serde_json::from_str(r#"{"query":"foo"}"#).unwrap();
        assert_eq!(input.query, "foo");
        assert!(input.scope_hint.is_none());
        assert!(input.limit.is_none());
        // C1.6: kind defaults to None on the wire (resolver maps it
        // to Record at call time).
        assert!(input.kind.is_none());
    }

    #[test]
    fn memory_search_input_round_trips_kind_field() {
        let input: MemorySearchInput =
            serde_json::from_str(r#"{"query":"foo","kind":"history"}"#).unwrap();
        assert_eq!(input.kind.as_deref(), Some("history"));
    }

    /// C1.6: the resolver defines the contract for memory_search.kind:
    /// missing or "record" → Record; "history"/"summary" → that kind;
    /// "any" → no filter; unknown → loud error.
    #[test]
    fn resolve_memory_search_kind_default_is_record() {
        use crate::memory::MemoryKind;
        assert_eq!(
            resolve_memory_search_kind(None).unwrap(),
            Some(MemoryKind::Record)
        );
        assert_eq!(
            resolve_memory_search_kind(Some("record")).unwrap(),
            Some(MemoryKind::Record)
        );
    }

    #[test]
    fn resolve_memory_search_kind_explicit_kinds() {
        use crate::memory::MemoryKind;
        assert_eq!(
            resolve_memory_search_kind(Some("history")).unwrap(),
            Some(MemoryKind::History)
        );
        assert_eq!(
            resolve_memory_search_kind(Some("summary")).unwrap(),
            Some(MemoryKind::Summary)
        );
    }

    #[test]
    fn resolve_memory_search_kind_any_returns_no_filter() {
        assert_eq!(resolve_memory_search_kind(Some("any")).unwrap(), None);
    }

    #[test]
    fn resolve_memory_search_kind_rejects_unknown() {
        let err = resolve_memory_search_kind(Some("episode")).unwrap_err();
        assert!(err.contains("episode"), "{err}");
        assert!(err.contains("expected"), "{err}");
    }

    #[test]
    fn tool_names_are_stable() {
        assert_eq!(TOOL_MEMORY_SEARCH, "memory_search");
        assert_eq!(TOOL_BLAST_RADIUS, "blast_radius");
        assert_eq!(TOOL_NODE_DOC, "node_doc");
        // C1.6: documented default kind is record.
        assert_eq!(MEMORY_SEARCH_DEFAULT_KIND, "record");
    }
}
