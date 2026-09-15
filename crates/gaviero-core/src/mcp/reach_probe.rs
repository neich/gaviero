//! MCP reach probe (Layer 4).
//!
//! Spawns each vendor CLI against a throwaway worktree, asks it to call
//! [`super::tools::TOOL_MEMORY_PING`] at depth 0 and (when the vendor has
//! a nesting switch) depth 1, then reads the server-side
//! [`super::probe::ProbeLedger`]. Agent text is logged and never consulted
//! (plan L2).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config_synth::McpConfigSynth;
use super::probe::{PingRecord, ProbeLedger};
use super::telemetry_sink::{McpCallRecord, default_telemetry_path};
use super::tools::TOOL_MEMORY_PING;
use crate::swarm::backend::{
    AgentBackend, CompletionRequest, UnifiedStreamEvent, shared::create_backend_for_model,
};

/// Vendor-neutral probe prompt. `{nonce}` is interpolated by
/// [`probe_prompt`]. Instructs a top-level ping, one nested ping, then
/// a reply with both receipts; nothing else.
pub const PROBE_PROMPT: &str = "\
You are running a Gaviero MCP reach probe. Do not read or edit any files.\n\
1. Call the MCP tool memory_ping with JSON arguments \
{\"nonce\":\"{nonce}\",\"depth\":0}.\n\
2. Delegate once to a native subagent. Instruct it to call memory_ping with \
{\"nonce\":\"{nonce}\",\"depth\":1} and to reply with the receipt only.\n\
3. Reply with both receipts (depth 0 and depth 1). Do nothing else.\n\
If you cannot spawn a subagent, still complete step 1 and say so.";

/// Same prompt naming the `gaviero-probe` subagent for Claude `--agents`.
pub const PROBE_PROMPT_EXPLICIT: &str = "\
You are running a Gaviero MCP reach probe. Do not read or edit any files.\n\
1. Call the MCP tool memory_ping with JSON arguments \
{\"nonce\":\"{nonce}\",\"depth\":0}.\n\
2. Spawn the gaviero-probe subagent (subagent_type gaviero-probe) and instruct \
it to call memory_ping with {\"nonce\":\"{nonce}\",\"depth\":1} and reply with \
the receipt.\n\
3. Reply with both receipts. Do nothing else.";

static NONCE_SEQ: AtomicU64 = AtomicU64::new(1);

/// How the probe talks to the memory server. HTTP is recorded here from
/// P0; the listener lands in P2, so live runs currently always synthesize
/// the stdio shim path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReachTransport {
    Stdio,
    Http,
    Both,
}

impl ReachTransport {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stdio" => Ok(Self::Stdio),
            "http" => Ok(Self::Http),
            "both" => Ok(Self::Both),
            other => anyhow::bail!("reach transport {other:?}: expected stdio | http | both"),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::Http => "http",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReachProbeConfig {
    pub providers: Vec<String>,
    pub depth: u8,
    pub transport: ReachTransport,
    pub timeout: Duration,
}

impl Default for ReachProbeConfig {
    fn default() -> Self {
        Self {
            providers: vec![
                "claude".into(),
                "codex".into(),
                "cursor".into(),
                "dsh".into(),
            ],
            depth: 1,
            transport: ReachTransport::Stdio,
            timeout: Duration::from_secs(180),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachVerdict {
    Verified,
    NestedFailed,
    TopLevelFailed,
    Unsupported,
    Skipped,
}

impl ReachVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::NestedFailed => "nested_failed",
            Self::TopLevelFailed => "top_level_failed",
            Self::Unsupported => "unsupported",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderReachResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
    pub transport: String,
    pub top_level: bool,
    pub nested: bool,
    pub nested_depth: u8,
    pub explicit_ref_required: bool,
    pub verdict: ReachVerdict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReachReport {
    pub workspace_id: String,
    pub probed_at: DateTime<Utc>,
    pub providers: BTreeMap<String, ProviderReachResult>,
}

/// Interpolate [`PROBE_PROMPT`] with a run nonce.
pub fn probe_prompt(nonce: &str) -> String {
    PROBE_PROMPT.replace("{nonce}", nonce)
}

/// Interpolate [`PROBE_PROMPT_EXPLICIT`].
pub fn probe_prompt_explicit(nonce: &str) -> String {
    PROBE_PROMPT_EXPLICIT.replace("{nonce}", nonce)
}

/// Claude `--agents` JSON declaring `gaviero-probe` with an explicit
/// server list. Open check #2: field name is `mcpServers` as in
/// `.claude/agents/*.md` frontmatter; if the CLI rejects it, P1.5 falls
/// back to a worktree agent file.
pub fn claude_probe_agents_json(server_name: &str) -> String {
    serde_json::json!({
        "gaviero-probe": {
            "description": "Nested MCP reach probe. Call memory_ping at depth 1.",
            "prompt": "Call memory_ping with the nonce and depth from the parent. Reply with the receipt. Read or edit nothing.",
            "mcpServers": [server_name]
        }
    })
    .to_string()
}

/// Map ledger observations + CLI/nesting facts to a verdict. Text from
/// the agent is not an input (L2).
pub fn classify_verdict(
    cli_present: bool,
    nesting_supported: bool,
    saw_top: bool,
    saw_nested: bool,
) -> ReachVerdict {
    if !cli_present {
        return ReachVerdict::Skipped;
    }
    if !nesting_supported {
        return ReachVerdict::Unsupported;
    }
    if saw_top && saw_nested {
        return ReachVerdict::Verified;
    }
    if saw_top {
        return ReachVerdict::NestedFailed;
    }
    ReachVerdict::TopLevelFailed
}

/// True when the default inheritance path missed nested MCP and the
/// explicit-reference pass recovered it.
pub fn explicit_ref_required(first: ReachVerdict, after_explicit: ReachVerdict) -> bool {
    first == ReachVerdict::NestedFailed && after_explicit == ReachVerdict::Verified
}

pub fn nesting_supported(provider: &str) -> bool {
    matches!(provider, "claude" | "codex")
}

pub fn provider_cli_bin(provider: &str) -> &str {
    match provider {
        "claude" => "claude",
        "codex" => "codex",
        "cursor" => "agent",
        "dsh" => "dsh",
        other => other,
    }
}

pub fn provider_model_spec(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("claude:sonnet"),
        "codex" => Some("codex:gpt-5.5"),
        "cursor" => Some("cursor:auto"),
        "dsh" => Some("dsh:deepseek-flash"),
        _ => None,
    }
}

pub fn cli_present(bin: &str) -> bool {
    crate::util::spawn::resolve_program(bin).is_some()
}

pub fn capture_cli_version(bin: &str) -> Option<String> {
    let mut cmd = crate::util::spawn::agent_command_std(bin);
    cmd.arg("--version");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let output = cmd.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let line = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    Some(line.to_string())
}

pub fn fresh_nonce() -> String {
    let seq = NONCE_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(
        Utc::now()
            .timestamp_nanos_opt()
            .unwrap_or_default()
            .to_le_bytes(),
    );
    let digest = hasher.finalize();
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

pub fn ledger_saw_depth(ledger: &ProbeLedger, nonce: &str, depth: u8) -> bool {
    ledger
        .records_for_nonce(nonce)
        .iter()
        .any(|r| r.depth == depth)
}

/// Fold `memory_ping` rows from the workspace NDJSON telemetry into the
/// ledger. Used when the probe reuses a live TUI server that has no
/// in-process [`ProbeLedger`] attached (L2 still holds: the server wrote
/// the telemetry, not the agent text).
pub fn ingest_telemetry_pings(ledger: &ProbeLedger, telemetry_path: &Path, nonce: &str) -> usize {
    let Ok(text) = std::fs::read_to_string(telemetry_path) else {
        return 0;
    };
    let mut n = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(rec) = serde_json::from_str::<McpCallRecord>(line) else {
            continue;
        };
        if rec.tool_name != TOOL_MEMORY_PING {
            continue;
        }
        let Some(got) = rec.input.get("nonce").and_then(|v| v.as_str()) else {
            continue;
        };
        if got != nonce {
            continue;
        }
        let depth = rec.input.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
        if ledger_saw_depth(ledger, nonce, depth) {
            continue;
        }
        let at = DateTime::parse_from_rfc3339(&rec.ts)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let receipt = rec
            .output
            .get("receipt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        ledger.record(PingRecord {
            nonce: nonce.to_string(),
            depth,
            at,
            receipt,
        });
        n += 1;
    }
    n
}

fn ingest_workspace_telemetry(ledger: &ProbeLedger, workspace_root: &Path, nonce: &str) {
    ingest_telemetry_pings(ledger, &default_telemetry_path(workspace_root), nonce);
}

fn allowed_tools_for(provider: &str) -> Vec<String> {
    // Restrict built-ins to delegation; MCP exposure is configured separately.
    if provider == "claude" { vec!["Agent".into()] } else { Vec::new() }
}

fn probe_request(
    prompt: String,
    workspace_root: PathBuf,
    provider: &str,
    agents_json: Option<String>,
) -> CompletionRequest {
    CompletionRequest {
        prompt,
        system_prompt: Some("You are a Gaviero MCP reach probe. Do not read or edit files.".into()),
        workspace_root,
        additional_roots: vec![],
        allowed_tools: allowed_tools_for(provider),
        file_attachments: vec![],
        conversation_history: vec![],
        file_refs: vec![],
        effort: Some("low".into()),
        extra: agents_json
            .into_iter()
            .map(|j| ("agents_json".into(), j))
            .collect(),
        max_tokens: Some(2048),
        auto_approve: true,
        suppress_hooks: true,
        file_scope: crate::types::FileScope::default(),
        tool_policy: None,
        exposed_tools: None,
        write_gate: None,
    }
}

/// Drain a backend stream until `Done`/`Error` or `timeout`.
pub async fn drain_backend(
    backend: &dyn AgentBackend,
    request: CompletionRequest,
    timeout: Duration,
) -> Result<()> {
    let mut stream = backend.stream_completion(request).await?;
    let drain = async {
        while let Some(item) = stream.next().await {
            match item {
                Ok(UnifiedStreamEvent::Done(_)) | Ok(UnifiedStreamEvent::Error(_)) => break,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(target: "mcp_reach", error = %e, "probe stream error");
                    break;
                }
            }
        }
    };
    match tokio::time::timeout(timeout, drain).await {
        Ok(()) => Ok(()),
        Err(_) => {
            tracing::warn!(target: "mcp_reach", "probe timed out");
            Ok(())
        }
    }
}

fn result_from_ledger(
    ledger: &ProbeLedger,
    nonce: &str,
    cli_present: bool,
    nesting_supported: bool,
    cli_version: Option<String>,
    transport: &str,
    depth: u8,
    explicit_ref_required: bool,
) -> ProviderReachResult {
    let top_level = ledger_saw_depth(ledger, nonce, 0);
    let nested = depth > 0 && ledger_saw_depth(ledger, nonce, 1);
    let verdict = classify_verdict(cli_present, nesting_supported, top_level, nested);
    ProviderReachResult {
        cli_version,
        transport: transport.to_string(),
        top_level,
        nested,
        nested_depth: if nested { 1 } else { 0 },
        explicit_ref_required,
        verdict,
    }
}

/// Run the probe for every configured provider.
///
/// `synth` is cloned into a throwaway worktree when git worktrees are
/// available; otherwise configs are written into a temp directory so
/// tests (and machines without a commit) still exercise the path.
pub async fn run_reach_probe(
    workspace_root: &Path,
    synth: &McpConfigSynth,
    ledger: &ProbeLedger,
    cfg: &ReachProbeConfig,
) -> Result<ReachReport> {
    let transports: &[&str] = match cfg.transport {
        ReachTransport::Http => &["http"],
        ReachTransport::Both => &["stdio", "http"],
        ReachTransport::Stdio => &["stdio"],
    };
    if transports.contains(&"http") && synth.http.is_none() {
        anyhow::bail!("HTTP reach probe requires a live HTTP endpoint; enable mcp.gavieroServer.http.enabled");
    }
    let mut providers = BTreeMap::new();
    for transport_label in transports {
        let mut transport_synth = synth.clone();
        transport_synth.transport.default = super::config_synth::McpTransportKind::parse(transport_label);
        transport_synth.transport.per_vendor.clear();
        let (agent_root, _guard) = prepare_probe_worktree(workspace_root, &transport_synth)?;
    for provider in &cfg.providers {
        let row = probe_one_provider(
            provider,
            workspace_root,
            &agent_root,
            ledger,
            cfg,
            transport_label,
            LiveBackendFactory,
        )
        .await;
        if transports.len() > 1 {
            providers.insert(format!("{provider}@{transport_label}"), row.clone());
        }
        providers.insert(provider.clone(), row);
    }
    }

    Ok(ReachReport {
        workspace_id: crate::workspace::identity::workspace_id_hex16(workspace_root),
        probed_at: Utc::now(),
        providers,
    })
}

struct ProbeWorktreeGuard {
    mgr: Option<crate::git::WorktreeManager>,
    handle: Option<crate::git::WorktreeHandle>,
    tmp: Option<tempfile::TempDir>,
}

impl Drop for ProbeWorktreeGuard {
    fn drop(&mut self) {
        if let (Some(mgr), Some(handle)) = (self.mgr.as_mut(), self.handle.as_ref()) {
            let _ = mgr.teardown(handle);
        }
        self.tmp.take();
    }
}

fn prepare_probe_worktree(
    workspace_root: &Path,
    synth: &McpConfigSynth,
) -> Result<(PathBuf, ProbeWorktreeGuard)> {
    let mut mgr = crate::git::WorktreeManager::new(workspace_root.to_path_buf());
    match mgr.provision("mcp-reach-probe") {
        Ok(handle) => {
            let path = handle.path.clone();
            let mut synth = synth.clone();
            synth.worktree = path.clone();
            super::synthesize_for_worktree(&synth)?;
            Ok((
                path,
                ProbeWorktreeGuard {
                    mgr: Some(mgr),
                    handle: Some(handle),
                    tmp: None,
                },
            ))
        }
        Err(e) => {
            tracing::debug!(
                target: "mcp_reach",
                error = %e,
                "worktree provision failed; using tempdir"
            );
            let tmp = tempfile::TempDir::new()?;
            let path = tmp.path().to_path_buf();
            let mut synth = synth.clone();
            synth.worktree = path.clone();
            super::synthesize_for_worktree(&synth)?;
            Ok((
                path,
                ProbeWorktreeGuard {
                    mgr: None,
                    handle: None,
                    tmp: Some(tmp),
                },
            ))
        }
    }
}

trait BackendFactory: Send + Sync {
    fn backend_for(&self, spec: &str) -> Result<Box<dyn AgentBackend>>;
}

struct LiveBackendFactory;

impl BackendFactory for LiveBackendFactory {
    fn backend_for(&self, spec: &str) -> Result<Box<dyn AgentBackend>> {
        create_backend_for_model(spec, None)
    }
}

async fn probe_one_provider(
    provider: &str,
    workspace_root: &Path,
    agent_root: &Path,
    ledger: &ProbeLedger,
    cfg: &ReachProbeConfig,
    transport_label: &str,
    factory: impl BackendFactory,
) -> ProviderReachResult {
    let bin = provider_cli_bin(provider);
    let present = cli_present(bin);
    let nested_ok = nesting_supported(provider);
    let version = if present {
        capture_cli_version(bin)
    } else {
        None
    };

    if !present {
        return result_from_ledger(
            ledger,
            "",
            false,
            nested_ok,
            version,
            transport_label,
            cfg.depth,
            false,
        );
    }

    if !nested_ok {
        // Cursor (and dsh until P4) have no nesting switch we can drive.
        // Still try a top-level ping so the record's `top_level` bit is
        // evidence, not a guess — verdict stays `unsupported`.
        let nonce = fresh_nonce();
        if let Some(spec) = provider_model_spec(provider) {
            match factory.backend_for(spec) {
                Ok(backend) => {
                    let req = probe_request(
                        probe_prompt(&nonce),
                        agent_root.to_path_buf(),
                        provider,
                        None,
                    );
                    let _ = drain_backend(backend.as_ref(), req, cfg.timeout).await;
                    ingest_workspace_telemetry(ledger, workspace_root, &nonce);
                }
                Err(e) => {
                    tracing::debug!(target: "mcp_reach", provider, error = %e, "backend missing");
                }
            }
        }
        let mut row = result_from_ledger(
            ledger,
            &nonce,
            true,
            false,
            version,
            transport_label,
            cfg.depth,
            false,
        );
        row.verdict = ReachVerdict::Unsupported;
        return row;
    }

    let Some(spec) = provider_model_spec(provider) else {
        return result_from_ledger(
            ledger,
            "",
            true,
            nested_ok,
            version,
            transport_label,
            cfg.depth,
            false,
        );
    };

    let backend = match factory.backend_for(spec) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(target: "mcp_reach", provider, error = %e, "cannot create backend");
            return result_from_ledger(
                ledger,
                "",
                true,
                nested_ok,
                version,
                transport_label,
                cfg.depth,
                false,
            );
        }
    };

    let nonce = fresh_nonce();
    let req = probe_request(
        probe_prompt(&nonce),
        agent_root.to_path_buf(),
        provider,
        None,
    );
    let _ = drain_backend(backend.as_ref(), req, cfg.timeout).await;
    ingest_workspace_telemetry(ledger, workspace_root, &nonce);

    let first = result_from_ledger(
        ledger,
        &nonce,
        true,
        true,
        version.clone(),
        transport_label,
        cfg.depth,
        false,
    );

    if provider != "claude" || first.verdict != ReachVerdict::NestedFailed {
        return first;
    }

    // Second pass: explicit `--agents` reference (open check #2).
    let backend = match factory.backend_for(spec) {
        Ok(b) => b,
        Err(_) => return first,
    };
    let req = probe_request(
        probe_prompt_explicit(&nonce),
        agent_root.to_path_buf(),
        provider,
        Some(claude_probe_agents_json("gaviero")),
    );
    let _ = drain_backend(backend.as_ref(), req, cfg.timeout).await;
    ingest_workspace_telemetry(ledger, workspace_root, &nonce);
    let second = result_from_ledger(
        ledger,
        &nonce,
        true,
        true,
        version,
        transport_label,
        cfg.depth,
        explicit_ref_required(
            first.verdict,
            classify_verdict(
                true,
                true,
                ledger_saw_depth(ledger, &nonce, 0),
                ledger_saw_depth(ledger, &nonce, 1),
            ),
        ),
    );
    let mut row = second;
    row.explicit_ref_required = explicit_ref_required(first.verdict, row.verdict);
    row
}

/// Test seam: score a provider from a ledger without spawning a CLI.
pub fn score_from_ledger(
    ledger: &ProbeLedger,
    nonce: &str,
    cli_present: bool,
    nesting_supported: bool,
    transport: &str,
    depth: u8,
) -> ProviderReachResult {
    result_from_ledger(
        ledger,
        nonce,
        cli_present,
        nesting_supported,
        None,
        transport,
        depth,
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::backend::StopReason;
    use crate::swarm::backend::mock::MockBackend;

    fn seed(ledger: &ProbeLedger, nonce: &str, depths: &[u8]) {
        let t = Utc::now();
        for &depth in depths {
            ledger.record(PingRecord {
                nonce: nonce.into(),
                depth,
                at: t,
                receipt: format!("r{depth}{nonce}"),
            });
        }
    }

    fn probe_req() -> CompletionRequest {
        probe_request("ping".into(), PathBuf::from("/tmp"), "claude", None)
    }

    #[test]
    fn classify_maps_all_five_outcomes() {
        assert_eq!(
            classify_verdict(false, true, false, false),
            ReachVerdict::Skipped
        );
        assert_eq!(
            classify_verdict(true, false, true, false),
            ReachVerdict::Unsupported
        );
        assert_eq!(
            classify_verdict(true, true, true, true),
            ReachVerdict::Verified
        );
        assert_eq!(
            classify_verdict(true, true, true, false),
            ReachVerdict::NestedFailed
        );
        assert_eq!(
            classify_verdict(true, true, false, false),
            ReachVerdict::TopLevelFailed
        );
    }

    #[test]
    fn explicit_ref_flag_only_when_second_pass_recovers() {
        assert!(explicit_ref_required(
            ReachVerdict::NestedFailed,
            ReachVerdict::Verified
        ));
        assert!(!explicit_ref_required(
            ReachVerdict::Verified,
            ReachVerdict::Verified
        ));
        assert!(!explicit_ref_required(
            ReachVerdict::NestedFailed,
            ReachVerdict::NestedFailed
        ));
    }

    #[test]
    fn probe_prompt_carries_nonce_and_both_depths() {
        let p = probe_prompt("abc123");
        assert!(p.contains("abc123"));
        assert!(p.contains("\"depth\":0"));
        assert!(p.contains("\"depth\":1"));
        assert!(p.contains("memory_ping"));
        assert!(!p.contains("{nonce}"));
    }

    #[test]
    fn claude_agents_json_names_mcp_servers() {
        let json = claude_probe_agents_json("gaviero");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            v["gaviero-probe"]["mcpServers"][0].as_str(),
            Some("gaviero")
        );
    }

    #[tokio::test]
    async fn mock_backend_feeds_ledger_for_all_five_outcomes() {
        let cases: &[(&str, bool, bool, &[u8], ReachVerdict)] = &[
            ("skip", false, true, &[], ReachVerdict::Skipped),
            ("unsup", true, false, &[0], ReachVerdict::Unsupported),
            ("ok", true, true, &[0, 1], ReachVerdict::Verified),
            ("nest", true, true, &[0], ReachVerdict::NestedFailed),
            ("top", true, true, &[], ReachVerdict::TopLevelFailed),
        ];
        for (nonce, present, nested_ok, depths, want) in cases {
            let ledger = ProbeLedger::new();
            seed(&ledger, nonce, depths);
            let backend =
                MockBackend::new(nonce, vec![UnifiedStreamEvent::Done(StopReason::EndTurn)]);
            drain_backend(&backend, probe_req(), Duration::from_secs(2))
                .await
                .unwrap();
            let row = score_from_ledger(&ledger, nonce, *present, *nested_ok, "stdio", 1);
            assert_eq!(row.verdict, *want, "nonce={nonce}");
        }
    }

    #[tokio::test]
    async fn nonce_isolation_across_two_concurrent_runs() {
        let ledger = ProbeLedger::new();
        let backend_a = MockBackend::new("a", vec![UnifiedStreamEvent::Done(StopReason::EndTurn)]);
        let backend_b = MockBackend::new("b", vec![UnifiedStreamEvent::Done(StopReason::EndTurn)]);

        let (ra, rb) = tokio::join!(
            async {
                seed(&ledger, "nonce-a", &[0, 1]);
                drain_backend(&backend_a, probe_req(), Duration::from_secs(2))
                    .await
                    .unwrap();
                score_from_ledger(&ledger, "nonce-a", true, true, "stdio", 1)
            },
            async {
                seed(&ledger, "nonce-b", &[0]);
                drain_backend(&backend_b, probe_req(), Duration::from_secs(2))
                    .await
                    .unwrap();
                score_from_ledger(&ledger, "nonce-b", true, true, "stdio", 1)
            }
        );

        assert_eq!(ra.verdict, ReachVerdict::Verified);
        assert_eq!(rb.verdict, ReachVerdict::NestedFailed);
        assert_eq!(ledger.records_for_nonce("nonce-a").len(), 2);
        assert_eq!(ledger.records_for_nonce("nonce-b").len(), 1);
    }

    #[test]
    fn agent_text_cannot_fake_a_nested_hit() {
        // A subagent that *claims* success without calling memory_ping
        // leaves the ledger empty at depth 1 → nested_failed.
        let ledger = ProbeLedger::new();
        seed(&ledger, "hallucinated", &[0]);
        let row = score_from_ledger(&ledger, "hallucinated", true, true, "stdio", 1);
        assert_eq!(row.verdict, ReachVerdict::NestedFailed);
        assert!(!row.nested);
    }

    #[test]
    fn ingest_telemetry_pings_folds_matching_nonce() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("mcp_calls.ndjson");
        let line = serde_json::json!({
            "ts": "2026-09-14T12:00:00Z",
            "tool_name": "memory_ping",
            "duration_us": 12,
            "empty_result": false,
            "input": { "nonce": "abc123", "depth": 0 },
            "output": { "receipt": "deadbeefcafe", "depth": 0 }
        });
        std::fs::write(&path, format!("{line}\n")).unwrap();
        let ledger = ProbeLedger::new();
        assert_eq!(ingest_telemetry_pings(&ledger, &path, "abc123"), 1);
        assert_eq!(ingest_telemetry_pings(&ledger, &path, "abc123"), 0);
        assert!(ledger_saw_depth(&ledger, "abc123", 0));
        assert_eq!(ingest_telemetry_pings(&ledger, &path, "other"), 0);
    }
}
