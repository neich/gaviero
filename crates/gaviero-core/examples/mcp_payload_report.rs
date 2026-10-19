//! Payload sizes of gaviero MCP graph tools on a real tree, for before/after
//! comparisons (XERJ plan v2, X8). Builds the code graph of `<root>` into
//! `<graph-db>` (never the workspace's own database), then runs each call in
//! `<inputs.json>` through the in-process dispatch and through the native
//! tool adapter, and prints one JSON line per call:
//!
//! - `compact_bytes` / `est_tokens`: the logical payload as compact JSON,
//!   `ceil(bytes / 4)` — one copy, no MCP envelope;
//! - `native_bytes` / `native_truncated`: the text the in-process adapter
//!   actually hands the model (pretty-printed, capped at 24,000 bytes).
//!
//! ```text
//! mcp_payload_report <root> <graph-db> <inputs.json>
//! ```
//!
//! `inputs.json` is `[{"tool": "...", "args": {...}}, ...]`. The file only
//! uses API that predates the X-series changes, so the same source builds
//! against the old tree for the "before" numbers.

use std::sync::Arc;

use gaviero_core::agent_session::tool_agent::tools::mcp::McpTool;
use gaviero_core::agent_session::tool_agent::tools::{Tool, ToolCtx};
use gaviero_core::mcp::GavieroMcpServer;
use gaviero_core::memory::MemoryStores;
use gaviero_core::memory::embedder::NullEmbedder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, db, inputs] = args.as_slice() else {
        anyhow::bail!("usage: mcp_payload_report <root> <graph-db> <inputs.json>");
    };
    let root = std::path::PathBuf::from(root);
    let calls: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(inputs)?)?;

    let embedder = Arc::new(NullEmbedder::default()) as Arc<dyn gaviero_core::memory::Embedder>;
    let stores = MemoryStores::for_tests_in_memory(embedder)?;
    let server = Arc::new(
        GavieroMcpServer::with_defaults(stores, root.clone())
            .with_graph_db_path(std::path::PathBuf::from(db)),
    );
    let specs = server.in_process_tool_specs();
    let ctx = ToolCtx {
        workspace_root: root.clone(),
        additional_roots: vec![],
        scope: Default::default(),
        snapshot: None,
        policy: Default::default(),
        auto_approve: false,
        observer: None,
        sensitive: Default::default(),
    };

    for call in calls {
        let tool = call["tool"].as_str().unwrap_or_default().to_string();
        let args = call["args"].clone();
        let logical = server.call_tool_in_process(&tool, args.clone()).await;
        let spec = specs.iter().find(|s| s.name == tool).cloned();
        let native = match spec {
            Some(spec) => Some(McpTool::new(server.clone(), spec).run(args.clone(), &ctx).await),
            None => None,
        };
        let line = match logical {
            Ok(value) => {
                let compact = serde_json::to_string(&value)?.len();
                serde_json::json!({
                    "tool": tool,
                    "args": args,
                    "ok": true,
                    "compact_bytes": compact,
                    "est_tokens": compact.div_ceil(4),
                    "native_bytes": native.as_ref().map(|n| n.content.len()),
                    "native_truncated": native.as_ref().map(|n| n.content.contains("[truncated:")),
                })
            }
            Err(e) => serde_json::json!({"tool": tool, "args": args, "ok": false, "error": e}),
        };
        println!("{line}");
    }
    Ok(())
}
