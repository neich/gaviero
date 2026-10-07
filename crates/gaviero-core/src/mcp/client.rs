//! In-process MCP **client** for `mcp.extraServers`.
//!
//! Subprocess providers reach an extra server because gaviero writes it into
//! their config (Claude/Codex/Cursor) or hands it over on `session/new` (dsh).
//! The in-process tool loop (`deepseek:`) has no such vendor, and DeepSeek's
//! API ignores MCP at every layer, so the host has to be the client. This is
//! rmcp's client half — the same crate the server half already uses — with
//! transports gaviero owns:
//!
//! * **stdio** — spawned through [`crate::util::spawn::agent_command`]
//!   (PATHEXT resolution, batch-shim bypass, isolated console; the process-wide
//!   kill-tree Job Object covers it on Windows) and wrapped in
//!   `AsyncRwTransport`, instead of rmcp's `transport-child-process`, which
//!   would bypass that discipline and add `process-wrap`;
//! * **streamable HTTP** — [`super::client_http::ReqwestHttpClient`] over the
//!   workspace's `reqwest 0.12`.
//!
//! Connection is bounded by [`CONNECT_TIMEOUT`] (initialize + paginated
//! `tools/list`), each call by a per-request timeout that also sends MCP's
//! `notifications/cancelled` on expiry (rmcp's `RequestHandle::await_response`).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResult, ClientRequest, ServerResult,
};
use rmcp::service::{PeerRequestOptions, RunningService};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::transport::streamable_http_client::{
    StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
};
use rmcp::{RoleClient, ServiceExt};
use serde_json::Value;

use super::client_http::ReqwestHttpClient;
use super::{ExtraMcpServer, ExtraMcpTransport};

/// Upper bound on `initialize` + `tools/list` for one server.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A connected foreign MCP server and the tools it listed at connect time.
///
/// The tool list is a snapshot: the in-process session connects once, on its
/// first turn, so a `tools/list_changed` takes effect next session.
pub struct RemoteMcpServer {
    name: String,
    service: RunningService<RoleClient, ()>,
    tools: Vec<rmcp::model::Tool>,
    /// The stdio child, held so it lives exactly as long as the connection
    /// (`kill_on_drop`). `None` for HTTP servers.
    _child: Option<tokio::process::Child>,
}

impl RemoteMcpServer {
    /// Connect to `server` and list its tools, within [`CONNECT_TIMEOUT`].
    /// `cwd` is the working directory for a stdio server.
    pub async fn connect(server: &ExtraMcpServer, cwd: &Path) -> Result<Self> {
        tokio::time::timeout(CONNECT_TIMEOUT, Self::connect_inner(server, cwd))
            .await
            .map_err(|_| anyhow!("timed out after {}s", CONNECT_TIMEOUT.as_secs()))?
    }

    async fn connect_inner(server: &ExtraMcpServer, cwd: &Path) -> Result<Self> {
        let (service, child) = match &server.transport {
            ExtraMcpTransport::Stdio { command, args } => {
                let mut cmd = crate::util::spawn::agent_command(command);
                cmd.args(args)
                    .current_dir(cwd)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                let mut child = cmd
                    .spawn()
                    .with_context(|| format!("spawning `{command}`"))?;
                let stdout = child.stdout.take().context("child stdout")?;
                let stdin = child.stdin.take().context("child stdin")?;
                let service = ()
                    .serve(AsyncRwTransport::new_client(stdout, stdin))
                    .await
                    .context("MCP initialize")?;
                (service, Some(child))
            }
            ExtraMcpTransport::Url { url } => {
                let transport = StreamableHttpClientTransport::with_client(
                    ReqwestHttpClient::new(),
                    StreamableHttpClientTransportConfig::with_uri(url.clone()),
                );
                let service = ().serve(transport).await.context("MCP initialize")?;
                (service, None)
            }
        };
        Self::from_service(&server.name, service, child).await
    }

    /// Finish a connection over an already-initialized client: list tools.
    pub(crate) async fn from_service(
        name: &str,
        service: RunningService<RoleClient, ()>,
        child: Option<tokio::process::Child>,
    ) -> Result<Self> {
        let tools = service
            .peer()
            .list_all_tools()
            .await
            .context("MCP tools/list")?;
        Ok(Self {
            name: name.to_string(),
            service,
            tools,
            _child: child,
        })
    }

    /// The server's name from `mcp.extraServers`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Tools the server listed at connect time.
    pub fn tools(&self) -> &[rmcp::model::Tool] {
        &self.tools
    }

    /// Call `tool` (its MCP wire name) with `args`, bounded by `timeout`.
    pub async fn call(&self, tool: &str, args: Value, timeout: Duration) -> Result<CallToolResult> {
        let mut params = CallToolRequestParams::new(tool.to_string());
        match args {
            Value::Object(map) => params = params.with_arguments(map),
            Value::Null => {}
            other => bail!("tool arguments must be a JSON object, got {other}"),
        }
        let mut options = PeerRequestOptions::no_options();
        options.timeout = Some(timeout);
        let handle = self
            .service
            .peer()
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(params)),
                options,
            )
            .await?;
        match handle.await_response().await? {
            ServerResult::CallToolResult(result) => Ok(result),
            other => bail!("unexpected response to tools/call: {other:?}"),
        }
    }

    /// Close the connection (and, for stdio, the child) now rather than on drop.
    pub async fn close(self) {
        if let Err(e) = self.service.cancel().await {
            tracing::debug!(server = %self.name, "MCP client shutdown: {e}");
        }
    }
}

/// Render a `tools/call` result as the text the model sees: text parts
/// verbatim, other parts as a one-line placeholder, and `structured_content`
/// when no text part carried it.
pub fn render_call_result(result: &CallToolResult) -> String {
    let mut parts: Vec<String> = Vec::new();
    for content in &result.content {
        match content.as_text() {
            Some(text) => parts.push(text.text.clone()),
            None => parts.push("[non-text MCP content omitted]".to_string()),
        }
    }
    if parts.is_empty()
        && let Some(structured) = &result.structured_content
    {
        parts.push(structured.to_string());
    }
    parts.join("\n")
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use rmcp::model::{
        Content, ErrorData, ListToolsResult, PaginatedRequestParams, ServerCapabilities,
        ServerInfo,
    };
    use rmcp::service::RequestContext;
    use rmcp::{RoleServer, ServerHandler};

    use super::*;

    /// A one-tool MCP server: `echo { text }` → `echo:<text>`.
    #[derive(Clone)]
    pub(crate) struct EchoServer;

    impl ServerHandler for EchoServer {
        fn get_info(&self) -> ServerInfo {
            let mut info = ServerInfo::default();
            info.capabilities = ServerCapabilities::builder().enable_tools().build();
            info
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            let schema = serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } }
            });
            let schema = schema.as_object().cloned().unwrap_or_default();
            let mut result = ListToolsResult::default();
            result.tools = vec![rmcp::model::Tool::new(
                "echo",
                "Echo the text back.",
                Arc::new(schema),
            )];
            Ok(result)
        }

        async fn call_tool(
            &self,
            request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResult, ErrorData> {
            let text = request
                .arguments
                .as_ref()
                .and_then(|a| a.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            Ok(CallToolResult::success(vec![Content::text(format!(
                "echo:{text}"
            ))]))
        }
    }

    /// Serve [`EchoServer`] over an in-memory duplex and connect a client to
    /// it — the stdio code path without spawning a process.
    pub(crate) async fn connect_echo(name: &str) -> RemoteMcpServer {
        let (client_end, server_end) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            let (r, w) = tokio::io::split(server_end);
            if let Ok(running) = EchoServer.serve((r, w)).await {
                let _ = running.waiting().await;
            }
        });
        let (r, w) = tokio::io::split(client_end);
        let service = ()
            .serve(AsyncRwTransport::new_client(r, w))
            .await
            .expect("client initialize");
        RemoteMcpServer::from_service(name, service, None)
            .await
            .expect("tools/list")
    }

    #[tokio::test]
    async fn lists_and_calls_a_tool_over_an_async_rw_transport() {
        let server = connect_echo("fixture").await;
        assert_eq!(server.name(), "fixture");
        let names: Vec<&str> = server.tools().iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(names, vec!["echo"]);

        let result = server
            .call("echo", serde_json::json!({ "text": "hi" }), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(render_call_result(&result), "echo:hi");
        server.close().await;
    }

    /// The streamable-HTTP leg end to end: rmcp's HTTP server hosting
    /// [`EchoServer`] on loopback, reached through `ReqwestHttpClient`.
    #[tokio::test]
    async fn lists_and_calls_a_tool_over_streamable_http() {
        use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
        use rmcp::transport::streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService,
        };

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let cancel = tokio_util::sync::CancellationToken::new();
        let service = StreamableHttpService::new(
            || Ok(EchoServer),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default()
                .with_stateful_mode(true)
                .with_cancellation_token(cancel.clone())
                .with_allowed_hosts([format!("127.0.0.1:{port}"), "127.0.0.1".to_string()]),
        );
        let app = axum::Router::new().nest_service("/mcp", service);
        let shutdown = cancel.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async move { shutdown.cancelled().await })
                .await;
        });

        let server = ExtraMcpServer {
            name: "web".into(),
            transport: ExtraMcpTransport::Url {
                url: format!("http://127.0.0.1:{port}/mcp"),
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let remote = RemoteMcpServer::connect(&server, dir.path())
            .await
            .expect("connect over streamable HTTP");
        assert_eq!(remote.tools().len(), 1);
        let result = remote
            .call("echo", serde_json::json!({ "text": "http" }), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(render_call_result(&result), "echo:http");
        remote.close().await;
        cancel.cancel();
    }

    #[tokio::test]
    async fn a_missing_stdio_command_fails_to_connect() {
        let server = ExtraMcpServer {
            name: "ghost".into(),
            transport: ExtraMcpTransport::Stdio {
                command: "gaviero-no-such-mcp-server-binary".into(),
                args: vec![],
            },
        };
        let dir = tempfile::tempdir().unwrap();
        assert!(RemoteMcpServer::connect(&server, dir.path()).await.is_err());
    }
}
