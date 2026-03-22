use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;

use super::embedder::Embedder;
use super::schema;

/// Privacy filter for memory search operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFilter {
    /// Return all entries — for local-only agents.
    IncludeAll,
    /// Exclude entries with privacy: local_only — for API-bound contexts.
    ExcludeLocalOnly,
}

/// A memory entry returned from the store.
#[derive(Debug, Clone)]
pub struct MemoryEntry {
    pub id: i64,
    pub namespace: String,
    pub key: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A search result with similarity score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub entry: MemoryEntry,
    pub score: f32,
}

/// Semantic memory store backed by SQLite + vector embeddings.
///
/// Key pattern: CPU-heavy embedding runs BEFORE acquiring the SQLite lock.
/// The lock is held only for brief I/O operations.
pub struct MemoryStore {
    conn: Arc<Mutex<Connection>>,
    embedder: Arc<dyn Embedder>,
}

impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore")
            .field("model", &self.embedder.model_id())
            .finish()
    }
}

impl MemoryStore {
    /// Open or create a memory store at the given path.
    pub fn open(db_path: &Path, embedder: Arc<dyn Embedder>) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("opening memory database: {}", db_path.display()))?;
        Self::init(conn, embedder)
    }

    /// Create an in-memory store (for testing).
    pub fn in_memory(embedder: Arc<dyn Embedder>) -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("opening in-memory database")?;
        Self::init(conn, embedder)
    }

    fn init(conn: Connection, embedder: Arc<dyn Embedder>) -> Result<Self> {
        schema::run_migrations(&conn).context("running schema migrations")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            embedder,
        })
    }

    /// Store a memory entry. Upserts on (namespace, key).
    ///
    /// Embedding is computed BEFORE acquiring the database lock.
    pub async fn store(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        // Compute embedding outside the lock (CPU-heavy)
        let embedding = self.embedder.embed(content)
            .context("computing embedding")?;
        let embedding_blob = embedding_to_blob(&embedding);
        let model_id = self.embedder.model_id().to_string();

        let ns = namespace.to_string();
        let k = key.to_string();
        let c = content.to_string();
        let m = metadata.map(|s| s.to_string());

        // Brief lock for database write
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO memories (namespace, key, content, embedding, model_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(namespace, key) DO UPDATE SET
                content = excluded.content,
                embedding = excluded.embedding,
                model_id = excluded.model_id,
                metadata = excluded.metadata,
                updated_at = datetime('now')",
            rusqlite::params![ns, k, c, embedding_blob, model_id, m],
        ).context("inserting memory")?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    /// Search for memories similar to the query text.
    ///
    /// Returns results sorted by cosine similarity (highest first).
    /// Embedding is computed BEFORE acquiring the database lock.
    pub async fn search(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        // Compute query embedding outside the lock
        let query_embedding = self.embedder.embed(query)
            .context("computing query embedding")?;

        let ns = namespace.to_string();

        // Brief lock for database read
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, namespace, key, content, metadata, created_at, updated_at, embedding
             FROM memories WHERE namespace = ?1 AND embedding IS NOT NULL"
        ).context("preparing search query")?;

        let mut results: Vec<SearchResult> = stmt.query_map(
            rusqlite::params![ns],
            |row| {
                let embedding_blob: Vec<u8> = row.get(7)?;
                let entry = MemoryEntry {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    key: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                };
                Ok((entry, embedding_blob))
            },
        )
        .context("executing search query")?
        .filter_map(|r| r.ok())
        .map(|(entry, blob)| {
            let stored_embedding = blob_to_embedding(&blob);
            let score = cosine_similarity(&query_embedding, &stored_embedding);
            SearchResult { entry, score }
        })
        .collect();

        // Sort by similarity descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Search across multiple namespaces.
    ///
    /// Results from all namespaces are merged and sorted by similarity.
    pub async fn search_multi(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        if namespaces.is_empty() {
            return Ok(Vec::new());
        }

        let query_embedding = self.embedder.embed(query)
            .context("computing query embedding")?;

        let conn = self.conn.lock().await;

        // Build WHERE clause: namespace IN (?, ?, ...)
        let placeholders: Vec<String> = (1..=namespaces.len()).map(|i| format!("?{}", i)).collect();
        let sql = format!(
            "SELECT id, namespace, key, content, metadata, created_at, updated_at, embedding
             FROM memories WHERE namespace IN ({}) AND embedding IS NOT NULL",
            placeholders.join(", ")
        );

        let mut stmt = conn.prepare(&sql).context("preparing multi-namespace search")?;

        let params: Vec<&dyn rusqlite::types::ToSql> = namespaces
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut results: Vec<SearchResult> = stmt.query_map(
            params.as_slice(),
            |row| {
                let embedding_blob: Vec<u8> = row.get(7)?;
                let entry = MemoryEntry {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    key: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                };
                Ok((entry, embedding_blob))
            },
        )
        .context("executing multi-namespace search")?
        .filter_map(|r| r.ok())
        .map(|(entry, blob)| {
            let stored_embedding = blob_to_embedding(&blob);
            let score = cosine_similarity(&query_embedding, &stored_embedding);
            SearchResult { entry, score }
        })
        .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Search across namespaces and format results as a prompt-ready string.
    ///
    /// Returns an empty string on error or if no results are found.
    pub async fn search_context(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
    ) -> String {
        match self.search_multi(namespaces, query, limit).await {
            Ok(results) if !results.is_empty() => {
                let mut ctx = String::from("[Memory context]:\n");
                for r in &results {
                    ctx.push_str(&format!(
                        "- [{}] {} (score: {:.2})\n",
                        r.entry.namespace, r.entry.content, r.score
                    ));
                }
                ctx
            }
            _ => String::new(),
        }
    }

    /// Store a memory entry with explicit privacy level.
    ///
    /// Like `store()`, but writes the `privacy` column.
    pub async fn store_with_privacy(
        &self,
        namespace: &str,
        key: &str,
        content: &str,
        privacy: &str,
        metadata: Option<&str>,
    ) -> Result<i64> {
        let embedding = self.embedder.embed(content)
            .context("computing embedding")?;
        let embedding_blob = embedding_to_blob(&embedding);
        let model_id = self.embedder.model_id().to_string();

        let ns = namespace.to_string();
        let k = key.to_string();
        let c = content.to_string();
        let p = privacy.to_string();
        let m = metadata.map(|s| s.to_string());

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO memories (namespace, key, content, embedding, model_id, metadata, privacy)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(namespace, key) DO UPDATE SET
                content = excluded.content,
                embedding = excluded.embedding,
                model_id = excluded.model_id,
                metadata = excluded.metadata,
                privacy = excluded.privacy,
                updated_at = datetime('now')",
            rusqlite::params![ns, k, c, embedding_blob, model_id, m, p],
        ).context("inserting memory with privacy")?;

        let id = conn.last_insert_rowid();
        Ok(id)
    }

    /// Search across namespaces with privacy filtering, returning a prompt-ready string.
    ///
    /// When `filter` is `ExcludeLocalOnly`, entries with `privacy = 'local_only'`
    /// are excluded at the SQL level (not post-query).
    pub async fn search_context_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> String {
        match self.search_multi_filtered(namespaces, query, limit, filter).await {
            Ok(results) if !results.is_empty() => {
                let mut ctx = String::from("[Memory context]:\n");
                for r in &results {
                    ctx.push_str(&format!(
                        "- [{}] {} (score: {:.2})\n",
                        r.entry.namespace, r.entry.content, r.score
                    ));
                }
                ctx
            }
            _ => String::new(),
        }
    }

    /// Search across namespaces with privacy filtering.
    async fn search_multi_filtered(
        &self,
        namespaces: &[String],
        query: &str,
        limit: usize,
        filter: PrivacyFilter,
    ) -> Result<Vec<SearchResult>> {
        if namespaces.is_empty() {
            return Ok(Vec::new());
        }

        let query_embedding = self.embedder.embed(query)
            .context("computing query embedding")?;

        let conn = self.conn.lock().await;

        let placeholders: Vec<String> = (1..=namespaces.len()).map(|i| format!("?{}", i)).collect();
        let privacy_clause = match filter {
            PrivacyFilter::ExcludeLocalOnly => " AND privacy != 'local_only'",
            PrivacyFilter::IncludeAll => "",
        };
        let sql = format!(
            "SELECT id, namespace, key, content, metadata, created_at, updated_at, embedding
             FROM memories WHERE namespace IN ({}) AND embedding IS NOT NULL{}",
            placeholders.join(", "),
            privacy_clause,
        );

        let mut stmt = conn.prepare(&sql).context("preparing filtered search")?;
        let params: Vec<&dyn rusqlite::types::ToSql> = namespaces
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut results: Vec<SearchResult> = stmt.query_map(
            params.as_slice(),
            |row| {
                let embedding_blob: Vec<u8> = row.get(7)?;
                let entry = MemoryEntry {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    key: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                };
                Ok((entry, embedding_blob))
            },
        )
        .context("executing filtered search")?
        .filter_map(|r| r.ok())
        .map(|(entry, blob)| {
            let stored_embedding = blob_to_embedding(&blob);
            let score = cosine_similarity(&query_embedding, &stored_embedding);
            SearchResult { entry, score }
        })
        .collect();

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    /// Get a specific memory by namespace and key.
    pub async fn get(&self, namespace: &str, key: &str) -> Result<Option<MemoryEntry>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, namespace, key, content, metadata, created_at, updated_at
             FROM memories WHERE namespace = ?1 AND key = ?2"
        )?;

        let entry = stmt.query_row(
            rusqlite::params![namespace, key],
            |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    namespace: row.get(1)?,
                    key: row.get(2)?,
                    content: row.get(3)?,
                    metadata: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        );

        match entry {
            Ok(e) => Ok(Some(e)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List all keys in a namespace.
    pub async fn list_keys(&self, namespace: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT key FROM memories WHERE namespace = ?1 ORDER BY key"
        )?;
        let keys: Vec<String> = stmt
            .query_map(rusqlite::params![namespace], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(keys)
    }

    /// Delete a specific memory by namespace and key.
    pub async fn delete(&self, namespace: &str, key: &str) -> Result<bool> {
        let conn = self.conn.lock().await;
        let rows = conn.execute(
            "DELETE FROM memories WHERE namespace = ?1 AND key = ?2",
            rusqlite::params![namespace, key],
        )?;
        Ok(rows > 0)
    }

    /// Clear all memories in a namespace.
    pub async fn clear_namespace(&self, namespace: &str) -> Result<usize> {
        let conn = self.conn.lock().await;
        let rows = conn.execute(
            "DELETE FROM memories WHERE namespace = ?1",
            rusqlite::params![namespace],
        )?;
        Ok(rows)
    }

    /// Reindex all memories with the current embedder.
    /// Useful when switching embedding models.
    pub async fn reindex(&self, namespace: &str) -> Result<usize> {
        // Read all entries outside the lock
        let entries = {
            let conn = self.conn.lock().await;
            let mut stmt = conn.prepare(
                "SELECT id, content FROM memories WHERE namespace = ?1"
            )?;
            let entries: Vec<(i64, String)> = stmt
                .query_map(rusqlite::params![namespace], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            entries
        };

        // Compute embeddings outside the lock
        let model_id = self.embedder.model_id().to_string();
        let mut updates = Vec::with_capacity(entries.len());
        for (id, content) in &entries {
            let embedding = self.embedder.embed(content)?;
            updates.push((*id, embedding_to_blob(&embedding)));
        }

        // Write back with a brief lock
        let conn = self.conn.lock().await;
        for (id, blob) in &updates {
            conn.execute(
                "UPDATE memories SET embedding = ?1, model_id = ?2, updated_at = datetime('now')
                 WHERE id = ?3",
                rusqlite::params![blob, model_id, id],
            )?;
        }

        Ok(updates.len())
    }

    /// Delete stale swarm-generated entries based on retention policy.
    ///
    /// Preserves all `user:*` entries (explicit `/remember` commands).
    /// Preserves entries that don't match swarm key patterns (agents:, verification:, tiers:).
    /// Best-effort — failures are logged but never fail the pipeline.
    pub async fn prune(
        &self,
        namespace: &str,
        max_age_days: u32,
        max_runs: usize,
    ) -> Result<usize> {
        let conn = self.conn.lock().await;

        let mut total_deleted = 0;

        // Delete entries older than max_age_days (only swarm-generated keys)
        if max_age_days > 0 {
            let rows = conn.execute(
                "DELETE FROM memories WHERE namespace = ?1
                 AND key NOT LIKE 'user:%'
                 AND (key LIKE 'agents:%' OR key LIKE 'verification:%' OR key LIKE 'tiers:%')
                 AND created_at < datetime('now', ?2)",
                rusqlite::params![
                    namespace,
                    format!("-{} days", max_age_days),
                ],
            )?;
            total_deleted += rows;
        }

        // Enforce max_runs: keep only the latest N run IDs
        if max_runs > 0 {
            // Collect distinct run IDs from agents:* keys, sorted by creation time
            let mut stmt = conn.prepare(
                "SELECT DISTINCT substr(key, 8, instr(substr(key, 8), ':') - 1) as run_id,
                        MIN(created_at) as first_seen
                 FROM memories
                 WHERE namespace = ?1 AND key LIKE 'agents:%'
                 GROUP BY run_id
                 ORDER BY first_seen DESC"
            )?;

            let run_ids: Vec<String> = stmt
                .query_map(rusqlite::params![namespace], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();

            if run_ids.len() > max_runs {
                let old_runs = &run_ids[max_runs..];
                for run_id in old_runs {
                    let rows = conn.execute(
                        "DELETE FROM memories WHERE namespace = ?1
                         AND (key LIKE ?2 OR key LIKE ?3 OR key LIKE ?4)",
                        rusqlite::params![
                            namespace,
                            format!("agents:{}:%", run_id),
                            format!("verification:{}", run_id),
                            format!("tiers:{}", run_id),
                        ],
                    )?;
                    total_deleted += rows;
                }
            }
        }

        Ok(total_deleted)
    }
}

// ── Embedding serialization ─────────────────────────────────────

/// Encode a float vector as little-endian bytes for SQLite BLOB storage.
fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(embedding.len() * 4);
    for &v in embedding {
        blob.extend_from_slice(&v.to_le_bytes());
    }
    blob
}

/// Decode a BLOB back into a float vector.
fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock embedder that produces deterministic vectors from content hash.
    struct MockEmbedder;

    impl Embedder for MockEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            // Simple hash-based deterministic embedding
            let mut vec = vec![0.0f32; 8];
            for (i, byte) in text.bytes().enumerate() {
                vec[i % 8] += byte as f32;
            }
            // L2 normalize
            let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for v in &mut vec {
                    *v /= norm;
                }
            }
            Ok(vec)
        }

        fn dimensions(&self) -> usize { 8 }
        fn model_id(&self) -> &str { "mock" }
    }

    fn mock_embedder() -> Arc<dyn Embedder> {
        Arc::new(MockEmbedder)
    }

    #[tokio::test]
    async fn test_store_and_get() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let id = store.store("test", "greeting", "hello world", None).await.unwrap();
        assert!(id > 0);

        let entry = store.get("test", "greeting").await.unwrap().unwrap();
        assert_eq!(entry.content, "hello world");
        assert_eq!(entry.namespace, "test");
        assert_eq!(entry.key, "greeting");
    }

    #[tokio::test]
    async fn test_upsert() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "k", "original", None).await.unwrap();
        store.store("ns", "k", "updated", None).await.unwrap();

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "updated");
    }

    #[tokio::test]
    async fn test_search() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "rust", "Rust programming language", None).await.unwrap();
        store.store("ns", "python", "Python scripting language", None).await.unwrap();
        store.store("ns", "cooking", "How to make pasta", None).await.unwrap();

        let results = store.search("ns", "Rust language", 2).await.unwrap();
        assert_eq!(results.len(), 2);
        // First result should be most similar
        assert!(results[0].score >= results[1].score);
    }

    #[tokio::test]
    async fn test_list_keys() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "b", "content b", None).await.unwrap();
        store.store("ns", "a", "content a", None).await.unwrap();
        store.store("other", "c", "content c", None).await.unwrap();

        let keys = store.list_keys("ns").await.unwrap();
        assert_eq!(keys, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn test_delete() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "k", "content", None).await.unwrap();
        assert!(store.delete("ns", "k").await.unwrap());
        assert!(store.get("ns", "k").await.unwrap().is_none());
        assert!(!store.delete("ns", "k").await.unwrap()); // already deleted
    }

    #[tokio::test]
    async fn test_clear_namespace() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "a", "a", None).await.unwrap();
        store.store("ns", "b", "b", None).await.unwrap();
        store.store("other", "c", "c", None).await.unwrap();

        let cleared = store.clear_namespace("ns").await.unwrap();
        assert_eq!(cleared, 2);
        assert!(store.list_keys("ns").await.unwrap().is_empty());
        assert_eq!(store.list_keys("other").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn test_embedding_roundtrip() {
        let original = vec![1.0, -2.5, 3.14, 0.0];
        let blob = embedding_to_blob(&original);
        let decoded = blob_to_embedding(&blob);
        assert_eq!(original, decoded);
    }

    #[tokio::test]
    async fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &a) - 1.0).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 0.001);
    }

    #[tokio::test]
    async fn test_store_with_privacy() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        let id = store.store_with_privacy("ns", "k", "secret content", "local_only", None).await.unwrap();
        assert!(id > 0);

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "secret content");
    }

    #[tokio::test]
    async fn test_search_context_filtered_excludes_local_only() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        // Store entries with different privacy levels
        store.store_with_privacy("ns", "public1", "Rust programming", "public", None).await.unwrap();
        store.store_with_privacy("ns", "private1", "Secret clinical data", "local_only", None).await.unwrap();
        store.store_with_privacy("ns", "public2", "Python scripting", "public", None).await.unwrap();

        // ExcludeLocalOnly should not return the local_only entry
        let namespaces = vec!["ns".to_string()];
        let ctx = store.search_context_filtered(&namespaces, "programming", 10, PrivacyFilter::ExcludeLocalOnly).await;
        assert!(!ctx.contains("Secret clinical data"));
        assert!(ctx.contains("Rust programming") || ctx.contains("Python scripting"));

        // IncludeAll should return all entries
        let ctx_all = store.search_context_filtered(&namespaces, "programming", 10, PrivacyFilter::IncludeAll).await;
        assert!(ctx_all.contains("Secret clinical data") || ctx_all.contains("Rust programming"));
    }

    #[tokio::test]
    async fn test_store_with_privacy_upserts() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store_with_privacy("ns", "k", "v1", "public", None).await.unwrap();
        store.store_with_privacy("ns", "k", "v2", "local_only", Some("{\"v\":2}")).await.unwrap();

        let entry = store.get("ns", "k").await.unwrap().unwrap();
        assert_eq!(entry.content, "v2");
    }

    #[tokio::test]
    async fn test_prune_preserves_user_entries() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();
        store.store("ns", "user:note1", "my note", None).await.unwrap();
        store.store("ns", "agents:run1:unit1", "agent result", None).await.unwrap();
        store.store("ns", "verification:run1", "v result", None).await.unwrap();

        // Prune with 0 max_age (won't delete recent), but exercise the code
        let deleted = store.prune("ns", 0, 100).await.unwrap();
        assert_eq!(deleted, 0); // Nothing old enough

        // user entry should always survive
        let keys = store.list_keys("ns").await.unwrap();
        assert!(keys.contains(&"user:note1".to_string()));
    }

    #[tokio::test]
    async fn test_prune_by_max_runs() {
        let store = MemoryStore::in_memory(mock_embedder()).unwrap();

        // Create entries for 3 runs
        for run in 1..=3 {
            store.store("ns", &format!("agents:run{}:a", run), &format!("run {} a", run), None).await.unwrap();
            store.store("ns", &format!("agents:run{}:b", run), &format!("run {} b", run), None).await.unwrap();
        }
        // Also a user entry that should not be pruned
        store.store("ns", "user:important", "keep this", None).await.unwrap();

        let keys_before = store.list_keys("ns").await.unwrap();
        assert_eq!(keys_before.len(), 7); // 6 agent + 1 user

        // Prune to keep only 2 most recent runs
        let deleted = store.prune("ns", 0, 2).await.unwrap();
        // The oldest run (run1) should be pruned
        assert!(deleted > 0);

        let keys_after = store.list_keys("ns").await.unwrap();
        assert!(keys_after.contains(&"user:important".to_string()));
        // run1 entries should be gone
        assert!(!keys_after.contains(&"agents:run1:a".to_string()));
    }
}
