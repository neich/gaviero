//! The CONNECT / plain-HTTP relay behind [`super::EgressService`].

use std::sync::Arc;
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{EgressEvent, EgressService};

/// Largest request head the proxy reads before giving up.
const MAX_HEAD: usize = 16 * 1024;

const FORBIDDEN: &[u8] = b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const BAD_GATEWAY: &[u8] =
    b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
const BAD_REQUEST: &[u8] =
    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

pub(super) async fn accept_loop(svc: Arc<EgressService>, provider: String, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((socket, _)) => {
                let svc = svc.clone();
                let provider = provider.clone();
                tokio::spawn(async move { handle(svc, provider, socket).await });
            }
            Err(e) => {
                tracing::debug!(error = %e, provider, "egress proxy: accept failed");
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

/// Split `host:port` (IPv6 in brackets) with `default_port`.
fn split_host_port(authority: &str, default_port: u16) -> Option<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = tail
            .strip_prefix(':')
            .map(|p| p.parse().ok())
            .unwrap_or(Some(default_port))?;
        return Some((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None => Some((authority.to_string(), default_port)),
    }
}

enum Request {
    /// `CONNECT host:port` — tunnel bytes after the head.
    Connect { host: String, port: u16 },
    /// Absolute-form plain HTTP — forward the rewritten head.
    Http {
        host: String,
        port: u16,
        head: Vec<u8>,
    },
}

/// Parse the request head (`head` ends at `\r\n\r\n`).
fn parse_head(head: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(head).ok()?;
    let (first, rest) = text.split_once("\r\n")?;
    let mut parts = first.split_whitespace();
    let method = parts.next()?;
    let target = parts.next()?;
    let version = parts.next().unwrap_or("HTTP/1.1");
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_host_port(target, 443)?;
        return Some(Request::Connect { host, port });
    }
    let after_scheme = target.strip_prefix("http://")?;
    let (authority, path) = match after_scheme.find('/') {
        Some(i) => (&after_scheme[..i], &after_scheme[i..]),
        None => (after_scheme, "/"),
    };
    let (host, port) = split_host_port(authority, 80)?;
    let head = format!("{method} {path} {version}\r\n{rest}").into_bytes();
    Some(Request::Http { host, port, head })
}

async fn handle(svc: Arc<EgressService>, provider: String, mut client: TcpStream) {
    let started = Instant::now();
    let mut buf: Vec<u8> = Vec::with_capacity(2048);
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        if buf.len() > MAX_HEAD {
            let _ = client.write_all(BAD_REQUEST).await;
            return;
        }
        let mut chunk = [0u8; 2048];
        match client.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    };
    let Some(request) = parse_head(&buf[..head_end]) else {
        let _ = client.write_all(BAD_REQUEST).await;
        return;
    };
    let leftover = buf[head_end..].to_vec();
    let (host, port) = match &request {
        Request::Connect { host, port } | Request::Http { host, port, .. } => (host.clone(), *port),
    };

    let (decision, first_seen) = svc.decide(&provider, &host).await;
    let mut event = EgressEvent {
        ts: chrono::Utc::now().to_rfc3339(),
        provider,
        host: host.clone(),
        port,
        decision,
        first_seen,
        bytes_up: 0,
        bytes_down: 0,
        duration_ms: 0,
    };
    svc.observer.on_egress(&event);

    if matches!(decision, "denied" | "asked-deny") {
        let _ = client.write_all(FORBIDDEN).await;
        event.duration_ms = started.elapsed().as_millis() as u64;
        svc.record(event);
        return;
    }

    let mut upstream = match TcpStream::connect((host.as_str(), port)).await {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(error = %e, host, port, "egress proxy: upstream connect failed");
            let _ = client.write_all(BAD_GATEWAY).await;
            event.duration_ms = started.elapsed().as_millis() as u64;
            svc.record(event);
            return;
        }
    };

    let prelude = match request {
        Request::Connect { .. } => {
            if client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .is_err()
            {
                return;
            }
            leftover
        }
        Request::Http { mut head, .. } => {
            head.extend_from_slice(&leftover);
            head
        }
    };
    if !prelude.is_empty() && upstream.write_all(&prelude).await.is_err() {
        return;
    }
    let (up, down) = tokio::io::copy_bidirectional(&mut client, &mut upstream)
        .await
        .unwrap_or((0, 0));
    event.bytes_up = up + prelude.len() as u64;
    event.bytes_down = down;
    event.duration_ms = started.elapsed().as_millis() as u64;
    svc.record(event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_connect_and_absolute_form() {
        match parse_head(b"CONNECT api.anthropic.com:443 HTTP/1.1\r\nHost: x\r\n\r\n") {
            Some(Request::Connect { host, port }) => {
                assert_eq!(host, "api.anthropic.com");
                assert_eq!(port, 443);
            }
            _ => panic!("expected CONNECT"),
        }
        match parse_head(b"GET http://example.com/a?b HTTP/1.1\r\nHost: example.com\r\n\r\n") {
            Some(Request::Http { host, port, head }) => {
                assert_eq!(host, "example.com");
                assert_eq!(port, 80);
                assert!(String::from_utf8(head).unwrap().starts_with("GET /a?b HTTP/1.1\r\n"));
            }
            _ => panic!("expected HTTP"),
        }
        assert!(parse_head(b"GET /relative HTTP/1.1\r\n\r\n").is_none());
    }

    #[test]
    fn split_host_port_handles_ipv6() {
        assert_eq!(split_host_port("[::1]:8080", 443), Some(("::1".into(), 8080)));
        assert_eq!(split_host_port("h", 443), Some(("h".into(), 443)));
    }
}
