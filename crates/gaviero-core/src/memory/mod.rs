pub mod embedder;
pub mod schema;
pub mod onnx_embedder;
pub mod store;
pub mod model_manager;
pub mod code_graph;
pub mod consolidation;
pub mod scope;
pub mod scoring;

pub use embedder::Embedder;
pub use store::{MemoryStore, StoreOptions};
pub use model_manager::ModelManager;
pub use scope::{
    MemoryScope, WriteScope, ScopeFilter, WriteMeta, StoreResult, Trust, MemoryType,
    hash_path,
};
pub use scoring::{SearchConfig, ScoredMemory, format_memories_for_prompt};

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Initialize a MemoryStore with the default ONNX embedder (nomic-embed-text-v1.5).
///
/// This is blocking (model download + ONNX session load) — callers should wrap
/// in `tokio::task::spawn_blocking`.
///
/// `db_path` defaults to `dirs::data_dir()/gaviero/memory.db`.
pub fn init(db_path: Option<&Path>) -> anyhow::Result<Arc<MemoryStore>> {
    let db = match db_path {
        Some(p) => p.to_path_buf(),
        None => default_db_path()?,
    };

    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let embedder = Arc::new(
        onnx_embedder::OnnxEmbedder::from_model(&model_manager::NOMIC_EMBED_TEXT_V1_5)?
    ) as Arc<dyn Embedder>;

    let store = MemoryStore::open(&db, embedder)?;
    Ok(Arc::new(store))
}

/// Initialize a MemoryStore at a workspace-local path.
///
/// Used when the memory database should live alongside the workspace
/// (e.g., `<workspace-root>/.gaviero/memory.db`).
pub fn init_workspace(workspace_root: &Path) -> anyhow::Result<Arc<MemoryStore>> {
    let db = workspace_root.join(".gaviero/memory.db");
    init(Some(&db))
}

/// Initialize the global memory store at `~/.config/gaviero/memory.db`.
pub fn init_global() -> anyhow::Result<Arc<MemoryStore>> {
    let db = global_db_path()?;
    init(Some(&db))
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
