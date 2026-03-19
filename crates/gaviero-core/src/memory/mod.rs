pub mod embedder;
pub mod schema;
pub mod onnx_embedder;
pub mod store;
pub mod model_manager;

pub use embedder::Embedder;
pub use store::MemoryStore;
pub use model_manager::ModelManager;

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Initialize a MemoryStore with the default ONNX embedder.
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
        onnx_embedder::OnnxEmbedder::from_model(&model_manager::E5_SMALL_V2)?
    ) as Arc<dyn Embedder>;

    let store = MemoryStore::open(&db, embedder)?;
    Ok(Arc::new(store))
}

/// Default database path: `~/.local/share/gaviero/memory.db` (or platform equivalent).
fn default_db_path() -> anyhow::Result<PathBuf> {
    dirs::data_dir()
        .map(|d| d.join("gaviero").join("memory.db"))
        .ok_or_else(|| anyhow::anyhow!("cannot determine data directory for memory database"))
}
