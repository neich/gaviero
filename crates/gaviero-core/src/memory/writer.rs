//! Single-consumer writer task that serializes every write into `MemoryStore`.
//!
//! Transport (currently `tokio::sync::mpsc::UnboundedSender`) is encapsulated
//! behind `WriterHandle`; swapping mpsc for an IPC channel would touch only
//! this file. Callers never hold a `MemoryStore` reference for writes — they
//! enqueue a `WriterMessage` and, if they need confirmation, await the
//! optional `oneshot` ack with a 500ms timeout.
//!
//! Lock discipline: the writer task body never holds a `tokio::sync::Mutex`
//! guard across an `await`, tree-sitter call, or filesystem I/O. Embedding
//! runs inside `MemoryStore::store*` which already sequences "embed first,
//! lock briefly, release" — the task simply funnels messages into those
//! methods without re-acquiring any lock of its own.

#![deny(clippy::await_holding_lock)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value as JsonValue;
use tokio::sync::{mpsc, oneshot};

use super::consolidation_llm::ConsolidationLlm;
use super::observer::{ManifestObserver, MemoryObserver};
use super::scope::{StoreResult, WriteMeta, WriteScope};
use super::store::{MemoryStore, StoreOptions};
use super::stores::MemoryStores;
use super::trust_defaults::MemorySource;

/// Default ack timeout for synchronous variants.
pub const ACK_TIMEOUT_MS: u64 = 500;

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
    PanelEdit {
        op: PanelEditOp,
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
    /// Tier B / B5: end-of-session consolidator. The writer task
    /// gathers transcript + recent extractions, calls
    /// [`ConsolidationLlm`], parses the response into
    /// [`ConsolidationOp`]s, and applies them. `ack` lets the TUI's
    /// `/consolidate-session` command surface success/failure inline.
    SessionConsolidate {
        session_id: String,
        repo_id: String,
        module_path: Option<String>,
        run_id: String,
        transcript: String,
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
            WriterMessage::TelemetryClassify { .. } => "TelemetryClassify",
            WriterMessage::Restore { .. } => "Restore",
            WriterMessage::RestoreSince { .. } => "RestoreSince",
            WriterMessage::BulkForget { .. } => "BulkForget",
            WriterMessage::RedactHistory { .. } => "RedactHistory",
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
        match tokio::time::timeout(Duration::from_millis(ACK_TIMEOUT_MS), rx).await {
            Ok(Ok(Ok(r))) => Ok(r),
            Ok(Ok(Err(e))) => Err(anyhow!(e)),
            Ok(Err(_)) => Err(anyhow!("writer dropped ack channel")),
            Err(_) => Err(anyhow!("writer ack timeout after {}ms", ACK_TIMEOUT_MS)),
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
        let (tx, rx) = oneshot::channel();
        self.enqueue(WriterMessage::SwarmConsolidate {
            scope,
            content: content.into(),
            meta,
            ack: Some(tx),
        })?;
        Self::await_ack(rx).await
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
    pub async fn restore_deletion(
        &self,
        deletion_id: i64,
    ) -> Result<super::store::RestoreOutcome> {
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
    pub async fn redact_history(
        &self,
        memory_id: i64,
        reason: impl Into<String>,
    ) -> Result<i64> {
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
    tokio::spawn(writer_task(rx, cfg, inner));
    handle
}

/// Main drain loop. One message at a time — serialization of writes is the
/// whole point. If this ever needs parallelism, bound the channel and fan
/// out within the task body; do not add a second consumer to the channel.
async fn writer_task(
    mut rx: mpsc::UnboundedReceiver<WriterMessage>,
    cfg: WriterConfig,
    state: Arc<WriterHandleInner>,
) {
    let WriterConfig {
        stores,
        llm,
        observer,
        manifest_observer,
    } = cfg;

    while let Some(msg) = rx.recv().await {
        let kind = msg.kind();
        let drained = state.drained.fetch_add(1, Ordering::Relaxed) + 1;
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

        let outcome = process_message(&stores, llm.as_ref(), msg).await;
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
        WriterMessage::PanelEdit { op, ack } => {
            // Panel edits operate on rows in the store that owns the
            // selected memory id. Today every memory id is unique within
            // the workspace DB, so we route there. Step 7 generalises
            // this to look up the owning store by scope.
            let store = stores.workspace().clone();
            let res = process_panel_edit(&store, op).await;
            send_ack(ack, &res);
            res
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
            // Turn-complete extraction writes run-scope rows; lives in the
            // workspace store. Step 7 generalises if extractor promotes
            // to broader scopes.
            let store = stores.workspace().clone();
            process_turn_complete(
                &store,
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
                            .set_meta_value(
                                "last_sleeptime_at",
                                &chrono::Utc::now().to_rfc3339(),
                            )
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
            let res = process_session_consolidate(
                &store,
                llm,
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
        WriterMessage::TelemetryClassify {
            turn_id,
            session_id,
            response,
        } => {
            let store = stores.workspace().clone();
            process_telemetry_classify(&store, &turn_id, &session_id, &response).await
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
                super::store::RestoreOutcome::AlreadyCovered { .. } => {
                    WriteResult::AlreadyCovered
                }
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
fn send_ack_typed<T: Clone>(
    ack: Option<oneshot::Sender<Result<T, String>>>,
    res: &Result<T>,
) {
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

async fn process_session_consolidate(
    store: &Arc<MemoryStore>,
    llm: Option<&Arc<dyn ConsolidationLlm>>,
    _session_id: String,
    repo_id: String,
    module_path: Option<String>,
    run_id: String,
    transcript: String,
) -> Result<WriteResult> {
    use super::session_consolidator::{
        CandidateBrief, ExistingBrief, build_prompt, parse_response,
    };

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
    let recent = match store.recent_memories_for_run(&run_id, 24, 50).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(target: "memory_consolidator", error = %e, "skipping consolidate");
            return Ok(WriteResult::Skipped);
        }
    };
    let candidates: Vec<CandidateBrief> = recent
        .iter()
        .map(|m| CandidateBrief {
            text: m.content.clone(),
            kind: m.memory_type.as_str().to_string(),
            importance: m.importance,
        })
        .collect();
    if candidates.is_empty() {
        return Ok(WriteResult::Skipped);
    }

    let prompt = build_prompt(&transcript, &candidates, &[] as &[ExistingBrief]);
    let raw = match llm.complete(prompt).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "memory_consolidator", error = %e, "LLM failure; deferring");
            return Ok(WriteResult::Skipped);
        }
    };
    let parsed = match parse_response(&raw) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(target: "memory_consolidator", error = %e, "parse failed; logging raw");
            return Ok(WriteResult::Skipped);
        }
    };

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
    if !parsed.session_summary.trim().is_empty() {
        // C1: the consolidator's session-summary blob is the canonical
        // Summary kind row — semantic merge across similar sessions,
        // injected as session context when topically relevant.
        let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
            .with_importance(0.7)
            .with_type(super::scope::MemoryType::Factual)
            .with_kind(super::kind::MemoryKind::Summary)
            .with_tag(format!("session_summary:{run_id}"));
        let _ = store
            .store_scoped(&summary_scope, &parsed.session_summary, &meta)
            .await;
    }

    for op in parsed.operations {
        use super::session_consolidator::ConsolidationOp;
        match op {
            ConsolidationOp::Add { candidate_index } => {
                if let Some(c) = candidates.get(candidate_index) {
                    let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
                        .with_importance(c.importance)
                        .with_type(super::scope::MemoryType::parse_str(&c.kind));
                    let _ = store.store_scoped(&summary_scope, &c.text, &meta).await;
                }
            }
            ConsolidationOp::Merge { into_memory_id, .. } => {
                // Best-effort trust bump on the surviving row;
                // text-merge is deferred (the LLM picked the existing
                // row to "fold into" — keeping its content).
                let _ = store.set_trust_score(into_memory_id, 0.8).await;
            }
            ConsolidationOp::Supersede {
                candidate_index,
                supersedes_memory_id,
            } => {
                if let Some(c) = candidates.get(candidate_index) {
                    let meta = WriteMeta::for_source(MemorySource::LlmConsolidated)
                        .with_importance(c.importance)
                        .with_type(super::scope::MemoryType::parse_str(&c.kind));
                    if let Ok(res) = store.store_scoped(&summary_scope, &c.text, &meta).await {
                        let new_id = match res {
                            super::scope::StoreResult::Inserted(id)
                            | super::scope::StoreResult::Deduplicated(id) => Some(id),
                            super::scope::StoreResult::AlreadyCovered => None,
                        };
                        if let Some(id) = new_id {
                            let _ = store.supersede_memory(supersedes_memory_id, id).await;
                        }
                    }
                }
            }
            ConsolidationOp::Drop { .. } => {
                // Drop is the no-op decision (don't promote the
                // candidate beyond Run scope). Per-turn extractor's
                // run-scope row stays; nothing to do here.
            }
        }
    }
    Ok(WriteResult::Skipped)
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

/// Per-turn extractor path (Tier S / S3).
///
/// Flow:
/// 1. Invoke the configured `ConsolidationLlm` against the version-pinned
///    extractor prompt.
/// 2. Parse 0–5 candidates; drop below-threshold importance.
/// 3. Resolve each `scope_hint` to a concrete `WriteScope`, build
///    `WriteMeta` with `source="llm_extracted"` + `trust=Medium`, and
///    call `MemoryStore::store_scoped` (which owns SHA + cosine dedup
///    inside the brief connection lock — see `store.rs`).
/// 4. On LLM unavailable / JSON parse failure: write one Run-scope raw
///    record so the turn is never lost.
///
/// **Never blocks the chat event loop** — this runs inside the writer
/// task, which the chat path enqueues into and returns from immediately.
/// Per-stage latency is logged via `tracing` with a correlation id built
/// from `turn_id` for post-hoc perf work.
async fn process_turn_complete(
    store: &Arc<MemoryStore>,
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
    let history_id = write_history_row(store, &session_id, &turn_id, &repo_id, &run_id, &transcript)
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
                // Persist session ledger row (best-effort).
                let annotations_json = serde_json::to_string(&parsed).ok();
                if let Err(e) = store
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
                    match store.store_scoped(&scope, &f.text, &meta).await {
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
                return fallback_raw_turn(store, &repo_id, &run_id, &transcript).await;
            }
        },
        None => {
            tracing::debug!(
                target: "memory_extractor",
                turn_id = %turn_id,
                "no ConsolidationLlm configured — writing Run-scope fallback"
            );
            return fallback_raw_turn(store, &repo_id, &run_id, &transcript).await;
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
        match store.store_scoped(&scope, &ext.text, &meta).await {
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
                "conv-c1",
                "t-c1",
                "repo-c1",
                None,
                "conv-c1",
                transcript,
                None,
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
                ack: Some(tx),
            })
            .unwrap();
        let result = tokio::time::timeout(Duration::from_millis(500), rx)
            .await
            .expect("ack within timeout")
            .unwrap();
        assert!(result.is_ok(), "delete of record row must succeed: {result:?}");
    }
}
