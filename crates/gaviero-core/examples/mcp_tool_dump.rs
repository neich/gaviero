//! Full outputs of gaviero MCP tool calls on a real tree, for live checks.
//! Builds the code graph of `<root>` into `<graph-db>` (never the
//! workspace's own database), runs each call in `<inputs.json>` through the
//! in-process dispatch, and prints one compact JSON line per call:
//! `{"tool", "args", "ok", "ms", "output"}` or `{"tool", "args", "ok": false,
//! "ms", "error"}`.
//!
//! ```text
//! mcp_tool_dump <root> <graph-db> <inputs.json>
//! ```
//!
//! `inputs.json` is `[{"tool": "...", "args": {...}}, ...]`. The first call
//! pays the cold graph scan; put a warm-up call first.

use std::sync::Arc;
use std::time::Instant;

use gaviero_core::mcp::GavieroMcpServer;
use gaviero_core::memory::MemoryStores;
use gaviero_core::memory::embedder::NullEmbedder;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, db, inputs] = args.as_slice() else {
        anyhow::bail!("usage: mcp_tool_dump <root> <graph-db> <inputs.json>");
    };
    let root = std::path::PathBuf::from(root);
    let calls: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(inputs)?)?;

    let embedder = Arc::new(NullEmbedder::default()) as Arc<dyn gaviero_core::memory::Embedder>;
    let stores = MemoryStores::for_tests_in_memory(embedder)?;
    let server = GavieroMcpServer::with_defaults(stores, root.clone())
        .with_graph_db_path(std::path::PathBuf::from(db));

    for call in calls {
        let tool = call["tool"].as_str().unwrap_or_default().to_string();
        let args = call["args"].clone();
        let started = Instant::now();
        let result = server.call_tool_in_process(&tool, args.clone()).await;
        let ms = (started.elapsed().as_secs_f64() * 10_000.0).round() / 10.0;
        let line = match result {
            Ok(output) => serde_json::json!({
                "tool": tool,
                "args": args,
                "ok": true,
                "ms": ms,
                "output": output,
            }),
            Err(e) => serde_json::json!({"tool": tool, "args": args, "ok": false, "ms": ms, "error": e}),
        };
        println!("{line}");
    }
    Ok(())
}
