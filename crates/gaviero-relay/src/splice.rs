//! The splice port: raw TLS from phones (via nginx `stream`), routed by SNI.
//! The relay reads the ClientHello only far enough to learn the server
//! name, asks the owning host to dial in, then copies bytes both ways. It
//! never terminates TLS, so it never sees the bearer token or any content.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::proto::RelayMsg;
use crate::state::Relay;

pub(crate) async fn serve(listener: TcpListener, relay: Arc<Relay>) {
    loop {
        match listener.accept().await {
            Ok((tcp, peer)) => {
                tokio::spawn(handle(relay.clone(), tcp, peer));
            }
            Err(e) => {
                tracing::warn!(error = %e, "splice accept failed");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn handle(relay: Arc<Relay>, mut tcp: TcpStream, peer: SocketAddr) {
    let cfg = &relay.cfg;
    let client_ip = if cfg.proxy_protocol {
        match tokio::time::timeout(cfg.hello_timeout, read_proxy_v1(&mut tcp)).await {
            Ok(Some(ip)) => ip,
            _ => {
                tracing::debug!(%peer, "missing or bad PROXY header");
                return;
            }
        }
    } else {
        peer.ip()
    };
    let Some(_slot) = relay.session_guard(client_ip) else {
        tracing::warn!(%client_ip, "too many sessions from one address");
        return;
    };
    let (sni, hello) = match tokio::time::timeout(
        cfg.hello_timeout,
        peek_sni(&mut tcp, cfg.max_hello_bytes),
    )
    .await
    {
        Ok(Ok(found)) => found,
        _ => {
            tracing::debug!(%client_ip, "no ClientHello");
            return;
        }
    };
    let Some(route) = sni.map(|s| s.to_ascii_lowercase()) else {
        tracing::debug!(%client_ip, "ClientHello without SNI");
        return;
    };
    let Some(host) = relay.lookup(&route) else {
        tracing::info!(%client_ip, %route, "no host registered for route");
        return;
    };

    let nonce = crate::util::random_hex(16);
    let (ws_tx, ws_rx) = oneshot::channel();
    relay.add_pending(nonce.clone(), ws_tx);
    if host
        .try_send(RelayMsg::Dial {
            route: route.clone(),
            nonce: nonce.clone(),
        })
        .is_err()
    {
        relay.take_pending(&nonce);
        return;
    }
    let ws = match tokio::time::timeout(cfg.pending_timeout, ws_rx).await {
        Ok(Ok(ws)) => ws,
        _ => {
            relay.take_pending(&nonce);
            tracing::info!(%client_ip, %route, "host did not dial back in time");
            return;
        }
    };
    tracing::info!(%client_ip, %route, "session spliced");
    let (up, down) = splice(tcp, ws, hello, cfg.idle_timeout).await;
    tracing::info!(%client_ip, %route, bytes_up = up, bytes_down = down, "session ended");
}

/// Read one PROXY v1 line (`PROXY TCP4 src dst sport dport\r\n`, ≤ 107
/// bytes) byte by byte so nothing after it is consumed.
async fn read_proxy_v1(tcp: &mut TcpStream) -> Option<IpAddr> {
    let mut line = Vec::with_capacity(64);
    while line.len() < 107 {
        let mut b = [0u8; 1];
        if tcp.read(&mut b).await.ok()? == 0 {
            return None;
        }
        line.push(b[0]);
        if line.ends_with(b"\r\n") {
            let text = std::str::from_utf8(&line).ok()?;
            let mut parts = text.trim_end().split(' ');
            if parts.next()? != "PROXY" {
                return None;
            }
            return match parts.next()? {
                "TCP4" | "TCP6" => parts.next()?.parse().ok(),
                _ => None, // "UNKNOWN": no usable source
            };
        }
    }
    None
}

/// Buffer the ClientHello and return its SNI plus every byte read, so the
/// host receives the handshake untouched.
async fn peek_sni(tcp: &mut TcpStream, max: usize) -> std::io::Result<(Option<String>, Vec<u8>)> {
    let mut buffered = Vec::new();
    let mut acceptor = rustls::server::Acceptor::default();
    loop {
        let mut chunk = [0u8; 4096];
        let n = tcp.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buffered.extend_from_slice(&chunk[..n]);
        if buffered.len() > max {
            return Err(std::io::Error::other("ClientHello too large"));
        }
        let mut fresh: &[u8] = &chunk[..n];
        acceptor.read_tls(&mut fresh)?;
        match acceptor.accept() {
            Ok(Some(accepted)) => {
                let sni = accepted.client_hello().server_name().map(str::to_string);
                return Ok((sni, buffered));
            }
            Ok(None) => continue,
            Err((e, _alert)) => return Err(std::io::Error::other(e.to_string())),
        }
    }
}

/// Copy phone TCP ⇄ host data WebSocket. When either side ends, or nothing
/// moves for `idle`, BOTH directions are torn down (a half-closed pair can
/// otherwise keep a peer alive forever). Returns bytes (up, down).
async fn splice(tcp: TcpStream, ws: WebSocket, hello: Vec<u8>, idle: Duration) -> (u64, u64) {
    let start = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let touch = |last: &AtomicU64| last.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
    let (mut tcp_rx, mut tcp_tx) = tcp.into_split();
    let (mut ws_tx, mut ws_rx) = ws.split();
    let up_bytes = Arc::new(AtomicU64::new(hello.len() as u64));
    let down_bytes = Arc::new(AtomicU64::new(0));

    if ws_tx.send(Message::Binary(Bytes::from(hello))).await.is_err() {
        return (0, 0);
    }

    let up = {
        let last = last.clone();
        let up_bytes = up_bytes.clone();
        async move {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match tcp_rx.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if ws_tx
                            .send(Message::Binary(Bytes::copy_from_slice(&buf[..n])))
                            .await
                            .is_err()
                        {
                            break;
                        }
                        up_bytes.fetch_add(n as u64, Ordering::Relaxed);
                        touch(&last);
                    }
                }
            }
            let _ = ws_tx.close().await;
        }
    };
    let down = {
        let last = last.clone();
        let down_bytes = down_bytes.clone();
        async move {
            while let Some(Ok(msg)) = ws_rx.next().await {
                match msg {
                    Message::Binary(b) => {
                        if tcp_tx.write_all(&b).await.is_err() {
                            break;
                        }
                        down_bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
                        touch(&last);
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            let _ = tcp_tx.shutdown().await;
        }
    };
    let watchdog = {
        let last = last.clone();
        async move {
            loop {
                tokio::time::sleep(idle / 4).await;
                let quiet = start.elapsed().as_millis() as u64 - last.load(Ordering::Relaxed);
                if quiet > idle.as_millis() as u64 {
                    break;
                }
            }
        }
    };
    tokio::select! {
        _ = up => {}
        _ = down => {}
        _ = watchdog => tracing::debug!("spliced session idle — closing"),
    }
    (
        up_bytes.load(Ordering::Relaxed),
        down_bytes.load(Ordering::Relaxed),
    )
}
