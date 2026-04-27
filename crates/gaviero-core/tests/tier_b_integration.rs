//! Tier B whole-feature integration tests.
//!
//! These exercise the public API end-to-end — writer task, store,
//! sleeptime, telemetry, eval — wherever the per-module unit tests
//! cover only fragments. Each test names the plan acceptance criterion
//! it verifies and the gap in unit coverage it closes.
//!
//! Conventions:
//! - All tests use `MemoryStore::in_memory` with the deterministic
//!   `TestEmbedder` below; no model files, no network.
//! - The writer task is spawned and shut down via the same code path
//!   the TUI / CLI use, so writer-side regressions show up here.
//! - Cosines from `TestEmbedder` are deterministic but coarse — tests
//!   that compare ranks pick texts whose rank ordering is stable under
//!   the byte-bucketed embedding (avoid near-equal texts where small
//!   floating-point reordering would flake).

#![allow(clippy::field_reassign_with_default)]

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use gaviero_core::memory::eval::{EvalCase, run_from_manifests};
use gaviero_core::memory::sleeptime::{SleeptimeConfig, SleeptimeOperation, run_sleeptime};
use gaviero_core::memory::telemetry::{ClassifyConfig, classify_turn};
use gaviero_core::memory::{
    Embedder, MemorySource, MemoryStore, MemoryStores, MemoryType, ScopeFilter, SearchConfig,
    SleeptimeReport, StoreResult, WriteMeta, WriteResult, WriteScope, WriterConfig, WriterMessage,
    hash_path, spawn_writer_task,
};

/// Deterministic 8-dim embedder. Same texts → same vectors → fixed
/// cosine. Used everywhere so tests don't depend on ONNX or network.
struct TestEmbedder;

#[async_trait::async_trait]
impl Embedder for TestEmbedder {
    fn name(&self) -> &str {
        "test-embedder-v1"
    }

    fn dimension(&self) -> usize {
        8
    }

    async fn embed(
        &self,
        text: &str,
        _purpose: gaviero_core::memory::EmbeddingPurpose,
    ) -> Result<Vec<f32>> {
        let mut vec = vec![0.0f32; 8];
        for (i, byte) in text.bytes().enumerate() {
            vec[i % 8] += byte as f32;
        }
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        Ok(vec)
    }
}

fn store() -> Arc<MemoryStore> {
    Arc::new(MemoryStore::in_memory(Arc::new(TestEmbedder)).expect("open in-memory store"))
}

/// Insert a memory and return its row id, panicking on dedup-only or
/// already-covered outcomes (those would mean the test fixture text is
/// not distinctive enough).
async fn insert(
    store: &Arc<MemoryStore>,
    scope: &WriteScope,
    content: &str,
    meta: WriteMeta,
) -> i64 {
    match store
        .store_scoped(scope, content, &meta)
        .await
        .expect("insert")
    {
        StoreResult::Inserted(id) | StoreResult::Deduplicated(id) => id,
        StoreResult::AlreadyCovered => panic!("test fixture covered at broader scope: {content:?}"),
    }
}

/// Drain the writer task by polling queue depth. 200ms cap is generous
/// for in-memory tests; failures here imply a wedged writer.
async fn drain(handle: &gaviero_core::memory::WriterHandle) {
    for _ in 0..40 {
        if handle.queue_depth() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "writer task did not drain within 200ms — depth still {}",
        handle.queue_depth()
    );
}

// ─────────────────────────────────────────────────────────────────────
// T1. B6 + B5 loop end-to-end:
//     writer-persisted manifest → classify_turn → retrieval_use rows
//     → memory_utilization aggregate → run_sleeptime trust adjustment
//     → next-retrieval-time scoring uses the new trust.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b6_telemetry_to_b5_trust_rescore_full_loop() {
    let store = store();
    let writer = spawn_writer_task(WriterConfig {
        stores: MemoryStores::from_single_store(store.clone()),
        llm: None,
        observer: None,
        manifest_observer: None,
    });

    // Plant an LLM-authored memory at trust 0.6 (default for LlmExtracted).
    let scope = WriteScope::Repo {
        repo_id: "repo-alpha".into(),
    };
    let mem_id = insert(
        &store,
        &scope,
        "the canonical guidance about widget retries",
        WriteMeta::for_source(MemorySource::LlmExtracted).with_trust_score(0.6),
    )
    .await;

    // Persist a manifest via the writer task so the telemetry pass
    // sees exactly what production would write.
    let payload = serde_json::json!({
        "schema_version": 1,
        "scoring_formula_version": "v1-composite",
        "embedder_name": "test-embedder-v1",
        "query_text": "widget retries",
        "selected_ids": [mem_id],
        "token_budget_used": 64,
        "token_budget_limit": 1000,
    });
    writer
        .enqueue(WriterMessage::InjectionManifest {
            turn_id: "turn-1".into(),
            session_id: "conv-1".into(),
            payload,
        })
        .unwrap();
    drain(&writer).await;

    // B6 classify: response strongly mirrors the memory content so
    // cosine + substring both agree on Used.
    let cfg = ClassifyConfig::default();
    let report = classify_turn(
        &store,
        "turn-1",
        Some("conv-1"),
        "yes — the canonical guidance about widget retries applies here",
        &cfg,
    )
    .await
    .expect("classify_turn");
    assert_eq!(
        report.items.len(),
        1,
        "one injected memory, one classified item"
    );
    assert_eq!(
        report.items[0].class,
        gaviero_core::memory::UseClass::Used,
        "matching response must be classified Used; report={report:?}"
    );

    // Repeat 5 more times so we cross the trust_min_injections=5
    // threshold the sleeptime pass requires.
    for n in 2..=6 {
        let payload = serde_json::json!({
            "schema_version": 1,
            "query_text": format!("widget retries v{n}"),
            "selected_ids": [mem_id],
            "token_budget_used": 64,
            "token_budget_limit": 1000,
        });
        let turn_id = format!("turn-{n}");
        writer
            .enqueue(WriterMessage::InjectionManifest {
                turn_id: turn_id.clone(),
                session_id: "conv-1".into(),
                payload,
            })
            .unwrap();
        drain(&writer).await;
        classify_turn(
            &store,
            &turn_id,
            Some("conv-1"),
            "the canonical guidance about widget retries applies here too",
            &cfg,
        )
        .await
        .unwrap();
    }

    // memory_utilization should now report 6 injections, all Used.
    let utils = store.memory_utilization(&[mem_id]).await.unwrap();
    assert_eq!(utils.len(), 1);
    assert_eq!(utils[0].times_injected, 6, "{utils:?}");
    assert!(
        utils[0].utilization_rate > 0.99,
        "all 6 should be Used, got rate={}",
        utils[0].utilization_rate
    );

    // Run sleeptime with telemetry-driven trust rescore. Default
    // thresholds: ≥5 injections, used > 0.6 → +0.05 trust.
    let mut sleep_cfg = SleeptimeConfig::default();
    sleep_cfg.dry_run = false;
    let report: SleeptimeReport = run_sleeptime(&store, &sleep_cfg, None).await.unwrap();
    assert!(
        report.trust_adjusted >= 1,
        "expected ≥1 trust adjustment, got {report:?}"
    );

    // Verify the actual stored trust_score moved up.
    let hits = store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: "repo-alpha".into(),
            },
            "",
            10,
        )
        .await
        .unwrap();
    let row = hits
        .iter()
        .find(|m| m.id == mem_id)
        .expect("memory still present after sleeptime");
    assert!(
        (row.trust_score - 0.65).abs() < 1e-3,
        "expected trust_score 0.6+0.05=0.65, got {}",
        row.trust_score
    );
}

// ─────────────────────────────────────────────────────────────────────
// T2. SUPERSEDE causes the new row to outrank — and the old row to
//     disappear from — scoped retrieval. Goes beyond the unit-level
//     "marker landed" test (which only checks `superseded_by` SQL).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn supersede_flow_excludes_old_and_returns_only_successor() {
    let store = store();
    let scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let old_id = insert(
        &store,
        &scope,
        "OLD: avoid mutex; use channels everywhere always",
        WriteMeta::default(),
    )
    .await;
    let new_id = insert(
        &store,
        &scope,
        "NEW: use mutex when contention is bounded; channels when fanout",
        WriteMeta::default(),
    )
    .await;
    assert_eq!(store.supersede_memory(old_id, new_id).await.unwrap(), 1);

    let memory_scope = gaviero_core::memory::MemoryScope {
        global_db: std::path::PathBuf::new(),
        workspace_db: std::path::PathBuf::new(),
        repo_db: None,
        workspace_id: "ws".into(),
        repo_id: Some("r1".into()),
        module_path: None,
        run_id: None,
    };
    let cfg = SearchConfig::new("mutex channels", memory_scope);
    let results = store.search_scoped(&cfg).await.unwrap();
    let ids: Vec<i64> = results.iter().map(|m| m.id).collect();
    assert!(
        !ids.contains(&old_id),
        "superseded row {old_id} surfaced: {ids:?}"
    );
    assert!(
        ids.contains(&new_id),
        "successor row {new_id} missing: {ids:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// T3. Sleeptime non-dry-run actually deletes the loser row, audit
//     row lands, and a re-run is idempotent (no further merges).
//     The unit-level test covered dry_run=true only.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sleeptime_near_dup_merge_persists_and_is_idempotent() {
    let store = store();
    let scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let id_a = insert(
        &store,
        &scope,
        "tokio is the runtime choice",
        WriteMeta::for_source(MemorySource::LlmExtracted).with_trust_score(0.6),
    )
    .await;
    let id_b = insert(
        &store,
        &scope,
        "tokio runtime is the choice we picked",
        WriteMeta::for_source(MemorySource::LlmExtracted).with_trust_score(0.7),
    )
    .await;

    let mut cfg = SleeptimeConfig::default();
    cfg.dry_run = false;
    // Deterministic mock embedder produces high cosine on similar
    // byte buckets; threshold 0.0 covers the test fixture without
    // gambling on exact float values.
    cfg.near_dup_threshold = 0.0;
    let r1 = run_sleeptime(&store, &cfg, None).await.unwrap();
    assert_eq!(
        r1.near_dup_merged, 1,
        "first run should merge exactly one near-dup pair: {r1:?}"
    );

    // Exactly one of {id_a, id_b} should still exist after the merge.
    let all = store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: "r1".into(),
            },
            "",
            50,
        )
        .await
        .unwrap();
    let alive: Vec<i64> = [id_a, id_b]
        .iter()
        .copied()
        .filter(|id| all.iter().any(|m| m.id == *id))
        .collect();
    assert_eq!(
        alive.len(),
        1,
        "exactly one of the pair should survive; got {alive:?}"
    );
    // pick_merge_winner ties trust → the higher trust wins; b had 0.7.
    assert_eq!(alive[0], id_b, "higher-trust row should be the survivor");

    // Audit row landed (uses #[doc(hidden)] test helper).
    let audit_count = store.count_audit_for_test("near_dup_merged").await.unwrap();
    assert!(audit_count >= 1, "expected near_dup_merged audit row");

    // Idempotent: a second sleep produces zero further merges.
    let r2 = run_sleeptime(&store, &cfg, None).await.unwrap();
    assert_eq!(
        r2.near_dup_merged, 0,
        "second run must be a no-op for near-dup; got {r2:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// T4. SUPERSEDE excludes a row from sleeptime near-dup merge candidate
//     pool. Critical interaction between B5 step 2 and the SUPERSEDE
//     soft-delete contract.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn superseded_row_is_not_a_merge_candidate() {
    let store = store();
    let scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let a = insert(
        &store,
        &scope,
        "tokio is the runtime",
        WriteMeta::for_source(MemorySource::LlmExtracted),
    )
    .await;
    let b = insert(
        &store,
        &scope,
        "tokio runtime",
        WriteMeta::for_source(MemorySource::LlmExtracted),
    )
    .await;
    // Mark A as superseded by some other row id (use a synthetic id —
    // we only care that the column flips, not what it points at).
    store.supersede_memory(a, b).await.unwrap();

    let mut cfg = SleeptimeConfig::default();
    cfg.dry_run = false;
    cfg.near_dup_threshold = 0.0;
    let report = run_sleeptime(&store, &cfg, None).await.unwrap();
    // No pair to merge — only B remains in the eligible set.
    assert_eq!(
        report.near_dup_merged, 0,
        "superseded row must not be a merge candidate: {report:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// T5. T0 `--from-manifests` end-to-end:
//     writer-persisted manifest → run_from_manifests → ranked outcome
//     reflects the manifest's blended_score (or composite_score)
//     ordering.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn eval_from_manifests_replays_persisted_pool() {
    let store = store();
    let writer = spawn_writer_task(WriterConfig {
        stores: MemoryStores::from_single_store(store.clone()),
        llm: None,
        observer: None,
        manifest_observer: None,
    });

    // Persist one manifest with a 3-entry candidate_pool. Note the
    // ordering: distractor (mem 100) at composite=0.50, expected
    // (mem 42) at composite=0.90, another distractor (mem 7) at 0.20.
    // run_from_manifests must rank by score, putting expected at #1.
    let payload = serde_json::json!({
        "schema_version": 1,
        "query_text": "how do worktrees clean up",
        "selected_ids": [42],
        "candidate_pool": [
            {"memory_id": 100, "scope_label": "repo", "namespace": "n",
             "raw_similarity": 0.5, "composite_score": 0.50, "selected": false},
            {"memory_id": 42,  "scope_label": "repo", "namespace": "n",
             "raw_similarity": 0.9, "composite_score": 0.90, "selected": true},
            {"memory_id": 7,   "scope_label": "repo", "namespace": "n",
             "raw_similarity": 0.2, "composite_score": 0.20, "selected": false},
        ],
        "embedder_name": "test-embedder-v1",
    });
    writer
        .enqueue(WriterMessage::InjectionManifest {
            turn_id: "t-eval".into(),
            session_id: "conv-eval".into(),
            payload,
        })
        .unwrap();
    drain(&writer).await;

    let cases = vec![
        EvalCase {
            id: "c1".into(),
            query: "how do worktrees clean up".into(),
            expected_memory_id: 42,
            scope: "repo".into(),
            tags: vec!["worktree".into()],
        },
        EvalCase {
            id: "c2-no-manifest".into(),
            query: "totally different query never injected".into(),
            expected_memory_id: 999,
            scope: "repo".into(),
            tags: vec!["miss".into()],
        },
    ];
    let report = run_from_manifests(&store, &cases, 10).await.unwrap();
    assert_eq!(report.total, 2);
    // c1: expected at composite=0.90, top of pool → rank=1.
    let c1 = report.outcomes.iter().find(|o| o.id == "c1").unwrap();
    assert_eq!(c1.rank, Some(1), "rescore should put expected at rank 1");
    // c2: query never appeared → rank=None, contributes a miss.
    let c2 = report
        .outcomes
        .iter()
        .find(|o| o.id == "c2-no-manifest")
        .unwrap();
    assert_eq!(c2.rank, None);
    assert!(report.recall_at_5 > 0.0 && report.recall_at_5 < 1.0);
}

// ─────────────────────────────────────────────────────────────────────
// T6. B5 trust adjustment respects the floor: many "unused"
//     classifications drag trust down by 0.05 per pass, but it never
//     drops below `cfg.trust_floor` (default 0.2).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn trust_rescore_floors_at_configured_minimum() {
    let store = store();
    let scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let id = insert(
        &store,
        &scope,
        "unused observation about flubbering",
        WriteMeta::for_source(MemorySource::LlmExtracted).with_trust_score(0.30),
    )
    .await;
    // 6 unused classifications crosses the 5-injection floor.
    for rank in 1..=6 {
        store
            .record_retrieval_use(id, &format!("t-{rank}"), None, rank, "unused", 0.10, false)
            .await
            .unwrap();
    }
    let mut cfg = SleeptimeConfig::default();
    cfg.dry_run = false;
    // Run repeatedly until adjustment stops.
    let mut last_trust = 0.30_f32;
    for _ in 0..10 {
        run_sleeptime(&store, &cfg, None).await.unwrap();
        let row = store
            .search_at_level(
                &ScopeFilter::Repo {
                    repo_id: "r1".into(),
                },
                "",
                10,
            )
            .await
            .unwrap();
        let cur = row.iter().find(|m| m.id == id).unwrap().trust_score;
        if (cur - last_trust).abs() < 1e-6 {
            // Converged.
            break;
        }
        last_trust = cur;
    }
    assert!(
        (last_trust - cfg.trust_floor).abs() < 1e-3,
        "trust should floor at {}, got {}",
        cfg.trust_floor,
        last_trust
    );
}

// ─────────────────────────────────────────────────────────────────────
// T7. B4 acceptance: a 180-day-old `decision`-type memory is **not**
//     decay-flagged by sleeptime (exempt type), while a same-aged
//     `factual`-type memory IS flagged. Closes the gap from the
//     formula-only unit tests in scoring.rs.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn b4_decay_sweep_exempts_decision_type() {
    let store = store();
    let scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let decision = insert(
        &store,
        &scope,
        "we use git2 not the git CLI",
        WriteMeta::for_source(MemorySource::LlmExtracted)
            .with_type(MemoryType::Decision)
            .with_trust_score(0.6),
    )
    .await;
    let factual = insert(
        &store,
        &scope,
        "the build script copies tree-sitter grammars on first run",
        WriteMeta::for_source(MemorySource::LlmExtracted)
            .with_type(MemoryType::Factual)
            .with_trust_score(0.6),
    )
    .await;
    // Backdate both to 200 days ago via the test helper.
    store.force_age_for_test(decision, 200).await.unwrap();
    store.force_age_for_test(factual, 200).await.unwrap();
    let cfg = SleeptimeConfig::default();
    let ops = store.sleeptime_decay_sweep(cfg.dry_run).await.unwrap();
    let flagged_ids: Vec<i64> = ops
        .iter()
        .filter_map(|op| match op {
            SleeptimeOperation::DecayFlagged { memory_id, .. } => Some(*memory_id),
            _ => None,
        })
        .collect();
    assert!(
        !flagged_ids.contains(&decision),
        "decision-type was flagged but should be exempt: {flagged_ids:?}"
    );
    assert!(
        flagged_ids.contains(&factual),
        "factual-type at 200 days should hit the floor and be flagged: {flagged_ids:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// T8. Multi-scope dedup keeps the narrowest scope when the same
//     content_hash appears at both Repo and Module. This is the
//     dedup invariant the merged-multi-scope retrieval relies on.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn multi_scope_dedup_keeps_narrower_scope() {
    let store = store();
    let repo_scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let module_scope = WriteScope::Module {
        repo_id: "r1".into(),
        module_path: "crates/core".into(),
    };
    // Insert order matters: write-path coverage dedup drops a write
    // at narrower scope when an ancestor already has the same
    // content_hash. So we plant Module first, then Repo (Repo's
    // ancestors are Workspace + Global, neither holds the row, so
    // both rows survive in storage).
    insert(&store, &module_scope, "use git2 only", WriteMeta::default()).await;
    insert(&store, &repo_scope, "use git2 only", WriteMeta::default()).await;

    let memory_scope = gaviero_core::memory::MemoryScope {
        global_db: std::path::PathBuf::new(),
        workspace_db: std::path::PathBuf::new(),
        repo_db: None,
        workspace_id: "ws".into(),
        repo_id: Some("r1".into()),
        module_path: Some("crates/core".into()),
        run_id: None,
    };
    let cfg = SearchConfig::new("git2", memory_scope);
    let results = store.search_scoped(&cfg).await.unwrap();
    let same_text: Vec<&gaviero_core::memory::ScoredMemory> = results
        .iter()
        .filter(|m| m.content == "use git2 only")
        .collect();
    assert_eq!(
        same_text.len(),
        1,
        "cross-scope content_hash dedup must collapse duplicates: {results:?}"
    );
    // Narrower scope wins by scope_multiplier (Module > Repo).
    let kept = same_text[0];
    assert_eq!(
        kept.scope_level,
        gaviero_core::memory::scope::SCOPE_MODULE,
        "narrower (Module) scope should win the dedup tie: kept={kept:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// T9. H1 acceptance through the full open-stamp-detect cycle: an
//     in-memory store with our test embedder must immediately report
//     no mismatch (because init stamps the meta row), even on a
//     completely fresh DB.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn embedder_meta_stamp_round_trip() {
    let store = store();
    let stamped = store.get_meta_value("embedder_model").await.unwrap();
    assert_eq!(
        stamped.as_deref(),
        Some("test-embedder-v1"),
        "fresh DB must stamp configured embedder id"
    );
    assert!(
        store.detect_embedder_mismatch().await.is_none(),
        "configured embedder matches stamp → no mismatch"
    );
}

// ─────────────────────────────────────────────────────────────────────
// T10. Writer task continues to drain after a malformed
//     InjectionManifest payload. Validates the "manifest write
//     failure never fails the turn" comment in writer.rs:635-637 and
//     proves the writer is not poisoned by a single bad message.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn writer_task_survives_malformed_manifest() {
    let store = store();
    let writer = spawn_writer_task(WriterConfig {
        stores: MemoryStores::from_single_store(store.clone()),
        llm: None,
        observer: None,
        manifest_observer: None,
    });

    // First, a bad payload (intentionally absurd shape — but valid
    // JSON, so it persists. The writer never errors here in practice;
    // this test guards against future schema-validation changes that
    // would make the writer panic instead of degrade.)
    writer
        .enqueue(WriterMessage::InjectionManifest {
            turn_id: "bad".into(),
            session_id: "conv".into(),
            payload: serde_json::json!({"completely": "unstructured", "n": 12345}),
        })
        .unwrap();
    drain(&writer).await;

    // Then a valid one. Both must persist; the writer is single-
    // consumer, so a poisoned writer would silently drop the second.
    writer
        .enqueue(WriterMessage::InjectionManifest {
            turn_id: "good".into(),
            session_id: "conv".into(),
            payload: serde_json::json!({"query_text": "ok", "selected_ids": [1]}),
        })
        .unwrap();
    drain(&writer).await;

    let rows = store.recent_manifests(10).await.unwrap();
    assert_eq!(rows.len(), 2, "both manifests must persist; rows={rows:?}");
    let turn_ids: Vec<&str> = rows.iter().map(|r| r.turn_id.as_str()).collect();
    assert!(turn_ids.contains(&"good"));
    assert!(turn_ids.contains(&"bad"));
}

// ─────────────────────────────────────────────────────────────────────
// T11. A `UserRemember`-sourced row's `trust_score` (1.0) is
//     unaffected by sleeptime even when its utilization is high
//     enough to bump an LLM row. Strengthens the existing unit-level
//     check by going through `run_sleeptime` (not just the inner
//     rescore method).
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_sleeptime_never_modifies_user_remember_trust() {
    let store = store();
    let scope = WriteScope::Repo {
        repo_id: "r1".into(),
    };
    let user_id = insert(
        &store,
        &scope,
        "user pinned this",
        WriteMeta::for_source(MemorySource::UserRemember).with_trust_score(1.0),
    )
    .await;
    for rank in 1..=8 {
        store
            .record_retrieval_use(user_id, &format!("t-{rank}"), None, rank, "used", 0.9, true)
            .await
            .unwrap();
    }
    let mut cfg = SleeptimeConfig::default();
    cfg.dry_run = false;
    run_sleeptime(&store, &cfg, None).await.unwrap();
    let row = store
        .search_at_level(
            &ScopeFilter::Repo {
                repo_id: "r1".into(),
            },
            "",
            10,
        )
        .await
        .unwrap();
    let user = row.iter().find(|m| m.id == user_id).unwrap();
    assert!(
        (user.trust_score - 1.0).abs() < 1e-6,
        "user_remember trust must stay at 1.0; got {}",
        user.trust_score
    );
}

// ─────────────────────────────────────────────────────────────────────
// T12. H3 stress: with 3 concurrent runs interleaved in time,
//     `recent_memories_for_run` must return only the requested run's
//     rows. Goes beyond the 2-run unit check.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn recent_memories_for_run_under_interleaved_load() {
    let store = store();
    for run_n in 0..3 {
        let scope = WriteScope::Run {
            repo_id: "r1".into(),
            run_id: format!("run-{run_n}"),
        };
        for i in 0..5 {
            insert(
                &store,
                &scope,
                &format!("run {run_n} memory item {i}"),
                WriteMeta::default(),
            )
            .await;
        }
    }
    for run_n in 0..3 {
        let only = store
            .recent_memories_for_run(&format!("run-{run_n}"), 24, 50)
            .await
            .unwrap();
        assert_eq!(only.len(), 5, "run-{run_n}: expected 5, got {}", only.len());
        for m in &only {
            assert!(
                m.content.starts_with(&format!("run {run_n} ")),
                "session-{run_n} got cross-run leakage: {:?}",
                m.content
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// T13. UserRemember through writer task: scope is preserved (A2)
//     and trust 1.0 is honored end-to-end (not just at meta build
//     time). Acts as a smoke test that the writer's enqueue/ack
//     contract is intact for the most user-facing path.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn user_remember_via_writer_lands_at_correct_scope_with_full_trust() {
    let store = store();
    let writer = spawn_writer_task(WriterConfig {
        stores: MemoryStores::from_single_store(store.clone()),
        llm: None,
        observer: None,
        manifest_observer: None,
    });
    let scope = WriteScope::Module {
        repo_id: hash_path(std::path::Path::new("/test")),
        module_path: "crates/core".into(),
    };
    let res: WriteResult = writer
        .user_remember_scoped(scope.clone(), "explicit user fact")
        .await
        .expect("ack");
    let id = match res {
        WriteResult::Inserted(id) | WriteResult::Deduplicated(id) => id,
        other => panic!("unexpected result: {other:?}"),
    };
    let hits = store
        .search_at_level(
            &ScopeFilter::Module {
                repo_id: hash_path(std::path::Path::new("/test")),
                module_path: "crates/core".into(),
            },
            "",
            10,
        )
        .await
        .unwrap();
    let row = hits
        .iter()
        .find(|m| m.id == id)
        .expect("row at module scope");
    assert_eq!(
        row.source,
        MemorySource::UserRemember,
        "source must remain UserRemember through writer pipeline"
    );
    assert!(
        (row.trust_score - 1.0).abs() < 1e-6,
        "user_remember trust must be 1.0; got {}",
        row.trust_score
    );
}
