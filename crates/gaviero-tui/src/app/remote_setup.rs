//! Remote sidecar bootstrap and the `/remote` command. gaviero-relay is the
//! only transport: settings resolution, the pinned private CA and its leaf,
//! the machine-scoped token lifecycle, a loopback-only listener with a
//! bounded port window, the outbound relay client, and QR rendering.
//!
//! Availability is fail-closed (§3.1): the sidecar refuses to claim it is
//! reachable unless the relay is configured and the pinned leaf loads,
//! covers the machine host, and is not expired. The bootstrap runs in a
//! spawned task and reports through `Event::RemoteStarted` /
//! `Event::RemoteUnavailable` (Plan C invariant 13). Secrets never reach
//! logs — only [`gaviero_remote::pairing::token_fingerprint`].

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use gaviero_core::workspace::{Workspace, identity, settings};
use gaviero_remote::pairing;
use gaviero_remote::server::{HubOutput, RemoteHandle, RemoteServerConfig, ServerError};

use crate::app::App;
use crate::app::remote::RemoteStatus;
use crate::event::Event;

/// Derived-port fallback window (Plan C §2.5): derived, derived+1 … +9.
/// An explicit `remote.port` never falls forward.
pub const PORT_WINDOW: u16 = 10;

/// `remote.tokenScope` (Plan C §0.1 item 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenScope {
    /// One token per machine, `~/.gaviero/remote/token` — the default.
    Machine,
    /// Plan A behaviour: `<workspace>/.gaviero/remote/token`.
    Workspace,
}

impl TokenScope {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "workspace" => Self::Workspace,
            _ => Self::Machine,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Machine => "machine",
            Self::Workspace => "workspace",
        }
    }
}

/// `remote.relay.*`: the sidecar stays on loopback and dials OUT to
/// `gaviero-relay`; phones reach it by SNI through the relay.
#[derive(Clone)]
pub struct RelayConfig {
    /// The relay's public name, e.g. `gaviero.example.com`.
    pub host: String,
    pub secret: String,
    /// `wss://<host>` unless `remote.relay.url` overrides it.
    pub base_url: String,
    /// Sanitised machine label (`remote.relay.machine` or the hostname).
    pub machine: String,
    /// This instance's route: `<wsid>-<machine>.<host>` — the URL host and
    /// TLS server name the phone uses. Never resolved in DNS.
    pub route: String,
}

impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayConfig")
            .field("host", &self.host)
            .field("secret", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("machine", &self.machine)
            .field("route", &self.route)
            .finish()
    }
}

/// A DNS label from a machine name: lowercase `[a-z0-9-]`, no leading or
/// trailing `-`, at most 40 characters (route = 16-hex wsid + `-` + this).
pub fn machine_label(raw: &str) -> String {
    let mapped: String = raw
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed: String = mapped.trim_matches('-').chars().take(40).collect();
    let trimmed = trimmed.trim_end_matches('-').to_string();
    if trimmed.is_empty() {
        "gaviero".to_string()
    } else {
        trimmed
    }
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_default()
}

/// Resolved remote configuration, or the reason the sidecar cannot run.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub enabled: bool,
    /// Configured or derived loopback port. Absent setting ⇒ derived; the
    /// fallback window applies only when `explicit_port` is false.
    pub port: u16,
    pub explicit_port: bool,
    /// `<machine>.<relay host>`: the QR `machine`, `hello.machine.host`, and
    /// the pinned leaf's primary name.
    pub machine_host: String,
    pub relay: RelayConfig,
    pub max_frame_bytes: u64,
    pub max_prompt_bytes: u64,
    pub command_rate_per_second: u32,
    /// Workspace remote state (`<root>/.gaviero/remote`).
    pub state_dir: PathBuf,
    /// Machine remote state (`~/.gaviero/remote`); `None` without a home.
    /// Tests override this so they never touch the real home directory.
    pub machine_state_dir: Option<PathBuf>,
    pub token_scope: TokenScope,
    pub workspace_id: String,
    pub workspace_display_name: String,
}

/// Why the sidecar is unavailable — every variant is actionable.
#[derive(Debug, Clone)]
pub enum RemoteUnavailable {
    /// `remote.enabled` is false.
    Disabled,
    NoWorkspaceRoot,
    /// `remote.port = 0` is a configuration error (§3.2): `0` means
    /// OS-assigned ephemeral, which would change on every restart.
    PortZero,
    /// `remote.relay.host` is empty: there is no way to reach this machine.
    RelayNotConfigured,
    /// `remote.relay.host` is set but `remote.relay.secret` is empty.
    RelaySecretMissing,
    CertLoad(String),
    CertHostMismatch {
        host: String,
    },
    CertExpired {
        not_after: String,
    },
    /// Creating the machine CA or issuing its leaf failed.
    CertProvision(String),
    Bind(String),
    /// Every port in the derived window was busy.
    NoFreePort {
        from: u16,
        to: u16,
    },
    /// Token read/create failure.
    Token(String),
}

impl RemoteUnavailable {
    /// Expected states that need no warning in the log: the user turned the
    /// sidecar off, or never configured a relay.
    pub fn is_quiet(&self) -> bool {
        matches!(self, Self::Disabled | Self::RelayNotConfigured)
    }
}

impl std::fmt::Display for RemoteUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(
                f,
                "remote is disabled — remove `remote.enabled: false` from .gaviero/settings.json"
            ),
            Self::NoWorkspaceRoot => write!(f, "no workspace root to anchor remote state"),
            Self::PortZero => write!(
                f,
                "`remote.port: 0` is not allowed (0 means an OS-assigned ephemeral port, \
                 which would change on every restart). Remove the setting to use the stable \
                 derived port, or set a fixed port."
            ),
            Self::RelayNotConfigured => write!(
                f,
                "no gaviero-relay configured — set `remote.relay.host` and `remote.relay.secret` \
                 in ~/.gaviero/settings.json (see docs/remote-relay.md)"
            ),
            Self::RelaySecretMissing => write!(
                f,
                "`remote.relay.host` is set but `remote.relay.secret` is empty — copy the \
                 relay's secret (gaviero-relay --secret-file) into ~/.gaviero/settings.json"
            ),
            Self::CertLoad(e) => write!(f, "could not load the TLS certificate/key: {e}"),
            Self::CertHostMismatch { host } => write!(
                f,
                "the pinned certificate does not cover {host} — restart gaviero to reissue it"
            ),
            Self::CertExpired { not_after } => write!(
                f,
                "the pinned certificate expired at {not_after} — restart gaviero to reissue it"
            ),
            Self::CertProvision(e) => write!(f, "could not create the TLS certificate: {e}"),
            Self::Bind(e) => write!(f, "could not bind the remote listener: {e}"),
            Self::NoFreePort { from, to } => write!(
                f,
                "every port from {from} to {to} is busy — set `remote.port` to a free port"
            ),
            Self::Token(e) => write!(f, "could not read or create the pairing token: {e}"),
        }
    }
}

// ── Settings resolution (§3, sync — no subprocess, no network) ──────

/// Read the `remote.*` settings for the workspace. `remote.port` absent or
/// null derives a stable port from the workspace identity (§3.3); `0` is
/// rejected rather than bound. Without `remote.relay.host` there is nothing
/// to start. Pure: computes paths, never creates files.
pub fn resolve_config(workspace: &Workspace) -> Result<RemoteConfig, RemoteUnavailable> {
    let Some(root) = workspace.roots().first().map(|p| p.to_path_buf()) else {
        return Err(RemoteUnavailable::NoWorkspaceRoot);
    };
    let scope = Some(root.as_path());
    let get = |key: &str| workspace.resolve_setting(key, scope);

    let enabled = get(settings::REMOTE_ENABLED).as_bool().unwrap_or(true);

    let (port, explicit_port) = match get(settings::REMOTE_PORT) {
        serde_json::Value::Null => (identity::derive_remote_port(&root), false),
        v => match v.as_u64() {
            Some(0) => return Err(RemoteUnavailable::PortZero),
            Some(p) if p <= u16::MAX as u64 => (p as u16, true),
            _ => (identity::derive_remote_port(&root), false),
        },
    };

    let state_dir = workspace
        .remote_state_dir()
        .ok_or(RemoteUnavailable::NoWorkspaceRoot)?;
    let machine_state_dir = gaviero_core::workspace::remote_machine_state_dir();

    let workspace_id = identity::workspace_id_hex16(&root);
    let str_setting = |key: &str| get(key).as_str().unwrap_or("").trim().to_string();
    let relay_host = str_setting(settings::REMOTE_RELAY_HOST)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if relay_host.is_empty() {
        return Err(if enabled {
            RemoteUnavailable::RelayNotConfigured
        } else {
            RemoteUnavailable::Disabled
        });
    }
    let secret = str_setting(settings::REMOTE_RELAY_SECRET);
    if secret.is_empty() {
        return Err(RemoteUnavailable::RelaySecretMissing);
    }
    let mut machine_setting = str_setting(settings::REMOTE_RELAY_MACHINE);
    if machine_setting.is_empty() {
        machine_setting = hostname();
    }
    let machine = machine_label(&machine_setting);
    let url = str_setting(settings::REMOTE_RELAY_URL);
    // The machine identity (QR `machine`, hello, relay notifications) is a
    // name under the relay host; the route adds the workspace.
    let machine_host = format!("{machine}.{relay_host}");
    let relay = RelayConfig {
        base_url: if url.is_empty() {
            format!("wss://{relay_host}")
        } else {
            url.trim_end_matches('/').to_string()
        },
        route: format!("{workspace_id}-{machine}.{relay_host}"),
        host: relay_host,
        secret,
        machine,
    };

    Ok(RemoteConfig {
        enabled,
        port,
        explicit_port,
        machine_host,
        relay,
        max_frame_bytes: get(settings::REMOTE_MAX_FRAME_BYTES)
            .as_u64()
            .unwrap_or(262_144),
        max_prompt_bytes: get(settings::REMOTE_MAX_PROMPT_BYTES)
            .as_u64()
            .unwrap_or(131_072),
        command_rate_per_second: get(settings::REMOTE_COMMAND_RATE_PER_SECOND)
            .as_u64()
            .unwrap_or(10) as u32,
        state_dir,
        machine_state_dir,
        token_scope: TokenScope::parse(get(settings::REMOTE_TOKEN_SCOPE).as_str().unwrap_or("")),
        workspace_id,
        workspace_display_name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "workspace".to_string()),
    })
}

// ── Pinned private CA (§2.2 step 3) ─────────────────────────────────

/// `~/.gaviero/remote/tls/pinned` (workspace state dir without a home).
fn pinned_dir(config: &RemoteConfig) -> PathBuf {
    config
        .machine_state_dir
        .clone()
        .unwrap_or_else(|| config.state_dir.clone())
        .join("tls")
        .join(pairing::PINNED_DIR)
}

/// The leaf bundle: one file holds the certificate and its key.
pub fn leaf_path(config: &RemoteConfig) -> PathBuf {
    pinned_dir(config).join(pairing::LEAF_FILE)
}

/// Create the machine CA once and (re)issue its leaf for the machine host
/// plus `*.<relay host>`, which covers every route on this machine. Returns
/// whether anything was issued. Only ever awaited from the bootstrap task.
pub async fn ensure_certificate(config: &RemoteConfig) -> Result<bool, RemoteUnavailable> {
    // Key generation and file writes: keep them off the async workers.
    let dir = pinned_dir(config);
    let host = config.machine_host.clone();
    let extra = vec![format!("*.{}", config.relay.host)];
    let pinned = tokio::task::spawn_blocking(move || {
        pairing::ensure_pinned_ca_with(&dir, &host, &extra)
    })
    .await
    .map_err(|e| RemoteUnavailable::CertProvision(e.to_string()))?
    .map_err(RemoteUnavailable::CertProvision)?;
    if pinned.issued {
        tracing::info!(host = %config.machine_host, "issued pinned-CA TLS material");
    }
    Ok(pinned.issued)
}

// ── Token lifecycle (§3.4, Plan C §2.2 step 4) ──────────────────────

/// The token in force, where it lives, and which scope won.
#[derive(Debug, Clone)]
pub struct LoadedToken {
    pub token: String,
    pub scope: TokenScope,
    pub path: PathBuf,
}

/// Candidate locations in preference order. Machine scope falls back to
/// the workspace file when the machine root is unusable (invariant 17).
fn token_paths(config: &RemoteConfig) -> Vec<(TokenScope, PathBuf)> {
    let mut out = Vec::new();
    if config.token_scope == TokenScope::Machine
        && let Some(dir) = &config.machine_state_dir
    {
        out.push((TokenScope::Machine, dir.join("token")));
    }
    out.push((TokenScope::Workspace, config.state_dir.join("token")));
    out
}

/// Read the stored token, generating and persisting one on first use.
/// Owner-only permissions where supported; on Windows the file inherits
/// the directory ACL and a broadly-writable directory is warned about.
pub fn load_or_create_token(config: &RemoteConfig) -> Result<LoadedToken, String> {
    let mut last_err = None;
    for (scope, path) in token_paths(config) {
        match load_or_create_at(&path) {
            Ok(token) => return Ok(LoadedToken { token, scope, path }),
            Err(e) => {
                if scope == TokenScope::Machine {
                    tracing::warn!(
                        path = %path.display(),
                        "machine token unusable ({e}) — falling back to a workspace-scoped token"
                    );
                }
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| "no token location available".to_string()))
}

fn load_or_create_at(path: &std::path::Path) -> Result<String, String> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let token = pairing::generate_token();
    write_token_at(path, &token)?;
    Ok(token)
}

fn write_token_at(path: &std::path::Path, token: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| "token path has no parent".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    // Atomic replace so a concurrent reader (another instance's hub poll)
    // never sees a partial token.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token).map_err(|e| format!("writing token: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("replacing token: {e}"))?;
    #[cfg(windows)]
    {
        // Best-effort ACL note: Windows inherits the parent ACL. Warn when
        // the directory is writable by non-owners so the user can tighten it.
        if let Ok(meta) = std::fs::metadata(dir)
            && !meta.permissions().readonly()
        {
            tracing::info!(
                dir = %dir.display(),
                "remote token stored with inherited directory ACL — restrict this folder if the machine is shared"
            );
        }
    }
    Ok(())
}

/// Desktop-only rotation (§3.4): atomically replace the token file in the
/// scope in force. The caller closes its own live socket with 4006 via
/// `HubInput::TokenRotated`; every other instance sharing the machine
/// token notices the file change within its poll interval (invariant 15).
pub fn rotate_token(config: &RemoteConfig) -> Result<LoadedToken, String> {
    let token = pairing::generate_token();
    let mut last_err = None;
    for (scope, path) in token_paths(config) {
        match write_token_at(&path, &token) {
            Ok(()) => return Ok(LoadedToken { token, scope, path }),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| "no token location available".to_string()))
}

// ── Availability (§3.1) ─────────────────────────────────────────────

#[derive(Debug)]
pub struct Availability {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub cert: pairing::CertInfo,
    /// The machine CA the QR carries as `tls_ca` (base64 DER).
    pub tls_ca: String,
}

/// Fail-closed availability check. Every failure names the fix. Sync —
/// safe to call from `/remote` inside the event loop.
pub fn check_availability(config: &RemoteConfig) -> Result<Availability, RemoteUnavailable> {
    let leaf = leaf_path(config);
    let cert_pem = std::fs::read(&leaf).map_err(|e| {
        RemoteUnavailable::CertLoad(format!(
            "{}: {e} — restart gaviero to create it",
            leaf.display()
        ))
    })?;
    let cert = pairing::inspect_cert(&cert_pem, &config.machine_host)
        .map_err(RemoteUnavailable::CertLoad)?;
    if !cert.covers_host {
        return Err(RemoteUnavailable::CertHostMismatch {
            host: config.machine_host.clone(),
        });
    }
    if cert.is_expired() {
        return Err(RemoteUnavailable::CertExpired {
            not_after: cert.not_after.clone(),
        });
    }
    let tls_ca = pairing::pinned_ca_b64(&pinned_dir(config)).ok_or_else(|| {
        RemoteUnavailable::CertLoad(format!(
            "{}: no usable machine CA — restart gaviero to create it",
            pinned_dir(config).display()
        ))
    })?;
    Ok(Availability {
        key_pem: cert_pem.clone(),
        cert_pem,
        cert,
        tls_ca,
    })
}

/// Loopback only (`127.0.0.1`, then `::1`): the relay client is the way in,
/// so nothing else on the network can reach the listener.
pub fn bind_addrs(port: u16) -> Vec<SocketAddr> {
    vec![
        SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port),
        SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port),
    ]
}

/// `wss://<route>/v1/ws`. No port: the phone dials the relay on 443 and
/// names the route in SNI.
pub fn pairing_url(config: &RemoteConfig) -> String {
    format!("wss://{}{}", config.relay.route, gaviero_remote::WS_PATH)
}

/// `https://<route>/v1/instances` — every instance listener serves the
/// machine's instance list, so this instance's route lists them all.
pub fn directory_url(config: &RemoteConfig) -> String {
    format!(
        "https://{}{}",
        config.relay.route,
        gaviero_remote::INSTANCES_PATH
    )
}

/// The QR payload `/remote` renders: the 1.0 keys, the 1.1 identity keys,
/// the machine CA the app pins (`tls_ca`), and the relay fields.
pub fn qr_payload_for(config: &RemoteConfig, token: &str, tls_ca: &str) -> String {
    let url = pairing_url(config);
    let directory_url = directory_url(config);
    pairing::qr_payload_json(&pairing::QrPayloadInput {
        url: &url,
        token,
        workspace: &config.workspace_display_name,
        workspace_id: Some(&config.workspace_id),
        machine: Some(&config.machine_host),
        directory_url: Some(&directory_url),
        tls_ca: Some(tls_ca),
        relay_host: Some(&config.relay.host),
        relay_secret: Some(&config.relay.secret),
    })
}

// ── Server startup ──────────────────────────────────────────────────

/// Everything the bootstrap learned, delivered to the event loop as
/// `Event::RemoteStarted` and rendered by `/remote`.
#[derive(Clone)]
pub struct RemoteStarted {
    pub handle: RemoteHandle,
    /// The loopback port actually bound (may differ from the derived one).
    pub port: u16,
    pub host: String,
    pub cert_not_after: String,
    pub token_scope: TokenScope,
    pub token_fingerprint: String,
    pub max_prompt_bytes: usize,
    /// The outbound relay client (status + notifications). Set by the
    /// bootstrap right after the listener binds.
    pub relay: Option<gaviero_remote::relay_client::RelayClient>,
}

impl std::fmt::Debug for RemoteStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteStarted")
            .field("port", &self.port)
            .field("host", &self.host)
            .field("cert_not_after", &self.cert_not_after)
            .field("token_scope", &self.token_scope)
            .field("token_fingerprint", &self.token_fingerprint)
            .finish_non_exhaustive()
    }
}

/// Start the sidecar and bridge its outputs into the TUI event channel.
/// Tries the derived port window (§2.5) unless the port is explicit.
pub async fn start(
    config: &RemoteConfig,
    availability: Availability,
    token: LoadedToken,
    instance_id: String,
    event_tx: tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<RemoteStarted, RemoteUnavailable> {
    let (ping_interval, idle_timeout, hello_timeout) = RemoteServerConfig::timing_defaults();
    let candidates: Vec<u16> = if config.explicit_port {
        vec![config.port]
    } else {
        (0..PORT_WINDOW)
            .filter_map(|i| config.port.checked_add(i))
            .collect()
    };
    let last = *candidates.last().expect("at least one candidate");

    let mut last_bind_error = String::new();
    for port in candidates {
        let mut addrs = bind_addrs(port);
        let primary = addrs.remove(0);
        let registry = config.machine_state_dir.as_ref().map(|machine| {
            gaviero_remote::server::RegistryConfig {
                dir: machine.join("instances"),
                entry: gaviero_remote::dto::InstanceInfo {
                    instance_id: instance_id.clone(),
                    workspace: gaviero_remote::dto::WorkspaceInfo {
                        id: config.workspace_id.clone(),
                        display_name: config.workspace_display_name.clone(),
                    },
                    url: pairing_url(config),
                    port,
                    tui_version: env!("CARGO_PKG_VERSION").to_string(),
                    started_at: gaviero_remote::server::registry::utc_now_rfc3339(),
                    client_connected: false,
                },
            }
        });
        let spawned = gaviero_remote::server::spawn(RemoteServerConfig {
            bind_addr: primary,
            extra_bind_addrs: addrs,
            tls_cert_pem: availability.cert_pem.clone(),
            tls_key_pem: availability.key_pem.clone(),
            token: token.token.clone(),
            instance_id: instance_id.clone(),
            tui_version: env!("CARGO_PKG_VERSION").to_string(),
            workspace: gaviero_remote::dto::WorkspaceInfo {
                id: config.workspace_id.clone(),
                display_name: config.workspace_display_name.clone(),
            },
            capabilities: vec![
                "shell_sessions".to_string(),
                gaviero_remote::version::capability::LATEST_PAGE.to_string(),
                gaviero_remote::version::capability::INSTANCES.to_string(),
                gaviero_remote::version::capability::FILE_COMPLETIONS.to_string(),
                gaviero_remote::version::capability::FILE_BROWSER.to_string(),
                gaviero_remote::version::capability::TURN_REVIEW.to_string(),
                gaviero_remote::version::capability::PERMISSION_TURN_GRANT.to_string(),
            ],
            machine: Some(gaviero_remote::dto::MachineInfo {
                host: config.machine_host.clone(),
                directory_url: Some(directory_url(config)),
            }),
            token_path: Some(token.path.clone()),
            token_poll_interval: RemoteServerConfig::TOKEN_POLL_INTERVAL,
            registry,
            heartbeat_interval: gaviero_remote::server::registry::HEARTBEAT_INTERVAL,
            stale_after: gaviero_remote::server::registry::STALE_AFTER,
            confirm_required: crate::app::remote::REMOTE_CONFIRM_REQUIRED
                .iter()
                .map(|s| s.to_string())
                .collect(),
            allowed_slash_commands: crate::app::remote::REMOTE_ALLOWED_SLASH
                .iter()
                .map(|s| s.to_string())
                .collect(),
            limits: gaviero_remote::dto::Limits {
                max_frame_bytes: config.max_frame_bytes,
                max_prompt_bytes: config.max_prompt_bytes,
                command_rate_per_second: config.command_rate_per_second,
            },
            ping_interval,
            idle_timeout,
            hello_timeout,
        })
        .await;

        let spawned = match spawned {
            Ok(s) => s,
            Err(ServerError::Bind) => {
                last_bind_error = format!("{primary} is busy");
                if !config.explicit_port {
                    tracing::info!(port, "remote port busy — trying the next one in the window");
                }
                continue;
            }
            Err(e @ ServerError::Tls(_)) => return Err(RemoteUnavailable::Bind(e.to_string())),
        };

        // Bridge hub → event channel. No background task mutates App.
        let mut outputs = spawned.outputs;
        tokio::spawn(async move {
            while let Some(output) = outputs.recv().await {
                let event = match output {
                    HubOutput::Command(envelope) => Event::RemoteCommand(envelope),
                    HubOutput::ClientConnected => Event::RemoteClientConnected,
                    HubOutput::ClientDisconnected => Event::RemoteClientDisconnected,
                    HubOutput::SnapshotNeeded => Event::RemoteSnapshotNeeded,
                };
                if event_tx.send(event).is_err() {
                    break;
                }
            }
        });
        return Ok(RemoteStarted {
            handle: spawned.handle,
            port: spawned.local_addr.port(),
            host: config.machine_host.clone(),
            cert_not_after: availability.cert.not_after.clone(),
            token_scope: token.scope,
            token_fingerprint: pairing::token_fingerprint(&token.token),
            max_prompt_bytes: config.max_prompt_bytes as usize,
            relay: None,
        });
    }
    if config.explicit_port {
        Err(RemoteUnavailable::Bind(last_bind_error))
    } else {
        Err(RemoteUnavailable::NoFreePort {
            from: config.port,
            to: last,
        })
    }
}

// ── Background bootstrap (Plan C §2.2, invariant 13) ────────────────

/// Resolve → ensure certificate → token → bind → dial the relay, then
/// report through the event channel. Spawned from `main`; never awaited by
/// the event loop.
pub async fn bootstrap(workspace: Workspace, event_tx: tokio::sync::mpsc::UnboundedSender<Event>) {
    let event = match bootstrap_inner(&workspace, event_tx.clone()).await {
        Ok(started) => Event::RemoteStarted(Box::new(started)),
        Err(e) => Event::RemoteUnavailable(e),
    };
    let _ = event_tx.send(event);
}

async fn bootstrap_inner(
    workspace: &Workspace,
    event_tx: tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<RemoteStarted, RemoteUnavailable> {
    let config = resolve_config(workspace)?;
    if !config.enabled {
        return Err(RemoteUnavailable::Disabled);
    }
    ensure_certificate(&config).await?;
    let availability = check_availability(&config)?;
    let token = load_or_create_token(&config).map_err(RemoteUnavailable::Token)?;
    let instance_id = pairing::generate_token()[..16].to_string();
    let mut started = start(&config, availability, token, instance_id, event_tx).await?;
    started.relay = Some(gaviero_remote::relay_client::RelayClient::spawn(
        gaviero_remote::relay_client::RelayClientConfig {
            base_url: config.relay.base_url.clone(),
            secret: config.relay.secret.clone(),
            route: gaviero_remote::relay_client::RelayRoute {
                route: config.relay.route.clone(),
                machine: config.machine_host.clone(),
                workspace_id: config.workspace_id.clone(),
                display_name: config.workspace_display_name.clone(),
            },
            local_addr: SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), started.port),
        },
    ));
    Ok(started)
}

// ── QR rendering ────────────────────────────────────────────────────

/// QR quiet zone, in modules (spec minimum is 4).
const QUIET_ZONE: usize = 4;

/// Widest rendered QR (quiet zone included) that still fits a terminal
/// side panel without wrapping.
const MAX_QR_COLUMNS: usize = 120;

/// Encode at error-correction level M; fall back to L when M would not fit
/// [`MAX_QR_COLUMNS`] (the pinned-CA payload carries ~550 more bytes). A
/// crisp on-screen code scans reliably at L.
fn encode_qr(payload: &str) -> Result<qrcode::QrCode, String> {
    let medium = qrcode::QrCode::new(payload.as_bytes())
        .map_err(|e| format!("could not encode the pairing QR: {e}"))?;
    if medium.width() + QUIET_ZONE * 2 <= MAX_QR_COLUMNS {
        return Ok(medium);
    }
    qrcode::QrCode::with_error_correction_level(payload.as_bytes(), qrcode::EcLevel::L)
        .map_err(|e| format!("could not encode the pairing QR: {e}"))
}

/// Render the pairing payload as a half-block QR sized for a terminal.
/// Two vertical modules per character row keeps it scannable in a side
/// panel; a quiet zone is included because scanners need it.
pub fn render_qr(payload: &str) -> Result<String, String> {
    let code = encode_qr(payload)?;
    let width = code.width();
    // The spec requires a 4-module quiet zone; phone scanners are much
    // less reliable without it, and this code has exactly one job.
    let quiet = QUIET_ZONE;
    let side = width + quiet * 2;
    let dark = |x: usize, y: usize| -> bool {
        if x < quiet || y < quiet || x >= quiet + width || y >= quiet + width {
            return false;
        }
        code[(x - quiet, y - quiet)] == qrcode::Color::Dark
    };
    let mut out = String::new();
    let mut y = 0usize;
    while y < side {
        for x in 0..side {
            let top = dark(x, y);
            let bottom = if y + 1 < side { dark(x, y + 1) } else { false };
            // Terminal foreground is light-on-dark: a "dark" QR module is
            // drawn as an unlit cell, so invert against the block glyphs.
            out.push(match (top, bottom) {
                (true, true) => ' ',
                (true, false) => '▄',
                (false, true) => '▀',
                (false, false) => '█',
            });
        }
        out.push('\n');
        y += 2;
    }
    Ok(out)
}

// ── `/remote` command ───────────────────────────────────────────────

/// `/remote [status|rotate|hide]`. Rendered into the chat transcript as a
/// system message — the QR is the one intentional token display (§3.4).
/// Runs inside the event loop: file reads only.
pub fn handle_remote_command(app: &mut App, line: &str) {
    let arg = line
        .trim()
        .strip_prefix("/remote")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let config = match resolve_config(&app.workspace) {
        Ok(c) => c,
        Err(e) => {
            app.chat_state.add_system_message(&format!("Remote: {e}"));
            return;
        }
    };

    match arg.as_str() {
        "hide" => {
            // Drop every rendered QR/token from the visible transcript.
            let idx = app.chat_state.active_conv;
            let conv = &mut app.chat_state.conversations[idx];
            for msg in &mut conv.messages {
                if msg.content.contains("gaviero-remote pairing") {
                    msg.content = "[pairing QR hidden — run /remote to show it again]".to_string();
                }
            }
            app.chat_state
                .add_system_message("Remote: pairing QR cleared from this transcript.");
        }
        "rotate" => match rotate_token(&config) {
            Ok(loaded) => {
                if let Some(handle) = app.remote.handle.as_ref() {
                    // Desktop-only rotation closes the live client with 4006.
                    let _ = handle.try_send(gaviero_remote::server::HubInput::TokenRotated {
                        new_token: loaded.token.clone(),
                    });
                }
                let others = if loaded.scope == TokenScope::Machine {
                    " Every other gaviero on this machine rotates within a few seconds."
                } else {
                    ""
                };
                app.chat_state.add_system_message(&format!(
                    "Remote: {} token rotated ({}). Any paired device must scan again — \
                     run /remote for the new QR.{others}",
                    loaded.scope.as_str(),
                    pairing::token_fingerprint(&loaded.token)
                ));
            }
            Err(e) => {
                app.chat_state
                    .add_system_message(&format!("Remote: rotation failed — {e}"));
            }
        },
        // Bare `/remote` and `/remote status` both report; the QR is only
        // rendered when the sidecar is actually reachable.
        _ => {
            let report = status_report(app, &config);
            app.chat_state.add_system_message(&report);
        }
    }
}

fn status_report(app: &App, config: &RemoteConfig) -> String {
    let mut report = String::new();
    if !config.enabled {
        report.push_str(
            "Remote: disabled by `remote.enabled: false` in .gaviero/settings.json — remove it \
             and restart gaviero.\n",
        );
    }
    let running = match &app.remote.status {
        RemoteStatus::Running(s) => Some(s.as_ref()),
        _ => None,
    };
    let port = running.map(|s| s.port).unwrap_or(config.port);
    report.push_str(&format!(
        "Workspace: {} ({})\nMachine: {}\nLocal port: {} (loopback only{})\n",
        config.workspace_display_name,
        config.workspace_id,
        config.machine_host,
        port,
        if config.explicit_port {
            ", fixed by remote.port".to_string()
        } else if port != config.port {
            format!(", derived port {} was busy", config.port)
        } else {
            String::new()
        }
    ));
    let relay_state = match running.and_then(|s| s.relay.as_ref()) {
        Some(client) => client.status().to_string(),
        None => "not started".to_string(),
    };
    report.push_str(&format!(
        "Relay: {} — route {} — {relay_state}\n",
        config.relay.base_url, config.relay.route
    ));

    let availability = check_availability(config);
    match &availability {
        Ok(availability) => {
            let expiry = if availability.cert.is_near_expiry() {
                format!(
                    "{} (EXPIRES SOON — reissued automatically on the next start)",
                    availability.cert.not_after
                )
            } else {
                availability.cert.not_after.clone()
            };
            report.push_str(&format!(
                "Certificate: pinned private CA (~/.gaviero/remote/tls/pinned) — valid until \
                 {expiry}\n"
            ));
        }
        Err(e) => report.push_str(&format!("Certificate: unavailable — {e}\n")),
    }

    let token = load_or_create_token(config);
    match &token {
        Ok(loaded) => report.push_str(&format!(
            "Token: {}-scoped {}\n",
            loaded.scope.as_str(),
            pairing::token_fingerprint(&loaded.token)
        )),
        Err(e) => report.push_str(&format!("Token: {e}\n")),
    }

    if let Some(machine) = &config.machine_state_dir {
        let listed = gaviero_remote::server::registry::read_directory(
            &machine.join("instances"),
            &config.machine_host,
            gaviero_remote::server::registry::STALE_AFTER,
        );
        let others: Vec<_> = listed
            .instances
            .iter()
            .filter(|i| i.workspace.id != config.workspace_id)
            .collect();
        if others.is_empty() {
            report.push_str("Other instances: none\n");
        } else {
            report.push_str("Other instances:\n");
            for inst in others {
                let flag = if inst.client_connected {
                    "in use"
                } else {
                    "listening"
                };
                report.push_str(&format!(
                    "  {} ({}) :{} {flag}\n",
                    inst.workspace.display_name, inst.workspace.id, inst.port
                ));
            }
        }
    }

    report.push_str(&format!(
        "Status: {}\n",
        match &app.remote.status {
            RemoteStatus::Pending => "starting (certificate check in progress)".to_string(),
            RemoteStatus::Unavailable(e) => format!("unavailable — {e}"),
            RemoteStatus::Running(_) if app.remote.client_connected =>
                "client connected".to_string(),
            RemoteStatus::Running(_) => "listening, no client connected".to_string(),
        }
    ));

    // "Instance offline" in the app means the relay route isn't up.
    if running.is_some() && !app.remote.client_connected {
        report.push_str(
            "\nIf the app says \"instance offline\", check the Relay line above: it must say \
             \"registered\". A 401 means remote.relay.secret differs from the relay's \
             --secret-file; \"timed out\"/\"refused\" means the relay or its nginx is down.\n",
        );
    }

    if let (Some(_), Ok(loaded), Ok(availability)) = (running, &token, &availability) {
        let url = pairing_url(config);
        let payload = qr_payload_for(config, &loaded.token, &availability.tls_ca);
        match render_qr(&payload) {
            Ok(qr) => report.push_str(&format!(
                "\nScan this gaviero-remote pairing code with the Gaviero Remote app \
                 (one scan pairs every workspace on this machine):\n\n{qr}\n{url}\n\
                 (Token shown only in this code. /remote hide clears it; \
                 /remote rotate invalidates it.)"
            )),
            Err(e) => report.push_str(&format!("\nQR: {e}")),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace_with(settings: serde_json::Value) -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(
            dir.path().join(".gaviero/settings.json"),
            serde_json::to_string(&settings).unwrap(),
        )
        .unwrap();
        let ws = Workspace::single_folder(dir.path().to_path_buf());
        (dir, ws)
    }

    /// A workspace with a configured relay plus `extra` `remote.*` keys.
    fn relay_workspace(extra: serde_json::Value) -> (tempfile::TempDir, Workspace) {
        let mut remote = serde_json::json!({
            "relay": {
                "host": "Gaviero.Example.com.",
                "secret": "relay-secret-0123456789abcdef",
                "machine": "Desk Top"
            }
        });
        if let Some(extra) = extra.as_object() {
            for (k, v) in extra {
                remote[k] = v.clone();
            }
        }
        workspace_with(serde_json::json!({ "remote": remote }))
    }

    /// Tests must never touch the real `~/.gaviero/remote`.
    fn isolated(ws: &Workspace, machine: &tempfile::TempDir) -> RemoteConfig {
        let mut config = resolve_config(ws).unwrap();
        config.machine_state_dir = Some(machine.path().join("remote"));
        config
    }

    /// Explicit empty relay keys: workspace settings win over the user's
    /// `~/.gaviero/settings.json`, which may configure a real relay.
    #[test]
    fn without_a_relay_there_is_nothing_to_start_and_it_says_so_quietly() {
        let (_dir, ws) =
            workspace_with(serde_json::json!({ "remote": { "relay": { "host": "" } } }));
        let err = resolve_config(&ws).expect_err("no relay, no sidecar");
        assert!(matches!(err, RemoteUnavailable::RelayNotConfigured));
        assert!(err.is_quiet(), "an unconfigured relay is not a warning");
        assert!(err.to_string().contains("remote.relay.host"), "{err}");

        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "enabled": false, "relay": { "host": "" } }
        }));
        let err = resolve_config(&ws).expect_err("disabled");
        assert!(matches!(err, RemoteUnavailable::Disabled));
    }

    #[test]
    fn remote_is_enabled_by_default_once_a_relay_is_set() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let config = resolve_config(&ws).unwrap();
        assert!(config.enabled);
        assert_eq!(config.token_scope, TokenScope::Machine);
    }

    #[test]
    fn port_zero_is_rejected_with_an_actionable_message() {
        let (_dir, ws) = relay_workspace(serde_json::json!({ "port": 0 }));
        let err = resolve_config(&ws).expect_err("port 0 must be refused");
        assert!(matches!(err, RemoteUnavailable::PortZero));
        let msg = err.to_string();
        assert!(msg.contains("ephemeral"), "{msg}");
    }

    #[test]
    fn absent_port_derives_a_stable_workspace_port() {
        let (dir, ws) = relay_workspace(serde_json::json!({}));
        let a = resolve_config(&ws).unwrap();
        let b = resolve_config(&ws).unwrap();
        assert_eq!(a.port, b.port, "derived port is identical across restarts");
        assert_eq!(a.port, identity::derive_remote_port(dir.path()));
        assert!((49152..=65535).contains(&a.port));
        assert!(!a.explicit_port);
    }

    #[test]
    fn explicit_port_overrides_the_derivation() {
        let (_dir, ws) = relay_workspace(serde_json::json!({ "port": 51234 }));
        let config = resolve_config(&ws).unwrap();
        assert_eq!(config.port, 51234);
        assert!(config.explicit_port, "an explicit port never falls forward");
    }

    #[test]
    fn machine_token_is_created_once_reread_stably_and_rotated() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        let first = load_or_create_token(&config).unwrap();
        assert_eq!(first.scope, TokenScope::Machine);
        assert_eq!(first.path, machine.path().join("remote").join("token"));
        let second = load_or_create_token(&config).unwrap();
        assert_eq!(first.token, second.token, "token persists across reads");
        assert_eq!(first.token.len(), 64);
        let rotated = rotate_token(&config).unwrap();
        assert_ne!(rotated.token, first.token, "rotation replaces the token");
        assert_eq!(rotated.scope, TokenScope::Machine);
        assert_eq!(load_or_create_token(&config).unwrap().token, rotated.token);
        assert!(
            !config.state_dir.join("token").exists(),
            "machine scope never writes the workspace file"
        );
    }

    #[test]
    fn two_workspaces_share_the_machine_token() {
        let (_a, ws_a) = relay_workspace(serde_json::json!({}));
        let (_b, ws_b) = relay_workspace(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        let a = load_or_create_token(&isolated(&ws_a, &machine)).unwrap();
        let b = load_or_create_token(&isolated(&ws_b, &machine)).unwrap();
        assert_eq!(a.token, b.token, "one pairing per machine");
    }

    #[test]
    fn workspace_scope_opts_out_of_the_machine_token() {
        let (_dir, ws) = relay_workspace(serde_json::json!({ "tokenScope": "workspace" }));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        let loaded = load_or_create_token(&config).unwrap();
        assert_eq!(loaded.scope, TokenScope::Workspace);
        assert_eq!(loaded.path, config.state_dir.join("token"));
        assert!(!machine.path().join("remote/token").exists());
    }

    #[test]
    fn unwritable_machine_root_falls_back_to_the_workspace_token() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        // A regular file where the directory should be: create_dir_all fails.
        let blocker = machine.path().join("blocker");
        std::fs::write(&blocker, "x").unwrap();
        let mut config = resolve_config(&ws).unwrap();
        config.machine_state_dir = Some(blocker.join("remote"));
        let loaded = load_or_create_token(&config).expect("fallback must succeed");
        assert_eq!(loaded.scope, TokenScope::Workspace);
        assert_eq!(loaded.path, config.state_dir.join("token"));
    }

    #[test]
    fn token_scope_parses_leniently() {
        assert_eq!(TokenScope::parse("workspace"), TokenScope::Workspace);
        assert_eq!(TokenScope::parse(" Workspace "), TokenScope::Workspace);
        assert_eq!(TokenScope::parse("machine"), TokenScope::Machine);
        assert_eq!(TokenScope::parse(""), TokenScope::Machine);
        assert_eq!(TokenScope::parse("nonsense"), TokenScope::Machine);
    }

    #[test]
    fn the_listener_binds_loopback_only_never_a_wildcard() {
        let addrs = bind_addrs(50101);
        assert_eq!(addrs.len(), 2);
        assert!(addrs.iter().all(|a| a.ip().is_loopback()), "{addrs:?}");
        assert!(addrs.iter().all(|a| a.port() == 50101));
    }

    #[test]
    fn machine_labels_are_dns_safe() {
        assert_eq!(machine_label("DESKTOP-ABC123"), "desktop-abc123");
        assert_eq!(machine_label(" my_box.local "), "my-box-local");
        assert_eq!(machine_label("---"), "gaviero");
        assert_eq!(machine_label(&"x".repeat(80)).len(), 40);
        assert!(!machine_label("a-very-long-name-that-ends-exactly-at-forty-x-y").ends_with('-'));
    }

    #[test]
    fn relay_settings_derive_route_urls() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let config = resolve_config(&ws).unwrap();
        let relay = &config.relay;
        assert_eq!(relay.host, "gaviero.example.com");
        assert_eq!(relay.machine, "desk-top");
        assert_eq!(relay.base_url, "wss://gaviero.example.com");
        assert_eq!(
            relay.route,
            format!("{}-desk-top.gaviero.example.com", config.workspace_id)
        );
        assert_eq!(config.machine_host, "desk-top.gaviero.example.com");
        assert_eq!(
            pairing_url(&config),
            format!("wss://{}/v1/ws", relay.route),
            "no port: the phone dials the relay on 443"
        );
        assert_eq!(
            directory_url(&config),
            format!("https://{}/v1/instances", relay.route)
        );
        assert!(!format!("{relay:?}").contains("relay-secret"), "Debug redacts the secret");
    }

    #[test]
    fn a_relay_without_a_secret_is_an_actionable_error() {
        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "relay": { "host": "gaviero.example.com", "secret": "" } }
        }));
        let err = resolve_config(&ws).expect_err("secret required");
        assert!(matches!(err, RemoteUnavailable::RelaySecretMissing));
        assert!(!err.is_quiet(), "a half-configured relay is worth a warning");
        assert!(err.to_string().contains("remote.relay.secret"));
    }

    /// Without the pinned material on disk, availability reports an
    /// actionable CertLoad naming the path.
    #[test]
    fn missing_pinned_material_fails_closed() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        match check_availability(&config) {
            Err(RemoteUnavailable::CertLoad(msg)) => assert!(msg.contains("pinned"), "{msg}"),
            other => panic!("expected CertLoad, got {other:?}"),
        }
    }

    /// The pinned leaf is issued once, covers the machine host and every
    /// route, and the QR carries the CA (never a key) plus the relay fields.
    #[tokio::test]
    async fn the_leaf_is_issued_once_and_the_qr_carries_the_ca_and_relay_fields() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        assert!(leaf_path(&config).starts_with(machine.path()), "machine-scoped");
        assert!(ensure_certificate(&config).await.unwrap(), "first start issues");
        assert!(
            !ensure_certificate(&config).await.unwrap(),
            "a second start re-issues nothing"
        );
        let availability = check_availability(&config).expect("available");
        assert!(availability.cert.covers_host);
        let leaf = std::fs::read(leaf_path(&config)).unwrap();
        assert!(
            pairing::inspect_cert(&leaf, &config.relay.route)
                .unwrap()
                .covers_host,
            "the *.relay-host SAN covers the route"
        );

        let token = load_or_create_token(&config).unwrap();
        let payload = qr_payload_for(&config, &token.token, &availability.tls_ca);
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["kind"], "gaviero-remote");
        assert_eq!(v["protocol_major"], 1);
        assert_eq!(v["token"], token.token);
        assert_eq!(v["workspace_id"], config.workspace_id);
        assert_eq!(v["relay_host"], "gaviero.example.com");
        assert_eq!(v["relay_secret"], "relay-secret-0123456789abcdef");
        assert_eq!(v["machine"], "desk-top.gaviero.example.com");
        assert_eq!(v["url"], format!("wss://{}/v1/ws", config.relay.route));
        assert_eq!(
            v["directory_url"],
            format!("https://{}/v1/instances", config.relay.route)
        );
        assert_eq!(v["tls_ca"], availability.tls_ca.as_str());
        assert!(!payload.contains("PRIVATE"), "never a key in the QR");
        let qr = render_qr(&payload).expect("the relay payload encodes");
        let width = qr.lines().next().unwrap().chars().count();
        assert!(width <= 120, "relay QR is {width} columns — too wide");
    }

    #[test]
    fn qr_renders_and_encodes_the_payload() {
        let payload = pairing::qr_payload_json(&pairing::QrPayloadInput {
            url: "wss://0b7d245998c0e8c3-desk.gaviero.example.com/v1/ws",
            token: &pairing::generate_token(),
            workspace: "gaviero",
            ..Default::default()
        });
        let qr = render_qr(&payload).expect("payload fits in a QR code");
        assert!(qr.lines().count() > 10);
        let width = qr.lines().next().unwrap().chars().count();
        assert!(qr.lines().all(|l| l.chars().count() == width), "square");
    }

    /// A typical relay pairing — 64-hex token, real machine CA, both relay
    /// fields — must encode inside a normal terminal width (the QR is
    /// useless if it wraps). The route appears three times (`url`,
    /// `directory_url`, `machine`), so the width grows with the relay host,
    /// machine label and workspace name; with a 36-char workspace name this
    /// same setup sits on the 117/121-column version boundary, and a 26-char
    /// relay host or a 40-char label reaches 121–129.
    #[test]
    fn typical_relay_payload_encodes_and_fits_a_terminal() {
        let ca_dir = tempfile::tempdir().unwrap();
        let machine = "desktop-abc1234";
        let relay_host = "gaviero.example.com";
        let host = format!("{machine}.{relay_host}");
        let pinned = pairing::ensure_pinned_ca_with(
            ca_dir.path(),
            &host,
            &[format!("*.{relay_host}")],
        )
        .unwrap();
        let route = format!("0b7d245998c0e8c3-{machine}.{relay_host}");
        let url = format!("wss://{route}/v1/ws");
        let directory_url = format!("https://{route}/v1/instances");
        let payload = pairing::qr_payload_json(&pairing::QrPayloadInput {
            url: &url,
            token: &pairing::generate_token(),
            workspace: "gaviero",
            workspace_id: Some("0b7d245998c0e8c3"),
            machine: Some(&host),
            directory_url: Some(&directory_url),
            tls_ca: Some(&pinned.ca_der_b64),
            relay_host: Some(relay_host),
            relay_secret: Some(&pairing::generate_token()),
        });
        let qr = render_qr(&payload).expect("worst-case payload still encodes");
        let width = qr.lines().next().unwrap().chars().count();
        assert!(
            width <= 120,
            "QR is {width} columns wide — too wide for a terminal panel"
        );
        // Half-block rendering: two QR rows per text row, plus quiet zone.
        assert!(qr.lines().count() >= width / 2 - 1);
    }

    /// The rendering must reproduce the encoder's module grid exactly.
    /// Parses the half-block output back into modules and compares against
    /// `QrCode` itself — this is what catches an inverted palette, an
    /// off-by-one quiet zone, or mispacked half-blocks, any of which
    /// produces a picture that looks like a QR code and does not scan.
    #[test]
    fn rendered_modules_match_the_encoder_exactly() {
        let payload = pairing::qr_payload_json(&pairing::QrPayloadInput {
            url: "wss://0b7d245998c0e8c3-desk.gaviero.example.com/v1/ws",
            token: &pairing::generate_token(),
            workspace: "gaviero",
            ..Default::default()
        });
        let code = encode_qr(&payload).unwrap();
        let width = code.width();
        let quiet = QUIET_ZONE;
        let rendered = render_qr(&payload).unwrap();
        let rows: Vec<Vec<char>> = rendered.lines().map(|l| l.chars().collect()).collect();

        // Half-block glyph → (top dark, bottom dark). Terminal is
        // light-on-dark, so a lit block means a LIGHT module.
        let unpack = |c: char| -> (bool, bool) {
            match c {
                ' ' => (true, true),
                '▄' => (true, false),
                '▀' => (false, true),
                '█' => (false, false),
                other => panic!("unexpected glyph {other:?}"),
            }
        };

        for y in 0..width {
            for x in 0..width {
                let expected = code[(x, y)] == qrcode::Color::Dark;
                let row = rows[(y + quiet) / 2].clone();
                let (top, bottom) = unpack(row[x + quiet]);
                let actual = if (y + quiet).is_multiple_of(2) {
                    top
                } else {
                    bottom
                };
                assert_eq!(
                    actual, expected,
                    "module ({x},{y}) mismatched — the rendered code would not scan"
                );
            }
        }

        // Quiet zone must be entirely light (lit blocks) on all four sides.
        for row in &rows {
            assert!(row[..quiet].iter().all(|&c| c == '█'), "left quiet zone");
            assert!(
                row[row.len() - quiet..].iter().all(|&c| c == '█'),
                "right quiet zone"
            );
        }
        assert!(rows[0].iter().all(|&c| c == '█'), "top quiet zone");
    }

    /// Plan C §2.5: a busy derived port falls forward inside the window; an
    /// explicit port does not. Binds real loopback listeners with the
    /// pinned leaf.
    #[tokio::test]
    async fn busy_derived_port_falls_forward_but_explicit_port_does_not() {
        let (_dir, ws) = relay_workspace(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        let mut config = isolated(&ws, &machine);
        ensure_certificate(&config).await.unwrap();

        // Occupy a free port and point the "derived" port at it.
        let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = blocker.local_addr().unwrap().port();
        config.port = busy;
        config.explicit_port = false;

        let token = load_or_create_token(&config).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let started = start(
            &config,
            check_availability(&config).unwrap(),
            token.clone(),
            "inst-test".into(),
            tx.clone(),
        )
        .await
        .expect("falls forward to a free port");
        assert_ne!(started.port, busy);
        assert!(started.port > busy && started.port < busy + PORT_WINDOW);
        assert_eq!(started.token_scope, TokenScope::Machine);
        let _ = started
            .handle
            .try_send(gaviero_remote::server::HubInput::Shutdown);

        config.explicit_port = true;
        let err = start(
            &config,
            check_availability(&config).unwrap(),
            token,
            "inst-test-2".into(),
            tx,
        )
        .await
        .expect_err("an explicit busy port must fail, never fall forward");
        assert!(matches!(err, RemoteUnavailable::Bind(_)), "{err}");
    }
}

#[cfg(test)]
mod render_preview {
    /// Prints the actual pairing QR. Run explicitly:
    /// `cargo test -p gaviero-tui preview_pairing_qr -- --ignored --nocapture`
    #[test]
    #[ignore = "visual check: prints a scannable QR to stdout"]
    fn preview_pairing_qr() {
        let payload =
            gaviero_remote::pairing::qr_payload_json(&gaviero_remote::pairing::QrPayloadInput {
                url: "wss://0b7d245998c0e8c3-desk.gaviero.example.com/v1/ws",
                token: &gaviero_remote::pairing::generate_token(),
                workspace: "gaviero",
                ..Default::default()
            });
        let qr = super::render_qr(&payload).unwrap();
        println!("payload {} bytes", payload.len());
        println!(
            "{} columns x {} rows",
            qr.lines().next().unwrap().chars().count(),
            qr.lines().count()
        );
        println!("{qr}");
    }
}

#[cfg(test)]
mod live_diagnostic {
    use super::*;

    /// Prints what `/remote` would report for a workspace right now. Under
    /// `cargo test` the cwd is the crate dir, so pass the workspace root
    /// explicitly:
    /// `GAVIERO_WS=<root> cargo test -p gaviero-tui live_remote_status -- --ignored --nocapture`
    #[test]
    #[ignore = "diagnostic: reports this machine's real remote readiness"]
    fn live_remote_status() {
        let root = std::env::var("GAVIERO_WS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap());
        let ws = Workspace::single_folder(root);
        match resolve_config(&ws) {
            Ok(config) => {
                println!("enabled:   {}", config.enabled);
                println!(
                    "port:      {} (explicit: {})",
                    config.port, config.explicit_port
                );
                println!("machine:   {}", config.machine_host);
                println!("relay:     {} route {}", config.relay.base_url, config.relay.route);
                println!(
                    "workspace: {} ({})",
                    config.workspace_display_name, config.workspace_id
                );
                println!("leaf:      {}", leaf_path(&config).display());
                println!(
                    "token:     {:?} machine dir {:?}",
                    config.token_scope, config.machine_state_dir
                );
                match check_availability(&config) {
                    Ok(a) => println!("AVAILABLE — cert until {}", a.cert.not_after),
                    Err(e) => println!("UNAVAILABLE — {e}"),
                }
            }
            Err(e) => println!("CONFIG ERROR — {e}"),
        }
    }
}
