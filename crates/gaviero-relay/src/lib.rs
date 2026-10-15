//! Gaviero Remote relay.
//!
//! gaviero hosts (desktops, servers) dial **out** to the relay and register
//! a route; the phone connects **in**. Live control is a TLS byte splice
//! routed by SNI — TLS stays end to end between the phone and the host's
//! sidecar (pinned per-machine CA), so the relay sees only the server name
//! and ciphertext. Notifications are small `{machine, workspace, conv,
//! kind}` events streamed to phones (replacing ntfy).
//!
//! Wire protocol: `PROTOCOL.md`; the types are
//! [`gaviero_remote::relay_proto`], shared with the host-side client.
//! Deployment (nginx in front): `README.md`.

pub mod config;
pub use gaviero_remote::relay_proto as proto;

mod http;
mod splice;
mod state;
mod util;

use std::net::SocketAddr;
use std::sync::Arc;

pub use config::Config;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

/// A running relay. Dropping it stops both listeners.
pub struct RunningRelay {
    pub http_addr: SocketAddr,
    pub splice_addr: SocketAddr,
    tasks: Vec<JoinHandle<()>>,
}

impl RunningRelay {
    /// Wait until a listener task exits (it only does on a fatal error).
    pub async fn wait(mut self) {
        let tasks = std::mem::take(&mut self.tasks);
        let _ = futures::future::select_all(tasks).await;
    }
}

impl Drop for RunningRelay {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Bind both listeners and serve. Ports may be 0 (tests).
pub async fn start(cfg: Config) -> std::io::Result<RunningRelay> {
    if cfg.secret.len() < 16 {
        return Err(std::io::Error::other(
            "relay secret must be at least 16 characters",
        ));
    }
    let http = TcpListener::bind(cfg.http_addr).await?;
    let splice = TcpListener::bind(cfg.splice_addr).await?;
    let http_addr = http.local_addr()?;
    let splice_addr = splice.local_addr()?;
    let relay = Arc::new(state::Relay::new(cfg));

    let app = http::router(relay.clone());
    let http_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(http, app).await {
            tracing::error!(error = %e, "http listener stopped");
        }
    });
    let splice_task = tokio::spawn(splice::serve(splice, relay));
    tracing::info!(%http_addr, %splice_addr, "gaviero-relay listening");
    Ok(RunningRelay {
        http_addr,
        splice_addr,
        tasks: vec![http_task, splice_task],
    })
}

/// A fresh 256-bit secret, hex-encoded.
pub fn generate_secret() -> String {
    util::random_hex(32)
}
