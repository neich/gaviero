//! Gaviero as an MCP server (Tier A / A5).
//!
//! Nine tools for subprocess coding agents — eight read-only:
//! * `memory_search` — merged multi-scope hybrid search over memories
//!   (repo + workspace + global; `module` / `run` need per-file / per-run
//!   identity that does not cross the shim)
//! * `memory_get` — full stored row for one `memory_search` hit (id + scope)
//! * `memory_ping` — reach-probe ping; records nonce+depth on an in-memory
//!   ledger and never touches the writer
//! * `blast_radius` — graph impact / callers / tests for file paths
//! * `node_doc` — per-file symbol signatures (+ `qualified_name` for chaining)
//! * `repo_outline` — PageRank-ranked code outline (mid-run `<repo_outline>` pull)
//! * `symbol_search` / `symbol_doc` — semantic symbol retrieval (when enrichment on)
//!
//! …plus one that is write-adjacent:
//! * `memory_flag` — demote a stale memory's trust. Creates and deletes
//!   nothing; refuses user-authored and History rows; idempotent; every
//!   applied flag writes a reversible audit row.
//!
//! The server runs as an in-process tokio task launched at
//! `Workspace::open` time. A small `gaviero-mcp-shim` binary connects
//! subprocess agents' stdio to the server's workspace endpoint
//! ([`McpEndpoint`]): the Unix domain socket `<workspace>/.gaviero/mcp.sock`
//! on Unix, a `\\.\pipe\gaviero-…` named pipe on Windows. A loopback
//! streamable-HTTP listener ([`http`]) binds `127.0.0.1` with a bearer
//! token; `gaviero-mcp-shim --resolve` finds the live endpoint via
//! `.gaviero/mcp-endpoint.json`.
//!
//! ## The invariant, and the posture
//!
//! **Invariant (#11): every write goes through the Tier S2 writer
//! task.** That is the safety property, and it holds without exception.
//! Its enforcement at *this* boundary is that [`server`] holds no
//! [`crate::memory::WriterHandle`] and has no path to `store_scoped`;
//! `memory_flag` signals through the narrow
//! [`signal::MemorySignalSink`] trait object, which enqueues a
//! `WriterMessage` and awaits its ack like every other writer client.
//!
//! **Default posture (was "#12: the MCP surface is read-only"): keep the
//! surface read-only unless a tool earns its place.** This is a channel
//! choice, not a consequence of #11, and it has a price: each tool adds
//! ~150–250 tokens of schema to *every* subprocess system prompt on
//! *every* turn, paid whether or not any agent calls it. Weigh that cost
//! per tool rather than treating the whole class as forbidden.
//!
//! Provenance: the original read-only rule was stated in
//! `tier-a-part-2-surface.md` §A5, landed in `f11eca9` (2026-04-26) and
//! since deleted; recover it with
//! `git show f11eca9:tier-a-part-2-surface.md`. Its stated justification
//! — "Expose writes via MCP: reintroduces LLM-as-writer failure mode
//! that Cursor retreated from" — is uncited, and the trust argument does
//! not hold: trust is source-keyed ([`crate::memory::trust_defaults`]),
//! not channel-keyed. A row written via MCP carries whatever trust its
//! `MemorySource` implies, exactly as it would from any other channel.
//!
//! What is *not* negotiable: no tool may write to a store directly, and
//! nothing here bypasses the writer task or the Write Gate.

pub mod config_synth;
pub mod endpoint_file;
pub mod external_memory;
pub mod http;
mod legacy_handshake;
pub mod observer;
pub mod preflight;
pub mod probe;
pub mod reach;
pub mod reach_probe;
pub mod resolver;
pub mod server;
pub mod signal;
pub mod telemetry_sink;
pub mod tools;
pub mod transport;
pub mod user_scope;

pub use config_synth::{
    BashPermissions, Context7Config, ExtraMcpServer, ExtraMcpTransport, HttpSynthEndpoint,
    ManagedRules, McpConfigSynth, McpPermissions, McpTransportChoice, McpTransportKind,
    TrustConsent, claude_mcp_config_json,
    claude_settings_permissions, codex_mcp_config_toml, codex_mcp_overrides_from_config_file,
    codex_synth_has_any_mcp, codex_synth_has_remote_mcp, host_from_mcp_url,
    mcp_json_has_remote_urls, synth_has_remote_url_servers, synthesize_for_worktree,
    worktree_has_remote_mcp_urls,
};
pub use endpoint_file::{
    McpEndpointDescriptor, find_descriptor_upwards, read_descriptor, remove_descriptor,
    write_descriptor, write_listener_descriptor,
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
pub use probe::{PingRecord, ProbeLedger, ping_receipt};
pub use reach::{
    NestingPolicy, ReachPolicy, ReachRecord, ReachStore, filter_claude_tools, format_mcp_status,
    format_reach_table, push_codex_multi_agent_override,
};
pub use reach_probe::{
    ProviderReachResult, ReachProbeConfig, ReachReport, ReachTransport, ReachVerdict,
    classify_verdict, probe_prompt, run_reach_probe,
};
pub use resolver::{
    McpConfigOverrides, extra_servers_from_workspace, extra_urls_from_project_mcp_json,
    parse_mcp_codex_trust_flag, parse_mcp_stdio_flag, parse_mcp_url_flag, resolve_bash_permissions,
    resolve_context7_config, resolve_exposed_tools, resolve_mcp_config_synth,
    resolve_mcp_permissions, resolve_shim_binary, sibling_shim_path,
};
pub use server::{GavieroMcpServer, McpServerHandle, spawn_mcp_server};
pub use signal::{MemoryFlagOutcome, MemoryFlagRequest, MemorySignalSink};
pub use telemetry_sink::{
    McpCallRecord, NdjsonTelemetrySink, ToolStats, compute_stats, default_telemetry_path,
};
pub use tools::{
    ALL_MCP_TOOLS, BlastRadiusInput, BlastRadiusOutput, BlastRadiusRelation, LEAN_EXPOSED_TOOLS,
    MemoryFlagInput, MemoryFlagOutput, MemoryGetInput, MemoryGetOutput, MemoryGetRow,
    MemoryPingInput, MemoryPingOutput, MemorySearchInput, MemorySearchOutput, MemorySearchResult,
    NodeDoc, NodeDocInput, NodeDocSymbol, RepoOutlineEntry, RepoOutlineInput, RepoOutlineOutput,
    SymbolDocInput, SymbolDocOutput, SymbolSearchInput, SymbolSearchOutput, TOOL_BLAST_RADIUS,
    TOOL_MEMORY_FLAG, TOOL_MEMORY_GET, TOOL_MEMORY_PING, TOOL_MEMORY_SEARCH, TOOL_NODE_DOC,
    TOOL_REPO_OUTLINE, TOOL_SYMBOL_DOC, TOOL_SYMBOL_SEARCH,
};
pub use http::{
    CODEX_HTTP_TOKEN_ENV, HttpEndpoint, HttpListenerHandle, ensure_http_token,
    http_health_workspace_id, http_synth_from, maybe_spawn_http_listener, resolve_http_port,
    reuse_http_endpoint, spawn_http_listener, token_path, apply_codex_http_token,
};
pub use transport::McpEndpoint;
pub use user_scope::{
    USER_SCOPE_SERVER_NAME, UserScopeOutcome, UserScopeVendor, default_shim_path,
    register_user_scope, unregister_user_scope, user_scope_registered,
};
