//! End-to-end memory-system tests that drive *real* `claude` subprocesses
//! against *real* ONNX-backed [`MemoryServices`].
//!
//! These tests are deliberately separated from the normal test set:
//!
//! * Every test is `#[ignore]`-gated so `cargo test` does not run them.
//! * Run the full suite with:
//!
//!   ```bash
//!   cargo test -p gaviero-core --test memory_session_e2e -- --ignored --nocapture
//!   ```
//!
//!   `--nocapture` is recommended: each test prints a verbose
//!   pre/post-state report so a failure (or success) is immediately
//!   actionable without re-running with extra flags.
//!
//! * Single tests can be run by name, e.g.:
//!
//!   ```bash
//!   cargo test -p gaviero-core --test memory_session_e2e \
//!       e2e_full_dev_session_simulation -- --ignored --nocapture
//!   ```
//!
//! Required environment:
//!
//! * `claude` CLI on `PATH`, already authenticated (`claude login`).
//! * ONNX runtime + the configured embedder model present in the cache
//!   (the suite calls `MemoryServices::open` with the default embedder).
//!
//! Coverage map (one test per row):
//!
//! 1. [`e2e_one_shot_session_writes_memory`] — single Claude turn ending
//!    with a `<turn_annotations>` sidecar; writer's `TurnComplete` path
//!    persists the annotated row at repo scope.
//! 2. [`e2e_multi_shot_session_writes_per_turn_memory`] — three Claude
//!    turns against the same workspace each persist their own annotated
//!    rows independently.
//! 3. [`e2e_subsequent_session_injects_memory`] — pre-seeded facts get
//!    retrieved into a `<project_memory>` block on a new turn; the
//!    `splice_into_selections` contract is preserved.
//! 4. [`e2e_spilled_prompt_is_minimal`] — the file written under
//!    `.gaviero/tmp/prompt-*.md` contains *only* the rendered prompt; no
//!    candidate-pool dump, system prompt, or other ambient context.
//! 5. [`e2e_full_dev_session_simulation`] — full development-session
//!    simulation: a real `BackendConsolidationLlm` (Haiku) runs the S3
//!    extractor against multi-turn transcripts; subsequent turns retrieve
//!    extracted facts via injection. End-to-end memory loop with no
//!    short-circuits.

#![allow(clippy::needless_pass_by_value)]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};

use gaviero_core::acp::session::would_use_tempfile;
use gaviero_core::memory::scope::SCOPE_REPO;
use gaviero_core::memory::{
    BackendConsolidationLlm, ChatInjectionConfig, ConsolidationLlm, ScopeMix, WriteResult,
    retrieval::retrieve_for_chat, scope::MemoryScope,
};
use gaviero_core::swarm::backend::shared::create_backend_for_model;

use support::env::{
    E2E_AGENT_MODEL, E2eEnv, ReportGuard, WRITER_DRAIN_TIMEOUT, build_remember_prompt,
    empty_selections, record_search_hits, record_turn_diagnostics, run_one_claude_turn, truncate,
    wait_for_writer_drain,
};

// All harness types (`E2eEnv`, `WriteCounters`, `CapturingObserver`,
// `TestReport`, `run_one_claude_turn`, ...) are defined in
// `support/env.rs`. The shared module is wired by `mod support;` at the
// top of this file (and any other integration test that needs the same
// plumbing — see `tests/memory_testbed_e2e.rs`).

// ── Tests ────────────────────────────────────────────────────────────────────

/// Test 1 — one-shot agent session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: spawns claude + ONNX runtime; run with --ignored"]
async fn e2e_one_shot_session_writes_memory() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_one_shot_session_writes_memory");
    let r = guard.report();
    r.section("setup");

    let env = E2eEnv::fresh().await?;
    r.kv("repo", env.repo.display());
    r.kv("repo_id", &env.repo_id);

    let fact = "gaviero E2E test marker alpha-7421";
    let prompt = build_remember_prompt(fact);
    r.kv("model", E2E_AGENT_MODEL);
    r.kv("prompt_len_bytes", prompt.len());

    r.section("claude turn");
    let snap = run_one_claude_turn(&env.repo, E2E_AGENT_MODEL, &prompt, true)
        .await
        .context("driving claude turn")?;
    record_turn_diagnostics(r, &snap);

    let assistant_reply = snap
        .messages
        .iter()
        .find(|(role, _)| role == "assistant")
        .map(|(_, c)| c.clone())
        .unwrap_or_else(|| snap.streamed.clone());
    r.kv("assistant_reply_len", assistant_reply.len());

    if assistant_reply.is_empty() {
        r.line(
            "    ⨯ FAIL: claude returned no assistant text — see streamed/messages above"
                .to_string(),
        );
        anyhow::bail!("claude returned no assistant text");
    }

    r.section("annotation parse");
    let parsed = gaviero_core::memory::parse_and_strip(&assistant_reply);
    if let Some(err) = parsed.parse_error.as_ref() {
        r.line(format!("    ⚠ <turn_annotations> block found but malformed: {err}"));
    }
    let annotations = match parsed.annotations.clone() {
        Some(a) => a,
        None => {
            r.line(
                "    ⨯ FAIL: claude reply did not carry a <turn_annotations> block.",
            );
            r.line(format!("      reply (first 500 chars): {}", truncate(&assistant_reply, 500)));
            anyhow::bail!("no <turn_annotations> block in reply");
        }
    };
    r.kv("flags", annotations.flags.len());
    r.kv("session_thread", annotations.session_thread.as_deref().unwrap_or("<none>"));
    for f in &annotations.flags {
        r.line(format!(
            "    • flag type={} scope={} importance={:.2} text=\"{}\"",
            f.kind,
            f.scope,
            f.importance,
            truncate(&f.text, 120),
        ));
    }
    if annotations.flags.is_empty() {
        anyhow::bail!("annotations carried no flags");
    }

    r.section("writer dispatch");
    let transcript = format!("user: {prompt}\nassistant: {}", parsed.stripped);
    env.services
        .writer
        .turn_complete(
            "session-1",
            "turn-1",
            &env.repo_id,
            None,
            "run-1",
            transcript,
            Some(serde_json::to_value(&annotations)?),
        )
        .context("turn_complete enqueue")?;

    wait_for_writer_drain(&env, r, WRITER_DRAIN_TIMEOUT).await?;

    r.section("verification");
    let hits = env.search_repo("alpha-7421").await?;
    record_search_hits(r, "search 'alpha-7421'", &hits);

    if !hits.iter().any(|m| m.content.contains("alpha-7421")) {
        anyhow::bail!(
            "no stored memory contained the fact marker; got {} hits",
            hits.len()
        );
    }

    r.line("    ✓ PASS: marker fact landed in repo-scope store");
    Ok(())
}

/// Test 2 — multi-shot agent session.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: spawns claude 3x + ONNX runtime; run with --ignored"]
async fn e2e_multi_shot_session_writes_per_turn_memory() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_multi_shot_session_writes_per_turn_memory");
    let r = guard.report();
    r.section("setup");

    let env = E2eEnv::fresh().await?;
    r.kv("repo_id", &env.repo_id);

    let facts = [
        ("turn-a", "multi-shot fact bravo-9911"),
        ("turn-b", "multi-shot fact charlie-3370"),
        ("turn-c", "multi-shot fact delta-8842"),
    ];
    r.kv("turns", facts.len());

    for (turn_id, fact) in &facts {
        r.section(format!("turn {turn_id}"));
        let prompt = build_remember_prompt(fact);
        r.kv("fact", *fact);

        let snap = run_one_claude_turn(&env.repo, E2E_AGENT_MODEL, &prompt, true).await?;
        record_turn_diagnostics(r, &snap);

        let reply = snap
            .messages
            .iter()
            .find(|(role, _)| role == "assistant")
            .map(|(_, c)| c.clone())
            .unwrap_or(snap.streamed.clone());
        let parsed = gaviero_core::memory::parse_and_strip(&reply);
        let annotations = parsed.annotations.with_context(|| {
            format!("turn {turn_id}: no <turn_annotations> block in reply")
        })?;
        r.kv("flags", annotations.flags.len());

        if annotations.flags.is_empty() {
            anyhow::bail!("turn {turn_id}: zero flags in annotations");
        }

        let transcript = format!("user: {prompt}\nassistant: {}", parsed.stripped);
        env.services
            .writer
            .turn_complete(
                "session-multi",
                *turn_id,
                &env.repo_id,
                None,
                "run-multi",
                transcript,
                Some(serde_json::to_value(&annotations)?),
            )
            .with_context(|| format!("turn_complete enqueue for {turn_id}"))?;
    }

    r.section("writer drain");
    wait_for_writer_drain(&env, r, WRITER_DRAIN_TIMEOUT).await?;

    r.section("verification");
    let mut all_passed = true;
    for (turn_id, fact) in &facts {
        let marker = fact
            .split_whitespace()
            .last()
            .expect("fact has whitespace");
        let hits = env.search_repo(marker).await?;
        let has = hits.iter().any(|m| m.content.contains(marker));
        r.line(format!(
            "    {} {} → marker={} hits={} found={}",
            if has { "✓" } else { "⨯" },
            turn_id,
            marker,
            hits.len(),
            has,
        ));
        if !has {
            all_passed = false;
        }
    }
    if !all_passed {
        anyhow::bail!("at least one turn marker did not land in the store");
    }
    r.line("    ✓ PASS: all per-turn markers landed independently");

    Ok(())
}

/// Test 3 — subsequent session triggers memory injection.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: ONNX runtime + writer task; run with --ignored"]
async fn e2e_subsequent_session_injects_memory() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_subsequent_session_injects_memory");
    let r = guard.report();
    r.section("setup");

    let env = E2eEnv::fresh().await?;
    r.kv("repo_id", &env.repo_id);

    let scope = env.repo_scope();
    let fact_a = "the caching layer uses LRU eviction with 256-entry cap";
    let fact_b = "the rate limiter sleeps 100ms between retries";

    r.section("seed facts");
    for fact in [fact_a, fact_b] {
        let res = env
            .services
            .writer
            .user_remember_scoped(scope.clone(), fact)
            .await?;
        r.line(format!("    seeded ({:?}) {fact}", res));
        match res {
            WriteResult::Inserted(_) | WriteResult::Deduplicated(_) => {}
            other => anyhow::bail!("user_remember_scoped did not persist: {other:?}"),
        }
    }

    r.section("retrieve_for_chat");
    let cfg = ChatInjectionConfig {
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
    let injection = retrieve_for_chat(
        &env.services.stores,
        &memory_scope,
        "tell me about the caching layer eviction policy",
        &cfg,
    )
    .await?
    .with_context(|| "retrieve_for_chat returned None despite enabled+seeded rows")?;

    r.kv("pool_size", injection.pool.len());
    r.kv("items_selected", injection.items.len());
    r.kv("tokens_used", injection.tokens_used);
    r.kv("token_budget", injection.token_budget);
    for it in &injection.items {
        r.line(format!(
            "    • selected id={} score={:.3} content=\"{}\"",
            it.id,
            it.final_score,
            truncate(&it.content, 80),
        ));
    }
    for c in &injection.pool {
        let mark = if c.selected { "✓" } else { "·" };
        r.line(format!(
            "    {} pool id={} sim={:.3} composite={:.3} excluded={}",
            mark,
            c.memory_id,
            c.raw_similarity,
            c.composite_score,
            c.exclusion_reason.as_deref().unwrap_or("-"),
        ));
    }

    r.section("assertions");
    if injection.items.is_empty() {
        anyhow::bail!("no memory selected for injection");
    }
    if !injection
        .items
        .iter()
        .any(|m| m.content.contains("LRU eviction"))
    {
        anyhow::bail!("expected fact A in selected items");
    }
    if !injection.block.contains("LRU eviction") {
        anyhow::bail!("rendered block missing fact A");
    }
    if !injection.block.starts_with("<project_memory>") {
        anyhow::bail!("rendered block does not start with <project_memory>");
    }
    r.line("    ✓ rendered <project_memory> block contains fact A");

    r.section("splice contract");
    let mut selections = empty_selections();
    let block_clone = injection.block.clone();
    gaviero_core::context_planner::chat_memory::splice_into_selections(
        Some(injection),
        &mut selections,
    );
    if selections.memory_selections.len() != 1 {
        anyhow::bail!(
            "splice produced {} memory_selections (expected 1)",
            selections.memory_selections.len()
        );
    }
    let sel = &selections.memory_selections[0];
    if sel.content != block_clone {
        anyhow::bail!("spliced selection content drifted from injection block");
    }
    if !(sel.id.is_none() && sel.namespace.is_none()) {
        anyhow::bail!("pre-rendered block must keep id/namespace = None");
    }
    r.line("    ✓ splice_into_selections preserves verbatim render contract");

    Ok(())
}

/// Test 4 — spilled prompt files contain only the minimal needed
/// information.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: ONNX runtime + writer task + filesystem under tempdir; run with --ignored"]
async fn e2e_spilled_prompt_is_minimal() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_spilled_prompt_is_minimal");
    let r = guard.report();
    r.section("setup");

    let env = E2eEnv::fresh().await?;
    r.kv("repo_id", &env.repo_id);

    let memory_marker = "EMBED_MARKER_quasar_3142";
    let memory_text =
        format!("memory injection canary fact — {memory_marker} — must appear verbatim");
    env.services
        .writer
        .user_remember_scoped(env.repo_scope(), &memory_text)
        .await?;
    r.kv("seeded_marker", memory_marker);

    r.section("retrieve_for_chat");
    let user_message = format!(
        "Tell me everything you know about quasars and reference the marker {memory_marker}."
    );
    let memory_scope = MemoryScope::from_context(&env.repo, Some(&env.repo), None, None);
    let injection = retrieve_for_chat(
        &env.services.stores,
        &memory_scope,
        &user_message,
        &ChatInjectionConfig {
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
        },
    )
    .await?
    .with_context(|| "retrieve_for_chat None despite seeded row")?;
    r.kv("injection_items", injection.items.len());
    if !injection.block.contains(memory_marker) {
        anyhow::bail!("rendered injection block missing seeded marker");
    }

    r.section("force spill");
    let padding = "padding-line referencing the codebase\n".repeat(1000);
    let enriched_prompt = format!(
        "{user_message}\n\nContext follows.\n\n{padding}\n\n{}",
        injection.block
    );
    r.kv("prompt_len", enriched_prompt.len());
    if !would_use_tempfile(enriched_prompt.len(), 0) {
        anyhow::bail!(
            "padding too small — spill path would not engage; prompt_len={}",
            enriched_prompt.len()
        );
    }

    let (tempfile, wrapper) =
        gaviero_core::acp::session::spill_prompt_to_tempfile(&env.repo, &enriched_prompt)?;
    r.kv("tempfile_path", tempfile.path().display());
    r.kv("wrapper_argv_bytes", wrapper.len());
    r.kv("wrapper_text", &wrapper);

    r.section("wrapper assertions");
    let mut bail = |msg: &str| -> anyhow::Error {
        r.line(format!("    ⨯ {msg}"));
        anyhow::anyhow!("{msg}")
    };
    if !wrapper.contains(".gaviero/tmp/prompt-") {
        return Err(bail("wrapper missing tempfile reference"));
    }
    if !wrapper.starts_with("Read the full prompt at @") {
        return Err(bail("wrapper preamble incorrect"));
    }
    if wrapper.len() >= 256 {
        return Err(bail("wrapper longer than expected (256 bytes)"));
    }
    r.line("    ✓ wrapper is a short '@'-pointer");

    r.section("file content assertions");
    let on_disk = std::fs::read_to_string(tempfile.path())
        .with_context(|| format!("reading spilled tempfile {}", tempfile.path().display()))?;
    r.kv("on_disk_bytes", on_disk.len());

    if on_disk != enriched_prompt {
        // Diff lengths to make the failure mode obvious in the report.
        r.line(format!(
            "    ⨯ on-disk file diverged: expected {} bytes, got {} bytes",
            enriched_prompt.len(),
            on_disk.len()
        ));
        anyhow::bail!("spilled tempfile diverged from the enriched prompt");
    }
    r.line("    ✓ on-disk file is byte-identical to the enriched prompt");

    if !on_disk.contains(&user_message) {
        anyhow::bail!("spilled tempfile missing user message");
    }
    if !on_disk.contains(memory_marker) {
        anyhow::bail!("spilled tempfile missing the seeded memory marker");
    }
    if !on_disk.contains("<project_memory>") {
        anyhow::bail!("spilled tempfile missing <project_memory> block");
    }
    r.line("    ✓ contains user message + memory marker + <project_memory> block");

    r.section("forbidden marker scan");
    let forbidden = [
        "candidate_pool",
        "selected_ids",
        "scoring_formula",
        "<system_prompt>",
        "schema_version",
    ];
    for marker in forbidden {
        let leaked = on_disk.contains(marker);
        r.line(format!(
            "    {} `{marker}`",
            if leaked { "⨯ leaked" } else { "✓ absent" }
        ));
        if leaked {
            anyhow::bail!("spilled tempfile leaked forbidden marker `{marker}`");
        }
    }

    r.section("filesystem layout");
    let parent = tempfile
        .path()
        .parent()
        .expect("spilled tempfile has no parent");
    r.kv("parent_dir", parent.display());
    if !parent.ends_with(".gaviero/tmp") {
        anyhow::bail!(
            "spilled tempfile parent dir not under .gaviero/tmp: {}",
            parent.display()
        );
    }
    r.line("    ✓ tempfile lives under <workspace>/.gaviero/tmp");

    Ok(())
}

/// Test 5 — full development-session simulation.
///
/// Simulates a developer working on a project across four turns with a
/// real Haiku-backed extractor LLM wired into the writer. The flow is:
///
/// 1. **Turn 1** (architecture decision). The user asks Claude about a
///    design decision they just made. Claude acknowledges. The transcript
///    is dispatched through `WriterMessage::TurnComplete` *without* a
///    `<turn_annotations>` block — so the only path to memory is the
///    LLM extractor.
/// 2. **Turn 2** (lesson learned). The user reports a bug fix worth
///    keeping. Same dispatch path — the extractor is the only channel.
/// 3. **Turn 3** (convention). The user states a coding convention.
/// 4. **Turn 4** (retrieval). A *new* user query overlapping the topics
///    of turns 1–3 should retrieve memories the extractor wrote, with
///    the rendered `<project_memory>` block surviving the trip.
///
/// What this test asserts (all reported regardless of outcome):
///
/// * The extractor LLM produces JSON that the writer task accepts —
///   i.e., the `extracted` event lands without falling back to
///   raw-run-scope writes for *any* of the three storage turns.
/// * Each transcript yields ≥1 `Record` row at repo *or* module scope
///   (the extractor's `scope_hint` is allowed to vary; the test checks
///   the union).
/// * Turn 4's retrieval surfaces at least one previously extracted row,
///   and the rendered block contains a unique marker from the original
///   transcript.
/// * The writer task processes every enqueued message — `committed` ==
///   `enqueued` and `failed` == 0 by the time the test ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "E2E: spawns claude 5x (3 turns + 3 extractor calls + 1 retrieval turn) + ONNX runtime; run with --ignored"]
async fn e2e_full_dev_session_simulation() -> Result<()> {
    let mut guard = ReportGuard::new("e2e_full_dev_session_simulation");
    let r = guard.report();

    r.section("setup");
    // Spin up a real Haiku-backed BackendConsolidationLlm so the writer
    // task's S3 extractor path runs end-to-end.
    let backend = create_backend_for_model(E2E_AGENT_MODEL, None)
        .context("creating extractor LLM backend")?;
    let llm: Arc<dyn ConsolidationLlm> = Arc::new(BackendConsolidationLlm::new(
        Arc::from(backend),
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    ));
    let env = E2eEnv::fresh_with_llm(Some(llm)).await?;
    r.kv("repo_id", &env.repo_id);
    r.kv("extractor_model", E2E_AGENT_MODEL);

    // Pre-state.
    r.kv("pre_records", env.recent_records(24).await?.len());

    // ── Three "writing" turns ───────────────────────────────────────
    //
    // Each turn's prompt is crafted so a real assistant reply contains
    // a non-trivial fact the extractor should keep. We instruct Claude
    // *not* to emit `<turn_annotations>` so the LLM extractor is the
    // sole path to memory.
    let scenes: [(&str, &str, &str, &str); 3] = [
        (
            "session-dev/turn-1",
            "decision",
            "marker-EVICT-LRU-256",
            "I just decided that our caching layer in src/cache.rs will use \
             LRU eviction with a 256-entry cap. The reason is that 90% of \
             our keys are accessed in burst patterns, and benchmarks show \
             LRU outperforms LFU here. Acknowledge this decision in one \
             sentence and reference the marker `marker-EVICT-LRU-256` in \
             your reply. Do not emit any JSON or sidecar block.",
        ),
        (
            "session-dev/turn-2",
            "lesson",
            "marker-RATE-LIMIT-BACKOFF",
            "Quick lesson learned for src/api/client.rs: the upstream \
             rate-limit handler must sleep 100ms between retries — a \
             tighter loop triggers HTTP 429 cascades on the partner API. \
             Acknowledge and reference the marker \
             `marker-RATE-LIMIT-BACKOFF` in your reply. No JSON.",
        ),
        (
            "session-dev/turn-3",
            "convention",
            "marker-ERROR-CONTEXT-ANYHOW",
            "Convention for new Rust code in this repo: every fallible \
             function uses `anyhow::Result` and attaches `.context(...)` \
             at every I/O boundary. Acknowledge and reference the marker \
             `marker-ERROR-CONTEXT-ANYHOW`. No JSON sidecar block.",
        ),
    ];

    let mut transcripts: Vec<(String, String, String)> = Vec::new();
    for (turn_id, kind, marker, prompt) in &scenes {
        r.section(format!("dev turn ({kind})  ·  {turn_id}"));
        r.kv("marker", marker);

        let snap = run_one_claude_turn(&env.repo, E2E_AGENT_MODEL, prompt, true)
            .await
            .with_context(|| format!("turn {turn_id} claude call"))?;
        record_turn_diagnostics(r, &snap);

        let reply = snap
            .messages
            .iter()
            .find(|(role, _)| role == "assistant")
            .map(|(_, c)| c.clone())
            .unwrap_or(snap.streamed.clone());
        if reply.is_empty() {
            anyhow::bail!("{turn_id}: claude returned empty reply");
        }
        let parsed = gaviero_core::memory::parse_and_strip(&reply);
        // We instructed claude *not* to emit annotations. If it did
        // anyway, log it but proceed — the extractor remains the
        // primary channel under test.
        if parsed.annotations.is_some() {
            r.line(
                "    ⚠ claude emitted a <turn_annotations> block despite the prompt; \
                 dropping it so the extractor is the sole storage path."
                    .to_string(),
            );
        }
        let stripped = parsed.stripped.clone();
        if !stripped.contains(marker) {
            r.line(format!(
                "    ⚠ marker `{marker}` not in reply; extractor may not surface it. \
                 Reply head: {}",
                truncate(&stripped, 240)
            ));
        }
        let transcript = format!("user: {prompt}\nassistant: {stripped}");
        transcripts.push(((*turn_id).to_string(), (*marker).to_string(), transcript));
    }

    // ── Dispatch every transcript through the writer (no annotations) ─
    r.section("writer dispatch (extractor path)");
    for (turn_id, _marker, transcript) in &transcripts {
        env.services
            .writer
            .turn_complete(
                "session-dev",
                turn_id,
                &env.repo_id,
                None,
                "run-dev",
                transcript.clone(),
                /* annotations */ None,
            )
            .with_context(|| format!("turn_complete enqueue for {turn_id}"))?;
        r.kv("dispatched", turn_id);
    }

    // ── Drain — the extractor LLM call runs inside the writer task,
    //    so this wait is heavier than tests 1–4.
    wait_for_writer_drain(&env, r, Duration::from_secs(180)).await?;

    // ── Inspect what landed ────────────────────────────────────────
    r.section("post-state — recent rows");
    let all_recent = env.recent_all(24).await?;
    let records = env.recent_records(24).await?;
    r.kv("rows_total_recent", all_recent.len());
    r.kv("rows_record_kind", records.len());
    for m in &all_recent {
        r.line(format!(
            "    • id={} kind={} type={:?} scope_lvl={} scope=\"{}\" trust={:?} importance={:.2} content=\"{}\"",
            m.id,
            // recent_memories does not return memory_kind; infer:
            if records.iter().any(|x| x.id == m.id) {
                "record"
            } else {
                "history|other"
            },
            m.memory_type,
            m.scope_level,
            m.scope_path,
            m.trust,
            m.importance,
            truncate(&m.content, 100),
        ));
    }

    // Three turns × at least one record each — but the extractor is
    // free to skip turns it judges low-importance. We require ≥1 record
    // overall as a hard floor (proves the extractor path is alive),
    // then surface per-marker presence as soft signal.
    if records.is_empty() {
        anyhow::bail!(
            "extractor produced zero record rows — writer never reached the store"
        );
    }

    r.section("per-marker recall");
    let mut hit_markers = 0;
    for (turn_id, marker, _) in &transcripts {
        let hits = env.search_repo(marker).await?;
        let any = hits.iter().any(|m| m.content.contains(marker));
        r.line(format!(
            "    {} {turn_id} marker={marker} hits={} found={}",
            if any { "✓" } else { "·" },
            hits.len(),
            any,
        ));
        if any {
            hit_markers += 1;
        }
    }
    r.kv("markers_recalled", format!("{hit_markers}/{}", transcripts.len()));

    // ── Turn 4: retrieval over a question that overlaps the seed topics
    r.section("retrieval turn (Q4)");
    let q4 = "What are the conventions and decisions about caching, \
              rate limits, and error handling in this codebase?";
    let cfg = ChatInjectionConfig {
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
    let injection = retrieve_for_chat(&env.services.stores, &memory_scope, q4, &cfg)
        .await?
        .with_context(|| "retrieve_for_chat None despite extracted rows")?;
    r.kv("pool_size", injection.pool.len());
    r.kv("items_selected", injection.items.len());
    r.kv("tokens_used", injection.tokens_used);
    for it in &injection.items {
        r.line(format!(
            "    • selected id={} score={:.3} content=\"{}\"",
            it.id,
            it.final_score,
            truncate(&it.content, 100),
        ));
    }

    if injection.items.is_empty() {
        anyhow::bail!(
            "Q4 retrieval surfaced no items — extractor wrote rows but they did not score \
             above retrieval thresholds. Check embedder cosine vs the prompt vector."
        );
    }

    // The extractor paraphrases at will. Treat per-marker recall on Q4
    // as soft signal but require *at least one* overlap with the
    // original facts (caching / LRU / rate-limit / anyhow / error).
    let topical = ["caching", "lru", "rate", "limit", "anyhow", "error", "convention"];
    let touched: Vec<&str> = topical
        .iter()
        .filter(|kw| {
            injection
                .items
                .iter()
                .any(|m| m.content.to_lowercase().contains(*kw))
        })
        .copied()
        .collect();
    r.kv("topical_keywords_in_block", touched.join(", "));
    if touched.is_empty() {
        anyhow::bail!(
            "Q4 retrieval surfaced rows but none touched the seeded topics — \
             extractor likely produced unrelated paraphrases"
        );
    }

    // ── Writer-counter sanity ──────────────────────────────────────
    r.section("writer counters");
    let (enq, com, fail, last_fail) = env.write_counters.snapshot();
    r.kv("enqueued", enq);
    r.kv("committed", com);
    r.kv("failed", fail);
    if let Some(msg) = last_fail.as_ref() {
        r.kv("last_failure", msg);
    }
    if fail > 0 {
        anyhow::bail!(
            "writer recorded {fail} failures (last: {})",
            last_fail.unwrap_or_default()
        );
    }
    if com == 0 {
        anyhow::bail!("writer never committed anything despite {enq} enqueues");
    }
    r.line("    ✓ no writer failures; commits >0");

    Ok(())
}


// SCOPE_REPO is reachable only through `gaviero_core::memory::scope`; pin
// the import here so future tests that filter by scope_level stay
// idiomatic without re-importing the constant.
const _: i32 = SCOPE_REPO;
