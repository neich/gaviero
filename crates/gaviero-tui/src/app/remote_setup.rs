//! Remote sidecar bootstrap and the `/remote` command (Plan A unit A6,
//! reworked by Plan C V1 units C1/C2): settings resolution, MagicDNS host
//! detection, certificate selection and provisioning, machine-scoped
//! token lifecycle, bind policy with a bounded port window, background
//! server spawn, and QR rendering.
//!
//! Availability is fail-closed (§3.1): the sidecar refuses to claim it is
//! reachable unless a host is known, the certificate loads, covers that
//! host, and is not expired. The bootstrap runs in a spawned task and
//! reports through `Event::RemoteStarted` / `Event::RemoteUnavailable`
//! (Plan C invariant 13) — the event loop never runs the Tailscale CLI.
//! Secrets never reach logs — only
//! [`gaviero_remote::pairing::token_fingerprint`].

mod tailscale;

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

/// Where the MagicDNS host came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSource {
    /// `remote.magicDnsHost` in settings.
    Setting,
    /// `tailscale status --json` → `Self.DNSName`.
    Detected,
    /// Neither: the setting is empty and detection has not succeeded.
    Unresolved,
}

/// Which certificate pair is in force (Plan C §2.2 step 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertSource {
    /// `remote.certPath` / `remote.keyPath`.
    Explicit,
    /// `<workspace>/.gaviero/remote/tls/` from before Plan C, still valid.
    LegacyWorkspace,
    /// `~/.gaviero/remote/tls/` — shared by every workspace, auto-provisioned.
    Machine,
}

impl CertSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Explicit => "explicit (remote.certPath)",
            Self::LegacyWorkspace => "legacy workspace pair",
            Self::Machine => "machine pair (~/.gaviero/remote/tls)",
        }
    }
}

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

/// Resolved remote configuration, or the reason the sidecar cannot run.
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    pub enabled: bool,
    /// Configured or derived port. Absent setting ⇒ derived; the fallback
    /// window applies only when `explicit_port` is false.
    pub port: u16,
    pub explicit_port: bool,
    /// Empty until detected (setting empty and `resolve_host` not yet run).
    pub magic_dns_host: String,
    pub host_source: HostSource,
    /// Why detection failed, for `/remote`.
    pub host_detail: Option<String>,
    /// `remote.certPath` / `keyPath` when either is set.
    pub explicit_cert: Option<(PathBuf, PathBuf)>,
    /// The Plan A workspace pair, when both files exist.
    pub legacy_cert: Option<(PathBuf, PathBuf)>,
    pub allow_public_bind: bool,
    pub max_frame_bytes: u64,
    pub max_prompt_bytes: u64,
    pub command_rate_per_second: u32,
    /// Workspace remote state (`<root>/.gaviero/remote`).
    pub state_dir: PathBuf,
    /// Machine remote state (`~/.gaviero/remote`); `None` without a home.
    /// Tests override this so they never touch the real home directory.
    pub machine_state_dir: Option<PathBuf>,
    pub token_scope: TokenScope,
    pub auto_cert: bool,
    pub directory_enabled: bool,
    pub directory_port: u16,
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
    /// OS-assigned ephemeral, which would silently break QR stability.
    PortZero,
    /// Same rule for `remote.directoryPort`.
    DirectoryPortZero,
    /// No `remote.magicDnsHost` and auto-detection failed.
    NoMagicDnsHost {
        detail: String,
    },
    CertLoad(String),
    CertHostMismatch {
        host: String,
    },
    CertExpired {
        not_after: String,
    },
    /// `tailscale cert` failed; carries the CLI's own message.
    CertProvision(String),
    NoTailnetAddress,
    Bind(String),
    /// Every port in the derived window was busy.
    NoFreePort {
        from: u16,
        to: u16,
    },
    /// Token read/create failure.
    Token(String),
}

impl std::fmt::Display for RemoteUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(
                f,
                "remote is disabled — remove `remote.enabled: false` from .gaviero/settings.json \
                 (remote is on by default)"
            ),
            Self::NoWorkspaceRoot => write!(f, "no workspace root to anchor remote state"),
            Self::PortZero => write!(
                f,
                "`remote.port: 0` is not allowed (0 means an OS-assigned ephemeral port, \
                 which would change on every restart and break pairing). Remove the setting \
                 to use the stable derived port, or set a fixed port."
            ),
            Self::DirectoryPortZero => write!(
                f,
                "`remote.directoryPort: 0` is not allowed — remove the setting to use 49151, \
                 or set a fixed port"
            ),
            Self::NoMagicDnsHost { detail } => write!(
                f,
                "no MagicDNS host: {detail}. Either fix Tailscale or set `remote.magicDnsHost` \
                 (e.g. host.tailnet.ts.net) in .gaviero/settings.json"
            ),
            Self::CertLoad(e) => write!(f, "could not load the TLS certificate/key: {e}"),
            Self::CertHostMismatch { host } => write!(
                f,
                "the certificate does not cover {host} — reissue it with `tailscale cert {host}`"
            ),
            Self::CertExpired { not_after } => write!(
                f,
                "the certificate expired at {not_after} — renew with `tailscale cert`"
            ),
            Self::CertProvision(e) => write!(f, "could not provision the TLS certificate: {e}"),
            Self::NoTailnetAddress => write!(
                f,
                "no Tailscale address found on this machine — is Tailscale running and logged in?"
            ),
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
/// rejected rather than bound. Pure: computes paths, never creates files,
/// never runs the Tailscale CLI.
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
    let directory_port = match get(settings::REMOTE_DIRECTORY_PORT) {
        serde_json::Value::Null => gaviero_remote::DEFAULT_DIRECTORY_PORT,
        v => match v.as_u64() {
            Some(0) => return Err(RemoteUnavailable::DirectoryPortZero),
            Some(p) if p <= u16::MAX as u64 => p as u16,
            _ => gaviero_remote::DEFAULT_DIRECTORY_PORT,
        },
    };

    let state_dir = workspace
        .remote_state_dir()
        .ok_or(RemoteUnavailable::NoWorkspaceRoot)?;
    let machine_state_dir = gaviero_core::workspace::remote_machine_state_dir();
    let resolve_path = |raw: &str| -> PathBuf {
        let p = gaviero_core::workspace::expand_tilde_path(raw);
        if p.is_absolute() { p } else { root.join(p) }
    };

    let cert_setting = get(settings::REMOTE_CERT_PATH)
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let key_setting = get(settings::REMOTE_KEY_PATH)
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    let explicit_cert = if cert_setting.is_empty() && key_setting.is_empty() {
        None
    } else {
        // One of the two set ⇒ the other is its sibling with the default name.
        let cert = if cert_setting.is_empty() {
            resolve_path(&key_setting).with_file_name("cert.pem")
        } else {
            resolve_path(&cert_setting)
        };
        let key = if key_setting.is_empty() {
            cert.with_file_name("key.pem")
        } else {
            resolve_path(&key_setting)
        };
        Some((cert, key))
    };
    let legacy_dir = state_dir.join("tls");
    let legacy_pair = (legacy_dir.join("cert.pem"), legacy_dir.join("key.pem"));
    let legacy_cert = (legacy_pair.0.is_file() && legacy_pair.1.is_file()).then_some(legacy_pair);

    let magic_dns_host = get(settings::REMOTE_MAGIC_DNS_HOST)
        .as_str()
        .unwrap_or("")
        .trim()
        .trim_end_matches('.')
        .to_string();
    let host_source = if magic_dns_host.is_empty() {
        HostSource::Unresolved
    } else {
        HostSource::Setting
    };

    Ok(RemoteConfig {
        enabled,
        port,
        explicit_port,
        magic_dns_host,
        host_source,
        host_detail: None,
        explicit_cert,
        legacy_cert,
        allow_public_bind: get(settings::REMOTE_ALLOW_PUBLIC_BIND)
            .as_bool()
            .unwrap_or(false),
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
        auto_cert: get(settings::REMOTE_AUTO_CERT).as_bool().unwrap_or(true),
        directory_enabled: get(settings::REMOTE_DIRECTORY_ENABLED)
            .as_bool()
            .unwrap_or(true),
        directory_port,
        workspace_id: identity::workspace_id_hex16(&root),
        workspace_display_name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "workspace".to_string()),
    })
}

// ── Host detection (§2.2 step 2, async) ─────────────────────────────

/// Fill in the MagicDNS host when the setting is empty. Only ever awaited
/// from the bootstrap task.
pub async fn resolve_host(config: &mut RemoteConfig) {
    if !config.magic_dns_host.is_empty() {
        config.host_source = HostSource::Setting;
        return;
    }
    match tailscale::detect_magic_dns_host().await {
        Ok(host) => {
            config.magic_dns_host = host;
            config.host_source = HostSource::Detected;
            config.host_detail = None;
        }
        Err(detail) => {
            config.host_source = HostSource::Unresolved;
            config.host_detail = Some(detail);
        }
    }
}

// ── Certificate selection and provisioning (§2.2 step 3) ────────────

/// Explicit → legacy workspace pair (only if it still covers the host and
/// is not expired) → machine pair. Sync: a couple of file reads at most.
pub fn select_cert(config: &RemoteConfig) -> (CertSource, PathBuf, PathBuf) {
    if let Some((cert, key)) = &config.explicit_cert {
        return (CertSource::Explicit, cert.clone(), key.clone());
    }
    if let Some((cert, key)) = &config.legacy_cert {
        let usable = std::fs::read(cert)
            .ok()
            .and_then(|pem| pairing::inspect_cert(&pem, &config.magic_dns_host).ok())
            .is_some_and(|info| {
                (config.magic_dns_host.is_empty() || info.covers_host) && !info.is_expired()
            });
        if usable {
            return (CertSource::LegacyWorkspace, cert.clone(), key.clone());
        }
        tracing::info!(
            path = %cert.display(),
            "legacy workspace certificate ignored (does not cover the host or expired) — \
             using the machine pair"
        );
    }
    let dir = config
        .machine_state_dir
        .clone()
        .unwrap_or_else(|| config.state_dir.clone())
        .join("tls");
    (
        CertSource::Machine,
        dir.join("cert.pem"),
        dir.join("key.pem"),
    )
}

/// Provision or renew the machine pair with `tailscale cert` when it is
/// missing, does not cover the host, or expires within 7 days. Never
/// touches an explicit or legacy pair. Returns whether a certificate was
/// issued. Only ever awaited from the bootstrap task.
pub async fn ensure_certificate(config: &RemoteConfig) -> Result<bool, RemoteUnavailable> {
    if !config.auto_cert || config.magic_dns_host.is_empty() {
        return Ok(false);
    }
    let (source, cert_path, key_path) = select_cert(config);
    if source != CertSource::Machine {
        return Ok(false);
    }
    let cert_needs_issue = match std::fs::read(&cert_path)
        .ok()
        .and_then(|pem| pairing::inspect_cert(&pem, &config.magic_dns_host).ok())
    {
        Some(info) => !info.covers_host || info.is_expired() || info.is_near_expiry(),
        None => true,
    };
    // A certificate without its key is unusable: reissue the pair.
    if !cert_needs_issue && key_path.is_file() {
        return Ok(false);
    }
    tracing::info!(host = %config.magic_dns_host, path = %cert_path.display(), "issuing TLS certificate with tailscale cert");
    tailscale::provision_cert(&config.magic_dns_host, &cert_path, &key_path)
        .await
        .map_err(RemoteUnavailable::CertProvision)?;
    #[cfg(windows)]
    {
        if let Some(dir) = key_path.parent() {
            tracing::info!(
                dir = %dir.display(),
                "TLS key stored with inherited directory ACL — restrict this folder if the machine is shared"
            );
        }
    }
    Ok(true)
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
    pub cert_source: CertSource,
    pub cert_path: PathBuf,
    pub tailnet_addrs: Vec<IpAddr>,
}

/// Fail-closed availability check. Every failure names the fix. Sync —
/// safe to call from `/remote` inside the event loop.
pub fn check_availability(config: &RemoteConfig) -> Result<Availability, RemoteUnavailable> {
    if config.magic_dns_host.trim().is_empty() {
        return Err(RemoteUnavailable::NoMagicDnsHost {
            detail: config
                .host_detail
                .clone()
                .unwrap_or_else(|| "auto-detection has not run yet".to_string()),
        });
    }
    let (cert_source, cert_path, key_path) = select_cert(config);
    let cert_pem = std::fs::read(&cert_path)
        .map_err(|e| RemoteUnavailable::CertLoad(format!("{}: {e}", cert_path.display())))?;
    let key_pem = std::fs::read(&key_path)
        .map_err(|e| RemoteUnavailable::CertLoad(format!("{}: {e}", key_path.display())))?;
    let cert = pairing::inspect_cert(&cert_pem, &config.magic_dns_host)
        .map_err(RemoteUnavailable::CertLoad)?;
    if !cert.covers_host {
        return Err(RemoteUnavailable::CertHostMismatch {
            host: config.magic_dns_host.clone(),
        });
    }
    if cert.is_expired() {
        return Err(RemoteUnavailable::CertExpired {
            not_after: cert.not_after.clone(),
        });
    }
    let tailnet_addrs = pairing::detect_tailscale_addrs();
    if tailnet_addrs.is_empty() && !config.allow_public_bind {
        return Err(RemoteUnavailable::NoTailnetAddress);
    }
    Ok(Availability {
        cert_pem,
        key_pem,
        cert,
        cert_source,
        cert_path,
        tailnet_addrs,
    })
}

/// Bind list (§3.2): loopback plus detected tailnet addresses. Never a
/// wildcard; anything else requires `remote.allowPublicBind`.
pub fn bind_addrs(config: &RemoteConfig, tailnet: &[IpAddr]) -> Vec<SocketAddr> {
    bind_addrs_on(config, tailnet, config.port)
}

fn bind_addrs_on(config: &RemoteConfig, tailnet: &[IpAddr], port: u16) -> Vec<SocketAddr> {
    let mut addrs: Vec<SocketAddr> = vec![
        SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port),
        SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port),
    ];
    for ip in tailnet {
        if pairing::is_refused_bind_addr(ip) && !config.allow_public_bind {
            tracing::warn!(%ip, "refusing non-loopback, non-tailnet bind address");
            continue;
        }
        addrs.push(SocketAddr::new(*ip, port));
    }
    addrs
}

/// The pairing URL — always the MagicDNS hostname, never an IP or
/// `localhost`, because the certificate is issued to that name (§3.1).
pub fn pairing_url(config: &RemoteConfig) -> String {
    pairing_url_on(config, config.port)
}

pub fn pairing_url_on(config: &RemoteConfig, port: u16) -> String {
    format!(
        "wss://{}:{}{}",
        config.magic_dns_host,
        port,
        gaviero_remote::WS_PATH
    )
}

/// `https://<host>:<directoryPort>/v1/instances` when the directory is on.
pub fn directory_url(config: &RemoteConfig) -> Option<String> {
    config.directory_enabled.then(|| {
        format!(
            "https://{}:{}{}",
            config.magic_dns_host,
            config.directory_port,
            gaviero_remote::INSTANCES_PATH
        )
    })
}

/// The QR payload `/remote` renders: the 1.0 keys plus the 1.1 identity
/// keys (workspace id, machine host, directory URL when known).
pub fn qr_payload_for(
    config: &RemoteConfig,
    url: &str,
    token: &str,
    directory_url: Option<&str>,
) -> String {
    pairing::qr_payload_json(&pairing::QrPayloadInput {
        url,
        token,
        workspace: &config.workspace_display_name,
        workspace_id: Some(&config.workspace_id),
        machine: Some(&config.magic_dns_host),
        directory_url,
    })
}

// ── Server startup ──────────────────────────────────────────────────

/// Everything the bootstrap learned, delivered to the event loop as
/// `Event::RemoteStarted` and rendered by `/remote`.
#[derive(Clone)]
pub struct RemoteStarted {
    pub handle: RemoteHandle,
    /// The port actually bound (may differ from the derived one, §2.5).
    pub port: u16,
    pub host: String,
    pub host_source: HostSource,
    pub cert_source: CertSource,
    pub cert_path: PathBuf,
    pub cert_not_after: String,
    pub cert_near_expiry: bool,
    /// `tailscale cert` ran during this bootstrap.
    pub cert_provisioned: bool,
    pub tailnet_addrs: Vec<IpAddr>,
    pub token_scope: TokenScope,
    pub token_fingerprint: String,
    /// The machine directory port this instance participates in, if enabled.
    pub directory_port: Option<u16>,
    /// True while this process holds the directory listener. `None` if the
    /// directory is disabled.
    pub directory_leader: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub max_prompt_bytes: usize,
}

impl std::fmt::Debug for RemoteStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteStarted")
            .field("port", &self.port)
            .field("host", &self.host)
            .field("host_source", &self.host_source)
            .field("cert_source", &self.cert_source)
            .field("cert_not_after", &self.cert_not_after)
            .field("token_scope", &self.token_scope)
            .field("token_fingerprint", &self.token_fingerprint)
            .field("directory_port", &self.directory_port)
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
        let mut addrs = bind_addrs_on(config, &availability.tailnet_addrs, port);
        let primary = addrs.remove(0);
        let dir_url = directory_url(config);
        let directory_bind_addrs = if config.directory_enabled {
            bind_addrs_on(config, &availability.tailnet_addrs, config.directory_port)
        } else {
            Vec::new()
        };
        let registry = config.machine_state_dir.as_ref().map(|machine| {
            gaviero_remote::server::RegistryConfig {
                dir: machine.join("instances"),
                entry: gaviero_remote::dto::InstanceInfo {
                    instance_id: instance_id.clone(),
                    workspace: gaviero_remote::dto::WorkspaceInfo {
                        id: config.workspace_id.clone(),
                        display_name: config.workspace_display_name.clone(),
                    },
                    url: pairing_url_on(config, port),
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
            ],
            machine: Some(gaviero_remote::dto::MachineInfo {
                host: config.magic_dns_host.clone(),
                directory_url: dir_url.clone(),
            }),
            token_path: Some(token.path.clone()),
            token_poll_interval: RemoteServerConfig::TOKEN_POLL_INTERVAL,
            registry,
            directory_bind_addrs,
            heartbeat_interval: gaviero_remote::server::registry::HEARTBEAT_INTERVAL,
            directory_retry_interval: gaviero_remote::server::registry::DIRECTORY_RETRY_INTERVAL,
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
            host: config.magic_dns_host.clone(),
            host_source: config.host_source,
            cert_source: availability.cert_source,
            cert_path: availability.cert_path.clone(),
            cert_not_after: availability.cert.not_after.clone(),
            cert_near_expiry: availability.cert.is_near_expiry(),
            cert_provisioned: false,
            tailnet_addrs: availability.tailnet_addrs.clone(),
            token_scope: token.scope,
            token_fingerprint: pairing::token_fingerprint(&token.token),
            directory_port: config.directory_enabled.then_some(config.directory_port),
            directory_leader: config
                .directory_enabled
                .then(|| spawned.directory_leader.clone()),
            max_prompt_bytes: config.max_prompt_bytes as usize,
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

/// Resolve → detect host → ensure certificate → token → bind, then report
/// through the event channel. Spawned from `main`; never awaited by the
/// event loop.
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
    let mut config = resolve_config(workspace)?;
    if !config.enabled {
        return Err(RemoteUnavailable::Disabled);
    }
    resolve_host(&mut config).await;
    if config.magic_dns_host.is_empty() {
        return Err(RemoteUnavailable::NoMagicDnsHost {
            detail: config
                .host_detail
                .clone()
                .unwrap_or_else(|| "detection produced no host".to_string()),
        });
    }
    let provisioned = ensure_certificate(&config).await?;
    let availability = check_availability(&config)?;
    let token = load_or_create_token(&config).map_err(RemoteUnavailable::Token)?;
    let instance_id = pairing::generate_token()[..16].to_string();
    let mut started = start(&config, availability, token, instance_id, event_tx).await?;
    started.cert_provisioned = provisioned;
    Ok(started)
}

// ── QR rendering ────────────────────────────────────────────────────

/// QR quiet zone, in modules (spec minimum is 4).
const QUIET_ZONE: usize = 4;

/// Render the pairing payload as a half-block QR sized for a terminal.
/// Two vertical modules per character row keeps it scannable in a side
/// panel; a quiet zone is included because scanners need it.
pub fn render_qr(payload: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(payload.as_bytes())
        .map_err(|e| format!("could not encode the pairing QR: {e}"))?;
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
/// Runs inside the event loop: file reads only, never the Tailscale CLI —
/// the host comes from settings or from what the bootstrap detected.
pub fn handle_remote_command(app: &mut App, line: &str) {
    let arg = line
        .trim()
        .strip_prefix("/remote")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();

    let mut config = match resolve_config(&app.workspace) {
        Ok(c) => c,
        Err(e) => {
            app.chat_state.add_system_message(&format!("Remote: {e}"));
            return;
        }
    };
    if config.magic_dns_host.is_empty()
        && let RemoteStatus::Running(started) = &app.remote.status
    {
        config.magic_dns_host = started.host.clone();
        config.host_source = started.host_source;
    }

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
             and restart gaviero (remote is on by default).\n",
        );
    }
    let running = match &app.remote.status {
        RemoteStatus::Running(s) => Some(s.as_ref()),
        _ => None,
    };
    let port = running.map(|s| s.port).unwrap_or(config.port);
    report.push_str(&format!(
        "Workspace: {} ({})\nPort: {}{}\n",
        config.workspace_display_name,
        config.workspace_id,
        port,
        if config.explicit_port {
            " (fixed by remote.port)".to_string()
        } else if port != config.port {
            format!(
                " (derived port {} was busy — the phone keeps that port until it \
                 refreshes the directory; pull-to-refresh Instances or reopen the app)",
                config.port
            )
        } else {
            " (derived from the workspace identity)".to_string()
        }
    ));
    report.push_str(&format!(
        "Host: {}\n",
        match config.host_source {
            HostSource::Setting => format!("{} (remote.magicDnsHost)", config.magic_dns_host),
            HostSource::Detected =>
                format!("{} (auto-detected via tailscale)", config.magic_dns_host),
            HostSource::Unresolved => match &app.remote.status {
                RemoteStatus::Unavailable(e) => format!("unresolved — {e}"),
                RemoteStatus::Pending => "detecting…".to_string(),
                RemoteStatus::Running(s) => s.host.clone(),
            },
        }
    ));

    match check_availability(config) {
        Ok(availability) => {
            let expiry = if availability.cert.is_near_expiry() {
                format!(
                    "{} (EXPIRES SOON — {})",
                    availability.cert.not_after,
                    if availability.cert_source == CertSource::Machine && config.auto_cert {
                        "renews automatically on the next start".to_string()
                    } else {
                        format!("renew with `tailscale cert {}`", config.magic_dns_host)
                    }
                )
            } else {
                availability.cert.not_after.clone()
            };
            report.push_str(&format!(
                "Certificate: {} — valid until {}\nTailnet addresses: {}\n",
                availability.cert_source.label(),
                expiry,
                availability
                    .tailnet_addrs
                    .iter()
                    .map(|ip| ip.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
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

    let dir_role = match &app.remote.status {
        RemoteStatus::Running(s) if s.directory_port.is_none() => "disabled".to_string(),
        RemoteStatus::Running(s) => {
            let leader = s
                .directory_leader
                .as_ref()
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::Relaxed));
            if leader {
                format!(
                    "leader on {}",
                    s.directory_port.unwrap_or(config.directory_port)
                )
            } else {
                format!(
                    "follower (another instance holds {})",
                    s.directory_port.unwrap_or(config.directory_port)
                )
            }
        }
        _ if !config.directory_enabled => "disabled".to_string(),
        _ => "starting".to_string(),
    };
    report.push_str(&format!("Directory: {dir_role}\n"));

    if let Some(machine) = &config.machine_state_dir {
        let listed = gaviero_remote::server::registry::read_directory(
            &machine.join("instances"),
            &config.magic_dns_host,
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
            RemoteStatus::Pending =>
                "starting (host detection / certificate check in progress)".to_string(),
            RemoteStatus::Unavailable(e) => format!("unavailable — {e}"),
            RemoteStatus::Running(_) if app.remote.client_connected =>
                "client connected".to_string(),
            RemoteStatus::Running(_) => "listening, no client connected".to_string(),
        }
    ));

    {
        let roots = app.workspace.roots();
        if crate::notify::ntfy_enabled(&app.workspace, roots.first().copied()) {
            report.push_str("ntfy: on (run /ntfy)\n");
        }
    }

    // Listening with no client is usually the phone off the tailnet, then
    // (on Windows) the host firewall dropping inbound on the Tailscale NIC.
    // The app only reports "instance offline" for both.
    if running.is_some() && !app.remote.client_connected {
        report.push_str(
            "\nIf the app says \"instance offline\", check `tailscale status` on this PC: \
             the phone must be listed without \"offline\" (open the Tailscale app on the phone, \
             same tailnet). Disconnect Proton / other VPNs on the phone — a kill switch \
             swallows 100.x and MagicDNS.\n",
        );
        #[cfg(windows)]
        report.push_str(&format!(
            "If the phone is online and still cannot connect, Windows Firewall is dropping \
             inbound on the tailnet interface. In an ADMIN PowerShell (one rule covers every \
             workspace and the directory port):\n  \
             New-NetFirewallRule -DisplayName \"Gaviero Remote (tailnet)\" -Direction Inbound \
             -Action Allow -Protocol TCP -LocalPort {},49152-65535 \
             -RemoteAddress 100.64.0.0/10,fd7a:115c:a1e0::/48\n",
            config.directory_port
        ));
    }

    if let (Some(started), Ok(loaded)) = (running, &token) {
        let url = pairing_url_on(config, started.port);
        let dir_url = started.directory_port.map(|p| {
            format!(
                "https://{}:{p}{}",
                config.magic_dns_host,
                gaviero_remote::INSTANCES_PATH
            )
        });
        let payload = qr_payload_for(config, &url, &loaded.token, dir_url.as_deref());
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

    /// Tests must never touch the real `~/.gaviero/remote`.
    fn isolated(ws: &Workspace, machine: &tempfile::TempDir) -> RemoteConfig {
        let mut config = resolve_config(ws).unwrap();
        config.machine_state_dir = Some(machine.path().join("remote"));
        config
    }

    fn write_pair(dir: &std::path::Path, host: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec![host.to_string()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        std::fs::write(dir.join("cert.pem"), cert.pem()).unwrap();
        std::fs::write(dir.join("key.pem"), key.serialize_pem()).unwrap();
    }

    #[test]
    fn remote_is_enabled_by_default() {
        let (_dir, ws) = workspace_with(serde_json::json!({}));
        let config = resolve_config(&ws).unwrap();
        assert!(config.enabled, "Plan C: on by default");
        assert_eq!(config.token_scope, TokenScope::Machine);
        assert!(config.auto_cert);
        assert!(config.directory_enabled);
        assert_eq!(config.directory_port, 49151);
        assert_eq!(config.host_source, HostSource::Unresolved);
        assert!(config.explicit_cert.is_none());
        assert!(config.legacy_cert.is_none());
    }

    #[test]
    fn port_zero_is_rejected_with_an_actionable_message() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "port": 0 } }));
        let err = resolve_config(&ws).expect_err("port 0 must be refused");
        assert!(matches!(err, RemoteUnavailable::PortZero));
        let msg = err.to_string();
        assert!(msg.contains("ephemeral"), "{msg}");
    }

    #[test]
    fn directory_port_zero_is_rejected() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "directoryPort": 0 } }));
        let err = resolve_config(&ws).expect_err("directory port 0 must be refused");
        assert!(matches!(err, RemoteUnavailable::DirectoryPortZero));
    }

    #[test]
    fn absent_port_derives_a_stable_workspace_port() {
        let (dir, ws) = workspace_with(serde_json::json!({}));
        let a = resolve_config(&ws).unwrap();
        let b = resolve_config(&ws).unwrap();
        assert_eq!(a.port, b.port, "derived port is identical across restarts");
        assert_eq!(a.port, identity::derive_remote_port(dir.path()));
        assert!((49152..=65535).contains(&a.port));
        assert!(!a.explicit_port);
    }

    #[test]
    fn explicit_port_overrides_the_derivation() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "port": 51234 } }));
        let config = resolve_config(&ws).unwrap();
        assert_eq!(config.port, 51234);
        assert!(config.explicit_port, "an explicit port never falls forward");
    }

    #[test]
    fn setting_host_wins_and_trailing_dot_is_stripped() {
        let (_dir, ws) = workspace_with(
            serde_json::json!({ "remote": { "magicDnsHost": "host.tailnet.ts.net." } }),
        );
        let config = resolve_config(&ws).unwrap();
        assert_eq!(config.magic_dns_host, "host.tailnet.ts.net");
        assert_eq!(config.host_source, HostSource::Setting);
    }

    #[test]
    fn availability_fails_closed_without_a_magic_dns_host() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "enabled": true } }));
        let machine = tempfile::tempdir().unwrap();
        let mut config = isolated(&ws, &machine);
        config.host_detail = Some("tailscale CLI not found".to_string());
        let err = check_availability(&config).expect_err("must refuse to claim availability");
        match err {
            RemoteUnavailable::NoMagicDnsHost { ref detail } => {
                assert!(detail.contains("tailscale CLI not found"));
            }
            other => panic!("expected NoMagicDnsHost, got {other:?}"),
        }
        assert!(err.to_string().contains("remote.magicDnsHost"));
    }

    #[test]
    fn explicit_cert_setting_reports_a_hostname_mismatch() {
        let (dir, ws) = workspace_with(serde_json::json!({
            "remote": {
                "magicDnsHost": "wrong.tailnet.ts.net",
                "certPath": "certs/cert.pem",
                "keyPath": "certs/key.pem"
            }
        }));
        write_pair(&dir.path().join("certs"), "host.tailnet.ts.net");
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        let (source, cert, _) = select_cert(&config);
        assert_eq!(source, CertSource::Explicit);
        assert_eq!(cert, dir.path().join("certs").join("cert.pem"));
        let err = check_availability(&config).expect_err("hostname mismatch must fail");
        match err {
            RemoteUnavailable::CertHostMismatch { host } => {
                assert_eq!(host, "wrong.tailnet.ts.net");
            }
            other => panic!("expected CertHostMismatch, got {other:?}"),
        }
    }

    #[test]
    fn valid_legacy_workspace_pair_is_preferred_over_a_missing_machine_pair() {
        let (dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net" }
        }));
        write_pair(
            &dir.path().join(".gaviero/remote/tls"),
            "host.tailnet.ts.net",
        );
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        assert!(config.legacy_cert.is_some());
        let (source, cert, _) = select_cert(&config);
        assert_eq!(source, CertSource::LegacyWorkspace);
        assert_eq!(cert, dir.path().join(".gaviero/remote/tls/cert.pem"));
    }

    #[test]
    fn mismatched_legacy_pair_is_skipped_and_the_machine_path_is_named() {
        let (dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "wrong.tailnet.ts.net" }
        }));
        write_pair(
            &dir.path().join(".gaviero/remote/tls"),
            "host.tailnet.ts.net",
        );
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        let (source, cert, _) = select_cert(&config);
        assert_eq!(
            source,
            CertSource::Machine,
            "mismatched legacy pair must be ignored"
        );
        assert!(cert.starts_with(machine.path()));
        let err = check_availability(&config).expect_err("machine pair does not exist yet");
        match err {
            RemoteUnavailable::CertLoad(msg) => {
                assert!(msg.contains("remote"), "names the machine path: {msg}");
            }
            other => panic!("expected CertLoad, got {other:?}"),
        }
    }

    #[test]
    fn valid_machine_pair_is_used_when_nothing_else_exists() {
        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net" }
        }));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        write_pair(&machine.path().join("remote/tls"), "host.tailnet.ts.net");
        let (source, _, _) = select_cert(&config);
        assert_eq!(source, CertSource::Machine);
        // Tailnet detection may legitimately find nothing on CI; only the
        // certificate part of availability is asserted here.
        match check_availability(&config) {
            Ok(a) => assert_eq!(a.cert_source, CertSource::Machine),
            Err(RemoteUnavailable::NoTailnetAddress) => {}
            Err(other) => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ensure_certificate_is_a_no_op_for_explicit_and_legacy_pairs() {
        let (dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net" }
        }));
        write_pair(
            &dir.path().join(".gaviero/remote/tls"),
            "host.tailnet.ts.net",
        );
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        assert!(!ensure_certificate(&config).await.unwrap());
        assert!(!machine.path().join("remote/tls/cert.pem").exists());

        let mut off = config.clone();
        off.auto_cert = false;
        off.legacy_cert = None;
        assert!(
            !ensure_certificate(&off).await.unwrap(),
            "autoCert off never provisions"
        );
    }

    #[test]
    fn machine_token_is_created_once_reread_stably_and_rotated() {
        let (_dir, ws) = workspace_with(serde_json::json!({}));
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
        let (_a, ws_a) = workspace_with(serde_json::json!({}));
        let (_b, ws_b) = workspace_with(serde_json::json!({}));
        let machine = tempfile::tempdir().unwrap();
        let a = load_or_create_token(&isolated(&ws_a, &machine)).unwrap();
        let b = load_or_create_token(&isolated(&ws_b, &machine)).unwrap();
        assert_eq!(a.token, b.token, "one pairing per machine");
    }

    #[test]
    fn workspace_scope_opts_out_of_the_machine_token() {
        let (_dir, ws) =
            workspace_with(serde_json::json!({ "remote": { "tokenScope": "workspace" } }));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        let loaded = load_or_create_token(&config).unwrap();
        assert_eq!(loaded.scope, TokenScope::Workspace);
        assert_eq!(loaded.path, config.state_dir.join("token"));
        assert!(!machine.path().join("remote/token").exists());
    }

    #[test]
    fn unwritable_machine_root_falls_back_to_the_workspace_token() {
        let (_dir, ws) = workspace_with(serde_json::json!({}));
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
    fn bind_list_is_loopback_plus_tailnet_never_wildcard() {
        let (_dir, ws) = workspace_with(serde_json::json!({}));
        let config = resolve_config(&ws).unwrap();
        let tailnet: Vec<IpAddr> = vec![
            "100.101.102.103".parse().unwrap(),
            "192.168.1.5".parse().unwrap(), // LAN — must be refused
        ];
        let addrs = bind_addrs(&config, &tailnet);
        assert!(addrs.iter().any(|a| a.ip().is_loopback()));
        assert!(
            addrs
                .iter()
                .any(|a| a.ip().to_string() == "100.101.102.103")
        );
        assert!(
            !addrs.iter().any(|a| a.ip().to_string() == "192.168.1.5"),
            "LAN address must be refused without allowPublicBind"
        );
        assert!(
            !addrs.iter().any(|a| a.ip().is_unspecified()),
            "never bind a wildcard address"
        );
    }

    #[test]
    fn pairing_url_uses_magic_dns_never_an_ip() {
        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net", "port": 50123 }
        }));
        let config = resolve_config(&ws).unwrap();
        assert_eq!(
            pairing_url(&config),
            "wss://host.tailnet.ts.net:50123/v1/ws"
        );
        assert_eq!(
            directory_url(&config).unwrap(),
            "https://host.tailnet.ts.net:49151/v1/instances"
        );
    }

    #[test]
    fn qr_renders_and_encodes_the_payload() {
        let payload = pairing::qr_payload_json(&pairing::QrPayloadInput {
            url: "wss://host.tailnet.ts.net:50123/v1/ws",
            token: &pairing::generate_token(),
            workspace: "gaviero",
            ..Default::default()
        });
        let qr = render_qr(&payload).expect("payload fits in a QR code");
        assert!(qr.lines().count() > 10);
        let width = qr.lines().next().unwrap().chars().count();
        assert!(qr.lines().all(|l| l.chars().count() == width), "square");
    }

    /// The realistic worst case: a 64-hex token, a long MagicDNS hostname,
    /// a long workspace name, AND the three 1.1 keys (workspace id, machine,
    /// directory URL) must still encode, and the rendering must stay inside
    /// a normal terminal width (the QR is useless if it wraps).
    #[test]
    fn worst_case_payload_encodes_and_fits_a_terminal() {
        let payload = pairing::qr_payload_json(&pairing::QrPayloadInput {
            url: "wss://very-long-machine-name.tail9f2c81.ts.net:65535/v1/ws",
            token: &pairing::generate_token(),
            workspace: "a-fairly-long-workspace-display-name",
            workspace_id: Some("0b7d245998c0e8c3"),
            machine: Some("very-long-machine-name.tail9f2c81.ts.net"),
            directory_url: Some(
                "https://very-long-machine-name.tail9f2c81.ts.net:49151/v1/instances",
            ),
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
            url: "wss://host.tailnet.ts.net:50123/v1/ws",
            token: &pairing::generate_token(),
            workspace: "gaviero",
            ..Default::default()
        });
        let code = qrcode::QrCode::new(payload.as_bytes()).unwrap();
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

    /// The payload the app validates: `kind` and `protocol_major` gate
    /// pairing on the client (Plan B B9), so they must survive the exact
    /// path `/remote` uses — now with the 1.1 identity keys.
    #[test]
    fn rendered_pairing_payload_carries_what_the_app_validates() {
        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net", "port": 50123 }
        }));
        let machine = tempfile::tempdir().unwrap();
        let config = isolated(&ws, &machine);
        let loaded = load_or_create_token(&config).unwrap();
        let dir_url = directory_url(&config);
        let payload = qr_payload_for(
            &config,
            &pairing_url(&config),
            &loaded.token,
            dir_url.as_deref(),
        );
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["kind"], "gaviero-remote");
        assert_eq!(v["protocol_major"], 1);
        assert_eq!(v["url"], "wss://host.tailnet.ts.net:50123/v1/ws");
        assert_eq!(v["token"], loaded.token);
        assert_eq!(v["workspace_id"], config.workspace_id);
        assert_eq!(v["machine"], "host.tailnet.ts.net");
        assert_eq!(
            v["directory_url"],
            "https://host.tailnet.ts.net:49151/v1/instances"
        );
        render_qr(&payload).expect("the /remote payload encodes");
    }

    /// Plan C §2.5: a busy derived port falls forward inside the window; an
    /// explicit port does not. Binds real loopback listeners with a
    /// generated certificate.
    #[tokio::test]
    async fn busy_derived_port_falls_forward_but_explicit_port_does_not() {
        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net", "allowPublicBind": true }
        }));
        let machine = tempfile::tempdir().unwrap();
        let mut config = isolated(&ws, &machine);
        write_pair(&machine.path().join("remote/tls"), "host.tailnet.ts.net");

        // Occupy a free port and point the "derived" port at it.
        let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let busy = blocker.local_addr().unwrap().port();
        config.port = busy;
        config.explicit_port = false;

        let availability = || {
            let cert_pem = std::fs::read(machine.path().join("remote/tls/cert.pem")).unwrap();
            let key_pem = std::fs::read(machine.path().join("remote/tls/key.pem")).unwrap();
            let cert = pairing::inspect_cert(&cert_pem, "host.tailnet.ts.net").unwrap();
            Availability {
                cert_pem,
                key_pem,
                cert,
                cert_source: CertSource::Machine,
                cert_path: machine.path().join("remote/tls/cert.pem"),
                tailnet_addrs: Vec::new(),
            }
        };
        let token = load_or_create_token(&config).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        let started = start(
            &config,
            availability(),
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
        let err = start(&config, availability(), token, "inst-test-2".into(), tx)
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
                url: "wss://host.tailnet.ts.net:50123/v1/ws",
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

    /// Prints what `/remote` would report for a workspace right now,
    /// including real host detection. Under `cargo test` the cwd is the
    /// crate dir, so pass the workspace root explicitly:
    /// `GAVIERO_WS=<root> cargo test -p gaviero-tui live_remote_status -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "diagnostic: reports this machine's real remote readiness"]
    async fn live_remote_status() {
        let root = std::env::var("GAVIERO_WS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::current_dir().unwrap());
        let ws = Workspace::single_folder(root);
        match resolve_config(&ws) {
            Ok(mut config) => {
                resolve_host(&mut config).await;
                println!("enabled:   {}", config.enabled);
                println!(
                    "port:      {} (explicit: {})",
                    config.port, config.explicit_port
                );
                println!(
                    "host:      {} ({:?}) {}",
                    config.magic_dns_host,
                    config.host_source,
                    config.host_detail.clone().unwrap_or_default()
                );
                println!(
                    "workspace: {} ({})",
                    config.workspace_display_name, config.workspace_id
                );
                let (source, cert, _) = select_cert(&config);
                println!("cert:      {:?} {}", source, cert.display());
                println!(
                    "token:     {:?} machine dir {:?}",
                    config.token_scope, config.machine_state_dir
                );
                match check_availability(&config) {
                    Ok(a) => println!(
                        "AVAILABLE — cert until {}, tailnet {:?}",
                        a.cert.not_after, a.tailnet_addrs
                    ),
                    Err(e) => println!("UNAVAILABLE — {e}"),
                }
            }
            Err(e) => println!("CONFIG ERROR — {e}"),
        }
    }
}
