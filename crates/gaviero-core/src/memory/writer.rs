//! Single-consumer writer task that serializes every write into `MemoryStore`.
//!
//! Transport (currently `tokio::sync::mpsc::UnboundedSender`) is encapsulated
//! behind `WriterHandle`; swapping mpsc for an IPC channel would touch only
//! this file. Callers never hold a `MemoryStore` reference for writes — they
//! enqueue a `WriterMessage` and, if they need confirmation, await the
//! optional `oneshot` ack under [`ACK_TIMEOUT_MS`].
//!
//! Lock discipline: the writer task body never holds a `tokio::sync::Mutex`
//! guard across an `await`, tree-sitter call, or filesystem I/O. Embedding
//! runs inside `MemoryStore::store*` which already sequences "embed first,
//! lock briefly, release" — the task simply funnels messages into those
//! methods without re-acquiring any lock of its own.

#![deny(clippy::await_holding_lock)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::Value as JsonValue;
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use super::consolidation_llm::ConsolidationLlm;
use super::observer::{ManifestObserver, MemoryObserver};
use super::scope::{StoreResult, WriteMeta, WriteScope, store_kind_for_scope};
use super::store::{MemoryStore, StoreOptions};
use super::stores::MemoryStores;
use super::trust_defaults::MemorySource;

/// Default ack timeout for synchronous variants.
///
/// Sized for the slowest thing a handler does before it can ack, not the
/// fastest. Every acked variant reaches `store_scoped` — an ONNX embed
/// plus up to three cosine dedup scans — or a bulk store operation, and
/// the clock starts at *enqueue*, so time spent waiting behind other
/// messages counts against it too. At the previous 500ms one slow embed
/// made every message queued behind it report a spurious timeout while
/// the writer was still committing them, and the caller had no way to
/// tell that from a real failure. Matches the 30s the restore, forget,
/// and redact paths already use for the same reason.
pub const ACK_TIMEOUT_MS: u64 = 30_000;

/// Outcome of a processed write message.
#[derive(Debug, Clone)]
pub enum WriteResult {
    /// New row inserted at the requested scope.
    Inserted(i64),
    /// Existing row at the same scope was reinforced (dedup hit).
    Deduplicated(i64),
    /// Write skipped because the content is already covered by a broader scope.
    AlreadyCovered,
    /// Message was accepted but no store write was performed (placeholder /
    /// not-yet-implemented variants — Phase 3/4 will fill these in).
    Skipped,
    /// Accepted and handed to a background task; the resulting writes
    /// arrive later as their own message. Only
    /// [`WriterMessage::SessionConsolidate`] returns this: its LLM call
    /// runs off the writer task, so "the request was taken" is the most
    /// the caller can be told synchronously.
    ///
    /// Distinct from [`Self::Skipped`] on purpose — "nothing will
    /// happen" and "something is happening elsewhere" are different
    /// answers, and conflating them is what made the old ack timeout
    /// unreadable.
    Queued,
}

impl From<StoreResult> for WriteResult {
    fn from(value: StoreResult) -> Self {
        match value {
            StoreResult::Inserted(id) => Self::Inserted(id),
            StoreResult::Deduplicated(id) => Self::Deduplicated(id),
            StoreResult::AlreadyCovered => Self::AlreadyCovered,
        }
    }
}

/// Outcome of a [`WriterMessage::AgentFlag`].
///
/// `accepted: false` is a *successful* refusal — the row's source is not
/// agent-flaggable (D1) — not an error. Errors are reserved for "the row
/// or its owning store could not be resolved".
#[derive(Debug, Clone)]
pub struct AgentFlagOutcome {
    pub accepted: bool,
    pub detail: String,
}

/// Every write to `MemoryStore` flows through one of these variants.
///
/// `#[non_exhaustive]` so adding variants in future phases does not force
/// every `match` site to update. Production `match` arms SHOULD include a
/// `_ => {}` fallback; tests assert concrete variants.
#[non_exhaustive]
#[derive(Debug)]
pub enum WriterMessage {
    /// User-initiated `/remember` command. Synchronous: caller awaits ack.
    ///
    /// When `scope` is `Some`, the writer uses `store_scoped` with
    /// `WriteMeta::for_source(MemorySource::UserRemember)` (A3 +
    /// A2-aware path). When `scope` is `None`, the writer falls back to
    /// the legacy namespace/key `store_with_options` path for pre-A2
    /// callers. New call sites should pass `Some(scope)`.
    UserRemember {
        namespace: String,
        key: String,
        content: String,
        metadata: Option<String>,
        scope: Option<WriteScope>,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// Legacy namespace/key write with full `StoreOptions`. Used by
    /// compatibility paths that still need staleness metadata.
    Store {
        namespace: String,
        key: String,
        content: String,
        options: StoreOptions,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// Chat turn completed — hand off transcript to the per-turn extractor
    /// (Phase 4). Fire-and-forget; failures are structured-logged.
    TurnComplete {
        session_id: String,
        turn_id: String,
        repo_id: String,
        module_path: Option<String>,
        run_id: String,
        transcript: String,
        annotations: Option<JsonValue>,
    },
    /// Swarm consolidation promoting a run-scope fact to a wider scope.
    /// Optional ack for call sites that need to block on commit.
    SwarmConsolidate {
        scope: WriteScope,
        content: String,
        meta: WriteMeta,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// Delete all run-scope memories for a completed run after
    /// consolidation has promoted durable entries.
    DeleteRun {
        run_id: String,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// TUI memory panel edit (Tier A / A4). `op` distinguishes delete,
    /// pin (trust raise to 1.0), scope change, and text edit. All four
    /// route through the writer task so the Tier S2 single-consumer
    /// invariant holds; the panel never touches `MemoryStore` directly.
    ///
    /// `scope_level` / `repo_id` are the *persisted* scope of the row
    /// being edited (not a new scope — `PanelEditOp::SetScope` carries
    /// that separately). They exist because memory ids are only unique
    /// within one physical DB: the global, workspace, and folder stores
    /// have independent rowid spaces, so an id alone cannot identify a
    /// row. The writer resolves the owning store with
    /// [`store_kind_for_scope`] and refuses the edit when the pair is
    /// inconsistent rather than guessing.
    PanelEdit {
        op: PanelEditOp,
        scope_level: i32,
        repo_id: Option<String>,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// Injection manifest produced by the chat retrieval stage (Phase 3).
    /// Fire-and-forget; Phase 3 adds the actual persistence inside the task.
    InjectionManifest {
        turn_id: String,
        session_id: String,
        payload: JsonValue,
    },
    /// Sleeptime consolidation / pruning job (Tier B5). `payload` is
    /// retained for forward-compat with bespoke configs the CLI may
    /// send; defaults are resolved from settings inside the writer
    /// task when `payload` is empty.
    Sleeptime { payload: JsonValue },
    /// Tier B / B5: end-of-session consolidator, phase 1 of 2.
    ///
    /// The writer gathers transcript + recent extractions and builds the
    /// prompt — all cheap reads — then hands the [`ConsolidationLlm`]
    /// call to a background task and acks [`WriteResult::Queued`]
    /// immediately. The model's operations come back as
    /// [`Self::SessionConsolidateApply`].
    ///
    /// The LLM call used to run inline here. It is a full model
    /// round-trip on a whole-session prompt — one measured run took
    /// **3m41s** — and the writer is deliberately single-consumer, so
    /// every other memory write in the process queued behind it for that
    /// whole time while the caller's 30s ack budget expired and reported
    /// a failure for work that in fact succeeded.
    SessionConsolidate {
        session_id: String,
        repo_id: String,
        module_path: Option<String>,
        run_id: String,
        transcript: String,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// Tier B / B5: end-of-session consolidator, phase 2 of 2 — apply
    /// the operations the model produced.
    ///
    /// Normally enqueued by the background task that
    /// [`Self::SessionConsolidate`] spawned, so `ack` is `None`; it is
    /// `Some` when a test drives the apply path directly. Every write
    /// still happens here, on the writer task — the split moves the
    /// *thinking* off it, not the writing.
    SessionConsolidateApply {
        repo_id: String,
        module_path: Option<String>,
        run_id: String,
        candidates: Vec<super::session_consolidator::CandidateBrief>,
        parsed: Box<super::session_consolidator::ConsolidatorResponse>,
        ack: Option<oneshot::Sender<Result<WriteResult, String>>>,
    },
    /// Tier B / B6: post-turn retrieval-use telemetry. Reads the S4
    /// manifest for `turn_id`, embeds the response, classifies each
    /// injected memory as Used / Partial / Unused, and persists rows
    /// to `retrieval_use`. Fire-and-forget; never blocks the user.
    TelemetryClassify {
        turn_id: String,
        session_id: String,
        response: String,
    },
    /// Tier C / C2.2: restore a single audit row by id. The handler
    /// reconstructs the original `WriteScope` + `WriteMeta` + content
    /// from `original_row_json` and replays them through
    /// [`MemoryStore::store_scoped`] so dedup decides whether the
    /// payload reinserts cleanly, dedups against a newer row, or is
    /// already covered at a broader scope. Audit row is consumed on
    /// success.
    Restore {
        deletion_id: i64,
        ack: Option<oneshot::Sender<Result<super::store::RestoreOutcome, String>>>,
    },
    /// Tier C / C2.2: restore every still-pending deletion newer than
    /// `since_sql_offset` (a SQLite relative-datetime spec like
    /// `"-7 days"`). Each row goes through the dedup pipeline; the
    /// per-id outcome is returned so the caller can summarise.
    RestoreSince {
        since_sql_offset: String,
        ack: Option<oneshot::Sender<Result<Vec<super::store::RestoreOutcome>, String>>>,
    },
    /// Tier C / C2.3: bulk soft-delete by [`super::store::ForgetFilter`].
    /// `dry_run = true` returns a populated report without writing —
    /// the caller (TUI / CLI) shows the count, the user confirms, and
    /// the live call goes back through the writer task. `deleted_by`
    /// is always `UserCommand` for slash-command and CLI invocations;
    /// the panel's per-row `d` keeps using `PanelEdit { Delete }`.
    BulkForget {
        filter: super::store::ForgetFilter,
        dry_run: bool,
        reason: Option<String>,
        ack: Option<oneshot::Sender<Result<super::store::BulkForgetReport, String>>>,
    },
    /// Tier C / C2.4: `/forget-history` — redact a single history row
    /// in-place. The handler routes through
    /// [`super::store::MemoryStore::redact_history_row`], which is the
    /// **only** code path authorised to disable the C1.3 immutability
    /// trigger besides [`super::store::MemoryStore::compress_history_row`].
    /// One-way per the plan: the audit row stores the post-redaction
    /// tombstone, not the original transcript. `ack` carries the
    /// audit row's id on success.
    RedactHistory {
        memory_id: i64,
        reason: String,
        ack: Option<oneshot::Sender<Result<i64, String>>>,
    },
    /// Agent-raised "this memory is wrong or stale" signal, arriving from
    /// the `memory_flag` MCP tool through
    /// [`crate::mcp::signal::MemorySignalSink`].
    ///
    /// The handler demotes `trust_score` and writes an audit row. It never
    /// creates or deletes memory content, and it refuses user-authored and
    /// History rows outright (D1). `scope_level` / `repo_id` identify the
    /// owning physical DB, same as [`Self::PanelEdit`].
    AgentFlag {
        memory_id: i64,
        scope_level: i32,
        repo_id: Option<String>,
        reason: String,
        ack: Option<oneshot::Sender<Result<AgentFlagOutcome, String>>>,
    },
    /// Tier H / H1: undo a consolidation run by replaying the inverse
    /// of each audited operation. Always acked — a rollback is a
    /// user-initiated mutation whose result they are waiting on.
    ConsolidationRollback {
        run_id: String,
        ack: oneshot::Sender<Result<RollbackOutcome, String>>,
    },
    /// No-op drain barrier. Because the writer processes messages strictly
    /// FIFO, acking this guarantees every message enqueued before it has
    /// been fully processed. Used by headless callers ([`WriterHandle::flush`])
    /// to drain fire-and-forget work (e.g. swarm-finding extraction) before
    /// the process exits.
    Flush { ack: oneshot::Sender<()> },
}

/// Discrete operation for `WriterMessage::PanelEdit` (Tier A / A4).
#[derive(Debug, Clone)]
pub enum PanelEditOp {
    /// Remove the memory row from the store. No soft-delete here — the
    /// audit trail lives on Tier C2's `/forget`.
    Delete { memory_id: i64 },
    /// Raise `trust_score` on the row. Panel's `p` action sets it to
    /// 1.0; a user override can pass any [0.0, 1.0].
    Pin { memory_id: i64, trust_score: f32 },
    /// Migrate the row to a new scope. Reinserts at the new scope and
    /// deletes the original — cheaper than a bare UPDATE because of
    /// sqlite-vec's scope_level partition key.
    SetScope {
        memory_id: i64,
        new_scope: WriteScope,
    },
    /// Replace the row's content (and re-embed / re-hash).
    UpdateText { memory_id: i64, new_text: String },
}

impl WriterMessage {
    /// Stable, non-PII name for metrics / observer callbacks.
    pub fn kind(&self) -> &'static str {
        match self {
            WriterMessage::UserRemember { .. } => "UserRemember",
            WriterMessage::Store { .. } => "Store",
            WriterMessage::TurnComplete { .. } => "TurnComplete",
            WriterMessage::SwarmConsolidate { .. } => "SwarmConsolidate",
            WriterMessage::DeleteRun { .. } => "DeleteRun",
            WriterMessage::PanelEdit { .. } => "PanelEdit",
            WriterMessage::InjectionManifest { .. } => "InjectionManifest",
            WriterMessage::Sleeptime { .. } => "Sleeptime",
            WriterMessage::SessionConsolidate { .. } => "SessionConsolidate",
            WriterMessage::SessionConsolidateApply { .. } => "SessionConsolidateApply",
            WriterMessage::TelemetryClassify { .. } => "TelemetryClassify",
            WriterMessage::Restore { .. } => "Restore",
            WriterMessage::RestoreSince { .. } => "RestoreSince",
            WriterMessage::BulkForget { .. } => "BulkForget",
            WriterMessage::RedactHistory { .. } => "RedactHistory",
            WriterMessage::AgentFlag { .. } => "AgentFlag",
            WriterMessage::ConsolidationRollback { .. } => "ConsolidationRollback",
            WriterMessage::Flush { .. } => "Flush",
        }
    }
}

/// Construction config for the writer task.
pub struct WriterConfig {
    /// Multi-DB registry. The writer dispatches each operation to the
    /// store identified by [`WriteScope::target_store`] (or the workspace
    /// store for legacy / unscoped paths).
    pub stores: Arc<MemoryStores>,
    /// LLM used by `TurnComplete` extraction (Phase 4). `None` keeps the
    /// writer path alive but falls back to a low-importance run-scope record.
    pub llm: Option<Arc<dyn ConsolidationLlm>>,
    pub observer: Option<Arc<dyn MemoryObserver>>,
    /// Tier A / A4: notified after each `InjectionManifest` row lands,
    /// so the TUI memory panel can refresh its "Injected Now" section.
    pub manifest_observer: Option<Arc<dyn ManifestObserver>>,
}

/// Caller-facing handle to the writer task. Cheap to clone.
#[derive(Clone)]
pub struct WriterHandle {
    inner: Arc<WriterHandleInner>,
}

struct WriterHandleInner {
    tx: mpsc::UnboundedSender<WriterMessage>,
    observer: Option<Arc<dyn MemoryObserver>>,
    enqueued: AtomicU64,
    drained: AtomicU64,
}

/// Weak re-entry point into the writer's own queue.
///
/// The writer task holds only a `Weak` to the handle state on purpose
/// (see [`spawn_writer_task`]): a strong sender living inside the task
/// would keep the channel — and the SQLite handles behind it — alive
/// forever. Work the task spawns and that needs to enqueue a follow-up
/// message goes through this, and degrades to a plain error once the
/// last external handle has dropped.
#[derive(Clone)]
struct Reenqueue(Weak<WriterHandleInner>);

impl Reenqueue {
    /// Enqueue through a temporarily-reconstructed handle, so observer
    /// notifications and queue-depth accounting stay identical to an
    /// external caller's.
    fn send(&self, msg: WriterMessage) -> Result<()> {
        let inner = self
            .0
            .upgrade()
            .ok_or_else(|| anyhow!("writer task is shutting down"))?;
        WriterHandle { inner }.enqueue(msg)
    }
}

/// Run ids with a consolidation LLM call in flight.
///
/// Before the LLM moved off the writer task, two `/consolidate-session`
/// invocations on one conversation serialized behind the single
/// consumer. They now overlap, and because a consolidation run is keyed
/// by `run_id` — the conversation id — two concurrent runs write
/// interleaved audit rows that `recent_consolidation_runs` cannot tell
/// apart and `consolidation_rollback` cannot undo separately. A second
/// request for a run already in flight is refused instead.
type InFlightRuns = Arc<std::sync::Mutex<std::collections::HashSet<String>>>;

impl WriterHandle {
    /// Enqueue a raw message. Fires `on_write_enqueued` synchronously.
    /// Returns an error only if the writer task has terminated.
    pub fn enqueue(&self, msg: WriterMessage) -> Result<()> {
        let kind = msg.kind();
        if let Some(obs) = &self.inner.observer {
            obs.on_write_enqueued(kind);
        }
        let depth = self.inner.enqueued.fetch_add(1, Ordering::Relaxed) + 1
            - self.inner.drained.load(Ordering::Relaxed);
        tracing::debug!(
            target: "memory_writer",
            kind = kind,
            queue_depth = depth,
            "writer enqueue"
        );
        self.inner
            .tx
            .send(msg)
            .map_err(|_| anyhow!("writer task terminated"))
    }

    /// Enqueue a [`WriterMessage::Flush`] barrier and await it. Because the
    /// writer drains strictly FIFO, this resolves only once every message
    /// enqueued *before* this call has been fully processed — including
    /// slow fire-and-forget work like swarm-finding extraction. Headless
    /// callers (the CLI swarm) await this before exit so the runtime does
    /// not cancel in-flight extractor LLM calls. Resolves immediately if
    /// the writer task is already gone (nothing left to drain).
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        if self.enqueue(WriterMessage::Flush { ack: tx }).is_err() {
            return Ok(());
        }
        // A receive error means the task dropped its sender (terminated);
        // treat that as "drained" rather than propagating.
        let _ = rx.await;
        Ok(())
    }

    /// Enqueue a legacy-namespace `UserRemember` and await the ack with a
    /// 500ms timeout. Kept for pre-A2 call sites that don't resolve a
    /// `WriteScope`.
    pub async fn user_remember(
        &self,
        namespace: impl Into<String>,
        key: impl Into<String>,
        content: impl Into<String>,
        metadata: Option<String>,
    ) -> Result<WriteResult> {
        let (tx, rx) = oneshot::channel();
        let msg = WriterMessage::UserRemember {
            namespace: namespace.into(),
            key: key.into(),
            content: content.into(),
            metadata,
            scope: None,
            ack: Some(tx),
        };
        self.enqueue(msg)?;
        Self::await_ack(rx).await
    }

    /// A2: enqueue a scoped `UserRemember`. Routes through `store_scoped`
    /// with `MemoryMeta::for_source(MemorySource::UserRemember)` so the
    /// record lands in the proper scope level and carries `trust_score
    /// = 1.0`. Use this from the `/remember*` handlers.
    pub async fn user_remember_scoped(
        &self,
        scope: WriteScope,
        content: impl Into<String>,
    ) -> Result<WriteResult> {
        let (tx, rx) = oneshot::channel();
        // Namespace / key are still needed by the legacy columns; leave
        // them as a stable synthetic value — `store_scoped` derives its
        // own key from the scope path + content hash.
        let msg = WriterMessage::UserRemember {
            namespace: "user_remember".to_string(),
            key: "user_remember".to_string(),
            content: content.into(),
            metadata: None,
            scope: Some(scope),
            ack: Some(tx),
        };
        self.enqueue(msg)?;
        Self::await_ack(rx).await
    }

    async fn await_ack(rx: oneshot::Receiver<Result<WriteResult, String>>) -> Result<WriteResult> {
        Self::await_ack_within(rx, Duration::from_millis(ACK_TIMEOUT_MS)).await
    }

    /// [`Self::await_ack`] with a caller-supplied budget, for call sites
    /// that consolidate several writes under one overall deadline.
    async fn await_ack_within(
        rx: oneshot::Receiver<Result<WriteResult, String>>,
        budget: Duration,
    ) -> Result<WriteResult> {
        match tokio::time::timeout(budget, rx).await {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped ack channel")),
            Err(_) => Err(anyhow!("writer ack timeout after {}ms", budget.as_millis())),
        }
    }

    /// Enqueue a [`WriterMessage::AgentFlag`] and await the writer's
    /// decision under the standard ack budget. Used by the `memory_flag`
    /// MCP tool through [`crate::mcp::signal::MemorySignalSink`].
    pub async fn agent_flag(
        &self,
        memory_id: i64,
        scope_level: i32,
        repo_id: Option<String>,
        reason: impl Into<String>,
    ) -> Result<AgentFlagOutcome> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::AgentFlag {
            memory_id,
            scope_level,
            repo_id,
            reason: reason.into(),
            ack: Some(tx),
        })?;
        match tokio::time::timeout(Duration::from_millis(ACK_TIMEOUT_MS), rx).await {
            Ok(Ok(Ok(outcome))) => Ok(outcome),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped ack channel")),
            Err(_) => Err(anyhow!("writer ack timeout after {ACK_TIMEOUT_MS}ms")),
        }
    }

    /// Enqueue a `SwarmConsolidate`. `ack.is_some()` blocks the caller on
    /// the subsequent `await`; `None` is fire-and-forget.
    pub fn swarm_consolidate(
        &self,
        scope: WriteScope,
        content: impl Into<String>,
        meta: WriteMeta,
    ) -> Result<()> {
        self.enqueue(WriterMessage::SwarmConsolidate {
            scope,
            content: content.into(),
            meta,
            ack: None,
        })
    }

    /// Enqueue a legacy namespace/key write with full store options.
    pub fn store_with_options(
        &self,
        namespace: impl Into<String>,
        key: impl Into<String>,
        content: impl Into<String>,
        options: StoreOptions,
    ) -> Result<()> {
        self.enqueue(WriterMessage::Store {
            namespace: namespace.into(),
            key: key.into(),
            content: content.into(),
            options,
            ack: None,
        })
    }

    /// Enqueue a consolidation write and wait for the writer ack.
    pub async fn swarm_consolidate_wait(
        &self,
        scope: WriteScope,
        content: impl Into<String>,
        meta: WriteMeta,
    ) -> Result<WriteResult> {
        self.swarm_consolidate_wait_within(
            scope,
            content,
            meta,
            Duration::from_millis(ACK_TIMEOUT_MS),
        )
        .await
    }

    /// [`Self::swarm_consolidate_wait`] bounded by an explicit budget.
    /// Consolidation promotes a whole run's worth of rows one message at
    /// a time and must still terminate, so it spends a single wall-clock
    /// budget across the batch rather than [`ACK_TIMEOUT_MS`] per row.
    pub async fn swarm_consolidate_wait_within(
        &self,
        scope: WriteScope,
        content: impl Into<String>,
        meta: WriteMeta,
        budget: Duration,
    ) -> Result<WriteResult> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::SwarmConsolidate {
            scope,
            content: content.into(),
            meta,
            ack: Some(tx),
        })?;
        Self::await_ack_within(rx, budget).await
    }

    /// Delete all memories associated with a run through the writer task.
    pub async fn delete_run(&self, run_id: impl Into<String>) -> Result<WriteResult> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::DeleteRun {
            run_id: run_id.into(),
            ack: Some(tx),
        })?;
        Self::await_ack(rx).await
    }

    /// C2.2: restore a soft-deleted memory by audit id. Embedding +
    /// dedup happen inside the writer task so the caller doesn't need
    /// to hold the store mutex; ack timeout is bumped to 30s because
    /// re-embedding restored content can dwarf the default 500ms used
    /// for hash-only writes.
    pub async fn restore_deletion(&self, deletion_id: i64) -> Result<super::store::RestoreOutcome> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::Restore {
            deletion_id,
            ack: Some(tx),
        })?;
        Self::await_restore_ack(rx).await
    }

    /// C2.2: restore every still-pending deletion newer than
    /// `since_sql_offset` (a SQLite relative-datetime spec, e.g.
    /// `"-7 days"`). Returns the per-id outcome list so the caller
    /// can render a summary.
    pub async fn restore_deletions_since(
        &self,
        since_sql_offset: impl Into<String>,
    ) -> Result<Vec<super::store::RestoreOutcome>> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::RestoreSince {
            since_sql_offset: since_sql_offset.into(),
            ack: Some(tx),
        })?;
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped restore-since ack channel")),
            Err(_) => Err(anyhow!("restore-since ack timeout after 30s")),
        }
    }

    async fn await_restore_ack(
        rx: oneshot::Receiver<Result<super::store::RestoreOutcome, String>>,
    ) -> Result<super::store::RestoreOutcome> {
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped restore ack channel")),
            Err(_) => Err(anyhow!("restore ack timeout after 30s")),
        }
    }

    /// C2.4: enqueue a `/forget-history` redaction. The caller is
    /// expected to have already collected the two-step `REDACT` user
    /// confirmation; this handle just funnels the message into the
    /// writer task. Returns the audit row id on success. The CI grep
    /// check on `drop_history_immutable_triggers` enforces that the
    /// store-side handler stays the only trigger-disable callsite
    /// besides compression.
    pub async fn redact_history(&self, memory_id: i64, reason: impl Into<String>) -> Result<i64> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::RedactHistory {
            memory_id,
            reason: reason.into(),
            ack: Some(tx),
        })?;
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(audit_id))) => Ok(audit_id),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped redact-history ack channel")),
            Err(_) => Err(anyhow!("redact-history ack timeout after 30s")),
        }
    }

    /// C2.3: enqueue a bulk soft-delete. `dry_run = true` returns the
    /// preview report (counts + breakdowns) without writing; live
    /// calls write one audit row per deleted memory. Always tagged
    /// `DeletedBy::UserCommand`; the panel's per-row `d` action
    /// continues to flow through [`WriterMessage::PanelEdit`].
    pub async fn bulk_forget(
        &self,
        filter: super::store::ForgetFilter,
        dry_run: bool,
        reason: Option<String>,
    ) -> Result<super::store::BulkForgetReport> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::BulkForget {
            filter,
            dry_run,
            reason,
            ack: Some(tx),
        })?;
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped bulk-forget ack channel")),
            Err(_) => Err(anyhow!("bulk-forget ack timeout after 30s")),
        }
    }

    /// Fire-and-forget turn-complete notification (Phase 4 will handle extraction).
    pub fn turn_complete(
        &self,
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        repo_id: impl Into<String>,
        module_path: Option<String>,
        run_id: impl Into<String>,
        transcript: impl Into<String>,
        annotations: Option<JsonValue>,
    ) -> Result<()> {
        self.enqueue(WriterMessage::TurnComplete {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            repo_id: repo_id.into(),
            module_path,
            run_id: run_id.into(),
            transcript: transcript.into(),
            annotations,
        })
    }

    pub fn is_alive(&self) -> bool {
        !self.inner.tx.is_closed()
    }

    /// Tier B / B5: enqueue an end-of-session consolidator run.
    /// `ack.is_some()` blocks the caller until the writer applies the
    /// LLM's operations.
    pub async fn session_consolidate(
        &self,
        session_id: impl Into<String>,
        repo_id: impl Into<String>,
        module_path: Option<String>,
        run_id: impl Into<String>,
        transcript: impl Into<String>,
    ) -> Result<WriteResult> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::SessionConsolidate {
            session_id: session_id.into(),
            repo_id: repo_id.into(),
            module_path,
            run_id: run_id.into(),
            transcript: transcript.into(),
            ack: Some(tx),
        })?;
        Self::await_ack(rx).await
    }

    /// Tier H / H1: undo a consolidation run, blocking until the
    /// writer has replayed every inverse operation.
    pub async fn consolidation_rollback(
        &self,
        run_id: impl Into<String>,
    ) -> Result<RollbackOutcome> {
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::ConsolidationRollback {
            run_id: run_id.into(),
            ack: tx,
        })?;
        match tokio::time::timeout(Duration::from_millis(ACK_TIMEOUT_MS), rx).await {
            Ok(Ok(Ok(outcome))) => Ok(outcome),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped ack channel")),
            Err(_) => Err(anyhow!("writer ack timeout after {ACK_TIMEOUT_MS}ms")),
        }
    }

    /// Tier B / B5: enqueue a sleeptime pass. Fire-and-forget; the
    /// caller observes progress via [`SleeptimeObserver`] events.
    pub fn sleeptime(&self, payload: JsonValue) -> Result<()> {
        self.enqueue(WriterMessage::Sleeptime { payload })
    }

    /// Tier B / B6: enqueue a post-turn telemetry classification.
    /// Fire-and-forget; never blocks the user.
    pub fn telemetry_classify(
        &self,
        turn_id: impl Into<String>,
        session_id: impl Into<String>,
        response: impl Into<String>,
    ) -> Result<()> {
        self.enqueue(WriterMessage::TelemetryClassify {
            turn_id: turn_id.into(),
            session_id: session_id.into(),
            response: response.into(),
        })
    }

    /// Current in-flight queue depth (enqueued minus drained). Exposed for
    /// Phase 2–4 load monitoring.
    pub fn queue_depth(&self) -> u64 {
        let enq = self.inner.enqueued.load(Ordering::Relaxed);
        let drn = self.inner.drained.load(Ordering::Relaxed);
        enq.saturating_sub(drn)
    }
}

/// Spawn the writer task and return a handle. Safe to call once per workspace.
pub fn spawn_writer_task(cfg: WriterConfig) -> WriterHandle {
    let (tx, rx) = mpsc::unbounded_channel::<WriterMessage>();
    let observer = cfg.observer.clone();
    let inner = Arc::new(WriterHandleInner {
        tx,
        observer: observer.clone(),
        enqueued: AtomicU64::new(0),
        drained: AtomicU64::new(0),
    });
    let handle = WriterHandle {
        inner: inner.clone(),
    };
    // The task gets only a Weak reference to the handle state: `inner`
    // owns the channel's sole sender, so an Arc here would keep the
    // channel open from inside the task and make it immortal — the
    // task (and its `stores` SQLite handles) must instead exit once
    // every external `WriterHandle` has dropped and the queue drained.
    // (On Windows a leaked task pins `memory.db` against deletion.)
    tokio::spawn(writer_task(rx, cfg, Arc::downgrade(&inner)));
    handle
}

/// Main drain loop. One message at a time — serialization of writes is the
/// whole point. If this ever needs parallelism, bound the channel and fan
/// out within the task body; do not add a second consumer to the channel.
async fn writer_task(
    mut rx: mpsc::UnboundedReceiver<WriterMessage>,
    cfg: WriterConfig,
    state: Weak<WriterHandleInner>,
) {
    let WriterConfig {
        stores,
        llm,
        observer,
        manifest_observer,
    } = cfg;
    let reenqueue = Reenqueue(state.clone());
    let in_flight: InFlightRuns = Default::default();

    while let Some(msg) = rx.recv().await {
        // Drain barrier: ack and move on. Reaching it in FIFO order means
        // every prior message has already been processed to completion.
        if let WriterMessage::Flush { ack } = msg {
            if let Some(s) = state.upgrade() {
                s.drained.fetch_add(1, Ordering::Relaxed);
            }
            let _ = ack.send(());
            continue;
        }
        let kind = msg.kind();
        // `None` upgrade = every handle already dropped; we're just
        // draining the tail of the queue before exiting.
        let drained = state
            .upgrade()
            .map(|s| s.drained.fetch_add(1, Ordering::Relaxed) + 1)
            .unwrap_or(0);
        tracing::debug!(
            target: "memory_writer",
            kind = kind,
            drained = drained,
            "writer drain"
        );

        // Snapshot manifest id pair before the message is consumed so
        // `ManifestObserver::on_manifest_persisted` can fire with the
        // right key after a successful commit.
        let manifest_ids: Option<(String, String)> = match &msg {
            WriterMessage::InjectionManifest {
                turn_id,
                session_id,
                ..
            } => Some((turn_id.clone(), session_id.clone())),
            _ => None,
        };

        let outcome = process_message(&stores, llm.as_ref(), &reenqueue, &in_flight, msg).await;
        match outcome {
            Ok(result) => {
                if let Some(obs) = &observer {
                    obs.on_write_committed(kind, &result);
                }
                if let (Some(obs), Some((turn_id, session_id))) =
                    (&manifest_observer, &manifest_ids)
                {
                    obs.on_manifest_persisted(turn_id, session_id);
                }
            }
            Err(err) => {
                tracing::error!(
                    target: "memory_writer",
                    kind = kind,
                    error = %err,
                    "writer task: message failed"
                );
                if let Some(obs) = &observer {
                    obs.on_write_failed(kind, &err.to_string());
                }
            }
        }
    }

    tracing::info!(target: "memory_writer", "writer task exiting (channel closed)");
}

async fn process_message(
    stores: &Arc<MemoryStores>,
    llm: Option<&Arc<dyn ConsolidationLlm>>,
    reenqueue: &Reenqueue,
    in_flight: &InFlightRuns,
    msg: WriterMessage,
) -> Result<WriteResult> {
    match msg {
        WriterMessage::UserRemember {
            namespace,
            key,
            content,
            metadata,
            scope,
            ack,
        } => match scope {
            Some(scope) => {
                // A2/A3: scoped user_remember. `WriteMeta` is built with
                // `MemorySource::UserRemember` (trust_score = 1.0) and
                // importance bumped to 0.8 per the legacy `/remember`
                // behaviour. Cross-DB dedup against broader scopes
                // happens inside `MemoryStores::store_scoped`.
                let meta = WriteMeta::for_source(MemorySource::UserRemember).with_importance(0.8);
                let res = stores
                    .store_scoped(&scope, &content, &meta)
                    .await
                    .map(WriteResult::from);
                send_ack(ack, &res);
                res
            }
            None => {
                // Legacy path: namespace/key write for pre-A2 callers.
                // Routes to workspace store (legacy semantics).
                let opts = StoreOptions {
                    importance: 0.8,
                    metadata,
                    ..Default::default()
                };
                let store = stores.workspace().clone();
                let res = store
                    .store_with_options(&namespace, &key, &content, &opts)
                    .await
                    .map(WriteResult::Inserted);
                send_ack(ack, &res);
                res
            }
        },
        WriterMessage::Store {
            namespace,
            key,
            content,
            options,
            ack,
        } => {
            // Legacy namespace/key — workspace store.
            let store = stores.workspace().clone();
            let res = store
                .store_with_options(&namespace, &key, &content, &options)
                .await
                .map(WriteResult::Inserted);
            send_ack(ack, &res);
            res
        }
        WriterMessage::SwarmConsolidate {
            scope,
            content,
            meta,
            ack,
        } => {
            let res = stores
                .store_scoped(&scope, &content, &meta)
                .await
                .map(WriteResult::from);
            send_ack(ack, &res);
            res
        }
        WriterMessage::DeleteRun { run_id, ack } => {
            // Run rows live in the workspace store (per StoreKind routing).
            let store = stores.workspace().clone();
            let res = store.delete_by_run(&run_id).await.map(|deleted| {
                if deleted == 0 {
                    WriteResult::Skipped
                } else {
                    WriteResult::Inserted(deleted as i64)
                }
            });
            send_ack(ack, &res);
            res
        }
        WriterMessage::PanelEdit {
            op,
            scope_level,
            repo_id,
            ack,
        } => {
            // Route to the DB that owns the row. No workspace fallback:
            // an unresolvable (scope_level, repo_id) pair means we would
            // be mutating an unrelated row that happens to share the id
            // in another DB.
            let res = match store_kind_for_scope(scope_level, repo_id.as_deref()) {
                Some(kind) => match stores.get(&kind).await {
                    Ok(store) => process_panel_edit(&store, op).await,
                    Err(e) => Err(e),
                },
                None => Err(anyhow!(
                    "panel edit: cannot resolve the store owning scope_level \
                     {scope_level} (repo_id {repo_id:?}) — refusing to guess"
                )),
            };
            send_ack(ack, &res);
            res
        }
        WriterMessage::AgentFlag {
            memory_id,
            scope_level,
            repo_id,
            reason,
            ack,
        } => {
            let res =
                process_agent_flag(stores, memory_id, scope_level, repo_id.as_deref(), &reason)
                    .await;
            if let Some(tx) = ack {
                let payload = match &res {
                    Ok(outcome) => Ok(outcome.clone()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = tx.send(payload);
            }
            res.map(|outcome| {
                if outcome.accepted {
                    WriteResult::Inserted(memory_id)
                } else {
                    WriteResult::Skipped
                }
            })
        }
        WriterMessage::TurnComplete {
            session_id,
            turn_id,
            repo_id,
            module_path,
            run_id,
            transcript,
            annotations,
        } => {
            // Annotation flags and extractor rows carry their own scope,
            // which may target any of the three physical DBs — the whole
            // registry goes in so `MemoryStores::store_scoped` can route
            // by `WriteScope::target_store`. History rows and the session
            // ledger stay on the workspace store (see below).
            process_turn_complete(
                stores,
                llm,
                session_id,
                turn_id,
                repo_id,
                module_path,
                run_id,
                transcript,
                annotations,
            )
            .await
        }
        WriterMessage::InjectionManifest {
            turn_id,
            session_id,
            payload,
        } => {
            // S4: persist the manifest, fire-and-forget semantics. Any
            // error is returned to the writer loop which logs it —
            // manifest-write failure never fails the turn.
            // Forensic log lives in the workspace store.
            let store = stores.workspace().clone();
            let id = store
                .store_injection_manifest(&turn_id, &session_id, "chat", &payload)
                .await
                .map_err(|e| anyhow!("persist injection_manifests: {e}"))?;
            Ok(WriteResult::Inserted(id))
        }
        WriterMessage::Sleeptime { payload } => {
            let cfg = parse_sleeptime_payload(&payload);
            // Sleeptime currently runs against the workspace store only.
            // Step 7 fans out to per-folder stores.
            let store = stores.workspace().clone();
            match super::sleeptime::run_sleeptime(&store, &cfg, None).await {
                Ok(report) => {
                    tracing::info!(
                        target: "memory_sleeptime",
                        run_id = %report.run_id,
                        dry_run = report.dry_run,
                        decay_flagged = report.decay_flagged,
                        near_dup_merged = report.near_dup_merged,
                        promoted = report.promoted,
                        trust_adjusted = report.trust_adjusted,
                        telemetry_pruned = report.telemetry_pruned,
                        "sleeptime complete"
                    );
                    // Phase 2 (Tier B / B5): stamp `last_sleeptime_at`
                    // so the scheduler honours the 24h gating across
                    // process restarts. Live runs only — dry-runs
                    // intentionally don't update the timestamp so
                    // exploratory `--sleep-dry-run` invocations don't
                    // suppress real passes.
                    if !report.dry_run
                        && let Err(e) = store
                            .set_meta_value("last_sleeptime_at", &chrono::Utc::now().to_rfc3339())
                            .await
                    {
                        tracing::warn!(
                            target: "memory_sleeptime",
                            error = %e,
                            "failed to stamp last_sleeptime_at"
                        );
                    }
                    Ok(WriteResult::Skipped)
                }
                Err(e) => Err(anyhow!("sleeptime: {e}")),
            }
        }
        WriterMessage::SessionConsolidate {
            session_id,
            repo_id,
            module_path,
            run_id,
            transcript,
            ack,
        } => {
            // Session consolidation reads run rows from workspace, may
            // promote into folder-scoped rows. Step 7 wires the cross-DB
            // promotion path; for now both stay in the workspace store
            // (legacy single-DB behaviour).
            let store = stores.workspace().clone();
            let res = start_session_consolidate(
                &store,
                llm,
                reenqueue,
                in_flight,
                session_id,
                repo_id,
                module_path,
                run_id,
                transcript,
            )
            .await;
            send_ack(ack, &res);
            res
        }
        WriterMessage::SessionConsolidateApply {
            repo_id,
            module_path,
            run_id,
            candidates,
            parsed,
            ack,
        } => {
            let store = stores.workspace().clone();
            // Release the in-flight slot whatever happens, so a failed
            // run never wedges the conversation out of consolidating
            // again.
            let res = apply_session_consolidate(
                &store,
                repo_id,
                module_path,
                &run_id,
                &candidates,
                *parsed,
            )
            .await;
            if let Ok(mut guard) = in_flight.lock() {
                guard.remove(&run_id);
            }
            send_ack(ack, &res);
            res
        }
        WriterMessage::TelemetryClassify {
            turn_id,
            session_id,
            response,
        } => {
            let store = stores.workspace().clone();
            process_telemetry_classify(&store, &turn_id, &session_id, &response).await
        }
        WriterMessage::ConsolidationRollback { run_id, ack } => {
            // Consolidation writes land in the workspace store, so its
            // audit trail and the rows to reverse are both there.
            let store = stores.workspace().clone();
            let res = process_consolidation_rollback(&store, &run_id).await;
            send_ack_typed(Some(ack), &res);
            res.map(|_| WriteResult::Skipped)
        }
        WriterMessage::Restore { deletion_id, ack } => {
            // C2.2: deletions audit is per-DB, and current soft-delete
            // call sites only land into workspace. Step 7 generalises
            // by walking opened stores; today workspace is sufficient.
            let store = stores.workspace().clone();
            let res = store.restore_deletion(deletion_id).await;
            send_ack_typed(ack, &res);
            res.map(|outcome| match outcome {
                super::store::RestoreOutcome::Inserted { new_memory_id, .. } => {
                    WriteResult::Inserted(new_memory_id)
                }
                super::store::RestoreOutcome::Deduplicated {
                    surviving_memory_id,
                    ..
                } => WriteResult::Deduplicated(surviving_memory_id),
                super::store::RestoreOutcome::AlreadyCovered { .. } => WriteResult::AlreadyCovered,
                super::store::RestoreOutcome::Refused { .. } => WriteResult::Skipped,
            })
        }
        WriterMessage::RestoreSince {
            since_sql_offset,
            ack,
        } => {
            let store = stores.workspace().clone();
            let res = store.restore_deletions_since(&since_sql_offset).await;
            send_ack_typed(ack, &res);
            res.map(|_| WriteResult::Skipped)
        }
        WriterMessage::RedactHistory {
            memory_id,
            reason,
            ack,
        } => {
            let store = stores.workspace().clone();
            let res = store.redact_history_row(memory_id, &reason).await;
            send_ack_typed(ack, &res);
            res.map(|audit_id| WriteResult::Inserted(audit_id))
        }
        WriterMessage::Flush { ack } => {
            // The receive loop intercepts `Flush` as a drain barrier before
            // reaching here; this arm only satisfies match exhaustiveness.
            // Ack defensively in case that ever changes.
            let _ = ack.send(());
            Ok(WriteResult::Skipped)
        }
        WriterMessage::BulkForget {
            filter,
            dry_run,
            reason,
            ack,
        } => {
            let store = stores.workspace().clone();
            let res = store
                .bulk_forget(
                    &filter,
                    dry_run,
                    reason.as_deref(),
                    super::deletions::DeletedBy::UserCommand,
                )
                .await;
            send_ack_typed(ack, &res);
            res.map(|report| {
                if dry_run {
                    WriteResult::Skipped
                } else if report.deleted == 0 {
                    WriteResult::Skipped
                } else {
                    WriteResult::Inserted(report.deleted as i64)
                }
            })
        }
    }
}

/// C2.2: shared `send_ack` helper for variants whose ack carries a
/// non-`WriteResult` payload (e.g. [`super::store::RestoreOutcome`]).
/// Mirrors [`send_ack`] but is generic over the payload so each
/// variant can have its own oneshot type without duplicated code.
fn send_ack_typed<T: Clone>(ack: Option<oneshot::Sender<Result<T, String>>>, res: &Result<T>) {
    if let Some(tx) = ack {
        let payload = match res {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(payload);
    }
}

/// Resolve sleeptime config from a writer-message payload. Empty
/// payload → defaults; `{"dry_run": true}` flips dry-run.
fn parse_sleeptime_payload(payload: &JsonValue) -> super::sleeptime::SleeptimeConfig {
    let mut cfg = super::sleeptime::SleeptimeConfig::default();
    if let Some(b) = payload.get("dry_run").and_then(|v| v.as_bool()) {
        cfg.dry_run = b;
    }
    if let Some(t) = payload.get("near_dup_threshold").and_then(|v| v.as_f64()) {
        cfg.near_dup_threshold = t as f32;
    }
    if let Some(n) = payload.get("trust_min_injections").and_then(|v| v.as_u64()) {
        cfg.trust_min_injections = n as u32;
    }
    cfg
}

/// Everything a consolidation run needs before it can ask the model.
struct ConsolidationPrep {
    /// The CANDIDATES list, in prompt order. The apply phase resolves
    /// `candidate_index` against exactly this slice, so it must travel
    /// with the response.
    candidates: Vec<super::session_consolidator::CandidateBrief>,
    prompt: String,
}

/// Read the inputs for one consolidation run and render the prompt.
///
/// Split out of [`start_session_consolidate`] so the tests can drive a
/// whole run inline (prepare → model → apply) while production keeps the
/// model call off the writer task. Both paths therefore exercise the
/// same candidate selection and the same prompt.
///
/// `None` means "nothing to consolidate" — the session produced no
/// run-scoped rows at all.
async fn prepare_session_consolidate(
    store: &Arc<MemoryStore>,
    repo_id: &str,
    module_path: &Option<String>,
    run_id: &str,
    transcript: &str,
) -> Option<ConsolidationPrep> {
    use super::session_consolidator::{
        CandidateBrief, ExistingBrief, MAX_EXISTING_MEMORIES, build_prompt,
    };

    let recent = match store.recent_memories_for_run(run_id, 24, 50).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(target: "memory_consolidator", error = %e, "skipping consolidate");
            return None;
        }
    };
    if recent.is_empty() {
        return None;
    }

    // A-fix: History rows are not consolidation candidates.
    //
    // `recent_memories_for_run` returns every run-scoped row, which
    // includes the verbatim `RawTranscript` turn records. Shown as
    // undifferentiated CANDIDATES, the model dutifully kept them: the
    // first live run ADDed five raw `USER:` / `ASSISTANT:` blobs (2.7–5.4
    // KB each) into the durable repo record at importance 1.0. That is
    // History being copied into Records — the transcript is already in
    // the prompt's own TRANSCRIPT section, and the session summary is
    // the intended distillation of it.
    //
    // An all-transcript session still runs: it yields an empty CANDIDATES
    // list and a session summary, which is the honest output for a
    // session that extracted nothing.
    let candidates: Vec<CandidateBrief> = recent
        .iter()
        .filter(|m| m.source != MemorySource::RawTranscript)
        .map(|m| CandidateBrief {
            text: m.content.clone(),
            kind: m.memory_type.as_str().to_string(),
            importance: m.importance,
        })
        .collect();

    // C-5 fix: the prompt used to be built with an empty existing set,
    // so every id the model produced was necessarily invented and every
    // MERGE / SUPERSEDE it emitted was rejected at apply time. Give it
    // the rows it is actually allowed to name.
    //
    // Ranking fix: *which* rows matters as much as having any. Ranked by
    // relevance to this session's candidates rather than by importance —
    // see `consolidation_existing_memories` for why importance order
    // made MERGE structurally impossible.
    //
    // With no candidates there is nothing to rank against, and no
    // MERGE / SUPERSEDE can be valid anyway: both carry a
    // `candidate_index`. Skip the lookup and let the prompt say so.
    let landing_scope_level = if module_path.is_some() {
        super::scope::SCOPE_MODULE
    } else {
        super::scope::SCOPE_REPO
    };
    let existing: Vec<ExistingBrief> = if candidates.is_empty() {
        Vec::new()
    } else {
        let ranking_query = candidates
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        store
            .consolidation_existing_memories(
                landing_scope_level,
                Some(repo_id),
                MAX_EXISTING_MEMORIES,
                Some(&ranking_query),
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    target: "memory_consolidator",
                    error = %e,
                    "could not load existing memories; MERGE/SUPERSEDE will have no valid targets"
                );
                Vec::new()
            })
            .into_iter()
            .map(|r| ExistingBrief {
                id: r.id,
                text: r.content,
                kind: r.memory_type,
                scope_label: r.scope_label,
            })
            .collect()
    };

    let history = load_consolidation_history(store).await;
    let prompt = build_prompt(transcript, &candidates, &existing, &history);
    Some(ConsolidationPrep { candidates, prompt })
}

/// Phase 1: gather inputs, build the prompt, hand the model call to a
/// background task. Everything here is a cheap read; the expensive part
/// deliberately leaves the writer task before returning.
#[allow(clippy::too_many_arguments)]
async fn start_session_consolidate(
    store: &Arc<MemoryStore>,
    llm: Option<&Arc<dyn ConsolidationLlm>>,
    reenqueue: &Reenqueue,
    in_flight: &InFlightRuns,
    _session_id: String,
    repo_id: String,
    module_path: Option<String>,
    run_id: String,
    transcript: String,
) -> Result<WriteResult> {
    use super::session_consolidator::parse_response;

    let Some(llm) = llm else {
        tracing::warn!(target: "memory_consolidator", "no LLM configured; skipping session consolidate");
        return Ok(WriteResult::Skipped);
    };

    // B5 fix: pull this session's extractions only. The pre-fix code
    // queried `recent_memories(24, 50)` and filtered to scope_level
    // ≥ Run, which leaked memories from any other session whose run
    // rows hadn't been promoted yet. We now scope by `run_id` (the
    // canonical session identifier in this codebase — `session_id`
    // remains in the message for forward-compat with future
    // multi-run sessions but is unused here, see comment in the
    // WriterMessage variant).
    if run_id.is_empty() {
        tracing::warn!(
            target: "memory_consolidator",
            "session consolidate requested with empty run_id; skipping to avoid \
             cross-session leak"
        );
        return Ok(WriteResult::Skipped);
    }
    // Refuse a second concurrent run for the same conversation. A run is
    // identified by `run_id` throughout the audit trail, so overlapping
    // runs are indistinguishable afterwards and cannot be rolled back
    // apart. Claim the slot before the spawn and release it in the apply
    // arm (or below, on any early return).
    match in_flight.lock() {
        Ok(mut guard) => {
            if !guard.insert(run_id.clone()) {
                tracing::warn!(
                    target: "memory_consolidator",
                    %run_id,
                    "a consolidation for this session is already running; ignoring"
                );
                return Ok(WriteResult::Skipped);
            }
        }
        Err(e) => {
            tracing::warn!(target: "memory_consolidator", error = %e, "in-flight set poisoned");
            return Ok(WriteResult::Skipped);
        }
    }
    // Any early return from here on must give the slot back.
    let release = |in_flight: &InFlightRuns, run_id: &str| {
        if let Ok(mut guard) = in_flight.lock() {
            guard.remove(run_id);
        }
    };

    let Some(prep) =
        prepare_session_consolidate(store, &repo_id, &module_path, &run_id, &transcript).await
    else {
        release(in_flight, &run_id);
        return Ok(WriteResult::Skipped);
    };
    let ConsolidationPrep { candidates, prompt } = prep;

    // B-fix: the model call leaves the writer task here.
    //
    // Everything above is SQLite reads measured in milliseconds. What
    // follows is a full round-trip on a session-sized prompt. Running it
    // inline blocked every other memory write for its duration and blew
    // the caller's ack budget; the writes it produces still land on the
    // writer, as `SessionConsolidateApply`.
    let llm = Arc::clone(llm);
    let reenqueue = reenqueue.clone();
    let in_flight = Arc::clone(in_flight);
    let spawn_run_id = run_id.clone();
    tokio::spawn(async move {
        let finish = |in_flight: &InFlightRuns| {
            if let Ok(mut guard) = in_flight.lock() {
                guard.remove(&spawn_run_id);
            }
        };
        let raw = match llm.complete(prompt).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(target: "memory_consolidator", error = %e, "LLM failure; deferring");
                finish(&in_flight);
                return;
            }
        };
        let parsed = match parse_response(&raw) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(target: "memory_consolidator", error = %e, "parse failed; logging raw");
                finish(&in_flight);
                return;
            }
        };
        // Ownership of the in-flight slot passes to the apply arm, which
        // releases it once the operations have landed.
        if let Err(e) = reenqueue.send(WriterMessage::SessionConsolidateApply {
            repo_id,
            module_path,
            run_id: spawn_run_id.clone(),
            candidates,
            parsed: Box::new(parsed),
            ack: None,
        }) {
            tracing::warn!(
                target: "memory_consolidator",
                error = %e,
                "consolidation finished but its operations could not be enqueued"
            );
            finish(&in_flight);
        }
    });

    Ok(WriteResult::Queued)
}

/// Identify one consolidation invocation.
///
/// Ten hex characters — short enough to retype after
/// `/consolidate history`, and sortable, since the high bits are the
/// timestamp. The counter makes two consolidations in the same second
/// distinct, which a bare timestamp would not.
fn new_batch_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:08x}{:02x}", ts as u32, seq & 0xFF)
}

/// Phase 2: apply the operations the model produced. Runs on the writer
/// task — every write in this function is why the split exists.
async fn apply_session_consolidate(
    store: &Arc<MemoryStore>,
    repo_id: String,
    module_path: Option<String>,
    run_id: &str,
    candidates: &[super::session_consolidator::CandidateBrief],
    parsed: super::session_consolidator::ConsolidatorResponse,
) -> Result<WriteResult> {
    // One identity for every row this invocation writes.
    let batch_id = new_batch_id();
    let batch_id = batch_id.as_str();
    // Apply: ADD as a session_summary memory + apply each op against
    // the matching candidate. MERGE / SUPERSEDE / DROP are best-effort
    // and never block on each other.
    let summary_scope = if module_path.is_some() {
        WriteScope::Module {
            repo_id: repo_id.clone(),
            module_path: module_path.clone().unwrap_or_default(),
        }
    } else {
        WriteScope::Repo {
            repo_id: repo_id.clone(),
        }
    };
    let scope_label = match &module_path {
        Some(m) => format!("module:{m}"),
        None => "repo".to_string(),
    };

    if !parsed.session_summary.trim().is_empty() {
        // C1: the consolidator's session-summary blob is the canonical
        // Summary kind row — semantic merge across similar sessions,
        // injected as session context when topically relevant.
        let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
            .with_importance(0.7)
            .with_type(super::scope::MemoryType::Factual)
            .with_kind(super::kind::MemoryKind::Summary)
            .with_tag(format!("session_summary:{run_id}"));
        let outcome = store
            .store_scoped(&summary_scope, &parsed.session_summary, &meta)
            .await;
        let (memory_id, error) = match outcome {
            Ok(res) => (stored_id(&res), None),
            Err(e) => {
                tracing::warn!(
                    target: "memory_consolidator",
                    error = %e,
                    "session summary could not be stored"
                );
                (None, Some(format!("{e:#}")))
            }
        };
        record_consolidation_op(
            store,
            ConsolidationAudit {
                run_id,
                batch_id,
                kind: "session_summary",
                memory_id,
                related_id: None,
                op_json: &json!({ "op": "SESSION_SUMMARY" }).to_string(),
                before_json: None,
                after_json: None,
                error: error.as_deref(),
                scope: &scope_label,
                expected_outcome: None,
            },
        )
        .await;
    }

    for annotated in parsed.operations {
        use super::session_consolidator::ConsolidationOp;

        let op_json = serde_json::to_string(&annotated).unwrap_or_else(|_| "null".to_string());
        let expected = annotated.expected_outcome.clone();
        let op = annotated.op;
        let kind = op.kind_str();

        // Validate references *before* touching the store. An op naming
        // a candidate that does not exist, or a memory id the model
        // invented, used to be silently skipped by `if let Some(..)` —
        // indistinguishable from an op that ran.
        if candidates.get(op.candidate_index()).is_none()
            && !matches!(op, ConsolidationOp::Merge { .. })
        {
            let error = format!(
                "candidate_index {} out of range (batch had {} candidates)",
                op.candidate_index(),
                candidates.len()
            );
            tracing::warn!(target: "memory_consolidator", %error, op = kind, "rejecting op");
            record_consolidation_op(
                store,
                ConsolidationAudit {
                    run_id,
                    batch_id,
                    kind,
                    memory_id: None,
                    related_id: None,
                    op_json: &op_json,
                    before_json: None,
                    after_json: None,
                    error: Some(&error),
                    scope: &scope_label,
                    expected_outcome: expected.as_deref(),
                },
            )
            .await;
            continue;
        }

        match op {
            ConsolidationOp::Add { candidate_index } => {
                let c = &candidates[candidate_index];
                let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
                    .with_importance(c.importance)
                    .with_type(super::scope::MemoryType::parse_str(&c.kind));
                let (memory_id, error) =
                    match store.store_scoped(&summary_scope, &c.text, &meta).await {
                        Ok(res) => (stored_id(&res), None),
                        Err(e) => {
                            tracing::warn!(target: "memory_consolidator", error = %e, "ADD failed");
                            (None, Some(format!("{e:#}")))
                        }
                    };
                record_consolidation_op(
                    store,
                    ConsolidationAudit {
                        run_id,
                        batch_id,
                        kind,
                        memory_id,
                        related_id: None,
                        op_json: &op_json,
                        before_json: None,
                        after_json: None,
                        error: error.as_deref(),
                        scope: &scope_label,
                        expected_outcome: expected.as_deref(),
                    },
                )
                .await;
            }

            ConsolidationOp::Merge { into_memory_id, .. } => {
                // Best-effort trust bump on the surviving row;
                // text-merge is deferred (the LLM picked the existing
                // row to "fold into" — keeping its content).
                let before = store.get_memory_row(into_memory_id).await.ok().flatten();
                let (error, before_json) = match &before {
                    None => (
                        Some(format!(
                            "merge target memory {into_memory_id} does not exist"
                        )),
                        None,
                    ),
                    Some(row) => {
                        let before_json =
                            json!({ "trust_score": row.trust_score, "memory_id": row.id })
                                .to_string();
                        match store.set_trust_score(into_memory_id, MERGE_TRUST).await {
                            Ok(()) => (None, Some(before_json)),
                            Err(e) => {
                                tracing::warn!(
                                    target: "memory_consolidator", error = %e, "MERGE failed"
                                );
                                (Some(format!("{e:#}")), Some(before_json))
                            }
                        }
                    }
                };
                if let Some(ref cause) = error {
                    tracing::warn!(target: "memory_consolidator", %cause, "rejecting MERGE");
                }
                let after_json = error
                    .is_none()
                    .then(|| json!({ "trust_score": MERGE_TRUST }).to_string());
                record_consolidation_op(
                    store,
                    ConsolidationAudit {
                        run_id,
                        batch_id,
                        kind,
                        memory_id: Some(into_memory_id),
                        related_id: None,
                        op_json: &op_json,
                        before_json: before_json.as_deref(),
                        after_json: after_json.as_deref(),
                        error: error.as_deref(),
                        scope: &scope_label,
                        expected_outcome: expected.as_deref(),
                    },
                )
                .await;
            }

            ConsolidationOp::Supersede {
                candidate_index,
                supersedes_memory_id,
            } => {
                let c = &candidates[candidate_index];
                let superseded = store
                    .get_memory_row(supersedes_memory_id)
                    .await
                    .ok()
                    .flatten();
                if superseded.is_none() {
                    let error = format!("superseded memory {supersedes_memory_id} does not exist");
                    tracing::warn!(target: "memory_consolidator", %error, "rejecting SUPERSEDE");
                    record_consolidation_op(
                        store,
                        ConsolidationAudit {
                            run_id,
                            batch_id,
                            kind,
                            memory_id: None,
                            related_id: Some(supersedes_memory_id),
                            op_json: &op_json,
                            before_json: None,
                            after_json: None,
                            error: Some(&error),
                            scope: &scope_label,
                            expected_outcome: expected.as_deref(),
                        },
                    )
                    .await;
                    continue;
                }

                let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
                    .with_importance(c.importance)
                    .with_type(super::scope::MemoryType::parse_str(&c.kind));
                let (memory_id, mut error) =
                    match store.store_scoped(&summary_scope, &c.text, &meta).await {
                        Ok(res) => (stored_id(&res), None),
                        Err(e) => (None, Some(format!("{e:#}"))),
                    };
                match memory_id {
                    Some(id) if error.is_none() => {
                        if let Err(e) = store.supersede_memory(supersedes_memory_id, id).await {
                            error = Some(format!("{e:#}"));
                        }
                    }
                    // The replacement was already covered by an existing
                    // row, so there is nothing to supersede *with*.
                    None if error.is_none() => {
                        error = Some(
                            "replacement text was already covered; nothing to supersede with"
                                .to_string(),
                        );
                    }
                    _ => {}
                }
                if let Some(ref cause) = error {
                    tracing::warn!(target: "memory_consolidator", %cause, "SUPERSEDE failed");
                }
                record_consolidation_op(
                    store,
                    ConsolidationAudit {
                        run_id,
                        batch_id,
                        kind,
                        memory_id,
                        related_id: Some(supersedes_memory_id),
                        op_json: &op_json,
                        before_json: None,
                        after_json: None,
                        error: error.as_deref(),
                        scope: &scope_label,
                        expected_outcome: expected.as_deref(),
                    },
                )
                .await;
            }

            ConsolidationOp::Drop { .. } => {
                // Drop is the no-op decision (don't promote the
                // candidate beyond Run scope). Per-turn extractor's
                // run-scope row stays; nothing to do here — but it is
                // still a decision the model made, so it is recorded.
                record_consolidation_op(
                    store,
                    ConsolidationAudit {
                        run_id,
                        batch_id,
                        kind,
                        memory_id: None,
                        related_id: None,
                        op_json: &op_json,
                        before_json: None,
                        after_json: None,
                        error: None,
                        scope: &scope_label,
                        expected_outcome: expected.as_deref(),
                    },
                )
                .await;
            }
        }
    }
    Ok(WriteResult::Skipped)
}

/// Trust score a `MERGE` promotes its surviving row to.
const MERGE_TRUST: f32 = 0.8;

/// Replay the last few consolidation runs for the prompt.
///
/// Best-effort: a consolidator that cannot see its history is worse at
/// self-correcting but still perfectly able to run, so a read failure
/// degrades to no history rather than aborting the pass.
async fn load_consolidation_history(
    store: &Arc<MemoryStore>,
) -> Vec<super::session_consolidator::ConsolidationRunBrief> {
    use super::session_consolidator::{ConsolidationRunBrief, MAX_HISTORY_RUNS};

    let runs = match store.recent_consolidation_runs(MAX_HISTORY_RUNS).await {
        Ok(runs) => runs,
        Err(e) => {
            tracing::warn!(
                target: "memory_consolidator",
                error = %e,
                "could not read consolidation history"
            );
            return Vec::new();
        }
    };

    let mut out = Vec::with_capacity(runs.len());
    for run in runs {
        // Per invocation, not per session. Reading by `run_id` replayed
        // every consolidation this conversation ever ran as one blob,
        // and — because the rolled-back flag was equally session-wide —
        // told the model its whole history had been rejected.
        let rows = match store.consolidation_audit_for_batch(&run.batch_id).await {
            Ok(rows) => rows,
            Err(_) => continue,
        };
        let ops = rows
            .iter()
            .map(|r| {
                let target = r
                    .related_id
                    .or(r.memory_id)
                    .map(|id| format!(" -> {id}"))
                    .unwrap_or_default();
                let verdict = match (&r.applied, &r.error) {
                    (true, _) => "ok".to_string(),
                    (false, Some(e)) => format!("rejected ({e})"),
                    (false, None) => "rejected".to_string(),
                };
                // The prediction is what makes the outcome instructive:
                // "you said X would be true; here is what happened".
                match &r.expected_outcome {
                    Some(x) if !x.trim().is_empty() => {
                        format!("{}{target}: {verdict} — you expected: {x}", r.kind)
                    }
                    _ => format!("{}{target}: {verdict}", r.kind),
                }
            })
            .collect();
        out.push(ConsolidationRunBrief {
            run_id: run.batch_id,
            ops,
            rolled_back: run.rolled_back,
        });
    }
    out
}

/// What a `/consolidate rollback` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackOutcome {
    /// The invocation that was undone.
    pub batch_id: String,
    /// Operations successfully reversed.
    pub reversed: usize,
    /// Rows skipped because the original operation never applied.
    pub skipped: usize,
    /// Inverse operations that themselves failed, with the reason.
    pub failed: Vec<String>,
}

/// Undo a consolidation run.
///
/// Walks the run's audit rows in reverse and applies the inverse of
/// each: an `ADD` becomes a soft-delete, a `SUPERSEDE` soft-deletes the
/// replacement and reinstates what it retired, a `MERGE` restores the
/// trust score it overwrote. `DROP` decided nothing, so it undoes to
/// nothing.
///
/// Reverse order matters for the same reason it does in a database
/// undo log: a later operation may depend on an earlier one, so
/// unwinding forwards could remove a row the next inverse still needs.
///
/// Rows whose operation never applied are skipped — there is nothing to
/// reverse, and re-deleting on their behalf would destroy state the
/// consolidator never created.
async fn process_consolidation_rollback(
    store: &Arc<MemoryStore>,
    batch_key: &str,
) -> Result<RollbackOutcome> {
    use super::session_consolidator::ConsolidationOp;

    if store.consolidation_run_was_rolled_back(batch_key).await? {
        bail!(
            "consolidation run '{batch_key}' has already been rolled back; the inverse \
             operations are not idempotent, so applying them twice would delete rows \
             the first rollback already restored"
        );
    }

    // Scoped to the invocation, not the session: rolling back one run
    // must not touch what a later run of the same conversation did.
    let rows = store.consolidation_audit_for_batch(batch_key).await?;
    if rows.is_empty() {
        bail!("no consolidation run '{batch_key}' found in the audit trail");
    }

    let mut outcome = RollbackOutcome {
        batch_id: batch_key.to_string(),
        reversed: 0,
        skipped: 0,
        failed: Vec::new(),
    };

    for row in rows.iter().rev() {
        if !row.applied {
            outcome.skipped += 1;
            continue;
        }

        let reason = format!("consolidate rollback of run {batch_key}");
        let result: Result<()> = match row.kind.as_str() {
            // Both created a row; undoing means retiring it again.
            "add" | "session_summary" => match row.memory_id {
                Some(id) => store
                    .soft_delete_memory(
                        id,
                        super::deletions::DeletedBy::ConsolidationRollback,
                        Some(&reason),
                        None,
                    )
                    .await
                    .map(|_| ()),
                None => Ok(()),
            },

            "supersede" => {
                // Reinstate the retired row *first*, then delete the
                // replacement. Not a preference — `memories.superseded_by`
                // is a foreign key onto `memories(id)`, so while the old
                // row still points at the replacement the replacement
                // cannot be deleted at all.
                let mut res = Ok(());
                if let Some(old_id) = row.related_id {
                    res = store.clear_supersede(old_id).await.map(|_| ());
                }
                if res.is_ok()
                    && let Some(new_id) = row.memory_id
                {
                    res = store
                        .soft_delete_memory(
                            new_id,
                            super::deletions::DeletedBy::ConsolidationRollback,
                            Some(&reason),
                            None,
                        )
                        .await
                        .map(|_| ());
                }
                res
            }

            "merge" => match (row.memory_id, trust_from(row.before_json.as_deref())) {
                (Some(id), Some(before)) => store.set_trust_score(id, before).await,
                (Some(_), None) => Err(anyhow!(
                    "merge audit row {} has no recorded prior trust to restore",
                    row.id
                )),
                (None, _) => Ok(()),
            },

            // A DROP declined to promote a candidate. Nothing happened,
            // so nothing comes back.
            "drop" => Ok(()),

            other => Err(anyhow!("unknown consolidation op kind '{other}'")),
        };

        let (applied, error) = match &result {
            Ok(()) => {
                outcome.reversed += 1;
                (true, None)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                tracing::warn!(
                    target: "memory_consolidator",
                    batch_key,
                    audit_id = row.id,
                    error = %msg,
                    "could not reverse consolidation op"
                );
                outcome.failed.push(format!("{}: {msg}", row.kind));
                (false, Some(msg))
            }
        };

        let op_json = serde_json::to_string(&json!({
            "reverses_audit_id": row.id,
            "op": row.kind,
        }))
        .unwrap_or_else(|_| "null".to_string());
        if let Err(e) = store
            .log_consolidation_rollback(super::store::consolidation_ops::RollbackAuditRow {
                run_id: &row.run_id,
                batch_id: batch_key,
                kind: &row.kind,
                memory_id: row.memory_id,
                related_id: row.related_id,
                op_json: &op_json,
                applied,
                error: error.as_deref(),
            })
            .await
        {
            tracing::warn!(
                target: "memory_consolidator",
                error = %e,
                "could not record rollback audit row"
            );
        }
    }

    // Referenced so the op enum stays wired to this path if its shape
    // changes; the audit rows carry the kind as a string.
    let _: Option<ConsolidationOp> = None;

    Ok(outcome)
}

/// The `trust_score` a merge recorded before it overwrote it.
fn trust_from(before_json: Option<&str>) -> Option<f32> {
    let parsed: JsonValue = serde_json::from_str(before_json?).ok()?;
    parsed
        .get("trust_score")
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
}

/// The id a store write landed on, if it produced one.
fn stored_id(result: &super::scope::StoreResult) -> Option<i64> {
    match result {
        super::scope::StoreResult::Inserted(id) | super::scope::StoreResult::Deduplicated(id) => {
            Some(*id)
        }
        super::scope::StoreResult::AlreadyCovered => None,
    }
}

/// Fields of one consolidation audit row, before it gains the details
/// this module fills in (`applied`, prompt version).
struct ConsolidationAudit<'a> {
    run_id: &'a str,
    /// Identifies this one invocation — see [`new_batch_id`].
    batch_id: &'a str,
    kind: &'a str,
    memory_id: Option<i64>,
    related_id: Option<i64>,
    op_json: &'a str,
    before_json: Option<&'a str>,
    after_json: Option<&'a str>,
    /// `None` means the operation landed.
    error: Option<&'a str>,
    scope: &'a str,
    /// What the consolidator predicted this op would achieve.
    expected_outcome: Option<&'a str>,
}

/// Write one consolidation audit row.
///
/// Audit failure must not abort the batch — losing the record of an
/// operation is bad, losing the *remaining operations* is worse — so it
/// is logged and swallowed here, the one place that is the right
/// trade-off.
async fn record_consolidation_op(store: &Arc<MemoryStore>, audit: ConsolidationAudit<'_>) {
    use super::store::consolidation_ops::ConsolidationAuditRow;

    let row = ConsolidationAuditRow {
        run_id: audit.run_id,
        batch_id: audit.batch_id,
        kind: audit.kind,
        memory_id: audit.memory_id,
        related_id: audit.related_id,
        op_json: audit.op_json,
        before_json: audit.before_json,
        after_json: audit.after_json,
        applied: audit.error.is_none(),
        error: audit.error,
        prompt_version: super::session_consolidator::PROMPT_VERSION,
        scope: audit.scope,
        expected_outcome: audit.expected_outcome,
    };
    if let Err(e) = store.log_consolidation_audit(row).await {
        tracing::warn!(
            target: "memory_consolidator",
            error = %e,
            "could not record consolidation audit row"
        );
    }
}

async fn process_telemetry_classify(
    store: &Arc<MemoryStore>,
    turn_id: &str,
    session_id: &str,
    response: &str,
) -> Result<WriteResult> {
    use super::telemetry::{ClassifyConfig, classify_turn};
    let cfg = ClassifyConfig::default();
    if response.split_whitespace().count() < cfg.min_response_tokens as usize {
        return Ok(WriteResult::Skipped);
    }
    classify_turn(store, turn_id, Some(session_id), response, &cfg).await?;
    Ok(WriteResult::Skipped)
}

/// Dispatch a TUI panel edit through the store (Tier A / A4).
///
/// All four ops enforce the Tier S2 single-consumer invariant — the
/// panel hands the op to the writer task, never calls `MemoryStore`
/// directly.
async fn process_panel_edit(store: &Arc<MemoryStore>, op: PanelEditOp) -> Result<WriteResult> {
    // C1: the panel cannot mutate history rows. Guard at the writer
    // task before the operation reaches the DB; the SQL immutability
    // trigger added in C1.3 is the second line of defense.
    let target_id = match &op {
        PanelEditOp::Delete { memory_id }
        | PanelEditOp::Pin { memory_id, .. }
        | PanelEditOp::SetScope { memory_id, .. }
        | PanelEditOp::UpdateText { memory_id, .. } => *memory_id,
    };
    if let Some(super::kind::MemoryKind::History) = store.get_memory_kind(target_id).await? {
        return Err(anyhow!(
            "panel cannot edit history row {target_id} — history is append-only \
             (use /forget-history to redact)"
        ));
    }

    match op {
        PanelEditOp::Delete { memory_id } => {
            // C2.1: soft-delete through the audit table so `/restore`
            // can reinstate within the retention window. The history
            // refusal upstream guarantees we never reach this branch
            // for a history row, but soft_delete_memory enforces it
            // again as belt-and-braces.
            match store
                .soft_delete_memory(memory_id, super::deletions::DeletedBy::Panel, None, None)
                .await
            {
                Ok(audit_id) => Ok(WriteResult::Inserted(audit_id)),
                Err(e) => {
                    // Distinguish "row not found" (cosmetic — the
                    // panel's view raced ahead of the user) from a
                    // real failure.
                    if format!("{e}").contains("not found") {
                        Ok(WriteResult::Skipped)
                    } else {
                        Err(e)
                    }
                }
            }
        }
        PanelEditOp::Pin {
            memory_id,
            trust_score,
        } => {
            let clamped = super::trust_defaults::clamp_trust(trust_score);
            store.set_trust_score(memory_id, clamped).await?;
            Ok(WriteResult::Inserted(memory_id))
        }
        PanelEditOp::SetScope {
            memory_id,
            new_scope,
        } => {
            let id = store.change_memory_scope(memory_id, &new_scope).await?;
            Ok(WriteResult::Inserted(id))
        }
        PanelEditOp::UpdateText {
            memory_id,
            new_text,
        } => {
            store.update_memory_text(memory_id, &new_text).await?;
            Ok(WriteResult::Inserted(memory_id))
        }
    }
}

/// Audit `kind` recorded in `sleeptime_audit` for an applied agent flag.
/// Shares the table with the sleeptime operations so the Tier C2
/// `/forget` audit trail can reverse a demotion by hand — the convention
/// `memory/sleeptime.rs` sets for every trust-changing path.
const AGENT_FLAG_AUDIT_KIND: &str = "agent_flag";

/// D1: demote one memory's trust in response to an agent flag.
///
/// Refuses user-authored and History rows (returns `accepted: false`,
/// which is a successful call). Errors only when the owning store or the
/// row itself cannot be resolved — never falls back to another DB.
async fn process_agent_flag(
    stores: &Arc<MemoryStores>,
    memory_id: i64,
    scope_level: i32,
    repo_id: Option<&str>,
    reason: &str,
) -> Result<AgentFlagOutcome> {
    use super::trust_defaults::{flagged_trust, is_agent_flaggable};

    let kind = store_kind_for_scope(scope_level, repo_id).ok_or_else(|| {
        anyhow!(
            "memory_flag: cannot resolve the store owning scope_level {scope_level} \
             (repo_id {repo_id:?})"
        )
    })?;
    let store = stores.get(&kind).await?;
    let row = store
        .get_memory_row(memory_id)
        .await?
        .ok_or_else(|| anyhow!("memory_flag: no memory {memory_id} in the {kind:?} store"))?;

    if !is_agent_flaggable(row.source) {
        return Ok(AgentFlagOutcome {
            accepted: false,
            detail: format!("{} rows are not agent-flaggable", row.source.as_str()),
        });
    }

    let prior = row.trust_score;
    let next = flagged_trust(row.source, prior);
    store.set_trust_score(memory_id, next).await?;

    // Without an audit row a flag would be an unreversible mutation,
    // which no other trust-changing path in the codebase is.
    let payload = serde_json::json!({
        "memory_id": memory_id,
        "scope_level": scope_level,
        "repo_id": repo_id,
        "source": row.source.as_str(),
        "trust_before": prior,
        "trust_after": next,
        "reason": reason,
    })
    .to_string();
    if let Err(e) = store
        .log_sleeptime_audit(
            AGENT_FLAG_AUDIT_KIND,
            AGENT_FLAG_AUDIT_KIND,
            Some(memory_id),
            None,
            &payload,
            false,
        )
        .await
    {
        tracing::warn!(
            target: "memory_writer",
            memory_id = memory_id,
            error = %e,
            "agent flag audit row write failed"
        );
    }

    tracing::info!(
        target: "memory_writer",
        memory_id = memory_id,
        source = row.source.as_str(),
        trust_before = prior,
        trust_after = next,
        "agent flag applied"
    );
    Ok(AgentFlagOutcome {
        accepted: true,
        detail: format!("trust {prior:.2} → {next:.2}"),
    })
}

/// Per-turn extractor path (Tier S / S3).
///
/// Flow:
/// 1. Invoke the configured `ConsolidationLlm` against the version-pinned
///    extractor prompt.
/// 2. Parse 0–5 candidates; drop below-threshold importance.
/// 3. Resolve each `scope_hint` to a concrete `WriteScope`, build
///    `WriteMeta` with `source="llm_extracted"` + `trust=Medium`, and
///    call `MemoryStores::store_scoped`, which routes the row to the DB
///    named by [`WriteScope::target_store`] and then delegates to that
///    store's own SHA + cosine dedup (see `stores.rs` / `store.rs`).
///
/// Scope routing matters here: a flag or extraction declaring `global` /
/// `repo` / `module` scope belongs in the global or folder DB. Writing it
/// to the workspace DB makes it unreachable — retrieval asks the global
/// store for Global-level rows and folder stores for Repo/Module-level
/// rows (`MemoryStores::multi_scope_retrieve`).
/// 4. On LLM unavailable / JSON parse failure: write one Run-scope raw
///    record so the turn is never lost.
///
/// **Never blocks the chat event loop** — this runs inside the writer
/// task, which the chat path enqueues into and returns from immediately.
/// Per-stage latency is logged via `tracing` with a correlation id built
/// from `turn_id` for post-hoc perf work.
async fn process_turn_complete(
    stores: &Arc<MemoryStores>,
    llm: Option<&Arc<dyn ConsolidationLlm>>,
    session_id: String,
    turn_id: String,
    repo_id: String,
    module_path: Option<String>,
    run_id: String,
    transcript: String,
    annotations: Option<JsonValue>,
) -> Result<WriteResult> {
    use super::annotations as ann_mod;
    use super::extractor;

    let started = std::time::Instant::now();

    // ── C1: persist the immutable History row first ───────────────────
    //
    // Every TurnComplete now also writes a `kind = history` row carrying
    // the verbatim transcript at Run scope. The row is the source of
    // truth for every derived record. Plan §C1: "History-write failure
    // does not lose records and vice versa." We capture errors but
    // never propagate — the extractor path below still runs even if
    // this write fails.
    //
    // History rows are Run scope, which routes to the workspace store by
    // definition (`store_kind_for_scope`), so this one stays store-local.
    let history_id = write_history_row(
        stores.workspace(),
        &session_id,
        &turn_id,
        &repo_id,
        &run_id,
        &transcript,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(
            target: "memory_history",
            turn_id = %turn_id,
            error = %e,
            "history-row write failed; proceeding to extractor path"
        );
        None
    });
    if let Some(id) = history_id {
        tracing::debug!(
            target: "memory_history",
            turn_id = %turn_id,
            history_id = id,
            "history row persisted"
        );
    }

    // ── A1: persist session-ledger row + annotated flags ─────────────
    //
    // `annotations` is `Some(serde_json::Value)` when the chat path
    // parsed a `<turn_annotations>` block. The writer task:
    //   1. persists `session_thread` + `open_questions` to
    //      `session_ledger_turns` (for B5 consolidation + A4 panel),
    //   2. inserts each flag as an `LlmAnnotated` memory through
    //      `store_scoped` (SHA-256 + cosine dedup lives inside the
    //      store, so same-content-at-broader-scope is still a no-op).
    //
    // The block's presence does **not** short-circuit the extractor —
    // plan §A1 calls for the extractor as a safety net, with the store-
    // level dedup taking care of cross-source overlap.
    let mut annotated_ids: Vec<i64> = Vec::new();
    if let Some(raw) = &annotations {
        match serde_json::from_value::<ann_mod::TurnAnnotations>(raw.clone()) {
            Ok(parsed) => {
                // Persist session ledger row (best-effort). The ledger is
                // workspace-level bookkeeping, not a scoped memory.
                let annotations_json = serde_json::to_string(&parsed).ok();
                if let Err(e) = stores
                    .workspace()
                    .store_session_ledger_turn(
                        &session_id,
                        &turn_id,
                        parsed.session_thread.as_deref(),
                        &parsed.open_questions,
                        annotations_json.as_deref(),
                    )
                    .await
                {
                    tracing::warn!(
                        target: "memory_annotations",
                        turn_id = %turn_id,
                        error = %e,
                        "session_ledger write failed"
                    );
                }

                // Insert each flag as an LlmAnnotated memory.
                for f in &parsed.flags {
                    let ext_shape = super::extractor::ExtractedMemory {
                        kind: f.kind.clone(),
                        scope_hint: f.scope.clone(),
                        text: f.text.clone(),
                        importance: f.importance,
                        refs: f.refs.clone(),
                    };
                    let scope = extractor::resolve_scope(
                        &f.scope,
                        &repo_id,
                        module_path.as_deref(),
                        &run_id,
                    );
                    let meta = super::extractor::build_annotated_write_meta(&ext_shape);
                    match stores.store_scoped(&scope, &f.text, &meta).await {
                        Ok(super::scope::StoreResult::Inserted(id)) => {
                            annotated_ids.push(id);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(
                                target: "memory_annotations",
                                turn_id = %turn_id,
                                error = %e,
                                "annotated flag store_scoped failed"
                            );
                        }
                    }
                }
            }
            Err(e) => {
                // The JSON shape didn't match — metric this so prompt
                // drift is visible. Fall through to extractor-only.
                tracing::warn!(
                    target: "memory_annotations",
                    turn_id = %turn_id,
                    error = %e,
                    "annotations parse failed — falling through to extractor"
                );
            }
        }
    }

    // ── 1. LLM extraction (or fallback) ──────────────────────────────
    let extractions = match llm {
        Some(llm) => match extractor::extract(llm.as_ref(), &transcript).await {
            Ok(items) => {
                tracing::info!(
                    target: "memory_extractor",
                    turn_id = %turn_id,
                    session_id = %session_id,
                    extracted = items.len(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "extractor succeeded"
                );
                items
            }
            Err(e) => {
                tracing::warn!(
                    target: "memory_extractor",
                    turn_id = %turn_id,
                    error = %e,
                    "extractor failed — writing Run-scope fallback"
                );
                return fallback_raw_turn(stores.workspace(), &repo_id, &run_id, &transcript).await;
            }
        },
        None => {
            tracing::debug!(
                target: "memory_extractor",
                turn_id = %turn_id,
                "no ConsolidationLlm configured — writing Run-scope fallback"
            );
            return fallback_raw_turn(stores.workspace(), &repo_id, &run_id, &transcript).await;
        }
    };

    if extractions.is_empty() {
        tracing::info!(
            target: "memory_extractor",
            turn_id = %turn_id,
            "extractor produced 0 memories"
        );
        return Ok(WriteResult::Skipped);
    }

    // ── 2. Dedup + insert each extraction ────────────────────────────
    let mut last_result = WriteResult::Skipped;
    let mut inserted_count = 0usize;
    for ext in extractions {
        let scope =
            extractor::resolve_scope(&ext.scope_hint, &repo_id, module_path.as_deref(), &run_id);
        let meta = extractor::build_write_meta(&ext, extractor::PROMPT_VERSION);
        match stores.store_scoped(&scope, &ext.text, &meta).await {
            Ok(res) => {
                if matches!(res, super::scope::StoreResult::Inserted(_)) {
                    inserted_count += 1;
                }
                last_result = WriteResult::from(res);
            }
            Err(e) => {
                tracing::warn!(
                    target: "memory_extractor",
                    turn_id = %turn_id,
                    kind = %ext.kind,
                    error = %e,
                    "extractor: store_scoped failed for one extraction"
                );
            }
        }
    }

    tracing::info!(
        target: "memory_extractor",
        turn_id = %turn_id,
        inserted = inserted_count,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "extractor turn complete"
    );

    Ok(last_result)
}

/// C1: write a `kind = history` row carrying the verbatim transcript
/// for one chat turn, plus the provenance bundle (`session_id`, `turn_id`,
/// `manifest_id` cross-ref) as a tag-encoded payload so the panel can
/// navigate to the matching S4 manifest. Returns the inserted row id
/// when the write produces a fresh row, `None` otherwise (dedup is
/// disabled for history but the path still passes through `store_scoped`
/// for consistency).
async fn write_history_row(
    store: &Arc<MemoryStore>,
    session_id: &str,
    turn_id: &str,
    repo_id: &str,
    run_id: &str,
    transcript: &str,
) -> Result<Option<i64>> {
    use super::kind::MemoryKind;
    use super::scope::{MemoryType, WriteMeta, WriteScope};
    use super::trust_defaults::MemorySource;

    let scope = WriteScope::Run {
        repo_id: repo_id.to_string(),
        run_id: run_id.to_string(),
    };
    // The tag carries the navigational keys (session_id + turn_id) so
    // the panel can join history rows to manifests without an extra
    // index. Trust = 1.0 from `RawTranscript`. Importance is high
    // because History rows are the audit trail; B4-style decay does
    // not apply since they're not injected into chat anyway.
    let meta = WriteMeta::for_source(MemorySource::RawTranscript)
        .with_kind(MemoryKind::History)
        .with_type(MemoryType::Factual)
        .with_importance(1.0)
        .with_tag(format!("history:{session_id}:{turn_id}"));

    let res = store
        .store_scoped(&scope, transcript, &meta)
        .await
        .map_err(|e| anyhow!("history store_scoped: {e}"))?;
    Ok(match res {
        super::scope::StoreResult::Inserted(id) => Some(id),
        super::scope::StoreResult::Deduplicated(id) => Some(id),
        super::scope::StoreResult::AlreadyCovered => None,
    })
}

/// Last-resort write for `TurnComplete` when extraction can't run (LLM
/// down, parse error, no backend configured). Writes a single Run-scope
/// record with the raw transcript so the turn isn't lost — consolidation
/// passes can upgrade or prune it later.
///
/// Run scope always resolves to the workspace store, so this takes the
/// concrete store rather than the registry.
async fn fallback_raw_turn(
    store: &Arc<MemoryStore>,
    repo_id: &str,
    run_id: &str,
    transcript: &str,
) -> Result<WriteResult> {
    use super::scope::{MemoryType, WriteMeta, WriteScope};
    use super::trust_defaults::MemorySource;
    let scope = WriteScope::Run {
        repo_id: repo_id.to_string(),
        run_id: run_id.to_string(),
    };
    // A3: fallback writes — LLM extractor failed, so the record carries
    // the extractor's provenance but a lower importance. Trust_score
    // defaults to 0.6 via `LlmExtracted`; no need to override.
    let meta = WriteMeta::for_source(MemorySource::LlmExtracted)
        .with_type(MemoryType::Factual)
        .with_importance(0.4)
        .with_tag("turn_fallback");
    let trimmed = truncate_transcript(transcript, 2000);
    let res = store
        .store_scoped(&scope, &trimmed, &meta)
        .await
        .map_err(|e| anyhow!("fallback store_scoped: {e}"))?;
    Ok(WriteResult::from(res))
}

fn truncate_transcript(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

fn send_ack(ack: Option<oneshot::Sender<Result<WriteResult, String>>>, res: &Result<WriteResult>) {
    if let Some(tx) = ack {
        let payload = match res {
            Ok(r) => Ok(r.clone()),
            Err(e) => Err(e.to_string()),
        };
        let _ = tx.send(payload);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Mutex as StdMutex;

    use super::*;
    use crate::memory::embedder::Embedder;

    /// Minimal in-memory embedder for tests — content-addressed so dedup
    /// against the store has a deterministic but non-zero vector.
    struct MockEmbedder;

    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        fn name(&self) -> &str {
            "mock"
        }

        fn dimension(&self) -> usize {
            8
        }

        async fn embed(
            &self,
            text: &str,
            _purpose: crate::memory::embedder::EmbeddingPurpose,
        ) -> Result<Vec<f32>> {
            let mut v = vec![0.0f32; 8];
            for (i, b) in text.bytes().enumerate() {
                v[i % 8] += b as f32;
            }
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in &mut v {
                    *x /= norm;
                }
            }
            Ok(v)
        }
    }

    #[derive(Default)]
    struct RecordingObserver {
        events: StdMutex<Vec<String>>,
    }

    impl MemoryObserver for RecordingObserver {
        fn on_write_enqueued(&self, kind: &str) {
            self.events.lock().unwrap().push(format!("enqueued:{kind}"));
        }
        fn on_write_committed(&self, kind: &str, _result: &WriteResult) {
            self.events
                .lock()
                .unwrap()
                .push(format!("committed:{kind}"));
        }
        fn on_write_failed(&self, kind: &str, _error: &str) {
            self.events.lock().unwrap().push(format!("failed:{kind}"));
        }
    }

    // ── H1 / PR-5: consolidation audit rows ──────────────────────────

    /// Returns one fixed consolidator response, whatever the prompt.
    struct ScriptedLlm(String);

    #[async_trait::async_trait]
    impl ConsolidationLlm for ScriptedLlm {
        async fn complete(&self, _prompt: String) -> Result<String> {
            Ok(self.0.clone())
        }
    }

    /// Drive one consolidation run start to finish, inline.
    ///
    /// Production splits this across the writer queue — the model call
    /// must not occupy the single-consumer writer task — but these tests
    /// are about what the operations *do*, so they run the same three
    /// production steps back to back rather than reimplementing any of
    /// them. `WriterMessage::SessionConsolidateApply` is what carries the
    /// middle result in the real path.
    async fn consolidate_inline(
        store: &Arc<MemoryStore>,
        llm: &Arc<dyn ConsolidationLlm>,
        repo_id: &str,
        module_path: Option<String>,
        run_id: &str,
        transcript: &str,
    ) -> Result<WriteResult> {
        let Some(prep) =
            prepare_session_consolidate(store, repo_id, &module_path, run_id, transcript).await
        else {
            return Ok(WriteResult::Skipped);
        };
        let raw = llm.complete(prep.prompt).await?;
        let parsed = super::super::session_consolidator::parse_response(&raw)?;
        apply_session_consolidate(
            store,
            repo_id.to_string(),
            module_path,
            run_id,
            &prep.candidates,
            parsed,
        )
        .await
    }

    /// A store holding `count` run-scoped candidate rows, which is what
    /// the consolidator reads as its CANDIDATES list.
    async fn store_with_candidates(run_id: &str, count: usize) -> Arc<MemoryStore> {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        for i in 0..count {
            store
                .store_scoped(
                    &WriteScope::Run {
                        repo_id: "r1".into(),
                        run_id: run_id.to_string(),
                    },
                    &format!("candidate number {i}"),
                    &WriteMeta::default(),
                )
                .await
                .unwrap();
        }
        store
    }

    #[tokio::test]
    async fn every_consolidation_op_is_audited_applied_or_not() {
        let run_id = "run-audit-1";
        let store = store_with_candidates(run_id, 2).await;

        // One op that should land, one naming a memory id that does not
        // exist, one naming a candidate outside the batch.
        let response = json!({
            "session_summary": "the session did things",
            "operations": [
                { "op": "ADD", "candidate_index": 0 },
                { "op": "MERGE", "candidate_index": 0, "into_memory_id": 999_999 },
                { "op": "ADD", "candidate_index": 42 },
            ]
        })
        .to_string();
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(ScriptedLlm(response));

        consolidate_inline(&store, &llm, "r1", None, run_id, "transcript")
            .await
            .expect("consolidation runs to completion");

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let ops: Vec<_> = rows
            .iter()
            .filter(|r| r.kind != "session_summary")
            .collect();
        assert_eq!(ops.len(), 3, "one row per attempted op: {rows:?}");

        // 1. The valid ADD landed and names the row it created.
        let add = ops[0];
        assert_eq!(add.kind, "add");
        assert!(add.applied, "valid ADD should apply: {:?}", add.error);
        assert!(add.error.is_none());
        assert!(add.memory_id.is_some(), "an applied ADD records its row");

        // 2. The MERGE named an id the model invented.
        let merge = ops[1];
        assert_eq!(merge.kind, "merge");
        assert!(!merge.applied, "merge into a missing id must not apply");
        let merge_error = merge.error.as_deref().unwrap_or_default();
        assert!(
            merge_error.contains("999999") || merge_error.contains("does not exist"),
            "error should name the missing target: {merge_error:?}"
        );

        // 3. The out-of-range candidate.
        let bad = ops[2];
        assert!(!bad.applied);
        assert!(
            bad.error
                .as_deref()
                .unwrap_or_default()
                .contains("out of range"),
            "error should name the bad index: {:?}",
            bad.error
        );

        // Every row carries the prompt version that produced it.
        for r in &rows {
            assert_eq!(
                r.prompt_version.as_deref(),
                Some(super::super::session_consolidator::PROMPT_VERSION)
            );
        }
    }

    #[tokio::test]
    async fn a_module_scoped_consolidation_records_its_scope() {
        let run_id = "run-audit-scope";
        let store = store_with_candidates(run_id, 1).await;

        let response = json!({
            "session_summary": "",
            "operations": [{ "op": "ADD", "candidate_index": 0 }]
        })
        .to_string();
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(ScriptedLlm(response));

        consolidate_inline(
            &store,
            &llm,
            "r1",
            Some("crates/gaviero-core".into()),
            run_id,
            "transcript",
        )
        .await
        .unwrap();

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].scope.as_deref(),
            Some("module:crates/gaviero-core"),
            "#229 made the landing scope variable; the audit records which"
        );
    }

    // ── Consolidator candidate pool ──────────────────────────────────

    /// Store one extracted row and one verbatim History row under the
    /// same run, in the shape `/consolidate-session` actually sees.
    async fn store_with_transcript_and_extraction(run_id: &str) -> Arc<MemoryStore> {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let scope = WriteScope::Run {
            repo_id: "r1".into(),
            run_id: run_id.to_string(),
        };
        store
            .store_scoped(
                &scope,
                "the writer task is the single owner of SQLite writes",
                &WriteMeta::for_source(MemorySource::LlmExtracted),
            )
            .await
            .unwrap();
        store
            .store_scoped(
                &scope,
                "USER: go for pr-6\n\nASSISTANT: PR-6 — rollback. Let me first find the C2 primitives",
                &WriteMeta::for_source(MemorySource::RawTranscript),
            )
            .await
            .unwrap();
        store
    }

    /// A-fix. The first live run promoted five raw transcript blobs into
    /// the durable repo record at importance 1.0, because they were shown
    /// to the model as ordinary candidates.
    #[tokio::test]
    async fn raw_transcript_rows_never_become_candidates() {
        let run_id = "run-transcript-filter";
        let store = store_with_transcript_and_extraction(run_id).await;

        let prep = prepare_session_consolidate(&store, "r1", &None, run_id, "the transcript")
            .await
            .expect("a session with rows prepares a run");

        assert_eq!(
            prep.candidates.len(),
            1,
            "only the extracted row is a candidate: {:?}",
            prep.candidates
        );
        assert!(prep.candidates[0].text.contains("single owner"));
        assert!(
            !prep
                .candidates
                .iter()
                .any(|c| c.text.starts_with("USER: go for pr-6")),
            "a History row reached the candidate list"
        );
    }

    /// The transcript belongs in the prompt — once, in the TRANSCRIPT
    /// section — not a second time as a candidate the model can ADD.
    #[tokio::test]
    async fn the_prompt_shows_the_transcript_but_not_as_a_candidate() {
        let run_id = "run-transcript-prompt";
        let store = store_with_transcript_and_extraction(run_id).await;

        let prep = prepare_session_consolidate(&store, "r1", &None, run_id, "USER: hello there")
            .await
            .unwrap();

        assert!(
            prep.prompt.contains("USER: hello there"),
            "transcript section missing"
        );
        let candidates_section = prep
            .prompt
            .split("CANDIDATES (extracted this session):")
            .nth(1)
            .expect("prompt has a candidates section");
        assert!(
            !candidates_section.contains("USER: go for pr-6"),
            "History row rendered into CANDIDATES"
        );
    }

    /// Filtering must not turn a transcript-only session into a silent
    /// no-op: the session summary is still worth producing.
    #[tokio::test]
    async fn a_transcript_only_session_still_prepares_a_run() {
        let run_id = "run-transcript-only";
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        store
            .store_scoped(
                &WriteScope::Run {
                    repo_id: "r1".into(),
                    run_id: run_id.into(),
                },
                "USER: only chatter here",
                &WriteMeta::for_source(MemorySource::RawTranscript),
            )
            .await
            .unwrap();

        let prep = prepare_session_consolidate(&store, "r1", &None, run_id, "t")
            .await
            .expect("a transcript-only session still runs");
        assert!(prep.candidates.is_empty());
    }

    /// With no candidates there is nothing to rank against, and no
    /// MERGE / SUPERSEDE could be valid anyway — both carry a
    /// `candidate_index`. The prompt should say so rather than list
    /// targets the model cannot legally use.
    #[tokio::test]
    async fn a_session_with_no_candidates_offers_no_merge_targets() {
        let run_id = "run-no-candidates";
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        // A repo-scope row that would otherwise be an eligible target.
        store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "r1".into(),
                },
                "an existing repo-scope belief",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        store
            .store_scoped(
                &WriteScope::Run {
                    repo_id: "r1".into(),
                    run_id: run_id.into(),
                },
                "USER: only chatter",
                &WriteMeta::for_source(MemorySource::RawTranscript),
            )
            .await
            .unwrap();

        let prep = prepare_session_consolidate(&store, "r1", &None, run_id, "t")
            .await
            .unwrap();

        assert!(prep.candidates.is_empty());
        assert!(
            prep.prompt.contains("do not emit MERGE or SUPERSEDE"),
            "empty-candidate prompt should forbid merges outright"
        );
        assert!(
            !prep.prompt.contains("an existing repo-scope belief"),
            "listed a target no operation could legally reference"
        );
    }

    /// A session that produced nothing at all is still skipped, so an
    /// idle conversation never spends a model call.
    #[tokio::test]
    async fn a_session_with_no_rows_prepares_nothing() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        assert!(
            prepare_session_consolidate(&store, "r1", &None, "run-empty", "t")
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn a_merge_records_the_trust_it_overwrote() {
        // Rollback (PR-6) restores the prior trust from `before_json`,
        // so a successful merge has to capture it.
        let run_id = "run-audit-merge";
        let store = store_with_candidates(run_id, 1).await;
        let target = store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "r1".into(),
                },
                "an existing repo-scoped memory",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        let target_id = match target {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            StoreResult::AlreadyCovered => panic!("expected an id"),
        };
        let before_trust = store
            .get_memory_row(target_id)
            .await
            .unwrap()
            .expect("target exists")
            .trust_score;

        let response = json!({
            "session_summary": "",
            "operations": [
                { "op": "MERGE", "candidate_index": 0, "into_memory_id": target_id }
            ]
        })
        .to_string();
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(ScriptedLlm(response));

        consolidate_inline(&store, &llm, "r1", None, run_id, "transcript")
            .await
            .unwrap();

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let merge = rows.iter().find(|r| r.kind == "merge").expect("merge row");
        assert!(
            merge.applied,
            "merge into a real id applies: {:?}",
            merge.error
        );

        let before: JsonValue =
            serde_json::from_str(merge.before_json.as_deref().expect("before_json")).unwrap();
        assert_eq!(
            before["trust_score"].as_f64().unwrap() as f32,
            before_trust,
            "before_json must capture the trust rollback will restore"
        );
    }

    #[tokio::test]
    async fn a_sleeptime_pass_still_writes_its_own_origin() {
        // Regression: widening the table must not reclassify the rows
        // it already served.
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = MemoryStore::in_memory(embedder).unwrap();

        store
            .log_sleeptime_audit("run-s", "decay_flag", Some(1), None, "{}", false)
            .await
            .unwrap();

        // The row landed …
        assert_eq!(store.count_audit_for_test("decay_flag").await.unwrap(), 1);
        // … and is not visible to the consolidation reader, which proves
        // it kept `origin = 'sleeptime'` rather than being reclassified.
        assert!(
            store
                .consolidation_audit_for_run("run-s")
                .await
                .unwrap()
                .is_empty()
        );
    }

    // ── H1 / PR-7: session_v2 end to end ─────────────────────────────

    /// Captures the prompt it was given, so a test can assert on what
    /// the consolidator was actually told.
    struct CapturingLlm {
        response: String,
        seen: Arc<StdMutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl ConsolidationLlm for CapturingLlm {
        async fn complete(&self, prompt: String) -> Result<String> {
            self.seen.lock().unwrap().push(prompt);
            Ok(self.response.clone())
        }
    }

    #[tokio::test]
    async fn the_prompt_lists_real_existing_memories() {
        // C-5: this is the whole point of PR-7. Before it, the prompt
        // was built with an empty existing set, so every id the model
        // emitted was invented and every MERGE was rejected.
        let run_id = "run-v2-existing";
        let store = store_with_candidates(run_id, 1).await;
        let target = store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "r1".into(),
                },
                "a repo-scoped belief the model may merge into",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        let target_id = match target {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            StoreResult::AlreadyCovered => panic!("expected an id"),
        };

        let seen = Arc::new(StdMutex::new(Vec::new()));
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(CapturingLlm {
            response: json!({ "session_summary": "", "operations": [] }).to_string(),
            seen: seen.clone(),
        });
        consolidate_inline(&store, &llm, "r1", None, run_id, "transcript")
            .await
            .unwrap();

        let prompt = seen.lock().unwrap()[0].clone();
        assert!(
            prompt.contains(&format!("id={target_id} ")),
            "the prompt must list the id the model is allowed to name"
        );
        assert!(prompt.contains("a repo-scoped belief"));
    }

    #[tokio::test]
    async fn a_merge_naming_an_injected_id_applies() {
        // The acceptance criterion for PR-7: with real ids in the
        // prompt, reference validation stops rejecting everything.
        let run_id = "run-v2-merge";
        let store = store_with_candidates(run_id, 1).await;
        let target = store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "r1".into(),
                },
                "the belief being merged into",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        let target_id = match target {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            StoreResult::AlreadyCovered => panic!("expected an id"),
        };

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [{
                    "op": "MERGE",
                    "candidate_index": 0,
                    "into_memory_id": target_id,
                    "expected_outcome": "the existing belief absorbs this session's note"
                }]
            }),
        )
        .await;

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let merge = rows.iter().find(|r| r.kind == "merge").expect("merge row");
        assert!(merge.applied, "should apply: {:?}", merge.error);
        assert_eq!(
            merge.prompt_version.as_deref(),
            Some(super::super::session_consolidator::PROMPT_VERSION),
            "the audit row records which rubric produced the op"
        );
        assert_eq!(
            merge.expected_outcome.as_deref(),
            Some("the existing belief absorbs this session's note")
        );
    }

    #[tokio::test]
    async fn a_later_run_sees_what_happened_to_the_earlier_one() {
        // The self-correction loop: run 1's verdicts, including its
        // rejections and the user's rollback, are replayed to run 2.
        let store = store_with_candidates("run-v2-a", 1).await;
        consolidate(
            &store,
            "run-v2-a",
            json!({
                "session_summary": "",
                "operations": [{
                    "op": "MERGE",
                    "candidate_index": 0,
                    "into_memory_id": 999_111,
                    "expected_outcome": "this will not work"
                }]
            }),
        )
        .await;

        // The second run needs candidates of its own, or the
        // consolidator skips before it ever builds a prompt.
        store
            .store_scoped(
                &WriteScope::Run {
                    repo_id: "r1".into(),
                    run_id: "run-v2-b".into(),
                },
                "something learned in the second session",
                &WriteMeta::default(),
            )
            .await
            .unwrap();

        let seen = Arc::new(StdMutex::new(Vec::new()));
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(CapturingLlm {
            response: json!({ "session_summary": "", "operations": [] }).to_string(),
            seen: seen.clone(),
        });
        consolidate_inline(&store, &llm, "r1", None, "run-v2-b", "transcript")
            .await
            .unwrap();

        let prompt = seen.lock().unwrap()[0].clone();
        assert!(prompt.contains("CONSOLIDATION_HISTORY (your previous runs)"));
        // Replayed per invocation, so the entry is keyed by batch id
        // rather than by the session it belonged to.
        let first_batch = store
            .recent_consolidation_runs(10)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.run_id == "run-v2-a")
            .expect("the earlier run is in history")
            .batch_id;
        assert!(prompt.contains(&format!("run {first_batch}")));
        assert!(
            prompt.contains("rejected"),
            "the failed merge should be replayed as a rejection"
        );
        assert!(
            prompt.contains("this will not work"),
            "and paired with what the model predicted"
        );
    }

    // ── H1 / PR-6: /consolidate rollback ─────────────────────────────

    /// Every live (not soft-deleted) memory id in the store.
    async fn live_ids(store: &Arc<MemoryStore>) -> Vec<i64> {
        let mut ids: Vec<i64> = store
            .recent_memories(24 * 365, 500)
            .await
            .unwrap()
            .iter()
            .map(|m| m.id)
            .collect();
        ids.sort_unstable();
        ids
    }

    async fn consolidate(store: &Arc<MemoryStore>, run_id: &str, response: serde_json::Value) {
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(ScriptedLlm(response.to_string()));
        consolidate_inline(store, &llm, "r1", None, run_id, "transcript")
            .await
            .unwrap();
    }

    /// The id of the most recent consolidation invocation.
    ///
    /// Tests discover it the way a user does — from the history — rather
    /// than assuming it equals `run_id`. That assumption is exactly what
    /// schema v16 removed: several invocations share one `run_id`, so it
    /// cannot identify which of them to undo.
    async fn latest_batch_id(store: &Arc<MemoryStore>) -> String {
        store
            .recent_consolidation_runs(1)
            .await
            .unwrap()
            .first()
            .expect("a consolidation run should exist")
            .batch_id
            .clone()
    }

    #[tokio::test]
    async fn rollback_restores_the_rows_a_run_created() {
        let run_id = "run-rb-1";
        let store = store_with_candidates(run_id, 2).await;
        let before = live_ids(&store).await;

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "a summary",
                "operations": [
                    { "op": "ADD", "candidate_index": 0 },
                    { "op": "ADD", "candidate_index": 1 },
                ]
            }),
        )
        .await;

        let after_apply = live_ids(&store).await;
        assert!(
            after_apply.len() > before.len(),
            "the run should have added rows"
        );

        let outcome = process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .unwrap();
        assert_eq!(outcome.reversed, 3, "2 adds + the session summary");
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        assert_eq!(
            live_ids(&store).await,
            before,
            "rollback must leave exactly the rows that existed before the run"
        );
    }

    #[tokio::test]
    async fn rollback_restores_the_trust_a_merge_overwrote() {
        let run_id = "run-rb-merge";
        let store = store_with_candidates(run_id, 1).await;
        let target = store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "r1".into(),
                },
                "a long-lived repo memory",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        let target_id = match target {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            StoreResult::AlreadyCovered => panic!("expected an id"),
        };
        let original_trust = store
            .get_memory_row(target_id)
            .await
            .unwrap()
            .unwrap()
            .trust_score;

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [
                    { "op": "MERGE", "candidate_index": 0, "into_memory_id": target_id }
                ]
            }),
        )
        .await;
        assert_eq!(
            store
                .get_memory_row(target_id)
                .await
                .unwrap()
                .unwrap()
                .trust_score,
            MERGE_TRUST,
            "the merge should have bumped trust"
        );

        process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .unwrap();

        assert_eq!(
            store
                .get_memory_row(target_id)
                .await
                .unwrap()
                .unwrap()
                .trust_score,
            original_trust,
            "rollback must restore the trust the merge overwrote"
        );
    }

    #[tokio::test]
    async fn rollback_reinstates_a_superseded_memory() {
        let run_id = "run-rb-supersede";
        let store = store_with_candidates(run_id, 1).await;
        let old = store
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: "r1".into(),
                },
                "the older belief about the thing",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        let old_id = match old {
            StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
            StoreResult::AlreadyCovered => panic!("expected an id"),
        };

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [
                    { "op": "SUPERSEDE", "candidate_index": 0, "supersedes_memory_id": old_id }
                ]
            }),
        )
        .await;

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let sup = rows.iter().find(|r| r.kind == "supersede").unwrap();
        assert!(sup.applied, "supersede should apply: {:?}", sup.error);

        let outcome = process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .unwrap();
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        // The retired row is back in play …
        let restored = store.get_memory_row(old_id).await.unwrap().unwrap();
        assert_eq!(restored.id, old_id);
        // … and its replacement is gone.
        assert!(
            !live_ids(&store).await.contains(&sup.memory_id.unwrap()),
            "the replacement row should have been retired by the rollback"
        );
    }

    #[tokio::test]
    async fn rolling_back_twice_is_refused() {
        let run_id = "run-rb-twice";
        let store = store_with_candidates(run_id, 1).await;
        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [{ "op": "ADD", "candidate_index": 0 }]
            }),
        )
        .await;

        process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .unwrap();
        let err = process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .expect_err("a second rollback must be refused");

        let msg = format!("{err:#}");
        assert!(
            msg.contains("already been rolled back"),
            "error should say why: {msg}"
        );
    }

    #[tokio::test]
    async fn rollback_skips_ops_that_never_applied() {
        let run_id = "run-rb-partial";
        let store = store_with_candidates(run_id, 1).await;
        let before = live_ids(&store).await;

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [
                    { "op": "ADD", "candidate_index": 0 },
                    // Never applies: the id is invented.
                    { "op": "MERGE", "candidate_index": 0, "into_memory_id": 987_654 },
                    // Never applies: out of range.
                    { "op": "ADD", "candidate_index": 99 },
                ]
            }),
        )
        .await;

        let outcome = process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .unwrap();
        assert_eq!(outcome.reversed, 1, "only the ADD actually landed");
        assert_eq!(
            outcome.skipped, 2,
            "the two rejected ops have nothing to undo"
        );
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(live_ids(&store).await, before);
    }

    #[tokio::test]
    async fn rolling_back_an_unknown_run_is_an_error() {
        let store = store_with_candidates("run-rb-none", 1).await;
        let err = process_consolidation_rollback(&store, "no-such-run")
            .await
            .expect_err("there is nothing to roll back");
        assert!(format!("{err:#}").contains("no consolidation run"));
    }

    #[tokio::test]
    async fn history_summarises_runs_and_marks_rollbacks() {
        let store = store_with_candidates("run-h1", 1).await;
        consolidate(
            &store,
            "run-h1",
            json!({
                "session_summary": "",
                "operations": [
                    { "op": "ADD", "candidate_index": 0 },
                    { "op": "ADD", "candidate_index": 42 },
                ]
            }),
        )
        .await;

        let runs = store.recent_consolidation_runs(10).await.unwrap();
        let run = runs
            .iter()
            .find(|r| r.run_id == "run-h1")
            .expect("run listed");
        assert_eq!(run.ops, 2);
        assert_eq!(run.applied, 1, "one op was rejected");
        assert!(!run.rolled_back);

        let batch = run.batch_id.clone();
        process_consolidation_rollback(&store, &batch)
            .await
            .unwrap();

        let runs = store.recent_consolidation_runs(10).await.unwrap();
        let run = runs.iter().find(|r| r.batch_id == batch).unwrap();
        assert!(run.rolled_back, "history must show the run was reversed");
    }

    /// The v16 fix. A rollback must mark the invocation it undid and
    /// nothing else — the flag is replayed into the consolidator prompt
    /// as "[ROLLED BACK BY THE USER]", so a session-wide flag told every
    /// later run that all its own work had been rejected.
    #[tokio::test]
    async fn rolling_back_one_run_does_not_mark_the_next_one() {
        let run_id = "run-two-batches";
        let store = store_with_candidates(run_id, 2).await;

        let add_first = json!({
            "session_summary": "",
            "operations": [{ "op": "ADD", "candidate_index": 0 }]
        });
        consolidate(&store, run_id, add_first.clone()).await;
        let first = latest_batch_id(&store).await;

        consolidate(&store, run_id, add_first).await;
        let second = latest_batch_id(&store).await;
        assert_ne!(
            first, second,
            "two consolidations of one session must get distinct ids"
        );

        process_consolidation_rollback(&store, &first)
            .await
            .unwrap();

        let runs = store.recent_consolidation_runs(10).await.unwrap();
        let first_run = runs.iter().find(|r| r.batch_id == first).unwrap();
        let second_run = runs.iter().find(|r| r.batch_id == second).unwrap();
        assert!(first_run.rolled_back, "the undone run should be marked");
        assert!(
            !second_run.rolled_back,
            "the untouched run must not inherit the flag"
        );

        // And the second run is still undoable in its own right.
        process_consolidation_rollback(&store, &second)
            .await
            .expect("a later run stays rollable after an earlier one is undone");
    }

    /// The prompt-facing consequence of the same fix.
    #[tokio::test]
    async fn a_rolled_back_run_does_not_taint_the_next_prompt() {
        let run_id = "run-taint";
        let store = store_with_candidates(run_id, 2).await;

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [{ "op": "ADD", "candidate_index": 0 }]
            }),
        )
        .await;
        let first = latest_batch_id(&store).await;
        process_consolidation_rollback(&store, &first)
            .await
            .unwrap();

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [{ "op": "ADD", "candidate_index": 1 }]
            }),
        )
        .await;

        // A third run's prompt sees both: the undone one marked, the
        // other not. Before v16 every entry carried the marker.
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(CapturingLlm {
            response: json!({ "session_summary": "", "operations": [] }).to_string(),
            seen: seen.clone(),
        });
        consolidate_inline(&store, &llm, "r1", None, run_id, "transcript")
            .await
            .unwrap();

        let prompt = seen.lock().unwrap()[0].clone();
        let marker = "[ROLLED BACK BY THE USER]";
        assert_eq!(
            prompt.matches(marker).count(),
            1,
            "exactly the undone run should carry the marker:\n{prompt}"
        );
    }

    // ── B-fix: the model call runs off the writer task ───────────────

    /// A consolidator that takes a visible amount of wall-clock time,
    /// like the real one (a measured live run took 3m41s).
    struct SlowLlm {
        delay: Duration,
        response: String,
        calls: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl ConsolidationLlm for SlowLlm {
        async fn complete(&self, _prompt: String) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            Ok(self.response.clone())
        }
    }

    /// How long the fake model "thinks". Generous enough that the
    /// assertions below are about ordering, not about machine speed.
    const LLM_DELAY: Duration = Duration::from_millis(1500);
    /// Budget for work that must complete *while* the model is thinking.
    const WHILE_THINKING: Duration = Duration::from_millis(600);

    async fn writer_with_slow_llm(
        store: Arc<MemoryStore>,
        response: &str,
    ) -> (WriterHandle, Arc<AtomicU64>) {
        let calls = Arc::new(AtomicU64::new(0));
        let llm: Arc<dyn ConsolidationLlm> = Arc::new(SlowLlm {
            delay: LLM_DELAY,
            response: response.to_string(),
            calls: calls.clone(),
        });
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store),
            llm: Some(llm),
            observer: None,
            manifest_observer: None,
        });
        (handle, calls)
    }

    fn one_add_response() -> String {
        json!({
            "session_summary": "the session did things",
            "operations": [{ "op": "ADD", "candidate_index": 0 }]
        })
        .to_string()
    }

    /// The regression this whole split exists for. The caller used to
    /// wait out the entire model round-trip under a 30s ack budget, and
    /// report "writer ack timeout after 30000ms" for work that in fact
    /// succeeded three minutes later.
    #[tokio::test]
    async fn session_consolidate_acks_before_the_model_answers() {
        let run_id = "run-ack-fast";
        let store = store_with_candidates(run_id, 1).await;
        let (handle, calls) = writer_with_slow_llm(store, &one_add_response()).await;

        let started = tokio::time::Instant::now();
        let res = handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .expect("ack arrives");
        let elapsed = started.elapsed();

        assert!(
            matches!(res, WriteResult::Queued),
            "expected Queued, got {res:?}"
        );
        assert!(
            elapsed < WHILE_THINKING,
            "ack waited on the model: {elapsed:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the model was still asked");
    }

    /// The other half of the same defect: the writer is single-consumer,
    /// so an inline model call stalled *every* memory write for its
    /// duration.
    #[tokio::test]
    async fn other_writes_proceed_while_the_model_is_thinking() {
        let run_id = "run-not-blocking";
        let store = store_with_candidates(run_id, 1).await;
        let (handle, _) = writer_with_slow_llm(store, &one_add_response()).await;

        handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();

        let started = tokio::time::Instant::now();
        let result = handle
            .user_remember("ns", "k1", "an unrelated write", None)
            .await
            .expect("unrelated write is not blocked by the consolidator");
        let elapsed = started.elapsed();

        assert!(matches!(result, WriteResult::Inserted(_)));
        assert!(
            elapsed < WHILE_THINKING,
            "the writer was blocked behind the model call: {elapsed:?}"
        );
    }

    /// The operations must still land — off the writer task, but on it
    /// for the writes themselves, via `SessionConsolidateApply`.
    #[tokio::test]
    async fn the_operations_land_once_the_model_answers() {
        let run_id = "run-deferred-apply";
        let store = store_with_candidates(run_id, 1).await;
        let (handle, _) = writer_with_slow_llm(store.clone(), &one_add_response()).await;

        handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();

        assert!(
            store
                .consolidation_audit_for_run(run_id)
                .await
                .unwrap()
                .is_empty(),
            "nothing should have been applied before the model answered"
        );

        let rows = await_audit_rows(&store, run_id).await;
        let kinds: Vec<&str> = rows.iter().map(|r| r.kind.as_str()).collect();
        assert!(kinds.contains(&"session_summary"), "kinds: {kinds:?}");
        assert!(kinds.contains(&"add"), "kinds: {kinds:?}");
        assert!(
            rows.iter().all(|r| r.applied),
            "every op should have applied: {rows:?}"
        );
    }

    /// Poll until the background consolidation has enqueued and the
    /// writer has applied it.
    async fn await_audit_rows(
        store: &Arc<MemoryStore>,
        run_id: &str,
    ) -> Vec<crate::memory::store::consolidation_ops::ConsolidationAuditRecord> {
        for _ in 0..100 {
            let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
            if !rows.is_empty() {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("consolidation never applied its operations");
    }

    /// Moving the model call off the writer let two `/consolidate-session`
    /// invocations overlap — which the observed double-run did. Runs are
    /// keyed by `run_id`, so overlapping ones cannot be told apart in
    /// history or rolled back separately.
    #[tokio::test]
    async fn a_second_run_for_the_same_session_is_refused_while_one_is_in_flight() {
        let run_id = "run-duplicate";
        let store = store_with_candidates(run_id, 1).await;
        let (handle, calls) = writer_with_slow_llm(store.clone(), &one_add_response()).await;

        let first = handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();
        let second = handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();

        assert!(matches!(first, WriteResult::Queued));
        assert!(
            matches!(second, WriteResult::Skipped),
            "a concurrent duplicate should be refused, got {second:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the duplicate must not spend a second model call"
        );

        // And the slot is released, so the session can consolidate again
        // once the first run has landed.
        await_audit_rows(&store, run_id).await;
        let third = handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();
        assert!(
            matches!(third, WriteResult::Queued),
            "in-flight slot was never released, got {third:?}"
        );
    }

    /// A different conversation is unrelated and must not be blocked by
    /// the guard.
    #[tokio::test]
    async fn a_different_session_may_consolidate_concurrently() {
        let store = store_with_candidates("run-a", 1).await;
        store
            .store_scoped(
                &WriteScope::Run {
                    repo_id: "r1".into(),
                    run_id: "run-b".into(),
                },
                "a candidate belonging to the other session",
                &WriteMeta::default(),
            )
            .await
            .unwrap();
        let (handle, _) = writer_with_slow_llm(store, &one_add_response()).await;

        let a = handle
            .session_consolidate("sess", "r1", None, "run-a", "transcript")
            .await
            .unwrap();
        let b = handle
            .session_consolidate("sess", "r1", None, "run-b", "transcript")
            .await
            .unwrap();

        assert!(matches!(a, WriteResult::Queued));
        assert!(
            matches!(b, WriteResult::Queued),
            "the guard is per-session, got {b:?}"
        );
    }

    #[tokio::test]
    async fn user_remember_round_trip_fires_enqueue_then_commit() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let observer = Arc::new(RecordingObserver::default());

        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: Some(observer.clone() as Arc<dyn MemoryObserver>),
            manifest_observer: None,
        });

        let result = handle
            .user_remember("ns", "k1", "hello world", None)
            .await
            .expect("ack within 500ms");

        match result {
            WriteResult::Inserted(id) => assert!(id > 0),
            other => panic!("expected Inserted, got {other:?}"),
        }

        // Give the writer task a moment to run the committed hook (it fires
        // after the ack from within the same message, so a yield is enough).
        tokio::task::yield_now().await;
        // Small wait ensures committed hook records (the ack is sent before
        // the observer call, so in rare scheduling races the observer may
        // log committed after the await returns).
        for _ in 0..10 {
            if observer.events.lock().unwrap().len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let events = observer.events.lock().unwrap().clone();
        assert_eq!(events.len(), 2, "events were: {events:?}");
        assert_eq!(events[0], "enqueued:UserRemember");
        assert_eq!(events[1], "committed:UserRemember");
    }

    #[tokio::test]
    async fn queue_depth_reflects_enqueue_drain() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store),
            llm: None,
            observer: None,
            manifest_observer: None,
        });
        // Before any enqueue, depth is 0.
        assert_eq!(handle.queue_depth(), 0);
        let _ = handle
            .user_remember("ns", "k", "c", None)
            .await
            .expect("ack");
        // After the ack round-trip, drained has caught up to enqueued.
        for _ in 0..10 {
            if handle.queue_depth() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(handle.queue_depth(), 0);
    }

    #[test]
    fn writer_message_kind_is_stable() {
        let m = WriterMessage::TurnComplete {
            session_id: "s".into(),
            turn_id: "t".into(),
            repo_id: "repo-1".into(),
            module_path: Some("crates/core".into()),
            run_id: "run-1".into(),
            transcript: "".into(),
            annotations: None,
        };
        assert_eq!(m.kind(), "TurnComplete");
    }

    #[tokio::test]
    async fn flush_barrier_drains_prior_messages() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: None,
            manifest_observer: None,
        });

        handle
            .enqueue(WriterMessage::InjectionManifest {
                turn_id: "t-flush".into(),
                session_id: "conv-flush".into(),
                payload: serde_json::json!({
                    "schema_version": 1,
                    "query_text": "x",
                    "selected_ids": [],
                }),
            })
            .unwrap();

        // No polling: `flush` must not resolve until the FIFO-prior write
        // has been fully processed.
        handle.flush().await.unwrap();

        let rows = store.recent_manifests(10).await.unwrap();
        assert_eq!(rows.len(), 1, "flush must guarantee the prior write landed");
        assert_eq!(rows[0].turn_id, "t-flush");
        assert_eq!(handle.queue_depth(), 0);
    }

    #[tokio::test]
    async fn injection_manifest_round_trip_persists_row() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: None,
            manifest_observer: None,
        });

        let payload = serde_json::json!({
            "schema_version": 1,
            "query_text": "hello",
            "selected_ids": [1, 2, 3],
        });
        handle
            .enqueue(WriterMessage::InjectionManifest {
                turn_id: "t-1".into(),
                session_id: "conv-1".into(),
                payload,
            })
            .unwrap();

        // Drain — the writer task is async, so poll briefly.
        for _ in 0..20 {
            if !store.recent_manifests(10).await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let rows = store.recent_manifests(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turn_id, "t-1");
        assert_eq!(rows[0].session_id, "conv-1");
        assert_eq!(rows[0].source_channel, "chat");
        let parsed: serde_json::Value = serde_json::from_str(&rows[0].payload).unwrap();
        assert_eq!(parsed["query_text"], "hello");
    }

    #[tokio::test]
    async fn a1_turn_complete_with_annotations_writes_flags_and_ledger() {
        // Plan §A1 acceptance: `<turn_annotations>` flags appear in
        // memory tagged `source = llm_annotated`, `trust_score = 0.7`;
        // `session_thread` + `open_questions` land in the ledger.
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: None,
            manifest_observer: None,
        });

        let payload = serde_json::json!({
            "v": 1,
            "flags": [
                { "type": "decision", "text": "use git2, never shell git",
                  "importance": 0.9, "scope": "repo", "refs": ["src/git.rs"] }
            ],
            "session_thread": "git backend choice",
            "open_questions": ["what about submodules?"]
        });
        handle
            .turn_complete(
                "conv-1",
                "t-a1",
                "repo-1",
                Some("crates/core".to_string()),
                "conv-1",
                "USER: ...\nASSISTANT: ...",
                Some(payload),
            )
            .unwrap();

        // Allow the async writer task to drain.
        for _ in 0..20 {
            if handle.queue_depth() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let rows = store.session_ledger_for("conv-1", 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].turn_id, "t-a1");
        assert_eq!(
            rows[0].session_thread.as_deref(),
            Some("git backend choice")
        );
        assert_eq!(rows[0].open_questions(), vec!["what about submodules?"]);

        // The flag landed as an llm_annotated memory with trust_score 0.7
        // in the concrete repo scope threaded through TurnComplete.
        use crate::memory::ScopeFilter;
        let hits = store
            .search_at_level(
                &ScopeFilter::Repo {
                    repo_id: "repo-1".to_string(),
                },
                "",
                10,
            )
            .await
            .unwrap();
        let annotated = hits
            .iter()
            .find(|m| m.source == crate::memory::MemorySource::LlmAnnotated);
        assert!(
            annotated.is_some(),
            "expected llm_annotated memory; got {:?}",
            hits.iter()
                .map(|m| (m.source, &m.content))
                .collect::<Vec<_>>()
        );
        let annotated = annotated.unwrap();
        assert!((annotated.trust_score - 0.7).abs() < 1e-6);
    }

    #[tokio::test]
    async fn turn_complete_without_llm_writes_run_scope_fallback() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: None,
            manifest_observer: None,
        });

        handle
            .turn_complete(
                "conv-1",
                "t-fallback",
                "repo-1",
                Some("crates/core".to_string()),
                "conv-1",
                "USER: hello\nASSISTANT: hi back",
                None,
            )
            .unwrap();

        // The fallback inserts at Run scope; wait for drain.
        tokio::time::sleep(Duration::from_millis(80)).await;

        // The fallback writes at Run scope with the provided repo/run id.
        assert_eq!(handle.queue_depth(), 0);
    }

    /// C1.2: TurnComplete writes a `kind = history` row carrying the
    /// verbatim transcript at Run scope, regardless of whether the
    /// extractor LLM is configured (None here).
    #[tokio::test]
    async fn c1_turn_complete_writes_history_row() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: None,
            manifest_observer: None,
        });

        let transcript = "USER: c1 history check\nASSISTANT: ack";
        handle
            .turn_complete(
                "conv-c1", "t-c1", "repo-c1", None, "conv-c1", transcript, None,
            )
            .unwrap();

        // Drain.
        for _ in 0..40 {
            if handle.queue_depth() == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Read the row through the public lookup and assert it
        // carries the History kind with the transcript intact.
        let tag = "history:conv-c1:t-c1";
        let found = store
            .find_memory_by_tag(tag)
            .await
            .expect("lookup ok")
            .expect("history row must exist");
        let (id, kind, content) = found;
        assert!(id > 0);
        assert_eq!(kind, super::super::kind::MemoryKind::History);
        assert_eq!(content, transcript);

        // get_memory_kind matches.
        let by_id = store.get_memory_kind(id).await.unwrap();
        assert_eq!(by_id, Some(super::super::kind::MemoryKind::History));
    }

    /// C1.2: PanelEdit operations refuse to mutate history rows. The
    /// guard runs in the writer task, before SQL hits the DB. (The
    /// SQL trigger from C1.3 will be the second line of defense.)
    #[tokio::test]
    async fn c1_panel_edit_refuses_to_touch_history() {
        use super::super::scope::{MemoryType, WriteMeta, WriteScope};
        use super::super::trust_defaults::MemorySource;
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let store = Arc::new(MemoryStore::in_memory(embedder).unwrap());
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: None,
            observer: None,
            manifest_observer: None,
        });

        // Seed one history row + one record row at the same scope.
        let scope = WriteScope::Run {
            repo_id: "repo-x".into(),
            run_id: "run-x".into(),
        };
        let history_meta = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(super::super::kind::MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("history-x");
        let record_meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("record-x");

        let history_id = match store
            .store_scoped(&scope, "TRANSCRIPT body", &history_meta)
            .await
            .unwrap()
        {
            super::super::scope::StoreResult::Inserted(id) => id,
            _ => panic!("history insert should succeed"),
        };
        let record_id = match store
            .store_scoped(&scope, "Record body", &record_meta)
            .await
            .unwrap()
        {
            super::super::scope::StoreResult::Inserted(id) => id,
            _ => panic!("record insert should succeed"),
        };

        // Try to delete the history row through the panel — must fail.
        let (tx, rx) = oneshot::channel();
        handle
            .enqueue(WriterMessage::PanelEdit {
                op: PanelEditOp::Delete {
                    memory_id: history_id,
                },
                scope_level: super::super::scope::SCOPE_RUN,
                repo_id: Some("repo-x".into()),
                ack: Some(tx),
            })
            .unwrap();
        let result = tokio::time::timeout(Duration::from_millis(500), rx)
            .await
            .expect("ack within timeout")
            .unwrap();
        assert!(
            result.is_err(),
            "delete of history row must be rejected: {result:?}"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("history is append-only") || err_msg.contains("history row"),
            "unexpected error: {err_msg}"
        );

        // The record path through the same op still works.
        let (tx, rx) = oneshot::channel();
        handle
            .enqueue(WriterMessage::PanelEdit {
                op: PanelEditOp::Delete {
                    memory_id: record_id,
                },
                scope_level: super::super::scope::SCOPE_RUN,
                repo_id: Some("repo-x".into()),
                ack: Some(tx),
            })
            .unwrap();
        let result = tokio::time::timeout(Duration::from_millis(500), rx)
            .await
            .expect("ack within timeout")
            .unwrap();
        assert!(
            result.is_ok(),
            "delete of record row must succeed: {result:?}"
        );
    }
}
