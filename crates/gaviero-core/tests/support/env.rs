//! Lifted from `tests/memory_session_e2e.rs`. The original test file
//! kept these as private items; T1.2 promotes them to `pub` so the new
//! `tests/memory_testbed_e2e.rs` can reuse the same plumbing without
//! copy-pasting.

use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::oneshot;

use gaviero_core::acp::protocol::StreamEvent;
use gaviero_core::acp::session::{AcpSession, AgentOptions};
use gaviero_core::context_planner::types::{ContinuityMode, PlannerMetadata, PlannerSelections};
use gaviero_core::memory::{
    ConsolidationLlm, MemoryKind, MemoryObserver, MemoryServices, ScoredMemory, SearchConfig,
    ServicesOpts, WriteResult, WriteScope, hash_path, scope::MemoryScope,
};
use gaviero_core::observer::AcpObserver;
use gaviero_core::workspace::Workspace;

// ── Constants ────────────────────────────────────────────────────────────────

/// Hard ceiling per Claude turn. CI should never wait longer than this for
/// a single turn even on a slow box.
pub const PER_TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// Idle timeout between stream events from `claude --output-format stream-json`.
pub const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Wall-clock cap for waiting on the writer to drain its queue.
pub const WRITER_DRAIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Cheap, fast model used for both the agent turns and the extractor LLM.
pub const E2E_AGENT_MODEL: &str = "claude:haiku";

// ── Reporting ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct TestReport {
    name: String,
    started_at: Option<Instant>,
    sections: Vec<(String, Vec<String>)>,
}

impl TestReport {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            started_at: Some(Instant::now()),
            sections: Vec::new(),
        }
    }

    pub fn section(&mut self, title: impl Into<String>) {
        self.sections.push((title.into(), Vec::new()));
    }

    pub fn line(&mut self, line: impl Into<String>) {
        if self.sections.is_empty() {
            self.sections.push(("misc".to_string(), Vec::new()));
        }
        let last = self.sections.last_mut().unwrap();
        last.1.push(line.into());
    }

    pub fn kv(&mut self, key: &str, value: impl std::fmt::Display) {
        self.line(format!("  {key:>32} = {value}"));
    }

    pub fn print(&self) {
        let elapsed = self
            .started_at
            .map(|s| s.elapsed())
            .unwrap_or_default();
        let mut out = String::new();
        out.push_str(
            "\n┌──────────────────────────────────────────────────────────────────────────\n",
        );
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
        print!("{out}");
    }
}

pub struct ReportGuard {
    report: TestReport,
}

impl ReportGuard {
    pub fn new(name: &str) -> Self {
        Self {
            report: TestReport::new(name),
        }
    }
    pub fn report(&mut self) -> &mut TestReport {
        &mut self.report
    }
}

impl Drop for ReportGuard {
    fn drop(&mut self) {
        self.report.print();
    }
}

// ── Test harness ─────────────────────────────────────────────────────────────

pub struct E2eEnv {
    pub _tmp: tempfile::TempDir,
    pub repo: PathBuf,
    pub services: Arc<MemoryServices>,
    pub repo_id: String,
    pub write_counters: Arc<WriteCounters>,
}

impl E2eEnv {
    pub async fn fresh_with_llm(llm: Option<Arc<dyn ConsolidationLlm>>) -> Result<Self> {
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
            tokio::task::spawn_blocking(move || MemoryServices::open(&repo_b, &workspace_b, opts))
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

    pub async fn fresh() -> Result<Self> {
        Self::fresh_with_llm(None).await
    }

    pub fn repo_scope(&self) -> WriteScope {
        WriteScope::Repo {
            repo_id: self.repo_id.clone(),
        }
    }

    pub async fn search_repo(&self, query: &str) -> Result<Vec<ScoredMemory>> {
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

    pub async fn recent_records(&self, hours: u32) -> Result<Vec<ScoredMemory>> {
        let folder_kind = self.repo_scope().target_store();
        let store = self.services.stores.get(&folder_kind).await?;
        store
            .recent_memories_by_kind(MemoryKind::Record, hours, 32)
            .await
    }

    pub async fn recent_all(&self, hours: u32) -> Result<Vec<ScoredMemory>> {
        let folder_kind = self.repo_scope().target_store();
        let store = self.services.stores.get(&folder_kind).await?;
        store.recent_memories(hours, 64).await
    }
}

#[derive(Default)]
pub struct WriteCounters {
    pub enqueued: AtomicUsize,
    pub committed: AtomicUsize,
    pub failed: AtomicUsize,
    pub last_failure: Mutex<Option<String>>,
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
    pub fn snapshot(&self) -> (usize, usize, usize, Option<String>) {
        (
            self.enqueued.load(Ordering::Relaxed),
            self.committed.load(Ordering::Relaxed),
            self.failed.load(Ordering::Relaxed),
            self.last_failure.lock().unwrap().clone(),
        )
    }
}

#[derive(Default)]
pub struct CapturingObserver {
    pub inner: Mutex<CapturingInner>,
}

#[derive(Default, Clone)]
pub struct CapturingInner {
    pub streamed: String,
    pub messages: Vec<(String, String)>,
    pub tool_calls: Vec<String>,
    pub claude_session_id: Option<String>,
    pub denied_permissions: Vec<String>,
}

impl CapturingObserver {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn snapshot(&self) -> CapturingInner {
        self.inner.lock().unwrap().clone()
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
/// observer state.
pub async fn run_one_claude_turn(
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
        "",
        &[],
        &[],
        &options,
        &[],
        &[],
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

pub const TURN_ANNOTATIONS_INSTRUCTION: &str = r#"
You MUST end your response with a JSON sidecar block exactly like:

<turn_annotations>
{"v":1,"flags":[{"type":"convention","importance":0.7,"scope":"repo","text":"<one short fact>","refs":[]}],"session_thread":"e2e","open_questions":[]}
</turn_annotations>

Use the `text` value provided by the user verbatim. Do not paraphrase it. Output the block exactly once, immediately at the end of your reply.
"#;

pub fn build_remember_prompt(fact: &str) -> String {
    format!(
        "{}\n\nFact to record verbatim: {}\n\nReply with a single short sentence acknowledging the fact, then the required block.",
        TURN_ANNOTATIONS_INSTRUCTION.trim(),
        fact,
    )
}

/// Spin until the writer has drained all enqueued messages.
pub async fn wait_for_writer_drain(
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
            if com + fail >= enq {
                r.kv("writer_enq/com/fail", format!("{enq}/{com}/{fail}"));
                if let Some(msg) = last_fail {
                    r.kv("writer_last_failure", msg);
                }
                return Ok(());
            }
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

pub fn record_turn_diagnostics(r: &mut TestReport, snap: &CapturingInner) {
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

pub fn record_search_hits(r: &mut TestReport, label: &str, hits: &[ScoredMemory]) {
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

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.replace('\n', " ⏎ ");
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out.replace('\n', " ⏎ ")
}

pub fn empty_selections() -> PlannerSelections {
    PlannerSelections {
        memory_selections: vec![],
        graph_selections: vec![],
        skill_selections: vec![],
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
