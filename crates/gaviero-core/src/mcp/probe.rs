//! In-memory reach-probe ledger for [`super::tools::TOOL_MEMORY_PING`].
//!
//! The ledger is process-local and never touches the memory writer or
//! SQLite. A brief `Mutex` records each ping; callers must not hold it
//! across `.await`. Receipts are `sha256(nonce ‖ depth ‖ started_at)`
//! truncated to 12 hex chars so a nested ping counts only when the
//! *server* saw it (plan L2).

use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// One `memory_ping` observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PingRecord {
    pub nonce: String,
    pub depth: u8,
    pub at: DateTime<Utc>,
    pub receipt: String,
}

/// Shared ping log attached to a [`super::GavieroMcpServer`] for a probe run.
///
/// Cheap to clone (`Arc`); empty until [`ProbeLedger::record`] is called.
#[derive(Debug, Clone, Default)]
pub struct ProbeLedger {
    inner: Arc<Mutex<Vec<PingRecord>>>,
}

impl ProbeLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one ping. Brief lock, no I/O.
    pub fn record(&self, rec: PingRecord) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(rec);
    }

    pub fn records(&self) -> Vec<PingRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn records_for_nonce(&self, nonce: &str) -> Vec<PingRecord> {
        self.records()
            .into_iter()
            .filter(|r| r.nonce == nonce)
            .collect()
    }
}

/// First 12 hex chars of `sha256(nonce ‖ depth ‖ started_at)`.
pub fn ping_receipt(nonce: &str, depth: u8, started_at: DateTime<Utc>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(nonce.as_bytes());
    hasher.update([depth]);
    hasher.update(started_at.to_rfc3339().as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..12].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn receipt_is_twelve_hex_chars_and_stable() {
        let t = Utc.with_ymd_and_hms(2026, 9, 14, 12, 0, 0).unwrap();
        let a = ping_receipt("nonce-1", 0, t);
        let b = ping_receipt("nonce-1", 0, t);
        assert_eq!(a, b);
        assert_eq!(a.len(), 12);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, ping_receipt("nonce-1", 1, t));
        assert_ne!(a, ping_receipt("nonce-2", 0, t));
    }

    #[test]
    fn ledger_isolates_nonces() {
        let ledger = ProbeLedger::new();
        let t = Utc::now();
        ledger.record(PingRecord {
            nonce: "a".into(),
            depth: 0,
            at: t,
            receipt: "aaaaaaaaaaaa".into(),
        });
        ledger.record(PingRecord {
            nonce: "b".into(),
            depth: 1,
            at: t,
            receipt: "bbbbbbbbbbbb".into(),
        });
        assert_eq!(ledger.records_for_nonce("a").len(), 1);
        assert_eq!(ledger.records_for_nonce("b")[0].depth, 1);
        assert!(ledger.records_for_nonce("c").is_empty());
    }
}
