//! Session-consolidator policy: prepare, start, apply, abort, rollback.
//!
//! The writer task in [`super::writer`] remains the single consumer of SQLite
//! writes. This module owns the consolidator steps that module dispatches.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Result, anyhow, bail};
use serde_json::Value as JsonValue;
use serde_json::json;

use super::consolidation_llm::ConsolidationLlm;
use super::scope::{StoreResult, WriteMeta, WriteScope};
use super::store::MemoryStore;
use super::trust_defaults::MemorySource;
use super::writer::{InFlightRuns, Reenqueue, WriteResult, WriterMessage};

/// Everything a consolidation run needs before it can ask the model.
pub(crate) struct ConsolidationPrep {
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
pub(crate) async fn prepare_session_consolidate(
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
pub(crate) async fn start_session_consolidate(
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
    let batch_id = new_batch_id();

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
    let spawn_batch_id = batch_id.clone();
    tokio::spawn(async move {
        let finish = |in_flight: &InFlightRuns| {
            if let Ok(mut guard) = in_flight.lock() {
                guard.remove(&spawn_run_id);
            }
        };
        let abort = |kind: &str, error: String| {
            if let Err(e) = reenqueue.send(WriterMessage::SessionConsolidateAbort {
                run_id: spawn_run_id.clone(),
                batch_id: spawn_batch_id.clone(),
                kind: kind.to_string(),
                error,
            }) {
                tracing::warn!(
                    target: "memory_consolidator",
                    error = %e,
                    "consolidation failed but its abort could not be enqueued"
                );
                finish(&in_flight);
            }
        };
        let raw = match llm.complete(prompt).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(target: "memory_consolidator", error = %e, "LLM failure; deferring");
                abort("llm_error", format!("{e:#}"));
                return;
            }
        };
        let parsed = match parse_response(&raw) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(target: "memory_consolidator", error = %e, "parse failed; logging raw");
                abort("parse_error", format!("{e:#}"));
                return;
            }
        };
        // Ownership of the in-flight slot passes to the apply arm, which
        // releases it once the operations have landed.
        if let Err(e) = reenqueue.send(WriterMessage::SessionConsolidateApply {
            repo_id,
            module_path,
            run_id: spawn_run_id.clone(),
            batch_id: spawn_batch_id,
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
pub(crate) fn new_batch_id() -> String {
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
pub(crate) async fn apply_session_consolidate(
    store: &Arc<MemoryStore>,
    repo_id: String,
    module_path: Option<String>,
    run_id: &str,
    batch_id: &str,
    candidates: &[super::session_consolidator::CandidateBrief],
    parsed: super::session_consolidator::ConsolidatorResponse,
) -> Result<WriteResult> {
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

    let promotions = parsed.promotions;
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
        if candidates.get(op.candidate_index()).is_none() {
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
                        Ok(StoreResult::AlreadyCovered) => {
                            (None, Some("already covered; nothing to add".to_string()))
                        }
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

    for promo in promotions {
        let error = "session consolidation does not apply promotions; \
             use the deterministic promoter (run triage / cross-scope hits)"
            .to_string();
        let op_json = serde_json::to_string(&json!({
            "op": "PROMOTION",
            "memory_id": promo.memory_id,
            "to_scope": promo.to_scope,
        }))
        .unwrap_or_else(|_| "null".to_string());
        tracing::warn!(
            target: "memory_consolidator",
            memory_id = promo.memory_id,
            to_scope = %promo.to_scope,
            "rejecting promotion; session consolidator does not promote"
        );
        record_consolidation_op(
            store,
            ConsolidationAudit {
                run_id,
                batch_id,
                kind: "promotion",
                memory_id: Some(promo.memory_id),
                related_id: None,
                op_json: &op_json,
                before_json: None,
                after_json: None,
                error: Some(&error),
                scope: &scope_label,
                expected_outcome: None,
            },
        )
        .await;
    }
    Ok(WriteResult::Skipped)
}

/// Record a failed model call or unparseable response as one audit row.
///
/// The `batch_id` was minted before the spawn, so history can list the
/// invocation even though no operations landed. `kind` is `llm_error`
/// or `parse_error`. Releases nothing: the writer abort arm does that.
pub(crate) async fn abort_session_consolidate(
    store: &Arc<MemoryStore>,
    run_id: &str,
    batch_id: &str,
    kind: &str,
    error: &str,
) -> Result<WriteResult> {
    record_consolidation_op(
        store,
        ConsolidationAudit {
            run_id,
            batch_id,
            kind,
            memory_id: None,
            related_id: None,
            op_json: &json!({ "op": kind, "error": error }).to_string(),
            before_json: None,
            after_json: None,
            error: Some(error),
            scope: "repo",
            expected_outcome: None,
        },
    )
    .await;
    Ok(WriteResult::Skipped)
}

/// Trust score a `MERGE` promotes its surviving row to.
const MERGE_TRUST: f32 = 0.8;

/// Replay the last few consolidation runs for the prompt.
///
/// Best-effort: a consolidator that cannot see its history is worse at
/// self-correcting but still perfectly able to run, so a read failure
/// degrades to no history rather than aborting the pass.
pub(crate) async fn load_consolidation_history(
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
pub(crate) async fn process_consolidation_rollback(
    store: &Arc<MemoryStore>,
    batch_key: &str,
) -> Result<RollbackOutcome> {
    use super::session_consolidator::ConsolidationOp;

    let rows = store.consolidation_audit_for_batch(batch_key).await?;
    if rows.is_empty() {
        bail!("no consolidation run '{batch_key}' found in the audit trail");
    }

    let already_reversed = store.successful_rollback_targets(batch_key).await?;
    let originally_applied: Vec<i64> = rows.iter().filter(|r| r.applied).map(|r| r.id).collect();
    if !originally_applied.is_empty()
        && originally_applied
            .iter()
            .all(|id| already_reversed.contains(id))
    {
        bail!(
            "consolidation run '{batch_key}' has already been rolled back; the inverse \
             operations are not idempotent, so applying them twice would delete rows \
             the first rollback already restored"
        );
    }

    let mut outcome = RollbackOutcome {
        batch_id: batch_key.to_string(),
        reversed: 0,
        skipped: 0,
        failed: Vec::new(),
    };

    for row in rows.iter().rev() {
        if !row.applied || already_reversed.contains(&row.id) {
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
pub(crate) struct ConsolidationAudit<'a> {
    run_id: &'a str,
    /// Identifies this one invocation — see [`new_batch_id`].
    batch_id: &'a str,
    kind: &'a str,
    memory_id: Option<i64>,
    related_id: Option<i64>,
    op_json: &'a str,
    before_json: Option<&'a str>,
    /// Post-op snapshot for inspection. Rollback does not read this.
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
pub(crate) async fn record_consolidation_op(
    store: &Arc<MemoryStore>,
    audit: ConsolidationAudit<'_>,
) {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use anyhow::Result;
    use serde_json::json;

    use super::*;
    use crate::memory::consolidation_llm::ConsolidationLlm;
    use crate::memory::embedder::Embedder;
    use crate::memory::scope::{StoreResult, WriteMeta, WriteScope};
    use crate::memory::store::MemoryStore;
    use crate::memory::stores::MemoryStores;
    use crate::memory::trust_defaults::MemorySource;
    use crate::memory::writer::{WriteResult, WriterConfig, WriterHandle, spawn_writer_task};
    use serde_json::Value as JsonValue;

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
        let batch_id = new_batch_id();
        apply_session_consolidate(
            store,
            repo_id.to_string(),
            module_path,
            run_id,
            &batch_id,
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

    #[tokio::test]
    async fn merge_out_of_range_candidate_index_does_not_bump_trust() {
        let run_id = "run-merge-oor";
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
                    { "op": "MERGE", "candidate_index": 99, "into_memory_id": target_id }
                ]
            }),
        )
        .await;

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let merge = rows.iter().find(|r| r.kind == "merge").expect("merge row");
        assert!(
            !merge.applied,
            "out-of-range MERGE must not apply: {merge:?}"
        );
        assert!(
            merge
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("out of range"),
            "{:?}",
            merge.error
        );
        assert_eq!(
            store
                .get_memory_row(target_id)
                .await
                .unwrap()
                .unwrap()
                .trust_score,
            original_trust,
            "rejected MERGE must not bump trust"
        );
    }

    #[tokio::test]
    async fn add_already_covered_is_not_applied_and_is_not_reversed() {
        let run_id = "run-add-covered";
        let store = store_with_candidates(run_id, 1).await;
        store
            .store_scoped(
                &WriteScope::Workspace,
                "candidate number 0",
                &WriteMeta::for_source(MemorySource::LlmConsolidated),
            )
            .await
            .unwrap();
        let before = live_ids(&store).await;

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [{ "op": "ADD", "candidate_index": 0 }]
            }),
        )
        .await;

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let add = rows.iter().find(|r| r.kind == "add").expect("add row");
        assert!(!add.applied, "AlreadyCovered ADD must not apply: {add:?}");
        assert!(
            add.error
                .as_deref()
                .unwrap_or_default()
                .contains("already covered"),
            "{:?}",
            add.error
        );

        let outcome = process_consolidation_rollback(&store, &latest_batch_id(&store).await)
            .await
            .unwrap();
        assert_eq!(outcome.reversed, 0, "nothing applied to reverse");
        assert_eq!(live_ids(&store).await, before);
    }

    #[tokio::test]
    async fn promotions_in_the_response_are_audited_and_not_applied() {
        let run_id = "run-promo-reject";
        let store = store_with_candidates(run_id, 1).await;
        let before = live_ids(&store).await;

        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [],
                "promotions": [
                    { "memory_id": 8, "to_scope": "repo" },
                    { "memory_id": 9, "to_scope": "global" }
                ]
            }),
        )
        .await;

        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let promos: Vec<_> = rows.iter().filter(|r| r.kind == "promotion").collect();
        assert_eq!(promos.len(), 2, "one audit row per promotion: {rows:?}");
        assert!(promos.iter().all(|r| !r.applied));
        assert_eq!(
            live_ids(&store).await,
            before,
            "promotions must not write memories"
        );
    }

    #[tokio::test]
    async fn every_row_in_a_batch_shares_the_minted_batch_id() {
        let run_id = "run-same-batch";
        let store = store_with_candidates(run_id, 1).await;
        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "a summary",
                "operations": [{ "op": "ADD", "candidate_index": 0 }]
            }),
        )
        .await;
        let rows = store.consolidation_audit_for_run(run_id).await.unwrap();
        let history = latest_batch_id(&store).await;
        assert!(rows.len() >= 2);
        let batch_rows = store.consolidation_audit_for_batch(&history).await.unwrap();
        assert_eq!(batch_rows.len(), rows.len());
    }

    #[tokio::test]
    async fn partial_rollback_retries_the_remaining_inverses() {
        let run_id = "run-partial-retry";
        let store = store_with_candidates(run_id, 2).await;
        consolidate(
            &store,
            run_id,
            json!({
                "session_summary": "",
                "operations": [
                    { "op": "ADD", "candidate_index": 0 },
                    { "op": "ADD", "candidate_index": 1 },
                ]
            }),
        )
        .await;

        let batch = latest_batch_id(&store).await;
        let applied: Vec<_> = store
            .consolidation_audit_for_batch(&batch)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.applied)
            .collect();
        assert!(applied.len() >= 2, "need two applied ops to split");

        store
            .log_consolidation_rollback(crate::memory::store::consolidation_ops::RollbackAuditRow {
                run_id,
                batch_id: &batch,
                kind: &applied[0].kind,
                memory_id: applied[0].memory_id,
                related_id: applied[0].related_id,
                op_json: &json!({
                    "reverses_audit_id": applied[0].id,
                    "op": applied[0].kind,
                })
                .to_string(),
                applied: true,
                error: None,
            })
            .await
            .unwrap();

        let remaining = applied.len() - 1;
        let outcome = process_consolidation_rollback(&store, &batch)
            .await
            .expect("partial undo must retry the rest");
        assert_eq!(outcome.reversed, remaining);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        let err = process_consolidation_rollback(&store, &batch)
            .await
            .expect_err("a complete undo must then latch");
        assert!(
            format!("{err:#}").contains("already been rolled back"),
            "{err:#}"
        );
    }

    #[tokio::test]
    async fn abort_only_batch_rollback_skips_and_does_not_latch() {
        let store = store_with_candidates("run-abort-rb", 1).await;
        let batch = new_batch_id();
        abort_session_consolidate(&store, "run-abort-rb", &batch, "llm_error", "boom")
            .await
            .unwrap();

        let outcome = process_consolidation_rollback(&store, &batch)
            .await
            .expect("nothing applied is a no-op, not a latch");
        assert_eq!(outcome.reversed, 0);
        assert!(outcome.skipped >= 1);

        process_consolidation_rollback(&store, &batch)
            .await
            .expect("a second rollback of an abort-only batch must not latch");
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

    struct FailingLlm;

    #[async_trait::async_trait]
    impl ConsolidationLlm for FailingLlm {
        async fn complete(&self, _prompt: String) -> Result<String> {
            anyhow::bail!("stub llm exploded")
        }
    }

    #[tokio::test]
    async fn an_llm_error_writes_an_abort_audit_row() {
        let run_id = "run-llm-err";
        let store = store_with_candidates(run_id, 1).await;
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: Some(Arc::new(FailingLlm) as Arc<dyn ConsolidationLlm>),
            observer: None,
            manifest_observer: None,
        });

        let res = handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();
        assert!(matches!(res, WriteResult::Queued));

        let rows = await_audit_rows(&store, run_id).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, "llm_error");
        assert!(!rows[0].applied);
        assert!(
            rows[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("stub llm exploded"),
            "{:?}",
            rows[0].error
        );

        let listed = store
            .recent_consolidation_runs(10)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.run_id == run_id)
            .expect("abort batch listed in history");
        assert_eq!(
            store
                .consolidation_audit_for_batch(&listed.batch_id)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn unparseable_llm_text_writes_a_parse_error_audit_row() {
        let run_id = "run-parse-err";
        let store = store_with_candidates(run_id, 1).await;
        let handle = spawn_writer_task(WriterConfig {
            stores: MemoryStores::from_single_store(store.clone()),
            llm: Some(Arc::new(ScriptedLlm("totally not JSON".into())) as Arc<dyn ConsolidationLlm>),
            observer: None,
            manifest_observer: None,
        });

        handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();

        let rows = await_audit_rows(&store, run_id).await;
        assert_eq!(rows[0].kind, "parse_error");
        assert!(!rows[0].applied);
    }

    #[tokio::test]
    async fn a_queued_run_lands_every_op_under_the_start_batch_id() {
        let run_id = "run-start-batch";
        let store = store_with_candidates(run_id, 1).await;
        let (handle, _) = writer_with_slow_llm(store.clone(), &one_add_response()).await;
        handle
            .session_consolidate("sess", "r1", None, run_id, "transcript")
            .await
            .unwrap();
        let rows = await_audit_rows(&store, run_id).await;
        let history = latest_batch_id(&store).await;
        let batch_rows = store.consolidation_audit_for_batch(&history).await.unwrap();
        assert_eq!(batch_rows.len(), rows.len());
        assert!(batch_rows.iter().all(|r| r.run_id == run_id));
    }
}
