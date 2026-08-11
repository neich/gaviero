//! The one narrow seam through which an MCP tool can cause a write.
//!
//! `mcp/server.rs` deliberately holds no [`crate::memory::WriterHandle`]
//! and has no path to `store_scoped` — see the module header there. The
//! `memory_flag` tool still needs to *signal* the writer task, so it does
//! so through this trait object: the server knows only "there is a sink";
//! it cannot construct a write, choose a scope, or reach a store.
//!
//! The implementation over `WriterHandle` lives in
//! [`crate::memory::signal_sink`], not here, so the module boundary the
//! server's doc comment asserts stays literally true.

/// One agent-originated "this memory is wrong or stale" signal.
///
/// `scope_level` + `repo_id` identify the owning physical DB. The MCP
/// handler resolves them from the tool's `scope` string before the
/// request is built, so an unresolvable pair never reaches the writer.
#[derive(Debug, Clone)]
pub struct MemoryFlagRequest {
    pub memory_id: i64,
    pub scope_level: i32,
    pub repo_id: Option<String>,
    pub reason: String,
}

/// Decision the writer task reached for a flag. Re-exported from the
/// writer so there is exactly one definition.
pub use crate::memory::writer::AgentFlagOutcome as MemoryFlagOutcome;

/// Sink for memory signals raised by MCP tools.
///
/// Implementations must not write to a store directly — the contract is
/// that they enqueue a [`crate::memory::WriterMessage`] and await its
/// ack, so the Tier S2 single-consumer invariant holds.
///
/// The method is async and returns the writer's decision because the
/// tool has to report it: D1 makes a refused flag a *successful* call
/// with `accepted: false`, which the server can only know after the
/// writer has looked at the row's source.
#[async_trait::async_trait]
pub trait MemorySignalSink: Send + Sync {
    async fn flag(&self, req: MemoryFlagRequest) -> anyhow::Result<MemoryFlagOutcome>;
}
