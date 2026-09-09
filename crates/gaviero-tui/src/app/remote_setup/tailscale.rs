//! Tailscale CLI helpers for zero-config remote (Plan C V1 §2.2):
//! MagicDNS host detection from `tailscale status --json` and certificate
//! provisioning with `tailscale cert`. Every call is a bounded subprocess
//! with an explicit argument vector — never a shell — and is only ever
//! awaited from the background bootstrap task, never from the event loop.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// `tailscale status --json` is local and fast; 5 s is generous.
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
/// First issuance goes through the ACME flow and can take a while.
const CERT_TIMEOUT: Duration = Duration::from_secs(60);

/// Locate the CLI: `PATH` first, then the well-known install locations.
pub fn tailscale_binary() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["tailscale.exe", "tailscale"]
    } else {
        &["tailscale"]
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let fixed: &[&str] = &[
        r"C:\Program Files\Tailscale\tailscale.exe",
        "/usr/bin/tailscale",
        "/usr/local/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ];
    fixed
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Pull `Self.DNSName` out of `tailscale status --json`, without the
/// trailing dot the CLI prints (`neichtop.tail9b1d28.ts.net.`).
pub fn parse_status_json(json: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("tailscale status is not JSON: {e}"))?;
    let backend = v["BackendState"].as_str().unwrap_or("");
    if !backend.is_empty() && backend != "Running" {
        return Err(format!(
            "tailscale is not running (BackendState = {backend}) — start it and log in"
        ));
    }
    let name = v["Self"]["DNSName"]
        .as_str()
        .map(|s| s.trim().trim_end_matches('.').to_string())
        .unwrap_or_default();
    if name.is_empty() {
        return Err(
            "tailscale status has no Self.DNSName — is MagicDNS enabled on the tailnet?"
                .to_string(),
        );
    }
    Ok(name)
}

fn command(binary: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(binary);
    cmd.kill_on_drop(true);
    cmd.stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: a console child must not flash a window over
        // the TUI.
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

async fn run(binary: &Path, args: &[&str], timeout: Duration) -> Result<std::process::Output, String> {
    let mut cmd = command(binary);
    cmd.args(args);
    match tokio::time::timeout(timeout, cmd.output()).await {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(e)) => Err(format!("could not run {}: {e}", binary.display())),
        Err(_) => Err(format!(
            "{} {} timed out after {}s",
            binary.display(),
            args.join(" "),
            timeout.as_secs()
        )),
    }
}

fn stderr_tail(output: &std::process::Output) -> String {
    let text = String::from_utf8_lossy(&output.stderr);
    let text = text.trim();
    if text.is_empty() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        text.to_string()
    }
}

/// The MagicDNS name of this machine, e.g. `host.tailnet.ts.net`.
pub async fn detect_magic_dns_host() -> Result<String, String> {
    let Some(binary) = tailscale_binary() else {
        return Err("tailscale CLI not found — install Tailscale or set `remote.magicDnsHost` \
                    in .gaviero/settings.json"
            .to_string());
    };
    let output = run(&binary, &["status", "--json"], STATUS_TIMEOUT).await?;
    if !output.status.success() {
        return Err(format!(
            "`tailscale status --json` failed: {}",
            stderr_tail(&output)
        ));
    }
    parse_status_json(&String::from_utf8_lossy(&output.stdout))
}

/// `tailscale cert --cert-file <cert> --key-file <key> <host>`. Requires
/// HTTPS certificates to be enabled on the tailnet; the CLI's own message
/// (which names the admin console) is returned verbatim on failure.
pub async fn provision_cert(host: &str, cert_path: &Path, key_path: &Path) -> Result<(), String> {
    let Some(binary) = tailscale_binary() else {
        return Err("tailscale CLI not found — cannot issue a certificate".to_string());
    };
    if let Some(dir) = cert_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let cert = cert_path.to_string_lossy().to_string();
    let key = key_path.to_string_lossy().to_string();
    let output = run(
        &binary,
        &["cert", "--cert-file", &cert, "--key-file", &key, host],
        CERT_TIMEOUT,
    )
    .await?;
    if !output.status.success() {
        let detail = stderr_tail(&output);
        let hint = if detail.to_ascii_lowercase().contains("not enabled")
            || detail.to_ascii_lowercase().contains("https")
        {
            " — enable HTTPS certificates at https://login.tailscale.com/admin/dns"
        } else {
            ""
        };
        return Err(format!("`tailscale cert {host}` failed: {detail}{hint}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `tailscale status --json` on 2026-09-09.
    const STATUS: &str = r#"{
      "Version": "1.102.2-t6cac91817-g6ff0ddc72",
      "BackendState": "Running",
      "TailscaleIPs": ["100.80.174.80", "fd7a:115c:a1e0::f835:ae52"],
      "Self": {
        "HostName": "NEICHTOP",
        "DNSName": "neichtop.tail9b1d28.ts.net.",
        "OS": "windows",
        "Online": true
      }
    }"#;

    #[test]
    fn dns_name_is_extracted_without_the_trailing_dot() {
        assert_eq!(
            parse_status_json(STATUS).unwrap(),
            "neichtop.tail9b1d28.ts.net"
        );
    }

    #[test]
    fn missing_dns_name_is_an_actionable_error() {
        let err = parse_status_json(r#"{"BackendState":"Running","Self":{"HostName":"x"}}"#)
            .unwrap_err();
        assert!(err.contains("MagicDNS"), "{err}");
    }

    #[test]
    fn stopped_backend_is_reported() {
        let err = parse_status_json(
            r#"{"BackendState":"NeedsLogin","Self":{"DNSName":"h.t.ts.net."}}"#,
        )
        .unwrap_err();
        assert!(err.contains("NeedsLogin"), "{err}");
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse_status_json("{not json").is_err());
    }

    /// Runs the real CLI. `cargo test -p gaviero-tui detect_real_magic_dns -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs a logged-in Tailscale on this machine"]
    async fn detect_real_magic_dns() {
        match detect_magic_dns_host().await {
            Ok(host) => println!("MagicDNS host: {host}"),
            Err(e) => println!("detection failed: {e}"),
        }
    }
}
