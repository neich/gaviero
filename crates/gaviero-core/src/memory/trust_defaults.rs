//! Per-source trust defaults (Tier A / A3).
//!
//! Single source of truth for the mapping from write-origin `MemorySource`
//! to its default `trust_score` in [0.0, 1.0]. Keeping the table here
//! (not scattered across writer call sites) lets future changes — e.g.
//! Tier B5 sleeptime re-scoring, Tier A4 panel pin operations — land
//! without hunting down if-else chains.

use serde::{Deserialize, Serialize};

/// Write origin for a memory record. Stored in the `memories.source`
/// column as a string (the `as_str()` form) so forward-compatible
/// additions don't break existing DBs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySource {
    /// Chat `/remember` command.
    UserRemember,
    /// TUI memory panel edit (Phase 2 / A4).
    UserPanel,
    /// LLM `<turn_annotations>` sidecar flag (Phase 1 / A1).
    LlmAnnotated,
    /// Per-turn extractor output (Tier S3).
    LlmExtracted,
    /// LLM-driven session / sleeptime consolidator (Tier B5).
    LlmConsolidated,
    /// Swarm post-execution consolidation.
    SwarmConsolidated,
    /// One-shot import from an external MCP memory server.
    McpImport,
    /// Compiler / test / tool output captured as memory.
    ToolOutput,
    /// Tier C / C1: raw turn transcript (the History row itself, not
    /// any derived Memory). Trust is 1.0 because it *is* the source of
    /// truth — every derived record traces back here.
    RawTranscript,
    /// Pre-A3 row with no recorded source.
    UnknownLegacy,
}

impl MemorySource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UserRemember => "user_remember",
            Self::UserPanel => "user_panel",
            Self::LlmAnnotated => "llm_annotated",
            Self::LlmExtracted => "llm_extracted",
            Self::LlmConsolidated => "llm_consolidated",
            Self::SwarmConsolidated => "swarm_consolidated",
            Self::McpImport => "mcp_import",
            Self::ToolOutput => "tool_output",
            Self::RawTranscript => "raw_transcript",
            Self::UnknownLegacy => "unknown_legacy",
        }
    }

    pub fn parse_str(s: &str) -> Self {
        match s {
            "user_remember" => Self::UserRemember,
            "user_panel" => Self::UserPanel,
            "llm_annotated" => Self::LlmAnnotated,
            "llm_extracted" => Self::LlmExtracted,
            "llm_consolidated" => Self::LlmConsolidated,
            "swarm_consolidated" => Self::SwarmConsolidated,
            "mcp_import" => Self::McpImport,
            "tool_output" => Self::ToolOutput,
            "raw_transcript" => Self::RawTranscript,
            _ => Self::UnknownLegacy,
        }
    }

    /// Default trust multiplier for this source. Multiplier ∈ [0.0, 1.0]
    /// — higher means retrieval scores scale up more.
    ///
    /// Rationale for each value (plan §A3):
    /// * 1.00 user_remember / user_panel — the user said so.
    /// * 0.85 tool_output — deterministic compiler / test output.
    /// * 0.75 llm_consolidated / swarm_consolidated — post-hoc reflection.
    /// * 0.75 unknown_legacy — generous backfill default.
    /// * 0.70 llm_annotated — LLM self-flagged with full turn context.
    /// * 0.60 llm_extracted — inferred from transcript alone.
    /// * 0.50 mcp_import — external, not audited.
    pub fn default_trust(&self) -> f32 {
        match self {
            Self::UserRemember | Self::UserPanel => 1.0,
            // C1: a History row's content is the verbatim transcript.
            // Whatever derives from it can be wrong; the row itself
            // is by definition correct.
            Self::RawTranscript => 1.0,
            Self::ToolOutput => 0.85,
            Self::LlmConsolidated | Self::SwarmConsolidated => 0.75,
            Self::UnknownLegacy => 0.75,
            Self::LlmAnnotated => 0.7,
            Self::LlmExtracted => 0.6,
            Self::McpImport => 0.5,
        }
    }
}

/// Clamp an arbitrary trust override to the valid range.
pub fn clamp_trust(t: f32) -> f32 {
    if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) }
}

/// Fraction of a source's default trust an agent flag demotes to (D1).
pub const FLAG_DEMOTION_FACTOR: f32 = 0.5;

/// Floor an agent flag will never demote below (D1). Trust multiplies
/// the composite score, so a flagged row loses ranking weight without
/// being erased — consistent with OD-3 (down-weight + soft-delete only).
pub const FLAG_TRUST_FLOOR: f32 = 0.2;

/// Trust a row should carry after an agent flag.
///
/// Computed from the source's *default* trust rather than as a repeated
/// decrement, so flagging the same row twice is a genuine no-op — which
/// is what makes `idempotent_hint = true` on the `memory_flag` tool
/// truthful and removes any need for a dedup table.
///
/// `max(FLAG_TRUST_FLOOR, min(current, default * FLAG_DEMOTION_FACTOR))`
pub fn flagged_trust(source: MemorySource, current_trust: f32) -> f32 {
    let demoted = source.default_trust() * FLAG_DEMOTION_FACTOR;
    clamp_trust(current_trust.min(demoted).max(FLAG_TRUST_FLOOR))
}

/// Whether an agent may flag a row with this source (D1).
///
/// `UserRemember` / `UserPanel` are user ground truth; `RawTranscript` is
/// the immutable History row whose trust is 1.0 by definition. Everything
/// else — including `ToolOutput` — is flaggable: compiler and test output
/// is deterministic at capture time but goes stale like anything else.
pub fn is_agent_flaggable(source: MemorySource) -> bool {
    !matches!(
        source,
        MemorySource::UserRemember | MemorySource::UserPanel | MemorySource::RawTranscript
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_sources_roundtrip_through_string() {
        for s in [
            MemorySource::UserRemember,
            MemorySource::UserPanel,
            MemorySource::LlmAnnotated,
            MemorySource::LlmExtracted,
            MemorySource::LlmConsolidated,
            MemorySource::SwarmConsolidated,
            MemorySource::McpImport,
            MemorySource::ToolOutput,
            MemorySource::RawTranscript,
            MemorySource::UnknownLegacy,
        ] {
            assert_eq!(MemorySource::parse_str(s.as_str()), s);
        }
    }

    #[test]
    fn parse_unknown_string_falls_back_to_legacy() {
        assert_eq!(
            MemorySource::parse_str("future_source_v999"),
            MemorySource::UnknownLegacy
        );
    }

    #[test]
    fn trust_ordering_matches_plan() {
        assert!(
            MemorySource::UserRemember.default_trust() > MemorySource::LlmAnnotated.default_trust()
        );
        assert!(
            MemorySource::LlmAnnotated.default_trust() > MemorySource::LlmExtracted.default_trust()
        );
        assert!(
            MemorySource::LlmExtracted.default_trust() > MemorySource::McpImport.default_trust()
        );
    }

    #[test]
    fn clamp_trust_handles_edges() {
        assert_eq!(clamp_trust(-1.0), 0.0);
        assert_eq!(clamp_trust(2.0), 1.0);
        assert_eq!(clamp_trust(f32::NAN), 0.0);
    }

    #[test]
    fn flagged_trust_matches_d1_table() {
        for (source, expected) in [
            (MemorySource::LlmAnnotated, 0.35),
            (MemorySource::SwarmConsolidated, 0.375),
            (MemorySource::LlmExtracted, 0.30),
            (MemorySource::McpImport, 0.25),
        ] {
            let got = flagged_trust(source, source.default_trust());
            assert!(
                (got - expected).abs() < 1e-6,
                "{source:?}: expected {expected}, got {got}"
            );
        }
    }

    #[test]
    fn flagged_trust_is_idempotent() {
        let s = MemorySource::LlmAnnotated;
        let once = flagged_trust(s, s.default_trust());
        let twice = flagged_trust(s, once);
        assert_eq!(once, twice, "a repeat flag must be a no-op");
    }

    #[test]
    fn flagged_trust_never_goes_below_the_floor() {
        // McpImport halves to 0.25, but a row already sitting at 0.05
        // must not be pushed further down.
        let got = flagged_trust(MemorySource::McpImport, 0.05);
        assert_eq!(got, FLAG_TRUST_FLOOR);
    }

    #[test]
    fn agent_flaggable_excludes_user_and_history_sources() {
        assert!(!is_agent_flaggable(MemorySource::UserRemember));
        assert!(!is_agent_flaggable(MemorySource::UserPanel));
        assert!(!is_agent_flaggable(MemorySource::RawTranscript));
        assert!(is_agent_flaggable(MemorySource::ToolOutput));
        assert!(is_agent_flaggable(MemorySource::LlmAnnotated));
    }
}
