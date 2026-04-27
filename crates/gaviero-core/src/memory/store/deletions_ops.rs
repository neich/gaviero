//! Tier C / C2.x deletion + restore + redaction surface on
//! `MemoryStore`. Carved out of `store/mod.rs` (Phase 4.5).
//!
//! Cohesively owns:
//!
//! - the three audit types `ForgetFilter`, `BulkForgetReport`,
//!   `RestoreOutcome`,
//! - the private `RawMemoryRow` projection used to capture the audit
//!   `original_row_json`,
//! - eight methods on `MemoryStore` covering soft-delete, recent /
//!   single audit reads, retention prune, restore (single + bulk),
//!   `/forget` preview + bulk execute, and the C2.4 redaction path,
//! - three private helpers (`forget_predicate`, `scope_from_payload`,
//!   `meta_from_payload`).
//!
//! `redact_history_row` is the second of the two sanctioned callsites
//! of `schema::drop_history_immutable_triggers`; the C2.4 grep
//! invariant test (in `tests/c24_trigger_disable_invariant.rs`)
//! enforces that this list stays at exactly two entries inside the
//! `store/` subtree.

use anyhow::{Context, Result, anyhow};

use super::MemoryStore;
use crate::memory::deletions::{DeletedBy, DeletedRow};
use crate::memory::scope::{MemoryType, StoreResult, WriteMeta, WriteScope};
use crate::memory::schema;
use crate::memory::trust_defaults::MemorySource;

// ── Public types ───────────────────────────────────────────────

/// C2.3: filter used by `/forget*` bulk-delete paths. Each variant
/// either translates into a SQL predicate via `forget_predicate`, or
/// routes each id through [`MemoryStore::soft_delete_memory`] so
/// the audit trail stays consistent.
///
/// History rows are excluded from every variant. The C1.3 SQL trigger
/// would block a hard-delete anyway, but we also bar them at the
/// application layer so dry-run counts don't include rows that
/// `/forget` could never act on. The `/forget-history` redaction path
/// (C2.4) is the only legitimate way to mutate a history row.
#[derive(Debug, Clone)]
pub enum ForgetFilter {
    /// Free-text fuzzy match against `content`. Case-insensitive
    /// `LIKE %query%`. Records and Summaries only.
    ByQuery(String),
    /// Every row at a specific scope (canonical `scope_path` value).
    /// Pass the same string [`crate::memory::scope::WriteScope::to_path_string`]
    /// produces.
    ByScope { scope_level: i32, scope_path: String },
    /// Every row of a given [`MemoryType`].
    ByType(MemoryType),
    /// Every row produced by a given [`MemorySource`] — e.g.
    /// `LlmExtracted` for a "factory reset of LLM extractions".
    BySource(MemorySource),
}

/// C2.3: result of a [`MemoryStore::bulk_forget`] invocation. The
/// `candidates` vector lists the ids the filter matched (so the TUI
/// can render a per-row preview); `kind_breakdown` and `scope_breakdown`
/// power the confirmation prompt's at-a-glance counts.
#[derive(Debug, Clone, Default)]
pub struct BulkForgetReport {
    pub candidates: Vec<i64>,
    pub kind_breakdown: std::collections::BTreeMap<String, usize>,
    pub scope_breakdown: std::collections::BTreeMap<String, usize>,
    pub deleted: usize,
    pub dry_run: bool,
}

/// C2.2: outcome of a [`MemoryStore::restore_deletion`] call. The
/// payload from `deletions.original_row_json` is replayed through
/// [`MemoryStore::store_scoped`], so the dedup pipeline has the same
/// three terminal states as a fresh write — plus a `Refused` arm for
/// rows the policy bars from restore (`user_redaction`, `history`).
#[derive(Debug, Clone)]
pub enum RestoreOutcome {
    /// New row inserted at the original scope.
    Inserted {
        deletion_id: i64,
        new_memory_id: i64,
    },
    /// Existing row at the same scope absorbed the payload (dedup
    /// reinforced the surviving row instead of creating a new one).
    Deduplicated {
        deletion_id: i64,
        surviving_memory_id: i64,
    },
    /// Content already lives at a broader ancestor scope — restore
    /// skipped, audit row consumed anyway.
    AlreadyCovered { deletion_id: i64 },
    /// Policy refused the restore (`deleted_by = user_redaction` is
    /// one-way; `memory_kind = history` is append-only). The audit
    /// row is left in place so the user can see why.
    Refused { deletion_id: i64, reason: String },
}

// ── Private projection used by the audit capture path ──────────

/// C2.1: full raw projection of a `memories` row, used to capture
/// the audit `original_row_json` for soft-delete and to drive the
/// restore path (C2.2). Carries everything we'd need to reconstruct
/// the row at insert time, plus C1.4's compression columns so a
/// round-trip preserves the on-disk encoding.
#[derive(Debug, Clone)]
struct RawMemoryRow {
    id: i64,
    namespace: String,
    key: String,
    content: String,
    embedding: Option<Vec<u8>>,
    model_id: Option<String>,
    scope_level: i32,
    scope_path: String,
    repo_id: Option<String>,
    module_path: Option<String>,
    run_id: Option<String>,
    content_hash: Option<String>,
    memory_type: String,
    trust: String,
    tag: Option<String>,
    importance: f32,
    privacy: String,
    source: String,
    trust_score: f32,
    memory_kind: String,
    compressed: i64,
    content_blob: Option<Vec<u8>>,
    created_at: String,
    updated_at: String,
    last_accessed_at: Option<String>,
    access_count: i64,
    superseded_by: Option<i64>,
}

impl RawMemoryRow {
    fn from_query_row(r: &rusqlite::Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            namespace: r.get(1)?,
            key: r.get(2)?,
            content: r.get(3)?,
            embedding: r.get(4)?,
            model_id: r.get(5)?,
            scope_level: r.get(6)?,
            scope_path: r.get(7)?,
            repo_id: r.get(8)?,
            module_path: r.get(9)?,
            run_id: r.get(10)?,
            content_hash: r.get(11)?,
            memory_type: r.get(12)?,
            trust: r.get(13)?,
            tag: r.get(14)?,
            importance: r.get(15)?,
            privacy: r.get(16)?,
            source: r.get(17)?,
            trust_score: r.get(18)?,
            memory_kind: r.get(19)?,
            compressed: r.get(20)?,
            content_blob: r.get(21)?,
            created_at: r.get(22)?,
            updated_at: r.get(23)?,
            last_accessed_at: r.get(24)?,
            access_count: r.get(25)?,
            superseded_by: r.get(26)?,
        })
    }

    /// Serialize as JSON for the `deletions.original_row_json` audit
    /// payload. Embeddings + content_blob ride as base64 so the JSON
    /// stays printable.
    fn to_json_value(&self) -> serde_json::Value {
        use base64::Engine;
        let b64 = |bytes: &Option<Vec<u8>>| {
            bytes
                .as_ref()
                .map(|b| base64::engine::general_purpose::STANDARD.encode(b))
        };
        serde_json::json!({
            "id": self.id,
            "namespace": self.namespace,
            "key": self.key,
            "content": self.content,
            "embedding_b64": b64(&self.embedding),
            "model_id": self.model_id,
            "scope_level": self.scope_level,
            "scope_path": self.scope_path,
            "repo_id": self.repo_id,
            "module_path": self.module_path,
            "run_id": self.run_id,
            "content_hash": self.content_hash,
            "memory_type": self.memory_type,
            "trust": self.trust,
            "tag": self.tag,
            "importance": self.importance,
            "privacy": self.privacy,
            "source": self.source,
            "trust_score": self.trust_score,
            "memory_kind": self.memory_kind,
            "compressed": self.compressed,
            "content_blob_b64": b64(&self.content_blob),
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "last_accessed_at": self.last_accessed_at,
            "access_count": self.access_count,
            "superseded_by": self.superseded_by,
        })
    }
}

// ── MemoryStore methods ────────────────────────────────────────

impl MemoryStore {
    /// C2.1: soft-delete a memory by id. Captures the full row into
    /// the `deletions` audit table (with `deleted_by` and an optional
    /// `reason`), then hard-deletes the source row. The audit row is
    /// later auto-pruned past its retention window via
    /// [`Self::prune_expired_deletions`].
    ///
    /// Refuses for `memory_kind = 'history'`: the C1.3 trigger would
    /// veto the hard-delete anyway, but we abort earlier with a clear
    /// error so the caller can route to the C2.4 `/forget-history`
    /// path explicitly.
    ///
    /// `merged_into` is set when the deletion is a Sleeptime merge —
    /// it stores the surviving row's id inside `original_row_json`
    /// so the restore path can carry the merge edge through dedup.
    /// Returns the audit row's id on success.
    pub async fn soft_delete_memory(
        &self,
        memory_id: i64,
        deleted_by: DeletedBy,
        reason: Option<&str>,
        merged_into: Option<i64>,
    ) -> Result<i64> {
        // Read the full row body so the audit captures everything we
        // would need to reconstruct it. Includes content_blob and the
        // compressed flag so a round-trip can preserve compression.
        let mut conn = self.conn.lock().await;
        let row: Result<RawMemoryRow, rusqlite::Error> = conn.query_row(
            "SELECT id, namespace, key, content, embedding, model_id,
                    scope_level, scope_path, repo_id, module_path, run_id,
                    content_hash, memory_type, trust, tag,
                    importance, privacy, source, trust_score,
                    memory_kind, compressed, content_blob,
                    created_at, updated_at, last_accessed_at, access_count,
                    superseded_by
               FROM memories WHERE id = ?1",
            rusqlite::params![memory_id],
            RawMemoryRow::from_query_row,
        );
        let row = match row {
            Ok(r) => r,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Err(anyhow!("memory id {memory_id} not found"));
            }
            Err(e) => return Err(anyhow!("reading row for soft-delete: {e}")),
        };

        if row.memory_kind == "history" {
            return Err(anyhow!(
                "cannot soft-delete history row {memory_id}: history is append-only \
                 (use /forget-history to redact via RedactHistory)"
            ));
        }

        // Refuse if the caller passed UserRedaction here. That tag is
        // reserved for the dedicated C2.4 RedactHistory path which
        // does not go through soft_delete_memory.
        if matches!(deleted_by, DeletedBy::UserRedaction) {
            return Err(anyhow!(
                "DeletedBy::UserRedaction is reserved for the C2.4 RedactHistory path"
            ));
        }

        let mut row_json = row.to_json_value();
        if let Some(into) = merged_into
            && let Some(obj) = row_json.as_object_mut()
        {
            obj.insert("merged_into".to_string(), serde_json::Value::from(into));
        }
        let row_json_str = serde_json::to_string(&row_json)
            .context("serializing original row for deletions audit")?;

        let tx = conn.transaction().context("soft-delete: begin tx")?;
        let audit_id = tx
            .query_row(
                "INSERT INTO deletions (
                    memory_id, memory_content_hash, memory_kind, memory_source,
                    memory_trust, deleted_by, reason, original_row_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 RETURNING id",
                rusqlite::params![
                    memory_id,
                    row.content_hash,
                    row.memory_kind,
                    row.source,
                    row.trust_score,
                    deleted_by.as_str(),
                    reason,
                    row_json_str,
                ],
                |r| r.get::<_, i64>(0),
            )
            .context("inserting deletions audit row")?;

        // Hard-delete the source row + its derived index entries.
        // The C1.3 trigger only blocks history rows; we already
        // refused those above. Vec / FTS / access_log entries follow.
        let _ = tx.execute(
            "DELETE FROM vec_memories_scoped WHERE memory_id = ?1",
            rusqlite::params![memory_id],
        );
        let _ = tx.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            rusqlite::params![memory_id],
        );
        let _ = tx.execute(
            "DELETE FROM memory_access_log WHERE memory_id = ?1",
            rusqlite::params![memory_id],
        );
        tx.execute(
            "DELETE FROM memories WHERE id = ?1",
            rusqlite::params![memory_id],
        )
        .context("hard-deleting memory after audit")?;
        tx.commit().context("soft-delete: commit tx")?;

        Ok(audit_id)
    }

    /// C2.1: list the most recent N deletions, newest first. Drives
    /// the TUI Deletions tab (C2.6) and `gaviero-cli memory list-
    /// deletions` (C2.3 follow-up).
    pub async fn recent_deletions(&self, limit: usize) -> Result<Vec<DeletedRow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, memory_id, memory_content_hash, memory_kind, memory_source,
                    memory_trust, deleted_at, deleted_by, reason, original_row_json
               FROM deletions
              ORDER BY id DESC
              LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |r| {
                Ok(DeletedRow {
                    id: r.get(0)?,
                    memory_id: r.get(1)?,
                    memory_content_hash: r.get(2)?,
                    memory_kind: r.get(3)?,
                    memory_source: r.get(4)?,
                    memory_trust: r.get(5)?,
                    deleted_at: r.get(6)?,
                    deleted_by: r.get(7)?,
                    reason: r.get(8)?,
                    original_row_json: r.get(9)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// C2.1: fetch a single deletion-audit row by id. Returns `None`
    /// when the row has been auto-pruned past its retention window.
    pub async fn get_deletion(&self, deletion_id: i64) -> Result<Option<DeletedRow>> {
        let conn = self.conn.lock().await;
        let row: Result<DeletedRow, rusqlite::Error> = conn.query_row(
            "SELECT id, memory_id, memory_content_hash, memory_kind, memory_source,
                    memory_trust, deleted_at, deleted_by, reason, original_row_json
               FROM deletions WHERE id = ?1",
            rusqlite::params![deletion_id],
            |r| {
                Ok(DeletedRow {
                    id: r.get(0)?,
                    memory_id: r.get(1)?,
                    memory_content_hash: r.get(2)?,
                    memory_kind: r.get(3)?,
                    memory_source: r.get(4)?,
                    memory_trust: r.get(5)?,
                    deleted_at: r.get(6)?,
                    deleted_by: r.get(7)?,
                    reason: r.get(8)?,
                    original_row_json: r.get(9)?,
                })
            },
        );
        match row {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(anyhow!("get_deletion: {e}")),
        }
    }

    /// C2.1: hard-delete audit rows that have outlived their retention
    /// window. User-initiated deletions (panel + slash command +
    /// merge) age out at `user_retention_days`; sleeptime-prune
    /// expirations age out at the shorter `sleeptime_retention_days`.
    /// User-redactions share the user window so the actor + reason
    /// persist long enough for typical audit needs (per plan §C2).
    pub async fn prune_expired_deletions(
        &self,
        user_retention_days: u32,
        sleeptime_retention_days: u32,
    ) -> Result<usize> {
        let conn = self.conn.lock().await;
        let user_n = conn.execute(
            "DELETE FROM deletions
              WHERE deleted_by IN ('user_command','panel','sleeptime_merge','user_redaction')
                AND deleted_at < datetime('now', ?1)",
            rusqlite::params![format!("-{} days", user_retention_days)],
        )?;
        let sleep_n = conn.execute(
            "DELETE FROM deletions
              WHERE deleted_by = 'sleeptime_prune'
                AND deleted_at < datetime('now', ?1)",
            rusqlite::params![format!("-{} days", sleeptime_retention_days)],
        )?;
        Ok(user_n + sleep_n)
    }

    /// C2.1: total audit-table size for the metric callers expose.
    pub async fn deletions_count(&self) -> Result<i64> {
        let conn = self.conn.lock().await;
        Ok(conn.query_row("SELECT COUNT(*) FROM deletions", [], |r| r.get(0))?)
    }

    /// C2.2: restore a soft-deleted memory by audit id, replaying
    /// the captured `original_row_json` through the standard
    /// [`Self::store_scoped`] dedup pipeline. On success the audit
    /// row is consumed (hard-deleted) so a subsequent
    /// `/restore --since` can't replay it.
    ///
    /// Refused for `deleted_by = 'user_redaction'` (one-way per the
    /// plan) and for `memory_kind = 'history'` (defensive — history
    /// soft-deletes are blocked upstream by [`Self::soft_delete_memory`]).
    /// Both refusals leave the audit row in place so the user can see
    /// why the restore was declined.
    pub async fn restore_deletion(&self, deletion_id: i64) -> Result<RestoreOutcome> {
        let row = self
            .get_deletion(deletion_id)
            .await?
            .ok_or_else(|| anyhow!("deletion id {deletion_id} not found"))?;

        if row.deleted_by == DeletedBy::UserRedaction.as_str() {
            return Ok(RestoreOutcome::Refused {
                deletion_id,
                reason: "user_redaction is one-way; redactions cannot be restored".into(),
            });
        }
        if row.memory_kind == "history" {
            return Ok(RestoreOutcome::Refused {
                deletion_id,
                reason: "history rows are not restorable (append-only)".into(),
            });
        }

        let payload: serde_json::Value = serde_json::from_str(&row.original_row_json)
            .with_context(|| format!("parsing original_row_json for deletion {deletion_id}"))?;
        let scope = scope_from_payload(&payload)
            .with_context(|| format!("rebuilding WriteScope for deletion {deletion_id}"))?;
        let meta = meta_from_payload(&payload);
        let content = payload
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("audit row {deletion_id} missing 'content'"))?
            .to_string();

        let result = self.store_scoped(&scope, &content, &meta).await?;

        // Audit consumed; drop the row so /restore --since doesn't replay.
        {
            let conn = self.conn.lock().await;
            let _ = conn.execute(
                "DELETE FROM deletions WHERE id = ?1",
                rusqlite::params![deletion_id],
            );
        }

        Ok(match result {
            StoreResult::Inserted(id) => RestoreOutcome::Inserted {
                deletion_id,
                new_memory_id: id,
            },
            StoreResult::Deduplicated(id) => RestoreOutcome::Deduplicated {
                deletion_id,
                surviving_memory_id: id,
            },
            StoreResult::AlreadyCovered => RestoreOutcome::AlreadyCovered { deletion_id },
        })
    }

    /// C2.2: restore every still-pending deletion newer than the
    /// SQL relative-datetime offset (e.g. `"-2 hours"`, `"-7 days"`),
    /// processed oldest-first so dedup against earlier restored rows
    /// behaves predictably. `user_redaction` rows are skipped silently.
    pub async fn restore_deletions_since(
        &self,
        since_sql_offset: &str,
    ) -> Result<Vec<RestoreOutcome>> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock().await;
            let mut stmt = conn.prepare(
                "SELECT id FROM deletions
                  WHERE deleted_at >= datetime('now', ?1)
                    AND deleted_by != 'user_redaction'
                  ORDER BY id ASC",
            )?;
            stmt.query_map(rusqlite::params![since_sql_offset], |r| r.get::<_, i64>(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.restore_deletion(id).await?);
        }
        Ok(out)
    }

    /// C2.3: enumerate matches for a [`ForgetFilter`] without touching
    /// any row. Returns the candidate ids plus `kind` / `scope`
    /// breakdowns for the confirmation UI. Always excludes history —
    /// `/forget` cannot act on history rows (use `/forget-history`).
    pub async fn preview_forget(&self, filter: &ForgetFilter) -> Result<BulkForgetReport> {
        let (where_clause, params) = forget_predicate(filter);
        let conn = self.conn.lock().await;
        let sql = format!(
            "SELECT id, memory_kind, scope_path FROM memories
              WHERE memory_kind != 'history' AND {where_clause}
              ORDER BY id ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut report = BulkForgetReport {
            dry_run: true,
            ..Default::default()
        };
        let rows = stmt.query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (id, kind, scope_path) = row?;
            report.candidates.push(id);
            *report.kind_breakdown.entry(kind).or_insert(0) += 1;
            *report.scope_breakdown.entry(scope_path).or_insert(0) += 1;
        }
        Ok(report)
    }

    /// C2.3: execute a [`ForgetFilter`] as a bulk soft-delete. Each
    /// match goes through [`Self::soft_delete_memory`] so the
    /// `deletions` audit table sees one row per deleted memory and
    /// `/restore` works the same as a single-row delete.
    ///
    /// `dry_run = true` returns a populated [`BulkForgetReport`]
    /// without touching any row (preview the destruction first; the
    /// TUI uses this for the confirmation prompt).
    pub async fn bulk_forget(
        &self,
        filter: &ForgetFilter,
        dry_run: bool,
        reason: Option<&str>,
        deleted_by: DeletedBy,
    ) -> Result<BulkForgetReport> {
        // UserRedaction is reserved for C2.4 and is rejected per-id by
        // soft_delete_memory anyway, but we abort early so the caller
        // gets a clear error before any work.
        if matches!(deleted_by, DeletedBy::UserRedaction) {
            return Err(anyhow!(
                "DeletedBy::UserRedaction is reserved for the C2.4 RedactHistory path"
            ));
        }

        let mut report = self.preview_forget(filter).await?;
        report.dry_run = dry_run;
        if dry_run {
            return Ok(report);
        }
        for id in report.candidates.clone() {
            // Best-effort: if a particular row vanishes between the
            // preview and the delete (concurrent /forget), skip it but
            // continue. Hard errors abort the rest.
            match self.soft_delete_memory(id, deleted_by, reason, None).await {
                Ok(_) => report.deleted += 1,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("not found") {
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Ok(report)
    }

    /// C2.4: redact a history row in-place — replaces the transcript
    /// body with a tombstone marker and writes a one-way audit row
    /// tagged `DeletedBy::UserRedaction`. The row continues to exist
    /// (preserving session_id / turn_id / repo_id provenance for the
    /// derived records that reference it); only `content` and the
    /// compressed blob are wiped.
    ///
    /// **Trigger-disable callsite #2.** This is the second of exactly
    /// two callsites authorised to drop the C1.3 history-immutable
    /// triggers (the first is [`Self::compress_history_row`]). The
    /// drop / UPDATE / install sequence runs inside a single
    /// transaction with no `await` between drop and reinstall — the
    /// privileged window is microseconds long. The CI grep check on
    /// `drop_history_immutable_triggers` enforces that no other
    /// caller appears.
    ///
    /// The audit row's `original_row_json` carries the **post-redaction
    /// tombstone**, not the original transcript: redaction is
    /// intentionally one-way per the plan ("If a user wants reversible
    /// deletion, they shouldn't use `/forget-history`; they should use
    /// `/forget` on the derived records"). The tombstone embeds the
    /// SHA of the *original* content (computed before this call mutates
    /// the row) so the audit trail still proves a redaction happened.
    ///
    /// Returns the audit row's id on success.
    pub async fn redact_history_row(&self, memory_id: i64, reason: &str) -> Result<i64> {
        use sha2::{Digest, Sha256};

        // Read first; abort cleanly for non-history / missing rows.
        // We need the original *raw* content to compute its SHA for the
        // tombstone marker. For compressed rows we route through the
        // standard read_history_content path so SHA verify still runs.
        let original_content = self
            .read_history_content(memory_id)
            .await?
            .ok_or_else(|| anyhow!("history row {memory_id} not found"))?;

        let original_sha = format!("{:x}", Sha256::digest(original_content.as_bytes()));
        let timestamp = chrono::Utc::now().to_rfc3339();
        let safe_reason = reason.replace(['\n', ']'], " ");
        let tombstone = format!(
            "[REDACTED: sha={original_sha} redacted_at={timestamp} reason={safe_reason}]"
        );

        // Build the audit-row JSON now (outside the privileged window)
        // so the tx body stays minimal. The audit captures the
        // *post-redaction* row body — restore is intentionally
        // impossible. Omit fields that don't apply to the tombstone:
        // we synthesize a small object instead of the full RawMemoryRow
        // serialisation since the row no longer carries the original
        // body anyway.
        let audit_payload = serde_json::json!({
            "memory_id": memory_id,
            "memory_kind": "history",
            "tombstone": tombstone,
            "original_sha": original_sha,
            "redacted_at": timestamp,
            "reason": safe_reason,
        });
        let audit_json = serde_json::to_string(&audit_payload)
            .context("serializing redaction audit payload")?;

        // Pre-read the row's `source` and `trust_score` so the audit
        // table's `memory_source` / `memory_trust` columns are populated
        // consistently with the C2.1 soft-delete path.
        let (source, trust_score, content_hash): (String, f32, Option<String>) = {
            let conn = self.conn.lock().await;
            conn.query_row(
                "SELECT source, trust_score, content_hash FROM memories
                  WHERE id = ?1 AND memory_kind = 'history'",
                rusqlite::params![memory_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .with_context(|| format!("reading history row {memory_id} pre-redaction"))?
        };

        // Privileged write window: same shape as compress_history_row
        // — drop triggers, UPDATE, reinstall, commit. No await inside.
        let audit_id = {
            let mut conn = self.conn.lock().await;
            let tx = conn.transaction().context("redact: begin transaction")?;
            schema::drop_history_immutable_triggers(&tx)
                .context("redact: drop triggers")?;
            // Wipe content + blob; reset compressed flag. Update
            // content_hash to the tombstone's hash so the canonical
            // column matches the new body.
            let tombstone_hash = schema::content_hash(&tombstone);
            tx.execute(
                "UPDATE memories
                    SET content = ?1,
                        content_blob = NULL,
                        compressed = 0,
                        content_hash = ?2,
                        updated_at = datetime('now')
                  WHERE id = ?3 AND memory_kind = 'history'",
                rusqlite::params![tombstone, tombstone_hash, memory_id],
            )
            .context("redact: UPDATE history row")?;
            schema::install_history_immutable_triggers(&tx)
                .context("redact: reinstall triggers")?;
            // Audit row written inside the same transaction so the
            // tombstone + audit either both land or neither does.
            let audit_id: i64 = tx
                .query_row(
                    "INSERT INTO deletions (
                        memory_id, memory_content_hash, memory_kind, memory_source,
                        memory_trust, deleted_by, reason, original_row_json
                     ) VALUES (?1, ?2, 'history', ?3, ?4, ?5, ?6, ?7)
                     RETURNING id",
                    rusqlite::params![
                        memory_id,
                        content_hash,
                        source,
                        trust_score,
                        DeletedBy::UserRedaction.as_str(),
                        safe_reason,
                        audit_json,
                    ],
                    |r| r.get::<_, i64>(0),
                )
                .context("redact: INSERT audit row")?;
            tx.commit().context("redact: commit transaction")?;
            audit_id
        };

        Ok(audit_id)
    }
}

// ── Private helpers ────────────────────────────────────────────

/// C2.3: translate a [`ForgetFilter`] into a SQL `WHERE` fragment plus
/// its bound parameters. The fragment never matches history rows; the
/// caller's outer query keeps the `memory_kind != 'history'` guard so
/// future filter variants can't accidentally widen the scope.
fn forget_predicate(filter: &ForgetFilter) -> (String, Vec<rusqlite::types::Value>) {
    use rusqlite::types::Value;
    match filter {
        ForgetFilter::ByQuery(q) => (
            "lower(content) LIKE ?1".to_string(),
            vec![Value::from(format!("%{}%", q.to_lowercase()))],
        ),
        ForgetFilter::ByScope {
            scope_level,
            scope_path,
        } => (
            "scope_level = ?1 AND scope_path = ?2".to_string(),
            vec![
                Value::from(*scope_level as i64),
                Value::from(scope_path.clone()),
            ],
        ),
        ForgetFilter::ByType(t) => (
            "memory_type = ?1".to_string(),
            vec![Value::from(t.as_str().to_string())],
        ),
        ForgetFilter::BySource(s) => (
            "source = ?1".to_string(),
            vec![Value::from(s.as_str().to_string())],
        ),
    }
}

/// C2.2: rebuild a [`WriteScope`] from the audit row's
/// `original_row_json` payload. The serializer in
/// [`RawMemoryRow::to_json_value`] writes `scope_level`, `repo_id`,
/// `module_path`, `run_id`, and `scope_path` — we use level + the
/// matching string fields to pick the variant, falling back through
/// `scope_path` parsing for legacy rows that may have been saved with
/// nulls in the dedicated columns.
fn scope_from_payload(payload: &serde_json::Value) -> Result<WriteScope> {
    use crate::memory::scope::{
        SCOPE_GLOBAL, SCOPE_MODULE, SCOPE_REPO, SCOPE_RUN, SCOPE_WORKSPACE,
    };

    let level = payload
        .get("scope_level")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("audit payload missing scope_level"))? as i32;
    let str_field = |k: &str| -> Option<String> {
        payload
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    let repo_id = str_field("repo_id");
    let module_path = str_field("module_path");
    let run_id = str_field("run_id");

    match level {
        l if l == SCOPE_GLOBAL => Ok(WriteScope::Global),
        l if l == SCOPE_WORKSPACE => Ok(WriteScope::Workspace),
        l if l == SCOPE_REPO => repo_id
            .map(|repo_id| WriteScope::Repo { repo_id })
            .ok_or_else(|| anyhow!("Repo scope_level missing repo_id")),
        l if l == SCOPE_MODULE => match (repo_id, module_path) {
            (Some(repo_id), Some(module_path)) => Ok(WriteScope::Module {
                repo_id,
                module_path,
            }),
            _ => Err(anyhow!("Module scope_level missing repo_id/module_path")),
        },
        l if l == SCOPE_RUN => match (repo_id, run_id) {
            (Some(repo_id), Some(run_id)) => Ok(WriteScope::Run { repo_id, run_id }),
            _ => Err(anyhow!("Run scope_level missing repo_id/run_id")),
        },
        other => Err(anyhow!("unknown scope_level {other} in audit payload")),
    }
}

/// C2.2: rebuild a [`WriteMeta`] from the audit row's
/// `original_row_json` payload. Retains source_kind, trust_score,
/// memory_type, importance, tag, and lifecycle kind so the restored
/// row's policy reads the same as before deletion.
fn meta_from_payload(payload: &serde_json::Value) -> WriteMeta {
    use crate::memory::kind::MemoryKind;
    use crate::memory::scope::Trust;
    use crate::memory::trust_defaults::{MemorySource, clamp_trust};
    use std::str::FromStr;

    let str_field = |k: &str| -> Option<&str> { payload.get(k).and_then(|v| v.as_str()) };
    let f32_field = |k: &str| -> Option<f32> {
        payload.get(k).and_then(|v| v.as_f64()).map(|x| x as f32)
    };

    let memory_type = str_field("memory_type")
        .map(MemoryType::parse_str)
        .unwrap_or(MemoryType::Factual);
    let trust = str_field("trust").map(Trust::parse_str).unwrap_or(Trust::Medium);
    let source_kind = str_field("source")
        .map(MemorySource::parse_str)
        .unwrap_or(MemorySource::UnknownLegacy);
    let trust_score = f32_field("trust_score")
        .map(clamp_trust)
        .unwrap_or_else(|| source_kind.default_trust());
    let importance = f32_field("importance").map(clamp_trust).unwrap_or(0.5);
    let tag = str_field("tag").map(|s| s.to_string());
    let kind = str_field("memory_kind")
        .and_then(|s| MemoryKind::from_str(s).ok())
        .unwrap_or(MemoryKind::Record);
    let source = str_field("source")
        .map(|s| s.to_string())
        .unwrap_or_else(|| source_kind.as_str().to_string());

    WriteMeta {
        memory_type,
        importance,
        trust,
        source,
        tag,
        source_kind,
        trust_score,
        kind,
    }
}
