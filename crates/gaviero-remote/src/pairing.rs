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

/// The QR payload (§3.4 — frozen 1.0 shape plus the optional 1.1 keys,
/// also in PROTOCOL.md). 1.0 apps read named keys only and ignore the rest.
#[derive(Serialize)]
pub struct QrPayload<'a> {
    pub kind: &'static str,
    pub url: &'a str,
    pub token: &'a str,
    pub workspace: &'a str,
    pub protocol_major: u16,
    /// 1.1: hex workspace identity, so the app can key instances before
    /// it has ever connected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<&'a str>,
    /// 1.1: MagicDNS host — the pairing is per machine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine: Option<&'a str>,
    /// 1.1: `https://<host>:<directoryPort>/v1/instances`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory_url: Option<&'a str>,
    /// `remote.tlsMode = "pinnedCa"`: the machine CA certificate as base64
    /// DER (PEM body without armor or line breaks — it keeps the QR small).
    /// The app trusts only this CA for the machine. Never a private key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls_ca: Option<&'a str>,
    /// Relay mode (`remote.relay.host`): the app dials this host on 443 and
    /// names the instance in SNI; `url`'s host is the route, never resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_host: Option<&'a str>,
    /// Relay mode: bearer secret for the relay's phone notification stream.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_secret: Option<&'a str>,
}

/// Everything `/remote` knows when it renders the QR.
#[derive(Clone, Copy, Debug, Default)]
pub struct QrPayloadInput<'a> {
    pub url: &'a str,
    pub token: &'a str,
    pub workspace: &'a str,
    pub workspace_id: Option<&'a str>,
    pub machine: Option<&'a str>,
    pub directory_url: Option<&'a str>,
    pub tls_ca: Option<&'a str>,
    pub relay_host: Option<&'a str>,
    pub relay_secret: Option<&'a str>,
}

pub fn qr_payload_json(input: &QrPayloadInput<'_>) -> String {
    serde_json::to_string(&QrPayload {
        kind: "gaviero-remote",
        url: input.url,
        token: input.token,
        workspace: input.workspace,
        protocol_major: crate::PROTOCOL_VERSION.major,
        workspace_id: input.workspace_id,
        machine: input.machine,
        directory_url: input.directory_url,
        tls_ca: input.tls_ca,
        relay_host: input.relay_host,
        relay_secret: input.relay_secret,
    })
    .expect("QR payload serializes")
}

// ── Pinned private CA (`remote.tlsMode = "pinnedCa"`) ───────────────
//
// For a relay that passes TLS through (nginx `stream` + `ssh -R`): the
// sidecar keeps terminating TLS, with a leaf issued by a per-machine CA the
// app pins from the QR. Nothing is logged in Certificate Transparency and
// nothing depends on Tailscale or a public CA.

/// Sub-directory of the machine `tls/` folder holding the pinned material.
pub const PINNED_DIR: &str = "pinned";
/// CA certificate followed by its private key. Owner-only.
const CA_FILE: &str = "ca.pem";
/// Leaf certificate followed by its private key — rustls reads both from it.
pub const LEAF_FILE: &str = "leaf.pem";
const CA_COMMON_NAME: &str = "gaviero remote CA";
const CA_VALID_DAYS: i64 = 3650;
const LEAF_VALID_DAYS: i64 = 397;
/// Re-issue a certificate this close to `notAfter`.
const RENEW_WITHIN_SECS: i64 = 30 * 24 * 3600;

/// The pinned material in force for one host.
#[derive(Debug, Clone)]
pub struct PinnedTls {
    /// CA certificate, base64 DER — the QR `tls_ca` value.
    pub ca_der_b64: String,
    /// Leaf certificate + key bundle (pass as both cert and key PEM).
    pub leaf_path: std::path::PathBuf,
    /// A CA or leaf was generated during this call.
    pub issued: bool,
}

/// Load or create the machine CA and a leaf for `host` under `dir`.
/// Idempotent: an existing CA is reused (re-pairing is only needed when the
/// CA itself changes); the leaf is re-issued when missing, not covering
/// `host`, within 30 days of expiry, or not issued by the current CA.
/// Concurrent first starts are safe: the CA is committed without clobbering
/// and the loser adopts the winner's CA.
pub fn ensure_pinned_ca(dir: &std::path::Path, host: &str) -> Result<PinnedTls, String> {
    ensure_pinned_ca_with(dir, host, &[])
}

/// [`ensure_pinned_ca`] with extra leaf SANs — the relay route wildcard
/// (`*.<relay host>`), which covers every `<wsid>-<machine>` route name on
/// this machine. The leaf is re-issued when any requested SAN is missing.
pub fn ensure_pinned_ca_with(
    dir: &std::path::Path,
    host: &str,
    extra_sans: &[String],
) -> Result<PinnedTls, String> {
    if host.trim().is_empty() {
        return Err("pinned CA needs `remote.magicDnsHost`".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let ca_path = dir.join(CA_FILE);
    let leaf_path = dir.join(LEAF_FILE);

    let mut issued = false;
    let (ca_cert_pem, ca_key) = match load_ca(&ca_path) {
        Some(ca) => ca,
        None => {
            issued = true;
            let replacing = ca_path.exists();
            let (cert_pem, key) = generate_ca()?;
            let bundle = format!("{cert_pem}{}", key.serialize_pem());
            if replacing {
                write_private_atomic(&ca_path, &bundle)?;
            } else if !commit_new(&ca_path, &bundle)? {
                // Another instance created the CA first: adopt it.
                issued = false;
            }
            load_ca(&ca_path).ok_or_else(|| {
                format!("{} was written but cannot be read back", ca_path.display())
            })?
        }
    };

    let ca_ski = key_ids(ca_cert_pem.as_bytes()).and_then(|(ski, _)| ski);
    let leaf_ok = std::fs::read(&leaf_path).ok().is_some_and(|pem| {
        let info_ok = inspect_cert(&pem, host)
            .is_ok_and(|i| i.covers_host && i.seconds_until_expiry > RENEW_WITHIN_SECS);
        let issued_by_ca = key_ids(&pem).and_then(|(_, aki)| aki).is_some_and(|aki| {
            ca_ski.as_ref().is_some_and(|ski| *ski == aki)
        });
        let names = san_names(&pem);
        let sans_ok = extra_sans.iter().all(|s| names.iter().any(|n| n == s));
        info_ok && sans_ok && issued_by_ca && has_private_key(&pem)
    });
    if !leaf_ok {
        issued = true;
        let bundle = generate_leaf(host, extra_sans, &ca_key)?;
        write_private_atomic(&leaf_path, &bundle)?;
    }

    Ok(PinnedTls {
        ca_der_b64: pem_body_b64(&ca_cert_pem),
        leaf_path,
        issued,
    })
}

/// The stored CA certificate as base64 DER (the QR `tls_ca` value), without
/// generating anything. `None` when `dir` holds no usable CA yet.
pub fn pinned_ca_b64(dir: &std::path::Path) -> Option<String> {
    load_ca(&dir.join(CA_FILE)).map(|(pem, _)| pem_body_b64(&pem))
}

/// Fixed CA parameters. Signing only reads the issuer's DN, key usages and
/// key-id method, so rebuilding these over the stored key reproduces the
/// stored CA as an issuer without parsing it back into rcgen.
fn ca_params() -> Result<rcgen::CertificateParams, String> {
    let mut params =
        rcgen::CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, CA_COMMON_NAME);
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::DigitalSignature,
    ];
    Ok(params)
}

fn generate_ca() -> Result<(String, rcgen::KeyPair), String> {
    let key = rcgen::KeyPair::generate().map_err(|e| format!("generating CA key: {e}"))?;
    let mut params = ca_params()?;
    set_validity(&mut params, CA_VALID_DAYS);
    let cert = params
        .self_signed(&key)
        .map_err(|e| format!("signing CA: {e}"))?;
    Ok((cert.pem(), key))
}

fn generate_leaf(
    host: &str,
    extra_sans: &[String],
    ca_key: &rcgen::KeyPair,
) -> Result<String, String> {
    let issuer = ca_params()?
        .self_signed(ca_key)
        .map_err(|e| format!("rebuilding CA issuer: {e}"))?;
    let key = rcgen::KeyPair::generate().map_err(|e| format!("generating leaf key: {e}"))?;
    let mut sans = vec![host.to_string()];
    sans.extend(extra_sans.iter().filter(|s| *s != host).cloned());
    let mut params = rcgen::CertificateParams::new(sans).map_err(|e| e.to_string())?;
    params.distinguished_name.push(rcgen::DnType::CommonName, host);
    params.is_ca = rcgen::IsCa::ExplicitNoCa;
    params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    params.use_authority_key_identifier_extension = true;
    set_validity(&mut params, LEAF_VALID_DAYS);
    let cert = params
        .signed_by(&key, &issuer, ca_key)
        .map_err(|e| format!("signing leaf: {e}"))?;
    Ok(format!("{}{}", cert.pem(), key.serialize_pem()))
}

/// The CA certificate PEM and key from the bundle, if both parse and the CA
/// is not within the renewal window.
fn load_ca(path: &std::path::Path) -> Option<(String, rcgen::KeyPair)> {
    let text = std::fs::read_to_string(path).ok()?;
    let cert_pem = pem_block(&text, "CERTIFICATE")?;
    let key = rcgen::KeyPair::from_pem(&pem_block(&text, "PRIVATE KEY")?).ok()?;
    let info = inspect_cert(cert_pem.as_bytes(), "").ok()?;
    (info.seconds_until_expiry > RENEW_WITHIN_SECS).then_some((cert_pem, key))
}

fn has_private_key(pem: &[u8]) -> bool {
    std::str::from_utf8(pem)
        .ok()
        .and_then(|t| pem_block(t, "PRIVATE KEY"))
        .is_some_and(|k| rcgen::KeyPair::from_pem(&k).is_ok())
}

/// The first `-----BEGIN <label>-----` … `-----END <label>-----` block.
fn pem_block(text: &str, label: &str) -> Option<String> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = text.find(&begin)?;
    let stop = text[start..].find(&end)? + start + end.len();
    Some(format!("{}\n", &text[start..stop]))
}

/// Base64 body of a PEM block: the DER, encoded, with armor and line
/// breaks removed.
fn pem_body_b64(pem: &str) -> String {
    pem.lines()
        .filter(|l| !l.starts_with("-----"))
        .map(str::trim)
        .collect()
}

/// DNS SANs of the first certificate in `pem` (empty when unparseable).
fn san_names(pem: &[u8]) -> Vec<String> {
    let Ok((_rest, block)) = x509_parser::pem::parse_x509_pem(pem) else {
        return Vec::new();
    };
    let Ok(cert) = block.parse_x509() else {
        return Vec::new();
    };
    let mut names = Vec::new();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let x509_parser::extensions::GeneralName::DNSName(dns) = name {
                names.push(dns.to_string());
            }
        }
    }
    names
}

/// (subject key id, authority key id) of the first certificate in `pem`.
fn key_ids(pem: &[u8]) -> Option<(Option<Vec<u8>>, Option<Vec<u8>>)> {
    let (_rest, block) = x509_parser::pem::parse_x509_pem(pem).ok()?;
    let cert = block.parse_x509().ok()?;
    let mut ski = None;
    let mut aki = None;
    for ext in cert.extensions() {
        match ext.parsed_extension() {
            x509_parser::extensions::ParsedExtension::SubjectKeyIdentifier(id) => {
                ski = Some(id.0.to_vec());
            }
            x509_parser::extensions::ParsedExtension::AuthorityKeyIdentifier(a) => {
                aki = a.key_identifier.as_ref().map(|id| id.0.to_vec());
            }
            _ => {}
        }
    }
    Some((ski, aki))
}

/// Valid from yesterday (clock skew) until `days` from today, UTC midnight.
fn set_validity(params: &mut rcgen::CertificateParams, days: i64) {
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64 / 86_400)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(today - 1);
    params.not_before = rcgen::date_time_ymd(y, m, d);
    let (y, m, d) = civil_from_days(today + days);
    params.not_after = rcgen::date_time_ymd(y, m, d);
}

/// Days since 1970-01-01 → (year, month, day), proleptic Gregorian
/// (H. Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Create `path` only if it does not exist. `Ok(false)` ⇒ someone else won.
fn commit_new(path: &std::path::Path, contents: &str) -> Result<bool, String> {
    let tmp = tmp_sibling(path);
    write_owner_only(&tmp, contents)?;
    let result = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(format!("committing {}: {e}", path.display())),
    }
}

/// Atomic replace, owner-only where supported.
fn write_private_atomic(path: &std::path::Path, contents: &str) -> Result<(), String> {
    let tmp = tmp_sibling(path);
    write_owner_only(&tmp, contents)?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replacing {}: {e}", path.display()))
}

fn tmp_sibling(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension(format!("tmp-{}", &generate_token()[..8]))
}

fn write_owner_only(path: &std::path::Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("writing {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
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
        let json = qr_payload_json(&QrPayloadInput {
            url: "wss://host.tail.ts.net:50123/v1/ws",
            token: "SECRET",
            workspace: "gaviero",
            ..Default::default()
        });
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["kind"], "gaviero-remote");
        assert_eq!(v["url"], "wss://host.tail.ts.net:50123/v1/ws");
        assert_eq!(v["token"], "SECRET");
        assert_eq!(v["workspace"], "gaviero");
        assert_eq!(v["protocol_major"], 1);
        // The 1.0 shape is exactly five keys: optional 1.1 keys are omitted,
        // never emitted as null.
        assert_eq!(v.as_object().unwrap().len(), 5);
    }

    #[test]
    fn qr_payload_carries_the_optional_1_1_keys_when_known() {
        let json = qr_payload_json(&QrPayloadInput {
            url: "wss://host.tail.ts.net:50123/v1/ws",
            token: "SECRET",
            workspace: "gaviero",
            workspace_id: Some("4b156f1de41da274"),
            machine: Some("host.tail.ts.net"),
            directory_url: Some("https://host.tail.ts.net:49151/v1/instances"),
            tls_ca: None,
            relay_host: None,
            relay_secret: None,
        });
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["workspace_id"], "4b156f1de41da274");
        assert_eq!(v["machine"], "host.tail.ts.net");
        assert_eq!(
            v["directory_url"],
            "https://host.tail.ts.net:49151/v1/instances"
        );
        assert_eq!(v["protocol_major"], 1, "still major 1 — the bump is minor");
    }

    #[test]
    fn tailscale_ranges_are_recognized() {
        assert!(is_tailscale_addr(&"100.101.102.103".parse().unwrap()));
        assert!(!is_tailscale_addr(&"100.10.0.1".parse().unwrap()));
        assert!(!is_tailscale_addr(&"192.168.1.10".parse().unwrap()));
        assert!(is_tailscale_addr(&"fd7a:115c:a1e0::1234".parse().unwrap()));
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
    fn qr_payload_carries_tls_ca_only_when_given() {
        let with = qr_payload_json(&QrPayloadInput {
            url: "wss://desk.example.com:50101/v1/ws",
            token: "SECRET",
            workspace: "gaviero",
            tls_ca: Some("MIIBkTCCATe"),
            ..Default::default()
        });
        let v: serde_json::Value = serde_json::from_str(&with).unwrap();
        assert_eq!(v["tls_ca"], "MIIBkTCCATe");
        assert!(!with.contains("PRIVATE"), "never a key in the QR");
        let without = qr_payload_json(&QrPayloadInput {
            url: "wss://desk.example.com:50101/v1/ws",
            token: "SECRET",
            workspace: "gaviero",
            ..Default::default()
        });
        assert!(!without.contains("tls_ca"));
    }

    #[test]
    fn civil_dates_are_correct() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn pinned_ca_is_generated_once_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let first = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        assert!(first.issued);
        let leaf = std::fs::read(&first.leaf_path).unwrap();
        let info = inspect_cert(&leaf, "desk.example.com").unwrap();
        assert!(info.covers_host);
        assert!(info.seconds_until_expiry > 300 * 24 * 3600, "~397-day leaf");
        assert!(has_private_key(&leaf), "leaf bundle carries its key");

        let second = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        assert!(!second.issued, "nothing re-issued on a second start");
        assert_eq!(first.ca_der_b64, second.ca_der_b64);
        assert_eq!(std::fs::read(&second.leaf_path).unwrap(), leaf);

        // The QR value is a bare base64 DER body, never a key.
        assert!(!first.ca_der_b64.contains("-----"));
        assert!(!first.ca_der_b64.contains('\n'));
        let ca_text = std::fs::read_to_string(dir.path().join(CA_FILE)).unwrap();
        assert_eq!(
            pem_body_b64(&pem_block(&ca_text, "CERTIFICATE").unwrap()),
            first.ca_der_b64
        );
    }

    #[test]
    fn host_change_reissues_the_leaf_under_the_same_ca() {
        let dir = tempfile::tempdir().unwrap();
        let a = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        let b = ensure_pinned_ca(dir.path(), "vps.example.com").unwrap();
        assert!(b.issued);
        assert_eq!(a.ca_der_b64, b.ca_der_b64, "re-pairing not needed");
        let leaf = std::fs::read(&b.leaf_path).unwrap();
        assert!(inspect_cert(&leaf, "vps.example.com").unwrap().covers_host);
        assert!(!inspect_cert(&leaf, "desk.example.com").unwrap().covers_host);
    }

    #[test]
    fn leaf_is_issued_by_the_stored_ca() {
        let dir = tempfile::tempdir().unwrap();
        let pinned = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        let ca_text = std::fs::read_to_string(dir.path().join(CA_FILE)).unwrap();
        let (ca_ski, _) = key_ids(ca_text.as_bytes()).unwrap();
        let (_, leaf_aki) = key_ids(&std::fs::read(&pinned.leaf_path).unwrap()).unwrap();
        assert!(ca_ski.is_some());
        assert_eq!(ca_ski, leaf_aki);

        // A leaf from a different CA is replaced on the next start.
        let other = tempfile::tempdir().unwrap();
        let foreign = ensure_pinned_ca(other.path(), "desk.example.com").unwrap();
        std::fs::copy(&foreign.leaf_path, &pinned.leaf_path).unwrap();
        let again = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        assert!(again.issued, "foreign leaf must be re-issued");
        let (_, aki) = key_ids(&std::fs::read(&again.leaf_path).unwrap()).unwrap();
        assert_eq!(aki, ca_ski);
    }

    #[test]
    fn a_concurrently_created_ca_is_adopted() {
        let dir = tempfile::tempdir().unwrap();
        let winner = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        let (cert_pem, key) = generate_ca().unwrap();
        let bundle = format!("{cert_pem}{}", key.serialize_pem());
        assert!(
            !commit_new(&dir.path().join(CA_FILE), &bundle).unwrap(),
            "an existing CA is never clobbered"
        );
        let after = ensure_pinned_ca(dir.path(), "desk.example.com").unwrap();
        assert_eq!(after.ca_der_b64, winner.ca_der_b64);
    }

    #[test]
    fn relay_wildcard_san_is_added_and_triggers_reissue() {
        let dir = tempfile::tempdir().unwrap();
        let plain = ensure_pinned_ca(dir.path(), "desk.gaviero.example.com").unwrap();
        let wildcard = vec!["*.gaviero.example.com".to_string()];
        let relay = ensure_pinned_ca_with(dir.path(), "desk.gaviero.example.com", &wildcard).unwrap();
        assert!(relay.issued, "missing SAN forces a new leaf");
        assert_eq!(plain.ca_der_b64, relay.ca_der_b64, "same CA: no re-pairing");
        let leaf = std::fs::read(&relay.leaf_path).unwrap();
        assert!(inspect_cert(&leaf, "abc123-desk.gaviero.example.com").unwrap().covers_host);
        assert!(inspect_cert(&leaf, "desk.gaviero.example.com").unwrap().covers_host);
        let again = ensure_pinned_ca_with(dir.path(), "desk.gaviero.example.com", &wildcard).unwrap();
        assert!(!again.issued);
    }

    #[test]
    fn pinned_ca_requires_a_host() {
        let dir = tempfile::tempdir().unwrap();
        assert!(ensure_pinned_ca(dir.path(), " ").is_err());
    }

    #[test]
    fn wildcard_san_matches_one_label() {
        assert!(dns_name_matches("*.tail.ts.net", "host.tail.ts.net"));
        assert!(!dns_name_matches("*.tail.ts.net", "a.b.tail.ts.net"));
        assert!(!dns_name_matches("*.tail.ts.net", "tail.ts.net"));
    }
}
