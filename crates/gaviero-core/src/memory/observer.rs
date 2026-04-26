//! Observer trait for memory writes.
//!
//! Implementors receive lifecycle callbacks as `WriterMessage`s flow through the
//! writer task: `on_write_enqueued` fires synchronously from the caller side,
//! `on_write_committed` / `on_write_failed` fire from the writer task after the
//! underlying `MemoryStore` operation completes.

use super::writer::WriteResult;

/// Lifecycle hooks fired around every `WriterMessage` processed by the
/// single-consumer writer task. Implementations must be `Send + Sync` and
/// cheap — the writer task holds no locks across these calls, so a slow
/// observer directly stalls the queue.
pub trait MemoryObserver: Send + Sync {
    /// Fired on the caller's thread immediately before a message is pushed
    /// into the writer channel. `kind` is a stable, non-PII string naming
    /// the `WriterMessage` variant (e.g. `"UserRemember"`).
    fn on_write_enqueued(&self, kind: &str);

    /// Fired from the writer task after the underlying store operation
    /// succeeds. `result` summarises the outcome (inserted / deduplicated /
    /// already-covered / skipped). `kind` matches the earlier enqueue hook.
    fn on_write_committed(&self, kind: &str, result: &WriteResult);

    /// Fired from the writer task when the underlying operation fails.
    /// Failure is non-fatal — the writer task continues draining subsequent
    /// messages. `error` is a human-readable message, not a typed error.
    fn on_write_failed(&self, kind: &str, error: &str);
}

/// Fired from the writer task after an `InjectionManifest` row lands in
/// the `injection_manifests` table (Tier S / S4). The TUI memory panel
/// (A4) subscribes to this to refresh the Injected Now section live.
///
/// `turn_id` is the same value the panel keys its manifest state on, so
/// the handler can fetch the row via `MemoryStore::manifests_for_turn`
/// after the callback fires.
pub trait ManifestObserver: Send + Sync {
    fn on_manifest_persisted(&self, turn_id: &str, session_id: &str);
}
