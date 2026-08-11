//! End-to-end coverage for the `memory_flag` write path (D1).
//!
//! Enters through [`WriterSignalSink`] — the exact object the MCP server
//! holds — so these tests exercise everything downstream of the tool
//! handler: store routing, the source gate, idempotent demotion, and the
//! audit row. The handler's own concerns (scope-string parsing, the
//! unwired-sink error, `mcp.permissions`) are covered in
//! `mcp::server`'s unit tests, because the tool methods are private to
//! the crate.

use std::sync::Arc;

use anyhow::Result;
use tempfile::TempDir;

use gaviero_core::mcp::{MemoryFlagRequest, MemorySignalSink};
use gaviero_core::memory::scope::{SCOPE_GLOBAL, SCOPE_REPO, SCOPE_WORKSPACE};
use gaviero_core::memory::{
    Embedder, MemorySource, MemoryStores, MemoryType, StoreKind, StoreResult, WriteMeta,
    WriteScope, WriterConfig, WriterSignalSink, spawn_writer_task,
};
use gaviero_core::workspace::Workspace;

// ── Harness ─────────────────────────────────────────────────────────

struct TestEmbedder;

#[async_trait::async_trait]
impl Embedder for TestEmbedder {
    fn name(&self) -> &str {
        "mock"
    }

    fn dimension(&self) -> usize {
        8
    }

    async fn embed(
        &self,
        text: &str,
        _purpose: gaviero_core::memory::EmbeddingPurpose,
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

struct Harness {
    stores: Arc<MemoryStores>,
    sink: Arc<dyn MemorySignalSink>,
    _global: TempDir,
    _workspace: TempDir,
}

fn harness() -> Harness {
    let global = TempDir::new().unwrap();
    let workspace_root = TempDir::new().unwrap();
    let ws = Workspace::single_folder(workspace_root.path().to_path_buf());
    let stores = MemoryStores::open_with_paths(
        workspace_root.path(),
        &ws,
        Arc::new(TestEmbedder) as Arc<dyn Embedder>,
        "mock".to_string(),
        &global.path().join("global.db"),
    )
    .expect("open registry");
    let writer = spawn_writer_task(WriterConfig {
        stores: stores.clone(),
        llm: None,
        observer: None,
        manifest_observer: None,
    });
    Harness {
        sink: WriterSignalSink::arc(writer),
        stores,
        _global: global,
        _workspace: workspace_root,
    }
}

async fn plant(
    stores: &Arc<MemoryStores>,
    scope: &WriteScope,
    text: &str,
    source: MemorySource,
) -> i64 {
    let meta = WriteMeta::for_source(source)
        .with_type(MemoryType::Decision)
        .with_importance(0.7);
    match stores.store_scoped(scope, text, &meta).await.unwrap() {
        StoreResult::Inserted(id) => id,
        other => panic!("expected insert, got {other:?}"),
    }
}

async fn trust_of(stores: &Arc<MemoryStores>, kind: &StoreKind, id: i64) -> f32 {
    stores
        .get(kind)
        .await
        .unwrap()
        .get_memory_row(id)
        .await
        .unwrap()
        .expect("row present")
        .trust_score
}

async fn audit_rows(stores: &Arc<MemoryStores>, kind: &StoreKind, memory_id: i64) -> Vec<String> {
    stores
        .get(kind)
        .await
        .unwrap()
        .audit_payloads_for_test("agent_flag", memory_id)
        .await
        .unwrap()
}

fn request(id: i64, scope_level: i32, reason: &str) -> MemoryFlagRequest {
    MemoryFlagRequest {
        memory_id: id,
        scope_level,
        repo_id: None,
        reason: reason.to_string(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────

/// (i) A flag against a Global-scope id reaches the global store and
/// leaves the same-id row in the workspace store alone.
#[tokio::test]
async fn flag_targets_the_owning_store() {
    let h = harness();

    let workspace_id = plant(
        &h.stores,
        &WriteScope::Workspace,
        "workspace decoy row",
        MemorySource::LlmAnnotated,
    )
    .await;
    let global_id = plant(
        &h.stores,
        &WriteScope::Global,
        "global row that went stale",
        MemorySource::LlmAnnotated,
    )
    .await;
    assert_eq!(
        global_id, workspace_id,
        "precondition: independent rowid spaces hand out the same id"
    );

    let outcome = h
        .sink
        .flag(request(
            global_id,
            SCOPE_GLOBAL,
            "superseded by the new API",
        ))
        .await
        .expect("flag applied");
    assert!(outcome.accepted, "{outcome:?}");

    assert_eq!(
        trust_of(&h.stores, &StoreKind::Global, global_id).await,
        0.35,
        "LlmAnnotated 0.70 must halve to 0.35"
    );
    assert_eq!(
        trust_of(&h.stores, &StoreKind::Workspace, workspace_id).await,
        MemorySource::LlmAnnotated.default_trust(),
        "the colliding id in the workspace DB must be untouched"
    );
}

/// (v) Source gate: user-authored rows are refused, and a refusal is a
/// successful call, not an error.
#[tokio::test]
async fn flag_refuses_user_authored_rows() {
    let h = harness();
    let id = plant(
        &h.stores,
        &WriteScope::Workspace,
        "the user said so",
        MemorySource::UserRemember,
    )
    .await;

    let outcome = h
        .sink
        .flag(request(id, SCOPE_WORKSPACE, "I disagree"))
        .await
        .expect("a refusal is not a protocol error");
    assert!(!outcome.accepted, "{outcome:?}");
    assert!(
        outcome.detail.contains("user_remember"),
        "detail should name the source: {}",
        outcome.detail
    );
    assert_eq!(
        trust_of(&h.stores, &StoreKind::Workspace, id).await,
        1.0,
        "a refused flag must not mutate trust"
    );
    assert!(
        audit_rows(&h.stores, &StoreKind::Workspace, id)
            .await
            .is_empty(),
        "(vii) a refused flag writes no audit row"
    );
}

/// (vi) Idempotence: the demotion is computed from the source's default
/// trust, not as a repeated decrement, so a second flag is a no-op.
#[tokio::test]
async fn flag_is_idempotent() {
    let h = harness();
    let id = plant(
        &h.stores,
        &WriteScope::Workspace,
        "an annotated claim that aged badly",
        MemorySource::LlmAnnotated,
    )
    .await;

    for _ in 0..2 {
        let outcome = h
            .sink
            .flag(request(id, SCOPE_WORKSPACE, "stale"))
            .await
            .expect("flag applied");
        assert!(outcome.accepted);
        assert_eq!(
            trust_of(&h.stores, &StoreKind::Workspace, id).await,
            0.35,
            "repeat flags must not compound to 0.175"
        );
    }
}

/// (vii) Every applied flag writes exactly one audit row carrying the
/// before/after trust and the agent's reason, so a human can reverse it.
#[tokio::test]
async fn applied_flag_writes_one_audit_row() {
    let h = harness();
    let id = plant(
        &h.stores,
        &WriteScope::Workspace,
        "a claim about the parser",
        MemorySource::LlmExtracted,
    )
    .await;

    h.sink
        .flag(request(
            id,
            SCOPE_WORKSPACE,
            "the parser was rewritten in #227",
        ))
        .await
        .expect("flag applied");

    let rows = audit_rows(&h.stores, &StoreKind::Workspace, id).await;
    assert_eq!(rows.len(), 1, "one audit row per applied flag: {rows:?}");
    let payload: serde_json::Value = serde_json::from_str(&rows[0]).expect("payload is JSON");
    assert_eq!(payload["memory_id"], id);
    assert_eq!(payload["source"], "llm_extracted");
    // f32 → f64 widening, so compare with tolerance rather than equality.
    let near = |v: &serde_json::Value, want: f64| (v.as_f64().unwrap() - want).abs() < 1e-6;
    assert!(near(&payload["trust_before"], 0.6), "{payload}");
    assert!(near(&payload["trust_after"], 0.3), "{payload}");
    assert_eq!(payload["reason"], "the parser was rewritten in #227");
}

/// An unresolvable `(scope_level, repo_id)` pair errors rather than
/// falling back to another DB.
#[tokio::test]
async fn flag_with_unresolvable_scope_errors() {
    let h = harness();
    let id = plant(
        &h.stores,
        &WriteScope::Workspace,
        "must not be touched",
        MemorySource::LlmAnnotated,
    )
    .await;

    // Repo scope with no repo_id → no owning store.
    let err = h
        .sink
        .flag(request(id, SCOPE_REPO, "wrong"))
        .await
        .expect_err("unresolvable scope must error");
    assert!(
        err.to_string().contains("memory_flag"),
        "unexpected error: {err}"
    );
    assert_eq!(
        trust_of(&h.stores, &StoreKind::Workspace, id).await,
        MemorySource::LlmAnnotated.default_trust(),
        "no fallback write may have happened"
    );
}

/// A flag against an id that does not exist in the resolved store is an
/// error, not a silent success.
#[tokio::test]
async fn flag_on_a_missing_row_errors() {
    let h = harness();
    let err = h
        .sink
        .flag(request(9999, SCOPE_WORKSPACE, "no such row"))
        .await
        .expect_err("a missing row must error");
    assert!(
        err.to_string().contains("no memory 9999"),
        "unexpected error: {err}"
    );
}
