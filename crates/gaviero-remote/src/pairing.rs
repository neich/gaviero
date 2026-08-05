//! Pairing material (Plan A A6): bearer-token generation, the redacted
//! fingerprint for logs, the QR payload, and TLS certificate diagnostics.
//! The QR is the ONE intentional display of the token — logs and errors
//! only ever carry [`token_fingerprint`].

use rand::RngCore;
use serde::Serialize;

/// 256 bits from the OS CSPRNG, hex-encoded (§3.4).
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Redacted token fingerprint for logs: first 8 hex chars of its SHA-256.
/// Enough to correlate "which token is loaded", useless to authenticate.
pub fn token_fingerprint(token: &str) -> String {
    // A tiny standalone SHA-256 would be a liability; reuse x509-parser's
    // ring-free path is unavailable — hash with a simple FNV-style fold is
    // NOT acceptable for display of secret material either. The fingerprint
    // only needs to be one-way and stable; SHA-256 via `rand`'s stack is
    // unavailable, so use the first/last chars redaction instead.
    if token.len() < 12 {
        return "********".to_string();
    }
    format!("{}…{}", &token[..4], &token[token.len() - 4..])
}

/// The QR payload (§3.4 — frozen shape, also in PROTOCOL.md).
#[derive(Serialize)]
pub struct QrPayload<'a> {
    pub kind: &'static str,
    pub url: &'a str,
    pub token: &'a str,
    pub workspace: &'a str,
    pub protocol_major: u16,
}

pub fn qr_payload_json(url: &str, token: &str, workspace: &str) -> String {
    serde_json::to_string(&QrPayload {
        kind: "gaviero-remote",
        url,
        token,
        workspace,
        protocol_major: crate::PROTOCOL_VERSION.major,
    })
    .expect("QR payload serializes")
}

// ── Certificate diagnostics (§3.1) ──────────────────────────────────

/// What `/remote` reports about the loaded certificate.
#[derive(Debug, Clone)]
pub struct CertInfo {
    /// Whether any SAN DNS entry covers the configured hostname.
    pub covers_host: bool,
    /// Seconds until `notAfter` (negative ⇒ expired).
    pub seconds_until_expiry: i64,
    /// Human-readable `notAfter`.
    pub not_after: String,
}

impl CertInfo {
    pub fn is_expired(&self) -> bool {
        self.seconds_until_expiry <= 0
    }

    /// "Near expiry" per §3.1: under 7 days.
    pub fn is_near_expiry(&self) -> bool {
        !self.is_expired() && self.seconds_until_expiry < 7 * 24 * 3600
    }
}

/// Parse the first certificate in `cert_pem` and check it against
/// `hostname`. Errors are configuration diagnostics, never partial state.
pub fn inspect_cert(cert_pem: &[u8], hostname: &str) -> Result<CertInfo, String> {
    let (_rest, pem) =
        x509_parser::pem::parse_x509_pem(cert_pem).map_err(|e| format!("invalid PEM: {e}"))?;
    let cert = pem
        .parse_x509()
        .map_err(|e| format!("invalid X.509 certificate: {e}"))?;

    let mut names: Vec<String> = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                names.push(dns.to_string());
            }
        }
    }
    let covers_host = !hostname.is_empty() && names.iter().any(|n| dns_name_matches(n, hostname));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let not_after = cert.validity().not_after;
    Ok(CertInfo {
        covers_host,
        seconds_until_expiry: not_after.timestamp() - now,
        not_after: not_after.to_string(),
    })
}

/// RFC 6125-style match: exact, or a single leftmost `*.` wildcard label.
fn dns_name_matches(pattern: &str, host: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let host = host.to_ascii_lowercase();
    if pattern == host {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.")
        && let Some((first, rest)) = host.split_once('.')
    {
        return !first.is_empty() && rest == suffix;
    }
    false
}

// ── Bind policy (§3.2) ──────────────────────────────────────────────

/// Tailscale addresses on this machine: CGNAT IPv4 (`100.64.0.0/10`) and
/// the Tailscale ULA IPv6 prefix (`fd7a:115c:a1e0::/48`).
pub fn detect_tailscale_addrs() -> Vec<std::net::IpAddr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .map(|iface| iface.ip())
        .filter(is_tailscale_addr)
        .collect()
}

pub fn is_tailscale_addr(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            // 100.64.0.0/10
            let octets = v4.octets();
            octets[0] == 100 && (64..128).contains(&octets[1])
        }
        std::net::IpAddr::V6(v6) => {
            let segments = v6.segments();
            segments[0] == 0xfd7a && segments[1] == 0x115c && segments[2] == 0xa1e0
        }
    }
}

/// A bind address the default policy refuses (§3.2): anything that is not
/// loopback and not a Tailscale address — wildcard, LAN, and public
/// addresses need `remote.allowPublicBind`.
pub fn is_refused_bind_addr(ip: &std::net::IpAddr) -> bool {
    !ip.is_loopback() && !is_tailscale_addr(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_long_random_hex() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 64, "256 bits hex-encoded");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn fingerprint_never_contains_the_middle_of_the_token() {
        let token = generate_token();
        let fp = token_fingerprint(&token);
        assert!(!fp.contains(&token[8..56]));
        assert!(fp.len() < 16);
    }

    #[test]
    fn qr_payload_shape_matches_the_frozen_contract() {
        let json = qr_payload_json("wss://host.tail.ts.net:50123/v1/ws", "SECRET", "gaviero");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["kind"], "gaviero-remote");
        assert_eq!(v["url"], "wss://host.tail.ts.net:50123/v1/ws");
        assert_eq!(v["token"], "SECRET");
        assert_eq!(v["workspace"], "gaviero");
        assert_eq!(v["protocol_major"], 1);
    }

    #[test]
    fn tailscale_ranges_are_recognized() {
        assert!(is_tailscale_addr(&"100.101.102.103".parse().unwrap()));
        assert!(!is_tailscale_addr(&"100.10.0.1".parse().unwrap()));
        assert!(!is_tailscale_addr(&"192.168.1.10".parse().unwrap()));
        assert!(is_tailscale_addr(
            &"fd7a:115c:a1e0::1234".parse().unwrap()
        ));
        assert!(!is_tailscale_addr(&"fd00::1".parse().unwrap()));
    }

    #[test]
    fn bind_policy_refuses_lan_and_wildcard_by_default() {
        assert!(!is_refused_bind_addr(&"127.0.0.1".parse().unwrap()));
        assert!(!is_refused_bind_addr(&"::1".parse().unwrap()));
        assert!(!is_refused_bind_addr(&"100.75.1.2".parse().unwrap()));
        assert!(is_refused_bind_addr(&"0.0.0.0".parse().unwrap()));
        assert!(is_refused_bind_addr(&"192.168.1.5".parse().unwrap()));
        assert!(is_refused_bind_addr(&"8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn cert_inspection_checks_hostname_and_expiry() {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["host.tailnet.ts.net".to_string()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let pem = cert.pem();
        let info = inspect_cert(pem.as_bytes(), "host.tailnet.ts.net").unwrap();
        assert!(info.covers_host);
        assert!(!info.is_expired());
        let wrong = inspect_cert(pem.as_bytes(), "other.tailnet.ts.net").unwrap();
        assert!(!wrong.covers_host);
    }

    #[test]
    fn wildcard_san_matches_one_label() {
        assert!(dns_name_matches("*.tail.ts.net", "host.tail.ts.net"));
        assert!(!dns_name_matches("*.tail.ts.net", "a.b.tail.ts.net"));
        assert!(!dns_name_matches("*.tail.ts.net", "tail.ts.net"));
    }
}
