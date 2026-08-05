//! A2 verification: adversarial transport tests against the real WSS
//! server with generated TLS fixtures and a real WebSocket client.
#![cfg(feature = "server")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use gaviero_remote::dto::{ClientHello, Limits, WorkspaceInfo};
use gaviero_remote::envelope::{
    ClientEnvelope, ClientFrame, SendPrompt, ServerEnvelope, ServerFrame, Snapshot, StreamChunk,
    StreamingStatus,
};
use gaviero_remote::server::{HubInput, HubOutput, RemoteServerConfig, SpawnedServer, spawn};
use gaviero_remote::version::{PROTOCOL_VERSION, ProtocolVersion};
use gaviero_remote::{SUBPROTOCOL, WS_PATH, close_code};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{Connector, MaybeTlsStream, WebSocketStream, client_async_tls_with_config};

const TEST_HOST: &str = "remote-test.gaviero.invalid";
const TOKEN: &str = "test-token-0123456789abcdef0123456789abcdef";
const INSTANCE: &str = "a3f9c2e14b7d8650";

struct TestTls {
    cert_pem: Vec<u8>,
    key_pem: Vec<u8>,
    client_config: Arc<rustls::ClientConfig>,
}

fn make_tls() -> TestTls {
    let key = rcgen::generate_simple_self_signed(vec![TEST_HOST.to_string()]).unwrap();
    let cert_pem = key.cert.pem().into_bytes();
    let key_pem = key.key_pair.serialize_pem().into_bytes();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(key.cert.der().clone()).unwrap();
    let client_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TestTls { cert_pem, key_pem, client_config: Arc::new(client_config) }
}

fn test_config(tls: &TestTls) -> RemoteServerConfig {
    RemoteServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        extra_bind_addrs: Vec::new(),
        tls_cert_pem: tls.cert_pem.clone(),
        tls_key_pem: tls.key_pem.clone(),
        token: TOKEN.to_string(),
        instance_id: INSTANCE.to_string(),
        tui_version: "0.1.0-test".to_string(),
        workspace: WorkspaceInfo { id: "4b156f1de41da274".into(), display_name: "gaviero".into() },
        capabilities: vec![],
        confirm_required: vec!["/autoapprove".into(), "/yolo".into(), "/reset".into(), "/clear".into()],
        allowed_slash_commands: vec!["/model".into(), "/help".into()],
        limits: Limits {
            max_frame_bytes: 262_144,
            max_prompt_bytes: 131_072,
            command_rate_per_second: 1000,
        },
        ping_interval: Duration::from_secs(20),
        idle_timeout: Duration::from_secs(60),
        hello_timeout: Duration::from_secs(5),
    }
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn connect_raw(
    tls: &TestTls,
    addr: SocketAddr,
    host: &str,
    token: Option<&str>,
    subprotocol: Option<&str>,
) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let tcp = TcpStream::connect(addr).await.expect("tcp connect");
    let url = format!("wss://{host}:{}{}", addr.port(), WS_PATH);
    let mut request = url.into_client_request().unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
    }
    if let Some(proto) = subprotocol {
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", proto.parse().unwrap());
    }
    let (ws, _resp) = client_async_tls_with_config(
        request,
        tcp,
        None,
        Some(Connector::Rustls(tls.client_config.clone())),
    )
    .await?;
    Ok(ws)
}

fn client_hello_frame(major: u16) -> String {
    let env = ClientEnvelope {
        version: PROTOCOL_VERSION,
        instance_id: None,
        command_id: "cmd-hello".into(),
        frame: ClientFrame::ClientHello(ClientHello {
            protocol_version: ProtocolVersion { major, minor: 0 },
            client_name: "test-client".into(),
            client_version: "0.0.1".into(),
        }),
    };
    serde_json::to_string(&env).unwrap()
}

async fn next_frame(ws: &mut Ws) -> ServerEnvelope {
    use futures::StreamExt;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for server frame")
            .expect("socket ended")
            .expect("socket error");
        match msg {
            Message::Text(text) => return serde_json::from_str(text.as_str()).expect("decode"),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected message: {other:?}"),
        }
    }
}

/// Wait for the close frame, skipping data frames, and return its code.
async fn next_close_code(ws: &mut Ws) -> u16 {
    use futures::StreamExt;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("timed out waiting for close");
        match msg {
            Some(Ok(Message::Close(Some(frame)))) => return frame.code.into(),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => panic!("socket ended without a close frame"),
        }
    }
}

/// Full happy-path handshake; returns the socket after verifying `hello`.
async fn connect_ok(tls: &TestTls, addr: SocketAddr) -> Ws {
    use futures::SinkExt;
    let mut ws = connect_raw(tls, addr, TEST_HOST, Some(TOKEN), Some(SUBPROTOCOL))
        .await
        .expect("handshake");
    ws.send(Message::Text(client_hello_frame(1).into())).await.unwrap();
    let hello = next_frame(&mut ws).await;
    assert_eq!(hello.instance_id, INSTANCE);
    let ServerFrame::Hello(h) = &hello.frame else {
        panic!("first frame must be hello, got {:?}", hello.frame)
    };
    assert_eq!(h.workspace.id, "4b156f1de41da274");
    ws
}

async fn expect_output(server: &mut SpawnedServer, expect: &str) {
    let out = tokio::time::timeout(Duration::from_secs(5), server.outputs.recv())
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for HubOutput::{expect}"))
        .expect("output channel closed");
    let got = match &out {
        HubOutput::Command(_) => "Command",
        HubOutput::ClientConnected => "ClientConnected",
        HubOutput::ClientDisconnected => "ClientDisconnected",
        HubOutput::SnapshotNeeded => "SnapshotNeeded",
    };
    assert_eq!(got, expect, "unexpected hub output: {out:?}");
}

fn send_prompt_envelope(command_id: &str) -> String {
    let env = ClientEnvelope {
        version: PROTOCOL_VERSION,
        instance_id: Some(INSTANCE.into()),
        command_id: command_id.into(),
        frame: ClientFrame::SendPrompt(SendPrompt {
            conv_id: "conv-1".into(),
            text: "hi".into(),
        }),
    };
    serde_json::to_string(&env).unwrap()
}

// ── Auth and TLS ─────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn missing_or_wrong_bearer_rejected_before_upgrade() {
    let tls = make_tls();
    let server = spawn(test_config(&tls)).await.unwrap();

    for token in [None, Some("wrong-token")] {
        let err = connect_raw(&tls, server.local_addr, TEST_HOST, token, Some(SUBPROTOCOL))
            .await
            .expect_err("upgrade must be rejected");
        match err {
            tokio_tungstenite::tungstenite::Error::Http(resp) => {
                assert_eq!(resp.status(), http::StatusCode::UNAUTHORIZED)
            }
            other => panic!("expected HTTP 401, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_subprotocol_rejected_before_upgrade() {
    let tls = make_tls();
    let server = spawn(test_config(&tls)).await.unwrap();
    let err = connect_raw(&tls, server.local_addr, TEST_HOST, Some(TOKEN), None)
        .await
        .expect_err("upgrade must be rejected");
    match err {
        tokio_tungstenite::tungstenite::Error::Http(resp) => {
            assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST)
        }
        other => panic!("expected HTTP 400, got {other:?}"),
    }
}

/// The client validates the certificate hostname: a mismatching SNI/URL
/// host fails TLS even though the certificate is otherwise trusted.
#[tokio::test(flavor = "multi_thread")]
async fn certificate_hostname_is_verified() {
    let tls = make_tls();
    let server = spawn(test_config(&tls)).await.unwrap();
    let err = connect_raw(
        &tls,
        server.local_addr,
        "wrong-host.gaviero.invalid",
        Some(TOKEN),
        Some(SUBPROTOCOL),
    )
    .await
    .expect_err("hostname mismatch must fail TLS");
    let msg = format!("{err:?}");
    assert!(msg.contains("Tls") || msg.contains("Io"), "unexpected error: {msg}");
}

// ── Handshake, versioning, eviction ──────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn wrong_major_closed_4002_without_evicting_live_client() {
    use futures::SinkExt;
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();

    let mut first = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    // Wrong-major client: closed 4002 before registration.
    let mut bad = connect_raw(&tls, server.local_addr, TEST_HOST, Some(TOKEN), Some(SUBPROTOCOL))
        .await
        .unwrap();
    bad.send(Message::Text(client_hello_frame(2).into())).await.unwrap();
    assert_eq!(next_close_code(&mut bad).await, close_code::UNSUPPORTED_VERSION);

    // The live client was not evicted: it still receives events.
    server
        .handle
        .try_send(HubInput::Event {
            revision: 1,
            frame: ServerFrame::StreamingStatus(StreamingStatus {
                conv_id: "conv-1".into(),
                turn_id: "turn-1".into(),
                status: "thinking".into(),
            }),
        })
        .unwrap();
    let frame = next_frame(&mut first).await;
    assert!(matches!(frame.frame, ServerFrame::StreamingStatus(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn second_valid_client_evicts_first_with_4005() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();

    let mut first = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    let _second = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientDisconnected").await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    assert_eq!(next_close_code(&mut first).await, close_code::REPLACED);
}

#[tokio::test(flavor = "multi_thread")]
async fn token_rotation_closes_4006_and_old_token_stops_working() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();

    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    server
        .handle
        .try_send(HubInput::TokenRotated { new_token: "rotated-token".into() })
        .unwrap();
    assert_eq!(next_close_code(&mut ws).await, close_code::TOKEN_ROTATED);
    expect_output(&mut server, "ClientDisconnected").await;

    let err = connect_raw(&tls, server.local_addr, TEST_HOST, Some(TOKEN), Some(SUBPROTOCOL))
        .await
        .expect_err("old token must be rejected");
    assert!(matches!(err, tokio_tungstenite::tungstenite::Error::Http(_)));

    let _ws = connect_raw(&tls, server.local_addr, TEST_HOST, Some("rotated-token"), Some(SUBPROTOCOL))
        .await
        .expect("new token accepted");
}

// ── Ordering and coalescing ──────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn interleaved_chunks_coalesce_per_conversation_in_order() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    let chunk = |conv: &str, turn: &str, text: &str| HubInput::Event {
        revision: 1,
        frame: ServerFrame::StreamChunk(StreamChunk {
            conv_id: conv.into(),
            turn_id: turn.into(),
            text: text.into(),
        }),
    };
    server.handle.try_send(chunk("conv-a", "turn-a", "a1 ")).unwrap();
    server.handle.try_send(chunk("conv-b", "turn-b", "b1 ")).unwrap();
    server.handle.try_send(chunk("conv-a", "turn-a", "a2")).unwrap();
    server.handle.try_send(chunk("conv-b", "turn-b", "b2")).unwrap();

    let mut seen = Vec::new();
    let mut last_seq = 0;
    for _ in 0..2 {
        let env = next_frame(&mut ws).await;
        assert!(env.seq > last_seq, "seq must be monotonic");
        last_seq = env.seq;
        let ServerFrame::StreamChunk(c) = env.frame else { panic!("expected chunk") };
        seen.push((c.conv_id, c.text));
    }
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("conv-a".to_string(), "a1 a2".to_string()),
            ("conv-b".to_string(), "b1 b2".to_string()),
        ],
        "chunks must coalesce per conversation, never across"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn non_chunk_event_flushes_preceding_chunk_run() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    server
        .handle
        .try_send(HubInput::Event {
            revision: 1,
            frame: ServerFrame::StreamChunk(StreamChunk {
                conv_id: "conv-a".into(),
                turn_id: "turn-a".into(),
                text: "partial text".into(),
            }),
        })
        .unwrap();
    server
        .handle
        .try_send(HubInput::Event {
            revision: 1,
            frame: ServerFrame::StreamingStatus(StreamingStatus {
                conv_id: "conv-a".into(),
                turn_id: "turn-a".into(),
                status: "running tool".into(),
            }),
        })
        .unwrap();

    let first = next_frame(&mut ws).await;
    let ServerFrame::StreamChunk(c) = first.frame else {
        panic!("chunk must be flushed before the status that follows it")
    };
    assert_eq!(c.text, "partial text");
    let second = next_frame(&mut ws).await;
    assert!(matches!(second.frame, ServerFrame::StreamingStatus(_)));
}

#[tokio::test(flavor = "multi_thread")]
async fn newer_snapshot_replaces_queued_snapshot() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    let snap = |revision: u64| {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/server/snapshot.json"),
        )
        .unwrap();
        let env: ServerEnvelope = serde_json::from_str(&text).unwrap();
        let ServerFrame::Snapshot(mut s) = env.frame else { unreachable!() };
        s.revision = revision;
        Box::new(s)
    };
    server.handle.try_send(HubInput::Snapshot(snap(10))).unwrap();
    server.handle.try_send(HubInput::Snapshot(snap(11))).unwrap();

    let env = next_frame(&mut ws).await;
    let ServerFrame::Snapshot(s) = env.frame else { panic!("expected snapshot") };
    assert_eq!(s.revision, 11, "older queued snapshot must be replaced");
}

// ── Command intake ───────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn commands_flow_and_duplicates_are_dropped() {
    use futures::SinkExt;
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    ws.send(Message::Text(send_prompt_envelope("cmd-1").into())).await.unwrap();
    ws.send(Message::Text(send_prompt_envelope("cmd-1").into())).await.unwrap();
    ws.send(Message::Text(send_prompt_envelope("cmd-2").into())).await.unwrap();

    let out = tokio::time::timeout(Duration::from_secs(5), server.outputs.recv())
        .await.unwrap().unwrap();
    let HubOutput::Command(env) = out else { panic!("expected command") };
    assert_eq!(env.command_id, "cmd-1");
    let out = tokio::time::timeout(Duration::from_secs(5), server.outputs.recv())
        .await.unwrap().unwrap();
    let HubOutput::Command(env) = out else { panic!("duplicate must be dropped silently") };
    assert_eq!(env.command_id, "cmd-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_command_type_answered_with_command_error() {
    use futures::SinkExt;
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    let raw = format!(
        r#"{{"version":{{"major":1,"minor":1}},"instance_id":"{INSTANCE}","command_id":"cmd-x","type":"rotate_token","payload":{{}}}}"#
    );
    ws.send(Message::Text(raw.into())).await.unwrap();
    let env = next_frame(&mut ws).await;
    let ServerFrame::CommandError(e) = env.frame else { panic!("expected command_error") };
    assert_eq!(e.command_id, "cmd-x");
    assert_eq!(e.code, gaviero_remote::dto::ErrorCode::UnknownType);
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_instance_id_rejected_as_invalid_payload() {
    use futures::SinkExt;
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    let env = ClientEnvelope {
        version: PROTOCOL_VERSION,
        instance_id: Some("stale-instance".into()),
        command_id: "cmd-stale".into(),
        frame: ClientFrame::SendPrompt(SendPrompt { conv_id: "c".into(), text: "hi".into() }),
    };
    ws.send(Message::Text(serde_json::to_string(&env).unwrap().into())).await.unwrap();
    let frame = next_frame(&mut ws).await;
    let ServerFrame::CommandError(e) = frame.frame else { panic!("expected command_error") };
    assert_eq!(e.code, gaviero_remote::dto::ErrorCode::InvalidPayload);
}

#[tokio::test(flavor = "multi_thread")]
async fn command_flood_hits_rate_limit() {
    use futures::SinkExt;
    let tls = make_tls();
    let mut config = test_config(&tls);
    config.limits.command_rate_per_second = 2;
    let mut server = spawn(config).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    for i in 0..6 {
        ws.send(Message::Text(send_prompt_envelope(&format!("cmd-{i}")).into())).await.unwrap();
    }
    let mut rate_limited = false;
    for _ in 0..6 {
        let env = next_frame(&mut ws).await;
        if let ServerFrame::CommandError(e) = env.frame
            && e.code == gaviero_remote::dto::ErrorCode::RateLimited
        {
            rate_limited = true;
            break;
        }
    }
    assert!(rate_limited, "flood must produce rate_limited errors");
}

// ── Malformed input ──────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn oversized_frame_closed_4004() {
    use futures::SinkExt;
    let tls = make_tls();
    let mut config = test_config(&tls);
    config.limits.max_frame_bytes = 4096;
    let server = spawn(config).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;

    let big = send_prompt_envelope(&"x".repeat(8192));
    ws.send(Message::Text(big.into())).await.unwrap();
    assert_eq!(next_close_code(&mut ws).await, close_code::FRAME_TOO_LARGE);
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_json_closed_4003() {
    use futures::SinkExt;
    let tls = make_tls();
    let server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    ws.send(Message::Text("{not json".into())).await.unwrap();
    assert_eq!(next_close_code(&mut ws).await, close_code::PROTOCOL_ERROR);
}

#[tokio::test(flavor = "multi_thread")]
async fn binary_frame_closed_4003() {
    use futures::SinkExt;
    let tls = make_tls();
    let server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    ws.send(Message::Binary(vec![1, 2, 3].into())).await.unwrap();
    assert_eq!(next_close_code(&mut ws).await, close_code::PROTOCOL_ERROR);
}

// ── Backpressure and liveness ────────────────────────────────────

/// A client that stops reading cannot grow server memory without bound:
/// the bounded outbound queue fills and the hub drops the client (4008).
#[tokio::test(flavor = "multi_thread")]
async fn slow_client_is_dropped() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let _ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    // Distinct non-chunk frames (no coalescing), large enough to overrun
    // both the 128-frame queue and the socket buffers while unread.
    let blob = "y".repeat(32 * 1024);
    for i in 0..400 {
        let _ = server.handle.try_send(HubInput::Event {
            revision: 1,
            frame: ServerFrame::StreamingStatus(StreamingStatus {
                conv_id: format!("conv-{}", i % 7),
                turn_id: "turn".into(),
                status: blob.clone(),
            }),
        });
        tokio::time::sleep(Duration::from_millis(1)).await;
        if let Ok(out) = server.outputs.try_recv() {
            if matches!(out, HubOutput::ClientDisconnected) {
                return; // dropped as slow — pass
            }
        }
    }
    expect_output(&mut server, "ClientDisconnected").await;
}

/// A half-open connection (peer stops reading and ponging) is closed by
/// the ping/idle deadline.
#[tokio::test(flavor = "multi_thread")]
async fn ping_timeout_closes_half_open_connection() {
    let tls = make_tls();
    let mut config = test_config(&tls);
    config.ping_interval = Duration::from_millis(100);
    config.idle_timeout = Duration::from_millis(350);
    let mut server = spawn(config).await.unwrap();
    let ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    // Stop reading entirely: no pongs, no traffic.
    std::mem::forget(ws);
    expect_output(&mut server, "ClientDisconnected").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn shutdown_closes_with_4007() {
    let tls = make_tls();
    let mut server = spawn(test_config(&tls)).await.unwrap();
    let mut ws = connect_ok(&tls, server.local_addr).await;
    expect_output(&mut server, "ClientConnected").await;
    expect_output(&mut server, "SnapshotNeeded").await;

    server.handle.try_send(HubInput::Shutdown).unwrap();
    assert_eq!(next_close_code(&mut ws).await, close_code::SERVER_SHUTDOWN);
}
