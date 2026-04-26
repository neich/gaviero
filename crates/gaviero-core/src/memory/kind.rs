//! C1: typed memory kinds.
//!
//! Discriminator that splits the single `memories` table into three
//! lifecycle classes — Records, History, Summaries — each with distinct
//! retention, injection, mutability, and dedup policies. The DB stores
//! the kind as a `TEXT` with a `CHECK` constraint; this enum is the
//! Rust-side mirror.
//!
//! Defaults across the system:
//! - SQL column default: `'record'` (legacy rows migrate to record).
//! - Retrieval / chat-injection default: record only.
//! - MCP `memory_search`: kind="record" by default, with explicit
//!   "history" | "summary" | "any" allowed.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKind {
    Record,
    History,
    Summary,
}

impl MemoryKind {
    pub const ALL: [MemoryKind; 3] = [MemoryKind::Record, MemoryKind::History, MemoryKind::Summary];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryKind::Record => "record",
            MemoryKind::History => "history",
            MemoryKind::Summary => "summary",
        }
    }

    /// Default for legacy rows and any insert that does not specify a kind.
    pub fn default_kind() -> Self {
        MemoryKind::Record
    }

    /// True for kinds that the C1 SQL trigger marks as immutable
    /// (no UPDATE/DELETE outside the dedicated `RedactHistory` writer
    /// variant). Today only `History`.
    pub fn is_immutable(self) -> bool {
        matches!(self, MemoryKind::History)
    }

    /// True for kinds eligible for the chat-auto-inject path by default.
    /// History is audit data, never injected. Summaries inject only when
    /// a session_thread topic matches (handled by retrieval, not here).
    pub fn injects_into_chat_by_default(self) -> bool {
        matches!(self, MemoryKind::Record)
    }
}

impl fmt::Display for MemoryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MemoryKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "record" => Ok(MemoryKind::Record),
            "history" => Ok(MemoryKind::History),
            "summary" => Ok(MemoryKind::Summary),
            other => Err(format!(
                "unknown memory_kind '{other}'; expected one of: record, history, summary"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_str() {
        for k in MemoryKind::ALL {
            assert_eq!(MemoryKind::from_str(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn unknown_kind_rejected() {
        assert!(MemoryKind::from_str("episode").is_err());
        assert!(MemoryKind::from_str("").is_err());
    }

    #[test]
    fn immutability_only_history() {
        assert!(!MemoryKind::Record.is_immutable());
        assert!(MemoryKind::History.is_immutable());
        assert!(!MemoryKind::Summary.is_immutable());
    }

    #[test]
    fn default_inject_only_record() {
        assert!(MemoryKind::Record.injects_into_chat_by_default());
        assert!(!MemoryKind::History.injects_into_chat_by_default());
        assert!(!MemoryKind::Summary.injects_into_chat_by_default());
    }

    #[test]
    fn serde_lowercase() {
        let s = serde_json::to_string(&MemoryKind::History).unwrap();
        assert_eq!(s, "\"history\"");
        let k: MemoryKind = serde_json::from_str("\"summary\"").unwrap();
        assert_eq!(k, MemoryKind::Summary);
    }
}
