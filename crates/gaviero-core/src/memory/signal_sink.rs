//! `WriterHandle`-backed implementation of the MCP memory signal sink.
//!
//! Lives in `memory/` rather than `mcp/` on purpose: `mcp/server.rs`
//! asserts it holds no [`WriterHandle`] and has no path to
//! `store_scoped`. Keeping the only `WriterHandle`-aware implementation
//! on this side of the boundary keeps that claim literally true — the
//! server holds an `Arc<dyn MemorySignalSink>` and nothing more.

use std::sync::Arc;

use crate::mcp::signal::{MemoryFlagOutcome, MemoryFlagRequest, MemorySignalSink};

use super::writer::WriterHandle;

/// Turns a [`MemoryFlagRequest`] into a `WriterMessage::AgentFlag`.
pub struct WriterSignalSink {
    writer: WriterHandle,
}

impl WriterSignalSink {
    pub fn new(writer: WriterHandle) -> Self {
        Self { writer }
    }

    /// Convenience for the two production construction sites, which
    /// always want the sink as a trait object.
    pub fn arc(writer: WriterHandle) -> Arc<dyn MemorySignalSink> {
        Arc::new(Self::new(writer))
    }
}

#[async_trait::async_trait]
impl MemorySignalSink for WriterSignalSink {
    async fn flag(&self, req: MemoryFlagRequest) -> anyhow::Result<MemoryFlagOutcome> {
        self.writer
            .agent_flag(req.memory_id, req.scope_level, req.repo_id, req.reason)
            .await
    }
}
