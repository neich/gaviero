//! PROTOTYPE for research/bidirectional-relay-replacing-ntfy-2026-10-15.md
//! (§5.1 / §7.1): a minimal SNI-routing splice relay in front of the REAL
//! sidecar. The relay peeks the ClientHello with `rustls::server::Acceptor`,
//! routes on SNI, writes the buffered bytes to the host, then copies both
//! ways — TLS (pinned CA) stays end to end between client and sidecar.
//!
//! `serve_for_dart_probe` is `#[ignore]`d: it keeps relay + sidecar up so a
//! Dart client can be pointed at it (env `GAVIERO_RELAY_PROBE_OUT`).

#![cfg(feature = "server")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use gaviero_remote::dto::{Limits, WorkspaceInfo};
use gaviero_remote::server::{RemoteServerConfig, SpawnedServer, spawn};
use gaviero_remote::{PROTOCOL_VERSION, pairing};
use rustls::pki_types::pem::PemObject;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const ROUTE: &str = "abc123-desk.gaviero.example.com";
const MAX_HELLO: usize = 16 * 1024;

/// What the relay observed, for assertions.
#[derive(Default)]
struct Seen {
    sni: Vec<Option<String>>,
    client_bytes: Vec<u8>,
    /// Source address from a PROXY v1 header (nginx `proxy_protocol on`).
    proxy_src: Vec<String>,
}

/// Read one PROXY v1 line (`PROXY TCP4 src dst sport dport\r\n`, ≤ 107
/// bytes per the spec) byte by byte, so nothing after it is consumed.
async fn read_proxy_v1(inbound: &mut TcpStream) -> Option<String> {
    let mut line = Vec::new();
    while line.len() < 107 {
        let mut b = [0u8; 1];
        let n = tokio::time::timeout(Duration::from_secs(5), inbound.read(&mut b))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        line.push(b[0]);
        if line.ends_with(b"\r\n") {
            let text = std::str::from_utf8(&line).ok()?;
            let parts: Vec<&str> = text.trim_end().split(' ').collect();
            return (parts.first() == Some(&"PROXY") && parts.len() >= 3)
                .then(|| parts[2].to_string());
        }
    }
    None
}

/// Peek SNI with rustls' Acceptor, forward the buffered bytes untouched.
/// With `expect_proxy`, a PROXY v1 line must precede the ClientHello.
async fn run_relay(
    listener: TcpListener,
    routes: HashMap<String, SocketAddr>,
    seen: Arc<Mutex<Seen>>,
    expect_proxy: bool,
) {
    loop {
        let Ok((mut inbound, _)) = listener.accept().await else {
            return;
        };
        let routes = routes.clone();
        let seen = seen.clone();
        tokio::spawn(async move {
            if expect_proxy {
                let Some(src) = read_proxy_v1(&mut inbound).await else {
                    return;
                };
                seen.lock().unwrap().proxy_src.push(src);
            }
            let mut buffered = Vec::new();
            let mut acceptor = rustls::server::Acceptor::default();
            let sni = loop {
                let mut chunk = [0u8; 4096];
                let n = match tokio::time::timeout(Duration::from_secs(5), inbound.read(&mut chunk))
                    .await
                {
                    Ok(Ok(n)) if n > 0 => n,
                    _ => return, // pending-open timeout / EOF
                };
                buffered.extend_from_slice(&chunk[..n]);
                if buffered.len() > MAX_HELLO {
                    return;
                }
                let mut fresh: &[u8] = &chunk[..n];
                if acceptor.read_tls(&mut fresh).is_err() {
                    return;
                }
                match acceptor.accept() {
                    Ok(Some(accepted)) => {
                        break accepted.client_hello().server_name().map(str::to_string);
                    }
                    Ok(None) => continue,
                    Err(_) => return,
                }
            };
            seen.lock().unwrap().sni.push(sni.clone());
            let Some(target) = sni.and_then(|name| routes.get(&name).copied()) else {
                return; // unknown route: close; the client's handshake fails
            };
            let Ok(mut outbound) = TcpStream::connect(target).await else {
                return;
            };
            if outbound.write_all(&buffered).await.is_err() {
                return;
            }
            seen.lock().unwrap().client_bytes.extend_from_slice(&buffered);
            // Splice. Record client→host bytes so the test can prove the
            // relay never sees the bearer token in clear.
            let (mut ir, mut iw) = inbound.into_split();
            let (mut or, mut ow) = outbound.into_split();
            let seen_up = seen.clone();
            let up = async move {
                let mut buf = [0u8; 8192];
                loop {
                    match ir.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            seen_up.lock().unwrap().client_bytes.extend_from_slice(&buf[..n]);
                            if ow.write_all(&buf[..n]).await.is_err() {
                                break;
                            }
                        }
                    }
                }
                let _ = ow.shutdown().await;
            };
            let down = async move {
                let _ = tokio::io::copy(&mut or, &mut iw).await;
                let _ = iw.shutdown().await;
            };
            // Full teardown: when either direction ends, drop both.
            tokio::select! { _ = up => {}, _ = down => {} }
        });
    }
}

async fn sidecar(bundle: Vec<u8>, token: String) -> SpawnedServer {
    let (ping, idle, hello_timeout) = RemoteServerConfig::timing_defaults();
    spawn(RemoteServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        extra_bind_addrs: Vec::new(),
        tls_cert_pem: bundle.clone(),
        tls_key_pem: bundle,
        token,
        instance_id: "inst-relay-proto".to_string(),
        tui_version: "0.1.0".to_string(),
        workspace: WorkspaceInfo {
            id: "abc123".to_string(),
            display_name: "gaviero".to_string(),
        },
        capabilities: Vec::new(),
        machine: None,
        token_path: None,
        token_poll_interval: RemoteServerConfig::TOKEN_POLL_INTERVAL,
        registry: None,
        directory_bind_addrs: Vec::new(),
        heartbeat_interval: gaviero_remote::server::registry::HEARTBEAT_INTERVAL,
        directory_retry_interval: gaviero_remote::server::registry::DIRECTORY_RETRY_INTERVAL,
        stale_after: gaviero_remote::server::registry::STALE_AFTER,
        confirm_required: Vec::new(),
        allowed_slash_commands: Vec::new(),
        limits: Limits {
            max_frame_bytes: 262_144,
            max_prompt_bytes: 131_072,
            command_rate_per_second: 10,
        },
        ping_interval: ping,
        idle_timeout: idle,
        hello_timeout,
    })
    .await
    .expect("sidecar starts")
}

struct Rig {
    _dir: tempfile::TempDir,
    _server: SpawnedServer,
    relay_port: u16,
    ca_b64: String,
    token: String,
    seen: Arc<Mutex<Seen>>,
}

async fn rig() -> Rig {
    rig_with(false).await
}

async fn rig_with(expect_proxy: bool) -> Rig {
    rig_on(expect_proxy, "127.0.0.1").await
}

async fn rig_on(expect_proxy: bool, relay_bind: &str) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let pinned = pairing::ensure_pinned_ca(dir.path(), ROUTE).unwrap();
    let bundle = std::fs::read(&pinned.leaf_path).unwrap();
    let token = pairing::generate_token();
    let server = sidecar(bundle, token.clone()).await;
    let mut routes = HashMap::new();
    routes.insert(ROUTE.to_string(), server.local_addr);
    let listener = TcpListener::bind((relay_bind, 0)).await.unwrap();
    let relay_port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    tokio::spawn(run_relay(listener, routes, seen.clone(), expect_proxy));
    Rig {
        _dir: dir,
        _server: server,
        relay_port,
        ca_b64: pinned.ca_der_b64,
        token,
        seen,
    }
}

fn pinned_client(ca_b64: &str) -> Arc<rustls::ClientConfig> {
    let pem = format!("-----BEGIN CERTIFICATE-----\n{ca_b64}\n-----END CERTIFICATE-----\n");
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from_pem_slice(pem.as_bytes()).unwrap())
        .unwrap();
    Arc::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

#[tokio::test]
async fn sni_relay_splices_end_to_end_tls_to_the_real_sidecar() {
    let rig = rig().await;

    // Phone: dial the relay, TLS with SNI = route, pinned CA, WSS upgrade.
    let mut request = format!("wss://{ROUTE}/v1/ws").into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", rig.token).parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "gaviero.v1".parse().unwrap());
    let stream = TcpStream::connect(("127.0.0.1", rig.relay_port)).await.unwrap();
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::client_async_tls_with_config(
            request,
            stream,
            None,
            Some(tokio_tungstenite::Connector::Rustls(pinned_client(&rig.ca_b64))),
        ),
    )
    .await
    .expect("no hang")
    .expect("TLS + WS upgrade through the relay");

    let hello = serde_json::json!({
        "version": { "major": PROTOCOL_VERSION.major, "minor": PROTOCOL_VERSION.minor },
        "instance_id": null, "command_id": "cmd-1", "type": "client_hello",
        "payload": { "protocol_version": { "major": PROTOCOL_VERSION.major, "minor": PROTOCOL_VERSION.minor },
                     "client_name": "relay-proto", "client_version": "0.1.0" }
    });
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(hello.to_string().into()))
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("hello arrives")
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert!(reply.contains("\"type\":\"hello\""), "{reply}");
    let _ = socket.close(None).await;

    let seen = rig.seen.lock().unwrap();
    assert_eq!(seen.sni, vec![Some(ROUTE.to_string())], "relay routed on SNI");
    assert!(!seen.client_bytes.is_empty());
    let token = rig.token.as_bytes();
    assert!(
        !seen.client_bytes.windows(token.len()).any(|w| w == token),
        "the relay must never see the bearer token in clear"
    );
    assert!(
        !seen.client_bytes.windows(9).any(|w| w == b"/v1/ws HT"),
        "the HTTP upgrade request is encrypted too"
    );
}

#[tokio::test]
async fn unknown_route_and_wrong_ca_fail_without_hanging() {
    let rig = rig().await;
    let other = tempfile::tempdir().unwrap();
    let foreign = pairing::ensure_pinned_ca(other.path(), ROUTE).unwrap();

    for (ca, name, why) in [
        (rig.ca_b64.clone(), "nobody-desk.gaviero.example.com", "unknown route"),
        (foreign.ca_der_b64.clone(), ROUTE, "another machine's CA"),
    ] {
        let stream = TcpStream::connect(("127.0.0.1", rig.relay_port)).await.unwrap();
        let connector = tokio_rustls::TlsConnector::from(pinned_client(&ca));
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            connector.connect(
                rustls::pki_types::ServerName::try_from(name.to_string()).unwrap(),
                stream,
            ),
        )
        .await
        .unwrap_or_else(|_| panic!("{why}: handshake hung"));
        assert!(result.is_err(), "{why}: handshake must fail");
    }
}

/// nginx `stream { proxy_protocol on; }` prepends a PROXY v1 line to every
/// upstream connection. The relay strips it, keeps the real client address,
/// and the end-to-end TLS handshake behind it is unaffected.
#[tokio::test]
async fn proxy_v1_header_before_the_client_hello_is_handled() {
    let rig = rig_with(true).await;
    let mut stream = TcpStream::connect(("127.0.0.1", rig.relay_port)).await.unwrap();
    stream
        .write_all(b"PROXY TCP4 203.0.113.7 127.0.0.1 51234 443\r\n")
        .await
        .unwrap();
    let connector = tokio_rustls::TlsConnector::from(pinned_client(&rig.ca_b64));
    let tls = tokio::time::timeout(
        Duration::from_secs(10),
        connector.connect(
            rustls::pki_types::ServerName::try_from(ROUTE.to_string()).unwrap(),
            stream,
        ),
    )
    .await
    .expect("no hang")
    .expect("TLS through relay after a PROXY header");
    drop(tls);
    let seen = rig.seen.lock().unwrap();
    assert_eq!(seen.proxy_src, vec!["203.0.113.7".to_string()]);
    assert_eq!(seen.sni, vec![Some(ROUTE.to_string())]);
}

/// Keep relay + sidecar up for a Dart client. Writes `relay_port`, `route`,
/// `ca_b64`, `token` (one per line) to `$GAVIERO_RELAY_PROBE_OUT`.
/// `GAVIERO_RELAY_PROBE_PROXY=1` expects nginx's PROXY v1 line;
/// `GAVIERO_RELAY_PROBE_BIND` (default 127.0.0.1) sets the relay address.
#[tokio::test]
#[ignore = "manual: serves a relay + sidecar for the Dart connectionFactory probe"]
async fn serve_for_dart_probe() {
    let proxy = std::env::var("GAVIERO_RELAY_PROBE_PROXY").is_ok_and(|v| v == "1");
    let bind = std::env::var("GAVIERO_RELAY_PROBE_BIND").unwrap_or_else(|_| "127.0.0.1".into());
    let rig = rig_on(proxy, &bind).await;
    let out = std::env::var("GAVIERO_RELAY_PROBE_OUT").expect("GAVIERO_RELAY_PROBE_OUT");
    std::fs::write(
        &out,
        format!("{}\n{ROUTE}\n{}\n{}\n", rig.relay_port, rig.ca_b64, rig.token),
    )
    .unwrap();
    let secs: u64 = std::env::var("GAVIERO_RELAY_PROBE_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    tokio::time::sleep(Duration::from_secs(secs)).await;
    let seen = rig.seen.lock().unwrap();
    let token = rig.token.as_bytes();
    println!(
        "relay saw SNI {:?}; PROXY sources {:?}; token in clear: {}",
        seen.sni,
        seen.proxy_src,
        seen.client_bytes.windows(token.len()).any(|w| w == token)
    );
}
