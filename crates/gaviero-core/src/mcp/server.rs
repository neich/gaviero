//! In-process MCP server task (Tier A / A5).
//!
//! Listens on the workspace's [`McpEndpoint`] — a Unix domain socket
//! at `.gaviero/mcp.sock` on Unix, a `\\.\pipe\gaviero-…` named pipe
//! on Windows. Each shim connection is a single MCP session
//! speaking JSON-RPC 2.0 over `AsyncRead + AsyncWrite` — rmcp handles
//! framing, initialize, and tools/list. Gaviero owns only the three
//! tool handlers.
//!
//! **Read-only invariant**: the handler takes an `Arc<MemoryStore>` for
//! search + a `RepoMap`-backed graph accessor for `blast_radius`, but
//! never a `WriterHandle`. There is no code path from here to
//! `store_scoped`. Plan-rejected `memory_store` / `memory_update` /
//! `memory_delete` tools remain unimplementable by construction.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::{Context as _, Result};
use rmcp::ServiceExt;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{ErrorData, tool, tool_router};

use crate::memory::{
    MemoryScope, MemoryStores, RerankConfig, Reranker, RetrievalConfig, retrieve_ranked_with_levels,
};
use crate::repo_map::store::BlastRadiusMode;

use super::observer::{McpCallLogEntry, McpToolCallObserver, NoopMcpObserver};
use super::tools::{
    BlastRadiusInput, BlastRadiusOutput, BlastRadiusRelation, MemoryGetInput, MemoryGetOutput,
    MemoryGetRow, MemorySearchInput, MemorySearchOutput, MemorySearchResult, NodeDoc, NodeDocInput,
    NodeDocSymbol, RepoOutlineEntry, RepoOutlineInput, RepoOutlineOutput,
    SYMBOL_DOC_SNIPPET_MAX_CHARS, SymbolDocImpl, SymbolDocInput, SymbolDocOutput, SymbolSearchHit,
    SymbolSearchInput, SymbolSearchOutput, clamp_blast_depth, clamp_memory_search_limit,
    clamp_repo_outline_token_cap, clamp_symbol_search_limit, truncate_symbol_snippet,
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
    /// Cached in-memory `RepoMap` backing the `repo_outline` tool,
    /// mirroring `graph_cache`'s lifecycle: lazily built on first use,
    /// shared across per-connection clones via `Arc`, and cleared by
    /// [`Self::invalidate_graph_cache`]. Kept separate from
    /// `graph_cache` because the outline renderer / budget admit live
    /// on `RepoMap`, not on the persisted `GraphStore`.
    repo_map_cache: Arc<tokio::sync::Mutex<Option<crate::repo_map::RepoMap>>>,
    /// S2.3: when false, `symbol_search` / `symbol_doc` return a clear
    /// error directing the agent to run `--graph --enrich` first.
    symbol_enrichment_enabled: bool,
    /// G2 / OD-2: embedder for symbol-vector queries — the resolved
    /// `repoMap.embedder.model` (default `jina-code`). `None` =
    /// "inherit" (use the memory embedder). Built lazily on the first
    /// symbol call (model load is blocking and may download) and cached
    /// in `symbol_embedder`, shared across per-connection clones.
    symbol_embedder_name: Option<String>,
    symbol_embedder: Arc<tokio::sync::Mutex<Option<Arc<dyn crate::memory::Embedder>>>>,
    /// Option A: gaviero-level MCP permission policy (`mcp.permissions`),
    /// enforced server-side for this server's own tools. A tool the policy
    /// denies is rejected here regardless of how the calling provider was
    /// launched (authoritative even under Claude
    /// `--dangerously-skip-permissions`). Default (empty) allows all.
    permissions: super::McpPermissions,
    /// PUSH→PULL Phase 0: per-connection "first tool call seen" latch. Each
    /// accepted connection is one MCP session, so [`Self::clone_for_connection`]
    /// gives every connection a fresh latch; the first tool call on it stamps
    /// `first_tool_call_initiated = true` in the telemetry entry. Shared by
    /// `Arc` so any per-request clone rmcp makes still sees one latch per
    /// connection.
    first_tool_call_done: Arc<AtomicBool>,
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
            repo_map_cache: Arc::new(tokio::sync::Mutex::new(None)),
            symbol_enrichment_enabled: false,
            symbol_embedder_name: None,
            symbol_embedder: Arc::new(tokio::sync::Mutex::new(None)),
            permissions: super::McpPermissions::default(),
            first_tool_call_done: Arc::new(AtomicBool::new(false)),
            tool_router: Self::tool_router(),
        }
    }

    /// Clone the server for a freshly accepted connection, giving it its own
    /// first-tool-call latch while sharing the warm caches (`graph_cache`,
    /// reranker) via `Arc`. Each shim connection is one MCP session, so the
    /// latch is per-session rather than process-global.
    fn clone_for_connection(&self) -> Self {
        let mut s = self.clone();
        s.first_tool_call_done = Arc::new(AtomicBool::new(false));
        s
    }

    /// Build and dispatch one tool-call telemetry entry, stamping the
    /// per-connection first-call latch. `session_id` / `turn` are not yet
    /// wired (the shim passes no session identity), so they ride as `None`.
    fn emit_tool_call(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        output: serde_json::Value,
        started: Instant,
        error: Option<String>,
    ) {
        let first = !self.first_tool_call_done.swap(true, Ordering::Relaxed);
        self.observer.on_tool_call(&McpCallLogEntry {
            tool_name: tool_name.to_string(),
            input,
            output,
            duration: started.elapsed(),
            error,
            first_tool_call_initiated: first,
            session_id: None,
            turn: None,
        });
    }

    /// Install the gaviero-level MCP permission policy. Tools this policy
    /// denies are rejected server-side, the authoritative enforcement point
    /// for gaviero's own tools (the client-config translation in
    /// [`super::synthesize_for_worktree`] is best-effort by comparison).
    pub fn with_permissions(mut self, permissions: super::McpPermissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// Reject a call to one of this server's tools when the permission
    /// policy denies it (server name is always `gaviero`). Mirrors the
    /// `symbol_enrichment_enabled` gate: the tool stays listed but a denied
    /// call returns a clear error instead of running.
    fn ensure_tool_allowed(&self, tool: &str) -> Result<(), ErrorData> {
        if self.permissions.tool_allowed("gaviero", tool) {
            Ok(())
        } else {
            Err(ErrorData::invalid_request(
                format!("gaviero MCP tool {tool:?} is disabled by mcp.permissions"),
                None,
            ))
        }
    }

    /// Enable symbol MCP tools (`symbol_search`, `symbol_doc`). Requires a
    /// populated `symbol_docs` sidecar from `gaviero-cli --graph --enrich`.
    pub fn with_symbol_enrichment(mut self, enabled: bool) -> Self {
        self.symbol_enrichment_enabled = enabled;
        self
    }

    /// G2 / OD-2: set the symbol-vector query embedder — the resolved
    /// `repoMap.embedder.model` setting. Pass `None` for `"inherit"`
    /// (memory embedder). The sidecar's `symbol_embedder` stamp is
    /// checked per call, so a mismatched sidecar errors with a
    /// re-enrich remedy instead of returning cross-model cosine noise.
    pub fn with_symbol_embedder_name(mut self, name: Option<String>) -> Self {
        self.symbol_embedder_name = name;
        self
    }

    /// Resolve the embedder used for symbol-vector queries: the
    /// configured name when set (built lazily, cached, shared across
    /// connection clones), else the memory embedder.
    async fn resolve_symbol_embedder(&self) -> Result<Arc<dyn crate::memory::Embedder>, ErrorData> {
        let Some(name) = &self.symbol_embedder_name else {
            return Ok(self.stores.embedder().clone());
        };
        // Async mutex held across the one-time blocking build on
        // purpose: concurrent first calls must not race two model
        // loads (each can be a download).
        let mut guard = self.symbol_embedder.lock().await;
        if let Some(e) = guard.as_ref() {
            return Ok(e.clone());
        }
        let name_owned = name.clone();
        let built =
            tokio::task::spawn_blocking(move || crate::memory::build_embedder_by_name(&name_owned))
                .await
                .map_err(|e| ErrorData::internal_error(format!("symbol embedder join: {e}"), None))?
                .map_err(|e| {
                    ErrorData::internal_error(
                        format!("loading symbol embedder `{name}` (repoMap.embedder.model): {e}"),
                        None,
                    )
                })?;
        *guard = Some(built.clone());
        Ok(built)
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

    /// Drop the cached `GraphStore` and `RepoMap` so the next
    /// `blast_radius` / `repo_outline` call rebuilds them from the
    /// current workspace state. Embedding apps (TUI / CLI) should call
    /// this after a bulk workspace change (large checkout, large file
    /// deletion) — small per-file edits don't require it because the
    /// next builder run is incremental.
    pub async fn invalidate_graph_cache(&self) {
        let mut guard = self.graph_cache.lock().await;
        *guard = None;
        drop(guard);
        let mut rm = self.repo_map_cache.lock().await;
        *rm = None;
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
        let repo_map_cache = Arc::clone(&self.repo_map_cache);
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
            drop(guard);
            // Also pre-build the `repo_outline` RepoMap so a mid-run
            // outline pull never pays the cold workspace scan.
            let mut rm = repo_map_cache.blocking_lock();
            if rm.is_none() {
                match crate::repo_map::RepoMap::build(&workspace_root, &[]) {
                    Ok(map) => *rm = Some(map),
                    Err(e) => tracing::warn!(
                        target: "mcp_server",
                        error = %e,
                        "repo map warmup build failed"
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
        self.ensure_tool_allowed("memory_search")?;
        let limit = clamp_memory_search_limit(input.limit);
        // C1.6: resolve the kind filter. Default is `record`; `any`
        // disables filtering; explicit kinds filter to that one
        // class. Unknown values are a loud error so subprocess agents
        // see the contract violation instead of silently falling back.
        let kind_filter = super::tools::resolve_memory_search_kind(input.kind.as_deref())
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        // DRIFT-2: `scope_hint` is a *restriction* over the levels the
        // MCP context can reach ([Workspace, Global] — no folder/run
        // identity crosses the shim). Folder/run hints are a loud
        // error rather than a silent widen; unknown values likewise.
        let level_restriction = match input.scope_hint.as_deref() {
            None => None,
            Some("workspace") => Some(crate::memory::scope::SCOPE_WORKSPACE),
            Some("global") => Some(crate::memory::scope::SCOPE_GLOBAL),
            Some(other @ ("repo" | "module" | "run")) => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_search.scope_hint: {other:?} needs folder/run context that the \
                         MCP surface does not carry; reachable scopes are 'workspace' and \
                         'global' (omit the hint to merge both)"
                    ),
                    None,
                ));
            }
            Some(other) => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_search.scope_hint: unknown value {other:?}; expected \
                         'workspace' | 'global', or omit to merge both"
                    ),
                    None,
                ));
            }
        };
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
        let out = retrieve_ranked_with_levels(
            &self.stores,
            &scope,
            &input.query,
            fetch_limit,
            &self.retrieval_cfg,
            reranker_ref,
            Some(&self.rerank_cfg),
            level_restriction,
        )
        .await
        .map_err(|e| ErrorData::internal_error(format!("retrieve_ranked: {e}"), None))?;

        // C1.6 + BUG-1: post-filter the candidate list by memory_kind.
        // The candidate mix spans physical DBs with independent rowid
        // spaces (workspace + global today), so each row's kind lookup
        // must be routed to the store that owns it via its persisted
        // (scope_level, repo_id) — a fixed-store lookup would miss
        // global ids or, worse, read a colliding workspace row's kind.
        // Rows whose kind cannot be resolved are dropped when a filter
        // is active (forgive only on `any`); a bad row degrades that
        // row, never the whole call.
        let mut results: Vec<MemorySearchResult> = Vec::with_capacity(limit);
        let mut warned_unresolvable = false;
        for m in out.items.iter() {
            if results.len() >= limit {
                break;
            }
            if let Some(want) = kind_filter {
                let owning_store = match crate::memory::scope::store_kind_for_scope(
                    m.scope_level,
                    m.repo_id.as_deref(),
                ) {
                    Some(kind) => self.stores.get(&kind).await.ok(),
                    None => None,
                };
                let got = match owning_store {
                    Some(store) => store.get_memory_kind(m.id).await.ok().flatten(),
                    None => {
                        if !warned_unresolvable {
                            warned_unresolvable = true;
                            tracing::warn!(
                                target: "mcp_server",
                                memory_id = m.id,
                                scope_level = m.scope_level,
                                "memory_search kind filter: owning store unresolvable; dropping row(s)"
                            );
                        }
                        None
                    }
                };
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

        self.emit_tool_call(
            super::tools::TOOL_MEMORY_SEARCH,
            serde_json::to_value(&input).unwrap_or_default(),
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
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
        self.ensure_tool_allowed("blast_radius")?;
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
                let (store, _) = crate::repo_map::graph_builder::build_graph(&workspace_root, &[])?;
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
                qualified_name: path.clone(),
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
                path: path.clone(),
                qualified_name: path,
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
                qualified_name: path.clone(),
                relation: "test_gap".to_string(),
                distance: 1,
                purpose: None,
                score: Some(score),
                specificity: Some(sp),
            });
        }

        let out = BlastRadiusOutput { nodes };
        self.emit_tool_call(
            super::tools::TOOL_BLAST_RADIUS,
            serde_json::to_value(&input).unwrap_or_default(),
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
        Ok(Json(out))
    }

    // ── node_doc ────────────────────────────────────────────────────
    #[tool(
        name = "node_doc",
        description = "Call this when you need one file's symbol signatures. Returns \
                       {path, qualified_name, symbols[{qualified_name, signature, doc_snippet?}], \
                       signatures}. Use `qualified_name` to chain into `symbol_doc`. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn node_doc(
        &self,
        Parameters(input): Parameters<NodeDocInput>,
    ) -> Result<Json<NodeDoc>, ErrorData> {
        let started = Instant::now();
        self.ensure_tool_allowed("node_doc")?;
        let path = input.path.clone();
        let qualified_name = path.clone();
        let log_input = serde_json::to_value(&input).unwrap_or_default();
        let cache = Arc::clone(&self.graph_cache);
        let workspace_root = self.workspace_root.clone();
        let path_for_graph = path.clone();

        let (symbols, signatures) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                let (store, _) = crate::repo_map::graph_builder::build_graph(&workspace_root, &[])?;
                *guard = Some(store);
            }
            let store = guard.as_ref().expect("graph cache populated");
            let nodes = store.nodes_for_file(&path_for_graph)?;
            let mut symbols = Vec::new();
            let mut signatures = Vec::new();
            for node in nodes {
                if node.kind == "File" {
                    continue;
                }
                if let Some(doc) = store.symbol_doc(&node.qualified_name)? {
                    let snippet = if doc.doc.is_empty() {
                        None
                    } else {
                        Some(truncate_symbol_snippet(
                            &doc.doc,
                            SYMBOL_DOC_SNIPPET_MAX_CHARS,
                        ))
                    };
                    signatures.push(doc.signature.clone());
                    symbols.push(NodeDocSymbol {
                        qualified_name: node.qualified_name.clone(),
                        signature: doc.signature,
                        doc_snippet: snippet,
                    });
                } else {
                    signatures.push(node.name.clone());
                    symbols.push(NodeDocSymbol {
                        qualified_name: node.qualified_name.clone(),
                        signature: node.name.clone(),
                        doc_snippet: None,
                    });
                }
            }
            Ok((symbols, signatures))
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("node_doc join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("node_doc: {e}"), None))?;

        let out = NodeDoc {
            qualified_name,
            path,
            signatures,
            symbols,
            purpose: String::new(),
            summary: String::new(),
        };
        self.emit_tool_call(
            super::tools::TOOL_NODE_DOC,
            log_input,
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
        Ok(Json(out))
    }

    // ── memory_get ──────────────────────────────────────────────────
    #[tool(
        name = "memory_get",
        description = "Fetch the full stored row behind one memory_search hit — pass the \
                       hit's `id` and `scope`. Returns text, kind, type, importance, trust, \
                       timestamps, tag, and access stats; a miss is an empty result, not an \
                       error. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn memory_get(
        &self,
        Parameters(input): Parameters<MemoryGetInput>,
    ) -> Result<Json<MemoryGetOutput>, ErrorData> {
        let started = Instant::now();
        self.ensure_tool_allowed("memory_get")?;
        // `scope` names the owning *store* (ids are only unique per
        // physical DB — the BUG-1 lesson). Run rows live in the
        // workspace store; folder scopes need a repo_id the MCP
        // surface does not carry, so they error loudly (PR-2
        // precedent).
        let store = match input.scope.trim() {
            "global" => self.stores.global(),
            "workspace" | "run" => self.stores.workspace(),
            other @ ("repo" | "module") => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_get.scope: {other:?} rows live in per-folder DBs keyed by a \
                         repo_id the MCP surface does not carry; reachable scopes are \
                         'global', 'workspace', and 'run'"
                    ),
                    None,
                ));
            }
            other => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_get.scope: unknown value {other:?}; expected 'global' | \
                         'workspace' | 'run' (as returned by memory_search)"
                    ),
                    None,
                ));
            }
        };
        let row = store
            .get_memory_row(input.id)
            .await
            .map_err(|e| ErrorData::internal_error(format!("memory_get: {e}"), None))?;
        let out = MemoryGetOutput {
            memory: row.map(|r| MemoryGetRow {
                id: r.id,
                scope: format_scope(r.scope_level),
                kind: r.kind.as_str().to_string(),
                memory_type: r.memory_type.as_str().to_string(),
                text: r.content,
                importance: r.importance,
                trust: r.trust_score,
                created_at: r.created_at,
                updated_at: r.updated_at,
                tag: r.tag,
                access_count: r.access_count,
                accessed_at: r.accessed_at,
            }),
        };
        self.emit_tool_call(
            super::tools::TOOL_MEMORY_GET,
            serde_json::to_value(&input).unwrap_or_default(),
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
        Ok(Json(out))
    }

    // ── repo_outline ────────────────────────────────────────────────
    #[tool(
        name = "repo_outline",
        description = "Call this to pull the PageRank-ranked code outline mid-run — the same \
                       ranked view injected as <repo_outline> on turn 1 — when you need repo \
                       orientation without reading files. Optional seed_paths focus the \
                       ranking on an area; token_cap bounds the output (default 2000, max \
                       8000); mode weights edges like blast_radius. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn repo_outline(
        &self,
        Parameters(input): Parameters<RepoOutlineInput>,
    ) -> Result<Json<RepoOutlineOutput>, ErrorData> {
        let started = Instant::now();
        self.ensure_tool_allowed("repo_outline")?;
        let budget = clamp_repo_outline_token_cap(input.token_cap) as usize;
        let mode = BlastRadiusMode::from_str(input.mode.as_deref().unwrap_or("all"));
        // Empty / omitted seeds → "." which the ranker treats as
        // match-all (every file is owned; whole-workspace outline).
        let seeds: Vec<String> = match &input.seed_paths {
            Some(v) if !v.is_empty() => v.clone(),
            _ => vec![".".to_string()],
        };
        let cache = Arc::clone(&self.repo_map_cache);
        let workspace_root = self.workspace_root.clone();

        let candidates = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                // No exclude source exists server-side (excludes are a
                // host/TUI/CLI concern); mirror the graph_cache build's
                // empty exclude set so both caches see the same tree.
                *guard = Some(crate::repo_map::RepoMap::build(&workspace_root, &[])?);
            }
            let map = guard.as_ref().expect("repo map cache populated above");
            Ok(map.rank_for_agent_structured_with_mode(&seeds, budget, mode))
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("repo_outline join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("repo_outline: {e}"), None))?;

        let mut token_estimate: u32 = 0;
        let entries: Vec<RepoOutlineEntry> = candidates
            .iter()
            .map(|c| {
                token_estimate = token_estimate.saturating_add(c.token_estimate as u32);
                RepoOutlineEntry {
                    path: c.path.to_string_lossy().to_string(),
                    rank_score: c.rank_score,
                    specificity: c.specificity,
                    rendered: c.rendered_line.clone(),
                }
            })
            .collect();

        let out = RepoOutlineOutput {
            entries,
            token_estimate,
        };
        self.emit_tool_call(
            super::tools::TOOL_REPO_OUTLINE,
            serde_json::to_value(&input).unwrap_or_default(),
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
        Ok(Json(out))
    }

    // ── symbol_search ───────────────────────────────────────────────
    #[tool(
        name = "symbol_search",
        description = "Semantic search over enriched Rust symbols (signatures + docs). \
                       Returns {results: [{qualified_name, file_path, signature, score, \
                       doc_snippet?}]} — chain `qualified_name` into `symbol_doc`. Requires \
                       `repoMap.symbolEnrichment.enabled` and `gaviero-cli --graph --enrich`. \
                       Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn symbol_search(
        &self,
        Parameters(input): Parameters<SymbolSearchInput>,
    ) -> Result<Json<SymbolSearchOutput>, ErrorData> {
        let started = Instant::now();
        self.ensure_tool_allowed("symbol_search")?;
        if !self.symbol_enrichment_enabled {
            return Err(symbol_tools_disabled_error());
        }
        if input.query.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "symbol_search requires a non-empty query",
                None,
            ));
        }
        let limit = clamp_symbol_search_limit(input.limit);
        let query_embedder = self.resolve_symbol_embedder().await?;
        let query_emb = query_embedder
            .embed_query(&input.query)
            .await
            .map_err(|e| ErrorData::internal_error(format!("symbol_search embed: {e}"), None))?;
        let query_name = query_embedder.name().to_string();
        let memory_name = self.stores.embedder().name().to_string();

        let cache = Arc::clone(&self.graph_cache);
        let workspace_root = self.workspace_root.clone();
        let hits = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                let (store, _) = crate::repo_map::graph_builder::build_graph(&workspace_root, &[])?;
                *guard = Some(store);
            }
            let store = guard.as_ref().expect("graph cache populated");
            // G2 / OD-2: cross-model cosine is noise — verify the
            // sidecar's vectors were built by the query embedder.
            let stamp = store.graph_meta("symbol_embedder")?;
            crate::repo_map::symbol_search::check_symbol_embedder_stamp(
                stamp.as_deref(),
                &query_name,
                &memory_name,
            )?;
            crate::repo_map::symbol_search::search_symbol_docs(store, &query_emb, limit)
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("symbol_search join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("symbol_search: {e}"), None))?;

        let results: Vec<SymbolSearchHit> = hits
            .into_iter()
            .map(|hit| {
                let snippet = if hit.doc.doc.is_empty() {
                    None
                } else {
                    Some(truncate_symbol_snippet(
                        &hit.doc.doc,
                        SYMBOL_DOC_SNIPPET_MAX_CHARS,
                    ))
                };
                SymbolSearchHit {
                    qualified_name: hit.doc.qualified_name,
                    file_path: hit.doc.file_path,
                    signature: hit.doc.signature,
                    score: hit.score,
                    doc_snippet: snippet,
                }
            })
            .collect();
        let out = SymbolSearchOutput { results };
        self.emit_tool_call(
            super::tools::TOOL_SYMBOL_SEARCH,
            serde_json::to_value(&input).unwrap_or_default(),
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
        Ok(Json(out))
    }

    // ── symbol_doc ──────────────────────────────────────────────────
    #[tool(
        name = "symbol_doc",
        description = "Full symbol enrichment for one `qualified_name` from `symbol_search` \
                       or `node_doc`. Returns signature, bounds, doc, role_summary, and trait \
                       `implementations` when applicable. Read-only.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn symbol_doc(
        &self,
        Parameters(input): Parameters<SymbolDocInput>,
    ) -> Result<Json<SymbolDocOutput>, ErrorData> {
        let started = Instant::now();
        self.ensure_tool_allowed("symbol_doc")?;
        if !self.symbol_enrichment_enabled {
            return Err(symbol_tools_disabled_error());
        }
        if input.qualified_name.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "symbol_doc requires qualified_name",
                None,
            ));
        }

        let qn = input.qualified_name.clone();
        let cache = Arc::clone(&self.graph_cache);
        let workspace_root = self.workspace_root.clone();
        let out = tokio::task::spawn_blocking(move || -> anyhow::Result<SymbolDocOutput> {
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                let (store, _) = crate::repo_map::graph_builder::build_graph(&workspace_root, &[])?;
                *guard = Some(store);
            }
            let store = guard.as_ref().expect("graph cache populated");
            let Some(doc) = store.symbol_doc(&qn)? else {
                anyhow::bail!("no symbol_docs row for qualified_name `{qn}`");
            };
            let impl_qns = store.implementation_qns_for_trait(&qn)?;
            let mut implementations = Vec::new();
            for impl_qn in impl_qns {
                if let Some(impl_doc) = store.symbol_doc(&impl_qn)? {
                    let snippet = if impl_doc.doc.is_empty() {
                        None
                    } else {
                        Some(truncate_symbol_snippet(
                            &impl_doc.doc,
                            SYMBOL_DOC_SNIPPET_MAX_CHARS,
                        ))
                    };
                    implementations.push(SymbolDocImpl {
                        qualified_name: impl_qn,
                        signature: impl_doc.signature,
                        doc_snippet: snippet,
                    });
                }
            }
            Ok(SymbolDocOutput {
                qualified_name: doc.qualified_name,
                file_path: doc.file_path,
                signature: doc.signature,
                bounds: doc.bounds,
                doc: doc.doc,
                role_summary: doc.role_summary,
                implementations,
            })
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("symbol_doc join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("symbol_doc: {e}"), None))?;

        self.emit_tool_call(
            super::tools::TOOL_SYMBOL_DOC,
            serde_json::to_value(&input).unwrap_or_default(),
            serde_json::to_value(&out).unwrap_or_default(),
            started,
            None,
        );
        Ok(Json(out))
    }
}

fn symbol_tools_disabled_error() -> ErrorData {
    ErrorData::invalid_params(
        "symbol_search/symbol_doc are disabled: repoMap.symbolEnrichment.enabled was \
         false when this MCP server started (the flag is read once at startup). Set it \
         to true in .gaviero/settings.json and restart gaviero, then run \
         `gaviero-cli --graph --enrich` once to populate the symbol_docs sidecar.",
        None,
    )
}

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

// ── Accept loop (Unix socket / Windows named pipe) ────────────────

/// Handle returned by `spawn_mcp_server` — lets the caller signal
/// shutdown when the workspace closes.
pub struct McpServerHandle {
    shutdown: tokio::sync::broadcast::Sender<()>,
    join: tokio::task::JoinHandle<()>,
    pub endpoint: super::McpEndpoint,
}

impl McpServerHandle {
    /// Signal the accept loop to stop and await its exit. Idempotent.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.join.await;
        // Best-effort socket-file cleanup. Named pipes vanish with the
        // last open handle — nothing to remove on the pipe arm.
        if let super::McpEndpoint::Unix(path) = &self.endpoint {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Serve one accepted connection on its own task: per-connection
/// clone (fresh first-tool-call latch, shared warm caches), rmcp over
/// split `AsyncRead + AsyncWrite` halves.
fn spawn_connection<S>(server: &GavieroMcpServer, stream: S)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
{
    let server_clone = server.clone_for_connection();
    tokio::spawn(async move {
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

/// Spawn the MCP server accept loop on the workspace endpoint —
/// Unix domain socket on Unix, named pipe on Windows.
pub fn spawn_mcp_server(
    server: GavieroMcpServer,
    endpoint: &super::McpEndpoint,
) -> Result<McpServerHandle> {
    match endpoint {
        super::McpEndpoint::Unix(path) => spawn_unix(server, path.clone()),
        super::McpEndpoint::Pipe(name) => spawn_pipe(server, name.clone()),
    }
}

#[cfg(unix)]
fn spawn_unix(server: GavieroMcpServer, socket_path: PathBuf) -> Result<McpServerHandle> {
    use tokio::net::UnixListener;

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // Remove any stale socket from a previous run.
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding MCP socket at {}", socket_path.display()))?;
    let (shutdown, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let socket_path_accept = socket_path.clone();

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
                    spawn_connection(&server, stream);
                }
            }
        }
    });

    Ok(McpServerHandle {
        shutdown,
        join,
        endpoint: super::McpEndpoint::Unix(socket_path),
    })
}

#[cfg(not(unix))]
fn spawn_unix(_server: GavieroMcpServer, socket_path: PathBuf) -> Result<McpServerHandle> {
    anyhow::bail!(
        "MCP server: Unix-socket endpoint {} is not supported on this platform \
         (use a named-pipe endpoint)",
        socket_path.display()
    )
}

/// Windows: multi-instance named-pipe accept loop. A fresh pipe
/// instance must exist *before* the connected one is handed off, or a
/// fast second client hits `ERROR_FILE_NOT_FOUND` between accepts —
/// mirrors tokio's documented server pattern. The first instance sets
/// `first_pipe_instance(true)` so another process can't squat the
/// name; the default DACL (current user) is the ACL policy.
#[cfg(windows)]
fn spawn_pipe(server: GavieroMcpServer, pipe_name: String) -> Result<McpServerHandle> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut instance = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&pipe_name)
        .with_context(|| format!("creating MCP named pipe {pipe_name}"))?;
    let (shutdown, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let pipe_name_accept = pipe_name.clone();

    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    tracing::info!(
                        target: "mcp_server",
                        pipe = %pipe_name_accept,
                        "shutdown signal received"
                    );
                    break;
                }
                connected = instance.connect() => {
                    if let Err(e) = connected {
                        tracing::warn!(target: "mcp_server", error = %e, "pipe connect failed");
                        continue;
                    }
                    // Next instance first, then hand off the connected one.
                    let next = match ServerOptions::new().create(&pipe_name_accept) {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::warn!(
                                target: "mcp_server",
                                error = %e,
                                "creating next pipe instance failed — accept loop stopping"
                            );
                            spawn_connection(&server, instance);
                            break;
                        }
                    };
                    let stream = std::mem::replace(&mut instance, next);
                    spawn_connection(&server, stream);
                }
            }
        }
    });

    Ok(McpServerHandle {
        shutdown,
        join,
        endpoint: super::McpEndpoint::Pipe(pipe_name),
    })
}

#[cfg(not(windows))]
fn spawn_pipe(_server: GavieroMcpServer, pipe_name: String) -> Result<McpServerHandle> {
    anyhow::bail!("MCP server: named-pipe endpoint {pipe_name} is only supported on Windows")
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
            .store_scoped(
                &scope,
                "purple elephant convention seen in transcript",
                &history_meta,
            )
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

    /// BUG-1 fixture: distinct global and workspace in-memory DBs with
    /// independent rowid spaces (no single-store aliasing).
    fn split_fixture() -> GavieroMcpServer {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_split_in_memory(embedder).unwrap();
        GavieroMcpServer::with_defaults(stores, std::path::PathBuf::from("/tmp"))
    }

    /// BUG-1: a Record living only in the global DB must survive the
    /// default kind filter. Pre-fix, the kind lookup was hard-coded to
    /// the workspace DB, missed the global id, and dropped the row.
    #[tokio::test]
    async fn memory_search_default_filter_sees_global_records_across_stores() {
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let s = split_fixture();
        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("bug1-global-record");
        s.stores
            .global()
            .store_scoped(&WriteScope::Global, "orange giraffe convention", &meta)
            .await
            .unwrap();

        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "orange giraffe convention".into(),
                scope_hint: None,
                limit: Some(10),
                kind: None,
            }))
            .await
            .unwrap();
        assert!(
            out.0
                .results
                .iter()
                .any(|r| r.text.contains("orange giraffe") && r.scope == "global"),
            "global record must survive the default kind filter: {:?}",
            out.0.results
        );
    }

    /// BUG-1: id collision across DBs. Global id N is a Record while
    /// workspace id N is a History row; the default filter must route
    /// each row's kind lookup to its own store. Pre-fix, the global
    /// record's kind resolved via the colliding workspace row
    /// (History) and the record was wrongly dropped.
    #[tokio::test]
    async fn memory_search_kind_filter_routes_lookup_to_owning_store_on_id_collision() {
        use crate::memory::kind::MemoryKind;
        use crate::memory::scope::{MemoryType, StoreResult, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let s = split_fixture();
        let record_meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("bug1-collision-record");
        let history_meta = WriteMeta::for_source(MemorySource::RawTranscript)
            .with_kind(MemoryKind::History)
            .with_type(MemoryType::Factual)
            .with_tag("bug1-collision-history");
        let gid = match s
            .stores
            .global()
            .store_scoped(
                &WriteScope::Global,
                "purple elephant convention",
                &record_meta,
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) => id,
            other => panic!("expected fresh insert, got {other:?}"),
        };
        let wid = match s
            .stores
            .workspace()
            .store_scoped(
                &WriteScope::Workspace,
                "purple elephant convention seen in transcript",
                &history_meta,
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) => id,
            other => panic!("expected fresh insert, got {other:?}"),
        };
        // Precondition: the ids collide across the two DBs, and each
        // DB reports a different kind for that id.
        assert_eq!(
            gid, wid,
            "fresh in-memory DBs must hand out the same first id"
        );
        assert_eq!(
            s.stores.global().get_memory_kind(gid).await.unwrap(),
            Some(MemoryKind::Record)
        );
        assert_eq!(
            s.stores.workspace().get_memory_kind(wid).await.unwrap(),
            Some(MemoryKind::History)
        );

        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "purple elephant convention".into(),
                scope_hint: None,
                limit: Some(10),
                kind: None,
            }))
            .await
            .unwrap();
        assert!(
            out.0
                .results
                .iter()
                .any(|r| r.scope == "global" && !r.text.contains("transcript")),
            "global record must be returned despite the id collision: {:?}",
            out.0.results
        );
        // The workspace History row stays excluded under the default filter.
        assert!(
            out.0.results.iter().all(|r| !r.text.contains("transcript")),
            "history row must not leak through the default filter: {:?}",
            out.0.results
        );
    }

    /// DRIFT-2 (scope_hint): restriction semantics over the reachable
    /// levels. `"global"` returns only global rows, `"workspace"` only
    /// workspace rows, omitted merges both; folder/run hints and
    /// unknown values are loud invalid_params errors.
    #[tokio::test]
    async fn memory_search_scope_hint_restricts_levels() {
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let s = split_fixture();
        let meta = |tag: &str| {
            WriteMeta::for_source(MemorySource::UserRemember)
                .with_type(MemoryType::Decision)
                .with_tag(tag)
        };
        s.stores
            .global()
            .store_scoped(
                &WriteScope::Global,
                "quantum banana global fact",
                &meta("hint-global"),
            )
            .await
            .unwrap();
        s.stores
            .workspace()
            .store_scoped(
                &WriteScope::Workspace,
                "quantum banana workspace fact",
                &meta("hint-workspace"),
            )
            .await
            .unwrap();

        let search = |hint: Option<&str>| {
            let hint = hint.map(String::from);
            let s = &s;
            async move {
                s.memory_search(Parameters(MemorySearchInput {
                    query: "quantum banana fact".into(),
                    scope_hint: hint,
                    limit: Some(10),
                    kind: None,
                }))
                .await
            }
        };

        let both = search(None).await.unwrap().0.results;
        assert!(both.iter().any(|r| r.scope == "global"), "{both:?}");
        assert!(both.iter().any(|r| r.scope == "workspace"), "{both:?}");

        let only_global = search(Some("global")).await.unwrap().0.results;
        assert!(!only_global.is_empty());
        assert!(
            only_global.iter().all(|r| r.scope == "global"),
            "{only_global:?}"
        );

        let only_ws = search(Some("workspace")).await.unwrap().0.results;
        assert!(!only_ws.is_empty());
        assert!(
            only_ws.iter().all(|r| r.scope == "workspace"),
            "{only_ws:?}"
        );

        for bad in ["repo", "module", "run", "solar"] {
            match search(Some(bad)).await {
                Err(err) => {
                    assert!(err.message.contains("scope_hint"), "{bad}: {}", err.message)
                }
                Ok(_) => panic!("scope_hint {bad:?} must error"),
            }
        }
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
        let handle =
            spawn_mcp_server(server, &super::super::McpEndpoint::Unix(sock.clone())).unwrap();

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

    #[cfg(windows)]
    #[tokio::test]
    async fn server_accepts_named_pipe_connections() {
        // Windows mirror of the unix accept smoke test, plus a second
        // concurrent client to exercise the multi-instance re-create
        // step in the accept loop.
        use tokio::net::windows::named_pipe::ClientOptions;
        let pipe_name = format!(r"\\.\pipe\gaviero-test-{}", std::process::id());
        let server = fixture();
        let handle =
            spawn_mcp_server(server, &super::super::McpEndpoint::Pipe(pipe_name.clone())).unwrap();

        let mut clients = Vec::new();
        for _ in 0..2 {
            let mut attempts = 0;
            loop {
                match ClientOptions::new().open(&pipe_name) {
                    Ok(c) => {
                        clients.push(c);
                        break;
                    }
                    Err(_) => {
                        attempts += 1;
                        if attempts > 10 {
                            panic!("pipe connect never succeeded");
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    }
                }
            }
        }

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn node_doc_includes_qualified_name() {
        let s = fixture();
        let out = s
            .node_doc(Parameters(NodeDocInput {
                path: "src/lib.rs".into(),
            }))
            .await
            .unwrap();
        assert_eq!(out.0.path, "src/lib.rs");
        assert_eq!(out.0.qualified_name, "src/lib.rs");
    }

    /// PR-4: `memory_get` routes by the scope string to the owning
    /// store (disambiguating colliding ids), returns the full row,
    /// treats misses as empty results, and rejects folder scopes.
    #[tokio::test]
    async fn memory_get_returns_row_per_scope_and_empty_on_miss() {
        use crate::memory::scope::{MemoryType, StoreResult, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let s = split_fixture();
        let meta = |tag: &str| {
            WriteMeta::for_source(MemorySource::UserRemember)
                .with_type(MemoryType::Decision)
                .with_tag(tag)
        };
        let gid = match s
            .stores
            .global()
            .store_scoped(
                &WriteScope::Global,
                "global memo about lighthouses",
                &meta("get-global"),
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) => id,
            other => panic!("expected insert, got {other:?}"),
        };
        let wid = match s
            .stores
            .workspace()
            .store_scoped(
                &WriteScope::Workspace,
                "workspace memo about beacons",
                &meta("get-workspace"),
            )
            .await
            .unwrap()
        {
            StoreResult::Inserted(id) => id,
            other => panic!("expected insert, got {other:?}"),
        };
        // Fresh in-memory DBs: the ids collide, which is exactly why
        // the scope parameter exists.
        assert_eq!(gid, wid);

        let get = |id: i64, scope: &str| {
            let scope = scope.to_string();
            let s = &s;
            async move { s.memory_get(Parameters(MemoryGetInput { id, scope })).await }
        };

        let row = get(gid, "global")
            .await
            .unwrap()
            .0
            .memory
            .expect("global row");
        assert_eq!(row.id, gid);
        assert_eq!(row.scope, "global");
        assert_eq!(row.kind, "record");
        assert!(row.text.contains("lighthouses"), "{row:?}");
        assert_eq!(row.tag.as_deref(), Some("get-global"));

        let row = get(wid, "workspace")
            .await
            .unwrap()
            .0
            .memory
            .expect("workspace row");
        assert_eq!(row.scope, "workspace");
        assert!(row.text.contains("beacons"), "{row:?}");

        // Miss (id that exists in neither store) → empty result.
        let out = get(gid + 999, "workspace").await.unwrap();
        assert!(out.0.memory.is_none());

        // Folder scopes and unknown values → loud invalid_params.
        for bad in ["repo", "module", "cosmic"] {
            match get(gid, bad).await {
                Err(err) => assert!(
                    err.message.contains("memory_get.scope"),
                    "{bad}: {}",
                    err.message
                ),
                Ok(_) => panic!("scope {bad:?} must error"),
            }
        }
    }

    /// PR-3: `repo_outline` returns ranked entries from a real
    /// workspace scan, respects the token cap, and parses modes.
    #[tokio::test]
    async fn repo_outline_returns_budgeted_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("lib.rs"),
            "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("util.rs"),
            "pub struct Widget;\npub fn gamma() {}\n",
        )
        .unwrap();
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let s = GavieroMcpServer::with_defaults(stores, dir.path().to_path_buf());

        let out = s
            .repo_outline(Parameters(RepoOutlineInput {
                seed_paths: None,
                token_cap: Some(150),
                mode: Some("callers".into()),
            }))
            .await
            .unwrap();
        assert!(!out.0.entries.is_empty(), "expected outline entries");
        assert!(
            out.0.token_estimate <= 150,
            "budget exceeded: {}",
            out.0.token_estimate
        );
        for e in &out.0.entries {
            assert!(!e.rendered.is_empty());
            assert!(!e.path.is_empty());
        }
    }

    /// PR-3: the permission policy denies `repo_outline` server-side
    /// like any other gaviero tool.
    #[tokio::test]
    async fn repo_outline_denied_by_permission_policy() {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let s = GavieroMcpServer::with_defaults(stores, std::path::PathBuf::from("/tmp"))
            .with_permissions(super::super::McpPermissions {
                allow: vec![],
                deny: vec!["gaviero:repo_outline".into()],
            });
        match s
            .repo_outline(Parameters(RepoOutlineInput::default()))
            .await
        {
            Err(err) => assert!(
                err.message.contains("mcp.permissions"),
                "unexpected error: {}",
                err.message
            ),
            Ok(_) => panic!("expected repo_outline to be denied by policy"),
        }
    }

    #[tokio::test]
    async fn symbol_search_disabled_without_enrichment_flag() {
        let s = fixture();
        match s
            .symbol_search(Parameters(SymbolSearchInput {
                query: "handler".into(),
                limit: None,
            }))
            .await
        {
            Err(err) => assert!(err.message.contains("symbolEnrichment")),
            Ok(_) => panic!("expected symbol_search to fail without enrichment"),
        }
    }

    #[tokio::test]
    async fn permission_policy_rejects_denied_tool_server_side() {
        // A per-tool deny is enforced here regardless of how the calling
        // provider was launched — the authoritative gate for gaviero tools.
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let s = GavieroMcpServer::with_defaults(stores, std::path::PathBuf::from("/tmp"))
            .with_permissions(super::super::McpPermissions {
                allow: vec![],
                deny: vec!["gaviero:blast_radius".into()],
            });

        // Denied tool is rejected with a clear message.
        match s
            .blast_radius(Parameters(BlastRadiusInput {
                paths: vec!["src/lib.rs".into()],
                mode: None,
                depth: None,
            }))
            .await
        {
            Err(err) => assert!(
                err.message.contains("mcp.permissions"),
                "unexpected error: {}",
                err.message
            ),
            Ok(_) => panic!("expected blast_radius to be denied by policy"),
        }

        // A sibling tool not named in the deny list still runs.
        s.memory_search(Parameters(MemorySearchInput {
            query: "anything".into(),
            scope_hint: None,
            limit: None,
            kind: None,
        }))
        .await
        .expect("memory_search must remain allowed");
    }
}
