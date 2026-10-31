use std::sync::Mutex as StdMutex;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

struct Answer(Option<bool>, StdMutex<Vec<tokio::sync::oneshot::Sender<bool>>>);

impl EgressObserver for Answer {
    fn on_egress(&self, _event: &EgressEvent) {}
    fn ask(&self, _provider: &str, _host: &str, respond: tokio::sync::oneshot::Sender<bool>) {
        match self.0 {
            Some(answer) => {
                let _ = respond.send(answer);
            }
            // Never answer: keep the sender alive so the proxy times out.
            None => self.1.lock().unwrap().push(respond),
        }
    }
}

fn service(
    dir: &Path,
    mode: NetworkMode,
    allow: &[&str],
    answer: Option<bool>,
    ask_timeout: Duration,
) -> Arc<EgressService> {
    EgressService::new(
        EgressConfig {
            mode,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            ask_timeout,
            log_dir: dir.to_path_buf(),
        },
        Arc::new(Answer(answer, StdMutex::new(Vec::new()))),
    )
}

/// An upstream that echoes whatever it receives.
async fn echo_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                while let Ok(n) = s.read(&mut buf).await {
                    if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

/// CONNECT through `proxy` to `target`; returns the proxy's status line.
async fn connect_through(proxy: SocketAddr, target: SocketAddr) -> (TcpStream, String) {
    let mut s = TcpStream::connect(proxy).await.unwrap();
    let head = format!(
        "CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n",
        port = target.port()
    );
    s.write_all(head.as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 256];
    let n = s.read(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf[..n]).to_string();
    (s, text)
}

async fn wait_for_log(dir: &Path) -> String {
    let path = dir.join("connections.ndjson");
    for _ in 0..100 {
        if let Ok(body) = std::fs::read_to_string(&path)
            && !body.is_empty()
        {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no egress log at {}", path.display());
}

#[test]
fn host_allowed_matches_exact_and_wildcard_suffix() {
    let allow = vec!["*.anthropic.com".to_string(), "chatgpt.com".to_string()];
    assert!(host_allowed(&allow, "api.anthropic.com"));
    assert!(host_allowed(&allow, "API.Anthropic.com"));
    assert!(!host_allowed(&allow, "anthropic.com"));
    assert!(host_allowed(&allow, "chatgpt.com"));
    assert!(!host_allowed(&allow, "ab.chatgpt.com"));
    assert!(!host_allowed(&allow, "evilanthropic.com"));
}

#[test]
fn network_mode_parse_defaults_to_log() {
    assert_eq!(NetworkMode::parse("off"), NetworkMode::Off);
    assert_eq!(NetworkMode::parse("Enforce"), NetworkMode::Enforce);
    assert_eq!(NetworkMode::parse("ask"), NetworkMode::Ask);
    assert_eq!(NetworkMode::parse("bogus"), NetworkMode::Log);
}

#[test]
fn proxy_env_points_every_variable_at_loopback() {
    let env = proxy_env("127.0.0.1:4242".parse().unwrap());
    let get = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
    assert_eq!(get("HTTPS_PROXY").as_deref(), Some("http://127.0.0.1:4242"));
    assert_eq!(get("https_proxy").as_deref(), Some("http://127.0.0.1:4242"));
    assert_eq!(get("NO_PROXY").as_deref(), Some("127.0.0.1,localhost,::1"));
    assert_eq!(get("NODE_USE_ENV_PROXY").as_deref(), Some("1"));
}

#[test]
fn egress_settings_resolve_to_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::single_folder(dir.path().to_path_buf());
    let cfg = EgressConfig::from_workspace(&ws, dir.path());
    assert_eq!(cfg.mode, NetworkMode::Log);
    assert!(cfg.allow.contains(&"*.anthropic.com".to_string()));
    assert_eq!(cfg.ask_timeout, Duration::from_secs(60));
    assert_eq!(
        ws.resolve_setting(settings::AGENT_EGRESS_CAPTURE_BODIES, Some(dir.path())),
        serde_json::json!(true)
    );
}

#[tokio::test]
async fn log_mode_relays_and_records_an_allowed_connection() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = echo_server().await;
    let svc = service(dir.path(), NetworkMode::Log, &["127.0.0.1"], None, Duration::from_secs(1));
    let proxy = svc.listener_for("test").unwrap();
    assert_eq!(svc.listener_for("test"), Some(proxy), "listener is cached");
    let (mut s, status) = connect_through(proxy, upstream).await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
    s.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    s.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
    drop(s);
    let log = wait_for_log(dir.path()).await;
    let row: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(row["decision"], "allowed");
    assert_eq!(row["provider"], "test");
    assert_eq!(row["host"], "127.0.0.1");
    assert_eq!(row["first_seen"], true);
}

#[tokio::test]
async fn log_mode_lets_unlisted_hosts_through() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = echo_server().await;
    let svc = service(dir.path(), NetworkMode::Log, &[], None, Duration::from_secs(1));
    let proxy = svc.listener_for("test").unwrap();
    let (_s, status) = connect_through(proxy, upstream).await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
}

#[tokio::test]
async fn enforce_mode_refuses_unlisted_hosts() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = echo_server().await;
    let svc = service(dir.path(), NetworkMode::Enforce, &[], None, Duration::from_secs(1));
    let proxy = svc.listener_for("test").unwrap();
    let (_s, status) = connect_through(proxy, upstream).await;
    assert!(status.starts_with("HTTP/1.1 403"), "{status}");
    let log = wait_for_log(dir.path()).await;
    assert!(log.contains("\"decision\":\"denied\""), "{log}");
}

#[tokio::test]
async fn ask_mode_relays_when_the_user_allows() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = echo_server().await;
    let svc = service(dir.path(), NetworkMode::Ask, &[], Some(true), Duration::from_secs(5));
    let proxy = svc.listener_for("test").unwrap();
    let (_s, status) = connect_through(proxy, upstream).await;
    assert!(status.starts_with("HTTP/1.1 200"), "{status}");
}

#[tokio::test]
async fn ask_mode_times_out_to_deny() {
    let dir = tempfile::tempdir().unwrap();
    let upstream = echo_server().await;
    let svc = service(dir.path(), NetworkMode::Ask, &[], None, Duration::from_secs(1));
    let proxy = svc.listener_for("test").unwrap();
    let started = std::time::Instant::now();
    let (_s, status) = connect_through(proxy, upstream).await;
    assert!(status.starts_with("HTTP/1.1 403"), "{status}");
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[test]
fn claude_body_dir_is_inside_the_read_floor() {
    let dir = claude_body_dir(Path::new("/w"));
    assert!(
        dir.to_string_lossy().replace('\\', "/").ends_with(".gaviero/egress/claude-bodies"),
        "{}",
        dir.display()
    );
    assert!(crate::scope_enforcer::read_floor_match(&dir.join("x.request.json")).is_some());
}

#[test]
fn capture_claude_bodies_defaults_true_and_honours_settings() {
    let dir = tempfile::tempdir().unwrap();
    assert!(capture_claude_bodies(dir.path()));
    std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
    std::fs::write(
        dir.path().join(".gaviero/settings.json"),
        r#"{"agent":{"egress":{"captureClaudeBodies":false}}}"#,
    )
    .unwrap();
    assert!(!capture_claude_bodies(dir.path()));
}
