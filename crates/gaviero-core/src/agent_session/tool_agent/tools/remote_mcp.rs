//! `mcp.extraServers` tools for the in-process loop, over
//! [`crate::mcp::client::RemoteMcpServer`].
//!
//! The model-visible name is `mcp__<server>__<tool>` — the convention Claude
//! and dsh's own MCP client use, so `agent.availableTools` entries and prompts
//! read the same for every provider — sanitised to the `[A-Za-z0-9_-]{1,64}`
//! function-name alphabet. The call goes out under the tool's **wire** name.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use crate::mcp::client::{RemoteMcpServer, render_call_result};

use super::{Tool, ToolCtx, ToolOutcome};

/// Per-call ceiling. A foreign server can be slow (a search API, a paper
/// fetch); rmcp sends `notifications/cancelled` when it expires.
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Same cap as gaviero's own MCP adapter (`tools/mcp.rs`).
const MAX_OUTPUT_BYTES: usize = 24_000;

/// Function names are limited to 64 chars of `[A-Za-z0-9_-]`.
const MAX_NAME_LEN: usize = 64;

pub struct RemoteMcpTool {
    server: Arc<RemoteMcpServer>,
    /// Model-visible alias, `mcp__<server>__<tool>`.
    alias: String,
    /// The tool's name on the wire.
    wire_name: String,
    schema: Value,
}

#[async_trait::async_trait]
impl Tool for RemoteMcpTool {
    fn name(&self) -> &str {
        &self.alias
    }

    fn schema(&self) -> Value {
        self.schema.clone()
    }

    async fn run(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        match self.server.call(&self.wire_name, args, CALL_TIMEOUT).await {
            Ok(result) => {
                let mut text = render_call_result(&result);
                truncate_utf8(&mut text, MAX_OUTPUT_BYTES);
                if result.is_error == Some(true) {
                    ToolOutcome::error(text)
                } else {
                    ToolOutcome::ok(text)
                }
            }
            Err(e) => ToolOutcome::error(format!(
                "MCP server '{}' failed on {}: {e:#}",
                self.server.name(),
                self.wire_name
            )),
        }
    }
}

/// Wrap every tool `server` listed, skipping any `allowed` rejects and any
/// alias already in `taken` (which is extended). Collisions after sanitising
/// get a numeric suffix rather than shadowing an existing tool.
pub fn tools_for(
    server: Arc<RemoteMcpServer>,
    allowed: impl Fn(&str) -> bool,
    taken: &mut HashSet<String>,
) -> Vec<Box<dyn Tool>> {
    let mut out: Vec<Box<dyn Tool>> = Vec::new();
    for tool in server.tools() {
        let wire_name = tool.name.to_string();
        if !allowed(&wire_name) {
            continue;
        }
        let alias = unique_alias(&alias_for(server.name(), &wire_name), taken);
        let description = tool
            .description
            .as_deref()
            .unwrap_or_default()
            .to_string();
        let schema = json!({
            "type": "function",
            "function": {
                "name": alias,
                "description": description,
                "parameters": Value::Object((*tool.input_schema).clone()),
            }
        });
        out.push(Box::new(RemoteMcpTool {
            server: Arc::clone(&server),
            alias,
            wire_name,
            schema,
        }));
    }
    out
}

/// `mcp__<server>__<tool>` restricted to `[A-Za-z0-9_-]` and 64 chars.
pub(crate) fn alias_for(server: &str, tool: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let mut alias = format!("mcp__{}__{}", clean(server), clean(tool));
    alias.truncate(MAX_NAME_LEN);
    alias
}

fn unique_alias(base: &str, taken: &mut HashSet<String>) -> String {
    let mut alias = base.to_string();
    let mut n = 2;
    while taken.contains(&alias) {
        let suffix = format!("_{n}");
        let keep = MAX_NAME_LEN.saturating_sub(suffix.len()).min(base.len());
        alias = format!("{}{suffix}", &base[..keep]);
        n += 1;
    }
    taken.insert(alias.clone());
    alias
}

fn truncate_utf8(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push_str("\n... (truncated)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_are_namespaced_and_sanitised() {
        assert_eq!(alias_for("arxiv", "search_papers"), "mcp__arxiv__search_papers");
        assert_eq!(alias_for("semantic scholar", "get.paper"), "mcp__semantic_scholar__get_paper");
        let long = alias_for("server", &"x".repeat(100));
        assert_eq!(long.len(), MAX_NAME_LEN);
    }

    #[test]
    fn colliding_aliases_get_a_suffix() {
        let mut taken = HashSet::new();
        assert_eq!(unique_alias("mcp__a__b", &mut taken), "mcp__a__b");
        assert_eq!(unique_alias("mcp__a__b", &mut taken), "mcp__a__b_2");
        assert_eq!(unique_alias("mcp__a__b", &mut taken), "mcp__a__b_3");
        let long = "y".repeat(MAX_NAME_LEN);
        taken.insert(long.clone());
        let suffixed = unique_alias(&long, &mut taken);
        assert!(suffixed.len() <= MAX_NAME_LEN && suffixed.ends_with("_2"), "{suffixed}");
    }

    #[tokio::test]
    async fn remote_tools_round_trip_under_their_alias() {
        let server = Arc::new(crate::mcp::client::tests::connect_echo("fixture").await);
        let mut taken = HashSet::new();
        let tools = tools_for(server, |_| true, &mut taken);
        assert_eq!(tools.len(), 1);
        let tool = &tools[0];
        assert_eq!(tool.name(), "mcp__fixture__echo");
        assert_eq!(tool.schema()["function"]["name"], "mcp__fixture__echo");
        assert_eq!(tool.schema()["function"]["parameters"]["type"], "object");

        let ctx = ToolCtx {
            workspace_root: std::env::temp_dir(),
            additional_roots: vec![],
            scope: Default::default(),
            snapshot: None,
            policy: Default::default(),
            auto_approve: false,
            observer: None,
            sensitive: Default::default(),
        };
        let out = tool.run(json!({ "text": "ping" }), &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(out.content, "echo:ping");
    }

    #[tokio::test]
    async fn a_denied_tool_is_not_registered() {
        let server = Arc::new(crate::mcp::client::tests::connect_echo("fixture").await);
        let mut taken = HashSet::new();
        assert!(tools_for(server, |name| name != "echo", &mut taken).is_empty());
    }
}
