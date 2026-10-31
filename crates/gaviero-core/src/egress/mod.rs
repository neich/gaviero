//! Outbound-traffic observation for agents.
//!
//! Every spawned agent CLI (and gaviero's own HTTP clients) is pointed at a
//! loopback CONNECT proxy through `HTTPS_PROXY` / `HTTP_PROXY`. The proxy
//! does **not** decrypt TLS: it sees which host each provider label connects
//! to, logs it to `<root>/.gaviero/egress/connections.ndjson`, and — per
//! `agent.network.mode` — warns (`log`), asks (`ask`) or refuses (`enforce`)
//! when the host is not in `agent.network.allow`. Probed 2026-10-31: Claude
//! Code (and its Bash children), Codex and the Cursor CLI all honour the
//! proxy variables (Cursor needs `NODE_USE_ENV_PROXY=1`).
//!
//! One listener per provider label gives attribution without trusting the
//! client to identify itself. Clients that ignore the proxy variables are
//! not seen — this is observation, not confinement.

pub mod bodies;
mod proxy;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;

use crate::util::ndjson::NdjsonAppender;
use crate::workspace::{Workspace, settings};

/// Size cap of `connections.ndjson` before it rotates.
const CONNECTIONS_LOG_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// `agent.network.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    /// No proxy is installed.
    Off,
    /// Proxy everything; warn about unlisted hosts.
    Log,
    /// Hold connections to unlisted hosts until the user decides.
    Ask,
    /// Refuse connections to unlisted hosts.
    Enforce,
}

impl NetworkMode {
    /// Unknown values fall back to [`NetworkMode::Log`].
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Self::Off,
            "ask" => Self::Ask,
            "enforce" => Self::Enforce,
            _ => Self::Log,
        }
    }
}

/// The resolved egress settings for one workspace root.
#[derive(Debug, Clone)]
pub struct EgressConfig {
    pub mode: NetworkMode,
    pub allow: Vec<String>,
    pub ask_timeout: Duration,
    pub log_dir: PathBuf,
}

fn string_list(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

impl EgressConfig {
    /// Read `agent.network.*` through the workspace settings cascade.
    pub fn from_workspace(workspace: &Workspace, root: &Path) -> Self {
        let mode = workspace
            .resolve_setting(settings::AGENT_NETWORK_MODE, Some(root))
            .as_str()
            .map(NetworkMode::parse)
            .unwrap_or(NetworkMode::Log);
        let allow = string_list(&workspace.resolve_setting(settings::AGENT_NETWORK_ALLOW, Some(root)));
        let ask_timeout = workspace
            .resolve_setting(settings::AGENT_NETWORK_ASK_TIMEOUT_SECS, Some(root))
            .as_u64()
            .unwrap_or(60);
        Self {
            mode,
            allow,
            ask_timeout: Duration::from_secs(ask_timeout),
            log_dir: root.join(".gaviero").join("egress"),
        }
    }
}

/// Is `host` covered by `allow`? Exact (case-insensitive) or `*.suffix`,
/// which matches subdomains but not the bare suffix.
pub fn host_allowed(allow: &[String], host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allow.iter().any(|pattern| {
        let pattern = pattern.trim().to_ascii_lowercase();
        match pattern.strip_prefix("*.") {
            Some(suffix) => host.ends_with(&format!(".{suffix}")),
            None => host == pattern,
        }
    })
}

/// One proxied connection (or refusal).
#[derive(Debug, Clone, Serialize)]
pub struct EgressEvent {
    pub ts: String,
    pub provider: String,
    pub host: String,
    pub port: u16,
    /// `allowed` | `unlisted` | `asked-allow` | `asked-deny` | `denied`.
    pub decision: &'static str,
    /// First connection from this provider to this host in this process.
    pub first_seen: bool,
    pub bytes_up: u64,
    pub bytes_down: u64,
    pub duration_ms: u64,
}

/// Host side of the proxy: warnings and `ask` decisions.
pub trait EgressObserver: Send + Sync {
    /// Called once per connection, when the decision is taken.
    fn on_egress(&self, event: &EgressEvent);

    /// `ask` mode: decide whether `provider` may reach `host`. Unanswered
    /// requests time out to "deny". The default denies at once.
    fn ask(&self, provider: &str, host: &str, respond: tokio::sync::oneshot::Sender<bool>) {
        let _ = (provider, host);
        let _ = respond.send(false);
    }
}

/// The egress proxy: one loopback listener per provider label.
pub struct EgressService {
    config: EgressConfig,
    observer: Arc<dyn EgressObserver>,
    listeners: Mutex<HashMap<String, SocketAddr>>,
    /// `(provider, host)` → decision taken for it (first-seen bookkeeping
    /// and the cached `ask` answer).
    decisions: Mutex<HashMap<(String, String), &'static str>>,
    log: NdjsonAppender,
}

static GLOBAL: OnceLock<Arc<EgressService>> = OnceLock::new();

/// Install the process-wide service. A second install is ignored.
pub fn install(service: Arc<EgressService>) {
    let _ = GLOBAL.set(service);
}

/// The process-wide service, when one is installed.
pub fn global() -> Option<Arc<EgressService>> {
    GLOBAL.get().cloned()
}

impl EgressService {
    pub fn new(config: EgressConfig, observer: Arc<dyn EgressObserver>) -> Arc<Self> {
        let log = NdjsonAppender::new(
            config.log_dir.join("connections.ndjson"),
            CONNECTIONS_LOG_MAX_BYTES,
        );
        Arc::new(Self {
            config,
            observer,
            listeners: Mutex::new(HashMap::new()),
            decisions: Mutex::new(HashMap::new()),
            log,
        })
    }

    pub fn config(&self) -> &EgressConfig {
        &self.config
    }

    /// The loopback proxy address for `provider`, binding it on first use.
    /// `None` when no tokio runtime is current or the bind fails — the
    /// caller then spawns without a proxy.
    pub fn listener_for(self: &Arc<Self>, provider: &str) -> Option<SocketAddr> {
        if let Some(addr) = self
            .listeners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(provider)
            .copied()
        {
            return Some(addr);
        }
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let std_listener = match std::net::TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                tracing::debug!(error = %e, provider, "egress proxy: bind failed");
                return None;
            }
        };
        if let Err(e) = std_listener.set_nonblocking(true) {
            tracing::debug!(error = %e, provider, "egress proxy: set_nonblocking failed");
            return None;
        }
        let addr = std_listener.local_addr().ok()?;
        let listener = {
            let _enter = handle.enter();
            match tokio::net::TcpListener::from_std(std_listener) {
                Ok(l) => l,
                Err(e) => {
                    tracing::debug!(error = %e, provider, "egress proxy: from_std failed");
                    return None;
                }
            }
        };
        {
            let mut map = self.listeners.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = map.get(provider) {
                // Another caller won the race; drop ours.
                return Some(*existing);
            }
            map.insert(provider.to_string(), addr);
        }
        handle.spawn(proxy::accept_loop(self.clone(), provider.to_string(), listener));
        Some(addr)
    }

    /// Decide `provider` → `host`. Returns the decision and whether this is
    /// the first time the pair was seen.
    async fn decide(&self, provider: &str, host: &str) -> (&'static str, bool) {
        let key = (provider.to_string(), host.to_ascii_lowercase());
        let cached = self
            .decisions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .copied();
        let decision = if host_allowed(&self.config.allow, host) {
            "allowed"
        } else {
            match self.config.mode {
                NetworkMode::Off | NetworkMode::Log => "unlisted",
                NetworkMode::Enforce => "denied",
                NetworkMode::Ask => match cached {
                    Some(d @ ("asked-allow" | "asked-deny")) => d,
                    _ => {
                        let (tx, rx) = tokio::sync::oneshot::channel();
                        self.observer.ask(provider, host, tx);
                        match tokio::time::timeout(self.config.ask_timeout, rx).await {
                            Ok(Ok(true)) => "asked-allow",
                            _ => "asked-deny",
                        }
                    }
                },
            }
        };
        let first_seen = {
            let mut map = self.decisions.lock().unwrap_or_else(|e| e.into_inner());
            let first = !map.contains_key(&key);
            map.insert(key, decision);
            first
        };
        (decision, first_seen)
    }

    /// Append `event` to `connections.ndjson` off the async runtime.
    fn record(self: &Arc<Self>, event: EgressEvent) {
        let svc = self.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = svc.log.append_json(&event) {
                tracing::debug!(error = %e, "egress proxy: log append failed");
            }
        });
    }
}

/// Where Claude writes raw API bodies for the post-turn scan
/// (`OTEL_LOG_RAW_API_BODIES=file:<dir>`). Inside the hard read floor.
pub fn claude_body_dir(root: &Path) -> PathBuf {
    root.join(".gaviero").join("egress").join("claude-bodies")
}

/// `agent.egress.captureClaudeBodies` from `<root>/.gaviero/settings.json`
/// (read directly, like the other spawn-time policy readers). Default true.
pub fn capture_claude_bodies(root: &Path) -> bool {
    std::fs::read_to_string(root.join(".gaviero").join("settings.json"))
        .ok()
        .and_then(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
        .and_then(|doc| {
            doc.pointer("/agent/egress/captureClaudeBodies")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(true)
}

/// Environment that routes a child process through the proxy at `addr`.
pub fn proxy_env(addr: SocketAddr) -> Vec<(&'static str, String)> {
    let url = format!("http://127.0.0.1:{}", addr.port());
    let no_proxy = "127.0.0.1,localhost,::1".to_string();
    vec![
        ("https_proxy", url.clone()),
        ("HTTPS_PROXY", url.clone()),
        ("http_proxy", url.clone()),
        ("HTTP_PROXY", url),
        ("no_proxy", no_proxy.clone()),
        ("NO_PROXY", no_proxy),
        ("NODE_USE_ENV_PROXY", "1".to_string()),
    ]
}

/// A reqwest proxy for gaviero's own client labelled `provider`, when the
/// service is installed.
pub fn reqwest_proxy(provider: &str) -> Option<reqwest::Proxy> {
    let addr = global()?.listener_for(provider)?;
    reqwest::Proxy::all(format!("http://127.0.0.1:{}", addr.port()))
        .ok()
        .map(|p| p.no_proxy(reqwest::NoProxy::from_string("127.0.0.1,localhost,::1")))
}

/// `reqwest::Client::builder()` routed through the egress proxy for
/// `provider` when the service is installed.
pub fn client_builder(provider: &str) -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    match reqwest_proxy(provider) {
        Some(proxy) => builder.proxy(proxy),
        None => builder,
    }
}

#[cfg(test)]
mod tests;
