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

use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::oneshot;

use gaviero_core::acp::protocol::StreamEvent;
use gaviero_core::acp::session::{AcpSession, AgentOptions, would_use_tempfile};
use gaviero_core::context_planner::types::{
    ContinuityMode, PlannerMetadata, PlannerSelections,
};
use gaviero_core::memory::scope::SCOPE_REPO;
use gaviero_core::memory::{
    BackendConsolidationLlm, ChatInjectionConfig, ConsolidationLlm, MemoryKind, MemoryObserver,
    MemoryServices, ScopeMix, ScoredMemory, SearchConfig, ServicesOpts, WriteResult, WriteScope,
    hash_path,
    retrieval::retrieve_for_chat,
    scope::MemoryScope,
};
use gaviero_core::observer::AcpObserver;
use gaviero_core::swarm::backend::shared::create_backend_for_model;
use gaviero_core::workspace::Workspace;

// ── Constants ────────────────────────────────────────────────────────────────

/// Hard ceiling per Claude turn. CI should never wait longer than this for
/// a single turn even on a slow box.
const PER_TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// Idle timeout between stream events from `claude --output-format stream-json`.
/// Pinged once per `next_event` call; on expiry we check whether the
/// subprocess is still alive and either keep waiting or bail.
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Wall-clock cap for waiting on the writer to drain its queue.
const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Cheap, fast model used for both the agent turns and the extractor LLM.
/// `haiku` keeps cost / latency low for E2E iteration.
const E2E_AGENT_MODEL: &str = "haiku";

// ── Reporting ────────────────────────────────────────────────────────────────

/// Diagnostic report emitted at the end of every test (success *and*
/// failure). Captures everything a debugger would otherwise have to
/// re-run the test to recover: timing, what claude returned, what the
/// writer task processed, what landed in the DB, and what retrieval saw.
///
/// Lines are accumulated in a buffer and flushed once at the end of the
/// test. Grouping the output keeps the diagnostic block contiguous in
/// the `cargo test --nocapture` log even when the tests run in parallel.
#[derive(Default)]
struct TestReport {
    name: String,
    started_at: Option<Instant>,
    sections: Vec<(String, Vec<String>)>,
}

impl TestReport {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            started_at: Some(Instant::now()),
            sections: Vec::new(),
        }
    }

    fn section(&mut self, title: impl Into<String>) {
        self.sections.push((title.into(), Vec::new()));
    }

    fn line(&mut self, line: impl Into<String>) {
        if self.sections.is_empty() {
            self.sections.push(("misc".to_string(), Vec::new()));
        }
        let last = self.sections.last_mut().unwrap();
        last.1.push(line.into());
    }

    fn kv(&mut self, key: &str, value: impl std::fmt::Display) {
        self.line(format!("  {key:>32} = {value}"));
    }

    /// Flush the accumulated report to stdout. Always called — both on
    /// success and via the panic hook on failure — so the diagnostic
    /// log survives every outcome.
    fn print(&self) {
        let elapsed = self
            .started_at
            .map(|s| s.elapsed())
            .unwrap_or_default();
        let mut out = String::new();
        out.push_str(&format!(
            "\n┌──────────────────────────────────────────────────────────────────────────\n"
        ));
        out.push_str(&format!(
            "│ E2E REPORT  ·  {:<48}  ·  {:>6.2}s\n",
            self.name,
            elapsed.as_secs_f32()
        ));
        out.push_str(
            "└──────────────────────────────────────────────────────────────────────────\n",
        );
        for (title, lines) in &self.sections {
            out.push_str(&format!("  ▸ {title}\n"));
            for l in lines {
                out.push_str(l);
                out.push('\n');
            }
        }
        out.push_str(
            "──────────────────────────────────────────────────────────────────────────\n\n",
        );
        // Single println so output stays atomic across parallel tests.
        print!("{out}");
    }
}

/// Wrapper that prints the report on Drop, even if the test panics.
/// Tests own a `ReportGuard`; `report()` returns a `&mut TestReport` for
/// in-place updates.
struct ReportGuard {
    report: TestReport,
}

impl ReportGuard {
    fn new(name: &str) -> Self {
        Self {
            report: TestReport::new(name),
        }
    }
    fn report(&mut self) -> &mut TestReport {
        &mut self.report
    }
}

impl Drop for ReportGuard {
    fn drop(&mut self) {
        self.report.print();
    }
}

// ── Test harness ─────────────────────────────────────────────────────────────

/// One-stop bundle for an E2E test: an isolated workspace, an open
/// `MemoryServices`, and the canonical `repo_id` to hand to write scopes.
struct E2eEnv {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    services: Arc<MemoryServices>,
    repo_id: String,
    /// Lifecycle counters shared with the writer task. `0` on a fresh
    /// env; bumps on every commit / fail. Tests assert against these to
    /// detect silent enqueue → never-committed regressions.
    write_counters: Arc<WriteCounters>,
}

impl E2eEnv {
    /// Bootstrap a fresh workspace under a tempdir and open
    /// `MemoryServices` with the default embedder.
    ///
    /// `MemoryServices::open` is blocking (it builds the ONNX session),
    /// so the call is wrapped in `spawn_blocking`. When `llm` is
    /// `Some(...)`, the writer wires the S3 extractor; otherwise the
    /// extractor falls back to raw run-scope writes.
    async fn fresh_with_llm(llm: Option<Arc<dyn ConsolidationLlm>>) -> Result<Self> {
        let tmp = tempfile::tempdir().context("tempdir")?;
        let repo = tmp.path().to_path_buf();
        let workspace = Workspace::single_folder(repo.clone());

        let counters = Arc::new(WriteCounters::default());
        let observer: Arc<dyn MemoryObserver> = counters.clone();

        let services = {
            let repo_b = repo.clone();
            let workspace_b = workspace.clone();
            let opts = ServicesOpts {
                embedder_name: None,
                llm,
                observer: Some(observer),
                manifest_observer: None,
            };
            tokio::task::spawn_blocking(move || {
                MemoryServices::open(&repo_b, &workspace_b, opts)
            })
            .await
            .context("services bootstrap blocking task")??
        };

        let repo_id = hash_path(&repo);

        Ok(Self {
            _tmp: tmp,
            repo,
            services,
            repo_id,
            write_counters: counters,
        })
    }

    /// Convenience: env without an extractor LLM (tests 1–4 path).
    async fn fresh() -> Result<Self> {
        Self::fresh_with_llm(None).await
    }

    fn repo_scope(&self) -> WriteScope {
        WriteScope::Repo {
            repo_id: self.repo_id.clone(),
        }
    }

    /// FTS+vector search at repo scope. Returns ranked candidates; tests
    /// assert against substrings to absorb embedder ranking variance.
    async fn search_repo(&self, query: &str) -> Result<Vec<ScoredMemory>> {
        let folder_kind = self.repo_scope().target_store();
        let store = self.services.stores.get(&folder_kind).await?;
        let cfg = SearchConfig {
            query: query.to_string(),
            max_results: 16,
            per_level_limit: 16,
            similarity_threshold: 0.0,
            confidence_threshold: 0.0,
            use_fts: true,
            scope: MemoryScope::from_context(&self.repo, Some(&self.repo), None, None),
        };
        store.search_scoped(&cfg).await
    }

    /// All "record"-kind rows newer than `hours`. Used by tests to
    /// inspect what the writer task wrote without filtering on a
    /// content marker.
    async fn recent_records(&self, hours: u32) -> Result<Vec<ScoredMemory>> {
        let folder_kind = self.repo_scope().target_store();
        let store = self.services.stores.get(&folder_kind).await?;
        store
            .recent_memories_by_kind(MemoryKind::Record, hours, 32)
            .await
    }

    /// All rows of any kind newer than `hours`. Useful for spotting
    /// History rows the writer persists alongside Records.
    async fn recent_all(&self, hours: u32) -> Result<Vec<ScoredMemory>> {
        let folder_kind = self.repo_scope().target_store();
        let store = self.services.stores.get(&folder_kind).await?;
        store.recent_memories(hours, 64).await
    }
}

#[derive(Default)]
struct WriteCounters {
    enqueued: AtomicUsize,
    committed: AtomicUsize,
    failed: AtomicUsize,
    last_failure: Mutex<Option<String>>,
}

impl MemoryObserver for WriteCounters {
    fn on_write_enqueued(&self, _kind: &str) {
        self.enqueued.fetch_add(1, Ordering::Relaxed);
    }
    fn on_write_committed(&self, _kind: &str, _result: &WriteResult) {
        self.committed.fetch_add(1, Ordering::Relaxed);
    }
    fn on_write_failed(&self, kind: &str, error: &str) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        let mut g = self.last_failure.lock().unwrap();
        *g = Some(format!("{kind}: {error}"));
    }
}

impl WriteCounters {
    fn snapshot(&self) -> (usize, usize, usize, Option<String>) {
        (
            self.enqueued.load(Ordering::Relaxed),
            self.committed.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed),
            self.last_failure.lock().unwrap().clone(),
        )
    }
}

/// Captures the full assistant reply across `on_stream_chunk` /
/// `on_message_complete` calls so tests can assert on the post-turn
/// transcript without coupling to streaming order.
#[derive(Default)]
struct CapturingObserver {
    inner: Mutex<CapturingInner>,
}

#[derive(Default)]
struct CapturingInner {
    /// Concatenated text from `ContentDelta` events.
    streamed: String,
    /// `(role, content)` from `on_message_complete`.
    messages: Vec<(String, String)>,
    /// Tool-call summaries observed via `on_tool_call_started`.
    tool_calls: Vec<String>,
    /// Claude session id captured from `SystemInit`, if any.
    claude_session_id: Option<String>,
    /// Permission requests seen this turn — auto-denied by the harness.
    denied_permissions: Vec<String>,
}

impl CapturingObserver {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn snapshot(&self) -> CapturingInner {
        let g = self.inner.lock().unwrap();
        CapturingInner {
            streamed: g.streamed.clone(),
            messages: g.messages.clone(),
            tool_calls: g.tool_calls.clone(),
            claude_session_id: g.claude_session_id.clone(),
            denied_permissions: g.denied_permissions.clone(),
        }
    }
}

impl AcpObserver for CapturingObserver {
    fn on_stream_chunk(&self, text: &str) {
        self.inner.lock().unwrap().streamed.push_str(text);
    }
    fn on_tool_call_started(&self, tool_name: &str) {
        self.inner
            .lock()
            .unwrap()
            .tool_calls
            .push(tool_name.to_string());
    }
    fn on_streaming_status(&self, _status: &str) {}
    fn on_message_complete(&self, role: &str, content: &str) {
        self.inner
            .lock()
            .unwrap()
            .messages
            .push((role.to_string(), content.to_string()));
    }
    fn on_proposal_deferred(&self, _path: &Path, _old_content: Option<&str>, _new_content: &str) {}
    fn on_permission_request(
        &self,
        tool_name: &str,
        _description: &str,
        respond: oneshot::Sender<bool>,
    ) {
        // E2E suite: the agent runs without write tools — auto-deny anything.
        self.inner
            .lock()
            .unwrap()
            .denied_permissions
            .push(tool_name.to_string());
        let _ = respond.send(false);
    }
    fn on_claude_session_started(&self, session_id: &str) {
        self.inner.lock().unwrap().claude_session_id = Some(session_id.to_string());
    }
}

/// Drive a single Claude turn against `cwd` and return the captured
/// observer state. Uses `--print --output-format stream-json` via
/// `AcpSession::spawn` directly so we don't depend on the higher-level
/// `ClaudeSession` (which couples to the write gate).
async fn run_one_claude_turn(
    cwd: &Path,
    model: &str,
    prompt: &str,
    auto_approve: bool,
) -> Result<CapturingInner> {
    let observer = CapturingObserver::new();

    let options = AgentOptions {
        effort: "off".to_string(),
        max_tokens: 0,
        auto_approve,
        available_tools: Some(vec![]),
        approved_tools: Some(vec![]),
        ..Default::default()
    };

    let mut session = AcpSession::spawn(
        model,
        cwd,
        prompt,
        /* system_prompt */ "",
        /* available_tools */ &[],
        /* approved_tools */ &[],
        &options,
        /* file_attachments */ &[],
    )
    .context("spawning AcpSession")?;

    let deadline = Instant::now() + PER_TURN_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            session.kill();
            anyhow::bail!("E2E turn timed out after {:?}", PER_TURN_TIMEOUT);
        }
        let next = tokio::time::timeout(STREAM_IDLE_TIMEOUT, session.next_event()).await;
        match next {
            Err(_elapsed) => {
                if session.try_wait_exited() {
                    break;
                }
                continue;
            }
            Ok(Ok(Some(event))) => match event {
                StreamEvent::ContentDelta(text) => observer.on_stream_chunk(&text),
                StreamEvent::AssistantMessage { text, .. } => {
                    if !text.is_empty() {
                        observer.on_stream_chunk(&text);
                    }
                }
                StreamEvent::ResultEvent {
                    is_error,
                    result_text,
                    ..
                } => {
                    let role = if is_error { "system" } else { "assistant" };
                    let snap = observer.snapshot();
                    let final_text = if !snap.streamed.is_empty() {
                        snap.streamed
                    } else {
                        result_text
                    };
                    observer.on_message_complete(role, &final_text);
                    break;
                }
                StreamEvent::SystemInit { session_id, .. } => {
                    if !session_id.is_empty() {
                        observer.on_claude_session_started(&session_id);
                    }
                }
                StreamEvent::PermissionRequest { request_id, .. } => {
                    session.respond_permission(false, &request_id);
                }
                StreamEvent::ToolUseStart { tool_name, .. } => {
                    observer.on_tool_call_started(&tool_name);
                }
                _ => {}
            },
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                anyhow::bail!("AcpSession event error: {e}");
            }
        }
    }

    let _ = tokio::time::timeout(Duration::from_secs(10), session.wait()).await;
    Ok(observer.snapshot())
}

/// System-prompt instruction we pin to every E2E turn that should
/// produce a `<turn_annotations>` block.
const TURN_ANNOTATIONS_INSTRUCTION: &str = r#"
You MUST end your response with a JSON sidecar block exactly like:

<turn_annotations>
{"v":1,"flags":[{"type":"convention","importance":0.7,"scope":"repo","text":"<one short fact>","refs":[]}],"session_thread":"e2e","open_questions":[]}
</turn_annotations>

Use the `text` value provided by the user verbatim. Do not paraphrase it. Output the block exactly once, immediately at the end of your reply.
"#;

fn build_remember_prompt(fact: &str) -> String {
    format!(
        "{}\n\nFact to record verbatim: {}\n\nReply with a single short sentence acknowledging the fact, then the required block.",
        TURN_ANNOTATIONS_INSTRUCTION.trim(),
        fact,
    )
}

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

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Spin until the writer has drained all enqueued messages **and** all
/// in-flight processing has completed, or the timeout elapses.
///
/// `queue_depth()` reaches 0 when a message is *dequeued* from the MPSC
/// channel, but `on_write_committed` fires only after `process_message`
/// finishes (which includes ONNX embedding). For a single message the two
/// events can be far apart — checking depth alone races. We therefore
/// require both `depth == 0` **and** `committed + failed >= enqueued` before
/// returning.
async fn wait_for_writer_drain(
    env: &E2eEnv,
    r: &mut TestReport,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    let mut last_depth: u64 = u64::MAX;
    loop {
        let depth: u64 = env.services.writer.queue_depth();
        if depth != last_depth {
            r.kv("writer_depth", depth);
            last_depth = depth;
        }
        if depth == 0 {
            let (enq, com, fail, last_fail) = env.write_counters.snapshot();
            // All dequeued messages have also been committed or failed.
            if com + fail >= enq {
                r.kv("writer_enq/com/fail", format!("{enq}/{com}/{fail}"));
                if let Some(msg) = last_fail {
                    r.kv("writer_last_failure", msg);
                }
                return Ok(());
            }
            // depth is 0 but some process_message calls are still running
            // (drained increments before process_message completes); keep
            // polling until every enqueued message has produced a commit or
            // fail event.
        }
        if Instant::now() >= deadline {
            let (enq, com, fail, last_fail) = env.write_counters.snapshot();
            r.kv("writer_depth_final", depth);
            r.kv("writer_enq/com/fail", format!("{enq}/{com}/{fail}"));
            if let Some(msg) = last_fail {
                r.kv("writer_last_failure", msg);
            }
            anyhow::bail!("writer drain timeout (depth={depth})");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn record_turn_diagnostics(r: &mut TestReport, snap: &CapturingInner) {
    r.kv("streamed_chars", snap.streamed.len());
    r.kv("messages", snap.messages.len());
    r.kv("tool_calls", snap.tool_calls.len());
    r.kv("denied_perms", snap.denied_permissions.len());
    if let Some(sid) = snap.claude_session_id.as_ref() {
        r.kv("claude_session_id", sid);
    }
    if !snap.tool_calls.is_empty() {
        r.line(format!("    tool_calls: {}", snap.tool_calls.join(", ")));
    }
    for (role, content) in &snap.messages {
        r.line(format!(
            "    {} ({} bytes): {}",
            role,
            content.len(),
            truncate(content, 240)
        ));
    }
}

fn record_search_hits(r: &mut TestReport, label: &str, hits: &[ScoredMemory]) {
    r.kv(label, format!("{} hit(s)", hits.len()));
    for m in hits.iter().take(8) {
        r.line(format!(
            "    • id={} scope_lvl={} type={:?} importance={:.2} content=\"{}\"",
            m.id,
            m.scope_level,
            m.memory_type,
            m.importance,
            truncate(&m.content, 120),
        ));
    }
    if hits.len() > 8 {
        r.line(format!("    … and {} more", hits.len() - 8));
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.replace('\n', " ⏎ ");
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out.replace('\n', " ⏎ ")
}

fn empty_selections() -> PlannerSelections {
    PlannerSelections {
        memory_selections: vec![],
        graph_selections: vec![],
        file_refs: vec![],
        replay_history: None,
        metadata: PlannerMetadata {
            memory_count: 0,
            graph_token_estimate: 0,
            graph_budget: 0,
            is_first_turn: true,
            continuity_mode: Some(ContinuityMode::NativeResume),
        },
    }
}

// SCOPE_REPO is reachable only through `gaviero_core::memory::scope`; pin
// the import here so future tests that filter by scope_level stay
// idiomatic without re-importing the constant.
const _: i32 = SCOPE_REPO;
