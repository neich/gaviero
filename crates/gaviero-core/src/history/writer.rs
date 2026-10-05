//! The single per-workspace history recorder.
//!
//! One `HistoryRecorder` exists per application instance (plan invariant
//! 4: *no dual writers*). It owns the file, the open-turn table, the
//! per-turn latches (assistant output, bootstrap measurement, exact usage,
//! announced tool calls), and the deferred `turn_end` slot; every capture
//! point goes through it.
//!
//! Invariants this type has to keep (from the plan):
//!
//! 1. **Write-only.** Nothing reads through the recorder, and a write
//!    failure is logged at `warn!` and never propagated — a missing or
//!    broken log must never affect a turn.
//! 2. **Cheap.** Append-only, no `fsync`, no pretty-printing; the tool
//!    response path waits on this. The state lock is never held across the
//!    file append.
//! 3. **One `turn_end` per turn.** Ending a turn removes it from the open
//!    table, so a second end is a no-op; a turn whose provider usage has
//!    not landed yet can defer its line until [`HistoryRecorder::note_usage`].
//! 4. **Estimates are labelled.** Every record that carries an estimate
//!    names its estimator; exact provider usage is labelled
//!    `usage_source: "provider"`.
//!
//! # Why every call names the turn
//!
//! A conversation can start its next turn before the previous turn's task
//! has finished: the TUI clears `is_streaming` when the final message lands,
//! which is *before* the task reports the turn finished. Keying capture by
//! conversation would then close the new turn with the old turn's outcome.
//! Every capture point already holds the `turn_id`, so the recorder keys by
//! it and uses the conversation only for MCP attribution.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::record::{
    Attribution, CaptureMode, HistoryKind, HistoryRecord, McpCall, MemoryInjection, ProviderUsage,
    SCHEMA_VERSION, ToolCall, ToolOutput, TurnEnd, TurnStart, truncate_bytes,
};
use super::tokens::{Estimator, estimate_json_text_tokens, estimate_text_tokens};
use super::{
    HISTORY_MAX_ASSISTANT_BYTES, HISTORY_MAX_BLOCK_BYTES, HISTORY_MAX_BYTES,
    HISTORY_MAX_JSON_BYTES, HISTORY_MAX_PROMPT_BYTES, history_path,
};
use crate::mcp::McpCallLogEntry;
use crate::util::ndjson::NdjsonAppender;

/// A turn that has started and not yet written (or deferred) its `turn_end`.
struct OpenTurn {
    conv_id: String,
    /// Last seq handed out; `turn_start` takes 0.
    last_seq: u32,
    /// True when the host cannot guarantee usage arrives before the turn
    /// ends (an event-driven host). The end is then deferred until usage
    /// lands, so the single `turn_end` line can carry it.
    expects_usage: bool,
    usage: Option<ProviderUsage>,
    /// Tool calls announced by [`HistoryRecorder::tool_started`] whose
    /// completion has not arrived: `(summary, reserved seq)`, oldest first.
    pending_tools: VecDeque<(String, u32)>,
    assistant: AssistantOutput,
    bootstrap_tokens_est: Option<usize>,
}

impl OpenTurn {
    fn new(conv_id: &str, expects_usage: bool) -> Self {
        Self {
            conv_id: conv_id.to_string(),
            last_seq: 0,
            expects_usage,
            usage: None,
            pending_tools: VecDeque::new(),
            assistant: AssistantOutput::default(),
            bootstrap_tokens_est: None,
        }
    }

    fn next_seq(&mut self) -> u32 {
        self.last_seq += 1;
        self.last_seq
    }
}

/// The assistant's output for a turn, accumulated across messages.
#[derive(Default)]
struct AssistantOutput {
    seen: bool,
    bytes: usize,
    tokens_est: usize,
    /// Head of the output, capped at `HISTORY_MAX_ASSISTANT_BYTES`.
    head: String,
    truncated: bool,
}

/// A `turn_end` waiting for its provider usage.
struct DeferredEnd {
    conv_id: String,
    /// The seq the line was assigned when the turn closed.
    seq: u32,
    end: TurnEnd,
}

#[derive(Default)]
struct State {
    /// turn_id → open turn.
    open: HashMap<String, OpenTurn>,
    /// conv_id → the conversation's most recently started turn.
    latest: HashMap<String, String>,
    /// turn_id → deferred turn end.
    deferred: HashMap<String, DeferredEnd>,
}

/// The recorder. Shared through an `Arc`; `Mutex`-guarded state.
pub struct HistoryRecorder {
    appender: NdjsonAppender,
    state: Mutex<State>,
}

impl HistoryRecorder {
    /// Recorder for a workspace root: `<root>/.gaviero/history/turns.ndjson`.
    pub fn for_workspace(workspace_root: &Path) -> Arc<Self> {
        Self::with_path_and_cap(history_path(workspace_root), HISTORY_MAX_BYTES)
    }

    /// Recorder at an explicit path and rotation cap (tests, non-default
    /// workspaces).
    pub fn with_path_and_cap(path: PathBuf, max_bytes: u64) -> Arc<Self> {
        Arc::new(Self {
            appender: NdjsonAppender::new(path, max_bytes),
            state: Mutex::new(State::default()),
        })
    }

    /// The active NDJSON path.
    pub fn path(&self) -> &Path {
        self.appender.path()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open `turn_id` for `conv_id` and write its `turn_start` record
    /// (seq 0).
    ///
    /// Deferred turn ends of the same conversation are flushed first
    /// (without usage), so a turn whose usage never arrived is still closed.
    /// At most the conversation's previous turn stays open alongside the new
    /// one — the task that owns it may still be finishing. Older open turns
    /// of the conversation are forgotten without a `turn_end`, and read back
    /// as incomplete, which is what they are.
    ///
    /// `expects_usage` must be true only for hosts that learn of a turn's
    /// end *before* its provider usage can arrive.
    pub fn begin_turn(&self, conv_id: &str, turn_id: &str, start: TurnStart, expects_usage: bool) {
        let stale = {
            let mut st = self.lock();
            let stale_ids: Vec<String> = st
                .deferred
                .iter()
                .filter(|(_, d)| d.conv_id == conv_id)
                .map(|(t, _)| t.clone())
                .collect();
            let stale: Vec<(String, DeferredEnd)> = stale_ids
                .into_iter()
                .filter_map(|t| st.deferred.remove(&t).map(|d| (t, d)))
                .collect();
            let previous = st.latest.insert(conv_id.to_string(), turn_id.to_string());
            let before = st.open.len();
            st.open
                .retain(|id, t| t.conv_id != conv_id || previous.as_deref() == Some(id.as_str()));
            if st.open.len() < before {
                tracing::debug!(
                    target: "history",
                    conv_id,
                    abandoned = before - st.open.len(),
                    "open turns abandoned without a turn_end",
                );
            }
            st.open
                .insert(turn_id.to_string(), OpenTurn::new(conv_id, expects_usage));
            stale
        };
        for (stale_turn, d) in stale {
            self.append_record(
                Some(&d.conv_id),
                Some(&stale_turn),
                d.seq,
                HistoryKind::TurnEnd(d.end),
            );
        }
        self.append_record(
            Some(conv_id),
            Some(turn_id),
            0,
            HistoryKind::TurnStart(start),
        );
    }

    /// Append one record to an open turn, assigning `ts` and `seq`. Returns
    /// false (and logs) when the turn is not open — the record is dropped
    /// rather than written into a turn that already ended.
    pub fn push(&self, turn_id: &str, payload: HistoryKind) -> bool {
        let slot = self.lock().open.get_mut(turn_id).map(|t| {
            let seq = t.next_seq();
            (t.conv_id.clone(), seq)
        });
        let Some((conv_id, seq)) = slot else {
            tracing::warn!(
                target: "history",
                turn_id,
                kind = payload.kind_str(),
                "turn not open — history record dropped",
            );
            return false;
        };
        self.append_record(Some(&conv_id), Some(turn_id), seq, payload);
        true
    }

    /// Append a record to a turn that has already ended — the review of its
    /// file changes, which the user finalizes later (possibly after a
    /// restart). Sorted last within the turn (`seq = u32::MAX`).
    pub fn push_after_turn(&self, conv_id: Option<&str>, turn_id: &str, payload: HistoryKind) {
        self.append_record(conv_id, Some(turn_id), u32::MAX, payload);
    }

    /// A tool call started and only its one-line summary is known. Reserves
    /// the call's seq so its record keeps its place in the turn even though
    /// it is written later: by [`Self::tool_completed`] when the provider
    /// reports the result, or at [`Self::end_turn`] as a `summary_only`
    /// record when it never does.
    pub fn tool_started(&self, turn_id: &str, summary: &str) -> bool {
        let mut st = self.lock();
        let Some(turn) = st.open.get_mut(turn_id) else {
            return false;
        };
        let seq = turn.next_seq();
        turn.pending_tools.push_back((summary.to_string(), seq));
        true
    }

    /// A tool call finished with its raw arguments and/or result. Takes the
    /// seq reserved by the oldest [`Self::tool_started`] with the same
    /// summary, or a fresh one when the start was never announced.
    pub fn tool_completed(&self, turn_id: &str, call: ToolCall) -> bool {
        let slot = {
            let mut st = self.lock();
            st.open.get_mut(turn_id).map(|turn| {
                let reserved = call.summary.as_deref().and_then(|summary| {
                    let idx = turn.pending_tools.iter().position(|(s, _)| s == summary)?;
                    turn.pending_tools.remove(idx).map(|(_, seq)| seq)
                });
                let seq = reserved.unwrap_or_else(|| turn.next_seq());
                (turn.conv_id.clone(), seq)
            })
        };
        let Some((conv_id, seq)) = slot else {
            tracing::warn!(
                target: "history",
                turn_id,
                tool = %call.tool,
                "turn not open — tool call dropped",
            );
            return false;
        };
        self.append_record(
            Some(&conv_id),
            Some(turn_id),
            seq,
            HistoryKind::ToolCall(call),
        );
        true
    }

    /// Latch assistant output for the turn's `turn_end`. Several messages
    /// accumulate; the estimate covers all of them, the stored excerpt only
    /// the head.
    pub fn note_assistant_output(&self, turn_id: &str, text: &str) {
        let mut st = self.lock();
        let Some(turn) = st.open.get_mut(turn_id) else {
            return;
        };
        let out = &mut turn.assistant;
        out.seen = true;
        out.bytes = out.bytes.saturating_add(text.len());
        out.tokens_est = out.tokens_est.saturating_add(estimate_text_tokens(text));
        if out.truncated {
            return;
        }
        if !out.head.is_empty() {
            out.head.push('\n');
        }
        let room = HISTORY_MAX_ASSISTANT_BYTES.saturating_sub(out.head.len());
        let (head, cut) = truncate_bytes(text, room);
        out.head.push_str(&head);
        out.truncated = cut;
    }

    /// Latch the measured bootstrap size (graph + memory + file refs) for
    /// the turn's `turn_end`.
    pub fn note_bootstrap_tokens(&self, turn_id: &str, tokens: usize) {
        if let Some(turn) = self.lock().open.get_mut(turn_id) {
            turn.bootstrap_tokens_est = Some(tokens);
        }
    }

    /// Latch the exact provider usage for a turn. If the turn's end was
    /// already deferred waiting for it, the `turn_end` line is written now,
    /// carrying the usage — this is what makes a usage-after-end order safe.
    pub fn note_usage(&self, turn_id: &str, usage: ProviderUsage) {
        let flush = {
            let mut st = self.lock();
            match st.deferred.remove(turn_id) {
                Some(d) => Some((d.conv_id, d.seq, d.end.with_usage(Some(usage)))),
                None => {
                    if let Some(turn) = st.open.get_mut(turn_id) {
                        turn.usage = Some(usage);
                    }
                    None
                }
            }
        };
        if let Some((conv_id, seq, end)) = flush {
            self.append_record(
                Some(&conv_id),
                Some(turn_id),
                seq,
                HistoryKind::TurnEnd(end),
            );
        }
    }

    /// Close a turn with its `turn_end` line. Returns false when the turn is
    /// not open (never begun, or already ended) and writes nothing.
    ///
    /// Fills the assistant output, its estimate, and the bootstrap
    /// measurement from what was latched, unless `end` already carries
    /// them. Tool calls that started but never completed are written first,
    /// as `summary_only` records in their reserved slots.
    ///
    /// When the turn expects usage and none has arrived — and the turn was
    /// neither cancelled nor failed, so usage is genuinely still coming —
    /// the line is deferred until [`Self::note_usage`].
    pub fn end_turn(&self, turn_id: &str, mut end: TurnEnd) -> bool {
        let (conv_id, orphans, write) = {
            let mut st = self.lock();
            let Some(mut turn) = st.open.remove(turn_id) else {
                return false;
            };
            if st.latest.get(&turn.conv_id).map(String::as_str) == Some(turn_id) {
                st.latest.remove(&turn.conv_id);
            }
            if turn.assistant.seen && end.assistant_excerpt.is_none() {
                end.assistant_bytes = turn.assistant.bytes;
                end.output_tokens_est = turn.assistant.tokens_est;
                end.assistant_excerpt = Some(std::mem::take(&mut turn.assistant.head));
                end.assistant_truncated = turn.assistant.truncated;
                end.estimator = Estimator::WordsX13;
            }
            if end.bootstrap_tokens_est.is_none() {
                end.bootstrap_tokens_est = turn.bootstrap_tokens_est;
            }
            let orphans: Vec<(String, u32)> = turn.pending_tools.drain(..).collect();
            let seq = turn.last_seq + 1;
            let defer =
                turn.expects_usage && turn.usage.is_none() && !end.cancelled && end.error.is_none();
            let write = if defer {
                st.deferred.insert(
                    turn_id.to_string(),
                    DeferredEnd {
                        conv_id: turn.conv_id.clone(),
                        seq,
                        end,
                    },
                );
                None
            } else {
                Some((seq, end.with_usage(turn.usage)))
            };
            (turn.conv_id, orphans, write)
        };
        for (summary, seq) in orphans {
            let call = tool_call_record(
                tool_name_from_summary(&summary),
                None,
                None,
                None,
                ToolOutput::None,
                Some(summary),
            );
            self.append_record(
                Some(&conv_id),
                Some(turn_id),
                seq,
                HistoryKind::ToolCall(call),
            );
        }
        if let Some((seq, end)) = write {
            self.append_record(
                Some(&conv_id),
                Some(turn_id),
                seq,
                HistoryKind::TurnEnd(end),
            );
        }
        true
    }

    /// Write every deferred `turn_end` without provider usage. For hosts
    /// shutting down while a usage event is still outstanding. Returns how
    /// many lines were written.
    pub fn flush_deferred_ends(&self) -> usize {
        let deferred: Vec<(String, DeferredEnd)> = self.lock().deferred.drain().collect();
        let n = deferred.len();
        for (turn_id, d) in deferred {
            self.append_record(
                Some(&d.conv_id),
                Some(&turn_id),
                d.seq,
                HistoryKind::TurnEnd(d.end),
            );
        }
        n
    }

    /// Record one MCP tool call with its verbatim request and response.
    ///
    /// `McpCallLogEntry` carries no conversation and the server is shared
    /// across turns and conversations, so attribution is inferred: when
    /// exactly one conversation has open turns the call belongs to that
    /// conversation's latest turn (`turn_inferred`); otherwise the record is
    /// written without identity (`unattributed`) and read back in the
    /// UNATTRIBUTED bucket. It is never dropped.
    pub fn record_mcp_call(&self, entry: &McpCallLogEntry) {
        let mut call = McpCall {
            tool: entry.tool_name.clone(),
            input: entry.input.clone(),
            output: entry.output.clone(),
            duration_us: u64::try_from(entry.duration.as_micros()).unwrap_or(u64::MAX),
            error: entry.error.clone(),
            empty_result: crate::mcp::telemetry_sink::output_is_empty(&entry.output),
            input_truncated: false,
            output_truncated: false,
            input_tokens_est: 0,
            output_tokens_est: 0,
            estimator: Estimator::CharsDiv4,
            attribution: Attribution::Unattributed,
        };
        let slot = {
            let mut st = self.lock();
            match sole_open_turn(&st) {
                Some(turn_id) => st.open.get_mut(&turn_id).map(|turn| {
                    let seq = turn.next_seq();
                    (turn.conv_id.clone(), turn_id, seq)
                }),
                None => None,
            }
        };
        match slot {
            Some((conv_id, turn_id, seq)) => {
                call.attribution = Attribution::TurnInferred;
                self.append_record(
                    Some(&conv_id),
                    Some(&turn_id),
                    seq,
                    HistoryKind::McpCall(call),
                );
            }
            None => self.append_record(None, None, 0, HistoryKind::McpCall(call)),
        }
    }

    /// The `(conv_id, turn_id)` MCP calls are attributed to right now, or
    /// `None` when zero or several conversations have open turns.
    pub fn sole_active_turn(&self) -> Option<(String, String)> {
        let st = self.lock();
        let turn_id = sole_open_turn(&st)?;
        let conv_id = st.open.get(&turn_id)?.conv_id.clone();
        Some((conv_id, turn_id))
    }

    /// The conversation's latest turn, if it is still open.
    pub fn active_turn_for(&self, conv_id: &str) -> Option<String> {
        let st = self.lock();
        st.latest
            .get(conv_id)
            .filter(|t| st.open.contains_key(t.as_str()))
            .cloned()
    }

    /// How many turns are open (begun, not ended).
    pub fn open_turn_count(&self) -> usize {
        self.lock().open.len()
    }

    // ── internals ────────────────────────────────────────────────────

    /// Normalize, serialize, and append. Never propagates: a history I/O
    /// failure is a warning, never an error the caller has to handle.
    fn append_record(
        &self,
        conv_id: Option<&str>,
        turn_id: Option<&str>,
        seq: u32,
        payload: HistoryKind,
    ) {
        let record = HistoryRecord {
            v: SCHEMA_VERSION,
            ts: now_rfc3339(),
            conv_id: conv_id.map(str::to_string),
            turn_id: turn_id.map(str::to_string),
            seq,
            payload: normalize(payload),
        };
        if let Err(e) = self.appender.append_json(&record) {
            tracing::warn!(
                target: "history",
                error = %e,
                path = %self.appender.path().display(),
                kind = record.payload.kind_str(),
                "failed to append history record",
            );
        }
    }
}

/// The turn MCP calls are attributed to: the latest open turn of the only
/// conversation that has open turns.
fn sole_open_turn(st: &State) -> Option<String> {
    let convs: HashSet<&str> = st.open.values().map(|t| t.conv_id.as_str()).collect();
    if convs.len() != 1 {
        return None;
    }
    let conv = convs.into_iter().next()?;
    st.latest
        .get(conv)
        .filter(|t| st.open.contains_key(t.as_str()))
        .cloned()
        .or_else(|| st.open.keys().next().cloned())
}

/// The tool name at the head of a one-line summary (`"Read src/a.rs"` →
/// `"Read"`, `"Bash: ls"` → `"Bash"`).
fn tool_name_from_summary(summary: &str) -> String {
    let name: String = summary
        .trim()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != ':' && *c != '(')
        .collect();
    if name.is_empty() {
        summary.trim().to_string()
    } else {
        name
    }
}

/// RFC3339 UTC with millisecond precision — the same shape the plan's
/// examples use and the same shape `chrono` emits elsewhere.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Apply the size caps and fill in the estimates for a payload, before it
/// is written.
///
/// Estimates are always computed from the **untruncated** text, so a
/// capped record still reports an honest number; the `*_truncated` flags
/// tell the panel where the payload was cut.
fn normalize(payload: HistoryKind) -> HistoryKind {
    match payload {
        HistoryKind::TurnStart(mut s) => {
            let full = std::mem::take(&mut s.prompt);
            s.prompt_bytes = full.len();
            let (text, cut) = truncate_bytes(&full, HISTORY_MAX_PROMPT_BYTES);
            s.prompt = text;
            s.prompt_truncated = cut;
            if s.input_tokens_est.is_none() {
                s.input_tokens_est = Some(estimate_text_tokens(&full));
            }
            if s.estimator.is_none() {
                s.estimator = Some(Estimator::WordsX13);
            }
            HistoryKind::TurnStart(s)
        }
        HistoryKind::ToolCall(mut c) => {
            normalize_tool_call(&mut c);
            HistoryKind::ToolCall(c)
        }
        HistoryKind::McpCall(mut m) => {
            normalize_mcp_call(&mut m);
            HistoryKind::McpCall(m)
        }
        HistoryKind::MemoryInjection(mut m) => {
            if let Some(block) = m.response_block.take() {
                let (text, cut) = truncate_bytes(&block, HISTORY_MAX_BLOCK_BYTES);
                m.response_block = Some(text);
                m.response_block_truncated = cut;
            }
            if let Some(manifest) = m.manifest.take() {
                let (capped, cut) = cap_manifest(manifest, HISTORY_MAX_JSON_BYTES);
                m.manifest = Some(capped);
                m.manifest_truncated = cut;
            }
            HistoryKind::MemoryInjection(m)
        }
        HistoryKind::TurnEnd(mut e) => {
            if let Some(text) = e.assistant_excerpt.take() {
                let (head, cut) = truncate_bytes(&text, HISTORY_MAX_ASSISTANT_BYTES);
                e.assistant_excerpt = Some(head);
                e.assistant_truncated = e.assistant_truncated || cut;
            }
            HistoryKind::TurnEnd(e)
        }
        other @ (HistoryKind::FilesChanged(_) | HistoryKind::TurnReview(_)) => other,
    }
}

fn normalize_tool_call(c: &mut ToolCall) {
    if let Some(input) = c.input.take() {
        let text = input.to_string();
        if c.input_tokens_est.is_none() {
            c.input_tokens_est = Some(estimate_json_text_tokens(&text));
        }
        let (head, cut) = truncate_bytes(&text, HISTORY_MAX_JSON_BYTES);
        c.input_truncated = cut;
        c.input = Some(if cut { Value::String(head) } else { input });
    }
    if let ToolOutput::Full { content, .. } = &mut c.output {
        if c.output_tokens_est.is_none() {
            c.output_tokens_est = Some(estimate_json_text_tokens(content));
        }
        let (head, cut) = truncate_bytes(content, HISTORY_MAX_JSON_BYTES);
        if cut {
            *content = head;
        }
        c.output_truncated = cut;
    }
    if c.estimator.is_none() && (c.input_tokens_est.is_some() || c.output_tokens_est.is_some()) {
        c.estimator = Some(Estimator::CharsDiv4);
    }
}

fn normalize_mcp_call(m: &mut McpCall) {
    let in_text = m.input.to_string();
    let out_text = m.output.to_string();
    m.input_tokens_est = estimate_json_text_tokens(&in_text);
    m.output_tokens_est = estimate_json_text_tokens(&out_text);
    let (in_head, in_cut) = truncate_bytes(&in_text, HISTORY_MAX_JSON_BYTES);
    let (out_head, out_cut) = truncate_bytes(&out_text, HISTORY_MAX_JSON_BYTES);
    m.input_truncated = in_cut;
    m.output_truncated = out_cut;
    if in_cut {
        m.input = Value::String(in_head);
    }
    if out_cut {
        m.output = Value::String(out_head);
    }
    m.estimator = Estimator::CharsDiv4;
}

/// Cap an oversized manifest. The candidate pool is by far the largest
/// part and the least useful when reading one turn, so it goes first
/// (its length is kept as `candidate_pool_omitted`); only if the rest is
/// still over the cap is the manifest replaced by the head of its compact
/// serialization, as a JSON string. Returns whether anything was dropped.
fn cap_manifest(mut value: Value, cap: usize) -> (Value, bool) {
    if value.to_string().len() <= cap {
        return (value, false);
    }
    if let Some(obj) = value.as_object_mut()
        && let Some(pool) = obj.remove("candidate_pool")
    {
        let len = pool.as_array().map(Vec::len).unwrap_or(0);
        obj.insert("candidate_pool_omitted".to_string(), Value::from(len));
    }
    let text = value.to_string();
    let (head, cut) = truncate_bytes(&text, cap);
    if cut {
        (Value::String(head), true)
    } else {
        (value, true)
    }
}

/// Build a `ToolCall` record with the capture mode set. Caps and estimates
/// are filled in when the record is written. `input` is the raw argument
/// JSON (None when the provider only produced a summary).
pub fn tool_call_record(
    tool: impl Into<String>,
    tool_use_id: Option<String>,
    duration_ms: Option<u64>,
    input: Option<Value>,
    output: ToolOutput,
    summary: Option<String>,
) -> ToolCall {
    let capture = if input.is_none() && !matches!(output, ToolOutput::Full { .. }) {
        CaptureMode::SummaryOnly
    } else {
        CaptureMode::Full
    };
    ToolCall {
        tool: tool.into(),
        tool_use_id,
        duration_ms,
        input,
        output,
        summary,
        capture,
        input_truncated: false,
        output_truncated: false,
        input_tokens_est: None,
        output_tokens_est: None,
        estimator: None,
    }
}

/// Build a `MemoryInjection` record from the injection summary.
pub fn memory_injection_record(
    items_injected: usize,
    pool_size: usize,
    tokens_used_est: usize,
    token_budget: usize,
    response_block: Option<String>,
    manifest: Option<Value>,
) -> MemoryInjection {
    MemoryInjection {
        items_injected,
        pool_size,
        tokens_used_est,
        token_budget,
        estimator: Estimator::WordsX13,
        response_block,
        response_block_truncated: false,
        manifest,
        manifest_truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::reader;

    fn start(prompt: &str) -> TurnStart {
        TurnStart {
            provider: "claude".into(),
            model: "claude:sonnet".into(),
            conv_title: Some("history panel".into()),
            workspace_root: "C:/w".into(),
            prompt: prompt.into(),
            prompt_bytes: 0,
            prompt_truncated: false,
            input_tokens_est: None,
            estimator: None,
        }
    }

    fn end() -> TurnEnd {
        TurnEnd::new(false, None, 0)
    }

    fn usage(input_tokens: u64) -> ProviderUsage {
        ProviderUsage {
            input_tokens,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            output_tokens: 4,
        }
    }

    fn rec() -> (tempfile::TempDir, Arc<HistoryRecorder>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(".gaviero")
            .join("history")
            .join("turns.ndjson");
        (
            dir,
            HistoryRecorder::with_path_and_cap(path, HISTORY_MAX_BYTES),
        )
    }

    fn mcp_entry(tool: &str) -> McpCallLogEntry {
        McpCallLogEntry {
            tool_name: tool.into(),
            input: serde_json::json!({"query": "token estimator", "limit": 5}),
            output: serde_json::json!({"results": [{"id": 41, "text": "…"}]}),
            duration: std::time::Duration::from_micros(4211),
            error: None,
            first_tool_call_initiated: false,
            session_id: None,
            turn: None,
        }
    }

    fn turn_ends(r: &HistoryRecorder) -> Vec<TurnEnd> {
        reader::read_records(r.path(), true)
            .records
            .into_iter()
            .filter_map(|e| match e.record.payload {
                HistoryKind::TurnEnd(end) => Some(end),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn begin_push_end_round_trips_through_the_reader() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("hello world"), false);
        assert!(r.tool_completed(
            "c1-1",
            tool_call_record(
                "Read",
                Some("toolu_1".into()),
                Some(12),
                Some(serde_json::json!({"file_path": "a.rs"})),
                ToolOutput::Full {
                    content: "fn main() {}".into(),
                    is_error: false,
                },
                Some("Read a.rs".into()),
            )
        ));
        assert!(r.end_turn("c1-1", end()));

        let out = reader::read_records(r.path(), true);
        assert_eq!(out.skipped, 0);
        assert_eq!(out.records.len(), 3);
        let seqs: Vec<u32> = out.records.iter().map(|e| e.record.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);

        let turns = reader::group_turns(out.records);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].turn_id.as_deref(), Some("c1-1"));
        assert_eq!(turns[0].summary.status, reader::TurnStatus::Complete);
        assert_eq!(turns[0].summary.tool_count, 1);
    }

    #[test]
    fn push_to_a_closed_turn_is_dropped_not_misattributed() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.end_turn("c1-1", end());
        assert!(!r.push("c1-1", HistoryKind::TurnEnd(end())));
        assert_eq!(reader::read_records(r.path(), true).records.len(), 2);
    }

    #[test]
    fn end_turn_is_idempotent_per_turn() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        assert!(r.end_turn("c1-1", end()));
        assert!(!r.end_turn("c1-1", end()));
        assert_eq!(turn_ends(&r).len(), 1);
    }

    #[test]
    fn a_late_end_for_the_previous_turn_closes_that_turn_not_the_new_one() {
        // The TUI can start turn 2 before turn 1's task reports it finished.
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("first"), false);
        r.begin_turn("c1", "c1-2", start("second"), false);
        assert!(r.end_turn("c1-1", TurnEnd::new(true, None, 0)));
        assert_eq!(r.active_turn_for("c1").as_deref(), Some("c1-2"));
        assert!(r.end_turn("c1-2", end()));

        let turns = reader::group_turns(reader::read_records(r.path(), true).records);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].summary.status, reader::TurnStatus::Cancelled);
        assert_eq!(turns[1].summary.status, reader::TurnStatus::Complete);
    }

    #[test]
    fn only_the_previous_turn_of_a_conversation_stays_open() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.begin_turn("c1", "c1-2", start("b"), false);
        r.begin_turn("c1", "c1-3", start("c"), false);
        assert_eq!(r.open_turn_count(), 2);
        assert!(!r.end_turn("c1-1", end()), "the oldest turn was abandoned");
    }

    #[test]
    fn usage_before_turn_end_lands_on_the_single_turn_end() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        r.note_usage("c1-1", usage(10));
        assert!(r.end_turn("c1-1", end()));

        let ends = turn_ends(&r);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].usage.unwrap().input_tokens, 10);
        assert_eq!(ends[0].usage_source.as_deref(), Some("provider"));
    }

    #[test]
    fn usage_after_turn_end_lands_on_the_single_turn_end() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        // Turn finished first: the line is deferred, not written.
        assert!(r.end_turn("c1-1", end()));
        assert!(
            turn_ends(&r).is_empty(),
            "turn_end must wait for the usage it expects"
        );
        r.note_usage("c1-1", usage(7));

        let ends = turn_ends(&r);
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0].usage.unwrap().input_tokens, 7);
    }

    #[test]
    fn a_turn_that_does_not_expect_usage_ends_immediately_without_it() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        assert!(r.end_turn("c1-1", end()));
        let ends = turn_ends(&r);
        assert_eq!(ends.len(), 1);
        assert!(ends[0].usage.is_none());
        assert!(ends[0].usage_source.is_none());
    }

    #[test]
    fn a_cancelled_turn_never_waits_for_usage() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        assert!(r.end_turn("c1-1", TurnEnd::new(true, None, 0)));
        assert_eq!(turn_ends(&r).len(), 1);
    }

    #[test]
    fn a_stale_deferred_end_is_flushed_by_the_next_turn() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        assert!(r.end_turn("c1-1", end())); // deferred
        r.begin_turn("c1", "c1-2", start("b"), false);
        let out = reader::read_records(r.path(), true);
        // turn_end(c1-1) was written without usage, then turn_start(c1-2).
        assert_eq!(out.records.len(), 3);
        assert_eq!(out.records[1].record.payload.kind_str(), "turn_end");
        assert_eq!(out.records[1].record.turn_id.as_deref(), Some("c1-1"));
    }

    #[test]
    fn flush_deferred_ends_writes_outstanding_lines() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        r.end_turn("c1-1", end());
        assert_eq!(r.flush_deferred_ends(), 1);
        assert_eq!(r.flush_deferred_ends(), 0);
        assert_eq!(turn_ends(&r).len(), 1);
    }

    #[test]
    fn turn_end_carries_latched_assistant_output_and_bootstrap() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.note_bootstrap_tokens("c1-1", 11_240);
        r.note_assistant_output("c1-1", "one two three four five six seven");
        r.end_turn("c1-1", TurnEnd::new(false, None, 2));

        let ends = turn_ends(&r);
        assert_eq!(ends[0].assistant_bytes, 33);
        assert_eq!(
            ends[0].output_tokens_est,
            estimate_text_tokens("one two three four five six seven")
        );
        assert_eq!(ends[0].bootstrap_tokens_est, Some(11_240));
        assert_eq!(ends[0].proposal_count, 2);
        assert!(!ends[0].assistant_truncated);
    }

    #[test]
    fn long_assistant_output_keeps_a_capped_head_and_a_full_estimate() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        let text = "word ".repeat(4_000); // 20 KB > 8 KB
        r.note_assistant_output("c1-1", &text);
        r.end_turn("c1-1", end());
        let e = &turn_ends(&r)[0];
        assert!(e.assistant_truncated);
        assert!(e.assistant_excerpt.as_ref().unwrap().len() <= HISTORY_MAX_ASSISTANT_BYTES);
        assert_eq!(e.assistant_bytes, text.len());
        assert_eq!(e.output_tokens_est, estimate_text_tokens(&text));
    }

    #[test]
    fn a_completed_tool_takes_the_slot_reserved_when_it_started() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.tool_started("c1-1", "Read a.rs"); // seq 1
        r.tool_started("c1-1", "Bash: ls"); // seq 2
        r.record_mcp_call(&mcp_entry("memory_search")); // seq 3
        // Bash finishes first but keeps seq 2.
        r.tool_completed(
            "c1-1",
            tool_call_record(
                "Bash",
                None,
                None,
                Some(serde_json::json!({"command": "ls"})),
                ToolOutput::Full {
                    content: "a.rs".into(),
                    is_error: false,
                },
                Some("Bash: ls".into()),
            ),
        );
        r.end_turn("c1-1", end()); // Read never completed → summary_only at seq 1

        let turns = reader::group_turns(reader::read_records(r.path(), true).records);
        let kinds: Vec<(u32, &str)> = turns[0]
            .records
            .iter()
            .map(|e| (e.record.seq, e.record.payload.kind_str()))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (0, "turn_start"),
                (1, "tool_call"),
                (2, "tool_call"),
                (3, "mcp_call"),
                (4, "turn_end"),
            ]
        );
        match &turns[0].records[1].record.payload {
            HistoryKind::ToolCall(c) => {
                assert_eq!(c.tool, "Read");
                assert_eq!(c.capture, CaptureMode::SummaryOnly);
                assert!(c.output.is_none());
                assert_eq!(c.summary.as_deref(), Some("Read a.rs"));
            }
            other => panic!("expected tool_call, got {other:?}"),
        }
        match &turns[0].records[2].record.payload {
            HistoryKind::ToolCall(c) => {
                assert_eq!(c.capture, CaptureMode::Full);
                assert_eq!(c.estimator, Some(Estimator::CharsDiv4));
                assert!(c.output_tokens_est.is_some());
            }
            other => panic!("expected tool_call, got {other:?}"),
        }
    }

    #[test]
    fn mcp_calls_are_attributed_only_when_one_conversation_is_open() {
        let (_dir, r) = rec();
        r.record_mcp_call(&mcp_entry("node_doc")); // nothing open
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.record_mcp_call(&mcp_entry("memory_search")); // c1 only
        r.begin_turn("c1", "c1-2", start("b"), false);
        r.record_mcp_call(&mcp_entry("blast_radius")); // c1 twice → latest turn
        r.begin_turn("c2", "c2-1", start("c"), false);
        r.record_mcp_call(&mcp_entry("repo_outline")); // c1 + c2

        let out = reader::read_records(r.path(), true);
        let mcp: Vec<(Option<String>, Attribution, bool)> = out
            .records
            .iter()
            .filter_map(|e| match &e.record.payload {
                HistoryKind::McpCall(m) => {
                    Some((e.record.turn_id.clone(), m.attribution, m.empty_result))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            mcp,
            vec![
                (None, Attribution::Unattributed, false),
                (Some("c1-1".into()), Attribution::TurnInferred, false),
                (Some("c1-2".into()), Attribution::TurnInferred, false),
                (None, Attribution::Unattributed, false),
            ]
        );
        assert_eq!(r.sole_active_turn(), None);
    }

    #[test]
    fn mcp_records_keep_verbatim_io_and_estimates() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        let entry = mcp_entry("memory_search");
        r.record_mcp_call(&entry);
        let out = reader::read_records(r.path(), true);
        match &out.records[1].record.payload {
            HistoryKind::McpCall(m) => {
                assert_eq!(m.input, entry.input);
                assert_eq!(m.output, entry.output);
                assert_eq!(m.duration_us, 4211);
                assert_eq!(
                    m.output_tokens_est,
                    entry.output.to_string().len().div_ceil(4)
                );
                assert_eq!(m.estimator, Estimator::CharsDiv4);
            }
            other => panic!("expected mcp_call, got {other:?}"),
        }
    }

    #[test]
    fn prompt_and_payloads_are_capped_with_estimates_from_the_full_text() {
        let (_dir, r) = rec();
        let prompt = "word ".repeat(80_000); // 400 KB > 256 KB cap
        r.begin_turn("c1", "c1-1", start(&prompt), false);
        r.end_turn("c1-1", end());

        let out = reader::read_records(r.path(), true);
        match &out.records[0].record.payload {
            HistoryKind::TurnStart(s) => {
                assert!(s.prompt_truncated);
                assert_eq!(s.prompt.len(), HISTORY_MAX_PROMPT_BYTES);
                assert_eq!(s.prompt_bytes, prompt.len());
                // Estimate is taken from the full 80k words, not the cap.
                assert_eq!(s.input_tokens_est, Some(estimate_text_tokens(&prompt)));
            }
            other => panic!("expected turn_start, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_capture_mode_is_summary_only_without_args_or_result() {
        let c = tool_call_record(
            "Read",
            None,
            None,
            None,
            ToolOutput::None,
            Some("Read".into()),
        );
        assert_eq!(c.capture, CaptureMode::SummaryOnly);
        let c = tool_call_record(
            "Read",
            None,
            None,
            Some(serde_json::json!({})),
            ToolOutput::None,
            None,
        );
        assert_eq!(c.capture, CaptureMode::Full);
    }

    #[test]
    fn oversized_tool_input_is_replaced_by_a_capped_string() {
        let (_dir, r) = rec();
        let big = "x".repeat(HISTORY_MAX_JSON_BYTES + 10);
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.tool_completed(
            "c1-1",
            tool_call_record(
                "Write",
                None,
                None,
                Some(serde_json::json!({ "content": big })),
                ToolOutput::None,
                None,
            ),
        );
        let out = reader::read_records(r.path(), true);
        match &out.records[1].record.payload {
            HistoryKind::ToolCall(c) => {
                assert!(c.input_truncated);
                assert_eq!(
                    c.input.as_ref().unwrap().as_str().unwrap().len(),
                    HISTORY_MAX_JSON_BYTES
                );
                // Estimated from the untruncated payload.
                assert!(c.input_tokens_est.unwrap() > HISTORY_MAX_JSON_BYTES / 4);
            }
            other => panic!("expected tool_call, got {other:?}"),
        }
    }

    #[test]
    fn an_oversized_manifest_drops_its_candidate_pool_first() {
        let pool: Vec<Value> = (0..4_000)
            .map(|i| serde_json::json!({"memory_id": i, "scope_label": "workspace"}))
            .collect();
        let manifest = serde_json::json!({
            "schema_version": 2,
            "selected_ids": [1, 2, 3],
            "candidate_pool": pool,
        });
        let (capped, cut) = cap_manifest(manifest, HISTORY_MAX_JSON_BYTES);
        assert!(cut);
        assert_eq!(capped["candidate_pool_omitted"], 4_000);
        assert_eq!(capped["selected_ids"], serde_json::json!([1, 2, 3]));
        assert!(capped.get("candidate_pool").is_none());

        let small = serde_json::json!({"selected_ids": [1]});
        assert_eq!(
            cap_manifest(small.clone(), HISTORY_MAX_JSON_BYTES),
            (small, false)
        );
    }

    #[test]
    fn summary_heads_name_the_tool() {
        assert_eq!(tool_name_from_summary("Read src/a.rs"), "Read");
        assert_eq!(tool_name_from_summary("Bash: cargo test"), "Bash");
        assert_eq!(tool_name_from_summary("Task (background): x"), "Task");
        assert_eq!(tool_name_from_summary("Glob"), "Glob");
    }
}
