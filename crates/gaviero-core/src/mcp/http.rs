//! Loopback streamable-HTTP listener for the in-process MCP server (P2).
//!
//! Binds `127.0.0.1:<port>` only. `GET /health` is unauthenticated (reuse
//! detection). `/mcp` requires `Authorization: Bearer` matching the
//! workspace token file. rmcp `allowed_hosts` stays at its loopback
//! default plus `host:port` for this bind.

use std::net::TcpListener as StdTcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde::Serialize;
use rand::RngCore;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use super::server::GavieroMcpServer;
use crate::workspace::{Workspace, identity, settings as S};

pub const HTTP_TOKEN_FILENAME: &str = "mcp-http-token";
pub const CODEX_HTTP_TOKEN_ENV: &str = "GAVIERO_MCP_TOKEN";
const FALLBACK_WINDOW: u16 = 9;

/// Bound (or reused) HTTP endpoint. The bearer token is never logged.
#[derive(Debug, Clone)]
pub struct HttpEndpoint {
    pub url: String,
    pub token: String,
    pub token_path: PathBuf,
    pub port: u16,
    pub workspace_id: String,
}

/// Handle for the HTTP accept loop. `shutdown` cancels the token and
/// awaits the serve task. Reused endpoints have no task.
pub struct HttpListenerHandle {
    cancel: CancellationToken,
    join: Option<tokio::task::JoinHandle<()>>,
    pub endpoint: HttpEndpoint,
    /// `true` when this process owns the TcpListener.
    pub spawned: bool,
}

impl HttpListenerHandle {
    pub async fn shutdown(self) {
        if !self.spawned {
            return;
        }
        self.cancel.cancel();
        if let Some(join) = self.join {
            let _ = join.await;
        }
    }
}

#[derive(Clone)]
struct HealthState {
    workspace_id: String,
    pid: u32,
}

#[derive(Serialize)]
struct HealthBody {
    workspace_id: String,
    pid: u32,
}

/// Preferred port from settings, else [`identity::derive_mcp_http_port`].
/// `0` is rejected.
pub fn resolve_http_port(root: &Path, setting: Option<u64>) -> Result<u16> {
    match setting {
        Some(0) => bail!("mcp.gavieroServer.http.port must not be 0"),
        Some(p) if p > u16::MAX as u64 => {
            bail!("mcp.gavieroServer.http.port {p} is outside 1..=65535")
        }
        Some(p) => Ok(p as u16),
        None => Ok(identity::derive_mcp_http_port(root)),
    }
}

pub fn token_path(root: &Path) -> PathBuf {
    root.join(".gaviero").join(HTTP_TOKEN_FILENAME)
}

/// Read or create `<root>/.gaviero/mcp-http-token` (0600 on Unix).
pub fn ensure_http_token(root: &Path) -> Result<String> {
    let path = token_path(root);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let token = generate_token(root);
    write_token_file(&path, &token)?;
    Ok(token)
}

fn generate_token(_root: &Path) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn write_token_file(path: &Path, token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token).with_context(|| format!("writing {}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Probe `GET http://127.0.0.1:{port}/health`. Matching `workspace_id`
/// means reuse; anything else is a foreign occupant.
pub fn http_health_workspace_id(port: u16) -> Option<String> {
    use std::io::{Read, Write};
    let addr: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = std::net::TcpStream::connect_timeout(
        &addr,
        std::time::Duration::from_millis(200),
    )
    .ok()?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_millis(250)))
        .ok()?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_millis(250)))
        .ok()?;
    let req = format!(
        "GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let body = text.split("\r\n\r\n").nth(1)?;
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("workspace_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Bind loopback HTTP (or reuse a matching occupant) and serve `/health`
/// plus streamable `/mcp`.
pub fn spawn_http_listener(
    server: GavieroMcpServer,
    root: &Path,
    preferred_port: u16,
    token: String,
) -> Result<HttpListenerHandle> {
    let workspace_id = identity::workspace_id_hex16(root);
    let token_path = token_path(root);

    if let Some(occupant) = http_health_workspace_id(preferred_port) {
        if occupant == workspace_id {
            let endpoint = HttpEndpoint {
                url: format!("http://127.0.0.1:{preferred_port}/mcp"),
                token,
                token_path,
                port: preferred_port,
                workspace_id,
            };
            return Ok(HttpListenerHandle {
                cancel: CancellationToken::new(),
                join: None,
                endpoint,
                spawned: false,
            });
        }
        tracing::info!(
            target: "mcp_http",
            port = preferred_port,
            occupant = %occupant,
            "MCP HTTP port occupied by another workspace — falling forward"
        );
    }

    let (std_listener, port) = bind_loopback_window(preferred_port)?;
    std_listener
        .set_nonblocking(true)
        .context("MCP HTTP listener nonblocking")?;
    let listener = tokio::net::TcpListener::from_std(std_listener)
        .context("adopting MCP HTTP listener")?;

    let cancel = CancellationToken::new();
    let url = format!("http://127.0.0.1:{port}/mcp");
    let endpoint = HttpEndpoint {
        url: url.clone(),
        token: token.clone(),
        token_path,
        port,
        workspace_id: workspace_id.clone(),
    };

    let mcp_service = StreamableHttpService::new(
        {
            let server = server.clone();
            move || Ok(server.clone_for_connection())
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_stateful_mode(true)
            .with_json_response(false)
            .with_cancellation_token(cancel.clone())
            .with_allowed_hosts([
                "localhost".to_string(),
                "127.0.0.1".to_string(),
                "::1".to_string(),
                format!("localhost:{port}"),
                format!("127.0.0.1:{port}"),
                format!("[::1]:{port}"),
            ]),
    );

    let health = HealthState {
        workspace_id: workspace_id.clone(),
        pid: std::process::id(),
    };
    let mcp_router = Router::new()
        .fallback_service(mcp_service)
        .layer(middleware::from_fn_with_state(
            token.clone(),
            require_bearer,
        ));
    let app = Router::new()
        .route("/health", get(health_handler))
        .nest("/mcp", mcp_router)
        .with_state(health);

    let cancel_serve = cancel.clone();
    let join = tokio::spawn(async move {
        let serve = axum::serve(listener, app).with_graceful_shutdown(async move {
            cancel_serve.cancelled().await;
        });
        if let Err(e) = serve.await {
            tracing::warn!(target: "mcp_http", error = %e, "MCP HTTP serve exited");
        }
    });

    tracing::info!(target: "mcp_http", %url, "mcp http listening");
    Ok(HttpListenerHandle {
        cancel,
        join: Some(join),
        endpoint,
        spawned: true,
    })
}

/// Spawn the HTTP listener when `mcp.gavieroServer.http.enabled` is true
/// (the default after P2.2). Bind failures are returned to the caller.
pub fn maybe_spawn_http_listener(
    server: GavieroMcpServer,
    root: &Path,
    workspace: &Workspace,
) -> Result<Option<HttpListenerHandle>> {
    let enabled = workspace
        .resolve_setting(S::MCP_GAVIERO_HTTP_ENABLED, Some(root))
        .as_bool()
        .unwrap_or(true);
    if !enabled {
        return Ok(None);
    }
    let port_setting = workspace
        .resolve_setting(S::MCP_GAVIERO_HTTP_PORT, Some(root))
        .as_u64();
    let port = resolve_http_port(root, port_setting)?;
    let token = ensure_http_token(root)?;
    Ok(Some(spawn_http_listener(server, root, port, token)?))
}

/// If a live HTTP listener already serves this workspace, return its
/// coordinates (token from the on-disk file). Used when reusing a pipe.
pub fn reuse_http_endpoint(root: &Path, workspace: &Workspace) -> Option<HttpEndpoint> {
    let enabled = workspace
        .resolve_setting(S::MCP_GAVIERO_HTTP_ENABLED, Some(root))
        .as_bool()
        .unwrap_or(true);
    if !enabled {
        return None;
    }
    let port_setting = workspace
        .resolve_setting(S::MCP_GAVIERO_HTTP_PORT, Some(root))
        .as_u64();
    let port = resolve_http_port(root, port_setting).ok()?;
    let occupant = http_health_workspace_id(port)?;
    if occupant != identity::workspace_id_hex16(root) {
        return None;
    }
    let token = std::fs::read_to_string(token_path(root))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())?;
    Some(HttpEndpoint {
        url: format!("http://127.0.0.1:{port}/mcp"),
        token,
        token_path: token_path(root),
        port,
        workspace_id: occupant,
    })
}

pub fn http_synth_from(endpoint: &HttpEndpoint) -> super::config_synth::HttpSynthEndpoint {
    super::config_synth::HttpSynthEndpoint {
        url: endpoint.url.clone(),
        token: endpoint.token.clone(),
        token_path: endpoint.token_path.clone(),
    }
}

/// Set `GAVIERO_MCP_TOKEN` from the workspace token file when present
/// (Codex HTTP MCP auth; harmless for stdio).
pub fn apply_codex_http_token(cmd: &mut tokio::process::Command, root: &Path) {
    if let Ok(token) = std::fs::read_to_string(token_path(root)) {
        let token = token.trim();
        if !token.is_empty() {
            cmd.env(CODEX_HTTP_TOKEN_ENV, token);
        }
    }
}

fn bind_loopback_window(start: u16) -> Result<(StdTcpListener, u16)> {
    let last = start.saturating_add(FALLBACK_WINDOW).min(65535);
    let mut last_err = None;
    for port in start..=last {
        match StdTcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => return Ok((l, port)),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err
        .map(Into::into)
        .unwrap_or_else(|| anyhow::anyhow!("MCP HTTP bind failed for ports {start}..={last}")))
}

async fn health_handler(State(state): State<HealthState>) -> Json<HealthBody> {
    Json(HealthBody {
        workspace_id: state.workspace_id.clone(),
        pid: state.pid,
    })
}

async fn require_bearer(
    State(expected): State<String>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let some = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    match some {
        Some(header) if bearer_matches(header, &expected) => next.run(request).await,
        _ => (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    }
}

fn bearer_matches(header: &str, token: &str) -> bool {
    let header = header.trim();
    let prefix = "Bearer ";
    if header.len() < prefix.len()
        || !header[..prefix.len()].eq_ignore_ascii_case(prefix)
    {
        return false;
    }
    let got = header[prefix.len()..].trim().as_bytes();
    let exp = token.as_bytes();
    if got.len() != exp.len() {
        return false;
    }
    bool::from(got.ct_eq(exp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::embedder::Embedder;
    use crate::memory::MemoryStores;
    use crate::mcp::server::GavieroMcpServer;
    use anyhow::Result as AResult;

    struct MockEmbedder;
    #[async_trait::async_trait]
    impl Embedder for MockEmbedder {
        fn name(&self) -> &str {
            "mock"
        }
        fn dimension(&self) -> usize {
            8
        }
        async fn embed(
            &self,
            text: &str,
            _purpose: crate::memory::embedder::EmbeddingPurpose,
        ) -> AResult<Vec<f32>> {
            let mut v = vec![0.0f32; 8];
            for (i, b) in text.bytes().enumerate() {
                v[i % 8] += b as f32;
            }
            Ok(v)
        }
    }

    fn fixture_server() -> GavieroMcpServer {
        let embedder = Arc::new(MockEmbedder) as Arc<dyn Embedder>;
        let stores = MemoryStores::for_tests_in_memory(embedder).unwrap();
        GavieroMcpServer::with_defaults(stores, PathBuf::from("/tmp"))
    }

    #[test]
    fn bearer_matches_is_constant_time_length_checked() {
        assert!(bearer_matches("Bearer abc", "abc"));
        assert!(bearer_matches("bearer abc", "abc"));
        assert!(!bearer_matches("Bearer abcd", "abc"));
        assert!(!bearer_matches("Basic abc", "abc"));
        assert!(!bearer_matches("Bearer abc", "xyz"));
    }

    #[test]
    fn resolve_http_port_rejects_zero() {
        let root = Path::new("/tmp");
        assert!(resolve_http_port(root, Some(0)).is_err());
        let derived = resolve_http_port(root, None).unwrap();
        assert!((49152..=65535).contains(&derived));
    }

    #[tokio::test]
    async fn http_health_and_mcp_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let token = "test-token-abcdefghijklmnopqrstuvwxyz".to_string();
        write_token_file(&token_path(dir.path()), &token).unwrap();
        let std_l = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
        let preferred = std_l.local_addr().unwrap().port();
        drop(std_l);

        let handle = spawn_http_listener(
            fixture_server(),
            dir.path(),
            preferred,
            token.clone(),
        )
        .unwrap();
        let port = handle.endpoint.port;
        let client = reqwest::Client::new();

        let health: serde_json::Value = client
            .get(format!("http://127.0.0.1:{port}/health"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            health["workspace_id"].as_str().unwrap(),
            identity::workspace_id_hex16(dir.path())
        );

        let unauth = client
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .header("Host", format!("127.0.0.1:{port}"))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

        let init = client
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .header("Host", format!("127.0.0.1:{port}"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#)
            .send()
            .await
            .unwrap();
        assert!(
            init.status().is_success(),
            "initialize status {}",
            init.status()
        );
        let session = init
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let _ = init.bytes().await;

        let mut list = client
            .post(format!("http://127.0.0.1:{port}/mcp"))
            .header("Host", format!("127.0.0.1:{port}"))
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json, text/event-stream")
            .header("Content-Type", "application/json")
            .body(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#);
        if let Some(sid) = session {
            list = list.header("Mcp-Session-Id", sid);
        }
        let list = list.send().await.unwrap();
        assert!(
            list.status().is_success(),
            "tools/list status {}",
            list.status()
        );

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn busy_port_falls_forward() {
        let dir = tempfile::tempdir().unwrap();
        let occupant = StdTcpListener::bind(("127.0.0.1", 0)).unwrap();
        let occupied = occupant.local_addr().unwrap().port();
        let token = generate_token(dir.path());
        let handle = spawn_http_listener(fixture_server(), dir.path(), occupied, token).unwrap();
        assert_ne!(handle.endpoint.port, occupied);
        handle.shutdown().await;
        drop(occupant);
    }
}
