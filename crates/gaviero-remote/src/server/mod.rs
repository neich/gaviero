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
pub mod registry;

pub use hub::{HubInput, HubOutput};
pub use registry::RegistryConfig;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, watch};

use crate::dto::{Limits, MachineInfo, WorkspaceInfo};

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
    /// 1.1 `hello.machine`; `None` keeps the 1.0 shape.
    pub machine: Option<MachineInfo>,
    /// Plan C invariant 15: when set, the hub re-reads this file every
    /// `token_poll_interval` and, if its trimmed content differs from the
    /// accepted token, performs the `TokenRotated` transition (close 4006).
    /// This is how a rotation in one TUI reaches every instance sharing the
    /// machine token.
    pub token_path: Option<std::path::PathBuf>,
    pub token_poll_interval: Duration,
    /// Machine registry heartbeat. `None` ⇒ no writes (invariant 17).
    pub registry: Option<RegistryConfig>,
    /// Bind addresses for the directory-port leader listener (same TLS and
    /// token). Empty disables the directory listener.
    pub directory_bind_addrs: Vec<SocketAddr>,
    pub heartbeat_interval: Duration,
    pub directory_retry_interval: Duration,
    pub stale_after: Duration,
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

    /// Production token-file poll cadence (Plan C §2.2).
    pub const TOKEN_POLL_INTERVAL: Duration = Duration::from_secs(5);
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
    /// True while this process holds the machine directory port.
    pub directory_leader: Arc<AtomicBool>,
}

#[derive(Clone)]
pub(crate) struct SharedState {
    pub token: Arc<Mutex<String>>,
    pub max_frame_bytes: usize,
    pub hello_timeout: Duration,
    pub ping_interval: Duration,
    pub idle_timeout: Duration,
    pub registration_tx: mpsc::Sender<conn::Registration>,
    pub inbound_tx: mpsc::Sender<conn::ConnIn>,
    registry_dir: Option<std::path::PathBuf>,
    host: String,
    stale_after: Duration,
}

#[derive(Clone)]
pub(crate) struct AppState {
    shared: SharedState,
    /// Per-listener token bucket for `GET /v1/instances` (10/s).
    http_rate: Arc<Mutex<HttpRate>>,
}

impl std::ops::Deref for AppState {
    type Target = SharedState;
    fn deref(&self) -> &SharedState {
        &self.shared
    }
}

struct HttpRate {
    tokens: f64,
    cap: f64,
    refilled: Instant,
}

impl HttpRate {
    fn new(per_sec: u32) -> Self {
        let cap = per_sec as f64;
        Self {
            tokens: cap,
            cap,
            refilled: Instant::now(),
        }
    }

    fn take(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.refilled).as_secs_f64();
        self.refilled = now;
        self.tokens = (self.tokens + elapsed * self.cap).min(self.cap);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn instance_router(shared: SharedState) -> Router {
    let state = AppState {
        shared,
        http_rate: Arc::new(Mutex::new(HttpRate::new(
            registry::DIRECTORY_RATE_PER_SECOND,
        ))),
    };
    Router::new()
        .route(crate::WS_PATH, any(ws_handler))
        .route(crate::INSTANCES_PATH, get(instances_handler))
        .with_state(state)
}

fn directory_router(shared: SharedState) -> Router {
    let state = AppState {
        shared,
        http_rate: Arc::new(Mutex::new(HttpRate::new(
            registry::DIRECTORY_RATE_PER_SECOND,
        ))),
    };
    Router::new()
        .route(crate::INSTANCES_PATH, get(instances_handler))
        .with_state(state)
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
    let host = config
        .machine
        .as_ref()
        .map(|m| m.host.clone())
        .unwrap_or_default();
    let shared = SharedState {
        token: token.clone(),
        max_frame_bytes: config.limits.max_frame_bytes as usize,
        hello_timeout: config.hello_timeout,
        ping_interval: config.ping_interval,
        idle_timeout: config.idle_timeout,
        registration_tx,
        inbound_tx,
        registry_dir: config.registry.as_ref().map(|r| r.dir.clone()),
        host,
        stale_after: config.stale_after,
    };

    let axum_handle = axum_server::Handle::new();
    let server = axum_server::bind_rustls(config.bind_addr, rustls_config.clone())
        .handle(axum_handle.clone());
    tokio::spawn(server.serve(instance_router(shared.clone()).into_make_service()));
    let local_addr = axum_handle.listening().await.ok_or(ServerError::Bind)?;

    // §3.2: extra listeners (loopback + tailnet) share the hub. A failed
    // extra bind is reported by log, not fatal — the primary carries the QR.
    // Each listener gets its own HTTP rate bucket (Plan C §5.2).
    let mut handles = vec![axum_handle];
    for addr in &config.extra_bind_addrs {
        let handle = axum_server::Handle::new();
        let server = axum_server::bind_rustls(*addr, rustls_config.clone())
            .handle(handle.clone());
        tokio::spawn(server.serve(instance_router(shared.clone()).into_make_service()));
        if handle.listening().await.is_none() {
            tracing::warn!(%addr, "extra remote listener failed to bind");
            continue;
        }
        handles.push(handle);
    }

    let directory_leader = Arc::new(AtomicBool::new(false));
    let (dir_shutdown_tx, dir_shutdown_rx) = watch::channel(false);
    let directory_shutdown = if config.directory_bind_addrs.is_empty() {
        None
    } else {
        tokio::spawn(directory_leader_loop(
            rustls_config,
            config.directory_bind_addrs.clone(),
            shared.clone(),
            config.directory_retry_interval,
            directory_leader.clone(),
            dir_shutdown_rx,
        ));
        Some(dir_shutdown_tx)
    };

    let hub = hub::RemoteHub::new(
        config,
        token,
        registration_rx,
        inbound_rx,
        input_rx,
        output_tx,
        handles,
        directory_shutdown,
    );
    tokio::spawn(hub.run());

    Ok(SpawnedServer {
        handle: RemoteHandle { input_tx },
        outputs: output_rx,
        local_addr,
        directory_leader,
    })
}

async fn directory_leader_loop(
    rustls_config: axum_server::tls_rustls::RustlsConfig,
    addrs: Vec<SocketAddr>,
    shared: SharedState,
    retry: Duration,
    leader: Arc<AtomicBool>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            return;
        }
        let mut handles = Vec::new();
        for addr in &addrs {
            let handle = axum_server::Handle::new();
            let server = axum_server::bind_rustls(*addr, rustls_config.clone())
                .handle(handle.clone());
            tokio::spawn(server.serve(directory_router(shared.clone()).into_make_service()));
            if handle.listening().await.is_some() {
                handles.push(handle);
            }
        }
        if handles.is_empty() {
            tracing::debug!("directory port busy — another instance is the leader");
            tokio::select! {
                _ = tokio::time::sleep(retry) => {}
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
            }
            continue;
        }
        leader.store(true, Ordering::Relaxed);
        tracing::info!(?addrs, "directory leader bound");
        let _ = shutdown.changed().await;
        leader.store(false, Ordering::Relaxed);
        for handle in handles {
            handle.graceful_shutdown(Some(Duration::from_millis(250)));
        }
        return;
    }
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
            let current = state.shared.token.lock().expect("token lock");
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

fn bearer_ok(state: &AppState, headers: &HeaderMap) -> bool {
    let presented = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(p) => {
            let current = state.shared.token.lock().expect("token lock");
            p.as_bytes().ct_eq(current.as_bytes()).into()
        }
        None => false,
    }
}

async fn instances_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !bearer_ok(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let allowed = state.http_rate.lock().expect("http rate").take();
    if !allowed {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let host = state.shared.host.clone();
    let body = match state.shared.registry_dir.clone() {
        Some(dir) => {
            let stale_after = state.shared.stale_after;
            let host_for_read = host.clone();
            tokio::task::spawn_blocking(move || {
                registry::read_directory(&dir, &host_for_read, stale_after)
            })
            .await
            .unwrap_or_else(|_| crate::dto::InstanceDirectory {
                protocol_version: crate::PROTOCOL_VERSION,
                host,
                generated_at: registry::utc_now_rfc3339(),
                instances: Vec::new(),
            })
        }
        None => crate::dto::InstanceDirectory {
            protocol_version: crate::PROTOCOL_VERSION,
            host,
            generated_at: registry::utc_now_rfc3339(),
            instances: Vec::new(),
        },
    };
    let mut response = Json(body).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}
