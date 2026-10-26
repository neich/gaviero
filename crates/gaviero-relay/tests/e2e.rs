//! End to end: the REAL relay + the REAL host side from gaviero-remote
//! (`gaviero_remote::relay_client` + the sidecar with a pinned-CA leaf) +
//! a rustls "phone". Proves live control and notifications through the
//! relay, and that the relay never needs (or sees) the bearer token.

use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use gaviero_relay::Config;
use gaviero_remote::dto::{Limits, WorkspaceInfo};
use gaviero_remote::pairing;
use gaviero_remote::relay_client::{
    RelayClient, RelayClientConfig, RelayNotifyKind, RelayRoute, RelayStatus,
};
use gaviero_remote::server::{RemoteServerConfig, SpawnedServer, spawn};
use rustls::pki_types::pem::PemObject;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const PUBLIC_HOST: &str = "gaviero.example.com";
const MACHINE_HOST: &str = "desk.gaviero.example.com";
const ROUTE: &str = "0b7d245998c0e8c3-desk.gaviero.example.com";
const SECRET: &str = "relay-secret-for-tests-0123456789";

struct Rig {
    _dir: tempfile::TempDir,
    _sidecar: SpawnedServer,
    relay: gaviero_relay::RunningRelay,
    client: RelayClient,
    ca_b64: String,
    token: String,
}

async fn sidecar(bundle: Vec<u8>, token: String) -> SpawnedServer {
    let (ping, idle, hello_timeout) = RemoteServerConfig::timing_defaults();
    spawn(RemoteServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        extra_bind_addrs: Vec::new(),
        tls_cert_pem: bundle.clone(),
        tls_key_pem: bundle,
        token,
        instance_id: "inst-relay-e2e".to_string(),
        tui_version: "0.1.0".to_string(),
        workspace: WorkspaceInfo {
            id: "0b7d245998c0e8c3".to_string(),
            display_name: "gaviero".to_string(),
        },
        capabilities: Vec::new(),
        machine: None,
        token_path: None,
        token_poll_interval: RemoteServerConfig::TOKEN_POLL_INTERVAL,
        registry: None,
        heartbeat_interval: gaviero_remote::server::registry::HEARTBEAT_INTERVAL,
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

async fn rig_with_secret(client_secret: &str) -> Rig {
    let mut cfg = Config::new(SECRET);
    cfg.http_addr = "127.0.0.1:0".parse().unwrap();
    cfg.splice_addr = "127.0.0.1:0".parse().unwrap();
    cfg.public_host = Some(PUBLIC_HOST.into());
    let relay = gaviero_relay::start(cfg).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let pinned = pairing::ensure_pinned_ca_with(
        dir.path(),
        MACHINE_HOST,
        &[format!("*.{PUBLIC_HOST}")],
    )
    .unwrap();
    let bundle = std::fs::read(&pinned.leaf_path).unwrap();
    let token = pairing::generate_token();
    let server = sidecar(bundle, token.clone()).await;

    let client = RelayClient::spawn(RelayClientConfig {
        base_url: format!("ws://{}", relay.http_addr),
        secret: client_secret.into(),
        route: RelayRoute {
            route: ROUTE.into(),
            machine: MACHINE_HOST.into(),
            workspace_id: "0b7d245998c0e8c3".into(),
            display_name: "gaviero".into(),
        },
        local_addr: server.local_addr,
    });
    Rig {
        _dir: dir,
        _sidecar: server,
        relay,
        client,
        ca_b64: pinned.ca_der_b64,
        token,
    }
}

async fn wait_status(client: &RelayClient, want: impl Fn(&RelayStatus) -> bool) -> RelayStatus {
    let mut rx = client.status_receiver();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let now = rx.borrow().clone();
            if want(&now) {
                return now;
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("status never matched; last {:?}", client.status()))
}

fn phone_tls(ca_b64: &str) -> Arc<rustls::ClientConfig> {
    let pem = format!("-----BEGIN CERTIFICATE-----\n{ca_b64}\n-----END CERTIFICATE-----\n");
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from_pem_slice(pem.as_bytes()).unwrap())
        .unwrap();
    Arc::new(
        rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
}

#[tokio::test]
async fn phone_reaches_the_sidecar_through_the_relay() {
    let rig = rig_with_secret(SECRET).await;
    wait_status(&rig.client, |s| *s == RelayStatus::Registered).await;

    // The phone: TCP to the relay's splice port, TLS with SNI = route and
    // the pinned CA, then the normal Gaviero Remote WSS handshake.
    let mut request = format!("wss://{ROUTE}/v1/ws").into_client_request().unwrap();
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", rig.token).parse().unwrap(),
    );
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "gaviero.v1".parse().unwrap());
    let tcp = TcpStream::connect(rig.relay.splice_addr).await.unwrap();
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::client_async_tls_with_config(
            request,
            tcp,
            None,
            Some(tokio_tungstenite::Connector::Rustls(phone_tls(&rig.ca_b64))),
        ),
    )
    .await
    .expect("no hang")
    .expect("TLS + WSS upgrade through the relay");

    let hello = serde_json::json!({
        "version": { "major": 1, "minor": 2 }, "instance_id": null, "command_id": "cmd-1",
        "type": "client_hello",
        "payload": { "protocol_version": { "major": 1, "minor": 2 },
                     "client_name": "relay-e2e", "client_version": "0.1.0" }
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
    rig.client.shutdown();
}

#[tokio::test]
async fn unknown_route_fails_fast_instead_of_hanging() {
    let rig = rig_with_secret(SECRET).await;
    wait_status(&rig.client, |s| *s == RelayStatus::Registered).await;
    let tcp = TcpStream::connect(rig.relay.splice_addr).await.unwrap();
    let connector = tokio_rustls::TlsConnector::from(phone_tls(&rig.ca_b64));
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        connector.connect(
            rustls::pki_types::ServerName::try_from("nobody-desk.gaviero.example.com").unwrap(),
            tcp,
        ),
    )
    .await
    .expect("must not hang");
    assert!(result.is_err());
}

/// Read the NDJSON phone stream over plain HTTP until `pred` matches a line.
async fn read_stream_until(
    http: std::net::SocketAddr,
    query: &str,
    secret: &str,
    pred: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let mut tcp = TcpStream::connect(http).await.unwrap();
    let req = format!(
        "GET /v1/phone/stream?{query} HTTP/1.1\r\nHost: relay\r\nAuthorization: Bearer {secret}\r\n\r\n"
    );
    tcp.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let mut chunk = [0u8; 4096];
            let n = tcp.read(&mut chunk).await.unwrap();
            assert!(n > 0, "stream closed early: {}", String::from_utf8_lossy(&buf));
            buf.extend_from_slice(&chunk[..n]);
            // Chunked framing interleaves size lines; JSON lines start with '{'.
            for line in String::from_utf8_lossy(&buf).lines() {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) {
                    if pred(&v) {
                        return v;
                    }
                }
            }
        }
    })
    .await
    .expect("expected event arrives")
}

#[tokio::test]
async fn notifications_reach_phones_live_and_on_replay() {
    let rig = rig_with_secret(SECRET).await;
    wait_status(&rig.client, |s| *s == RelayStatus::Registered).await;

    // Presence on connect lists the route (the phone's instance directory).
    let presence = read_stream_until(rig.relay.http_addr, "since=0", SECRET, |v| {
        v["type"] == "presence"
    })
    .await;
    assert_eq!(presence["routes"][0]["route"], ROUTE);

    // Sent while no phone is connected → replayed on connect (since=0).
    rig.client.notify(RelayNotifyKind::AgentWaiting, "conv-1");
    let replayed = read_stream_until(rig.relay.http_addr, "since=0", SECRET, |v| {
        v["type"] == "notify"
    })
    .await;
    assert_eq!(replayed["conv"], "conv-1");
    assert_eq!(replayed["kind"], "agent_waiting");
    assert_eq!(replayed["machine"], MACHINE_HOST);
    assert_eq!(replayed["workspace_id"], "0b7d245998c0e8c3");
    let id = replayed["id"].as_u64().unwrap();

    // since=<last id> skips what the phone already has; a live one arrives.
    let http = rig.relay.http_addr;
    let live = tokio::spawn(async move {
        read_stream_until(http, &format!("since={id}"), SECRET, |v| v["type"] == "notify").await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.client.notify(RelayNotifyKind::AgentFinished, "conv-2");
    let live = live.await.unwrap();
    assert_eq!(live["conv"], "conv-2", "the replayed conv-1 must be skipped");
    assert_eq!(live["kind"], "agent_finished");

    // A `since` from another relay epoch (the relay restarted and its ids
    // began again at 1) is ignored: everything stored is replayed.
    let epoch = presence["epoch"].as_str().unwrap().to_string();
    assert_eq!(epoch.len(), 16);
    let stale = read_stream_until(
        rig.relay.http_addr,
        "since=999&epoch=0000000000000000",
        SECRET,
        |v| v["type"] == "notify",
    )
    .await;
    assert_eq!(stale["conv"], "conv-1", "stale epoch ⇒ replay from the start");
    // The same `since` with the current epoch is honoured.
    let current = read_stream_until(
        rig.relay.http_addr,
        &format!("since={id}&epoch={epoch}"),
        SECRET,
        |v| v["type"] == "notify",
    )
    .await;
    assert_eq!(current["conv"], "conv-2");

    // A phone that never saw an epoch but holds a `since` from a previous
    // relay run (higher than any id issued here) must still get NEW
    // notifications live — not have them filtered out forever.
    let http = rig.relay.http_addr;
    let fresh = tokio::spawn(async move {
        read_stream_until(http, "since=999", SECRET, |v| {
            v["type"] == "notify" && v["conv"] == "conv-3"
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.client.notify(RelayNotifyKind::AgentWaiting, "conv-3");
    assert_eq!(fresh.await.unwrap()["conv"], "conv-3");
    rig.client.shutdown();
}

#[tokio::test]
async fn a_wrong_secret_is_reported_not_retried_silently() {
    let rig = rig_with_secret("not-the-relay-secret-at-all").await;
    let status = wait_status(&rig.client, |s| matches!(s, RelayStatus::Disconnected(_))).await;
    let RelayStatus::Disconnected(reason) = status else { unreachable!() };
    assert!(reason.contains("401"), "{reason}");
    rig.client.shutdown();
}

/// Manual: run a relay on `GAVIERO_RELAY_FEED_PORT` (127.0.0.1, http) with
/// the test secret, register a host and send a notification every 20 s for
/// `GAVIERO_RELAY_FEED_SECS` — for the Android emulator check of the native
/// alert service (it reaches the host's loopback as 10.0.2.2).
#[tokio::test]
#[ignore = "manual: notification feed for the emulator end-to-end check"]
async fn feed_notifications_for_the_emulator() {
    let port: u16 = std::env::var("GAVIERO_RELAY_FEED_PORT").unwrap().parse().unwrap();
    let secs: u64 = std::env::var("GAVIERO_RELAY_FEED_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    let mut cfg = Config::new(SECRET);
    cfg.http_addr = ([127, 0, 0, 1], port).into();
    cfg.splice_addr = "127.0.0.1:0".parse().unwrap();
    cfg.public_host = Some(PUBLIC_HOST.into());
    let relay = gaviero_relay::start(cfg).await.unwrap();
    let client = RelayClient::spawn(RelayClientConfig {
        base_url: format!("ws://{}", relay.http_addr),
        secret: SECRET.into(),
        route: RelayRoute {
            route: ROUTE.into(),
            machine: MACHINE_HOST.into(),
            workspace_id: "0b7d245998c0e8c3".into(),
            display_name: "gaviero".into(),
        },
        local_addr: "127.0.0.1:9".parse().unwrap(),
    });
    wait_status(&client, |s| *s == RelayStatus::Registered).await;
    println!("feed ready on {}", relay.http_addr);
    let start = std::time::Instant::now();
    let mut n = 0u32;
    while start.elapsed() < Duration::from_secs(secs) {
        tokio::time::sleep(Duration::from_secs(20)).await;
        n += 1;
        let kind = if n % 2 == 0 { RelayNotifyKind::AgentFinished } else { RelayNotifyKind::AgentWaiting };
        client.notify(kind, &format!("conv-{n}"));
        println!("{:>6.1}s sent notify conv-{n}", start.elapsed().as_secs_f64());
    }
    client.shutdown();
}

#[tokio::test]
async fn phone_stream_requires_the_secret() {
    let rig = rig_with_secret(SECRET).await;
    let mut tcp = TcpStream::connect(rig.relay.http_addr).await.unwrap();
    tcp.write_all(b"GET /v1/phone/stream HTTP/1.1\r\nHost: relay\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = String::new();
    tokio::time::timeout(Duration::from_secs(5), tcp.read_to_string(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert!(buf.starts_with("HTTP/1.1 401"), "{buf}");
    rig.client.shutdown();
}
