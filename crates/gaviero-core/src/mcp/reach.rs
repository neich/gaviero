//! Persistent MCP reach record and spawn-time nesting policy.
//!
//! `<workspace>/.gaviero/mcp_reach.json` is the measurement (plan invariant 2).
//! [`ReachPolicy::for_workspace`] is the cheap per-spawn reader: verified +
//! fresh + matching CLI version → [`NestingPolicy::Allowed`]; a failed probe
//! → [`NestingPolicy::Blocked`]; missing/stale/skipped/unsupported →
//! [`NestingPolicy::Unknown`] (P0.4 leaves argv alone).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::reach_probe::{
    ProviderReachResult, ReachReport, ReachVerdict, capture_cli_version, provider_cli_bin,
};
use crate::workspace::settings;

pub const REACH_RECORD_VERSION: u32 = 1;
pub const REACH_FILENAME: &str = "mcp_reach.json";

/// On-disk reach record. Same body as [`ReachReport`] plus a format version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReachRecord {
    pub v: u32,
    pub workspace_id: String,
    pub probed_at: DateTime<Utc>,
    pub providers: BTreeMap<String, ProviderReachResult>,
}

impl From<ReachReport> for ReachRecord {
    fn from(report: ReachReport) -> Self {
        Self {
            v: REACH_RECORD_VERSION,
            workspace_id: report.workspace_id,
            probed_at: report.probed_at,
            providers: report.providers,
        }
    }
}

impl ReachRecord {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".gaviero").join(REACH_FILENAME)
    }
}

/// tmp + rename persistence for [`ReachRecord`].
pub struct ReachStore;

impl ReachStore {
    pub fn load(root: &Path) -> Result<Option<ReachRecord>> {
        let path = ReachRecord::path(root);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let rec: ReachRecord = serde_json::from_str(&text)
                    .with_context(|| format!("parsing {}", path.display()))?;
                Ok(Some(rec))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(root: &Path, record: &ReachRecord) -> Result<()> {
        let path = ReachRecord::path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(record).context("serialising mcp_reach.json")?;
        write_atomic(&path, &json)
    }
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().context("mcp_reach.json has no parent")?;
    let mut tmp_name = path
        .file_name()
        .context("mcp_reach.json has no file name")?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| {
        format!("renaming {} onto {}", tmp.display(), path.display())
    })?;
    Ok(())
}

/// Per-provider nesting decision consumed at spawn (P0.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestingPolicy {
    Allowed,
    Blocked(String),
    Unknown,
}

/// Workspace-wide reach policy. Cheap: one JSON read, no cache.
#[derive(Debug, Clone)]
pub struct ReachPolicy {
    pub enforce: bool,
    pub max_age_days: u32,
    providers: BTreeMap<String, NestingPolicy>,
}

impl ReachPolicy {
    pub fn for_workspace(root: &Path) -> Self {
        let now = Utc::now();
        let versions: BTreeMap<String, Option<String>> = ["claude", "codex", "cursor", "dsh"]
            .into_iter()
            .map(|p| (p.to_string(), capture_cli_version(provider_cli_bin(p))))
            .collect();
        Self::for_workspace_at(root, now, &versions)
    }

    /// Test seam: inject `now` and current CLI versions.
    pub fn for_workspace_at(
        root: &Path,
        now: DateTime<Utc>,
        current_versions: &BTreeMap<String, Option<String>>,
    ) -> Self {
        let (enforce, max_age_days) = read_reach_settings(root);
        let record = ReachStore::load(root).ok().flatten();
        let mut providers = BTreeMap::new();
        for name in ["claude", "codex", "cursor", "dsh"] {
            let policy = match record.as_ref() {
                None => NestingPolicy::Unknown,
                Some(rec) => policy_for_provider(
                    rec,
                    name,
                    now,
                    max_age_days,
                    current_versions.get(name).cloned().flatten(),
                ),
            };
            providers.insert(name.to_string(), policy);
        }
        Self {
            enforce,
            max_age_days,
            providers,
        }
    }

    pub fn for_provider(&self, name: &str) -> NestingPolicy {
        self.providers
            .get(name)
            .cloned()
            .unwrap_or(NestingPolicy::Unknown)
    }
}

fn policy_for_provider(
    rec: &ReachRecord,
    name: &str,
    now: DateTime<Utc>,
    max_age_days: u32,
    current_version: Option<String>,
) -> NestingPolicy {
    let Some(row) = rec.providers.get(name) else {
        return NestingPolicy::Unknown;
    };
    if is_stale_age(rec.probed_at, now, max_age_days) {
        return NestingPolicy::Unknown;
    }
    if is_stale_version(row.cli_version.as_deref(), current_version.as_deref()) {
        return NestingPolicy::Unknown;
    }
    match row.verdict {
        ReachVerdict::Verified => NestingPolicy::Allowed,
        ReachVerdict::NestedFailed => NestingPolicy::Blocked(format!(
            "{name} nested MCP ping failed; run gaviero-cli --mcp-reach-probe"
        )),
        ReachVerdict::TopLevelFailed => NestingPolicy::Blocked(format!(
            "{name} top-level MCP ping failed; run gaviero-cli --mcp-reach-probe"
        )),
        ReachVerdict::Unsupported | ReachVerdict::Skipped => NestingPolicy::Unknown,
    }
}

pub fn is_stale_age(probed_at: DateTime<Utc>, now: DateTime<Utc>, max_age_days: u32) -> bool {
    now.signed_duration_since(probed_at) > Duration::days(max_age_days as i64)
}

pub fn is_stale_version(recorded: Option<&str>, current: Option<&str>) -> bool {
    match (recorded, current) {
        (Some(a), Some(b)) => a != b,
        // No current version to compare (CLI absent) — do not treat as a
        // verified match, but also do not invent a version-mismatch block.
        // Unknown is the spawn-safe default.
        (Some(_), None) => true,
        (None, Some(_)) => true,
        (None, None) => false,
    }
}

fn read_reach_settings(root: &Path) -> (bool, u32) {
    let path = root.join(".gaviero").join("settings.json");
    let doc = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let enforce = doc
        .as_ref()
        .and_then(|d| dot_get(d, settings::MCP_REACH_ENFORCE))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let max_age_days = doc
        .as_ref()
        .and_then(|d| dot_get(d, settings::MCP_REACH_MAX_AGE_DAYS))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(30)
        .min(u32::MAX as u64) as u32;
    (enforce, max_age_days)
}

fn dot_get<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    if let Some(v) = value.get(key) {
        return Some(v);
    }
    let parts: Vec<&str> = key.splitn(2, '.').collect();
    if parts.len() == 2 {
        return dot_get(value.get(parts[0])?, parts[1]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::reach_probe::ReachVerdict;

    fn row(verdict: ReachVerdict, version: &str) -> ProviderReachResult {
        ProviderReachResult {
            cli_version: Some(version.into()),
            transport: "stdio".into(),
            top_level: matches!(
                verdict,
                ReachVerdict::Verified | ReachVerdict::NestedFailed
            ),
            nested: verdict == ReachVerdict::Verified,
            nested_depth: if verdict == ReachVerdict::Verified { 1 } else { 0 },
            explicit_ref_required: false,
            verdict,
        }
    }

    fn record_at(t: DateTime<Utc>, claude: ProviderReachResult) -> ReachRecord {
        let mut providers = BTreeMap::new();
        providers.insert("claude".into(), claude);
        ReachRecord {
            v: 1,
            workspace_id: "abc".into(),
            probed_at: t,
            providers,
        }
    }

    fn versions(v: &str) -> BTreeMap<String, Option<String>> {
        BTreeMap::from([
            ("claude".into(), Some(v.into())),
            ("codex".into(), None),
            ("cursor".into(), None),
            ("dsh".into(), None),
        ])
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::TempDir::new().unwrap();
        let rec = record_at(Utc::now(), row(ReachVerdict::Verified, "2.1.269"));
        ReachStore::save(dir.path(), &rec).unwrap();
        let loaded = ReachStore::load(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.v, 1);
        assert_eq!(loaded.workspace_id, "abc");
        assert_eq!(
            loaded.providers["claude"].verdict,
            ReachVerdict::Verified
        );
        assert!(!ReachRecord::path(dir.path())
            .with_file_name("mcp_reach.json.tmp")
            .exists());
    }

    #[test]
    fn missing_file_is_unknown() {
        let dir = tempfile::TempDir::new().unwrap();
        let policy = ReachPolicy::for_workspace_at(dir.path(), Utc::now(), &versions("2.1.269"));
        assert_eq!(policy.for_provider("claude"), NestingPolicy::Unknown);
        assert!(policy.enforce);
        assert_eq!(policy.max_age_days, 30);
    }

    #[test]
    fn stale_by_age_is_unknown() {
        let dir = tempfile::TempDir::new().unwrap();
        let probed = Utc::now() - Duration::days(40);
        ReachStore::save(
            dir.path(),
            &record_at(probed, row(ReachVerdict::Verified, "2.1.269")),
        )
        .unwrap();
        let policy = ReachPolicy::for_workspace_at(dir.path(), Utc::now(), &versions("2.1.269"));
        assert_eq!(policy.for_provider("claude"), NestingPolicy::Unknown);
    }

    #[test]
    fn stale_by_version_is_unknown() {
        let dir = tempfile::TempDir::new().unwrap();
        ReachStore::save(
            dir.path(),
            &record_at(Utc::now(), row(ReachVerdict::Verified, "2.1.269")),
        )
        .unwrap();
        let policy = ReachPolicy::for_workspace_at(dir.path(), Utc::now(), &versions("2.1.270"));
        assert_eq!(policy.for_provider("claude"), NestingPolicy::Unknown);
    }

    #[test]
    fn verified_fresh_matching_version_is_allowed() {
        let dir = tempfile::TempDir::new().unwrap();
        ReachStore::save(
            dir.path(),
            &record_at(Utc::now(), row(ReachVerdict::Verified, "2.1.269")),
        )
        .unwrap();
        let policy = ReachPolicy::for_workspace_at(dir.path(), Utc::now(), &versions("2.1.269"));
        assert_eq!(policy.for_provider("claude"), NestingPolicy::Allowed);
    }

    #[test]
    fn nested_failed_is_blocked() {
        let dir = tempfile::TempDir::new().unwrap();
        ReachStore::save(
            dir.path(),
            &record_at(Utc::now(), row(ReachVerdict::NestedFailed, "2.1.269")),
        )
        .unwrap();
        let policy = ReachPolicy::for_workspace_at(dir.path(), Utc::now(), &versions("2.1.269"));
        match policy.for_provider("claude") {
            NestingPolicy::Blocked(reason) => {
                assert!(reason.contains("--mcp-reach-probe"), "{reason}");
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
    }

    #[test]
    fn settings_override_enforce_and_max_age() {
        let dir = tempfile::TempDir::new().unwrap();
        let gav = dir.path().join(".gaviero");
        std::fs::create_dir_all(&gav).unwrap();
        std::fs::write(
            gav.join("settings.json"),
            r#"{ "mcp.reach.enforce": false, "mcp.reach.maxAgeDays": 7 }"#,
        )
        .unwrap();
        let policy = ReachPolicy::for_workspace_at(dir.path(), Utc::now(), &versions("x"));
        assert!(!policy.enforce);
        assert_eq!(policy.max_age_days, 7);
    }
}
