//! Gaviero as an MCP server (Tier A / A5).
//!
//! Exposes three **read-only** tools to subprocess coding agents:
//! * `memory_search(query, scope_hint?, limit?)` — scoped cascading
//!   search over the workspace memory store. Same retrieval path as
//!   chat injection (Tier S1), but callable mid-turn.
//! * `blast_radius(paths, depth?, mode?)` — impact / callers / tests
//!   affected by one or more source paths. Backed by the code graph.
//! * `node_doc(path)` — Tier D1 schema stub. Returns what's available
//!   today (signatures list) with empty `purpose` fields pending D1.
//!
//! The server runs as an in-process tokio task launched at
//! `Workspace::open` time. A small `gaviero-mcp-shim` binary connects
//! subprocess agents' stdio to the server's Unix domain socket at
//! `<workspace>/.gaviero/mcp.sock`.
//!
//! **Read-only by construction** (Phase 1 invariant): there are no
//! `memory_store` / `memory_update` / `memory_delete` tools. Writes
//! flow through the S2 writer task via transcripts and annotations.

pub mod config_synth;
pub mod external_memory;
pub mod observer;
pub mod server;
pub mod tools;

pub use config_synth::{
    Context7Config, McpConfigSynth, TrustConsent, claude_mcp_config_json, codex_mcp_config_toml,
    synthesize_for_worktree,
};
pub use external_memory::{
    ExternalMemoryServer, detect_external_memory_servers, disable_external_memory_servers,
    import_server_memory_jsonl,
};
pub use observer::{McpCallLogEntry, McpToolCallObserver, NoopMcpObserver};
pub use server::{GavieroMcpServer, McpServerHandle, spawn_mcp_server};
pub use tools::{
    BlastRadiusInput, BlastRadiusOutput, BlastRadiusRelation, MemorySearchInput,
    MemorySearchOutput, MemorySearchResult, NodeDoc, NodeDocInput, TOOL_BLAST_RADIUS,
    TOOL_MEMORY_SEARCH, TOOL_NODE_DOC,
};
