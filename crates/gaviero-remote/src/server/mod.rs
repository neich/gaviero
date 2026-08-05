//! WSS sidecar server (Plan A unit A2). D1 resolved to in-process rustls:
//! TLS terminates here via `axum-server`.
//!
//! Topology (Plan §2.3): the axum layer authenticates and upgrades; each
//! socket runs a [`conn`] task; a single [`hub::RemoteHub`] actor owns the
//! active client generation, sequence numbers, per-conversation chunk
//! coalescing, the bounded outbound path, and eviction. The hub never sees
//! `App` — it speaks channels of protocol types only (invariant 1).

mod conn;
mod hub;

pub use hub::{HubInput, HubOutput};

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use subtle::ConstantTimeEq;
use tokio::sync::mpsc;

use crate::dto::{Limits, WorkspaceInfo};

pub struct RemoteServerConfig {
    pub bind_addr: SocketAddr,
    /// Additional listeners sharing the same hub and TLS material —
    /// §3.2 binds loopback AND detected tailnet addresses, never a
    /// wildcard. Empty for tests.
    pub extra_bind_addrs: Vec<SocketAddr>,
    pub tls_cert_pem: Vec<u8>,
    pub tls_key_pem: Vec<u8>,
    pub token: String,
    /// Random per TUI launch; not workspace-derived.
    pub instance_id: String,
    pub tui_version: String,
    pub workspace: WorkspaceInfo,
    pub capabilities: Vec<String>,
    pub confirm_required: Vec<String>,
    pub allowed_slash_commands: Vec<String>,
    pub limits: Limits,
    /// Wire defaults: ping 20 s, idle 60 s. Configurable for tests only.
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
    pub hello_timeout: Duration,
}

impl RemoteServerConfig {
    /// Production timing defaults (§3.5).
    pub fn timing_defaults() -> (Duration, Duration, Duration) {
        (
            Duration::from_secs(20),
            Duration::from_secs(60),
            Duration::from_secs(10),
        )
    }
}

#[derive(Debug)]
pub enum ServerError {
    Tls(std::io::Error),
    Bind,
}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::Tls(e) => write!(f, "TLS configuration failed: {e}"),
            ServerError::Bind => write!(f, "could not bind the remote listener"),
        }
    }
}

impl std::error::Error for ServerError {}

/// Host-side handle: the TUI event loop feeds the hub with `try_send` only
/// (invariant 11 — never `.await` a bounded channel inside `handle`).
#[derive(Clone)]
pub struct RemoteHandle {
    input_tx: mpsc::Sender<HubInput>,
}

impl RemoteHandle {
    pub fn try_send(&self, input: HubInput) -> Result<(), mpsc::error::TrySendError<HubInput>> {
        self.input_tx.try_send(input)
    }
}

pub struct SpawnedServer {
    pub handle: RemoteHandle,
    /// Decoded, deduplicated, rate-limited client commands plus connection
    /// lifecycle. Bounded; the host drains it into its own event channel.
    pub outputs: mpsc::Receiver<HubOutput>,
    pub local_addr: SocketAddr,
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub token: Arc<Mutex<String>>,
    pub max_frame_bytes: usize,
    pub hello_timeout: Duration,
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
    pub registration_tx: mpsc::Sender<conn::Registration>,
    pub inbound_tx: mpsc::Sender<conn::ConnIn>,
}

pub async fn spawn(config: RemoteServerConfig) -> Result<SpawnedServer, ServerError> {
    let rustls_config = axum_server::tls_rustls::RustlsConfig::from_pem(
        config.tls_cert_pem.clone(),
        config.tls_key_pem.clone(),
    )
    .await
    .map_err(ServerError::Tls)?;

    // Bounds per Plan §6.1.
    let (registration_tx, registration_rx) = mpsc::channel(4);
    let (inbound_tx, inbound_rx) = mpsc::channel(64);
    let (input_tx, input_rx) = mpsc::channel(256);
    let (output_tx, output_rx) = mpsc::channel(64);

    let token = Arc::new(Mutex::new(config.token.clone()));

    let state = AppState {
        token: token.clone(),
        max_frame_bytes: config.limits.max_frame_bytes as usize,
        hello_timeout: config.hello_timeout,
        ping_interval: config.ping_interval,
        idle_timeout: config.idle_timeout,
        registration_tx,
        inbound_tx,
    };

    let app = Router::new()
        .route(crate::WS_PATH, any(ws_handler))
        .with_state(state);

    let axum_handle = axum_server::Handle::new();
    let server = axum_server::bind_rustls(config.bind_addr, rustls_config.clone())
        .handle(axum_handle.clone());
    tokio::spawn(server.serve(app.clone().into_make_service()));
    let local_addr = axum_handle.listening().await.ok_or(ServerError::Bind)?;

    // §3.2: extra listeners (loopback + tailnet) share the hub. A failed
    // extra bind is reported by log, not fatal — the primary carries the QR.
    let mut handles = vec![axum_handle];
    for addr in &config.extra_bind_addrs {
        let handle = axum_server::Handle::new();
        let server = axum_server::bind_rustls(*addr, rustls_config.clone())
            .handle(handle.clone());
        tokio::spawn(server.serve(app.clone().into_make_service()));
        if handle.listening().await.is_none() {
            tracing::warn!(%addr, "extra remote listener failed to bind");
            continue;
        }
        handles.push(handle);
    }

    let hub = hub::RemoteHub::new(config, token, registration_rx, inbound_rx, input_rx, output_tx, handles);
    tokio::spawn(hub.run());

    Ok(SpawnedServer {
        handle: RemoteHandle { input_tx },
        outputs: output_rx,
        local_addr,
    })
}

/// Bearer + subprotocol are checked *before* the upgrade; a failed check can
/// never evict the live client (§3.5).
async fn ws_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let authorized = match presented {
        Some(p) => {
            let current = state.token.lock().expect("token lock");
            p.as_bytes().ct_eq(current.as_bytes()).into()
        }
        None => false,
    };
    if !authorized {
        // Deliberately unspecific: no validation internals in errors (§5.2).
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    let requested = headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !requested
        .split(',')
        .map(str::trim)
        .any(|p| p == crate::SUBPROTOCOL)
    {
        return (StatusCode::BAD_REQUEST, "unsupported subprotocol").into_response();
    }

    // Hard transport ceiling; the precise 4004 close for frames over
    // max_frame_bytes is enforced in the conn task below this limit.
    let transport_cap = state.max_frame_bytes.saturating_mul(2).max(64 * 1024);
    ws.protocols([crate::SUBPROTOCOL])
        .max_message_size(transport_cap)
        .on_upgrade(move |socket| conn::run(socket, state))
}
