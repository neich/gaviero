//! Machine instance registry (Plan C §2.3): heartbeat files under
//! `~/.gaviero/remote/instances/<instance_id>.json`, and the reader used by
//! `GET /v1/instances`. Never carries a token, an absolute path, or
//! conversation content (invariant 14).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::dto::{InstanceDirectory, InstanceInfo};
use crate::version::PROTOCOL_VERSION;

/// How long after the last heartbeat write an entry is treated as stale.
/// Stale files are skipped and deleted best-effort; no pid probing.
pub const STALE_AFTER: Duration = Duration::from_secs(90);

/// Heartbeat cadence the hub writes at.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Directory-port leader election retry.
pub const DIRECTORY_RETRY_INTERVAL: Duration = Duration::from_secs(10);

/// `GET /v1/instances` cap per listener (Plan C §5.2).
pub const DIRECTORY_RATE_PER_SECOND: u32 = 10;

/// Passed into [`crate::server::RemoteServerConfig`]. `entry.client_connected`
/// is overwritten on each write from the hub's live-client flag.
#[derive(Clone, Debug)]
pub struct RegistryConfig {
    pub dir: PathBuf,
    pub entry: InstanceInfo,
}

fn entry_path(dir: &Path, instance_id: &str) -> PathBuf {
    dir.join(format!("{}.json", sanitize_id(instance_id)))
}

fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Atomic write (`*.json.tmp` + rename) of one heartbeat entry.
pub fn write_entry(dir: &Path, entry: &InstanceInfo) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let dest = entry_path(dir, &entry.instance_id);
    let tmp = dest.with_extension("json.tmp");
    let body = serde_json::to_vec_pretty(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(&tmp, body)?;
    fs::rename(&tmp, &dest)?;
    Ok(())
}

/// Best-effort delete of this instance's heartbeat file.
pub fn delete_entry(dir: &Path, instance_id: &str) {
    let path = entry_path(dir, instance_id);
    let _ = fs::remove_file(path);
}

/// RFC 3339 UTC, second precision, no subseconds — enough for `started_at`
/// / `generated_at`. Kept in this crate so `gaviero-remote` does not grow a
/// chrono dependency.
pub fn utc_now_rfc3339() -> String {
    format_system_time(SystemTime::now())
}

pub fn format_system_time(t: SystemTime) -> String {
    let secs = t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let (y, m, d, hh, mm, ss) = civil_utc(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant's `civil_from_days` (public domain).
fn civil_utc(unix: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = unix.div_euclid(86400);
    let rem = unix.rem_euclid(86400) as u32;
    let hh = rem / 3600;
    let mm = (rem % 3600) / 60;
    let ss = rem % 60;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, hh, mm, ss)
}

/// Read every heartbeat file in `dir`. Entries whose mtime is older than
/// `stale_after` are omitted and deleted best-effort. Unreadable files are
/// skipped. Never panics.
pub fn read_directory(dir: &Path, host: &str, stale_after: Duration) -> InstanceDirectory {
    let mut instances = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return empty_directory(host);
    };
    let now = SystemTime::now();
    for ent in entries.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(meta) = ent.metadata() else { continue };
        let stale = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_some_and(|age| age > stale_after);
        if stale {
            let _ = fs::remove_file(&path);
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<InstanceInfo>(&text) {
            Ok(info) => instances.push(info),
            Err(_) => continue,
        }
    }
    instances.sort_by(|a, b| a.workspace.display_name.cmp(&b.workspace.display_name));
    InstanceDirectory {
        protocol_version: PROTOCOL_VERSION,
        host: host.to_string(),
        generated_at: utc_now_rfc3339(),
        instances,
    }
}

fn empty_directory(host: &str) -> InstanceDirectory {
    InstanceDirectory {
        protocol_version: PROTOCOL_VERSION,
        host: host.to_string(),
        generated_at: utc_now_rfc3339(),
        instances: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::WorkspaceInfo;

    fn sample(id: &str, name: &str, connected: bool) -> InstanceInfo {
        InstanceInfo {
            instance_id: id.into(),
            workspace: WorkspaceInfo {
                id: format!("ws-{id}"),
                display_name: name.into(),
            },
            url: format!("wss://host.tailnet.ts.net:52093{}", crate::WS_PATH),
            port: 52093,
            tui_version: "0.1.0".into(),
            started_at: "2026-09-09T12:00:00Z".into(),
            client_connected: connected,
        }
    }

    #[test]
    fn write_then_read_round_trips_and_omits_stale() {
        let dir = tempfile::tempdir().unwrap();
        write_entry(dir.path(), &sample("aaaa", "alpha", false)).unwrap();
        write_entry(dir.path(), &sample("bbbb", "beta", true)).unwrap();
        let listed = read_directory(dir.path(), "host.tailnet.ts.net", STALE_AFTER);
        assert_eq!(listed.host, "host.tailnet.ts.net");
        assert_eq!(listed.instances.len(), 2);
        assert!(listed.instances.iter().any(|i| i.client_connected));

        let stale_path = dir.path().join("aaaa.json");
        let f = fs::File::options().write(true).open(&stale_path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(120))
            .unwrap();
        drop(f);

        let listed = read_directory(dir.path(), "host.tailnet.ts.net", STALE_AFTER);
        assert_eq!(listed.instances.len(), 1);
        assert_eq!(listed.instances[0].instance_id, "bbbb");
        assert!(!stale_path.exists(), "stale entry must be deleted");
    }

    #[test]
    fn unreadable_entry_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("garbage.json"), "not-json").unwrap();
        write_entry(dir.path(), &sample("cccc", "gamma", false)).unwrap();
        let listed = read_directory(dir.path(), "h", STALE_AFTER);
        assert_eq!(listed.instances.len(), 1);
        assert_eq!(listed.instances[0].instance_id, "cccc");
    }

    #[test]
    fn missing_dir_is_an_empty_directory() {
        let listed = read_directory(Path::new("/no/such/gaviero-registry-dir"), "h", STALE_AFTER);
        assert!(listed.instances.is_empty());
    }

    #[test]
    fn shutdown_deletes_the_entry() {
        let dir = tempfile::tempdir().unwrap();
        write_entry(dir.path(), &sample("dddd", "delta", false)).unwrap();
        assert!(dir.path().join("dddd.json").exists());
        delete_entry(dir.path(), "dddd");
        assert!(!dir.path().join("dddd.json").exists());
    }
}
