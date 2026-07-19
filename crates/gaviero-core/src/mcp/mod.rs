//! Gaviero as an MCP server (Tier A / A5).
//!
//! Exposes read-only tools to subprocess coding agents:
//! * `memory_search` — merged multi-scope hybrid search over memories
//! * `memory_get` — full stored row for one `memory_search` hit (id + scope)
//! * `blast_radius` — graph impact / callers / tests for file paths
//! * `node_doc` — per-file symbol signatures (+ `qualified_name` for chaining)
//! * `repo_outline` — PageRank-ranked code outline (mid-run `<repo_outline>` pull)
//! * `symbol_search` / `symbol_doc` — semantic symbol retrieval (when enrichment on)
//!
//! The server runs as an in-process tokio task launched at
//! `Workspace::open` time. A small `gaviero-mcp-shim` binary connects
//! subprocess agents' stdio to the server's workspace endpoint
//! ([`McpEndpoint`]): the Unix domain socket `<workspace>/.gaviero/mcp.sock`
//! on Unix, a `\\.\pipe\gaviero-…` named pipe on Windows.
//!
//! **Read-only by construction** (Phase 1 invariant): there are no
//! `memory_store` / `memory_update` / `memory_delete` tools. Writes
//! flow through the S2 writer task via transcripts and annotations.

pub mod config_synth;
pub mod external_memory;
pub mod observer;
pub mod preflight;
pub mod resolver;
pub mod server;
pub mod telemetry_sink;
pub mod tools;
pub mod transport;

pub use config_synth::{
    Context7Config, ExtraMcpServer, ExtraMcpTransport, McpConfigSynth, McpPermissions,
    TrustConsent, claude_mcp_config_json, claude_settings_permissions, codex_mcp_config_toml,
    codex_mcp_overrides_from_config_file, codex_synth_has_any_mcp, codex_synth_has_remote_mcp,
    host_from_mcp_url, mcp_json_has_remote_urls, synth_has_remote_url_servers,
    synthesize_for_worktree, worktree_has_remote_mcp_urls,
};
pub use external_memory::{
    ExternalMemoryServer, detect_external_memory_servers, disable_external_memory_servers,
    import_server_memory_jsonl,
};
pub use observer::{FanOutMcpObserver, McpCallLogEntry, McpToolCallObserver, NoopMcpObserver};
pub use preflight::{
    PreflightOpts, plan_uses_codex, preflight_mcp, shim_binary_resolvable,
    validate_codex_trust_for_extras, validate_synthesized_cursor_remote_mcp,
};
pub use resolver::{
    McpConfigOverrides, extra_servers_from_workspace, extra_urls_from_project_mcp_json,
    parse_mcp_codex_trust_flag, parse_mcp_stdio_flag, parse_mcp_url_flag, resolve_context7_config,
    resolve_mcp_config_synth, resolve_mcp_permissions,
};
pub use server::{GavieroMcpServer, McpServerHandle, spawn_mcp_server};
pub use telemetry_sink::{
    McpCallRecord, NdjsonTelemetrySink, ToolStats, compute_stats, default_telemetry_path,
};
pub use tools::{
    BlastRadiusInput, BlastRadiusOutput, BlastRadiusRelation, MemoryGetInput, MemoryGetOutput,
    MemoryGetRow, MemorySearchInput, MemorySearchOutput, MemorySearchResult, NodeDoc, NodeDocInput,
    NodeDocSymbol, RepoOutlineEntry, RepoOutlineInput, RepoOutlineOutput, SymbolDocInput,
    SymbolDocOutput, SymbolSearchInput, SymbolSearchOutput, TOOL_BLAST_RADIUS, TOOL_MEMORY_GET,
    TOOL_MEMORY_SEARCH, TOOL_NODE_DOC, TOOL_REPO_OUTLINE, TOOL_SYMBOL_DOC, TOOL_SYMBOL_SEARCH,
};
pub use transport::McpEndpoint;
