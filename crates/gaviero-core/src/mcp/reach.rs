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

    /// Test / argv helper: build a policy without reading disk.
    pub fn from_parts(
        enforce: bool,
        max_age_days: u32,
        providers: BTreeMap<String, NestingPolicy>,
    ) -> Self {
        Self {
            enforce,
            max_age_days,
            providers,
        }
    }
}

pub const CODEX_DISABLE_MULTI_AGENT: &str = "features.multi_agent=false";
pub const CURSOR_REACH_WARNING: &str =
    "cursor: nested MCP reach not verified; native subagents may miss memory tools";

pub fn is_claude_nesting_tool(name: &str) -> bool {
    name.eq_ignore_ascii_case("Agent") || name.eq_ignore_ascii_case("Task")
}

/// Drop Claude `Agent`/`Task` when enforcement is on and nesting is blocked.
/// `Unknown` and `Allowed` leave the list untouched (today's behaviour).
pub fn filter_claude_tools(mut tools: Vec<String>, policy: &ReachPolicy) -> Vec<String> {
    if policy.enforce && matches!(policy.for_provider("claude"), NestingPolicy::Blocked(_)) {
        tools.retain(|t| !is_claude_nesting_tool(t));
    }
    tools
}

/// Append `--config features.multi_agent=false` when Codex nesting is blocked.
pub fn push_codex_multi_agent_override(args: &mut Vec<String>, policy: &ReachPolicy) {
    if policy.enforce && matches!(policy.for_provider("codex"), NestingPolicy::Blocked(_)) {
        args.push("--config".into());
        args.push(CODEX_DISABLE_MULTI_AGENT.into());
    }
}

pub fn cursor_reach_status(policy: &ReachPolicy) -> Option<&'static str> {
    if !policy.enforce {
        return None;
    }
    match policy.for_provider("cursor") {
        NestingPolicy::Allowed => None,
        _ => Some(CURSOR_REACH_WARNING),
    }
}

/// Human table for a persisted (or just-written) reach record.
pub fn format_reach_table(record: &ReachRecord) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "workspace {}  probed {}\n",
        record.workspace_id,
        record.probed_at.to_rfc3339()
    ));
    out.push_str(&format!(
        "{:<10} {:<20} {:<10} {:<4} {:<7} {:<16} {:<8}\n",
        "provider", "version", "transport", "top", "nested", "verdict", "explicit"
    ));
    for (name, row) in &record.providers {
        let version = row.cli_version.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "{:<10} {:<20} {:<10} {:<4} {:<7} {:<16} {:<8}\n",
            name,
            trunc(version, 20),
            row.transport,
            yn(row.top_level),
            yn(row.nested),
            row.verdict.as_str(),
            yn(row.explicit_ref_required),
        ));
    }
    out
}

fn yn(v: bool) -> &'static str {
    if v { "yes" } else { "no" }
}

fn trunc(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// TUI `/mcp` (and CLI human status): endpoint, transport, exposed-tool
/// setting, reach policy, and the last probe table when present.
pub fn format_mcp_status(root: &Path, endpoint: &str) -> String {
    let (enforce, max_age_days, exposed, transport) = read_mcp_status_settings(root);
    let mut out = String::new();
    out.push_str(&format!("endpoint:        {endpoint}\n"));
    out.push_str(&format!("transport:       {transport}\n"));
    out.push_str(&format!("exposed tools:   {exposed}\n"));
    out.push_str(&format!(
        "reach.enforce:   {enforce}   maxAgeDays: {max_age_days}\n"
    ));
    match ReachStore::load(root) {
        Ok(Some(rec)) => {
            out.push('\n');
            out.push_str(&format_reach_table(&rec));
        }
        Ok(None) => {
            out.push_str(
                "\nno mcp_reach.json — run `gaviero-cli --mcp-reach-probe` to measure nesting.\n",
            );
        }
        Err(e) => {
            out.push_str(&format!("\nfailed to read mcp_reach.json: {e}\n"));
        }
    }
    out
}

fn read_mcp_status_settings(root: &Path) -> (bool, u32, String, String) {
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
    let exposed = doc
        .as_ref()
        .and_then(|d| dot_get(d, settings::MCP_GAVIERO_EXPOSED_TOOLS))
        .map(format_exposed_tools)
        .unwrap_or_else(|| "memory_search, blast_radius, node_doc".into());
    let transport = doc
        .as_ref()
        .and_then(|d| dot_get(d, "mcp.gavieroServer.transport"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("stdio")
        .to_string();
    (enforce, max_age_days, exposed, transport)
}

fn format_exposed_tools(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(arr) => {
            let names: Vec<&str> = arr.iter().filter_map(|v| v.as_str()).collect();
            if names.is_empty() {
                "(empty — live server lists the full surface)".into()
            } else {
                names.join(", ")
            }
        }
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
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

    fn policy(enforce: bool, claude: NestingPolicy, codex: NestingPolicy) -> ReachPolicy {
        ReachPolicy::from_parts(
            enforce,
            30,
            BTreeMap::from([
                ("claude".into(), claude),
                ("codex".into(), codex),
                ("cursor".into(), NestingPolicy::Unknown),
            ]),
        )
    }

    #[test]
    fn claude_tools_drop_agent_only_when_blocked_and_enforced() {
        let tools = vec!["Read".into(), "Agent".into(), "Task".into()];
        let blocked = policy(
            true,
            NestingPolicy::Blocked("nested failed".into()),
            NestingPolicy::Unknown,
        );
        let filtered = filter_claude_tools(tools.clone(), &blocked);
        assert_eq!(filtered, vec!["Read".to_string()]);

        let unknown = policy(true, NestingPolicy::Unknown, NestingPolicy::Unknown);
        assert_eq!(filter_claude_tools(tools.clone(), &unknown), tools);

        let allowed = policy(true, NestingPolicy::Allowed, NestingPolicy::Unknown);
        assert_eq!(filter_claude_tools(tools.clone(), &allowed), tools);

        let off = policy(
            false,
            NestingPolicy::Blocked("x".into()),
            NestingPolicy::Unknown,
        );
        assert_eq!(filter_claude_tools(tools, &off), vec!["Read", "Agent", "Task"]);
    }

    #[test]
    fn codex_multi_agent_override_present_only_when_blocked_and_enforced() {
        let mut args = vec!["exec".into()];
        let blocked = policy(
            true,
            NestingPolicy::Unknown,
            NestingPolicy::Blocked("nested failed".into()),
        );
        push_codex_multi_agent_override(&mut args, &blocked);
        assert!(
            args.windows(2)
                .any(|w| w == ["--config", CODEX_DISABLE_MULTI_AGENT])
        );

        let mut args = vec!["exec".into()];
        let unknown = policy(true, NestingPolicy::Unknown, NestingPolicy::Unknown);
        push_codex_multi_agent_override(&mut args, &unknown);
        assert!(!args.iter().any(|a| a.contains("multi_agent")));

        let mut args = vec!["exec".into()];
        let off = policy(
            false,
            NestingPolicy::Unknown,
            NestingPolicy::Blocked("x".into()),
        );
        push_codex_multi_agent_override(&mut args, &off);
        assert!(!args.iter().any(|a| a.contains("multi_agent")));
    }

    #[test]
    fn cursor_warns_unless_allowed_or_unenforced() {
        let unknown = policy(true, NestingPolicy::Unknown, NestingPolicy::Unknown);
        assert_eq!(cursor_reach_status(&unknown), Some(CURSOR_REACH_WARNING));
        let off = policy(false, NestingPolicy::Unknown, NestingPolicy::Unknown);
        assert_eq!(cursor_reach_status(&off), None);
        let allowed = ReachPolicy::from_parts(
            true,
            30,
            BTreeMap::from([("cursor".into(), NestingPolicy::Allowed)]),
        );
        assert_eq!(cursor_reach_status(&allowed), None);
    }

    #[test]
    fn format_reach_table_lists_verdict() {
        let rec = record_at(Utc::now(), row(ReachVerdict::NestedFailed, "2.1.269"));
        let table = format_reach_table(&rec);
        assert!(table.contains("claude"), "{table}");
        assert!(table.contains("nested_failed"), "{table}");
        assert!(table.contains("stdio"), "{table}");
    }

    #[test]
    fn format_mcp_status_without_record_points_at_cli() {
        let dir = tempfile::TempDir::new().unwrap();
        let text = format_mcp_status(dir.path(), r"\\.\pipe\gaviero-test");
        assert!(text.contains("no mcp_reach.json"), "{text}");
        assert!(text.contains("--mcp-reach-probe"), "{text}");
        assert!(text.contains(r"\\.\pipe\gaviero-test"), "{text}");
        assert!(text.contains("stdio"), "{text}");
    }
}
