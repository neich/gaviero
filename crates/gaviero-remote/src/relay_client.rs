//! Outbound client for `gaviero-relay` (`crates/gaviero-relay`).
//!
//! The sidecar keeps listening on loopback only; this task dials OUT to the
//! relay, registers the instance's route, and for every `dial` opens a data
//! WebSocket that it pipes byte-for-byte to the sidecar's own TLS listener.
//! TLS therefore stays end to end (phone ⇄ sidecar, pinned CA): the relay
//! carries ciphertext only. Notifications go up the same control socket.
//!
//! Wire types come from [`crate::relay_proto`], the same module the relay
//! itself uses; `crates/gaviero-relay/tests/e2e.rs` drives this client
//! against the real relay.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_util::sync::CancellationToken;

use crate::relay_proto::{HostMsg, NotifyKind, RelayMsg, RouteInfo};

/// Control-channel ping period; the relay pings at the same rate.
const KEEPALIVE: Duration = Duration::from_secs(25);
/// No frame at all from the relay for this long ⇒ reconnect.
const DEAD_AFTER: Duration = Duration::from_secs(75);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Spliced sessions with no bytes either way (the protocol pings every 20 s).
const DATA_IDLE: Duration = Duration::from_secs(180);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The route this instance claims on the relay
/// (`route` = `<wsid>-<machine>.<relay host>`).
pub type RelayRoute = RouteInfo;
/// What a phone notification is about.
pub type RelayNotifyKind = NotifyKind;

#[derive(Clone)]
pub struct RelayClientConfig {
    /// `wss://gaviero.example.com` (tests: `ws://127.0.0.1:<port>`), no
    /// trailing slash.
    pub base_url: String,
    pub secret: String,
    pub route: RelayRoute,
    /// The sidecar's loopback listener.
    pub local_addr: SocketAddr,
}

impl std::fmt::Debug for RelayClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayClientConfig")
            .field("base_url", &self.base_url)
            .field("secret", &"<redacted>")
            .field("route", &self.route)
            .field("local_addr", &self.local_addr)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayStatus {
    Connecting,
    /// The relay accepted the route; phones can reach this instance.
    Registered,
    /// The relay refused the route (bad name, limit) — retried later.
    Rejected(String),
    Disconnected(String),
}

impl std::fmt::Display for RelayStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connecting => write!(f, "connecting"),
            Self::Registered => write!(f, "registered"),
            Self::Rejected(r) => write!(f, "route rejected — {r}"),
            Self::Disconnected(r) => write!(f, "disconnected — {r}"),
        }
    }
}

struct Note {
    conv: String,
    kind: RelayNotifyKind,
}

/// Handle to the background relay task. Cheap to clone.
#[derive(Clone)]
pub struct RelayClient {
    notes: mpsc::Sender<Note>,
    status: watch::Receiver<RelayStatus>,
    cancel: CancellationToken,
    route: Arc<RelayRoute>,
}

impl RelayClient {
    /// Start the reconnecting client. Must be called inside a tokio runtime.
    pub fn spawn(cfg: RelayClientConfig) -> Self {
        let (notes_tx, notes_rx) = mpsc::channel(64);
        let (status_tx, status_rx) = watch::channel(RelayStatus::Connecting);
        let cancel = CancellationToken::new();
        let route = Arc::new(cfg.route.clone());
        tokio::spawn(run(cfg, notes_rx, status_tx, cancel.clone()));
        Self {
            notes: notes_tx,
            status: status_rx,
            cancel,
            route,
        }
    }

    /// Queue a phone notification (dropped if the queue is full or the
    /// relay is unreachable — a notification is never worth blocking for).
    pub fn notify(&self, kind: RelayNotifyKind, conv: &str) {
        let _ = self.notes.try_send(Note {
            conv: conv.to_string(),
            kind,
        });
    }

    pub fn status(&self) -> RelayStatus {
        self.status.borrow().clone()
    }

    pub fn status_receiver(&self) -> watch::Receiver<RelayStatus> {
        self.status.clone()
    }

    pub fn route(&self) -> &RelayRoute {
        &self.route
    }

    pub fn shutdown(&self) {
        self.cancel.cancel();
    }
}

async fn run(
    cfg: RelayClientConfig,
    mut notes: mpsc::Receiver<Note>,
    status: watch::Sender<RelayStatus>,
    cancel: CancellationToken,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if cancel.is_cancelled() {
            return;
        }
        let _ = status.send(RelayStatus::Connecting);
        let started = Instant::now();
        let reason = control_session(&cfg, &mut notes, &status, &cancel).await;
        if cancel.is_cancelled() {
            return;
        }
        tracing::info!(route = %cfg.route.route, %reason, "relay control session ended");
        if !matches!(*status.borrow(), RelayStatus::Rejected(_)) {
            let _ = status.send(RelayStatus::Disconnected(reason));
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        // 50–100 % jitter so many hosts don't reconnect in lockstep.
        let jitter = 0.5 + rand::random::<f64>() * 0.5;
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(backoff.mul_f64(jitter)) => {}
        }
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn request(cfg: &RelayClientConfig, path: &str) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request, String> {
    let mut req = format!("{}{path}", cfg.base_url)
        .into_client_request()
        .map_err(|e| format!("bad relay URL: {e}"))?;
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {}", cfg.secret)
            .parse()
            .map_err(|_| "relay secret is not a valid header value".to_string())?,
    );
    Ok(req)
}

fn describe(e: tokio_tungstenite::tungstenite::Error) -> String {
    use tokio_tungstenite::tungstenite::Error;
    match e {
        Error::Http(resp) if resp.status().as_u16() == 401 => {
            "the relay rejected the secret (401) — check remote.relay.secret".into()
        }
        Error::Http(resp) => format!("relay answered HTTP {}", resp.status()),
        other => other.to_string(),
    }
}

/// One control connection. Returns why it ended.
async fn control_session(
    cfg: &RelayClientConfig,
    notes: &mut mpsc::Receiver<Note>,
    status: &watch::Sender<RelayStatus>,
    cancel: &CancellationToken,
) -> String {
    let req = match request(cfg, "/v1/host") {
        Ok(r) => r,
        Err(e) => return e,
    };
    let ws = match tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req)).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(e)) => return describe(e),
        Err(_) => return "timed out connecting to the relay".into(),
    };
    let (mut sink, mut stream) = ws.split();
    let register = serde_json::to_string(&HostMsg::Register {
        info: cfg.route.clone(),
    })
    .expect("register serializes");
    if let Err(e) = sink.send(Message::Text(register.into())).await {
        return e.to_string();
    }
    let mut ping = tokio::time::interval(KEEPALIVE);
    ping.tick().await;
    let mut last_rx = Instant::now();
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = sink.close().await;
                return "shutdown".into();
            }
            msg = stream.next() => {
                let msg = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => return e.to_string(),
                    None => return "closed by the relay".into(),
                };
                last_rx = Instant::now();
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => return "closed by the relay".into(),
                    _ => continue,
                };
                match serde_json::from_str::<RelayMsg>(&text) {
                    Ok(RelayMsg::Registered { .. }) => {
                        tracing::info!(route = %cfg.route.route, "relay route registered");
                        let _ = status.send(RelayStatus::Registered);
                    }
                    Ok(RelayMsg::Rejected { reason, .. }) => {
                        let _ = status.send(RelayStatus::Rejected(reason.clone()));
                        return format!("route rejected: {reason}");
                    }
                    Ok(RelayMsg::Dial { nonce, .. }) => {
                        let cfg = cfg.clone();
                        tokio::spawn(async move {
                            if let Err(e) = data_session(&cfg, &nonce).await {
                                tracing::info!(error = %e, "relay data session failed");
                            }
                        });
                    }
                    Ok(RelayMsg::Error { message }) => {
                        tracing::warn!(%message, "relay reported an error");
                    }
                    Err(_) => tracing::debug!("ignoring unknown relay message"),
                }
            }
            note = notes.recv() => {
                let Some(note) = note else { return "client dropped".into() };
                let text = serde_json::to_string(&HostMsg::Notify {
                    machine: cfg.route.machine.clone(),
                    workspace_id: cfg.route.workspace_id.clone(),
                    conv: note.conv,
                    kind: note.kind,
                })
                .expect("notify serializes");
                if let Err(e) = sink.send(Message::Text(text.into())).await {
                    return e.to_string();
                }
            }
            _ = ping.tick() => {
                if last_rx.elapsed() > DEAD_AFTER {
                    return "the relay stopped responding".into();
                }
                if let Err(e) = sink.send(Message::Ping(Vec::new().into())).await {
                    return e.to_string();
                }
            }
        }
    }
}

/// Pipe one phone session: relay data WebSocket ⇄ sidecar loopback TCP.
/// When either side ends or nothing moves for [`DATA_IDLE`], both are torn
/// down — never leave a half-closed pair behind.
async fn data_session(cfg: &RelayClientConfig, nonce: &str) -> Result<(), String> {
    let req = request(cfg, &format!("/v1/data/{nonce}"))?;
    let (ws, _) = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| "timed out opening the data channel".to_string())?
        .map_err(describe)?;
    let tcp = tokio::net::TcpStream::connect(cfg.local_addr)
        .await
        .map_err(|e| format!("sidecar {}: {e}", cfg.local_addr))?;
    let _ = tcp.set_nodelay(true);
    let (mut tcp_rx, mut tcp_tx) = tcp.into_split();
    let (mut ws_tx, mut ws_rx) = ws.split();
    let start = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let touch = |last: &AtomicU64| last.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);

    let to_phone = {
        let last = last.clone();
        async move {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match tcp_rx.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if ws_tx
                            .send(Message::Binary(buf[..n].to_vec().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                        touch(&last);
                    }
                }
            }
            let _ = ws_tx.close().await;
        }
    };
    let to_sidecar = {
        let last = last.clone();
        async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                match msg {
                    Message::Binary(b) => {
                        if tcp_tx.write_all(&b).await.is_err() {
                            break;
                        }
                        touch(&last);
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            let _ = tcp_tx.shutdown().await;
        }
    };
    let watchdog = async {
        loop {
            tokio::time::sleep(DATA_IDLE / 4).await;
            let quiet = start.elapsed().as_millis() as u64 - last.load(Ordering::Relaxed);
            if quiet > DATA_IDLE.as_millis() as u64 {
                break;
            }
        }
    };
    tokio::select! {
        _ = to_phone => {}
        _ = to_sidecar => {}
        _ = watchdog => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_secret() {
        let cfg = RelayClientConfig {
            base_url: "wss://relay.test".into(),
            secret: "super-secret-value".into(),
            route: RelayRoute {
                route: "r".into(),
                machine: "m".into(),
                workspace_id: "w".into(),
                display_name: "d".into(),
            },
            local_addr: "127.0.0.1:1".parse().unwrap(),
        };
        assert!(!format!("{cfg:?}").contains("super-secret-value"));
    }
}
