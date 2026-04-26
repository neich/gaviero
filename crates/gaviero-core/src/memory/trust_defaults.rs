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
}
