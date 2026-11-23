//! Provider usage / rate limits during a swarm run.
//!
//! Subscription providers stop answering for a while once a usage window
//! is spent (Claude Code: "Usage limit reached · resets 3pm", "5-hour limit
//! reached"; Codex and Cursor: "You've hit your usage limit"; APIs: HTTP 429
//! / `rate_limit_error`). The failure reaches the unit's manifest as text,
//! so detection here is a classifier over that text. `gaviero-cli
//! --wait-on-limit` uses these helpers to wait the limit out and resume the
//! run from its checkpoint.

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use super::models::{AgentStatus, SwarmResult};
use crate::memory::consolidation_llm::{BackendConsolidationLlm, ConsolidationLlm};

/// Prompt sent to check whether a limited model answers again.
const PROBE_PROMPT: &str = "Reply with the single word OK.";

static STATUS_429: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"\b429\b").expect("valid regex"));

/// True when an error message reports a provider usage or rate limit — a
/// refusal that clears with time. Context-window and spend-budget limits
/// are not: waiting does not lift them.
pub fn is_provider_limit(message: &str) -> bool {
    let m = message.to_lowercase();
    if m.contains("context limit") || m.contains("budget limit") {
        return false;
    }
    const MARKERS: [&str; 9] = [
        "usage limit",
        "usage_limit",
        "rate limit",
        "rate_limit",
        "ratelimit",
        "too many requests",
        "hit your limit",
        "-hour limit",
        "weekly limit",
    ];
    MARKERS.iter().any(|marker| m.contains(marker)) || STATUS_429.is_match(&m)
}

/// Ids of the units a provider limit stopped.
pub fn limited_unit_ids(result: &SwarmResult) -> Vec<String> {
    result
        .manifests
        .iter()
        .filter(|m| matches!(&m.status, AgentStatus::Failed(msg) if is_provider_limit(msg)))
        .map(|m| m.work_unit_id.clone())
        .collect()
}

/// What one limit check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// The model answered.
    Clear,
    /// The model still refuses with a usage / rate limit.
    Limited(String),
    /// The probe failed for another reason (network, auth, …). Not a limit:
    /// the run resumes and reports the real failure if it persists.
    Unavailable(String),
}

/// Send one tiny prompt to `model_spec` and classify the answer.
pub async fn probe_model(
    model_spec: &str,
    ollama_base_url: Option<&str>,
    workspace_root: &Path,
) -> ProbeOutcome {
    let backend = match super::backend::shared::create_backend_for_model(
        model_spec,
        ollama_base_url,
    ) {
        Ok(b) => b,
        Err(e) => return ProbeOutcome::Unavailable(format!("{e:#}")),
    };
    let backend: Arc<dyn super::backend::AgentBackend> = backend.into();
    let llm = BackendConsolidationLlm::new(backend, workspace_root.to_path_buf());
    match llm.complete(PROBE_PROMPT.to_string()).await {
        Ok(_) => ProbeOutcome::Clear,
        Err(e) => {
            let msg = format!("{e:#}");
            if is_provider_limit(&msg) {
                ProbeOutcome::Limited(msg)
            } else {
                ProbeOutcome::Unavailable(msg)
            }
        }
    }
}

/// Wait `interval`, probe, and repeat while the probe reports a limit.
/// `report` sees every check (1-based). Returns the number of checks.
pub async fn wait_until_clear<F, Fut>(
    interval: Duration,
    mut probe: F,
    mut report: impl FnMut(u32, &ProbeOutcome),
) -> u32
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ProbeOutcome>,
{
    let mut checks = 0u32;
    loop {
        tokio::time::sleep(interval).await;
        checks += 1;
        let outcome = probe().await;
        report(checks, &outcome);
        if !matches!(outcome, ProbeOutcome::Limited(_)) {
            return checks;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::swarm::models::AgentManifest;

    #[test]
    fn is_provider_limit_matches_provider_wording() {
        for msg in [
            "Usage limit reached · resets 3pm (Europe/Madrid)",
            "5-hour limit reached · resets 6pm",
            "Weekly limit reached · resets Mon 9am",
            "You've hit your usage limit. Upgrade to Pro or try again at 3:05 PM.",
            "Error: You've hit your usage limit",
            "{\"code\":\"usage_limit_exceeded\"}",
            "API Error: {\"type\":\"rate_limit_error\",\"message\":\"slow down\"}",
            "Ollama returned 429 Too Many Requests: usage limit reached",
            "HTTP 429",
            "Claude AI usage limit reached|1767225600",
        ] {
            assert!(is_provider_limit(msg), "should match: {msg}");
        }
    }

    #[test]
    fn is_provider_limit_ignores_lookalikes() {
        for msg in [
            "Context limit reached · /compact or /clear to continue",
            "Budget limit reached ($5.00 of $5.00); stopping background agents.",
            "claude exited with status 1 before producing a result: no stderr output",
            "edit failed at line 4290",
            "backend resolution failed for 'a': privacy",
        ] {
            assert!(!is_provider_limit(msg), "should not match: {msg}");
        }
    }

    #[test]
    fn limited_unit_ids_picks_limit_failures() {
        let manifest = |id: &str, status: AgentStatus| AgentManifest {
            work_unit_id: id.to_string(),
            status,
            modified_files: vec![],
            branch: None,
            summary: None,
            output: None,
            cost_usd: 0.0,
        };
        let result = SwarmResult {
            manifests: vec![
                manifest("ok", AgentStatus::Completed),
                manifest("limited", AgentStatus::Failed("Usage limit reached".into())),
                manifest("broken", AgentStatus::Failed("compile error".into())),
                manifest("pending", AgentStatus::Pending),
            ],
            merge_results: vec![],
            success: false,
            pre_swarm_sha: String::new(),
        };
        assert_eq!(limited_unit_ids(&result), vec!["limited".to_string()]);
    }

    #[tokio::test]
    async fn wait_until_clear_retries_until_not_limited() {
        let mut answers = vec![
            ProbeOutcome::Clear,
            ProbeOutcome::Limited("usage limit".into()),
            ProbeOutcome::Limited("usage limit".into()),
        ];
        let mut seen = Vec::new();
        let checks = wait_until_clear(
            Duration::from_millis(5),
            || {
                let next = answers.pop().expect("probe called too often");
                async move { next }
            },
            |n, outcome| seen.push((n, outcome.clone())),
        )
        .await;
        assert_eq!(checks, 3);
        assert_eq!(seen.len(), 3);
        assert_eq!(seen[2], (3, ProbeOutcome::Clear));

        // A non-limit failure ends the wait too.
        let checks = wait_until_clear(
            Duration::from_millis(1),
            || async { ProbeOutcome::Unavailable("connection refused".into()) },
            |_, _| {},
        )
        .await;
        assert_eq!(checks, 1);
    }
}
