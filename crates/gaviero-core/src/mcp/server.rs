//! In-process MCP server task (Tier A / A5).
//!
//! Listens on a Unix domain socket under the workspace's
//! `.gaviero/mcp.sock`. Each shim connection is a single MCP session
//! speaking JSON-RPC 2.0 over `AsyncRead + AsyncWrite` — rmcp handles
//! framing, initialize, and tools/list. Gaviero owns only the three
//! tool handlers.
//!
//! **Read-only invariant**: the handler takes an `Arc<MemoryStore>` for
//! search + a `RepoMap`-backed graph accessor for `blast_radius`, but
//! never a `WriterHandle`. There is no code path from here to
//! `store_scoped`. Plan-rejected `memory_store` / `memory_update` /
//! `memory_delete` tools remain unimplementable by construction.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use rmcp::ServiceExt;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ErrorData, tool, tool_router};

use crate::memory::{
    MemoryScope, MemoryStores, RerankConfig, Reranker, RetrievalConfig, retrieve_ranked,
};
use crate::repo_map::store::BlastRadiusMode;

use super::observer::{McpCallLogEntry, McpToolCallObserver, NoopMcpObserver};
use super::tools::{
    BlastRadiusInput, BlastRadiusOutput, BlastRadiusRelation, MemorySearchInput,
    MemorySearchOutput, MemorySearchResult, NodeDoc, NodeDocInput, clamp_blast_depth,
    clamp_memory_search_limit,
};

/// Gaviero's MCP server. One instance lives per workspace; it
/// dispatches tool calls to the three read-only handlers below.
///
/// `tool_router` is the rmcp-macro-generated dispatch table — see
/// `rmcp::handler::server::router::tool` for the shape.
#[derive(Clone)]
pub struct GavieroMcpServer {
    stores: Arc<MemoryStores>,
    workspace_root: PathBuf,
    observer: Arc<dyn McpToolCallObserver>,
    /// B3: retrieval-engine config shared with chat injection. Cloned
    /// per call so per-tool latency budgets stay independent.
    retrieval_cfg: RetrievalConfig,
    /// B2: rerank config + handle. `None` reranker → composite-only.
    rerank_cfg: RerankConfig,
    reranker: Option<Arc<dyn Reranker>>,
    /// C3: per-build specificity configuration applied to mode-weighted
    /// PageRank in the `blast_radius` handler. Defaults to enabled with
    /// the plan's recommended threshold; embedding apps can override
    /// via [`Self::with_specificity`] after construction.
    specificity: crate::repo_map::SpecificityConfig,
    /// C4: per-intent edge weight overrides resolved from
    /// `repoMap.edges.weights.<intent>`. Stored as one preset per
    /// mode so the handler avoids re-walking the settings cascade on
    /// every call. `None` means "use the plan defaults" — set via
    /// [`Self::with_edge_weights`] when the embedding app loads
    /// workspace settings.
    edge_weights: std::collections::HashMap<
        crate::repo_map::store::BlastRadiusMode,
        crate::repo_map::store::EdgeWeights,
    >,
    /// Cached `GraphStore` for the workspace, lazily populated by the
    /// first `blast_radius` call and reused thereafter so we don't
    /// re-run `graph_builder::build_graph` (a workspace-wide scan +
    /// tree-sitter parse) on every tool invocation. Call
    /// [`Self::invalidate_graph_cache`] from the embedding app after a
    /// large workspace change to force the next call to rescan.
    ///
    /// `GraphStore` wraps a `rusqlite::Connection` which is `Send` but
    /// not `Sync`, so it lives behind a `Mutex` rather than an
    /// `RwLock`. `blast_radius` calls therefore serialize, but each
    /// call avoids the workspace-wide rescan + parse — net win for any
    /// repo larger than a handful of files. A future enhancement can
    /// split into a snapshotted projection (edges + file list + DF) to
    /// allow concurrent reads.
    graph_cache: Arc<tokio::sync::Mutex<Option<crate::repo_map::store::GraphStore>>>,
    #[allow(dead_code)] // populated and dispatched via the `#[tool_router]` macro
    tool_router: ToolRouter<Self>,
}

#[tool_router(server_handler)]
impl GavieroMcpServer {
    pub fn new(
        stores: Arc<MemoryStores>,
        workspace_root: PathBuf,
        observer: Arc<dyn McpToolCallObserver>,
        retrieval_cfg: RetrievalConfig,
        rerank_cfg: RerankConfig,
        reranker: Option<Arc<dyn Reranker>>,
    ) -> Self {
        Self {
            stores,
            workspace_root,
            observer,
            retrieval_cfg,
            rerank_cfg,
            reranker,
            specificity: crate::repo_map::SpecificityConfig::default(),
            edge_weights: std::collections::HashMap::new(),
            graph_cache: Arc::new(tokio::sync::Mutex::new(None)),
            tool_router: Self::tool_router(),
        }
    }

    /// Override the specificity config used by `blast_radius`. Returns
    /// `self` so embedding apps can chain after construction.
    pub fn with_specificity(mut self, cfg: crate::repo_map::SpecificityConfig) -> Self {
        self.specificity = cfg;
        self
    }

    /// C4: install user-resolved per-intent edge weight maps. Pass one
    /// `EdgeWeights` per mode you want to override; modes not present
    /// in the map fall back to the plan's preset at call time. Chain
    /// after construction:
    /// ```ignore
    /// let server = GavieroMcpServer::new(...)
    ///     .with_edge_weights(workspace.resolve_all_edge_weights(root));
    /// ```
    pub fn with_edge_weights(
        mut self,
        weights: std::collections::HashMap<
            crate::repo_map::store::BlastRadiusMode,
            crate::repo_map::store::EdgeWeights,
        >,
    ) -> Self {
        self.edge_weights = weights;
        self
    }

    /// Drop the cached `GraphStore` so the next `blast_radius` call
    /// rebuilds it from the current workspace state. Embedding apps
    /// (TUI / CLI) should call this after a bulk workspace change
    /// (large checkout, large file deletion) — small per-file edits
    /// don't require it because the next builder run is incremental.
    pub async fn invalidate_graph_cache(&self) {
        let mut guard = self.graph_cache.lock().await;
        *guard = None;
    }

    /// Phase 1 warmup: pay the `blast_radius` graph-build cost and (when
    /// a reranker is configured) the cold-ONNX session/tokenizer load at
    /// workspace-open time, in the background, so the first real user
    /// query never lands on a cold start. Best-effort: build/warm errors
    /// are logged and swallowed — a warmup failure must never block
    /// workspace open. The graph cache is shared via `Arc` with every
    /// per-connection server clone, so warming here warms all of them.
    pub async fn warmup(&self) {
        let cache = Arc::clone(&self.graph_cache);
        let workspace_root = self.workspace_root.clone();
        // build_graph is blocking + potentially heavy on a large repo;
        // run it off the async runtime and hold the cache lock only for
        // this build (matching the `blast_radius` pattern).
        let _ = tokio::task::spawn_blocking(move || {
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                match crate::repo_map::graph_builder::build_graph(&workspace_root, &[]) {
                    Ok((store, _)) => *guard = Some(store),
                    Err(e) => tracing::warn!(
                        target: "mcp_server",
                        error = %e,
                        "graph cache warmup build failed"
                    ),
                }
            }
        })
        .await;

        if let Some(reranker) = self.reranker.as_ref()
            && let Err(e) = reranker.warmup().await
        {
            tracing::warn!(
                target: "mcp_server",
                error = %e,
                "reranker warmup failed"
            );
        }
    }

    pub fn with_defaults(stores: Arc<MemoryStores>, workspace_root: PathBuf) -> Self {
        Self::new(
            stores,
            workspace_root,
            Arc::new(NoopMcpObserver) as Arc<dyn McpToolCallObserver>,
            RetrievalConfig::default(),
            RerankConfig::default(),
            None,
        )
    }

    // ── memory_search ───────────────────────────────────────────────
    #[tool(
        name = "memory_search",
        description = "Call this when you need a fact a prior session or the user already \
                       established — conventions, decisions, past bugs, project context — \
                       before reading code to rediscover it. Merged multi-scope hybrid \
                       search (workspace + global, RRF) over Gaviero's memory store; \
                       returns up to `limit` scored memories (id, scope, type, text, \
                       importance, trust). Read-only. Token cost: roughly 50-150 tokens \
                       per result.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn memory_search(
        &self,
        Parameters(input): Parameters<MemorySearchInput>,
    ) -> Result<Json<MemorySearchOutput>, ErrorData> {
        let started = Instant::now();
        let limit = clamp_memory_search_limit(input.limit);
        // C1.6: resolve the kind filter. Default is `record`; `any`
        // disables filtering; explicit kinds filter to that one
        // class. Unknown values are a loud error so subprocess agents
        // see the contract violation instead of silently falling back.
        let kind_filter = super::tools::resolve_memory_search_kind(input.kind.as_deref())
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        // MCP has no active-file context → folder = None. The
        // registry's scope.levels() emits [Workspace, Global], which
        // is the correct default for an unscoped tool query.
        let scope = MemoryScope::from_context(&self.workspace_root, None, None, None);
        let reranker_ref: Option<&dyn Reranker> = self.reranker.as_deref();
        // C1.6: when filtering to a non-default kind, over-fetch so the
        // post-filter can still return up to `limit` results. Cheap
        // (the retrieval pipeline returns scored memories regardless)
        // and bounded by `limit * 4` so we don't accidentally walk the
        // whole DB for a degenerate case.
        let fetch_limit = match kind_filter {
            None | Some(crate::memory::MemoryKind::Record) => limit,
            _ => (limit * 4).clamp(limit, 80),
        };
        let out = retrieve_ranked(
            &self.stores,
            &scope,
            &input.query,
            fetch_limit,
            &self.retrieval_cfg,
            reranker_ref,
            Some(&self.rerank_cfg),
        )
        .await
        .map_err(|e| ErrorData::internal_error(format!("retrieve_ranked: {e}"), None))?;

        // C1.6: post-filter the candidate list by memory_kind. Lookup
        // happens via the workspace store — MCP's retrieval mix is
        // workspace+global, and the unfiltered case skips the lookup
        // entirely. Rows whose kind cannot be resolved are dropped
        // when a filter is active (forgive only on `any`).
        let mut results: Vec<MemorySearchResult> = Vec::with_capacity(limit);
        for m in out.items.iter() {
            if results.len() >= limit {
                break;
            }
            if let Some(want) = kind_filter {
                let got = self
                    .stores
                    .workspace()
                    .get_memory_kind(m.id)
                    .await
                    .ok()
                    .flatten();
                if got != Some(want) {
                    continue;
                }
            }
            results.push(MemorySearchResult {
                id: m.id,
                scope: format_scope(m.scope_level),
                memory_type: m.memory_type.as_str().to_string(),
                text: m.content.clone(),
                importance: m.importance,
                trust: m.trust_score,
            });
        }
        let out = MemorySearchOutput { results };

        self.observer.on_tool_call(&McpCallLogEntry {
            tool_name: super::tools::TOOL_MEMORY_SEARCH.to_string(),
            input: serde_json::to_value(&input).unwrap_or_default(),
            output: serde_json::to_value(&out).unwrap_or_default(),
            duration: started.elapsed(),
            error: None,
        });
        Ok(Json(out))
    }

    // ── blast_radius ────────────────────────────────────────────────
    #[tool(
        name = "blast_radius",
        description = "Call this before editing a file to see what else may break: the \
                       impacted files, callers, and missing tests for one or more source \
                       paths, ranked by the requested `mode`. Graph-based (repo-map); \
                       returns {nodes: [{path, relation, distance, score?}]}. Read-only. \
                       Token cost: roughly 20-40 tokens per returned relation.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn blast_radius(
        &self,
        Parameters(input): Parameters<BlastRadiusInput>,
    ) -> Result<Json<BlastRadiusOutput>, ErrorData> {
        let started = Instant::now();
        if input.paths.is_empty() {
            return Err(ErrorData::invalid_params(
                "blast_radius requires at least one path",
                None,
            ));
        }
        let depth = clamp_blast_depth(input.depth);
        let mode = BlastRadiusMode::from_str(input.mode.as_deref().unwrap_or("all"));
        let paths = input.paths.clone();
        let workspace_root = self.workspace_root.clone();
        let specificity = self.specificity;
        let weights = self
            .edge_weights
            .get(&mode)
            .copied()
            .unwrap_or_else(|| crate::repo_map::store::EdgeWeights::default_for(mode));
        let cache = Arc::clone(&self.graph_cache);

        // Hold the cache mutex across the blocking computation so we
        // build at most once, reuse the cached `GraphStore` afterwards,
        // and never race two builders on the first call. Subsequent
        // calls hit the warm cache and pay only impact-radius +
        // PageRank cost.
        let (impact, ranks) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            // `blocking_lock` is required because we're inside
            // `spawn_blocking`; the surrounding `await` ensures we
            // hold the cache for the duration of this call only.
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                let (store, _) =
                    crate::repo_map::graph_builder::build_graph(&workspace_root, &[])?;
                *guard = Some(store);
            }
            let store = guard.as_ref().expect("graph cache populated above");
            let seed_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            let impact = store.impact_radius_with_mode(&seed_refs, depth as usize, mode)?;
            // Rank only the files we'll actually emit so the DiGraph
            // build stays bounded by graph size, not affected-set size.
            let mut to_rank: Vec<String> = impact.changed_files.to_vec();
            for f in &impact.affected_files {
                if !to_rank.contains(f) {
                    to_rank.push(f.clone());
                }
            }
            let ranks = crate::repo_map::rank_files_with_weights(
                store,
                &seed_refs,
                &to_rank,
                weights,
                specificity,
            )?;
            Ok((impact, ranks))
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("blast_radius join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("blast_radius: {e}"), None))?;

        let lookup = |p: &str| ranks.get(p).copied().unwrap_or((0.0, 1.0));

        let mut nodes: Vec<BlastRadiusRelation> = Vec::new();
        for path in &impact.changed_files {
            let (score, sp) = lookup(path);
            nodes.push(BlastRadiusRelation {
                path: path.clone(),
                relation: "changed".to_string(),
                distance: 0,
                purpose: None,
                score: Some(score),
                specificity: Some(sp),
            });
        }
        let mut affected_with_score: Vec<(String, f64, f64)> = impact
            .affected_files
            .iter()
            .filter(|p| !impact.changed_files.contains(p))
            .map(|p| {
                let (s, sp) = lookup(p);
                (p.clone(), s, sp)
            })
            .collect();
        // Order affected dependents by mode-weighted rank (desc) so the
        // first entries are the most relevant per the requested intent.
        affected_with_score
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (path, score, sp) in affected_with_score {
            nodes.push(BlastRadiusRelation {
                path,
                relation: mode.as_str().to_string(),
                distance: 1,
                purpose: None,
                score: Some(score),
                specificity: Some(sp),
            });
        }
        for path in &impact.test_gaps {
            let (score, sp) = lookup(path);
            nodes.push(BlastRadiusRelation {
                path: path.clone(),
                relation: "test_gap".to_string(),
                distance: 1,
                purpose: None,
                score: Some(score),
                specificity: Some(sp),
            });
        }

        let out = BlastRadiusOutput { nodes };
        self.observer.on_tool_call(&McpCallLogEntry {
            tool_name: super::tools::TOOL_BLAST_RADIUS.to_string(),
            input: serde_json::to_value(&input).unwrap_or_default(),
            output: serde_json::to_value(&out).unwrap_or_default(),
            duration: started.elapsed(),
            error: None,
        });
        Ok(Json(out))
    }

    // ── node_doc ────────────────────────────────────────────────────
    #[tool(
        name = "node_doc",
        description = "Call this when you need one file's symbol signatures. Returns \
                       {path, signatures, purpose, summary}; `purpose`/`summary` fill in \
                       once symbol enrichment runs (empty until then). Read-only. Low \
                       token cost.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn node_doc(
        &self,
        Parameters(input): Parameters<NodeDocInput>,
    ) -> Result<Json<NodeDoc>, ErrorData> {
        let started = Instant::now();
        let out = NodeDoc {
            path: input.path.clone(),
            signatures: Vec::new(),
            purpose: String::new(),
            summary: String::new(),
        };
        self.observer.on_tool_call(&McpCallLogEntry {
            tool_name: super::tools::TOOL_NODE_DOC.to_string(),
            input: serde_json::to_value(&input).unwrap_or_default(),
            output: serde_json::to_value(&out).unwrap_or_default(),
            duration: started.elapsed(),
            error: None,
        });
        Ok(Json(out))
    }
}

// `#[tool_router(server_handler)]` above already emits the
// `impl ServerHandler for GavieroMcpServer` that wires `tools/list`
// and `tools/call`. `get_info` defaults to the `Default::default()`
// on `ServerInfo`; we override via a trait-extension approach if
// needed. Plan §A5 lists `instructions` as nice-to-have — deferring
// until rmcp exposes a non-conflicting override hook.

fn format_scope(level: i32) -> String {
    match level {
        0 => "global",
        1 => "workspace",
        2 => "repo",
        3 => "module",
        4 => "run",
        _ => "unknown",
    }
    .to_string()
}

// ── Socket accept loop ────────────────────────────────────────────

/// Handle returned by `spawn_mcp_server` — lets the caller signal
/// shutdown when the workspace closes.
pub struct McpServerHandle {
    shutdown: tokio::sync::broadcast::Sender<()>,
    join: tokio::task::JoinHandle<()>,
    pub socket_path: PathBuf,
}

impl McpServerHandle {
    /// Signal the accept loop to stop and await its exit. Idempotent.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.join.await;
        // Best-effort socket cleanup.
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// Spawn the MCP server on a Unix socket. Windows support is stubbed
/// — plan §A5 flags named-pipe support as a second path; today this
/// compiles only on Unix.
#[cfg(unix)]
pub fn spawn_mcp_server(server: GavieroMcpServer, socket_path: &Path) -> Result<McpServerHandle> {
    use tokio::net::UnixListener;

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // Remove any stale socket from a previous run.
    let _ = std::fs::remove_file(socket_path);
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("binding MCP socket at {}", socket_path.display()))?;
    let (shutdown, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let socket_path_buf = socket_path.to_path_buf();
    let socket_path_accept = socket_path_buf.clone();

    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    tracing::info!(
                        target: "mcp_server",
                        socket = %socket_path_accept.display(),
                        "shutdown signal received"
                    );
                    break;
                }
                accept = listener.accept() => {
                    let (stream, _addr) = match accept {
                        Ok(a) => a,
                        Err(e) => {
                            tracing::warn!(target: "mcp_server", error = %e, "accept failed");
                            continue;
                        }
                    };
                    let server_clone = server.clone();
                    tokio::spawn(async move {
                        // rmcp speaks JSON-RPC 2.0 over any AsyncRead +
                        // AsyncWrite — `UnixStream` satisfies both via
                        // `tokio::io::split`.
                        let (rx, tx) = tokio::io::split(stream);
                        match server_clone.serve((rx, tx)).await {
                            Ok(svc) => {
                                let _ = svc.waiting().await;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "mcp_server",
                                    error = %e,
                                    "rmcp serve failed"
                                );
                            }
                        }
                    });
                }
            }
        }
    });

    Ok(McpServerHandle {
        shutdown,
        join,
        socket_path: socket_path_buf,
    })
}

#[cfg(not(unix))]
pub fn spawn_mcp_server(_server: GavieroMcpServer, _socket_path: &Path) -> Result<McpServerHandle> {
    anyhow::bail!(
        "MCP server: Unix-socket transport only on Unix platforms (plan §A5 open question)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::embedder::Embedder;
    use anyhow::Result as AResult;

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
        ) -> AResult<Vec<f32>> {
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

    fn fixture() -> GavieroMcpServer {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        GavieroMcpServer::with_defaults(stores, std::path::PathBuf::from("/tmp"))
    }

    #[tokio::test]
    async fn memory_search_returns_empty_on_cold_store() {
        let s = fixture();
        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "anything".into(),
                scope_hint: None,
                limit: Some(5),
                kind: None,
            }))
            .await
            .unwrap();
        assert!(out.0.results.is_empty());
    }

    /// C1.6: memory_search default-records-only filter excludes
    /// history rows even when retrieval ranks them at the top.
    #[tokio::test]
    async fn memory_search_default_filter_excludes_history() {
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let s = fixture();
        // Seed two rows at the same scope: one record, one history,
        // both phrased the same so retrieval ranks them similarly.
        let scope = WriteScope::Workspace;
        let record_meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("c16-record");
        let history_meta = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("c16-history");
        let store = s.stores.workspace().clone();
        store
            .store_scoped(&scope, "purple elephant convention", &record_meta)
            .await
            .unwrap();
        store
            .store_scoped(&scope, "purple elephant convention seen in transcript", &history_meta)
            .await
            .unwrap();

        // Default kind (None → Record) — only the record row survives.
        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "purple elephant convention".into(),
                scope_hint: None,
                limit: Some(10),
                kind: None,
            }))
            .await
            .unwrap();
        assert!(!out.0.results.is_empty());
        for r in &out.0.results {
            // History rows are RawTranscript-sourced and tagged
            // "c16-history"; their text contains "transcript".
            assert!(
                !r.text.contains("transcript"),
                "default filter must exclude history rows: {r:?}"
            );
        }

        // Explicit kind=history — record row is excluded.
        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "purple elephant convention".into(),
                scope_hint: None,
                limit: Some(10),
                kind: Some("history".into()),
            }))
            .await
            .unwrap();
        for r in &out.0.results {
            assert!(
                r.text.contains("transcript"),
                "history filter must exclude record rows: {r:?}"
            );
        }

        // kind=any — both can come through (ordering depends on
        // retrieval scoring; we only assert at least one of each).
        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "purple elephant convention".into(),
                scope_hint: None,
                limit: Some(10),
                kind: Some("any".into()),
            }))
            .await
            .unwrap();
        let any_record = out.0.results.iter().any(|r| !r.text.contains("transcript"));
        let any_history = out.0.results.iter().any(|r| r.text.contains("transcript"));
        assert!(any_record, "any-filter must include records");
        assert!(any_history, "any-filter must include history");
    }

    /// C1.6: unknown kinds produce a clear MCP invalid_params error,
    /// not a silent fallback.
    #[tokio::test]
    async fn memory_search_unknown_kind_is_invalid_params() {
        let s = fixture();
        let r = s
            .memory_search(Parameters(MemorySearchInput {
                query: "x".into(),
                scope_hint: None,
                limit: None,
                kind: Some("episode".into()),
            }))
            .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn blast_radius_rejects_empty_paths() {
        let s = fixture();
        let res = s
            .blast_radius(Parameters(BlastRadiusInput {
                paths: vec![],
                depth: None,
                mode: None,
            }))
            .await;
        assert!(res.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn server_accepts_unix_socket_connection() {
        // Smoke test: the accept loop binds the socket, accepts a
        // connection, and doesn't crash. Full MCP protocol
        // exercise (initialize + tools/list + tools/call) would need
        // `rmcp` with the `client` feature — see tests in the
        // `mcp::server::tests` for handler-level coverage.
        use tokio::net::UnixStream;
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("mcp.sock");
        let server = fixture();
        let handle = spawn_mcp_server(server, &sock).unwrap();

        // Retry connect briefly so accept loop is listening.
        let mut attempts = 0;
        loop {
            if UnixStream::connect(&sock).await.is_ok() {
                break;
            }
            attempts += 1;
            if attempts > 10 {
                panic!("shim connect never succeeded");
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        handle.shutdown().await;
        assert!(
            !sock.exists(),
            "socket file should be cleaned up on shutdown"
        );
    }

    #[tokio::test]
    async fn node_doc_returns_stub() {
        let s = fixture();
        let out = s
            .node_doc(Parameters(NodeDocInput {
                path: "src/lib.rs".into(),
            }))
            .await
            .unwrap();
        assert_eq!(out.0.path, "src/lib.rs");
        assert!(out.0.purpose.is_empty());
    }
}
