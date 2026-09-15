//! The gaviero MCP retrieval tools, adapted to the in-process agent loop.
//!
//! Subprocess providers (`claude:`, `codex:`, `cursor:`, `dsh:`) reach
//! `memory_search`, `blast_radius`, `node_doc`, … by speaking MCP to the server
//! the host mounts on their session. In-process API providers (`deepseek:`,
//! `ollama:`) run *this* loop instead and never speak MCP — so before this
//! module they could read the filesystem but had no route to the project's
//! memory, and the system prompt told them to call `blast_radius(path)` anyway.
//!
//! They now call the same server object directly through
//! [`GavieroMcpServer::call_tool_in_process`]. That is the whole adapter: no
//! MCP client, no socket/pipe transport (Windows would need a named pipe), no
//! JSON-RPC framing, and no reconnect logic. Gating is inherited rather than
//! reimplemented — each server core starts with `ensure_tool_allowed`, so
//! `mcp.permissions`, `mcp.gavieroServer.exposedTools`, and the symbol
//! enrichment flag apply here exactly as they do over MCP.
//!
//! Tool *names* stay as the server advertises them (snake_case), unlike the
//! Claude-Code-shaped fs tools (`Read`, `Grep`). They must: the retrieval
//! ("pull") stanza in the system prompt names them verbatim, and
//! [`crate::swarm::backend::RetrievalToolset::from_exposed`] matches them by
//! exact string.
//!
//! Visibility is decided upstream by
//! [`ToolRegistry::extend_mcp`](super::ToolRegistry::extend_mcp), which only
//! appends tools that [`GavieroMcpServer::in_process_tool_specs`] advertises —
//! so this module never needs to know which tools are on.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome};
use crate::mcp::server::{GavieroMcpServer, InProcessToolSpec};

/// Cap on the JSON handed back as a `tool` message.
///
/// `memory_search` and `repo_outline` can return tens of KB; a single such
/// result can crowd out the rest of the turn's context, and the in-process
/// loop has a smaller budget than a full MCP client session. Truncation is
/// announced so the model knows to narrow the query rather than assume the
/// result was complete.
const MAX_OUTPUT_BYTES: usize = 24_000;

/// One gaviero MCP tool, callable from the in-process loop.
pub struct McpTool {
    server: Arc<GavieroMcpServer>,
    name: String,
    /// The OpenAI function-tool schema, built once at construction. Also the
    /// only home of the tool description — the server's own `description` is
    /// spliced into it, not kept alongside it.
    schema: Value,
}

impl McpTool {
    /// Build from a spec the server itself advertised.
    ///
    /// The OpenAI-shaped `schema()` is assembled here rather than stored, and
    /// the MCP `inputSchema` is passed through untouched — the argument keys the
    /// model must produce are exactly the ones the server deserializes (see
    /// [`GavieroMcpServer::call_tool_in_process`]), so rewriting them would
    /// break the call.
    pub fn new(server: Arc<GavieroMcpServer>, spec: InProcessToolSpec) -> Self {
        let schema = json!({
            "type": "function",
            "function": {
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.input_schema,
            }
        });
        Self {
            server,
            name: spec.name,
            schema,
        }
    }
}

#[async_trait::async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    async fn run(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        // `args` is forwarded verbatim: the server owns deserialization, so a
        // malformed argument is reported with the server's own error text
        // instead of a second, drifting copy of the input contract here.
        //
        // `ctx` is deliberately unused. MCP tools resolve and confine paths
        // against the server's own workspace roots; re-checking against this
        // session's `ToolCtx` would add a second policy that could disagree
        // with the one an MCP client gets for the same call.
        match self.server.call_tool_in_process(&self.name, args).await {
            Ok(value) => {
                let text = match serde_json::to_string_pretty(&value) {
                    Ok(t) => t,
                    Err(e) => {
                        return ToolOutcome::error(format!(
                            "{}: serializing result failed: {e}",
                            self.name
                        ));
                    }
                };
                ToolOutcome::ok(truncate(text))
            }
            // `call_tool_in_process` flattens `ErrorData` to `String`, so this
            // covers both a denied tool and a genuine failure inside the tool.
            // The name is prefixed because the model sees only the text.
            Err(e) => ToolOutcome::error(format!("{}: {e}", self.name)),
        }
    }
}

/// Truncate on a char boundary, appending the dropped byte count.
///
/// Byte counts are approximate for multi-byte input; the point is a stable,
/// visibly-incomplete result, not an exact ledger.
fn truncate(mut text: String) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text;
    }
    let dropped = text.len() - MAX_OUTPUT_BYTES;
    let mut cut = MAX_OUTPUT_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    format!(
        "{text}\n\n[truncated: {dropped} bytes omitted. Narrow the query \
         (e.g. a tighter `limit`, a specific `path`, or a narrower `query`) \
         to see the rest.]"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_is_a_noop_below_the_cap() {
        let text = "short".to_string();
        assert_eq!(truncate(text.clone()), text);
    }

    #[test]
    fn truncate_announces_what_it_dropped() {
        let text = "x".repeat(MAX_OUTPUT_BYTES + 100);
        let out = truncate(text);
        assert!(out.len() > MAX_OUTPUT_BYTES, "marker must be appended");
        assert!(out.contains("truncated: 100 bytes omitted"), "{out}");
        assert!(out.ends_with("to see the rest.]"), "{out}");
    }

    /// The cut point must land on a char boundary: slicing mid-codepoint would
    /// panic, and a 3-byte char straddling `MAX_OUTPUT_BYTES` is the realistic
    /// case for retrieval results carrying non-ASCII text.
    #[test]
    fn truncate_lands_on_a_char_boundary() {
        // 9000 3-byte chars = 27 000 bytes, so the cap falls inside a char.
        let text = "→".repeat(9000);
        let out = truncate(text);
        // No panic, and the retained prefix is still valid UTF-8.
        assert!(out.contains("truncated:"));
    }
}
