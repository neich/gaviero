//! The single per-workspace history recorder.
//!
//! One `HistoryRecorder` exists per application instance (plan invariant
//! 4: *no dual writers*). It owns the file, the per-conversation
//! active-turn map, the exact-usage latch, and the deferred `turn_end`
//! slot; every capture point goes through it.
//!
//! Invariants this type has to keep (from the plan):
//!
//! 1. **Write-only.** Nothing reads through the recorder, and a write
//!    failure is logged at `warn!` and never propagated — a missing or
//!    broken log must never affect a turn.
//! 2. **Cheap.** Append-only, no `fsync`, no pretty-printing; the tool
//!    response path waits on this.
//! 3. **Idempotent turn end.** `turn_end` is written exactly once per
//!    turn even though `TurnTokenUsage` and `AgentTurnFinished` race.
//! 4. **Estimates are labelled.** Every record that carries an estimate
//!    names its estimator; exact provider usage is labelled
//!    `usage_source: "provider"`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use super::ndjson::NdjsonAppender;
use super::record::{
    HistoryKind, HistoryRecord, McpCall, MemoryInjection, ProviderUsage, ToolCall, ToolOutput,
    TurnEnd, TurnStart, SCHEMA_VERSION, truncate_bytes,
};
use super::tokens::{estimate_json_text_tokens, estimate_text_tokens, Estimator};
use super::{
    HISTORY_MAX_ASSISTANT_BYTES, HISTORY_MAX_BLOCK_BYTES, HISTORY_MAX_BYTES, HISTORY_MAX_JSON_BYTES,
    HISTORY_MAX_PROMPT_BYTES, history_path,
};

/// The turn a conversation is currently streaming, if any.
struct ActiveTurn {
    turn_id: String,
    /// Next `seq` to hand out; `turn_start` takes 0.
    next_seq: u32,
    /// True when the provider is known to report exact usage for a
    /// successful turn (Claude today). A turn end is then deferred until
    /// the usage event lands, so the single `turn_end` line can carry it.
    expects_usage: bool,
}

/// A `turn_end` waiting for its provider usage.
struct PendingEnd {
    turn_id: String,
    /// The seq the line was assigned when the turn closed.
    seq: u32,
    end: TurnEnd,
}

#[derive(Default)]
struct State {
    /// conv_id → active turn.
    active: HashMap<String, ActiveTurn>,
    /// conv_id → exact usage seen for the active turn.
    usage: HashMap<String, ProviderUsage>,
    /// conv_id → deferred turn end.
    pending_end: HashMap<String, PendingEnd>,
    /// turn_ids whose `turn_end` line has already been written.
    ended: HashSet<String>,
}

/// The recorder. Cheap to clone through an `Arc`; `Mutex`-guarded state.
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

    /// Register a new turn for `conv_id` and write its `turn_start`
    /// record (seq 0). Returns the seq base. A stale deferred turn end
    /// for the same conversation is flushed first, so a turn whose usage
    /// never arrived is still closed rather than silently lost.
    ///
    /// `expects_usage` must be true only for providers that report exact
    /// token usage on a successful turn.
    pub fn begin_turn(
        &self,
        conv_id: &str,
        turn_id: &str,
        start: TurnStart,
        expects_usage: bool,
    ) -> u32 {
        let stale = {
            let mut st = self.lock();
            st.usage.remove(conv_id);
            let stale = st.pending_end.remove(conv_id);
            st.active.insert(
                conv_id.to_string(),
                ActiveTurn {
                    turn_id: turn_id.to_string(),
                    next_seq: 0,
                    expects_usage,
                },
            );
            stale
        };
        // A previous turn of this conversation whose usage never arrived is
        // closed now, so its records keep a `turn_end` marker.
        if let Some(p) = stale {
            self.write_end(conv_id, &p.turn_id, p.seq, p.end);
        }
        self.push_record(conv_id, Some(turn_id), 0, HistoryKind::TurnStart(start));
        0
    }

    /// Append one record to `conv_id`'s active turn, assigning `ts` and
    /// `seq`. Returns false (and logs) when the conversation has no
    /// active turn — the caller's record is dropped rather than written
    /// unattributed into another turn.
    pub fn push(&self, conv_id: &str, payload: HistoryKind) -> bool {
        let next = {
            let mut st = self.lock();
            match st.active.get_mut(conv_id) {
                Some(turn) => {
                    turn.next_seq += 1;
                    Some((turn.turn_id.clone(), turn.next_seq))
                }
                None => None,
            }
        };
        let Some((turn_id, seq)) = next else {
            tracing::warn!(
                target: "history",
                conv_id,
                kind = payload.kind_str(),
                "no active turn — history record dropped",
            );
            return false;
        };
        self.push_record(conv_id, Some(&turn_id), seq, payload);
        true
    }

    /// Append a record that could not be attributed to any turn (MCP
    /// calls issued while zero or several conversations were streaming).
    ///
    /// Such records carry no `conv_id`/`turn_id` and `seq: 0`; the reader
    /// groups them into a single UNATTRIBUTED bucket, in file order.
    pub fn push_unattributed(&self, payload: HistoryKind) -> bool {
        self.push_record_unattributed(payload);
        true
    }

    /// Latch the exact provider usage for `conv_id`'s active turn. If the
    /// turn end was already deferred waiting for it, the `turn_end` line
    /// is written now, carrying the usage — this is what makes the
    /// `TurnTokenUsage` / `AgentTurnFinished` race order-independent.
    pub fn note_usage(&self, conv_id: &str, usage: ProviderUsage) {
        let flush = {
            let mut st = self.lock();
            match st.pending_end.remove(conv_id) {
                Some(p) => Some((p.turn_id, p.seq, p.end.with_usage(Some(usage)))),
                None => {
                    st.usage.insert(conv_id.to_string(), usage);
                    None
                }
            }
        };
        if let Some((turn_id, seq, end)) = flush {
            self.write_end(conv_id, &turn_id, seq, end);
        }
    }

    /// Close `conv_id`'s turn with a `turn_end` line. Idempotent per
    /// turn: the second call for the same turn returns false and writes
    /// nothing.
    ///
    /// When the turn's provider reports usage and no usage has been seen
    /// yet — and the turn was neither cancelled nor failed, so usage is
    /// genuinely still expected — the line is deferred until
    /// [`Self::note_usage`] arrives. Anything else writes immediately.
    pub fn end_turn(&self, conv_id: &str, end: TurnEnd) -> bool {
        let action = {
            let mut st = self.lock();
            let Some(turn) = st.active.remove(conv_id) else {
                return false;
            };
            if st.ended.contains(&turn.turn_id) {
                return false;
            }
            let seq = turn.next_seq + 1;
            let usage = st.usage.remove(conv_id);
            let defer = turn.expects_usage && usage.is_none() && !end.cancelled && end.error.is_none();
            if defer {
                st.pending_end.insert(
                    conv_id.to_string(),
                    PendingEnd {
                        turn_id: turn.turn_id,
                        seq,
                        end,
                    },
                );
                None
            } else {
                Some((turn.turn_id, seq, end.with_usage(usage)))
            }
        };
        match action {
            // Deferred: `note_usage` (or the next `begin_turn` for this
            // conversation) writes the line. The turn is already closed
            // for the purposes of this recorder.
            None => true,
            Some((turn_id, seq, end)) => {
                self.write_end(conv_id, &turn_id, seq, end);
                true
            }
        }
    }

    /// Write a deferred `turn_end` without provider usage. Called when a
    /// conversation goes away (quit, conversation close) so a turn whose
    /// usage never arrived is still closed. Returns true when a line was
    /// written.
    pub fn flush_pending_end(&self, conv_id: &str) -> bool {
        let pending = self.lock().pending_end.remove(conv_id);
        match pending {
            Some(p) => {
                self.write_end(conv_id, &p.turn_id, p.seq, p.end);
                true
            }
            None => false,
        }
    }

    /// The single streaming turn, or `None` when zero or more than one
    /// conversation is streaming. This is the only turn attribution the
    /// MCP path can get (`McpCallLogEntry` carries no conversation).
    pub fn sole_active_turn(&self) -> Option<(String, String)> {
        let st = self.lock();
        if st.active.len() != 1 {
            return None;
        }
        st.active
            .iter()
            .next()
            .map(|(conv, turn)| (conv.clone(), turn.turn_id.clone()))
    }

    /// The active turn id for `conv_id`, if it is streaming.
    pub fn active_turn_for(&self, conv_id: &str) -> Option<String> {
        self.lock()
            .active
            .get(conv_id)
            .map(|t| t.turn_id.clone())
    }

    /// How many conversations are currently streaming (used by the MCP
    /// attribution decision and by tests).
    pub fn active_turn_count(&self) -> usize {
        self.lock().active.len()
    }

    // ── internals ────────────────────────────────────────────────────

    /// Serialize and append. Never propagates: a history I/O failure is a
    /// warning, never an error the caller has to handle.
    fn push_record(
        &self,
        conv_id: &str,
        turn_id: Option<&str>,
        seq: u32,
        payload: HistoryKind,
    ) {
        let record = HistoryRecord {
            v: SCHEMA_VERSION,
            ts: now_rfc3339(),
            conv_id: Some(conv_id.to_string()),
            turn_id: turn_id.map(str::to_string),
            seq,
            payload: normalize(payload),
        };
        self.append(&record);
    }

    fn push_record_unattributed(&self, payload: HistoryKind) {
        let record = HistoryRecord {
            v: SCHEMA_VERSION,
            ts: now_rfc3339(),
            conv_id: None,
            turn_id: None,
            seq: 0,
            payload: normalize(payload),
        };
        self.append(&record);
    }

    fn append(&self, record: &HistoryRecord) {
        if let Err(e) = self.appender.append_json(record) {
            tracing::warn!(
                target: "history",
                error = %e,
                path = %self.appender.path().display(),
                kind = record.payload.kind_str(),
                "failed to append history record",
            );
        }
    }

    /// Write the single `turn_end` line and remember the turn is closed.
    fn write_end(&self, conv_id: &str, turn_id: &str, seq: u32, end: TurnEnd) {
        self.lock().ended.insert(turn_id.to_string());
        self.push_record(conv_id, Some(turn_id), seq, HistoryKind::TurnEnd(end));
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
                m.manifest = Some(cap_json(manifest, HISTORY_MAX_JSON_BYTES));
            }
            HistoryKind::MemoryInjection(m)
        }
        HistoryKind::TurnEnd(mut e) => {
            if let Some(text) = e.assistant_excerpt.take() {
                let (head, cut) = truncate_bytes(&text, HISTORY_MAX_ASSISTANT_BYTES);
                e.assistant_excerpt = Some(head);
                e.assistant_truncated = cut;
            }
            HistoryKind::TurnEnd(e)
        }
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
        c.input = Some(if cut {
            Value::String(head)
        } else {
            input
        });
    }
    if let ToolOutput::Full { content, .. } = &c.output {
        let content = content.clone();
        if c.output_tokens_est.is_none() {
            c.output_tokens_est = Some(estimate_json_text_tokens(&content));
        }
        let (head, cut) = truncate_bytes(&content, HISTORY_MAX_JSON_BYTES);
        if let ToolOutput::Full { content, .. } = &mut c.output {
            *content = head;
        }
        c.output_truncated = cut;
    } else if c.output_tokens_est.is_none() {
        // Summary-only / no-result: nothing to estimate.
        c.output_tokens_est = Some(0);
    }
    if c.estimator.is_none() {
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

/// Cap an oversized JSON value by replacing it with the head of its
/// compact serialization (as a JSON string). The caller's
/// `*_truncated` flag records that this happened.
fn cap_json(value: Value, cap: usize) -> Value {
    let text = value.to_string();
    let (head, cut) = truncate_bytes(&text, cap);
    if cut {
        Value::String(head)
    } else {
        value
    }
}

/// Build a `ToolCall` record with the caps, estimates, and capture mode
/// filled in. `input` is the raw argument JSON (None when the provider
/// only produced a summary).
pub fn tool_call_record(
    tool: impl Into<String>,
    tool_use_id: Option<String>,
    duration_ms: Option<u64>,
    input: Option<Value>,
    output: ToolOutput,
    summary: Option<String>,
) -> ToolCall {
    let capture = if input.is_none() && matches!(output, ToolOutput::None | ToolOutput::Summary { .. })
    {
        super::record::CaptureMode::SummaryOnly
    } else {
        super::record::CaptureMode::Full
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::record::{CaptureMode, ToolOutput};
    use crate::history::reader;

    fn start(prompt: &str) -> TurnStart {
        TurnStart {
            provider: "claude".into(),
            model: "sonnet".into(),
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
        TurnEnd {
            cancelled: false,
            error: None,
            proposal_count: 0,
            assistant_bytes: 0,
            assistant_truncated: false,
            assistant_excerpt: None,
            output_tokens_est: 0,
            estimator: Estimator::WordsX13,
            bootstrap_tokens_est: None,
            usage: None,
            usage_source: None,
        }
    }

    fn rec() -> (tempfile::TempDir, Arc<HistoryRecorder>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gaviero").join("history").join("turns.ndjson");
        (dir, HistoryRecorder::with_path_and_cap(path, HISTORY_MAX_BYTES))
    }

    #[test]
    fn begin_push_end_round_trips_through_the_reader() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("hello world"), false);
        assert!(r.push(
            "c1",
            HistoryKind::ToolCall(tool_call_record(
                "Read",
                Some("toolu_1".into()),
                Some(12),
                Some(serde_json::json!({"file_path": "a.rs"})),
                ToolOutput::Full {
                    content: "fn main() {}".into(),
                    is_error: false,
                },
                Some("Read(a.rs)".into()),
            ))
        ));
        assert!(r.end_turn("c1", end()));

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
    fn push_without_an_active_turn_is_dropped_not_misattributed() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        r.end_turn("c1", end());
        assert!(!r.push("c1", HistoryKind::TurnEnd(end())));
        assert_eq!(reader::read_records(r.path(), true).records.len(), 2);
    }

    #[test]
    fn end_turn_is_idempotent_per_turn() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), false);
        assert!(r.end_turn("c1", end()));
        assert!(!r.end_turn("c1", end()));
        let out = reader::read_records(r.path(), true);
        assert_eq!(
            out.records
                .iter()
                .filter(|e| e.record.payload.kind_str() == "turn_end")
                .count(),
            1
        );
    }

    #[test]
    fn usage_before_turn_finished_lands_on_the_single_turn_end() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        r.note_usage(
            "c1",
            ProviderUsage {
                input_tokens: 10,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                output_tokens: 4,
            },
        );
        assert!(r.end_turn("c1", end()));

        let out = reader::read_records(r.path(), true);
        let ends: Vec<_> = out
            .records
            .iter()
            .filter(|e| e.record.payload.kind_str() == "turn_end")
            .collect();
        assert_eq!(ends.len(), 1);
        match &ends[0].record.payload {
            HistoryKind::TurnEnd(e) => {
                assert_eq!(e.usage.unwrap().input_tokens, 10);
                assert_eq!(e.usage_source.as_deref(), Some("provider"));
            }
            other => panic!("expected turn_end, got {other:?}"),
        }
    }

    #[test]
    fn usage_after_turn_finished_lands_on_the_single_turn_end() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        // Turn finished first: the line is deferred, not written.
        assert!(r.end_turn("c1", end()));
        assert_eq!(
            reader::read_records(r.path(), true).records.len(),
            1,
            "turn_end must wait for the usage it expects"
        );
        r.note_usage(
            "c1",
            ProviderUsage {
                input_tokens: 7,
                ..Default::default()
            },
        );
        let out = reader::read_records(r.path(), true);
        let ends: Vec<_> = out
            .records
            .iter()
            .filter(|e| e.record.payload.kind_str() == "turn_end")
            .collect();
        assert_eq!(ends.len(), 1);
        match &ends[0].record.payload {
            HistoryKind::TurnEnd(e) => assert_eq!(e.usage.unwrap().input_tokens, 7),
            other => panic!("expected turn_end, got {other:?}"),
        }
    }

    #[test]
    fn a_cancelled_turn_never_waits_for_usage() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        let mut e = end();
        e.cancelled = true;
        assert!(r.end_turn("c1", e));
        assert_eq!(reader::read_records(r.path(), true).records.len(), 2);
    }

    #[test]
    fn a_stale_deferred_end_is_flushed_by_the_next_turn() {
        let (_dir, r) = rec();
        r.begin_turn("c1", "c1-1", start("a"), true);
        assert!(r.end_turn("c1", end())); // deferred
        r.begin_turn("c1", "c1-2", start("b"), false);
        let out = reader::read_records(r.path(), true);
        // turn_end(c1-1) was written without usage, then turn_start(c1-2).
        assert_eq!(out.records.len(), 3);
        assert_eq!(out.records[1].record.payload.kind_str(), "turn_end");
        assert_eq!(out.records[1].record.turn_id.as_deref(), Some("c1-1"));
    }

    #[test]
    fn sole_active_turn_is_only_defined_for_one_streaming_conversation() {
        let (_dir, r) = rec();
        assert_eq!(r.sole_active_turn(), None);
        r.begin_turn("c1", "c1-1", start("a"), false);
        assert_eq!(
            r.sole_active_turn(),
            Some(("c1".to_string(), "c1-1".to_string()))
        );
        r.begin_turn("c2", "c2-1", start("b"), false);
        assert_eq!(r.sole_active_turn(), None);
        assert_eq!(r.active_turn_for("c2").as_deref(), Some("c2-1"));
    }

    #[test]
    fn unattributed_records_carry_no_identity() {
        let (_dir, r) = rec();
        r.push_unattributed(HistoryKind::McpCall(McpCall {
            tool: "memory_search".into(),
            input: serde_json::json!({"query": "q"}),
            output: serde_json::json!({"results": []}),
            duration_us: 4211,
            error: None,
            empty_result: true,
            input_truncated: false,
            output_truncated: false,
            input_tokens_est: 0,
            output_tokens_est: 0,
            estimator: Estimator::CharsDiv4,
            attribution: crate::history::record::Attribution::Unattributed,
        }));
        let out = reader::read_records(r.path(), true);
        assert_eq!(out.records.len(), 1);
        assert!(out.records[0].record.conv_id.is_none());
        let turns = reader::group_turns(out.records);
        assert_eq!(turns.len(), 1);
        assert!(!turns[0].attributed);
    }

    #[test]
    fn prompt_and_payloads_are_capped_with_estimates_from_the_full_text() {
        let (_dir, r) = rec();
        let prompt = "word ".repeat(80_000); // 400 KB > 256 KB cap
        r.begin_turn("c1", "c1-1", start(&prompt), false);
        r.end_turn("c1", end());

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
        r.push(
            "c1",
            HistoryKind::ToolCall(tool_call_record(
                "Write",
                None,
                None,
                Some(serde_json::json!({ "content": big })),
                ToolOutput::None,
                None,
            )),
        );
        let out = reader::read_records(r.path(), true);
        match &out.records[1].record.payload {
            HistoryKind::ToolCall(c) => {
                assert!(c.input_truncated);
                assert_eq!(c.input.as_ref().unwrap().as_str().unwrap().len(), HISTORY_MAX_JSON_BYTES);
                // Estimated from the untruncated payload.
                assert!(c.input_tokens_est.unwrap() > HISTORY_MAX_JSON_BYTES / 4);
            }
            other => panic!("expected tool_call, got {other:?}"),
        }
    }
}
