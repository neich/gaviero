//! Tier T1 testbed integration tests.
//!
//! All tests in this binary are `#[ignore]`-gated and use the lifted
//! harness from `tests/support/`. Run with:
//!
//! ```bash
//! cargo test -p gaviero-core --test memory_testbed_e2e -- --ignored --nocapture
//! ```
//!
//! T1.4 (`e2e_reset_residual_zero`) is the central deliverable of PR1:
//! it falsifies or confirms the user's hypothesis that prompts after
//! `/reset` are too large because residual context isn't cleaned up.
//!
//! T1.5 (`e2e_parallel_sessions_isolated`) and T1.6
//! (`e2e_prompt_bloat_baseline`) land in PR2.

#![allow(clippy::needless_pass_by_value)]

mod support;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use gaviero_core::memory::{
    ChatInjectionConfig, ScopeMix, WriteResult, retrieval::retrieve_for_chat, scope::MemoryScope,
};

use support::classifier::{self, PromptDigest, Section, SectionKind};
use support::env::{
    E2E_AGENT_MODEL, E2eEnv, ReportGuard, WRITER_DRAIN_TIMEOUT, wait_for_writer_drain,
};
use support::orchestrator::{TurnOutcome, run_turn};
use support::prompt_capture::RecordingPromptObserver;

/// Marker the seeded canary memory carries. Each turn references it so
/// retrieval has something concrete to surface.
const CANARY_MARKER: &str = "EMBED_MARKER_phase1_residual_canary";

/// SLO 1: t4 must not be more than 5% larger than t1 in approximate
/// tokens. If t4 bloats past this, residual context is leaking.
const SLO1_BULK_RATIO: f32 = 1.05;

/// T1.4 — `/reset` residual-zero test (the central PR1 deliverable).
///
/// Drives four real-Claude turns through the lifted orchestrator with
/// a `RecordingPromptObserver` wired in. Asserts three SLOs:
///
/// 1. **Bulk**: `tokens(t4) / tokens(t1) ≤ 1.05`.
/// 2. **History residual**: count of `ReplayHistory` SHA-256 sections in
///    t4 that match a `ReplayHistory` section from t2 or t3 but not from
///    t1 must be 0.
/// 3. **Manifest stability**: if `recent_manifests(8)` returns ≥2 rows,
///    the most-recent and oldest `selected_ids` must be equal. <2 rows
///    → SKIP (manifest emission is gated on writer pipeline; the test's
///    `retrieve_for_chat` direct call may not enqueue manifests). The
///    skip is logged as a known-limitation per the plan.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: spawns claude 4x + ONNX runtime; run with --ignored"]
async fn e2e_reset_residual_zero() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_reset_residual_zero");
    let r = guard.report();

    r.section("setup");
    let env = E2eEnv::fresh().await?;
    r.kv("repo", env.repo.display());
    r.kv("repo_id", &env.repo_id);
    r.kv("model", E2E_AGENT_MODEL);

    // ── Seed the canary memory at repo scope ────────────────────────
    r.section("seed canary");
    let canary_text = format!(
        "Composite scoring weights canary fact: similarity 0.50, importance 0.20, \
         recency 0.15, base 0.15. Marker: {CANARY_MARKER}"
    );
    let res = env
        .services
        .writer
        .user_remember_scoped(env.repo_scope(), &canary_text)
        .await
        .context("seeding canary memory")?;
    match res {
        WriteResult::Inserted(id) | WriteResult::Deduplicated(id) => {
            r.kv("canary_id", id);
        }
        other => anyhow::bail!("canary user_remember_scoped did not persist: {other:?}"),
    }
    wait_for_writer_drain(&env, r, WRITER_DRAIN_TIMEOUT).await?;

    // ── Drive four turns ────────────────────────────────────────────
    let recorder = RecordingPromptObserver::arc();

    let user_t1 = format!(
        "Read crates/gaviero-core/src/memory/retrieval.rs and summarise the composite \
         scoring formula. Reference the canary marker {CANARY_MARKER} in your reply."
    );
    let user_t2 =
        "List every callsite of retrieve_for_chat in the workspace.".to_string();
    let user_t3 =
        "Propose a refactor that extracts the rerank blending into a separate function."
            .to_string();
    // t4 is identical to t1.
    let user_t4 = user_t1.clone();

    let history_after_t1: String;
    let history_after_t2: String;
    let history_after_t3: String;

    let injection_cfg = ChatInjectionConfig {
        enabled: true,
        scopes: ScopeMix {
            workspace: true,
            repo: true,
            module: true,
            global: false,
        },
        max_items: 5,
        token_budget: 2000,
        min_similarity: 0.0,
    };
    let memory_scope = MemoryScope::from_context(&env.repo, Some(&env.repo), None, None);

    // ── t1 ──
    r.section("turn t1 (cold)");
    let outcome_t1 = drive_turn(
        &env,
        Arc::clone(&recorder),
        "t1",
        &user_t1,
        /* resume */ None,
        /* simulated_history */ None,
        &injection_cfg,
        &memory_scope,
    )
    .await?;
    r.kv("session_id", outcome_t1.session_id.as_deref().unwrap_or("?"));
    r.kv("elapsed_ms", outcome_t1.elapsed.as_millis());
    history_after_t1 =
        append_to_history("", &user_t1, &outcome_t1.assistant_text);

    let resume_id_t1 = outcome_t1.session_id.clone();

    // ── t2 ── continuation, history = t1 transcript
    r.section("turn t2 (continuation)");
    let outcome_t2 = drive_turn(
        &env,
        Arc::clone(&recorder),
        "t2",
        &user_t2,
        resume_id_t1.clone(),
        Some(history_after_t1.clone()),
        &injection_cfg,
        &memory_scope,
    )
    .await?;
    r.kv("session_id", outcome_t2.session_id.as_deref().unwrap_or("?"));
    r.kv("elapsed_ms", outcome_t2.elapsed.as_millis());
    history_after_t2 =
        append_to_history(&history_after_t1, &user_t2, &outcome_t2.assistant_text);

    // ── t3 ── continuation, history = t1+t2 transcripts
    r.section("turn t3 (continuation)");
    let outcome_t3 = drive_turn(
        &env,
        Arc::clone(&recorder),
        "t3",
        &user_t3,
        outcome_t2.session_id.clone(),
        Some(history_after_t2.clone()),
        &injection_cfg,
        &memory_scope,
    )
    .await?;
    r.kv("session_id", outcome_t3.session_id.as_deref().unwrap_or("?"));
    r.kv("elapsed_ms", outcome_t3.elapsed.as_millis());
    history_after_t3 =
        append_to_history(&history_after_t2, &user_t3, &outcome_t3.assistant_text);
    let _ = history_after_t3; // retained for diagnostic symmetry

    // ── /reset ── transport-layer model: drop resume_session_id +
    //              drop simulated history. Bootstrap context (memory
    //              block) intentionally still flows on the post-reset
    //              turn — this is documented behaviour, and the test
    //              measures whether *anything else* leaks through.

    // ── t4 ── identical to t1, cold
    r.section("turn t4 (post-reset, identical to t1)");
    let outcome_t4 = drive_turn(
        &env,
        Arc::clone(&recorder),
        "t4",
        &user_t4,
        /* resume */ None,
        /* simulated_history */ None,
        &injection_cfg,
        &memory_scope,
    )
    .await?;
    r.kv("session_id", outcome_t4.session_id.as_deref().unwrap_or("?"));
    r.kv("elapsed_ms", outcome_t4.elapsed.as_millis());

    // ── Pull captured PromptEvents and classify ─────────────────────
    r.section("classify captured prompts");
    let events = recorder.events();
    r.kv("total_events", events.len());

    let digest_t1 = digest_for_turn(&events, "t1")?;
    let digest_t2 = digest_for_turn(&events, "t2")?;
    let digest_t3 = digest_for_turn(&events, "t3")?;
    let digest_t4 = digest_for_turn(&events, "t4")?;

    log_digest(r, &digest_t1);
    log_digest(r, &digest_t2);
    log_digest(r, &digest_t3);
    log_digest(r, &digest_t4);

    // ── SLO 1: bulk ratio ───────────────────────────────────────────
    r.section("SLO 1 — bulk");
    let t1_tok = digest_t1.total_tokens_approx as f32;
    let t4_tok = digest_t4.total_tokens_approx as f32;
    let ratio = if t1_tok > 0.0 { t4_tok / t1_tok } else { 1.0 };
    r.kv(
        "tokens(t1)",
        format!("{} tok ({} B)", digest_t1.total_tokens_approx, digest_t1.total_bytes),
    );
    r.kv(
        "tokens(t4)",
        format!("{} tok ({} B)", digest_t4.total_tokens_approx, digest_t4.total_bytes),
    );
    r.kv("tokens(t4)/tokens(t1)", format!("{:.3}", ratio));
    if ratio > SLO1_BULK_RATIO {
        r.line(format!(
            "    ⨯ FAIL: ratio {:.3} > {:.2}",
            ratio, SLO1_BULK_RATIO
        ));
        log_section_breakdown(r, "t1", &digest_t1);
        log_section_breakdown(r, "t4", &digest_t4);
        anyhow::bail!(
            "SLO 1 violated: tokens(t4)/tokens(t1) = {:.3} > {:.2}",
            ratio,
            SLO1_BULK_RATIO,
        );
    }
    r.line(format!(
        "    ✓ PASS: ratio {:.3} ≤ {:.2}",
        ratio, SLO1_BULK_RATIO
    ));

    // ── SLO 2: history residual ─────────────────────────────────────
    r.section("SLO 2 — history residual");
    let t1_hist: HashSet<&str> = history_section_shas(&digest_t1);
    let t2_hist: HashSet<&str> = history_section_shas(&digest_t2);
    let t3_hist: HashSet<&str> = history_section_shas(&digest_t3);
    let t4_hist: HashSet<&str> = history_section_shas(&digest_t4);

    let leaked_into_t4: Vec<&str> = t4_hist
        .iter()
        .filter(|sha| (t2_hist.contains(*sha) || t3_hist.contains(*sha)) && !t1_hist.contains(*sha))
        .copied()
        .collect();
    r.kv("history_shas(t1)", t1_hist.len());
    r.kv("history_shas(t2)", t2_hist.len());
    r.kv("history_shas(t3)", t3_hist.len());
    r.kv("history_shas(t4)", t4_hist.len());
    r.kv("leaked(t4 ∩ (t2∪t3) \\ t1)", leaked_into_t4.len());
    if !leaked_into_t4.is_empty() {
        for sha in &leaked_into_t4 {
            r.line(format!("    ⨯ leaked sha: {}", sha));
        }
        anyhow::bail!(
            "SLO 2 violated: {} ReplayHistory section(s) from t2/t3 leaked into t4",
            leaked_into_t4.len()
        );
    }
    r.line("    ✓ PASS: no t2/t3-only history sections survive into t4");

    // ── SLO 3: manifest stability ──────────────────────────────────
    r.section("SLO 3 — manifest stability");
    let store = env
        .services
        .stores
        .get(&env.repo_scope().target_store())
        .await?;
    let manifests = store.recent_manifests(8).await?;
    r.kv("manifest_rows", manifests.len());
    if manifests.len() < 2 {
        r.line(
            "    ⊘ SKIP: manifest emission requires the chat-injection writer pipeline; \
             retrieve_for_chat direct call does not enqueue. Documented limitation; \
             ensure_injection-wired version is a follow-up.",
        );
    } else {
        let newest_ids = extract_selected_ids(&manifests[0].payload)?;
        let oldest_ids = extract_selected_ids(manifests.last().unwrap().payload.as_str())?;
        r.kv("newest_selected_ids", format!("{:?}", newest_ids));
        r.kv("oldest_selected_ids", format!("{:?}", oldest_ids));
        if newest_ids != oldest_ids {
            anyhow::bail!(
                "SLO 3 violated: manifest selected_ids drifted across runs (newest={:?}, oldest={:?})",
                newest_ids,
                oldest_ids,
            );
        }
        r.line("    ✓ PASS: newest and oldest manifest selected_ids are equal");
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn drive_turn(
    env: &E2eEnv,
    recorder: Arc<RecordingPromptObserver>,
    turn_id: &str,
    user_msg: &str,
    resume_session_id: Option<String>,
    simulated_history: Option<String>,
    injection_cfg: &ChatInjectionConfig,
    memory_scope: &MemoryScope,
) -> Result<TurnOutcome> {
    // Mirror the production chat-injection path: retrieve memory,
    // assemble enriched prompt = user + memory_block + history.
    let injection = retrieve_for_chat(
        &env.services.stores,
        memory_scope,
        user_msg,
        injection_cfg,
    )
    .await
    .context("retrieve_for_chat")?;

    let mut parts: Vec<String> = Vec::new();
    parts.push(user_msg.to_string());
    if let Some(inj) = injection.as_ref() {
        if !inj.block.is_empty() {
            parts.push(String::new());
            parts.push(inj.block.clone());
        }
    }
    if let Some(history) = simulated_history.as_ref() {
        if !history.trim().is_empty() {
            parts.push(String::new());
            parts.push(format!("\nPrevConv:\n{}", history.trim_end()));
        }
    }
    let enriched_prompt = parts.join("\n");

    run_turn(
        &env.repo,
        E2E_AGENT_MODEL,
        &recorder,
        turn_id,
        &enriched_prompt,
        resume_session_id,
    )
    .await
}

fn append_to_history(prev: &str, user_msg: &str, assistant_msg: &str) -> String {
    // Caveman PrevConv: convention from build_enriched_prompt.
    let mut out = String::from(prev);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("U: ");
    out.push_str(user_msg.trim());
    out.push('\n');
    out.push_str("A: ");
    out.push_str(assistant_msg.trim());
    out.push('\n');
    out
}

fn digest_for_turn(
    events: &[gaviero_core::observer::PromptEvent],
    turn_id: &str,
) -> Result<PromptDigest> {
    let ev = events
        .iter()
        .find(|e| e.turn_id == turn_id)
        .with_context(|| format!("no PromptEvent for turn `{turn_id}`"))?;
    Ok(classifier::classify(turn_id, &ev.prompt))
}

fn log_digest(r: &mut support::env::TestReport, d: &PromptDigest) {
    r.kv(
        &format!("{}/total", d.turn_id),
        format!("{} B / {} tok", d.total_bytes, d.total_tokens_approx),
    );
    let mut by_kind: std::collections::BTreeMap<SectionKind, (usize, usize)> =
        std::collections::BTreeMap::new();
    for s in &d.sections {
        let entry = by_kind.entry(s.kind).or_default();
        entry.0 += s.bytes;
        entry.1 += s.tokens_approx;
    }
    for (k, (b, t)) in by_kind {
        r.line(format!(
            "    {} · {:?} = {} B ({} tok)",
            d.turn_id, k, b, t
        ));
    }
}

fn log_section_breakdown(r: &mut support::env::TestReport, label: &str, d: &PromptDigest) {
    for s in &d.sections {
        r.line(format!(
            "    {} {:?} bytes={} tok={} sha={}",
            label,
            s.kind,
            s.bytes,
            s.tokens_approx,
            &s.sha256[..12.min(s.sha256.len())],
        ));
    }
}

fn history_section_shas(d: &PromptDigest) -> HashSet<&str> {
    d.sections
        .iter()
        .filter(|s: &&Section| s.kind == SectionKind::ReplayHistory)
        .map(|s| s.sha256.as_str())
        .collect()
}

fn extract_selected_ids(payload: &str) -> Result<Vec<i64>> {
    let v: serde_json::Value = serde_json::from_str(payload)
        .context("parsing manifest payload as JSON")?;
    let arr = v
        .get("selected_ids")
        .and_then(|x| x.as_array())
        .with_context(|| "manifest payload missing selected_ids array")?;
    let mut out = Vec::with_capacity(arr.len());
    for x in arr {
        out.push(
            x.as_i64()
                .with_context(|| "selected_ids entry not i64")?,
        );
    }
    Ok(out)
}

// Dummy to silence the unused Duration import on builds where the
// orchestrator's only dependency is via run_turn.
#[allow(dead_code)]
const _DURATION_PIN: Duration = Duration::from_secs(0);
