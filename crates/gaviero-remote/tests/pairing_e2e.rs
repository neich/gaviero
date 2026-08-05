//! End-to-end pairing check (Plan A A6): take the QR payload the TUI's
//! `/remote` renders, and connect to a live sidecar exactly the way the
//! mobile app does — bearer header only, subprotocol `gaviero.v1`,
//! `client_hello` first — asserting the `hello` that comes back carries
//! what the app needs to build its UI.
//!
//! This is the closest a test can get to "the QR works": it does not scan
//! the image, but it drives the full payload → URL → TLS → auth →
//! handshake path with the real server.

#![cfg(feature = "server")]

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use gaviero_remote::dto::{Limits, WorkspaceInfo};
use gaviero_remote::envelope::{ClientEnvelope, ClientFrame, ServerFrame};
use gaviero_remote::server::{RemoteServerConfig, spawn};
use gaviero_remote::{PROTOCOL_VERSION, pairing};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// The certificate is issued to the MagicDNS hostname, so the client must
/// present that name in the TLS handshake while dialing loopback — the
/// same split the plan describes for tests (§3.1).
const MAGIC_DNS_HOST: &str = "testhost.tailnet.ts.net";

struct Tls {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    client_config: Arc<rustls::ClientConfig>,
}

fn issue_cert() -> Tls {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec![MAGIC_DNS_HOST.to_string()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let cert_pem = cert.pem().into_bytes();
    let key_pem = key.serialize_pem().into_bytes();

    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(cert.der().to_vec()))
        .unwrap();
    let client_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Tls {
        cert_pem,
        key_pem,
        client_config: Arc::new(client_config),
    }
}

#[tokio::test]
async fn qr_payload_pairs_a_client_end_to_end() {
    let tls = issue_cert();

    // 1. The certificate the TUI would load passes the availability check.
    let info = pairing::inspect_cert(&tls.cert_pem, MAGIC_DNS_HOST)
        .expect("certificate parses");
    assert!(info.covers_host, "cert must cover the MagicDNS host");
    assert!(!info.is_expired());

    // 2. A token is generated the way `/remote` generates it.
    let token = pairing::generate_token();
    assert!(
        !pairing::token_fingerprint(&token).contains(&token[8..56]),
        "the log fingerprint must not leak the token"
    );

    // 3. Start the sidecar.
    let (ping, idle, hello_timeout) = RemoteServerConfig::timing_defaults();
    let spawned = spawn(RemoteServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        extra_bind_addrs: Vec::new(),
        tls_cert_pem: tls.cert_pem.clone(),
        tls_key_pem: tls.key_pem.clone(),
        token: token.clone(),
        instance_id: "inst-e2e".to_string(),
        tui_version: "0.1.0".to_string(),
        workspace: WorkspaceInfo {
            id: "4b156f1de41da274".to_string(),
            display_name: "gaviero".to_string(),
        },
        capabilities: Vec::new(),
        confirm_required: ["/autoapprove", "/yolo", "/reset", "/clear"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        allowed_slash_commands: ["/model", "/effort", "/lite", "/reset"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
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
    .expect("sidecar starts");
    let port = spawned.local_addr.port();

    // 4. Build the QR payload exactly as `/remote` does, then parse it back
    //    the way the app does (Plan B B9 validates kind + protocol_major).
    let url = format!("wss://{MAGIC_DNS_HOST}:{port}/v1/ws");
    let qr = pairing::qr_payload_json(&url, &token, "gaviero");
    let scanned: serde_json::Value = serde_json::from_str(&qr).unwrap();
    assert_eq!(scanned["kind"], "gaviero-remote");
    assert_eq!(scanned["protocol_major"], PROTOCOL_VERSION.major);
    let scanned_url = scanned["url"].as_str().unwrap();
    let scanned_token = scanned["token"].as_str().unwrap();

    // 5. Connect using ONLY what the QR carried: bearer header (never the
    //    URL), subprotocol gaviero.v1, TLS validated against the MagicDNS
    //    name while dialing loopback.
    let mut request = scanned_url.into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {scanned_token}").parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "gaviero.v1".parse().unwrap());
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("dial loopback");
    let (mut socket, response) = tokio_tungstenite::client_async_tls_with_config(
        request,
        stream,
        None,
        Some(tokio_tungstenite::Connector::Rustls(
            tls.client_config.clone(),
        )),
    )
    .await
    .expect("TLS + WebSocket upgrade succeeds with the QR's token");
    assert_eq!(
        response.headers().get("sec-websocket-protocol").unwrap(),
        "gaviero.v1"
    );

    // 6. `client_hello` first, with the literal-null instance_id.
    let hello_frame = serde_json::json!({
        "version": { "major": PROTOCOL_VERSION.major, "minor": PROTOCOL_VERSION.minor },
        "instance_id": null,
        "command_id": "cmd-1",
        "type": "client_hello",
        "payload": {
            "protocol_version": { "major": PROTOCOL_VERSION.major, "minor": PROTOCOL_VERSION.minor },
            "client_name": "gaviero-remote-android",
            "client_version": "0.1.0"
        }
    });
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            hello_frame.to_string().into(),
        ))
        .await
        .unwrap();

    // 7. The `hello` back carries everything the app builds its UI from.
    let reply = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("hello arrives")
        .expect("stream open")
        .expect("frame decodes");
    let text = reply.into_text().unwrap();
    let envelope: gaviero_remote::ServerEnvelope = serde_json::from_str(&text).unwrap();
    let ServerFrame::Hello(hello) = envelope.frame else {
        panic!("expected hello, got {:?}", envelope.frame);
    };
    assert_eq!(hello.protocol_version.major, PROTOCOL_VERSION.major);
    assert_eq!(hello.workspace.display_name, "gaviero");
    assert!(
        !hello.workspace.id.contains(std::path::MAIN_SEPARATOR),
        "workspace.id must be an opaque hash, never a path"
    );
    assert!(hello.allowed_slash_commands.contains(&"/model".to_string()));
    assert!(hello.confirm_required.contains(&"/reset".to_string()));
    assert_eq!(hello.limits.max_prompt_bytes, 131_072);
    assert_eq!(envelope.instance_id, "inst-e2e");
}

#[tokio::test]
async fn a_stale_qr_token_cannot_pair() {
    let tls = issue_cert();
    let (ping, idle, hello_timeout) = RemoteServerConfig::timing_defaults();
    let spawned = spawn(RemoteServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        extra_bind_addrs: Vec::new(),
        tls_cert_pem: tls.cert_pem.clone(),
        tls_key_pem: tls.key_pem.clone(),
        token: pairing::generate_token(),
        instance_id: "inst-stale".to_string(),
        tui_version: "0.1.0".to_string(),
        workspace: WorkspaceInfo {
            id: "abc".to_string(),
            display_name: "gaviero".to_string(),
        },
        capabilities: Vec::new(),
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
    .unwrap();
    let port = spawned.local_addr.port();

    // A QR printed before a `/remote rotate` carries the old token.
    let stale = pairing::generate_token();
    let url = format!("wss://{MAGIC_DNS_HOST}:{port}/v1/ws");
    let mut request = url.as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {stale}").parse().unwrap());
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "gaviero.v1".parse().unwrap());
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let result = tokio_tungstenite::client_async_tls_with_config(
        request,
        stream,
        None,
        Some(tokio_tungstenite::Connector::Rustls(tls.client_config.clone())),
    )
    .await;
    assert!(
        result.is_err(),
        "a stale token must be rejected before the upgrade"
    );
}

/// The token must never be accepted from the URL — a client that tries it
/// gets rejected exactly like one presenting nothing (§3.4).
#[tokio::test]
async fn a_token_in_the_query_string_is_not_accepted() {
    let tls = issue_cert();
    let token = pairing::generate_token();
    let (ping, idle, hello_timeout) = RemoteServerConfig::timing_defaults();
    let spawned = spawn(RemoteServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        extra_bind_addrs: Vec::new(),
        tls_cert_pem: tls.cert_pem.clone(),
        tls_key_pem: tls.key_pem.clone(),
        token: token.clone(),
        instance_id: "inst-url".to_string(),
        tui_version: "0.1.0".to_string(),
        workspace: WorkspaceInfo {
            id: "abc".to_string(),
            display_name: "gaviero".to_string(),
        },
        capabilities: Vec::new(),
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
    .unwrap();
    let port = spawned.local_addr.port();

    let url = format!("wss://{MAGIC_DNS_HOST}:{port}/v1/ws?token={token}");
    let mut request = url.as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "gaviero.v1".parse().unwrap());
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let result = tokio_tungstenite::client_async_tls_with_config(
        request,
        stream,
        None,
        Some(tokio_tungstenite::Connector::Rustls(tls.client_config.clone())),
    )
    .await;
    assert!(
        result.is_err(),
        "the token must be accepted only in the Authorization header"
    );
}

/// Unused import guard: keeps the client-frame types referenced so the
/// test file documents the shapes the app sends after `hello`.
#[allow(dead_code)]
fn _client_frame_shapes() -> ClientEnvelope {
    ClientEnvelope {
        version: PROTOCOL_VERSION,
        instance_id: Some("inst".to_string()),
        command_id: "cmd-2".to_string(),
        frame: ClientFrame::RequestSnapshot {},
    }
}
