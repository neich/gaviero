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
use support::orchestrator::{ParallelContext, TurnOutcome, run_parallel, run_turn};
use support::prompt_capture::RecordingPromptObserver;
use support::scripts::{
    SHARED_BARRIER_AFTER_TURN_1, bugfix_session, feature_session, refactor_session,
};

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

// ── T1.5 — e2e_parallel_sessions_isolated ───────────────────────────

/// Probe 3 threshold: warning, not hard fail. Host scheduling can
/// perturb embedder warmup beyond 500ms — the test logs the deltas
/// and lets the user review trends.
const PROBE3_TIMING_WARNING_MS: u128 = 500;

/// T1.5 — three concurrent E2eEnvs running heterogeneous scripts.
///
/// Asserts:
/// 1. **Memory write isolation.** S0 seeds a unique marker; S1/S2 must
///    never retrieve it from their own stores.
/// 2. **Manifest separation.** Each session's manifest `selected_ids`
///    are subsets of *its* writer's row ids.
/// 3. **Embedder cache fairness.** First-turn elapsed deltas across
///    sessions are < `PROBE3_TIMING_WARNING_MS` (warning only).
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
#[ignore = "E2E: spawns 3x claude concurrently + 3x ONNX runtimes; run with --ignored"]
async fn e2e_parallel_sessions_isolated() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_parallel_sessions_isolated");
    let r = guard.report();

    r.section("setup");

    let env_s0 = Arc::new(E2eEnv::fresh().await?);
    let env_s1 = Arc::new(E2eEnv::fresh().await?);
    let env_s2 = Arc::new(E2eEnv::fresh().await?);
    r.kv("repo_s0", env_s0.repo.display());
    r.kv("repo_s1", env_s1.repo.display());
    r.kv("repo_s2", env_s2.repo.display());

    // S0 seeds a unique marker into its own store.
    let s0_marker = format!("EMBED_MARKER_S0_{}", uuid_like());
    let seed_text = format!(
        "Cross-session isolation canary: this fact must not leak. Marker: {s0_marker}"
    );
    let res = env_s0
        .services
        .writer
        .user_remember_scoped(env_s0.repo_scope(), &seed_text)
        .await
        .context("seeding S0 marker")?;
    r.kv(
        "s0_seed",
        format!("{:?}", res),
    );
    wait_for_writer_drain(&env_s0, r, WRITER_DRAIN_TIMEOUT).await?;

    let s0_marker_for_assert = s0_marker.clone();
    let s1_marker = format!("EMBED_MARKER_S1_{}", uuid_like());
    let s2_marker = format!("EMBED_MARKER_S2_{}", uuid_like());

    // ── Build scripts + ParallelContexts ────────────────────────────
    r.section("dispatch parallel");
    let recorder_s0 = RecordingPromptObserver::arc();
    let recorder_s1 = RecordingPromptObserver::arc();
    let recorder_s2 = RecordingPromptObserver::arc();

    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut barriers = std::collections::HashMap::new();
    barriers.insert(SHARED_BARRIER_AFTER_TURN_1, barrier);

    let scripts = vec![
        refactor_session("s0", &s0_marker_for_assert),
        bugfix_session("s1", &s1_marker),
        feature_session("s2", &s2_marker),
    ];
    let contexts = vec![
        ParallelContext {
            env: Arc::clone(&env_s0),
            recorder: Arc::clone(&recorder_s0),
            barriers: barriers.clone(),
            model: E2E_AGENT_MODEL.to_string(),
        },
        ParallelContext {
            env: Arc::clone(&env_s1),
            recorder: Arc::clone(&recorder_s1),
            barriers: barriers.clone(),
            model: E2E_AGENT_MODEL.to_string(),
        },
        ParallelContext {
            env: Arc::clone(&env_s2),
            recorder: Arc::clone(&recorder_s2),
            barriers,
            model: E2E_AGENT_MODEL.to_string(),
        },
    ];

    let reports = run_parallel(scripts, contexts).await?;
    for sr in &reports {
        r.kv(
            &format!("session/{}", sr.id),
            format!(
                "{} turns, first_turn={:?}",
                sr.records.len(),
                sr.first_turn_elapsed
            ),
        );
    }

    // Drain each writer in case any post-turn extraction enqueued.
    wait_for_writer_drain(&env_s0, r, WRITER_DRAIN_TIMEOUT).await?;
    wait_for_writer_drain(&env_s1, r, WRITER_DRAIN_TIMEOUT).await?;
    wait_for_writer_drain(&env_s2, r, WRITER_DRAIN_TIMEOUT).await?;

    // ── Probe 1: memory write isolation ────────────────────────────
    r.section("probe 1 — write isolation");
    let s1_hits = env_s1
        .search_repo(&s0_marker_for_assert)
        .await
        .context("S1 search for S0 marker")?;
    let s2_hits = env_s2
        .search_repo(&s0_marker_for_assert)
        .await
        .context("S2 search for S0 marker")?;
    r.kv("s1_sees_s0_marker_count", s1_hits.len());
    r.kv("s2_sees_s0_marker_count", s2_hits.len());
    let s1_leaked = s1_hits
        .iter()
        .any(|m| m.content.contains(&s0_marker_for_assert));
    let s2_leaked = s2_hits
        .iter()
        .any(|m| m.content.contains(&s0_marker_for_assert));
    if s1_leaked || s2_leaked {
        anyhow::bail!(
            "Probe 1 violated: S0 marker leaked into S1={s1_leaked} S2={s2_leaked}"
        );
    }
    r.line("    ✓ PASS: S0 marker does not appear in S1 / S2 stores");

    // ── Probe 2: manifest separation ────────────────────────────────
    r.section("probe 2 — manifest separation");
    for env in [&env_s0, &env_s1, &env_s2] {
        let store = env.services.stores.get(&env.repo_scope().target_store()).await?;
        let manifests = store.recent_manifests(64).await?;
        let recent = env.recent_all(24).await?;
        let local_ids: std::collections::HashSet<i64> =
            recent.iter().map(|m| m.id).collect();
        let mut foreign: Vec<i64> = Vec::new();
        for m in &manifests {
            let payload: serde_json::Value = match serde_json::from_str(&m.payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Some(arr) = payload.get("selected_ids").and_then(|v| v.as_array()) {
                for x in arr {
                    if let Some(id) = x.as_i64() {
                        if !local_ids.contains(&id) {
                            foreign.push(id);
                        }
                    }
                }
            }
        }
        r.kv(
            &format!("manifests({})", env.repo_id),
            format!(
                "rows={} foreign_ids={}",
                manifests.len(),
                foreign.len()
            ),
        );
        if !foreign.is_empty() {
            anyhow::bail!(
                "Probe 2 violated: {} foreign memory ids appeared in {}'s manifests: {:?}",
                foreign.len(),
                env.repo_id,
                foreign
            );
        }
    }
    r.line("    ✓ PASS: no cross-session memory ids in any manifest");

    // ── Probe 3: embedder cache fairness (warning only) ─────────────
    r.section("probe 3 — embedder cache fairness");
    let firsts: Vec<u128> = reports
        .iter()
        .filter_map(|s| s.first_turn_elapsed.map(|d| d.as_millis()))
        .collect();
    if firsts.len() < 2 {
        r.line("    ⊘ SKIP: fewer than 2 sessions reported a first-turn elapsed");
    } else {
        let &min = firsts.iter().min().unwrap();
        let &max = firsts.iter().max().unwrap();
        let drift = max - min;
        r.kv("first_turn_min_ms", min);
        r.kv("first_turn_max_ms", max);
        r.kv("first_turn_drift_ms", drift);
        if drift > PROBE3_TIMING_WARNING_MS {
            r.line(format!(
                "    ⚠ WARNING: first-turn drift {} ms exceeds {} ms — possible shared ONNX session",
                drift, PROBE3_TIMING_WARNING_MS
            ));
        } else {
            r.line(format!(
                "    ✓ first-turn drift {} ms ≤ {} ms",
                drift, PROBE3_TIMING_WARNING_MS
            ));
        }
    }

    Ok(())
}

// ── T1.6 — e2e_prompt_bloat_baseline ────────────────────────────────

/// Sections that are deterministic across Claude responses keep their
/// SHA in the snapshot. Sections that depend on the assistant's text
/// (notably `ReplayHistory` past the first turn) get only kind+bytes
/// +tokens — the SHA there would churn even when nothing structural
/// changed.
fn sha_is_deterministic(kind: SectionKind) -> bool {
    matches!(
        kind,
        SectionKind::UserMessage
            | SectionKind::MemorySelections
            | SectionKind::GraphSelections
            | SectionKind::Wrapper
    )
}

/// Snapshot-shape rendering of a PromptDigest. Drops `byte_range`
/// entirely (would carry tempdir-derived offsets in the section index
/// across runs) and truncates SHAs to 12 chars.
fn render_digest_for_snapshot(d: &PromptDigest) -> String {
    let mut out = String::new();
    use std::fmt::Write as _;
    let _ = writeln!(out, "turn_id: {}", d.turn_id);
    let _ = writeln!(out, "total_tokens_approx: {}", d.total_tokens_approx);
    let _ = writeln!(out, "sections:");
    for s in &d.sections {
        let _ = writeln!(out, "  - kind: {:?}", s.kind);
        let _ = writeln!(out, "    tokens_approx: {}", s.tokens_approx);
        if sha_is_deterministic(s.kind) {
            let sha = &s.sha256[..12.min(s.sha256.len())];
            let _ = writeln!(out, "    sha256: {}", sha);
        }
    }
    out
}

/// T1.6 — sequential snapshot regression baseline.
///
/// Drives the three default scripts sequentially (separate concern
/// from T1.5's concurrency), classifies each captured prompt, and
/// snapshots a structural digest. Reviewer accepts via
/// `cargo insta review`; baseline lands under
/// `crates/gaviero-core/tests/snapshots/`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: spawns claude across 3 scripts (~19 turns total); run with --ignored"]
async fn e2e_prompt_bloat_baseline() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_prompt_bloat_baseline");
    let r = guard.report();

    r.section("setup");

    // Three independent envs so the writer task per session lines up
    // cleanly with the per-script invocation.
    let env_s0 = Arc::new(E2eEnv::fresh().await?);
    let env_s1 = Arc::new(E2eEnv::fresh().await?);
    let env_s2 = Arc::new(E2eEnv::fresh().await?);

    let recorder = RecordingPromptObserver::arc();
    let scripts = vec![
        refactor_session("snap_refactor", "EMBED_MARKER_SNAP_R"),
        bugfix_session("snap_bugfix", "EMBED_MARKER_SNAP_B"),
        feature_session("snap_feature", "EMBED_MARKER_SNAP_F"),
    ];
    let envs = [Arc::clone(&env_s0), Arc::clone(&env_s1), Arc::clone(&env_s2)];

    // Run each script sequentially. Skip Barrier steps (only meaningful
    // in run_parallel) so the same script bodies work in both contexts.
    for (script, env) in scripts.into_iter().zip(envs.iter()) {
        r.section(format!("script {}", script.id));
        let mut resume_id: Option<String> = None;
        let mut tcount: u32 = 0;
        for step in script.steps.into_iter() {
            match step {
                support::orchestrator::Step::User(prompt) => {
                    tcount += 1;
                    let turn_id = format!("{}/t{}", script.id, tcount);
                    let outcome = run_turn(
                        &env.repo,
                        E2E_AGENT_MODEL,
                        &recorder,
                        &turn_id,
                        &prompt,
                        resume_id.clone(),
                    )
                    .await
                    .with_context(|| format!("driving {turn_id}"))?;
                    if resume_id.is_none() {
                        resume_id = outcome.session_id.clone();
                    }
                }
                support::orchestrator::Step::Reset => {
                    resume_id = None;
                }
                support::orchestrator::Step::Sleep(_)
                | support::orchestrator::Step::Barrier(_)
                | support::orchestrator::Step::AssertPromptSizeMax(_) => {}
            }
        }
    }

    // Classify every captured event and snapshot in event order.
    r.section("snapshot digests");
    let events = recorder.events();
    r.kv("total_events", events.len());

    let mut bundle = String::new();
    for ev in &events {
        let digest = classifier::classify(&ev.turn_id, &ev.prompt);
        bundle.push_str(&render_digest_for_snapshot(&digest));
        bundle.push_str("---\n");
    }

    insta::with_settings!({ omit_expression => true }, {
        insta::assert_snapshot!("prompt_bloat_baseline", bundle);
    });

    Ok(())
}

/// Cheap unique marker — avoids a `uuid` dep just for two test markers.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    format!("{:x}_{:x}", nanos, pid)
}
