pub mod annotations;
pub mod consolidation;
pub mod consolidation_llm;
pub mod embedder;
pub mod eval;
pub mod extractor;
pub mod model_manager;
pub mod observer;
pub mod onnx_embedder;
pub mod reembed_migration;
pub mod reranker;
pub mod retrieval;
pub mod schema;
pub mod scope;
pub mod scoring;
pub mod session_consolidator;
pub mod sleeptime;
pub mod store;
pub mod stores;
pub mod telemetry;
pub mod trust_defaults;
pub mod writer;

pub use annotations::{
    AnnotationFlag, ParsedResponse, TurnAnnotations, apply_short_turn_cap, parse_and_strip,
};
pub use consolidation_llm::{BackendConsolidationLlm, ConsolidationLlm, NoopConsolidationLlm};
pub use embedder::{DualEmbedder, Embedder, EmbeddingPurpose, NullEmbedder};
pub use model_manager::ModelManager;
pub use observer::MemoryObserver;
pub use reranker::{
    GTE_RERANKER_MODERNBERT_BASE, ModernBertReranker, NullReranker, RerankConfig, Reranker,
    apply_reranker_blend, blend_rerank, build_reranker, resolve_reranker_model, sigmoid_calibrate,
};
pub use retrieval::{
    CandidatePoolEntry, ChatInjection, ChatInjectionConfig, RetrievalConfig, RetrievalMode,
    RetrievalOutput, ScopeMix, retrieve_for_chat, retrieve_for_chat_with_reranker, retrieve_ranked,
};
pub use scope::{
    MemoryScope, MemoryType, ScopeFilter, StoreKind, StoreResult, Trust, WriteMeta, WriteScope,
    hash_path,
};
pub use scoring::{ScoredMemory, SearchConfig, format_memories_for_prompt};
pub use session_consolidator::{
    CandidateBrief, ConsolidationOp, ConsolidatorResponse, ExistingBrief, PROMPT_VERSION,
    PromotionRequest,
};
pub use sleeptime::{
    SleeptimeConfig, SleeptimeObserver, SleeptimeOperation, SleeptimeReport, run_sleeptime,
};
pub use store::{MemoryStore, MemoryUtilization, StoreOptions};
pub use stores::{EmbedderMismatch, MemoryStores};
pub use telemetry::{
    ClassifiedItem, ClassifyConfig, TelemetryObserver, TelemetryReport, UseClass, classify_turn,
};
pub use trust_defaults::{MemorySource, clamp_trust};
pub use writer::{
    ACK_TIMEOUT_MS, WriteResult, WriterConfig, WriterHandle, WriterMessage, spawn_writer_task,
};

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Initialize a MemoryStore with the default ONNX embedder.
///
/// B1: respects the `GAVIERO_EMBEDDER_MODEL` env var (consumed by both
/// TUI and CLI bootstraps that read workspace settings) so callers can
/// switch between `nomic` and `gte-modernbert` without changing code
/// paths. Empty / absent → nomic (the pre-B1 default; flip to
/// gte-modernbert in the B1g follow-up PR).
///
/// This is blocking (model download + ONNX session load) — callers should wrap
/// in `tokio::task::spawn_blocking`.
///
/// `db_path` defaults to `dirs::data_dir()/gaviero/memory.db`.
pub fn init(db_path: Option<&Path>) -> anyhow::Result<Arc<MemoryStore>> {
    let model_name = std::env::var("GAVIERO_EMBEDDER_MODEL").unwrap_or_default();
    init_with_embedder_name(db_path, &model_name)
}

/// Like [`init`] but takes the embedder selection explicitly. Workspace
/// bootstrap passes the resolved `memory.embedder.model` setting here;
/// the env-var path in [`init`] is the headless / test fallback.
pub fn init_with_embedder_name(
    db_path: Option<&Path>,
    embedder_name: &str,
) -> anyhow::Result<Arc<MemoryStore>> {
    let db = match db_path {
        Some(p) => p.to_path_buf(),
        None => default_db_path()?,
    };

    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let embedder = build_embedder_by_name(embedder_name)?;
    let store = MemoryStore::open(&db, embedder)?;
    Ok(Arc::new(store))
}

/// Build a fresh embedder from a settings string. Used by both the
/// store-bootstrap path (cold start) and the B1 `/reembed` migration
/// (loads the *new* embedder while the *old* one stays live on the
/// existing store).
pub fn build_embedder_by_name(embedder_name: &str) -> anyhow::Result<Arc<dyn Embedder>> {
    create_embedder_from_settings(embedder_name)
}

/// Factory for configured embedders. New implementations should be wired here.
///
/// Supported static names today:
/// - `nomic`, `nomic-v15`, `nomic-embed-text-v1.5`
/// - `gte-modernbert`, `gte-modernbert-base`
/// - `e5`, `e5-small-v2`
/// - `null`
///
/// A/B mode accepts `dual:<primary>,<comparison>` and returns primary vectors
/// while logging comparison runs under `memory_embedder_ab`.
pub fn create_embedder_from_settings(embedder_name: &str) -> anyhow::Result<Arc<dyn Embedder>> {
    let trimmed = embedder_name.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return Ok(Arc::new(NullEmbedder::new(768)) as Arc<dyn Embedder>);
    }

    if let Some(rest) = trimmed.strip_prefix("dual:") {
        let mut parts = rest.split(',').map(str::trim).filter(|s| !s.is_empty());
        let primary_name = parts.next().unwrap_or("nomic");
        let comparison_name = parts.next().unwrap_or("gte-modernbert");
        let primary = build_single_embedder_by_name(primary_name)?;
        let comparison = build_single_embedder_by_name(comparison_name)?;
        return Ok(Arc::new(DualEmbedder::new(primary, comparison)) as Arc<dyn Embedder>);
    }

    build_single_embedder_by_name(trimmed)
}

fn build_single_embedder_by_name(embedder_name: &str) -> anyhow::Result<Arc<dyn Embedder>> {
    let model = model_manager::resolve_embedder_model(embedder_name);
    Ok(Arc::new(onnx_embedder::OnnxEmbedder::from_model(model)?) as Arc<dyn Embedder>)
}

/// Initialize a MemoryStore at a workspace-local path.
///
/// Used when the memory database should live alongside the workspace
/// (e.g., `<workspace-root>/.gaviero/memory.db`).
pub fn init_workspace(workspace_root: &Path) -> anyhow::Result<Arc<MemoryStore>> {
    let db = workspace_root.join(".gaviero/memory.db");
    init(Some(&db))
}

/// Workspace-local init with explicit embedder selection.
pub fn init_workspace_with_embedder_name(
    workspace_root: &Path,
    embedder_name: &str,
) -> anyhow::Result<Arc<MemoryStore>> {
    let db = workspace_root.join(".gaviero/memory.db");
    init_with_embedder_name(Some(&db), embedder_name)
}

/// Initialize the global memory store at `~/.config/gaviero/memory.db`.
pub fn init_global() -> anyhow::Result<Arc<MemoryStore>> {
    let db = global_db_path()?;
    init(Some(&db))
}

/// Bootstrap entry point for the multi-DB registry. The TUI / CLI
/// bootstrap should call this in place of [`init_workspace`] /
/// [`init_workspace_with_embedder_name`] once they're ready to thread
/// an `Arc<MemoryStores>` through the call graph.
///
/// Eagerly opens the global and workspace stores; folder stores are
/// pre-registered (their paths are known) but lazy-opened on first
/// retrieval / write. Runs the v10 split migration on the workspace
/// DB — moving any pre-v10 `scope_level >= SCOPE_REPO` rows into the
/// matching folder DBs. Idempotent.
///
/// **Blocking**: builds the embedder ONNX session. Wrap call in
/// `tokio::task::spawn_blocking` from async contexts.
///
/// Embedder selection follows the same rule as [`init`]: respects
/// `GAVIERO_EMBEDDER_MODEL` env var, otherwise the legacy default.
pub fn init_workspace_stores(
    workspace_root: &Path,
    workspace: &crate::workspace::Workspace,
) -> anyhow::Result<Arc<MemoryStores>> {
    let model_name = std::env::var("GAVIERO_EMBEDDER_MODEL").unwrap_or_default();
    init_workspace_stores_with_embedder_name(workspace_root, workspace, &model_name)
}

/// Like [`init_workspace_stores`] but takes the embedder selection
/// explicitly. Workspace bootstrap passes the resolved
/// `memory.embedder.model` setting here.
pub fn init_workspace_stores_with_embedder_name(
    workspace_root: &Path,
    workspace: &crate::workspace::Workspace,
    embedder_name: &str,
) -> anyhow::Result<Arc<MemoryStores>> {
    MemoryStores::open(workspace_root, workspace, embedder_name)
}

/// Path to the global memory database.
pub fn global_db_path() -> anyhow::Result<PathBuf> {
    dirs::config_dir()
        .map(|d| d.join("gaviero/memory.db"))
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory for global memory"))
}

/// Default database path: `~/.local/share/gaviero/memory.db` (or platform equivalent).
fn default_db_path() -> anyhow::Result<PathBuf> {
    dirs::data_dir()
        .map(|d| d.join("gaviero").join("memory.db"))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory for memory database"))
}
