//! MCP tool-call observability (Tier A / A5).
//!
//! Every tool invocation fires `McpToolCallObserver::on_tool_call` with
//! the tool name, JSON input, JSON output, and elapsed duration. The
//! TUI memory panel (A4) surfaces these in a sub-pane so the user can
//! see mid-turn MCP activity. The anti-pattern "log MCP reads as
//! tool-output memories" is avoided here — nothing touches the writer
//! task.

use std::sync::Arc;
use std::time::Duration;

/// One observable MCP tool invocation. Input/output are raw JSON so
/// new tools don't require an observer-trait bump.
#[derive(Debug, Clone)]
pub struct McpCallLogEntry {
    pub tool_name: String,
    pub input: serde_json::Value,
    pub output: serde_json::Value,
    pub duration: Duration,
    pub error: Option<String>,
}

/// Fired after every `tools/call`. Implementations MUST be cheap —
/// the tool response waits on this callback before the MCP reply
/// returns. A slow observer slows down the subprocess agent.
pub trait McpToolCallObserver: Send + Sync {
    fn on_tool_call(&self, entry: &McpCallLogEntry);
}

/// Fallback observer that drops every event. Useful in tests and
/// headless runs where tool-call logging isn't wired.
pub struct NoopMcpObserver;

impl McpToolCallObserver for NoopMcpObserver {
    fn on_tool_call(&self, _entry: &McpCallLogEntry) {}
}

/// Fan-out observer: forwards each tool-call event to every wrapped
/// observer in registration order. `GavieroMcpServer::new` accepts a
/// single observer, so the host composes (e.g.) the TUI audit-panel
/// observer with the NDJSON telemetry sink behind one slot. Each sink
/// must stay cheap — they run on the tool-response path in sequence.
pub struct FanOutMcpObserver {
    sinks: Vec<Arc<dyn McpToolCallObserver>>,
}

impl FanOutMcpObserver {
    pub fn new(sinks: Vec<Arc<dyn McpToolCallObserver>>) -> Self {
        Self { sinks }
    }
}

impl McpToolCallObserver for FanOutMcpObserver {
    fn on_tool_call(&self, entry: &McpCallLogEntry) {
        for sink in &self.sinks {
            sink.on_tool_call(entry);
        }
    }
}
