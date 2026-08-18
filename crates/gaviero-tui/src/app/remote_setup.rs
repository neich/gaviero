//! Remote sidecar bootstrap and the `/remote` command (Plan A unit A6):
//! settings resolution, workspace-derived port, token lifecycle, TLS
//! diagnostics, bind policy, server spawn, and QR rendering.
//!
//! Availability is fail-closed (§3.1): `/remote` refuses to claim it is
//! reachable unless the MagicDNS host is configured, the certificate loads,
//! covers that host, and is not expired. Secrets never reach logs — only
//! [`gaviero_remote::pairing::token_fingerprint`].

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use gaviero_core::workspace::{Workspace, identity, settings};
use gaviero_remote::pairing;
use gaviero_remote::server::{HubOutput, RemoteServerConfig};

use crate::app::App;
use crate::event::Event;

/// Resolved remote configuration, or the reason the sidecar cannot run.
#[derive(Debug)]
pub struct RemoteConfig {
    pub enabled: bool,
    pub port: u16,
    pub magic_dns_host: String,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub allow_public_bind: bool,
    pub max_frame_bytes: u64,
    pub max_prompt_bytes: u64,
    pub command_rate_per_second: u32,
    pub state_dir: PathBuf,
    pub workspace_id: String,
    pub workspace_display_name: String,
}

/// Why the sidecar is unavailable — every variant is actionable.
#[derive(Debug)]
pub enum RemoteUnavailable {
    /// Reported by `/remote` when `remote.enabled` is false; the startup
    /// path logs and skips instead of surfacing this.
    #[allow(dead_code)]
    Disabled,
    NoWorkspaceRoot,
    /// `remote.port = 0` is a configuration error (§3.2): `0` means
    /// OS-assigned ephemeral, which would silently break QR stability.
    PortZero,
    MissingMagicDnsHost,
    CertLoad(String),
    CertHostMismatch {
        host: String,
    },
    CertExpired {
        not_after: String,
    },
    NoTailnetAddress,
    Bind(String),
    /// Token read/create failure; the startup path reports it through its
    /// own log line, `/remote` renders it inline.
    #[allow(dead_code)]
    Token(String),
}

impl std::fmt::Display for RemoteUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(
                f,
                "remote is disabled — set `remote.enabled: true` in .gaviero/settings.json"
            ),
            Self::NoWorkspaceRoot => write!(f, "no workspace root to anchor remote state"),
            Self::PortZero => write!(
                f,
                "`remote.port: 0` is not allowed (0 means an OS-assigned ephemeral port, \
                 which would change on every restart and break pairing). Remove the setting \
                 to use the stable derived port, or set a fixed port."
            ),
            Self::MissingMagicDnsHost => write!(
                f,
                "`remote.magicDnsHost` is empty — set it to this machine's Tailscale MagicDNS \
                 name (e.g. host.tailnet.ts.net); the certificate is issued to that name"
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
            Self::NoTailnetAddress => write!(
                f,
                "no Tailscale address found on this machine — is Tailscale running and logged in?"
            ),
            Self::Bind(e) => write!(f, "could not bind the remote listener: {e}"),
            Self::Token(e) => write!(f, "could not read or create the pairing token: {e}"),
        }
    }
}

/// Read the `remote.*` settings for the workspace. `remote.port` absent or
/// null derives a stable port from the workspace identity (§3.3); `0` is
/// rejected rather than bound.
pub fn resolve_config(workspace: &Workspace) -> Result<RemoteConfig, RemoteUnavailable> {
    let Some(root) = workspace.roots().first().map(|p| p.to_path_buf()) else {
        return Err(RemoteUnavailable::NoWorkspaceRoot);
    };
    let scope = Some(root.as_path());
    let get = |key: &str| workspace.resolve_setting(key, scope);

    let enabled = get(settings::REMOTE_ENABLED).as_bool().unwrap_or(false);

    let port = match get(settings::REMOTE_PORT) {
        serde_json::Value::Null => identity::derive_remote_port(&root),
        v => match v.as_u64() {
            Some(0) => return Err(RemoteUnavailable::PortZero),
            Some(p) if p <= u16::MAX as u64 => p as u16,
            _ => identity::derive_remote_port(&root),
        },
    };

    let state_dir = workspace
        .remote_state_dir()
        .ok_or(RemoteUnavailable::NoWorkspaceRoot)?;
    let resolve_path = |raw: &str| -> PathBuf {
        let p = PathBuf::from(raw);
        if p.is_absolute() { p } else { root.join(p) }
    };

    Ok(RemoteConfig {
        enabled,
        port,
        magic_dns_host: get(settings::REMOTE_MAGIC_DNS_HOST)
            .as_str()
            .unwrap_or("")
            .to_string(),
        cert_path: resolve_path(
            get(settings::REMOTE_CERT_PATH)
                .as_str()
                .unwrap_or(".gaviero/remote/tls/cert.pem"),
        ),
        key_path: resolve_path(
            get(settings::REMOTE_KEY_PATH)
                .as_str()
                .unwrap_or(".gaviero/remote/tls/key.pem"),
        ),
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
        workspace_id: identity::workspace_id_hex16(&root),
        workspace_display_name: root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "workspace".to_string()),
    })
}

// ── Token lifecycle (§3.4) ──────────────────────────────────────────

fn token_path(config: &RemoteConfig) -> PathBuf {
    config.state_dir.join("token")
}

/// Read the stored token, generating and persisting one on first use.
/// Owner-only permissions where supported; on Windows the file inherits
/// the directory ACL and a broadly-writable directory is warned about.
pub fn load_or_create_token(config: &RemoteConfig) -> Result<String, String> {
    let path = token_path(config);
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }
    let token = pairing::generate_token();
    write_token(config, &token)?;
    Ok(token)
}

fn write_token(config: &RemoteConfig, token: &str) -> Result<(), String> {
    let dir = &config.state_dir;
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = token_path(config);
    // Atomic replace so a concurrent reader never sees a partial token.
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, token).map_err(|e| format!("writing token: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("replacing token: {e}"))?;
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

/// Desktop-only rotation (§3.4): atomically replace the token file. The
/// caller closes the live socket with 4006 via `HubInput::TokenRotated`.
pub fn rotate_token(config: &RemoteConfig) -> Result<String, String> {
    let token = pairing::generate_token();
    write_token(config, &token)?;
    Ok(token)
}

// ── Availability (§3.1) ─────────────────────────────────────────────

#[derive(Debug)]
pub struct Availability {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub cert: pairing::CertInfo,
    pub tailnet_addrs: Vec<IpAddr>,
}

/// Fail-closed availability check. Every failure names the fix.
pub fn check_availability(config: &RemoteConfig) -> Result<Availability, RemoteUnavailable> {
    if config.magic_dns_host.trim().is_empty() {
        return Err(RemoteUnavailable::MissingMagicDnsHost);
    }
    let cert_pem = std::fs::read(&config.cert_path)
        .map_err(|e| RemoteUnavailable::CertLoad(format!("{}: {e}", config.cert_path.display())))?;
    let key_pem = std::fs::read(&config.key_path)
        .map_err(|e| RemoteUnavailable::CertLoad(format!("{}: {e}", config.key_path.display())))?;
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
        tailnet_addrs,
    })
}

/// Bind list (§3.2): loopback plus detected tailnet addresses. Never a
/// wildcard; anything else requires `remote.allowPublicBind`.
pub fn bind_addrs(config: &RemoteConfig, tailnet: &[IpAddr]) -> Vec<SocketAddr> {
    let mut addrs: Vec<SocketAddr> = vec![
        SocketAddr::new(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), config.port),
        SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), config.port),
    ];
    for ip in tailnet {
        if pairing::is_refused_bind_addr(ip) && !config.allow_public_bind {
            tracing::warn!(%ip, "refusing non-loopback, non-tailnet bind address");
            continue;
        }
        addrs.push(SocketAddr::new(*ip, config.port));
    }
    addrs
}

/// The pairing URL — always the MagicDNS hostname, never an IP or
/// `localhost`, because the certificate is issued to that name (§3.1).
pub fn pairing_url(config: &RemoteConfig) -> String {
    format!(
        "wss://{}:{}{}",
        config.magic_dns_host,
        config.port,
        gaviero_remote::WS_PATH
    )
}

// ── Server startup ──────────────────────────────────────────────────

/// Start the sidecar and bridge its outputs into the TUI event channel.
/// Returns the hub handle for the event loop to `try_send` on.
pub async fn start(
    config: &RemoteConfig,
    availability: Availability,
    token: String,
    instance_id: String,
    event_tx: tokio::sync::mpsc::UnboundedSender<Event>,
) -> Result<gaviero_remote::server::RemoteHandle, RemoteUnavailable> {
    let mut addrs = bind_addrs(config, &availability.tailnet_addrs);
    let primary = addrs.remove(0);
    let (ping_interval, idle_timeout, hello_timeout) = RemoteServerConfig::timing_defaults();

    let spawned = gaviero_remote::server::spawn(RemoteServerConfig {
        bind_addr: primary,
        extra_bind_addrs: addrs,
        tls_cert_pem: availability.cert_pem,
        tls_key_pem: availability.key_pem,
        token,
        instance_id,
        tui_version: env!("CARGO_PKG_VERSION").to_string(),
        workspace: gaviero_remote::dto::WorkspaceInfo {
            id: config.workspace_id.clone(),
            display_name: config.workspace_display_name.clone(),
        },
        capabilities: Vec::new(),
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
    .await
    .map_err(|e| RemoteUnavailable::Bind(e.to_string()))?;

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
    Ok(spawned.handle)
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
            Ok(token) => {
                if let Some(handle) = app.remote.handle.as_ref() {
                    // Desktop-only rotation closes the live client with 4006.
                    let _ = handle.try_send(gaviero_remote::server::HubInput::TokenRotated {
                        new_token: token.clone(),
                    });
                }
                app.chat_state.add_system_message(&format!(
                    "Remote: token rotated ({}). Any paired device must scan again — \
                     run /remote for the new QR.",
                    pairing::token_fingerprint(&token)
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
            let mut report = String::new();
            if !config.enabled {
                report.push_str(
                    "Remote: disabled. Set `remote.enabled: true` in .gaviero/settings.json, \
                     then restart gaviero.\n",
                );
            }
            report.push_str(&format!(
                "Workspace: {} ({})\nPort: {} (derived from the workspace identity unless \
                 `remote.port` is set)\n",
                config.workspace_display_name, config.workspace_id, config.port
            ));
            match check_availability(&config) {
                Ok(availability) => {
                    let expiry = if availability.cert.is_near_expiry() {
                        format!(
                            "{} (EXPIRES SOON — renew with `tailscale cert {}`)",
                            availability.cert.not_after, config.magic_dns_host
                        )
                    } else {
                        availability.cert.not_after.clone()
                    };
                    report.push_str(&format!(
                        "Host: {}\nCertificate: valid until {}\nTailnet addresses: {}\n",
                        config.magic_dns_host,
                        expiry,
                        availability
                            .tailnet_addrs
                            .iter()
                            .map(|ip| ip.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    report.push_str(&format!(
                        "Status: {}\n",
                        if app.remote.handle.is_none() {
                            "sidecar not running (enable remote and restart)"
                        } else if app.remote.client_connected {
                            "client connected"
                        } else {
                            "listening, no client connected"
                        }
                    ));
                    // Listening but nothing has ever connected is, on
                    // Windows, almost always the host firewall dropping
                    // inbound on the tailnet interface — the phone just
                    // reports "instance offline" with no other clue.
                    #[cfg(windows)]
                    if app.remote.handle.is_some() && !app.remote.client_connected {
                        report.push_str(&format!(
                            "\nIf the app says \"instance offline\", Windows Firewall is \
                             probably dropping the connection. In an ADMIN PowerShell:\n  \
                             New-NetFirewallRule -DisplayName \"Gaviero Remote (tailnet)\" \
                             -Direction Inbound -Action Allow -Protocol TCP -LocalPort {} \
                             -RemoteAddress 100.64.0.0/10,fd7a:115c:a1e0::/48\n",
                            config.port
                        ));
                    }
                    match load_or_create_token(&config) {
                        Ok(token) => {
                            let url = pairing_url(&config);
                            let payload = pairing::qr_payload_json(
                                &url,
                                &token,
                                &config.workspace_display_name,
                            );
                            match render_qr(&payload) {
                                Ok(qr) => {
                                    report.push_str(&format!(
                                        "\nScan this gaviero-remote pairing code with the \
                                         Gaviero Remote app:\n\n{qr}\n{url}\n\
                                         (Token shown only in this code. /remote hide clears it; \
                                         /remote rotate invalidates it.)"
                                    ));
                                }
                                Err(e) => report.push_str(&format!("\nQR: {e}")),
                            }
                        }
                        Err(e) => report.push_str(&format!("\nToken: {e}")),
                    }
                }
                Err(e) => {
                    report.push_str(&format!("Status: unavailable — {e}\n"));
                }
            }
            app.chat_state.add_system_message(&report);
        }
    }
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

    #[test]
    fn port_zero_is_rejected_with_an_actionable_message() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "port": 0 } }));
        let err = resolve_config(&ws).expect_err("port 0 must be refused");
        assert!(matches!(err, RemoteUnavailable::PortZero));
        let msg = err.to_string();
        assert!(msg.contains("ephemeral"), "{msg}");
    }

    #[test]
    fn absent_port_derives_a_stable_workspace_port() {
        let (dir, ws) = workspace_with(serde_json::json!({}));
        let a = resolve_config(&ws).unwrap().port;
        let b = resolve_config(&ws).unwrap().port;
        assert_eq!(a, b, "derived port is identical across restarts");
        assert_eq!(a, identity::derive_remote_port(dir.path()));
        assert!((49152..=65535).contains(&a));
    }

    #[test]
    fn explicit_port_overrides_the_derivation() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "port": 51234 } }));
        assert_eq!(resolve_config(&ws).unwrap().port, 51234);
    }

    #[test]
    fn availability_fails_closed_without_a_magic_dns_host() {
        let (_dir, ws) = workspace_with(serde_json::json!({ "remote": { "enabled": true } }));
        let config = resolve_config(&ws).unwrap();
        let err = check_availability(&config).expect_err("must refuse to claim availability");
        assert!(matches!(err, RemoteUnavailable::MissingMagicDnsHost));
    }

    #[test]
    fn availability_reports_a_certificate_hostname_mismatch() {
        let (dir, ws) = workspace_with(serde_json::json!({
            "remote": { "enabled": true, "magicDnsHost": "wrong.tailnet.ts.net" }
        }));
        let tls_dir = dir.path().join(".gaviero/remote/tls");
        std::fs::create_dir_all(&tls_dir).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["host.tailnet.ts.net".to_string()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        std::fs::write(tls_dir.join("cert.pem"), cert.pem()).unwrap();
        std::fs::write(tls_dir.join("key.pem"), key.serialize_pem()).unwrap();

        let config = resolve_config(&ws).unwrap();
        let err = check_availability(&config).expect_err("hostname mismatch must fail");
        match err {
            RemoteUnavailable::CertHostMismatch { host } => {
                assert_eq!(host, "wrong.tailnet.ts.net");
            }
            other => panic!("expected CertHostMismatch, got {other:?}"),
        }
    }

    #[test]
    fn token_is_created_once_and_reread_stably() {
        let (_dir, ws) = workspace_with(serde_json::json!({}));
        let config = resolve_config(&ws).unwrap();
        let first = load_or_create_token(&config).unwrap();
        let second = load_or_create_token(&config).unwrap();
        assert_eq!(first, second, "token persists across reads");
        assert_eq!(first.len(), 64);
        let rotated = rotate_token(&config).unwrap();
        assert_ne!(rotated, first, "rotation replaces the token");
        assert_eq!(load_or_create_token(&config).unwrap(), rotated);
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
    }

    #[test]
    fn qr_renders_and_encodes_the_payload() {
        let payload = pairing::qr_payload_json(
            "wss://host.tailnet.ts.net:50123/v1/ws",
            &pairing::generate_token(),
            "gaviero",
        );
        let qr = render_qr(&payload).expect("payload fits in a QR code");
        assert!(qr.lines().count() > 10);
        let width = qr.lines().next().unwrap().chars().count();
        assert!(qr.lines().all(|l| l.chars().count() == width), "square");
    }

    /// The realistic worst case: a 64-hex token, a long MagicDNS hostname,
    /// and a long workspace name must still encode, and the rendering must
    /// stay inside a normal terminal width (the QR is useless if it wraps).
    #[test]
    fn worst_case_payload_encodes_and_fits_a_terminal() {
        let payload = pairing::qr_payload_json(
            "wss://very-long-machine-name.tail9f2c81.ts.net:65535/v1/ws",
            &pairing::generate_token(),
            "a-fairly-long-workspace-display-name",
        );
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
        let payload = pairing::qr_payload_json(
            "wss://host.tailnet.ts.net:50123/v1/ws",
            &pairing::generate_token(),
            "gaviero",
        );
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
    /// path `/remote` uses.
    #[test]
    fn rendered_pairing_payload_carries_what_the_app_validates() {
        let (_dir, ws) = workspace_with(serde_json::json!({
            "remote": { "magicDnsHost": "host.tailnet.ts.net", "port": 50123 }
        }));
        let config = resolve_config(&ws).unwrap();
        let token = load_or_create_token(&config).unwrap();
        let payload = pairing::qr_payload_json(
            &pairing_url(&config),
            &token,
            &config.workspace_display_name,
        );
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert_eq!(v["kind"], "gaviero-remote");
        assert_eq!(v["protocol_major"], 1);
        assert_eq!(v["url"], "wss://host.tailnet.ts.net:50123/v1/ws");
        assert_eq!(v["token"], token);
        render_qr(&payload).expect("the /remote payload encodes");
    }
}

#[cfg(test)]
mod render_preview {
    /// Prints the actual pairing QR. Run explicitly:
    /// `cargo test -p gaviero-tui preview_pairing_qr -- --ignored --nocapture`
    #[test]
    #[ignore = "visual check: prints a scannable QR to stdout"]
    fn preview_pairing_qr() {
        let payload = gaviero_remote::pairing::qr_payload_json(
            "wss://host.tailnet.ts.net:50123/v1/ws",
            &gaviero_remote::pairing::generate_token(),
            "gaviero",
        );
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

    /// Prints what `/remote` would report for a workspace right now.
    /// Under `cargo test` the cwd is the crate dir, so pass the workspace
    /// root explicitly:
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
                println!("port:      {} (derived)", config.port);
                println!(
                    "workspace: {} ({})",
                    config.workspace_display_name, config.workspace_id
                );
                println!("cert path: {}", config.cert_path.display());
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
