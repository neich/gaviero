//! In-process MCP server task (Tier A / A5).
//!
//! Listens on the workspace's [`McpEndpoint`] — a Unix domain socket
//! at `.gaviero/mcp.sock` on Unix, a `\\.\pipe\gaviero-…` named pipe
//! on Windows. Each shim connection is a single MCP session
//! speaking JSON-RPC 2.0 over `AsyncRead + AsyncWrite` — rmcp handles
//! framing, initialize, and tools/list. Gaviero owns only the nine
//! tool handlers below.
//!
//! **The invariant this module enforces is #11: every write goes
//! through the Tier S2 writer task.** The enforcement at this boundary
//! is that the server holds `Arc<MemoryStores>` for search and a
//! `RepoMap`-backed graph accessor for `blast_radius`, but never a
//! [`crate::memory::WriterHandle`] — there is no code path from here to
//! `store_scoped`. Rejected `memory_store` / `memory_update` /
//! `memory_delete` tools remain unimplementable by construction.
//!
//! Eight of the nine tools are strictly read-only. `memory_ping` writes
//! only to an in-memory [`super::probe::ProbeLedger`]. `memory_flag` is
//! write-adjacent: it emits a signal through
//! [`super::signal::MemorySignalSink`] — a narrow trait object, not the
//! writer handle — which the writer task turns into a trust demotion
//! plus an audit row. It creates and deletes nothing. See
//! `mcp/mod.rs`'s header for why read-only is a default posture rather
//! than a hard constraint.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use rmcp::ServiceExt;
use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{CallToolRequestParams, ListToolsResult};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};

use crate::memory::{
    MemoryScope, MemoryStores, RerankConfig, Reranker, RetrievalConfig, retrieve_ranked_with_levels,
};
use crate::repo_map::store::BlastRadiusMode;

use super::observer::{McpCallLogEntry, McpToolCallObserver, NoopMcpObserver};
use super::probe::{PingRecord, ProbeLedger, ping_receipt};
use super::signal::{MemoryFlagRequest, MemorySignalSink};
use super::tools::{
    BlastRadiusInput, BlastRadiusOutput, BlastRadiusRelation, MemoryFlagInput, MemoryFlagOutput,
    MemoryGetInput, MemoryGetOutput, MemoryGetRow, MemoryPingInput, MemoryPingOutput,
    MemorySearchInput, MemorySearchOutput, MemorySearchResult, NodeDoc, NodeDocInput,
    NodeDocSymbol, RepoOutlineEntry, RepoOutlineInput, RepoOutlineOutput,
    SYMBOL_DOC_SNIPPET_MAX_CHARS, SymbolDocImpl, SymbolDocInput, SymbolDocOutput, SymbolSearchHit,
    SymbolSearchInput, SymbolSearchOutput, clamp_blast_depth, clamp_blast_limit,
    clamp_memory_search_limit, one_based_line,
    clamp_repo_outline_token_cap, clamp_symbol_search_limit, truncate_symbol_snippet,
};

/// Gaviero's MCP server. One instance lives per workspace; it
/// dispatches tool calls to the nine handlers below (eight read-only
/// plus write-adjacent `memory_flag`).
///
/// `tool_router` is the rmcp-macro-generated dispatch table — see
/// `rmcp::handler::server::router::tool` for the shape.
#[derive(Clone)]
pub struct GavieroMcpServer {
    stores: Arc<MemoryStores>,
    workspace_root: PathBuf,
    /// Folder whose repo scope `memory_search` reaches. Defaults to
    /// `workspace_root`; differs when the host serves a run root below the
    /// folder that owns the workspace memory ([`Self::with_memory_root`]).
    memory_root: PathBuf,
    /// Code graph database for `workspace_root`. Defaults to
    /// `<workspace_root>/.gaviero/code_graph.db` ([`Self::with_graph_db_path`]).
    graph_db_path: PathBuf,
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
    /// first graph tool (or [`Self::warmup`]) so we don't re-run
    /// `graph_builder::build_graph` (a workspace-wide scan + tree-sitter
    /// parse) on every invocation. The cached connection is a *build
    /// sentinel* only: query handlers open a fresh WAL connection via
    /// [`with_graph_store`] so `blast_radius` PageRank cannot serialize
    /// `node_doc` / `symbol_*` for minutes. Call
    /// [`Self::invalidate_graph_cache`] from the embedding app after a
    /// large workspace change to force the next call to rescan.
    ///
    /// `GraphStore` wraps a `rusqlite::Connection` which is `Send` but
    /// not `Sync`, so the sentinel lives behind a `Mutex`.
    graph_cache: Arc<tokio::sync::Mutex<Option<crate::repo_map::store::GraphStore>>>,
    /// Cached in-memory `RepoMap` backing the `repo_outline` tool,
    /// mirroring `graph_cache`'s lifecycle: lazily built on first use,
    /// shared across per-connection clones via `Arc`, and cleared by
    /// [`Self::invalidate_graph_cache`]. Stored as `Arc<RepoMap>` so
    /// ranking can run without holding the mutex (same convoy as
    /// `blast_radius` / `node_doc`).
    repo_map_cache: Arc<tokio::sync::Mutex<Option<Arc<crate::repo_map::RepoMap>>>>,
    /// Extra exclude patterns (from `files.exclude`) applied when this
    /// process first builds the graph. Scratch dir names in
    /// `repo_map::builder::SKIP_DIRS` always apply; this list is the
    /// host's additional skip set.
    graph_excludes: Vec<String>,
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
    /// The one seam through which a tool call can cause a write: the
    /// `memory_flag` signal sink. Deliberately an
    /// `Arc<dyn MemorySignalSink>` and *not* a `WriterHandle` — the
    /// server cannot construct a write, pick a scope, or reach a store
    /// through it. `None` (the default, and the state of any server built
    /// without [`Self::with_signal_sink`]) makes `memory_flag` return a
    /// clear error rather than silently no-op.
    signal_sink: Option<Arc<dyn MemorySignalSink>>,
    /// Optional in-memory ping ledger for `memory_ping`. Unattached
    /// servers still answer the tool; they just do not record.
    probe_ledger: Option<ProbeLedger>,
    /// Instant the server struct was built. Mixed into ping receipts so
    /// two probe runs cannot collide on the same nonce.
    started_at: chrono::DateTime<chrono::Utc>,
    /// `mcp.gavieroServer.exposedTools`. `None` lists the full router
    /// surface (tests / callers that skip the setting). `Some` is the
    /// allow-list; `memory_ping` is always listed unless permissions deny it.
    exposed_tools: Option<Vec<String>>,
    /// Bound on one `blast_radius` graph query ([`GRAPH_QUERY_TIMEOUT`]).
    graph_query_timeout: Duration,
    /// Test hooks: a pause inside the blast query before it starts work,
    /// and a count of blast queries that ran to completion.
    #[cfg(test)]
    blast_test_delay: Duration,
    #[cfg(test)]
    blast_completed: Arc<std::sync::atomic::AtomicUsize>,
    #[allow(dead_code)] // populated and dispatched via the `#[tool_router]` macro
    tool_router: ToolRouter<Self>,
}

fn strip_schema_descriptions(schema: &mut serde_json::Value) {
    if let Some(object) = schema.as_object_mut() {
        object.remove("description");
        object.remove("title");
        for (key, value) in object {
            if matches!(key.as_str(), "properties" | "$defs" | "definitions") {
                if let Some(properties) = value.as_object_mut() {
                    for property in properties.values_mut() {
                        strip_schema_descriptions(property);
                    }
                }
            } else {
                strip_schema_descriptions(value);
            }
        }
    } else if let Some(array) = schema.as_array_mut() {
        for value in array {
            strip_schema_descriptions(value);
        }
    }
}

/// Bound for `blast_radius` so a stuck PageRank cannot hold the chat
/// spinner for the 10–20 minutes observed in `mcp_calls.ndjson`.
const GRAPH_QUERY_TIMEOUT: Duration = Duration::from_secs(12);

// ── repo_outline payload budget ──────────────────────────────────
//
// The cap bounds the whole compact JSON response, estimated as
// `ceil(bytes / 4)` (`super::payload`). Working in bytes: the response is
// `{"entries":[E1,E2,…],"token_estimate":N}`, so it costs the empty
// envelope plus each entry plus one comma per entry after the first. The
// ranker charges every entry `bytes(entry) + 1`, over-counting one comma,
// against `4·cap − bytes(envelope with N = cap)`. `token_estimate` never
// exceeds `cap`, so reserving `cap`'s digits is enough, and the total stays
// ≤ `4·cap − 1` bytes, i.e. ≤ `cap` estimated tokens.

/// Compact bytes of `{"entries":[],"token_estimate":<cap>}`.
fn outline_envelope_bytes(cap: u32) -> usize {
    super::payload::compact_json_len(&RepoOutlineOutput {
        entries: Vec::new(),
        token_estimate: cap,
    })
    .unwrap_or(usize::MAX)
}

/// Byte budget the ranker may spend on entries for `cap`.
fn outline_entry_budget_bytes(cap: u32) -> usize {
    (cap as usize * 4).saturating_sub(outline_envelope_bytes(cap))
}

/// Compact bytes of one `{"rendered":"…"}` entry plus its comma,
/// JSON escaping included.
fn outline_entry_cost_bytes(line: &str) -> usize {
    super::payload::compact_json_len(&RepoOutlineEntry {
        rendered: line.to_string(),
    })
    .map_or(usize::MAX, |b| b + 1)
}

/// Build the response with a self-consistent `token_estimate`: the
/// estimate of the payload that contains it. Adding digits can only grow
/// the estimate, so the fixpoint is reached in a few steps.
fn finish_outline(entries: Vec<RepoOutlineEntry>) -> RepoOutlineOutput {
    let mut out = RepoOutlineOutput {
        entries,
        token_estimate: 0,
    };
    for _ in 0..8 {
        let bytes = super::payload::compact_json_len(&out).unwrap_or(0);
        let estimate = super::payload::estimated_tokens(bytes) as u32;
        if estimate == out.token_estimate {
            break;
        }
        out.token_estimate = estimate;
    }
    out
}

/// Round a score for output, after ranking. Non-finite values have no
/// JSON form and are omitted.
fn round_score(x: f64) -> Option<f64> {
    x.is_finite().then(|| (x * 1000.0).round() / 1000.0)
}

/// Shape an impact query into the `blast_radius` response.
///
/// Changed rows come first (duplicate seeds collapsed, input order kept),
/// then at most `limit` dependents ranked by full-precision score
/// descending with path ascending as the tie-break (the impact query has
/// no `ORDER BY`, so ties would otherwise follow SQL row order), then the
/// test gaps. Scores are rounded only after ranking; NaN ranks last.
/// `not_shown` counts the dependents the limit dropped, never seed or
/// test-gap rows.
fn format_blast_radius(
    impact: &crate::repo_map::store::ImpactSummary,
    ranks: &std::collections::HashMap<String, (f64, f64)>,
    mode: BlastRadiusMode,
    limit: usize,
) -> BlastRadiusOutput {
    let lookup = |p: &str| ranks.get(p).copied().unwrap_or((0.0, 1.0));
    let row = |path: &str, relation: Option<&str>| {
        let (score, sp) = lookup(path);
        BlastRadiusRelation {
            path: path.to_string(),
            relation: relation.map(str::to_string),
            purpose: None,
            score: round_score(score),
            specificity: round_score(sp),
        }
    };

    let mut seen = std::collections::HashSet::new();
    let seeds: Vec<&str> = impact
        .changed_files
        .iter()
        .map(String::as_str)
        .filter(|p| seen.insert(*p))
        .collect();

    let mut dependents: Vec<(&str, f64)> = impact
        .affected_files
        .iter()
        .map(String::as_str)
        .filter(|p| seen.insert(*p))
        .map(|p| {
            let score = lookup(p).0;
            (p, if score.is_nan() { f64::NEG_INFINITY } else { score })
        })
        .collect();
    dependents.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let not_shown = dependents.len().saturating_sub(limit);
    dependents.truncate(limit);

    let mut gaps_seen = std::collections::HashSet::new();
    let mut nodes: Vec<BlastRadiusRelation> = seeds.iter().map(|p| row(p, Some("changed"))).collect();
    nodes.extend(dependents.iter().map(|(p, _)| row(p, None)));
    nodes.extend(
        impact
            .test_gaps
            .iter()
            .filter(|p| gaps_seen.insert(p.as_str()))
            .map(|p| row(p, Some("test_gap"))),
    );

    BlastRadiusOutput {
        mode: mode.as_str().to_string(),
        nodes,
        not_shown,
        truncated: not_shown > 0,
    }
}

/// Ensure the on-disk graph exists (first call in this process builds
/// it under `graph_cache`), then open a *fresh* WAL connection for `f`.
///
/// Do not run `f` while holding `graph_cache`: `blast_radius` PageRank
/// used to serialize every other graph tool for minutes.
fn with_graph_store<T, F>(
    cache: &tokio::sync::Mutex<Option<crate::repo_map::store::GraphStore>>,
    workspace_root: &Path,
    graph_db: &Path,
    excludes: &[String],
    f: F,
) -> anyhow::Result<T>
where
    F: FnOnce(&crate::repo_map::store::GraphStore) -> anyhow::Result<T>,
{
    {
        let mut guard = cache.blocking_lock();
        if guard.is_none() {
            let (store, _) =
                crate::repo_map::graph_builder::build_graph_at(workspace_root, graph_db, excludes)?;
            *guard = Some(store);
        }
    }
    let store = crate::repo_map::store::GraphStore::open(graph_db)?;
    f(&store)
}

fn with_repo_map<T, F>(
    cache: &tokio::sync::Mutex<Option<Arc<crate::repo_map::RepoMap>>>,
    workspace_root: &Path,
    excludes: &[String],
    f: F,
) -> anyhow::Result<T>
where
    F: FnOnce(&crate::repo_map::RepoMap) -> anyhow::Result<T>,
{
    let map = {
        let mut guard = cache.blocking_lock();
        if guard.is_none() {
            let built = crate::repo_map::RepoMap::build(workspace_root, excludes)?;
            *guard = Some(Arc::new(built));
        }
        Arc::clone(guard.as_ref().expect("repo map populated above"))
    };
    f(&map)
}

#[tool_router]
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
            memory_root: workspace_root.clone(),
            graph_db_path: crate::repo_map::graph_builder::graph_db_path(&workspace_root),
            workspace_root,
            observer,
            retrieval_cfg,
            rerank_cfg,
            reranker,
            specificity: crate::repo_map::SpecificityConfig::default(),
            edge_weights: std::collections::HashMap::new(),
            graph_cache: Arc::new(tokio::sync::Mutex::new(None)),
            repo_map_cache: Arc::new(tokio::sync::Mutex::new(None)),
            graph_excludes: Vec::new(),
            symbol_enrichment_enabled: false,
            symbol_embedder_name: None,
            symbol_embedder: Arc::new(tokio::sync::Mutex::new(None)),
            permissions: super::McpPermissions::default(),
            first_tool_call_done: Arc::new(AtomicBool::new(false)),
            signal_sink: None,
            probe_ledger: None,
            started_at: chrono::Utc::now(),
            exposed_tools: None,
            graph_query_timeout: GRAPH_QUERY_TIMEOUT,
            #[cfg(test)]
            blast_test_delay: Duration::ZERO,
            #[cfg(test)]
            blast_completed: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            tool_router: Self::tool_router(),
        }
    }

    /// Override the `blast_radius` query bound (default 12 s).
    pub fn with_graph_query_timeout(mut self, timeout: Duration) -> Self {
        self.graph_query_timeout = timeout;
        self
    }

    /// Clone the server for a freshly accepted connection, giving it its own
    /// first-tool-call latch while sharing the warm caches (`graph_cache`,
    /// reranker) via `Arc`. Each shim connection is one MCP session, so the
    /// latch is per-session rather than process-global.
    pub(crate) fn clone_for_connection(&self) -> Self {
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

    /// Install the gaviero-level MCP permission policy. Denied tools are
    /// omitted from `tools/list` and rejected on call.
    pub fn with_permissions(mut self, permissions: super::McpPermissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// Restrict `tools/list` to this allow-list. `memory_ping` is still
    /// listed even when omitted here. `mcp.permissions` denies hide tools
    /// from the list as well.
    pub fn with_exposed_tools(mut self, tools: Vec<String>) -> Self {
        self.exposed_tools = Some(tools);
        self
    }

    /// Names currently advertised on `tools/list`.
    pub fn listed_tool_names(&self) -> Vec<String> {
        self.listed_tools()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
    }

    fn listed_tools(&self) -> Vec<rmcp::model::Tool> {
        self.tool_router
            .list_all()
            .into_iter()
            .filter(|t| self.tool_is_listed(t.name.as_ref()))
            .map(|mut tool| {
                let mut schema = serde_json::Value::Object((*tool.input_schema).clone());
                strip_schema_descriptions(&mut schema);
                tool.input_schema = Arc::new(schema.as_object().expect("schema object").clone());
                if let Some(output) = &tool.output_schema {
                    let mut schema = serde_json::Value::Object((**output).clone());
                    strip_schema_descriptions(&mut schema);
                    tool.output_schema =
                        Some(Arc::new(schema.as_object().expect("schema object").clone()));
                }
                tool
            })
            .collect()
    }

    fn tool_is_listed(&self, tool: &str) -> bool {
        if !self.symbol_enrichment_enabled && matches!(tool, "symbol_search" | "symbol_doc") {
            return false;
        }
        if !self.permissions.tool_allowed("gaviero", tool) {
            return false;
        }
        if tool == super::tools::TOOL_MEMORY_PING {
            return true;
        }
        match &self.exposed_tools {
            None => true,
            Some(list) => list.iter().any(|t| t == tool),
        }
    }

    /// Reject a call when the permission policy, the exposed-tool
    /// allow-list, or the enrichment gate hides the tool.
    fn ensure_tool_allowed(&self, tool: &str) -> Result<(), ErrorData> {
        if self.tool_is_listed(tool) {
            Ok(())
        } else {
            Err(ErrorData::invalid_request(
                format!(
                    "gaviero MCP tool {tool:?} is disabled (mcp.permissions, \
                     mcp.gavieroServer.exposedTools, or repoMap.symbolEnrichment.enabled)"
                ),
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

    /// Install the `memory_flag` signal sink. Without it the tool stays
    /// listed but every call errors — the same "listed but gated" shape
    /// as `symbol_enrichment_enabled`, never a silent no-op.
    ///
    /// Post-construction builder so `new`'s signature stays stable across
    /// its three call sites.
    pub fn with_signal_sink(mut self, sink: Arc<dyn MemorySignalSink>) -> Self {
        self.signal_sink = Some(sink);
        self
    }

    /// Attach a [`ProbeLedger`] so `memory_ping` records nonce + depth.
    /// Without it the tool still returns a receipt (needed for nested
    /// sessions that inherit a server that was not started as a probe).
    pub fn with_probe_ledger(mut self, ledger: ProbeLedger) -> Self {
        self.probe_ledger = Some(ledger);
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

    /// Extra `files.exclude` patterns for the first graph / repo-map build.
    /// Scratch names in `repo_map::builder::SKIP_DIRS` always apply.
    pub fn with_graph_excludes(mut self, excludes: Vec<String>) -> Self {
        self.graph_excludes = excludes;
        self
    }

    /// Reach repo-scoped memory through `root` instead of the served
    /// `workspace_root`. For hosts serving a run root below the folder
    /// that owns the workspace memory; `root` must be a registered folder
    /// for repo scope to be searched.
    pub fn with_memory_root(mut self, root: PathBuf) -> Self {
        self.memory_root = root;
        self
    }

    /// Persist the code graph at `path` instead of
    /// `<workspace_root>/.gaviero/code_graph.db`. The file must hold no
    /// graph of a different scan root.
    pub fn with_graph_db_path(mut self, path: PathBuf) -> Self {
        self.graph_db_path = path;
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
        let graph_db = self.graph_db_path.clone();
        let excludes = self.graph_excludes.clone();
        // build_graph is blocking + potentially heavy on a large repo;
        // run it off the async runtime and hold the cache lock only for
        // this build (matching the `blast_radius` pattern).
        let _ = tokio::task::spawn_blocking(move || {
            let mut guard = cache.blocking_lock();
            if guard.is_none() {
                match crate::repo_map::graph_builder::build_graph_at(
                    &workspace_root,
                    &graph_db,
                    &excludes,
                ) {
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
                match crate::repo_map::RepoMap::build(&workspace_root, &excludes) {
                    Ok(map) => *rm = Some(Arc::new(map)),
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
        description = "Hybrid search over Gaviero memories (repo + workspace + global). Read-only. Token cost: ~50-150 per result.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn memory_search(
        &self,
        Parameters(input): Parameters<MemorySearchInput>,
    ) -> Result<Json<MemorySearchOutput>, ErrorData> {
        self.memory_search_impl(input).await
    }

    /// In-process core of [`Self::memory_search`]. See
    /// [`Self::call_tool_in_process`] for why the `#[tool]` wrapper is split off.
    async fn memory_search_impl(
        &self,
        input: MemorySearchInput,
    ) -> Result<Json<MemorySearchOutput>, ErrorData> {
        self.ensure_tool_allowed("memory_search")?;
        let limit = clamp_memory_search_limit(input.limit);
        // C1.6: resolve the kind filter. Default is `record`; `any`
        // disables filtering; explicit kinds filter to that one
        // class. Unknown values are a loud error so subprocess agents
        // see the contract violation instead of silently falling back.
        let kind_filter = super::tools::resolve_memory_search_kind(input.kind.as_deref())
            .map_err(|e| ErrorData::invalid_params(e, None))?;
        // DRIFT-2: `scope_hint` is a *restriction* over the levels the
        // MCP context can reach. `repo` is reachable because the server
        // knows its own `workspace_root` (see the folder resolution
        // below); `module` and `run` still need per-file / per-run
        // identity that does not cross the shim, so they stay a loud
        // error rather than a silent widen. Unknown values likewise.
        let level_restriction = match input.scope_hint.as_deref() {
            None => None,
            Some("workspace") => Some(crate::memory::scope::SCOPE_WORKSPACE),
            Some("global") => Some(crate::memory::scope::SCOPE_GLOBAL),
            Some("repo") => Some(crate::memory::scope::SCOPE_REPO),
            Some(other @ ("module" | "run")) => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_search.scope_hint: {other:?} needs per-file / per-run context \
                         that the MCP surface does not carry; reachable scopes are 'repo', \
                         'workspace' and 'global' (omit the hint to merge all three)"
                    ),
                    None,
                ));
            }
            Some(other) => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_search.scope_hint: unknown value {other:?}; expected \
                         'repo' | 'workspace' | 'global', or omit to merge all three"
                    ),
                    None,
                ));
            }
        };

        // Repo scope is where the bulk of a mature workspace's memory
        // lives, so the folder identity is supplied from the server's own
        // `memory_root` rather than left as `None` — otherwise
        // `MemoryScope::levels()` emits only [Workspace, Global] and every
        // repo-scoped memory is invisible to subprocess agents.
        //
        // Guarded, not unconditional: `multi_scope_retrieve` propagates
        // `MemoryStores::get`'s "unknown repo_id — not registered in
        // workspace" error, so handing it a folder the registry does not
        // know would turn every search into a hard failure. Registries
        // without the root registered (and `single_store_fallback` off)
        // therefore keep the old [Workspace, Global] behaviour.
        //
        // `module` is deliberately still out of reach: it derives from
        // `owned_paths`, which is per-file context the shim does not carry.
        let repo_folder = {
            let repo_id = crate::memory::hash_path(&self.memory_root);
            match self
                .stores
                .get(&crate::memory::StoreKind::Folder { repo_id })
                .await
            {
                Ok(_) => Some(self.memory_root.as_path()),
                Err(_) => None,
            }
        };
        let scope = MemoryScope::from_context(&self.memory_root, repo_folder, None, None);
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
        Ok(Json(MemorySearchOutput { results }))
    }

    // ── blast_radius ────────────────────────────────────────────────
    #[tool(
        name = "blast_radius",
        description = "Ranked dependents (impact, callers, tests) of file paths, best first; limit rows (default 40), not_shown counts the rest. Read-only. Token cost: ~20-30 per row.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn blast_radius(
        &self,
        Parameters(input): Parameters<BlastRadiusInput>,
    ) -> Result<Json<BlastRadiusOutput>, ErrorData> {
        self.blast_radius_impl(input).await
    }

    /// In-process core of [`Self::blast_radius`].
    async fn blast_radius_impl(
        &self,
        input: BlastRadiusInput,
    ) -> Result<Json<BlastRadiusOutput>, ErrorData> {
        self.ensure_tool_allowed("blast_radius")?;
        if input.paths.is_empty() {
            return Err(ErrorData::invalid_params(
                "blast_radius requires at least one path",
                None,
            ));
        }
        let depth = clamp_blast_depth(input.depth);
        let mode = BlastRadiusMode::parse(input.mode.as_deref().unwrap_or("all"));
        let paths = input.paths.clone();
        let workspace_root = self.workspace_root.clone();
        let graph_db = self.graph_db_path.clone();
        let specificity = self.specificity;
        let weights = self
            .edge_weights
            .get(&mode)
            .copied()
            .unwrap_or_else(|| crate::repo_map::store::EdgeWeights::default_for(mode));
        let cache = Arc::clone(&self.graph_cache);
        let excludes = self.graph_excludes.clone();

        // Build (if needed) under the sentinel mutex, then rank on a
        // fresh connection so this call cannot block `node_doc`. On timeout
        // the query is cancelled, not merely abandoned: the in-flight SQL
        // statement is interrupted and ranking stops at its next check.
        // (A first-call graph *build* is not cancelled: it is cached work.)
        let cancel = crate::repo_map::store::GraphCancel::new();
        let job = Arc::clone(&cancel);
        #[cfg(test)]
        let (test_delay, completed) = (self.blast_test_delay, Arc::clone(&self.blast_completed));
        let compute = tokio::task::spawn_blocking(move || {
            with_graph_store(&cache, &workspace_root, &graph_db, &excludes, |store| {
                job.attach(store)?;
                #[cfg(test)]
                std::thread::sleep(test_delay);
                job.check()?;
                let seed_refs: Vec<&str> = paths.iter().map(String::as_str).collect();
                let impact = store.impact_radius_with_mode(&seed_refs, depth as usize, mode)?;
                job.check()?;
                let mut to_rank: Vec<String> = impact.changed_files.to_vec();
                for f in &impact.affected_files {
                    if !to_rank.contains(f) {
                        to_rank.push(f.clone());
                    }
                }
                let ranks = crate::repo_map::rank_files_with_weights_cancellable(
                    store,
                    &seed_refs,
                    &to_rank,
                    weights,
                    specificity,
                    &|| job.is_cancelled(),
                )?;
                #[cfg(test)]
                completed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok((impact, ranks))
            })
        });
        let (impact, ranks) = match tokio::time::timeout(self.graph_query_timeout, compute).await
        {
            Ok(join) => join
                .map_err(|e| ErrorData::internal_error(format!("blast_radius join: {e}"), None))?
                .map_err(|e| ErrorData::internal_error(format!("blast_radius: {e}"), None))?,
            Err(_) => {
                cancel.cancel();
                return Err(ErrorData::internal_error(
                    format!(
                        "blast_radius timed out after {}s. Retry with a single source path.",
                        self.graph_query_timeout.as_secs()
                    ),
                    None,
                ));
            }
        };

        Ok(Json(format_blast_radius(
            &impact,
            &ranks,
            mode,
            clamp_blast_limit(input.limit),
        )))
    }

    // ── node_doc ────────────────────────────────────────────────────
    #[tool(
        name = "node_doc",
        description = "One file's indexed symbols with 1-based lines; chain qualified_name into symbol_doc. Read-only. Token cost: ~15-25 per symbol.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn node_doc(
        &self,
        Parameters(input): Parameters<NodeDocInput>,
    ) -> Result<Json<NodeDoc>, ErrorData> {
        self.node_doc_impl(input).await
    }

    /// In-process core of [`Self::node_doc`].
    async fn node_doc_impl(&self, input: NodeDocInput) -> Result<Json<NodeDoc>, ErrorData> {
        self.ensure_tool_allowed("node_doc")?;
        let path = input.path.clone();
        let qualified_name = path.clone();
        let cache = Arc::clone(&self.graph_cache);
        let workspace_root = self.workspace_root.clone();
        let graph_db = self.graph_db_path.clone();
        let excludes = self.graph_excludes.clone();
        let path_for_graph = path.clone();

        let (symbols, empty_status) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            with_graph_store(&cache, &workspace_root, &graph_db, &excludes, |store| {
                let nodes = store.nodes_for_file(&path_for_graph)?;
                let has_file_node = nodes.iter().any(|n| n.kind == "File");
                let mut symbols = Vec::new();
                for node in nodes {
                    if node.kind == "File" {
                        continue;
                    }
                    let line = one_based_line(node.line_start);
                    // Without a `symbol_docs` row the only "signature" is the
                    // name, which `qualified_name` already ends with. An
                    // enriched row whose signature is just the name is
                    // omitted by the same rule; anything else is kept verbatim.
                    let (signature, doc_snippet) = match store.symbol_doc(&node.qualified_name)? {
                        Some(doc) => {
                            let snippet = (!doc.doc.is_empty()).then(|| {
                                truncate_symbol_snippet(&doc.doc, SYMBOL_DOC_SNIPPET_MAX_CHARS)
                            });
                            ((doc.signature != node.name).then_some(doc.signature), snippet)
                        }
                        None => (None, None),
                    };
                    symbols.push(NodeDocSymbol {
                        qualified_name: node.qualified_name,
                        signature,
                        line,
                        doc_snippet,
                    });
                }
                // An empty result says why, from the persisted extraction
                // status only: no parse on demand, no guess from disk.
                let empty_status = if symbols.is_empty() {
                    let outcome = store.file_extraction(&path_for_graph)?.map(|r| r.outcome);
                    Some(super::tools::node_doc_empty_status(
                        outcome.as_deref(),
                        has_file_node,
                    ))
                } else {
                    None
                };
                Ok((symbols, empty_status))
            })
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("node_doc join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("node_doc: {e}"), None))?;

        let (status, note) = empty_status.unzip();
        Ok(Json(NodeDoc {
            qualified_name,
            path,
            symbols,
            purpose: String::new(),
            summary: String::new(),
            status,
            note,
        }))
    }

    // ── memory_get ──────────────────────────────────────────────────
    #[tool(
        name = "memory_get",
        description = "Fetch the full stored row for one memory_search hit by id and scope. Read-only. Token cost: ~80-200.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn memory_get(
        &self,
        Parameters(input): Parameters<MemoryGetInput>,
    ) -> Result<Json<MemoryGetOutput>, ErrorData> {
        self.memory_get_impl(input).await
    }

    /// In-process core of [`Self::memory_get`].
    async fn memory_get_impl(
        &self,
        input: MemoryGetInput,
    ) -> Result<Json<MemoryGetOutput>, ErrorData> {
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
        Ok(Json(out))
    }

    // ── memory_ping ─────────────────────────────────────────────────
    #[tool(
        name = "memory_ping",
        description = "Reach-probe ping; records nonce and depth on an in-memory ledger. Token cost: ~20.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn memory_ping(
        &self,
        Parameters(input): Parameters<MemoryPingInput>,
    ) -> Result<Json<MemoryPingOutput>, ErrorData> {
        self.memory_ping_impl(input).await
    }

    /// In-process core of [`Self::memory_ping`].
    async fn memory_ping_impl(
        &self,
        input: MemoryPingInput,
    ) -> Result<Json<MemoryPingOutput>, ErrorData> {
        self.ensure_tool_allowed("memory_ping")?;
        let receipt = ping_receipt(&input.nonce, input.depth, self.started_at);
        let workspace_id = crate::workspace::identity::workspace_id_hex16(&self.workspace_root);
        if let Some(ledger) = &self.probe_ledger {
            ledger.record(PingRecord {
                nonce: input.nonce.clone(),
                depth: input.depth,
                at: chrono::Utc::now(),
                receipt: receipt.clone(),
            });
        }
        Ok(Json(MemoryPingOutput {
            receipt,
            depth: input.depth,
            workspace_id,
        }))
    }

    // ── memory_flag ─────────────────────────────────────────────────
    #[tool(
        name = "memory_flag",
        description = "Demote a stale memory's trust by id and scope; never deletes. Token cost: ~40.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn memory_flag(
        &self,
        Parameters(input): Parameters<MemoryFlagInput>,
    ) -> Result<Json<MemoryFlagOutput>, ErrorData> {
        self.memory_flag_impl(input).await
    }

    /// In-process core of [`Self::memory_flag`].
    async fn memory_flag_impl(
        &self,
        input: MemoryFlagInput,
    ) -> Result<Json<MemoryFlagOutput>, ErrorData> {
        self.ensure_tool_allowed("memory_flag")?;

        let Some(sink) = self.signal_sink.as_ref() else {
            return Err(ErrorData::invalid_request(
                "memory_flag is not wired on this server (no signal sink); \
                 enable it with mcp.flag.enabled and restart the workspace",
                None,
            ));
        };

        // Same routing rule as `memory_get`: `scope` names the owning
        // *store*, because ids are only unique per physical DB.
        let (scope_level, repo_id) = match input.scope.trim() {
            "global" => (crate::memory::scope::SCOPE_GLOBAL, None),
            "workspace" => (crate::memory::scope::SCOPE_WORKSPACE, None),
            "run" => (crate::memory::scope::SCOPE_RUN, None),
            other @ ("repo" | "module") => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_flag.scope: {other:?} rows live in per-folder DBs keyed by a \
                         repo_id the MCP surface does not carry; reachable scopes are \
                         'global', 'workspace', and 'run'"
                    ),
                    None,
                ));
            }
            other => {
                return Err(ErrorData::invalid_params(
                    format!(
                        "memory_flag.scope: unknown value {other:?}; expected 'global' | \
                         'workspace' | 'run' (as returned by memory_search)"
                    ),
                    None,
                ));
            }
        };

        let reason = input.reason.trim();
        if reason.is_empty() {
            return Err(ErrorData::invalid_params(
                "memory_flag.reason must not be empty — it is recorded in the \
                 audit row so a human can reverse the demotion",
                None,
            ));
        }

        let outcome = sink
            .flag(MemoryFlagRequest {
                memory_id: input.id,
                scope_level,
                repo_id,
                reason: reason.to_string(),
            })
            .await
            .map_err(|e| ErrorData::internal_error(format!("memory_flag: {e}"), None))?;

        Ok(Json(MemoryFlagOutput {
            accepted: outcome.accepted,
            detail: outcome.detail,
        }))
    }

    // ── repo_outline ────────────────────────────────────────────────
    #[tool(
        name = "repo_outline",
        description = "PageRank-ranked code outline lines for mid-run orientation. Read-only. Token cost: compact JSON payload ≤ token_cap (default 2000).",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn repo_outline(
        &self,
        Parameters(input): Parameters<RepoOutlineInput>,
    ) -> Result<Json<RepoOutlineOutput>, ErrorData> {
        self.repo_outline_impl(input).await
    }

    /// In-process core of [`Self::repo_outline`].
    async fn repo_outline_impl(
        &self,
        input: RepoOutlineInput,
    ) -> Result<Json<RepoOutlineOutput>, ErrorData> {
        self.ensure_tool_allowed("repo_outline")?;
        let cap = clamp_repo_outline_token_cap(input.token_cap);
        let mode = BlastRadiusMode::parse(input.mode.as_deref().unwrap_or("all"));
        // Empty / omitted seeds → "." which the ranker treats as
        // match-all (every file is owned; whole-workspace outline).
        let seeds: Vec<String> = match &input.seed_paths {
            Some(v) if !v.is_empty() => v.clone(),
            _ => vec![".".to_string()],
        };
        let cache = Arc::clone(&self.repo_map_cache);
        let workspace_root = self.workspace_root.clone();
        let excludes = self.graph_excludes.clone();

        let candidates = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            with_repo_map(&cache, &workspace_root, &excludes, |map| {
                Ok(map.rank_with_line_cost(
                    &seeds,
                    outline_entry_budget_bytes(cap),
                    mode,
                    &outline_entry_cost_bytes,
                ))
            })
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("repo_outline join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("repo_outline: {e}"), None))?;

        let entries = candidates
            .into_iter()
            .map(|c| RepoOutlineEntry {
                rendered: c.rendered_line,
            })
            .collect();
        let out = finish_outline(entries);
        debug_assert!(out.token_estimate <= cap, "{} > {cap}", out.token_estimate);
        Ok(Json(out))
    }

    // ── symbol_search ───────────────────────────────────────────────
    #[tool(
        name = "symbol_search",
        description = "Semantic search over enriched symbols. Read-only. Token cost: ~40-100 per hit.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn symbol_search(
        &self,
        Parameters(input): Parameters<SymbolSearchInput>,
    ) -> Result<Json<SymbolSearchOutput>, ErrorData> {
        self.symbol_search_impl(input).await
    }

    /// In-process core of [`Self::symbol_search`].
    async fn symbol_search_impl(
        &self,
        input: SymbolSearchInput,
    ) -> Result<Json<SymbolSearchOutput>, ErrorData> {
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
        let graph_db = self.graph_db_path.clone();
        let excludes = self.graph_excludes.clone();
        let hits = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            with_graph_store(&cache, &workspace_root, &graph_db, &excludes, |store| {
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
        Ok(Json(SymbolSearchOutput { results }))
    }

    // ── symbol_doc ──────────────────────────────────────────────────
    #[tool(
        name = "symbol_doc",
        description = "Full enrichment for one qualified_name from symbol_search or node_doc. Read-only. Token cost: ~80-200.",
        annotations(read_only_hint = true, idempotent_hint = true)
    )]
    async fn symbol_doc(
        &self,
        Parameters(input): Parameters<SymbolDocInput>,
    ) -> Result<Json<SymbolDocOutput>, ErrorData> {
        self.symbol_doc_impl(input).await
    }

    /// In-process core of [`Self::symbol_doc`].
    async fn symbol_doc_impl(
        &self,
        input: SymbolDocInput,
    ) -> Result<Json<SymbolDocOutput>, ErrorData> {
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
        let graph_db = self.graph_db_path.clone();
        let excludes = self.graph_excludes.clone();
        let out = tokio::task::spawn_blocking(move || -> anyhow::Result<SymbolDocOutput> {
            with_graph_store(&cache, &workspace_root, &graph_db, &excludes, |store| {
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
        })
        .await
        .map_err(|e| ErrorData::internal_error(format!("symbol_doc join: {e}"), None))?
        .map_err(|e| ErrorData::internal_error(format!("symbol_doc: {e}"), None))?;

        Ok(Json(out))
    }

    // ── in-process (non-MCP) dispatch ───────────────────────────────

    /// Names + schemas of every tool this server currently advertises.
    ///
    /// Same gates as `tools/list` (`mcp.permissions`, `exposedTools`, symbol
    /// enrichment), so an embedding app that runs its own loop sees exactly the
    /// tool surface an MCP client would — including *not* being pointed at
    /// `symbol_search`/`symbol_doc` while the enrichment sidecar is absent.
    pub fn in_process_tool_specs(&self) -> Vec<InProcessToolSpec> {
        self.listed_tools()
            .into_iter()
            .map(|t| InProcessToolSpec {
                name: t.name.to_string(),
                description: t.description.map(|d| d.to_string()).unwrap_or_default(),
                input_schema: serde_json::Value::Object((*t.input_schema).clone()),
            })
            .collect()
    }

    /// Invoke one of this server's tools **without an MCP session**.
    ///
    /// rmcp's `#[tool]` handlers take a [`ToolCallContext`], whose only
    /// constructor requires a live `RequestContext<RoleServer>` — unreachable
    /// for an app that owns the server in-process instead of speaking JSON-RPC
    /// to it. Each handler is therefore a thin wrapper over a `*_impl` core
    /// taking the deserialized input directly, which is precisely the
    /// extraction rmcp's `Parameters<P>` performs (`#[serde(transparent)]` over
    /// `from_value`).
    ///
    /// Gating is unchanged: [`Self::ensure_tool_allowed`] runs here *and* at the
    /// top of every core, so a tool hidden by `mcp.permissions`,
    /// `mcp.gavieroServer.exposedTools`, or the symbol-enrichment flag is
    /// rejected identically over MCP and in-process.
    ///
    /// Errors are flattened to `String` through `ErrorData`'s `Display` so
    /// callers outside this module need no rmcp types.
    ///
    /// This is one of the two observation owners (the other is
    /// `ServerHandler::call_tool`): every invocation that reaches it emits
    /// exactly one telemetry entry — success or failure, including a
    /// rejected or unknown tool and undecodable arguments. The `*_impl`
    /// cores never emit.
    pub async fn call_tool_in_process(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let started = Instant::now();
        let input = args.clone();
        let result = self.dispatch_in_process(name, args).await;
        match &result {
            Ok(output) => self.emit_tool_call(name, input, output.clone(), started, None),
            Err(e) => self.emit_tool_call(
                name,
                input,
                serde_json::Value::Null,
                started,
                Some(e.clone()),
            ),
        }
        result
    }

    async fn dispatch_in_process(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.ensure_tool_allowed(name).map_err(|e| e.to_string())?;
        // Arity + remapping are identical for all nine tools, so the mapping
        // table below is the only per-tool code. `tools_are_dispatchable_...`
        // pins this table against `in_process_tool_specs()`.
        macro_rules! call {
            ($tool:literal, $input:ty, $core:ident) => {{
                let input: $input = serde_json::from_value(args)
                    .map_err(|e| format!("invalid arguments for {}: {e}", $tool))?;
                let Json(out) = self.$core(input).await.map_err(|e| e.to_string())?;
                serde_json::to_value(out)
                    .map_err(|e| format!("{}: serializing output failed: {e}", $tool))
            }};
        }
        match name {
            "memory_search" => call!("memory_search", MemorySearchInput, memory_search_impl),
            "blast_radius" => call!("blast_radius", BlastRadiusInput, blast_radius_impl),
            "node_doc" => call!("node_doc", NodeDocInput, node_doc_impl),
            "memory_get" => call!("memory_get", MemoryGetInput, memory_get_impl),
            "memory_ping" => call!("memory_ping", MemoryPingInput, memory_ping_impl),
            "memory_flag" => call!("memory_flag", MemoryFlagInput, memory_flag_impl),
            "repo_outline" => call!("repo_outline", RepoOutlineInput, repo_outline_impl),
            "symbol_search" => call!("symbol_search", SymbolSearchInput, symbol_search_impl),
            "symbol_doc" => call!("symbol_doc", SymbolDocInput, symbol_doc_impl),
            other => Err(format!(
                "unknown tool {other:?}: not handled by in-process dispatch"
            )),
        }
    }
}

/// One advertised tool, flattened for callers that drive the server in-process
/// instead of over MCP (see [`GavieroMcpServer::call_tool_in_process`]).
#[derive(Clone, Debug)]
pub struct InProcessToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments (the MCP `inputSchema`).
    pub input_schema: serde_json::Value,
}

#[tool_handler]
impl ServerHandler for GavieroMcpServer {
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult {
            tools: self.listed_tools(),
            ..Default::default()
        })
    }

    /// The MCP observation owner (stdio, pipe and loopback HTTP all land
    /// here). One telemetry entry per invocation, emitted after the router
    /// returns, so a gate rejection, an unknown tool, undecodable arguments
    /// and a core failure are observed exactly like a success. The logged
    /// output is the logical payload (`structuredContent`) once, not also
    /// its `content` text copy.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, ErrorData> {
        let started = Instant::now();
        let name = request.name.to_string();
        let input = request
            .arguments
            .clone()
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);
        let result = match self.ensure_tool_allowed(&request.name) {
            Ok(()) => {
                let tcc = ToolCallContext::new(self, request, context);
                self.tool_router.call(tcc).await
            }
            Err(e) => Err(e),
        };
        let (output, error) = match &result {
            Ok(r) if r.is_error == Some(true) => (
                serde_json::Value::Null,
                Some(super::client::render_call_result(r)),
            ),
            Ok(r) => (
                r.structured_content.clone().unwrap_or(serde_json::Value::Null),
                None,
            ),
            Err(e) => (serde_json::Value::Null, Some(e.to_string())),
        };
        self.emit_tool_call(&name, input, output, started, error);
        result
    }

    fn get_tool(&self, name: &str) -> Option<rmcp::model::Tool> {
        if !self.tool_is_listed(name) {
            return None;
        }
        self.tool_router.get(name).cloned()
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
    /// Path of `mcp-endpoint.json` when [`Self::with_endpoint_descriptor`]
    /// ran. Removed on shutdown so `--resolve` does not chase a dead pid.
    descriptor_path: Option<PathBuf>,
    http: Option<super::http::HttpListenerHandle>,
}

impl McpServerHandle {
    /// Persist `<root>/.gaviero/mcp-endpoint.json` for shim `--resolve`.
    /// Write failures are logged; the server still serves on the pipe/socket.
    pub fn with_endpoint_descriptor(mut self, root: &std::path::Path) -> Self {
        let mut desc = super::endpoint_file::McpEndpointDescriptor::from_listener(
            root,
            &self.endpoint,
            std::process::id(),
        );
        if let Some(http) = &self.http {
            desc.http_url = Some(http.endpoint.url.clone());
            desc.http_token_path = Some(http.endpoint.token_path.clone());
        }
        match super::endpoint_file::write_descriptor(root, &desc) {
            Ok(path) => self.descriptor_path = Some(path),
            Err(e) => tracing::warn!(
                target: "mcp_server",
                error = %e,
                "failed to write mcp-endpoint.json"
            ),
        }
        self
    }

    /// Attach the loopback HTTP listener (P2). Rewrites the descriptor
    /// when one was already written.
    pub fn with_http_listener(
        mut self,
        http: super::http::HttpListenerHandle,
        root: &std::path::Path,
    ) -> Self {
        self.http = Some(http);
        if self.descriptor_path.is_some() {
            self = self.with_endpoint_descriptor(root);
        }
        self
    }

    pub fn http_endpoint(&self) -> Option<&super::http::HttpEndpoint> {
        self.http.as_ref().map(|h| &h.endpoint)
    }

    /// Signal the accept loop to stop and await its exit. Idempotent.
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.join.await;
        if let Some(http) = self.http {
            http.shutdown().await;
        }
        // Best-effort socket-file cleanup. Named pipes vanish with the
        // last open handle — nothing to remove on the pipe arm.
        if let super::McpEndpoint::Unix(path) = &self.endpoint {
            let _ = std::fs::remove_file(path);
        }
        if let Some(path) = self.descriptor_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Serve one accepted connection on its own task: per-connection
/// clone (fresh first-tool-call latch, shared warm caches), absorb
/// dual-era `server/discover` probes, then rmcp over the remaining
/// `AsyncRead + AsyncWrite` halves.
fn spawn_connection<S>(server: &GavieroMcpServer, stream: S)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
{
    let server_clone = server.clone_for_connection();
    tokio::spawn(async move {
        serve_connection(server_clone, stream).await;
    });
}

async fn serve_connection<S>(server: GavieroMcpServer, stream: S)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static,
{
    let (mut rx, mut tx) = tokio::io::split(stream);
    let prefix = match super::legacy_handshake::absorb_discover_probes(&mut rx, &mut tx).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                target: "mcp_server",
                error = %e,
                "legacy MCP handshake failed"
            );
            return;
        }
    };
    if prefix.is_empty() {
        tracing::debug!(
            target: "mcp_server",
            "MCP connection closed during server/discover probe"
        );
        return;
    }
    use tokio::io::AsyncReadExt as _;
    let rx = std::io::Cursor::new(prefix).chain(rx);
    match server.serve((rx, tx)).await {
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
        descriptor_path: None,
        http: None,
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
        descriptor_path: None,
        http: None,
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

        // `module` / `run` still need per-file / per-run identity that
        // does not cross the shim. `repo` is no longer in this list — the
        // server supplies its own workspace_root as the folder, so repo
        // scope is reachable (see `memory_search_reaches_repo_scope`).
        for bad in ["module", "run", "solar"] {
            match search(Some(bad)).await {
                Err(err) => {
                    assert!(err.message.contains("scope_hint"), "{bad}: {}", err.message)
                }
                Ok(_) => panic!("scope_hint {bad:?} must error"),
            }
        }

        // This fixture registers no folders, so the repo level is skipped
        // by the guard and the restriction simply matches nothing —
        // it must not error.
        let only_repo = search(Some("repo")).await.unwrap().0.results;
        assert!(
            only_repo.is_empty(),
            "no folder is registered here, so repo scope has nothing to return: {only_repo:?}"
        );
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
                ..Default::default()
            }))
            .await;
        assert!(res.is_err());
    }

    // ── blast_radius formatting (X1) ────────────────────────────────

    fn impact(
        changed: &[&str],
        affected: &[&str],
        gaps: &[&str],
    ) -> crate::repo_map::store::ImpactSummary {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        crate::repo_map::store::ImpactSummary {
            changed_files: own(changed),
            affected_files: own(affected),
            test_gaps: own(gaps),
            ..Default::default()
        }
    }

    fn ranks(rows: &[(&str, f64)]) -> std::collections::HashMap<String, (f64, f64)> {
        rows.iter()
            .map(|(p, s)| (p.to_string(), (*s, 0.87654)))
            .collect()
    }

    fn dependent_paths(out: &BlastRadiusOutput) -> Vec<&str> {
        out.nodes
            .iter()
            .filter(|n| n.relation.is_none())
            .map(|n| n.path.as_str())
            .collect()
    }

    /// `n` dependents `d000..` with strictly decreasing scores.
    fn many(n: usize) -> (Vec<String>, std::collections::HashMap<String, (f64, f64)>) {
        let paths: Vec<String> = (0..n).map(|i| format!("src/d{i:03}.rs")).collect();
        let r = paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), (1.0 - i as f64 / 1000.0, 1.0)))
            .collect();
        (paths, r)
    }

    #[test]
    fn blast_format_bounds_dependents_and_counts_omissions() {
        for (n, limit, shown, not_shown) in [
            (0, 40, 0, 0),
            (40, 40, 40, 0),
            (41, 40, 40, 1),
            (41, 0, 0, 41),
            (250, clamp_blast_limit(Some(10_000)), 200, 50),
            (45, clamp_blast_limit(None), 40, 5),
        ] {
            let (paths, r) = many(n);
            let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            let mut affected = vec!["src/seed.rs"];
            affected.extend(refs.iter().copied());
            let out = format_blast_radius(
                &impact(&["src/seed.rs"], &affected, &["src/seed.rs"]),
                &r,
                BlastRadiusMode::Impact,
                limit,
            );
            let deps = dependent_paths(&out);
            assert_eq!(deps.len(), shown, "n={n} limit={limit}");
            assert_eq!(deps, refs[..shown].to_vec(), "best-ranked first");
            assert_eq!(out.not_shown, not_shown, "n={n} limit={limit}");
            assert_eq!(out.truncated, not_shown > 0);
            // Seed and test-gap rows survive every limit, including 0.
            assert_eq!(out.nodes.first().unwrap().relation.as_deref(), Some("changed"));
            assert_eq!(out.nodes.last().unwrap().relation.as_deref(), Some("test_gap"));
            assert_eq!(out.mode, "impact");
        }
    }

    #[test]
    fn blast_format_is_stable_for_ties_and_sql_order() {
        let r = ranks(&[
            ("src/b.rs", 0.5),
            ("src/a.rs", 0.5),
            ("src/c.rs", 0.5),
            ("src/hi.rs", 0.9),
            // Both round to 0.123; full precision decides the order.
            ("src/r2.rs", 0.1231),
            ("src/r1.rs", 0.1234),
        ]);
        let orders: [&[&str]; 2] = [
            &["src/b.rs", "src/r2.rs", "src/a.rs", "src/hi.rs", "src/c.rs", "src/r1.rs"],
            &["src/r1.rs", "src/c.rs", "src/hi.rs", "src/a.rs", "src/r2.rs", "src/b.rs"],
        ];
        let outs: Vec<String> = orders
            .iter()
            .map(|affected| {
                let out = format_blast_radius(
                    &impact(&["src/seed.rs"], affected, &[]),
                    &r,
                    BlastRadiusMode::All,
                    40,
                );
                assert_eq!(
                    dependent_paths(&out),
                    ["src/hi.rs", "src/a.rs", "src/b.rs", "src/c.rs", "src/r1.rs", "src/r2.rs"]
                );
                serde_json::to_string(&out).unwrap()
            })
            .collect();
        assert_eq!(outs[0], outs[1], "SQL row order must not leak into the output");
    }

    #[test]
    fn blast_format_collapses_duplicate_seeds_and_drops_non_finite_scores() {
        let mut r = ranks(&[("src/x.rs", 0.2)]);
        r.insert("src/nan.rs".into(), (f64::NAN, f64::INFINITY));
        let out = format_blast_radius(
            &impact(
                &["src/s.rs", "src/t.rs", "src/s.rs"],
                &["src/s.rs", "src/nan.rs", "src/t.rs", "src/x.rs"],
                &["src/s.rs", "src/s.rs"],
            ),
            &r,
            BlastRadiusMode::Callers,
            40,
        );
        let changed: Vec<&str> = out
            .nodes
            .iter()
            .filter(|n| n.relation.as_deref() == Some("changed"))
            .map(|n| n.path.as_str())
            .collect();
        assert_eq!(changed, ["src/s.rs", "src/t.rs"]);
        assert_eq!(dependent_paths(&out), ["src/x.rs", "src/nan.rs"], "NaN ranks last");
        let gaps = out
            .nodes
            .iter()
            .filter(|n| n.relation.as_deref() == Some("test_gap"))
            .count();
        assert_eq!(gaps, 1);
        let nan_row = out.nodes.iter().find(|n| n.path == "src/nan.rs").unwrap();
        assert!(nan_row.score.is_none() && nan_row.specificity.is_none());
        serde_json::to_string(&out).expect("non-finite scores never reach JSON");
    }

    #[test]
    fn blast_format_emits_the_compact_shape() {
        let out = format_blast_radius(
            &impact(&["src/s.rs"], &["src/s.rs", "src/x.rs"], &[]),
            &ranks(&[("src/x.rs", 0.123456789)]),
            BlastRadiusMode::parse("not-a-mode"),
            40,
        );
        let v = serde_json::to_value(&out).unwrap();
        assert_eq!(v["mode"], "all", "unknown modes keep normalizing to all");
        assert_eq!(v["not_shown"], 0);
        assert_eq!(v["truncated"], false);
        let dep = &v["nodes"][1];
        assert_eq!(dep["path"], "src/x.rs");
        assert_eq!(dep["score"], 0.123);
        assert_eq!(dep["specificity"], 0.877);
        for gone in ["qualified_name", "distance", "relation"] {
            assert!(dep.get(gone).is_none(), "dependent row must not carry {gone}: {dep}");
        }
        assert_eq!(v["nodes"][0]["relation"], "changed");
    }

    /// A workspace where `src/core.rs` has `callers` dependents.
    fn fan_out_workspace(callers: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("core.rs"), "pub fn gv_core_target() {}\n").unwrap();
        for i in 0..callers {
            std::fs::write(
                src.join(format!("caller_{i:02}.rs")),
                format!("pub fn gv_caller_{i:02}() {{ gv_core_target(); }}\n"),
            )
            .unwrap();
        }
        dir
    }

    #[tokio::test]
    async fn blast_radius_bounds_a_real_graph() {
        let dir = fan_out_workspace(45);
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let s = GavieroMcpServer::with_defaults(stores, dir.path().to_path_buf());
        let call = |limit: Option<u32>| {
            let s = s.clone();
            async move {
                s.blast_radius(Parameters(BlastRadiusInput {
                    paths: vec!["src/core.rs".into()],
                    depth: Some(1),
                    mode: Some("callers".into()),
                    limit,
                }))
                .await
                .unwrap()
                .0
            }
        };
        let default = call(None).await;
        assert_eq!(default.mode, "callers");
        assert_eq!(dependent_paths(&default).len(), 40);
        assert_eq!(default.not_shown, 5);
        assert!(default.truncated);
        let all = call(Some(200)).await;
        assert_eq!(dependent_paths(&all).len(), 45);
        assert!(!all.truncated);
        // The description's "~20-30 per row" hint: rows here use short
        // paths, so they must come in under the top of that band.
        let bytes = serde_json::to_string(&all).unwrap().len();
        let per_row = bytes.div_ceil(4) / all.nodes.len();
        assert!(per_row <= 30, "{per_row} estimated tokens per row");
        assert_eq!(
            dependent_paths(&default),
            dependent_paths(&all)[..40].to_vec(),
            "the limit keeps the best-ranked prefix"
        );
        let none = call(Some(0)).await;
        assert!(dependent_paths(&none).is_empty());
        assert_eq!(none.not_shown, 45);
        assert_eq!(none.nodes[0].path, "src/core.rs");
    }

    // ── observation ownership (X4) ──────────────────────────────────

    #[derive(Default)]
    struct RecordingObserver(std::sync::Mutex<Vec<McpCallLogEntry>>);

    impl McpToolCallObserver for RecordingObserver {
        fn on_tool_call(&self, entry: &McpCallLogEntry) {
            self.0.lock().unwrap().push(entry.clone());
        }
    }

    impl RecordingObserver {
        fn take(&self) -> Vec<McpCallLogEntry> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    struct FailingSink;

    #[async_trait::async_trait]
    impl MemorySignalSink for FailingSink {
        async fn flag(
            &self,
            _req: MemoryFlagRequest,
        ) -> anyhow::Result<super::super::signal::MemoryFlagOutcome> {
            anyhow::bail!("writer unavailable")
        }
    }

    fn observed_server(
        root: &std::path::Path,
    ) -> (GavieroMcpServer, Arc<RecordingObserver>) {
        let recorder = Arc::new(RecordingObserver::default());
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let s = GavieroMcpServer::new(
            stores,
            root.to_path_buf(),
            recorder.clone() as Arc<dyn McpToolCallObserver>,
            RetrievalConfig::default(),
            RerankConfig::default(),
            None,
        );
        (s, recorder)
    }

    /// Serve `server` over an in-memory duplex and connect a real rmcp
    /// client — the same path a shim connection takes.
    async fn mcp_client(
        server: &GavieroMcpServer,
    ) -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
        let (client_end, server_end) = tokio::io::duplex(256 * 1024);
        spawn_connection(server, server_end);
        let (r, w) = tokio::io::split(client_end);
        ().serve(rmcp::transport::async_rw::AsyncRwTransport::new_client(r, w))
            .await
            .expect("client initialize")
    }

    /// One call through either entry point: `Ok(output)` or `Err(message)`.
    async fn invoke(
        via_mcp: Option<&rmcp::service::RunningService<rmcp::RoleClient, ()>>,
        server: &GavieroMcpServer,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match via_mcp {
            None => server.call_tool_in_process(name, args).await,
            Some(client) => {
                let mut params = CallToolRequestParams::new(name.to_string());
                if let serde_json::Value::Object(map) = args {
                    params = params.with_arguments(map);
                }
                match client.peer().call_tool(params).await {
                    Ok(r) if r.is_error == Some(true) => Err(format!("{:?}", r.content)),
                    Ok(r) => Ok(r.structured_content.unwrap_or(serde_json::Value::Null)),
                    Err(e) => Err(e.to_string()),
                }
            }
        }
    }

    /// `(tool, args, expect_ok)` covering a success and the failure kinds
    /// of every advertised tool except the gate-only cases tested below.
    fn observation_cases() -> Vec<(&'static str, serde_json::Value, bool)> {
        use serde_json::json;
        vec![
            ("memory_search", json!({"query": "x"}), true),
            ("memory_search", json!({}), false),
            ("memory_search", json!({"query": "x", "kind": "episode"}), false),
            ("blast_radius", json!({"paths": ["src/core.rs"]}), true),
            ("blast_radius", json!({"paths": ["src/core.rs"], "limit": 0}), true),
            ("blast_radius", json!({"paths": []}), false),
            ("blast_radius", json!({"paths": ["a"], "limit": -1}), false),
            ("blast_radius", json!({"paths": ["a"], "limit": 1.5}), false),
            ("blast_radius", json!({"paths": ["a"], "limit": "10"}), false),
            ("node_doc", json!({"path": "src/core.rs"}), true),
            ("node_doc", json!({"path": 7}), false),
            ("memory_get", json!({"id": 1, "scope": "workspace"}), true),
            ("memory_get", json!({"id": 1, "scope": "repo"}), false),
            ("memory_ping", json!({"nonce": "n", "depth": 0}), true),
            ("memory_ping", json!({"nonce": "n", "depth": 300}), false),
            ("memory_flag", json!({"id": 1, "scope": "workspace", "reason": "stale"}), true),
            ("memory_flag", json!({"id": 1, "scope": "workspace", "reason": "  "}), false),
            ("memory_flag", json!({"id": 1, "scope": "cosmic", "reason": "stale"}), false),
            ("memory_flag", json!({"id": 1, "scope": "workspace"}), false),
            ("repo_outline", json!({}), true),
            ("symbol_search", json!({"query": "  "}), false),
            ("symbol_doc", json!({"qualified_name": "src/core.rs::nope"}), false),
            ("definitely_not_a_tool", json!({}), false),
        ]
    }

    #[tokio::test]
    async fn every_invocation_is_observed_exactly_once_on_both_entry_points() {
        let dir = fan_out_workspace(3);
        let sink = Arc::new(RecordingSink {
            accepted: true,
            ..Default::default()
        });
        let (s, recorder) = observed_server(dir.path());
        let s = s.with_signal_sink(sink.clone()).with_symbol_enrichment(true);
        let client = mcp_client(&s).await;

        for via_mcp in [false, true] {
            let path = if via_mcp { "mcp" } else { "in-process" };
            let flags_before = sink.seen.lock().unwrap().len();
            for (tool, args, expect_ok) in observation_cases() {
                let result = invoke(via_mcp.then_some(&client), &s, tool, args.clone()).await;
                assert_eq!(result.is_ok(), expect_ok, "{path} {tool} {args}: {result:?}");
                let rows = recorder.take();
                assert_eq!(rows.len(), 1, "{path} {tool} {args}: one row per invocation");
                let row = &rows[0];
                assert_eq!(row.tool_name, tool);
                assert_eq!(row.input, args, "{path} {tool}: input recorded as sent");
                match &result {
                    Ok(output) => {
                        assert!(row.error.is_none(), "{path} {tool}");
                        assert_eq!(&row.output, output, "{path} {tool}: logical payload once");
                    }
                    Err(_) => {
                        assert!(row.error.is_some(), "{path} {tool} {args}");
                        assert!(row.output.is_null(), "{path} {tool}: no payload on failure");
                    }
                }
            }
            assert_eq!(
                sink.seen.lock().unwrap().len() - flags_before,
                1,
                "{path}: exactly one signal, from the one successful flag"
            );
        }
        client.cancel().await.ok();
    }

    /// X8: the final payloads are identical through both dispatch paths,
    /// byte for byte as compact JSON.
    #[tokio::test]
    async fn both_entry_points_return_the_same_logical_payload() {
        let dir = fan_out_workspace(45);
        let sink = Arc::new(RecordingSink {
            accepted: true,
            ..Default::default()
        });
        let (s, _rec) = observed_server(dir.path());
        let s = s.with_signal_sink(sink).with_symbol_enrichment(true);
        let client = mcp_client(&s).await;
        let mut compared = 0;
        for (tool, args, expect_ok) in observation_cases() {
            if !expect_ok {
                continue;
            }
            let local = invoke(None, &s, tool, args.clone()).await.unwrap();
            let remote = invoke(Some(&client), &s, tool, args.clone()).await.unwrap();
            assert_eq!(
                serde_json::to_string(&local).unwrap(),
                serde_json::to_string(&remote).unwrap(),
                "{tool} {args}"
            );
            compared += 1;
        }
        assert!(compared >= 8, "{compared}");
        client.cancel().await.ok();
    }

    #[tokio::test]
    async fn gate_rejections_are_observed_on_both_entry_points() {
        let dir = fan_out_workspace(1);
        let (denied, denied_rec) = observed_server(dir.path());
        let denied = denied.with_permissions(super::super::McpPermissions {
            allow: vec![],
            deny: vec!["gaviero:blast_radius".into()],
        });
        let (hidden, hidden_rec) = observed_server(dir.path());
        let hidden = hidden.with_exposed_tools(vec!["memory_search".into()]);
        // Enrichment off: both symbol tools are hidden, not just failing.
        let (plain, plain_rec) = observed_server(dir.path());
        let (unwired, unwired_rec) = observed_server(dir.path());
        let failing = observed_server(dir.path());
        let failing_rec = failing.1.clone();
        let failing = failing.0.with_signal_sink(Arc::new(FailingSink));

        let flag = serde_json::json!({"id": 1, "scope": "workspace", "reason": "r"});
        let cases: Vec<(&GavieroMcpServer, &RecordingObserver, &str, serde_json::Value)> = vec![
            (&denied, &*denied_rec, "blast_radius", serde_json::json!({"paths": ["src/core.rs"]})),
            (&hidden, &*hidden_rec, "node_doc", serde_json::json!({"path": "src/core.rs"})),
            (&plain, &*plain_rec, "symbol_search", serde_json::json!({"query": "x"})),
            (&plain, &*plain_rec, "symbol_doc", serde_json::json!({"qualified_name": "a::b"})),
            (&unwired, &*unwired_rec, "memory_flag", flag.clone()),
            (&failing, &*failing_rec, "memory_flag", flag),
        ];
        for (server, rec, tool, args) in cases {
            let client = mcp_client(server).await;
            for via_mcp in [false, true] {
                let result = invoke(via_mcp.then_some(&client), server, tool, args.clone()).await;
                assert!(result.is_err(), "{tool} via_mcp={via_mcp}: {result:?}");
                let rows = rec.take();
                assert_eq!(rows.len(), 1, "{tool} via_mcp={via_mcp}");
                assert_eq!(rows[0].tool_name, tool);
                assert!(rows[0].error.is_some());
                assert!(rows[0].output.is_null());
            }
            client.cancel().await.ok();
        }
    }

    /// The in-process path preserves the exact error text callers saw
    /// before observation moved outward.
    #[tokio::test]
    async fn in_process_errors_keep_their_messages() {
        let (s, _rec) = observed_server(std::path::Path::new("/tmp"));
        let err = s
            .call_tool_in_process("memory_flag", serde_json::json!({"id": 1, "scope": "workspace", "reason": "r"}))
            .await
            .unwrap_err();
        assert!(err.contains("mcp.flag.enabled"), "{err}");
        let err = s
            .call_tool_in_process("blast_radius", serde_json::json!({"paths": ["a"], "limit": -1}))
            .await
            .unwrap_err();
        assert!(err.starts_with("invalid arguments for blast_radius"), "{err}");
    }

    /// A timed-out `blast_radius` is cancelled, not abandoned: its blocking
    /// work stops instead of running on after the caller gave up, and the
    /// next call works normally.
    #[tokio::test]
    async fn a_timed_out_blast_radius_stops_its_work() {
        let dir = fan_out_workspace(5);
        let mut s = outline_server(dir.path());
        s.blast_test_delay = Duration::from_millis(400);
        s.graph_query_timeout = Duration::from_millis(50);
        let input = || BlastRadiusInput {
            paths: vec!["src/core.rs".into()],
            ..Default::default()
        };
        let err = match s.blast_radius(Parameters(input())).await {
            Err(e) => e,
            Ok(_) => panic!("expected the timeout"),
        };
        assert!(err.message.contains("timed out"), "{}", err.message);
        // Give the abandoned job time to finish if it were still running.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            s.blast_completed.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the cancelled query must not run to completion"
        );

        s.blast_test_delay = Duration::ZERO;
        s.graph_query_timeout = GRAPH_QUERY_TIMEOUT;
        let out = s.blast_radius(Parameters(input())).await.unwrap().0;
        assert_eq!(out.nodes[0].path, "src/core.rs");
        assert_eq!(s.blast_completed.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn serve_survives_discover_then_initialize() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let (client, server_end) = tokio::io::duplex(8192);
        spawn_connection(&fixture(), server_end);
        let (client_r, mut client_w) = tokio::io::split(client);
        let mut client_r = BufReader::new(client_r);

        client_w
            .write_all(
                br#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{}}
"#,
            )
            .await
            .unwrap();
        client_w.flush().await.unwrap();

        let mut probe = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client_r.read_line(&mut probe),
        )
        .await
        .expect("timed out waiting for discover error")
        .unwrap();
        let probe_json: serde_json::Value = serde_json::from_str(probe.trim()).unwrap();
        assert_eq!(probe_json["id"], "server-discover-probe-1");
        assert_eq!(probe_json["error"]["code"], -32601);

        client_w
            .write_all(
                br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"gaviero-test","version":"0"}}}
"#,
            )
            .await
            .unwrap();
        client_w.flush().await.unwrap();

        let mut init = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client_r.read_line(&mut init),
        )
        .await
        .expect("timed out waiting for initialize result")
        .unwrap();
        let init_json: serde_json::Value = serde_json::from_str(init.trim()).unwrap();
        assert!(
            init_json.get("result").is_some(),
            "initialize must succeed after discover probe, got {init_json}"
        );
        assert_eq!(init_json["id"], 1);
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
    async fn descriptor_written_and_removed_on_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let server = GavieroMcpServer::with_defaults(stores, dir.path().to_path_buf());
        #[cfg(windows)]
        let endpoint = super::super::McpEndpoint::Pipe(format!(
            r"\\.\pipe\gaviero-desc-{}",
            std::process::id()
        ));
        #[cfg(unix)]
        let endpoint = super::super::McpEndpoint::Unix(dir.path().join("mcp.sock"));
        let handle = spawn_mcp_server(server, &endpoint)
            .unwrap()
            .with_endpoint_descriptor(dir.path());
        let path = crate::mcp::endpoint_file::McpEndpointDescriptor::path(dir.path());
        assert!(
            path.is_file(),
            "descriptor should exist at {}",
            path.display()
        );
        let desc = crate::mcp::endpoint_file::read_descriptor(&path).unwrap();
        assert_eq!(desc.pid, std::process::id());
        handle.shutdown().await;
        assert!(!path.exists(), "descriptor should be removed on shutdown");
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

    /// A long, realistic path so the per-symbol cost check below is not
    /// flattered by short names.
    const NODE_DOC_PATH: &str = "crates/gaviero-core/src/repo_map/node_doc_fixture.rs";

    fn node_doc_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(NODE_DOC_PATH);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(
            &file,
            "pub fn gv_alpha() {}\n\npub struct GvBeta;\n\npub fn gv_gamma(x: u32) -> u32 { x }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "# not source\n").unwrap();
        dir
    }

    fn node_doc_server(dir: &std::path::Path) -> GavieroMcpServer {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        GavieroMcpServer::with_defaults(stores, dir.to_path_buf()).with_symbol_enrichment(true)
    }

    async fn node_doc_of(s: &GavieroMcpServer, path: &str) -> NodeDoc {
        s.node_doc(Parameters(NodeDocInput { path: path.into() }))
            .await
            .unwrap()
            .0
    }

    #[tokio::test]
    async fn node_doc_returns_lines_without_duplicate_signatures() {
        let dir = node_doc_workspace();
        let s = node_doc_server(dir.path());
        let out = node_doc_of(&s, NODE_DOC_PATH).await;

        let by_name: std::collections::HashMap<&str, &NodeDocSymbol> = out
            .symbols
            .iter()
            .map(|sym| (sym.qualified_name.rsplit("::").next().unwrap(), sym))
            .collect();
        assert_eq!(by_name["gv_alpha"].line, Some(1), "stored row 0 is line 1");
        assert_eq!(by_name["GvBeta"].line, Some(3));
        assert_eq!(by_name["gv_gamma"].line, Some(5));
        for sym in &out.symbols {
            assert!(sym.qualified_name.starts_with(&format!("{NODE_DOC_PATH}::")));
            assert!(sym.signature.is_none(), "unenriched rows carry no signature");
        }
        assert!(out.note.is_none());

        let v = serde_json::to_value(&out).unwrap();
        assert!(v.get("signatures").is_none(), "legacy list is gone: {v}");
        assert!(!super::super::telemetry_sink::output_is_empty(&v));

        // The description's "~15-25 per symbol" hint: the marginal cost of
        // a symbol row, measured with a long path and real lines (the fixed
        // header is paid once per call).
        let bytes = serde_json::to_string(&out).unwrap().len();
        let mut header_only = out.clone();
        header_only.symbols.clear();
        let header = serde_json::to_string(&header_only).unwrap().len();
        let per_symbol = (bytes - header).div_ceil(4) / out.symbols.len();
        assert!((15..=25).contains(&per_symbol), "{per_symbol} estimated tokens per symbol");
    }

    #[tokio::test]
    async fn node_doc_keeps_real_enriched_signatures_and_symbol_doc_chaining() {
        let dir = node_doc_workspace();
        let s = node_doc_server(dir.path());
        // Build the graph, then add enrichment rows the way `--enrich` would.
        node_doc_of(&s, NODE_DOC_PATH).await;
        let store = crate::repo_map::store::GraphStore::open(&s.graph_db_path).unwrap();
        let enrich = |symbol: &str, signature: &str, doc: &str| {
            store
                .upsert_symbol_doc(&crate::repo_map::store::SymbolDoc {
                    qualified_name: format!("{NODE_DOC_PATH}::{symbol}"),
                    file_path: NODE_DOC_PATH.into(),
                    file_hash: None,
                    signature: signature.into(),
                    bounds: String::new(),
                    doc: doc.into(),
                    role_summary: String::new(),
                    embedding: None,
                })
                .unwrap();
        };
        enrich("gv_gamma", "pub fn gv_gamma(x: u32) -> u32", "Doubles nothing.");
        // An enriched row whose signature is only the name.
        enrich("GvBeta", "GvBeta", "");

        let out = node_doc_of(&s, NODE_DOC_PATH).await;
        let sym = |name: &str| {
            out.symbols
                .iter()
                .find(|x| x.qualified_name.ends_with(&format!("::{name}")))
                .unwrap()
        };
        assert_eq!(
            sym("gv_gamma").signature.as_deref(),
            Some("pub fn gv_gamma(x: u32) -> u32")
        );
        assert_eq!(sym("gv_gamma").doc_snippet.as_deref(), Some("Doubles nothing."));
        assert_eq!(sym("gv_gamma").line, Some(5));
        assert!(sym("GvBeta").signature.is_none());
        assert!(sym("gv_alpha").signature.is_none());

        let chained = s
            .symbol_doc(Parameters(SymbolDocInput {
                qualified_name: sym("gv_gamma").qualified_name.clone(),
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(chained.signature, "pub fn gv_gamma(x: u32) -> u32");
    }

    #[tokio::test]
    async fn node_doc_explains_empty_results_from_persisted_status() {
        let dir = node_doc_workspace();
        let root = dir.path();
        std::fs::write(root.join("mods.rs"), "mod a;\n// nothing defined here\n").unwrap();
        let mut big = String::from("pub fn gv_big() {}\n");
        while big.len() <= crate::repo_map::extraction::MAX_PARSE_BYTES {
            big.push_str("// padding padding padding padding padding padding\n");
        }
        std::fs::write(root.join("big.rs"), big).unwrap();
        std::fs::write(
            root.join("parser.c"),
            "/* Automatically @generated by tree-sitter v0.25.10 */\nint f(void) { return 0; }\n",
        )
        .unwrap();
        std::fs::write(root.join("bad.rs"), [0xffu8, 0xfe, 0x00]).unwrap();
        let s = node_doc_server(root);

        for (path, status) in [
            ("README.md", "not_indexed"),
            ("src/absent.rs", "not_indexed"),
            ("crates", "not_indexed"),
            ("mods.rs", "no_definitions"),
            ("big.rs", "parse_size_excluded"),
            ("parser.c", "generated_excluded"),
            ("bad.rs", "read_failed"),
        ] {
            let out = node_doc_of(&s, path).await;
            assert!(out.symbols.is_empty(), "{path}");
            assert_eq!(out.status.as_deref(), Some(status), "{path}");
            assert!(out.note.as_deref().is_some_and(|n| !n.is_empty()), "{path}");
            let v = serde_json::to_value(&out).unwrap();
            assert!(super::super::telemetry_sink::output_is_empty(&v), "{path}: {v}");
        }
        // A nonempty result carries neither field.
        let full = node_doc_of(&s, NODE_DOC_PATH).await;
        assert!(full.status.is_none() && full.note.is_none());

        // A graph built before statuses existed says `unknown`, not a guess.
        let conn = rusqlite::Connection::open(&s.graph_db_path).unwrap();
        conn.execute("DELETE FROM file_extraction WHERE file_path = 'mods.rs'", [])
            .unwrap();
        drop(conn);
        assert_eq!(node_doc_of(&s, "mods.rs").await.status.as_deref(), Some("unknown"));
    }

    #[test]
    fn node_doc_status_vocabulary_maps_every_outcome() {
        use crate::repo_map::extraction::ExtractionOutcome;
        let mut notes = std::collections::HashSet::new();
        for o in ExtractionOutcome::ALL {
            let (status, note) = super::super::tools::node_doc_empty_status(Some(o.as_str()), true);
            assert_eq!(status, o.as_str());
            assert!(!note.is_empty() && !note.contains("unrecognised"), "{status}");
            notes.insert(note);
        }
        assert_eq!(notes.len(), ExtractionOutcome::ALL.len(), "one distinct note per outcome");
        assert_eq!(super::super::tools::node_doc_empty_status(None, true).0, "unknown");
        assert_eq!(super::super::tools::node_doc_empty_status(None, false).0, "not_indexed");
        let (status, note) = super::super::tools::node_doc_empty_status(Some("from_the_future"), true);
        assert_eq!(status, "from_the_future");
        assert!(note.contains("unrecognised"));
    }

    #[tokio::test]
    async fn memory_ping_records_nonce_and_depth() {
        let ledger = ProbeLedger::new();
        let s = fixture().with_probe_ledger(ledger.clone());
        let out = s
            .memory_ping(Parameters(MemoryPingInput {
                nonce: "probe-nonce".into(),
                depth: 1,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(out.depth, 1);
        assert_eq!(out.receipt.len(), 12);
        assert!(!out.workspace_id.is_empty());
        let recs = ledger.records_for_nonce("probe-nonce");
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].depth, 1);
        assert_eq!(recs[0].receipt, out.receipt);
    }

    #[tokio::test]
    async fn memory_ping_without_ledger_still_answers() {
        let s = fixture();
        let out = s
            .memory_ping(Parameters(MemoryPingInput {
                nonce: "no-ledger".into(),
                depth: 0,
            }))
            .await
            .unwrap()
            .0;
        assert_eq!(out.depth, 0);
        assert_eq!(out.receipt.len(), 12);
        assert!(!out.workspace_id.is_empty());
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

    /// A server whose registry has the workspace root registered as a
    /// folder — i.e. what production looks like. `split_fixture` cannot
    /// exercise repo scope because `for_tests_split_in_memory` registers
    /// no folders at all.
    fn folder_fixture(root: &std::path::Path, global_dir: &std::path::Path) -> GavieroMcpServer {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let workspace = crate::workspace::Workspace::single_folder(root.to_path_buf());
        let stores = MemoryStores::open_with_paths(
            root,
            &workspace,
            embedder,
            "mock".to_string(),
            &global_dir.join("global.db"),
        )
        .expect("open registry");
        GavieroMcpServer::with_defaults(stores, root.to_path_buf())
    }

    /// Repo-scope memories must be reachable from the MCP surface: the
    /// server supplies its own `workspace_root` as the folder, so
    /// `MemoryScope::levels()` includes Repo. Before this, `folder = None`
    /// meant every repo-scoped row was invisible to subprocess agents —
    /// in a mature workspace that is the majority of the corpus.
    #[tokio::test]
    async fn memory_search_reaches_repo_scope() {
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let root = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let s = folder_fixture(root.path(), global.path());
        let repo_id = crate::memory::hash_path(root.path());

        let meta = WriteMeta::for_source(MemorySource::UserRemember)
            .with_type(MemoryType::Decision)
            .with_tag("repo-reachable");
        s.stores
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: repo_id.clone(),
                },
                "purple pelican repo-scoped convention",
                &meta,
            )
            .await
            .unwrap();

        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "purple pelican convention".into(),
                scope_hint: None,
                limit: Some(10),
                kind: None,
            }))
            .await
            .unwrap()
            .0
            .results;
        assert!(
            out.iter().any(|r| r.scope == "repo"),
            "repo-scoped memory must be reachable from MCP; got {out:?}"
        );

        // And `scope_hint: "repo"` is now a real restriction rather than
        // an invalid_params error.
        let only_repo = s
            .memory_search(Parameters(MemorySearchInput {
                query: "purple pelican convention".into(),
                scope_hint: Some("repo".into()),
                limit: Some(10),
                kind: None,
            }))
            .await
            .unwrap()
            .0
            .results;
        assert!(!only_repo.is_empty(), "scope_hint=repo returned nothing");
        assert!(
            only_repo.iter().all(|r| r.scope == "repo"),
            "scope_hint=repo must return only repo rows; got {only_repo:?}"
        );
    }

    /// `with_graph_db_path` keeps the graph out of the served root — the
    /// isolated headless mode relies on it to leave the repo's
    /// `.gaviero/` untouched.
    #[tokio::test(flavor = "multi_thread")]
    async fn warmup_builds_the_graph_at_the_configured_path() {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("lib.rs"), "pub fn hello() {}").unwrap();
        let db = state.path().join("code_graph.db");
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        let s = GavieroMcpServer::with_defaults(stores, root.path().to_path_buf())
            .with_graph_db_path(db.clone());

        s.warmup().await;

        assert!(db.is_file(), "graph must be persisted at the override path");
        assert!(
            !root.path().join(".gaviero").exists(),
            "the served root must not gain a .gaviero/ directory"
        );
    }

    /// A server rooted below the memory folder (headless run root inside a
    /// discovered workspace) reaches that folder's repo scope through
    /// `with_memory_root`; the served root alone is not a registered folder.
    #[tokio::test]
    async fn memory_search_reaches_repo_scope_through_memory_root() {
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let root = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let run_root = root.path().join("plans").join("x");
        std::fs::create_dir_all(&run_root).unwrap();
        let base = folder_fixture(root.path(), global.path());
        base.stores
            .store_scoped(
                &WriteScope::Repo {
                    repo_id: crate::memory::hash_path(root.path()),
                },
                "teal heron repo-scoped convention",
                &WriteMeta::for_source(MemorySource::UserRemember).with_type(MemoryType::Decision),
            )
            .await
            .unwrap();
        let search = |s: &GavieroMcpServer| {
            let s = s.clone();
            async move {
                s.memory_search(Parameters(MemorySearchInput {
                    query: "teal heron convention".into(),
                    scope_hint: None,
                    limit: Some(10),
                    kind: None,
                }))
                .await
                .unwrap()
                .0
                .results
            }
        };

        let unrooted = GavieroMcpServer::with_defaults(base.stores.clone(), run_root.clone());
        assert!(
            !search(&unrooted).await.iter().any(|r| r.scope == "repo"),
            "an unregistered run root must not reach repo scope on its own"
        );

        let rooted = GavieroMcpServer::with_defaults(base.stores.clone(), run_root)
            .with_memory_root(root.path().to_path_buf());
        let out = search(&rooted).await;
        assert!(
            out.iter().any(|r| r.scope == "repo"),
            "memory_root must expose the owning folder's repo scope; got {out:?}"
        );
    }

    /// The folder guard: a registry that does not know the workspace root
    /// (no folders registered, fallback off) must fall back to
    /// [Workspace, Global] instead of failing every search with
    /// "unknown repo_id — not registered in workspace".
    #[tokio::test]
    async fn memory_search_survives_an_unregistered_workspace_root() {
        use crate::memory::scope::{MemoryType, WriteMeta, WriteScope};
        use crate::memory::trust_defaults::MemorySource;

        let s = split_fixture();
        s.stores
            .workspace()
            .store_scoped(
                &WriteScope::Workspace,
                "amber wombat workspace fact",
                &WriteMeta::for_source(MemorySource::UserRemember).with_type(MemoryType::Decision),
            )
            .await
            .unwrap();

        let out = s
            .memory_search(Parameters(MemorySearchInput {
                query: "amber wombat fact".into(),
                scope_hint: None,
                limit: Some(5),
                kind: None,
            }))
            .await
            .expect("must not error when the root is not a registered folder")
            .0
            .results;
        assert!(out.iter().any(|r| r.scope == "workspace"), "{out:?}");
    }

    /// A recording sink: captures the request the handler builds without
    /// touching a writer or a store.
    #[derive(Default)]
    struct RecordingSink {
        seen: std::sync::Mutex<Vec<super::super::signal::MemoryFlagRequest>>,
        accepted: bool,
    }

    #[async_trait::async_trait]
    impl MemorySignalSink for RecordingSink {
        async fn flag(
            &self,
            req: super::super::signal::MemoryFlagRequest,
        ) -> anyhow::Result<super::super::signal::MemoryFlagOutcome> {
            self.seen.lock().unwrap().push(req);
            Ok(super::super::signal::MemoryFlagOutcome {
                accepted: self.accepted,
                detail: "recorded".into(),
            })
        }
    }

    fn flag_input(id: i64, scope: &str) -> MemoryFlagInput {
        MemoryFlagInput {
            id,
            scope: scope.to_string(),
            reason: "contradicted by the code I just read".into(),
        }
    }

    /// `Json<T>` is not `Debug`, so `expect_err` is unavailable.
    fn expect_flag_error(
        res: Result<Json<MemoryFlagOutput>, ErrorData>,
        context: &str,
    ) -> ErrorData {
        match res {
            Err(e) => e,
            Ok(ok) => panic!(
                "{context}: expected an error, got accepted={}",
                ok.0.accepted
            ),
        }
    }

    /// (ii) Without a sink the tool must error, never silently no-op.
    #[tokio::test]
    async fn memory_flag_without_a_sink_errors() {
        let s = split_fixture();
        let err = expect_flag_error(
            s.memory_flag(Parameters(flag_input(1, "workspace"))).await,
            "unwired server",
        );
        assert!(
            err.message.contains("mcp.flag.enabled"),
            "the error should name the setting that enables it: {}",
            err.message
        );
    }

    /// (iii) Folder scopes and unknown values are invalid_params, same
    /// rule as `memory_get` — the MCP surface carries no repo_id.
    #[tokio::test]
    async fn memory_flag_rejects_unreachable_and_unknown_scopes() {
        let sink = Arc::new(RecordingSink {
            accepted: true,
            ..Default::default()
        });
        let s = split_fixture().with_signal_sink(sink.clone());

        for bad in ["repo", "module", "cosmic"] {
            let err = expect_flag_error(s.memory_flag(Parameters(flag_input(1, bad))).await, bad);
            assert!(
                err.message.contains("memory_flag.scope"),
                "{bad}: {}",
                err.message
            );
        }
        assert!(
            sink.seen.lock().unwrap().is_empty(),
            "a rejected scope must never reach the sink"
        );
    }

    /// (iv) A `mcp.permissions` deny on `gaviero:memory_flag` is
    /// enforced server-side, before the sink is consulted.
    #[tokio::test]
    async fn memory_flag_honours_a_permissions_deny() {
        let sink = Arc::new(RecordingSink {
            accepted: true,
            ..Default::default()
        });
        let permissions = super::super::McpPermissions {
            allow: Vec::new(),
            deny: vec!["gaviero:memory_flag".to_string()],
        };
        let s = split_fixture()
            .with_signal_sink(sink.clone())
            .with_permissions(permissions);

        let err = expect_flag_error(
            s.memory_flag(Parameters(flag_input(1, "workspace"))).await,
            "denied tool",
        );
        assert!(err.message.contains("mcp.permissions"), "{}", err.message);
        assert!(sink.seen.lock().unwrap().is_empty());
    }

    /// The scope string resolves to the `(scope_level, repo_id)` pair the
    /// writer routes on, and an empty reason is refused before it can
    /// reach the audit row.
    #[tokio::test]
    async fn memory_flag_resolves_scope_and_requires_a_reason() {
        let sink = Arc::new(RecordingSink {
            accepted: true,
            ..Default::default()
        });
        let s = split_fixture().with_signal_sink(sink.clone());

        for (scope, expected) in [
            ("global", crate::memory::scope::SCOPE_GLOBAL),
            ("workspace", crate::memory::scope::SCOPE_WORKSPACE),
            ("run", crate::memory::scope::SCOPE_RUN),
        ] {
            let out = s
                .memory_flag(Parameters(flag_input(7, scope)))
                .await
                .expect("wired sink accepts");
            assert!(out.0.accepted);
            let last = sink.seen.lock().unwrap().last().cloned().unwrap();
            assert_eq!(last.memory_id, 7);
            assert_eq!(last.scope_level, expected, "scope {scope}");
            assert!(last.repo_id.is_none());
        }

        let err = expect_flag_error(
            s.memory_flag(Parameters(MemoryFlagInput {
                id: 7,
                scope: "workspace".into(),
                reason: "   ".into(),
            }))
            .await,
            "empty reason",
        );
        assert!(err.message.contains("reason"), "{}", err.message);
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
        }
    }

    // ── repo_outline payload budget (X5) ────────────────────────────

    /// `n` source files with long, partly non-ASCII paths and several
    /// symbols each, so rich lines, JSON escaping and the path-only
    /// downgrade all matter.
    fn outline_workspace(n: usize) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir
            .path()
            .join("crates/gaviero-core/src/a_rather_long_module_directory_name/señal_→");
        std::fs::create_dir_all(&deep).unwrap();
        for i in 0..n {
            let mut src = String::new();
            for j in 0..6 {
                src.push_str(&format!("pub fn gv_outline_symbol_{i:03}_{j}() {{}}\n"));
            }
            if i > 0 {
                src.push_str(&format!(
                    "pub fn gv_link_{i:03}() {{ gv_outline_symbol_{:03}_0(); }}\n",
                    i - 1
                ));
            }
            std::fs::write(deep.join(format!("file_número_{i:03}.rs")), src).unwrap();
        }
        dir
    }

    fn outline_server(root: &std::path::Path) -> GavieroMcpServer {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        GavieroMcpServer::with_defaults(stores, root.to_path_buf())
    }

    async fn outline(s: &GavieroMcpServer, input: RepoOutlineInput) -> RepoOutlineOutput {
        s.repo_outline(Parameters(input)).await.unwrap().0
    }

    fn payload_tokens(out: &RepoOutlineOutput) -> u32 {
        super::super::payload::estimated_tokens(
            serde_json::to_string(out).unwrap().len(),
        ) as u32
    }

    #[tokio::test]
    async fn outline_payload_stays_within_the_effective_cap() {
        let dir = outline_workspace(150);
        let s = outline_server(dir.path());
        for cap in [Some(0), Some(1), Some(100), Some(150), None, Some(4000), Some(8000), Some(999_999)] {
            let effective = clamp_repo_outline_token_cap(cap);
            let out = outline(
                &s,
                RepoOutlineInput {
                    token_cap: cap,
                    ..Default::default()
                },
            )
            .await;
            let actual = payload_tokens(&out);
            assert_eq!(out.token_estimate, actual, "cap {cap:?}: self-consistent estimate");
            assert!(actual <= effective, "cap {cap:?}: {actual} > {effective}");
            assert!(!out.entries.is_empty(), "cap {cap:?}: useful paths survive the overhead");
            // Stable across calls.
            let again = outline(
                &s,
                RepoOutlineInput {
                    token_cap: cap,
                    ..Default::default()
                },
            )
            .await;
            assert_eq!(
                serde_json::to_string(&out).unwrap(),
                serde_json::to_string(&again).unwrap()
            );
        }
        let big = outline(
            &s,
            RepoOutlineInput {
                token_cap: Some(8000),
                ..Default::default()
            },
        )
        .await;
        let v = serde_json::to_value(&big).unwrap();
        for e in v["entries"].as_array().unwrap() {
            assert_eq!(e.as_object().unwrap().len(), 1, "entries carry only `rendered`: {e}");
        }
    }

    #[tokio::test]
    async fn outline_empty_seeds_mean_the_whole_workspace_and_empty_fits() {
        let dir = outline_workspace(5);
        let s = outline_server(dir.path());
        let none = outline(&s, RepoOutlineInput::default()).await;
        let empty = outline(
            &s,
            RepoOutlineInput {
                seed_paths: Some(vec![]),
                ..Default::default()
            },
        )
        .await;
        assert_eq!(
            serde_json::to_string(&none).unwrap(),
            serde_json::to_string(&empty).unwrap()
        );
        assert_eq!(none.entries.len(), 5);

        let bare = tempfile::tempdir().unwrap();
        let out = outline(
            &outline_server(bare.path()),
            RepoOutlineInput {
                token_cap: Some(100),
                ..Default::default()
            },
        )
        .await;
        assert!(out.entries.is_empty());
        assert_eq!(out.token_estimate, payload_tokens(&out));
        assert!(out.token_estimate <= 100);
    }

    #[test]
    fn outline_entry_cost_counts_escaping_and_the_comma() {
        for line in ["  a.rs (s1.00)", "  dir/\"quoted\" \\back (s0.50)", "  señal_→/x.rs (s1.00)"] {
            let entry = serde_json::to_string(&RepoOutlineEntry {
                rendered: line.to_string(),
            })
            .unwrap();
            assert_eq!(outline_entry_cost_bytes(line), entry.len() + 1, "{line}");
        }
        assert!(outline_entry_cost_bytes("\"") > outline_entry_cost_bytes("a"));
    }

    #[test]
    fn finish_outline_estimate_is_self_consistent_across_digit_boundaries() {
        // Grow the payload byte by byte through the 1→2, 2→3 and 3→4 digit
        // transitions of `token_estimate`.
        for n in 0..4200 {
            let out = finish_outline(vec![RepoOutlineEntry {
                rendered: "x".repeat(n),
            }]);
            assert_eq!(out.token_estimate, payload_tokens(&out), "n={n}");
        }
    }

    #[test]
    fn outline_budget_reserves_the_envelope_for_every_cap() {
        for cap in [100u32, 101, 999, 1000, 2000, 8000] {
            let budget = outline_entry_budget_bytes(cap);
            // Fill exactly to the budget with one entry and check the cap.
            let overhead = outline_entry_cost_bytes("");
            let out = finish_outline(vec![RepoOutlineEntry {
                rendered: "x".repeat(budget - overhead),
            }]);
            assert!(out.token_estimate <= cap, "cap {cap}: {}", out.token_estimate);
            assert_eq!(out.token_estimate, payload_tokens(&out));
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
                ..Default::default()
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

    #[test]
    fn default_lists_all_nine_tools() {
        let s = fixture().with_symbol_enrichment(true);
        let names = s.listed_tool_names();
        for tool in super::super::tools::ALL_MCP_TOOLS {
            assert!(
                names.iter().any(|n| n == tool),
                "missing {tool} in {names:?}"
            );
        }
        assert_eq!(names.len(), 9);
    }

    // ── in-process dispatch (Route A: the API-provider tool path) ──────

    /// Every advertised tool must be reachable through
    /// [`GavieroMcpServer::call_tool_in_process`].
    ///
    /// The dispatch is a hand-written `match` (rmcp's `ToolCallContext` cannot
    /// be built without a live session), so a tool added to the router but
    /// forgotten here would be advertised to the model and then fail as
    /// "unknown tool" at call time. This pins the two lists together.
    #[tokio::test]
    async fn every_advertised_tool_is_dispatchable_in_process() {
        let s = fixture().with_symbol_enrichment(true);
        let specs = s.in_process_tool_specs();
        assert_eq!(specs.len(), 9, "expected the full advertised surface");

        for spec in specs {
            // Empty args are not universally invalid (`repo_outline`'s inputs are
            // all optional), so `Ok` is a pass — it proves routing reached the
            // core. Only the dispatch-table miss is a failure.
            match s
                .call_tool_in_process(&spec.name, serde_json::json!({}))
                .await
            {
                Ok(_) => {}
                Err(err) => assert!(
                    !err.contains("not handled by in-process dispatch"),
                    "{} is advertised but missing from the dispatch table: {err}",
                    spec.name
                ),
            }
        }
    }

    #[tokio::test]
    async fn in_process_specs_agree_with_listed_names() {
        let s = fixture().with_symbol_enrichment(true);
        let names: Vec<String> = s.listed_tool_names();
        let spec_names: Vec<String> = s
            .in_process_tool_specs()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, spec_names);
    }

    /// The exposure gate must bind the in-process path too, or an API provider
    /// would reach a tool an MCP client is denied.
    #[tokio::test]
    async fn in_process_call_honours_the_exposure_gate() {
        let s = fixture().with_exposed_tools(vec!["memory_search".into()]);
        s.call_tool_in_process("memory_search", serde_json::json!({}))
            .await
            .expect_err("empty args still fail, but must fail past the gate");
        let err = s
            .call_tool_in_process("blast_radius", serde_json::json!({"paths": ["src/lib.rs"]}))
            .await
            .expect_err("blast_radius is excluded from the exposed set");
        assert!(err.contains("disabled"), "unexpected error: {err}");
    }

    /// Symbol tools require the enrichment sidecar. While it is off they must
    /// be absent from the specs the in-process loop builds its tool list from —
    /// otherwise the model is handed a tool that always errors.
    #[tokio::test]
    async fn in_process_specs_omit_symbol_tools_without_enrichment() {
        let s = fixture();
        let names: Vec<String> = s
            .in_process_tool_specs()
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert!(!names.iter().any(|n| n == "symbol_search"), "{names:?}");
        assert!(!names.iter().any(|n| n == "symbol_doc"), "{names:?}");
        let err = s
            .call_tool_in_process("symbol_search", serde_json::json!({"query": "x"}))
            .await
            .expect_err("enrichment is off");
        assert!(err.contains("disabled"), "unexpected error: {err}");
    }

    /// A name the server does not advertise must not resolve silently. With no
    /// `exposedTools` allow-list the *gate* deliberately passes any name (it
    /// cannot know the router's contents), so the dispatch table itself is the
    /// backstop that rejects it.
    #[tokio::test]
    async fn in_process_call_rejects_an_unadvertised_name() {
        let s = fixture();
        let err = s
            .call_tool_in_process("definitely_not_a_tool", serde_json::json!({}))
            .await
            .expect_err("unknown tool");
        assert!(
            err.contains("not handled by in-process dispatch"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn hidden_tool_absent_from_list_and_rejected_on_call() {
        let s = fixture().with_exposed_tools(vec![
            super::super::tools::TOOL_MEMORY_SEARCH.to_string(),
            super::super::tools::TOOL_MEMORY_PING.to_string(),
        ]);
        let names = s.listed_tool_names();
        assert!(names.contains(&"memory_search".to_string()));
        assert!(names.contains(&"memory_ping".to_string()));
        assert!(!names.contains(&"blast_radius".to_string()));
        assert!(!names.contains(&"node_doc".to_string()));

        match s
            .blast_radius(Parameters(BlastRadiusInput {
                paths: vec!["src/lib.rs".into()],
                ..Default::default()
            }))
            .await
        {
            Err(err) => assert!(
                err.message.contains("disabled"),
                "unexpected error: {}",
                err.message
            ),
            Ok(_) => panic!("hidden blast_radius must be invalid_request"),
        }
    }

    #[test]
    fn memory_ping_stays_listed_when_omitted_from_exposed_set() {
        let s = fixture().with_exposed_tools(vec!["memory_search".into()]);
        let names = s.listed_tool_names();
        assert!(names.contains(&"memory_ping".to_string()));
        assert!(names.contains(&"memory_search".to_string()));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn permissions_deny_hides_tool_from_list() {
        let s = fixture().with_permissions(super::super::McpPermissions {
            allow: vec![],
            deny: vec!["gaviero:blast_radius".into()],
        });
        let names = s.listed_tool_names();
        assert!(!names.contains(&"blast_radius".to_string()));
        assert!(names.contains(&"memory_search".to_string()));
    }

    fn compact_tools_json(server: &GavieroMcpServer) -> (String, String) {
        let v: Vec<serde_json::Value> = server
            .listed_tools()
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                })
            })
            .collect();
        (
            serde_json::to_string(&v).unwrap(),
            serde_json::to_string_pretty(&v).unwrap(),
        )
    }

    #[test]
    fn tools_list_compact_snapshots_and_size_pins() {
        // Pre-shorten compact (name+description, minified) baseline: 2753 bytes
        // (2026-09-14). Full tools/list JSON is ~16 KiB because schemars copies
        // field docs into inputSchema; the 30% / 1200-byte budgets apply to
        // the compact listed-schema the model actually reads as descriptions.
        const PRE_SHORTEN_COMPACT_DEFAULT: usize = 2753;
        let s = fixture();
        let (default_min, default_pretty) = compact_tools_json(&s);
        insta::assert_snapshot!("mcp_tools_list_default", &default_pretty);
        assert!(
            default_min.len() <= PRE_SHORTEN_COMPACT_DEFAULT * 7 / 10,
            "default compact tools/list {} B should be ≤ 70% of {PRE_SHORTEN_COMPACT_DEFAULT}",
            default_min.len()
        );

        let lean = fixture().with_exposed_tools(
            super::super::tools::LEAN_EXPOSED_TOOLS
                .iter()
                .map(|t| t.to_string())
                .collect(),
        );
        let (lean_min, lean_pretty) = compact_tools_json(&lean);
        insta::assert_snapshot!("mcp_tools_list_lean", &lean_pretty);
        assert!(
            lean_min.len() < 1200,
            "lean compact tools/list {} B should be < 1200",
            lean_min.len()
        );
    }
}
