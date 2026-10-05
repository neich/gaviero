//! Tolerant reader for `<workspace>/.gaviero/history/turns.ndjson`.
//!
//! The log is a write-only audit artefact (plan invariant 1): nothing in
//! the agent path reads it, and a reader failure degrades to "fewer
//! turns", never to an error a caller must handle. This reader is shared
//! by the HISTORY panel and the `gaviero-cli --history` reader so both
//! render the same thing from the same parser.
//!
//! Rules:
//!
//! * Malformed lines are **skipped and counted**, never fatal — the file
//!   may carry a partially written tail or a mid-rotation remnant.
//! * A turn with no `turn_end` (crash, kill, still streaming) is valid
//!   and reads back as [`TurnStatus::Incomplete`].
//! * Inside a turn, records are ordered by `seq` (file order breaks ties):
//!   a tool call reserves its slot when it starts but is written when it
//!   completes. Turns come back in order of first appearance, so callers
//!   wanting newest-first reverse the list.

use std::collections::HashMap;
use std::path::Path;

use super::record::{CaptureMode, HistoryKind, HistoryRecord, ProviderUsage};
use crate::util::ndjson::rotated_path;

/// One parsed line plus its 1-based physical line number across the
/// concatenated generations (rotated first), for diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryEvent {
    pub record: HistoryRecord,
    pub line: u64,
}

/// Everything a read produced, including what it could not understand.
#[derive(Debug, Clone, Default)]
pub struct ReadOutcome {
    pub records: Vec<HistoryEvent>,
    /// Lines that did not parse as a [`HistoryRecord`].
    pub skipped: usize,
    /// Bytes read across both generations.
    pub bytes: u64,
}

/// How a turn ended, derived from its `turn_end` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    /// `turn_end` present, not cancelled, no error.
    Complete,
    /// No `turn_end` — the process died, or the turn is still streaming.
    Incomplete,
    /// `turn_end` with `cancelled: true`.
    Cancelled,
    /// `turn_end` with an `error`.
    Failed,
}

impl TurnStatus {
    /// Lower-case label shared by the panel and the CLI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }
}

/// The cheap per-turn view the turns column renders.
///
/// Token fields keep each number's perspective explicit rather than summing
/// across perspectives: a tool's *input* is text the model wrote, while its
/// *result* is text fed back to the model.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnSummary {
    pub conv_id: Option<String>,
    pub conv_title: Option<String>,
    pub turn_id: Option<String>,
    /// False only for the UNATTRIBUTED bucket (MCP calls issued while no
    /// single conversation was streaming).
    pub attributed: bool,
    pub started_at: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub prompt_preview: String,
    pub prompt_bytes: usize,
    pub tool_count: usize,
    /// Tool calls for which only the one-line summary was captured.
    pub summary_only_tools: usize,
    pub mcp_count: usize,
    pub mem_items: usize,
    /// `~` prompt estimate (`turn_start.input_tokens_est`).
    pub in_tokens_est: usize,
    /// `~` assistant output estimate (`turn_end.output_tokens_est`).
    pub out_tokens_est: usize,
    /// `~` tool arguments / tool results, summed over the turn's calls.
    pub tool_in_tokens_est: usize,
    pub tool_out_tokens_est: usize,
    /// `~` MCP requests / responses, summed over the turn's calls.
    pub mcp_in_tokens_est: usize,
    pub mcp_out_tokens_est: usize,
    /// Tokens the memory injection reported using (`words×1.3`).
    pub memory_tokens_est: usize,
    pub bootstrap_tokens_est: Option<usize>,
    pub exact_usage: Option<ProviderUsage>,
    pub status: TurnStatus,
    /// Files the turn changed on disk (`files_changed` record), 0 if none.
    pub files_changed: usize,
}

impl TurnSummary {
    /// `~in/~out` for the turns column: prompt and assistant output
    /// estimates, always `~`-prefixed by the renderer.
    pub fn est_pair(&self) -> (usize, usize) {
        (self.in_tokens_est, self.out_tokens_est)
    }
}

/// One turn's records, in `seq` order, with its derived summary.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRecords {
    pub conv_id: Option<String>,
    pub turn_id: Option<String>,
    pub attributed: bool,
    pub records: Vec<HistoryEvent>,
    pub summary: TurnSummary,
}

/// Stream-parse the log, optionally including the single rotated
/// generation. The rotated generation is read first so the returned
/// order matches chronological order.
pub fn read_records(path: &Path, include_rotated: bool) -> ReadOutcome {
    let mut out = ReadOutcome::default();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    if include_rotated {
        files.push(rotated_path(path));
    }
    files.push(path.to_path_buf());

    let mut line_no: u64 = 0;
    for file in files {
        let Ok(f) = std::fs::File::open(&file) else {
            continue;
        };
        out.bytes = out
            .bytes
            .saturating_add(f.metadata().map(|m| m.len()).unwrap_or(0));
        for line in std::io::BufRead::lines(std::io::BufReader::new(f)) {
            line_no += 1;
            let Ok(line) = line else {
                out.skipped += 1;
                continue;
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<HistoryRecord>(trimmed) {
                Ok(record) => out.records.push(HistoryEvent {
                    record,
                    line: line_no,
                }),
                Err(_) => out.skipped += 1,
            }
        }
    }
    out
}

/// Group records into turns. Attributed records group by `turn_id`;
/// every unattributed record (`turn_id: None`) lands in a single bucket
/// so unattributable MCP calls are visible rather than lost.
pub fn group_turns(records: Vec<HistoryEvent>) -> Vec<TurnRecords> {
    let mut turns: Vec<TurnRecords> = Vec::new();
    let mut index: HashMap<Option<String>, usize> = HashMap::new();
    for event in records {
        let turn_id = event.record.turn_id.clone();
        let idx = *index.entry(turn_id.clone()).or_insert_with(|| {
            let attributed = turn_id.is_some();
            turns.push(TurnRecords {
                conv_id: event.record.conv_id.clone(),
                turn_id: turn_id.clone(),
                attributed,
                records: Vec::new(),
                summary: empty_summary(&event.record, turn_id, attributed),
            });
            turns.len() - 1
        });
        let turn = &mut turns[idx];
        if turn.conv_id.is_none() {
            turn.conv_id = event.record.conv_id.clone();
        }
        apply_to_summary(&mut turn.summary, &event.record);
        turn.records.push(event);
    }
    for turn in &mut turns {
        if turn.attributed {
            // Stable: equal seqs keep file order.
            turn.records.sort_by_key(|e| e.record.seq);
        }
    }
    turns
}

/// Convenience: the summaries for a slice of turns.
pub fn summarize(turns: &[TurnRecords]) -> Vec<TurnSummary> {
    turns.iter().map(|t| t.summary.clone()).collect()
}

/// Read a single turn by id (across both generations when asked).
pub fn read_turn(path: &Path, turn_id: &str, include_rotated: bool) -> Option<TurnRecords> {
    let out = read_records(path, include_rotated);
    group_turns(out.records)
        .into_iter()
        .find(|t| t.turn_id.as_deref() == Some(turn_id))
}

fn empty_summary(record: &HistoryRecord, turn_id: Option<String>, attributed: bool) -> TurnSummary {
    TurnSummary {
        conv_id: record.conv_id.clone(),
        conv_title: None,
        turn_id,
        attributed,
        started_at: Some(record.ts.clone()),
        provider: None,
        model: None,
        prompt_preview: String::new(),
        prompt_bytes: 0,
        tool_count: 0,
        summary_only_tools: 0,
        mcp_count: 0,
        mem_items: 0,
        in_tokens_est: 0,
        out_tokens_est: 0,
        tool_in_tokens_est: 0,
        tool_out_tokens_est: 0,
        mcp_in_tokens_est: 0,
        mcp_out_tokens_est: 0,
        memory_tokens_est: 0,
        bootstrap_tokens_est: None,
        exact_usage: None,
        status: TurnStatus::Incomplete,
        files_changed: 0,
    }
}

fn apply_to_summary(summary: &mut TurnSummary, record: &HistoryRecord) {
    match &record.payload {
        HistoryKind::TurnStart(s) => {
            summary.started_at = Some(record.ts.clone());
            summary.conv_title = s.conv_title.clone();
            summary.provider = Some(s.provider.clone());
            summary.model = Some(s.model.clone());
            summary.prompt_preview = preview(&s.prompt);
            summary.prompt_bytes = s.prompt_bytes;
            summary.in_tokens_est = s.input_tokens_est.unwrap_or(0);
        }
        HistoryKind::ToolCall(c) => {
            summary.tool_count += 1;
            if c.capture == CaptureMode::SummaryOnly {
                summary.summary_only_tools += 1;
            }
            summary.tool_in_tokens_est = summary
                .tool_in_tokens_est
                .saturating_add(c.input_tokens_est.unwrap_or(0));
            summary.tool_out_tokens_est = summary
                .tool_out_tokens_est
                .saturating_add(c.output_tokens_est.unwrap_or(0));
        }
        HistoryKind::McpCall(m) => {
            summary.mcp_count += 1;
            summary.mcp_in_tokens_est =
                summary.mcp_in_tokens_est.saturating_add(m.input_tokens_est);
            summary.mcp_out_tokens_est = summary
                .mcp_out_tokens_est
                .saturating_add(m.output_tokens_est);
        }
        HistoryKind::MemoryInjection(m) => {
            summary.mem_items = summary.mem_items.max(m.items_injected);
            summary.memory_tokens_est = summary.memory_tokens_est.saturating_add(m.tokens_used_est);
        }
        HistoryKind::TurnEnd(e) => {
            summary.out_tokens_est = e.output_tokens_est;
            summary.bootstrap_tokens_est = e.bootstrap_tokens_est;
            summary.exact_usage = e.usage;
            summary.status = if e.cancelled {
                TurnStatus::Cancelled
            } else if e.error.is_some() {
                TurnStatus::Failed
            } else {
                TurnStatus::Complete
            };
        }
        HistoryKind::FilesChanged(f) => {
            summary.files_changed = f.files.len();
        }
        HistoryKind::TurnReview(_) => {}
    }
}

/// One-line, whitespace-collapsed head of a prompt for the turns column.
fn preview(prompt: &str) -> String {
    const MAX: usize = 120;
    let collapsed = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(MAX).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::record::{
        Attribution, McpCall, SCHEMA_VERSION, ToolOutput, TurnEnd, TurnStart,
    };
    use crate::history::tokens::Estimator;

    fn line(conv: &str, turn: &str, seq: u32, payload: HistoryKind) -> String {
        serde_json::to_string(&HistoryRecord {
            v: SCHEMA_VERSION,
            ts: "2026-09-15T18:43:44.512Z".into(),
            conv_id: Some(conv.into()),
            turn_id: Some(turn.into()),
            seq,
            payload,
        })
        .unwrap()
    }

    fn start() -> HistoryKind {
        HistoryKind::TurnStart(TurnStart {
            provider: "claude".into(),
            model: "claude:sonnet".into(),
            conv_title: Some("t".into()),
            workspace_root: "C:/w".into(),
            prompt: "hello\n\n  world".into(),
            prompt_bytes: 15,
            prompt_truncated: false,
            input_tokens_est: Some(3),
            estimator: Some(Estimator::WordsX13),
        })
    }

    fn end(cancelled: bool, error: Option<&str>) -> HistoryKind {
        let mut e = TurnEnd::new(cancelled, error.map(str::to_string), 0);
        e.output_tokens_est = 5;
        e.bootstrap_tokens_est = Some(100);
        HistoryKind::TurnEnd(e.with_usage(Some(ProviderUsage {
            input_tokens: 42,
            ..Default::default()
        })))
    }

    fn tool(input_est: usize, output_est: usize) -> HistoryKind {
        let mut c = crate::history::writer::tool_call_record(
            "Read",
            None,
            Some(5),
            Some(serde_json::json!({"file_path": "a"})),
            ToolOutput::Full {
                content: "ok".into(),
                is_error: false,
            },
            None,
        );
        c.input_tokens_est = Some(input_est);
        c.output_tokens_est = Some(output_est);
        HistoryKind::ToolCall(c)
    }

    #[test]
    fn groups_by_turn_and_derives_the_summary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let body = [
            line("c1", "c1-1", 0, start()),
            line("c1", "c1-1", 1, tool(7, 11)),
            line("c1", "c1-1", 2, end(false, None)),
        ]
        .join("\n");
        std::fs::write(&path, format!("{body}\n")).unwrap();

        let out = read_records(&path, false);
        assert_eq!(out.skipped, 0);
        let turns = group_turns(out.records);
        assert_eq!(turns.len(), 1);
        let s = &turns[0].summary;
        assert_eq!(s.status, TurnStatus::Complete);
        assert_eq!(s.tool_count, 1);
        assert_eq!(s.summary_only_tools, 0);
        assert_eq!(s.prompt_preview, "hello world");
        assert_eq!(s.provider.as_deref(), Some("claude"));
        assert_eq!(s.bootstrap_tokens_est, Some(100));
        assert_eq!(s.exact_usage.unwrap().input_tokens, 42);
        // Prompt in / assistant out; tool numbers stay in their own fields.
        assert_eq!(s.est_pair(), (3, 5));
        assert_eq!((s.tool_in_tokens_est, s.tool_out_tokens_est), (7, 11));
    }

    #[test]
    fn a_turn_without_a_turn_end_is_incomplete_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        std::fs::write(&path, format!("{}\n", line("c1", "c1-1", 0, start()))).unwrap();
        let turns = group_turns(read_records(&path, false).records);
        assert_eq!(turns[0].summary.status, TurnStatus::Incomplete);
    }

    #[test]
    fn cancelled_and_failed_are_distinguished() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let body = format!(
            "{}\n{}\n{}\n{}\n",
            line("c1", "c1-1", 0, start()),
            line("c1", "c1-1", 1, end(true, None)),
            line("c1", "c1-2", 0, start()),
            line("c1", "c1-2", 1, end(false, Some("boom"))),
        );
        std::fs::write(&path, body).unwrap();
        let turns = group_turns(read_records(&path, false).records);
        assert_eq!(turns[0].summary.status, TurnStatus::Cancelled);
        assert_eq!(turns[1].summary.status, TurnStatus::Failed);
    }

    #[test]
    fn malformed_lines_are_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let body = format!(
            "not json\n{}\n{{\"v\":1,\"kind\":\"turn_end\"}}\n",
            line("c1", "c1-1", 0, start())
        );
        std::fs::write(&path, body).unwrap();
        let out = read_records(&path, false);
        assert_eq!(out.records.len(), 1);
        // Physical line number, counting the malformed line before it.
        assert_eq!(out.records[0].line, 2);
        // `not json` and the truncated record both fail to parse.
        assert_eq!(out.skipped, 2);
    }

    #[test]
    fn records_inside_a_turn_follow_seq_and_turns_first_appearance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        // A tool call written after the MCP call that followed it.
        let body = format!(
            "{}\n{}\n{}\n{}\n",
            line("c1", "c1-1", 0, start()),
            line("c2", "c2-1", 0, start()),
            line("c1", "c1-1", 2, tool(1, 1)),
            line("c1", "c1-1", 1, tool(2, 2)),
        );
        std::fs::write(&path, body).unwrap();
        let turns = group_turns(read_records(&path, false).records);
        assert_eq!(turns[0].turn_id.as_deref(), Some("c1-1"));
        assert_eq!(turns[1].turn_id.as_deref(), Some("c2-1"));
        let seqs: Vec<u32> = turns[0].records.iter().map(|e| e.record.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2]);
    }

    #[test]
    fn unattributed_records_group_into_one_bucket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let mcp = serde_json::to_string(&HistoryRecord {
            v: SCHEMA_VERSION,
            ts: "2026-09-15T18:43:44.512Z".into(),
            conv_id: None,
            turn_id: None,
            seq: 0,
            payload: HistoryKind::McpCall(McpCall {
                tool: "memory_search".into(),
                input: serde_json::json!({}),
                output: serde_json::json!({"results": []}),
                duration_us: 10,
                error: None,
                empty_result: true,
                input_truncated: false,
                output_truncated: false,
                input_tokens_est: 1,
                output_tokens_est: 1,
                estimator: Estimator::CharsDiv4,
                attribution: Attribution::Unattributed,
            }),
        })
        .unwrap();
        std::fs::write(&path, format!("{mcp}\n{mcp}\n")).unwrap();
        let turns = group_turns(read_records(&path, false).records);
        assert_eq!(turns.len(), 1);
        assert!(!turns[0].attributed);
        assert_eq!(turns[0].summary.mcp_count, 2);
    }

    #[test]
    fn read_turn_finds_a_turn_in_the_rotated_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        let rotated = rotated_path(&path);
        std::fs::write(&rotated, format!("{}\n", line("c1", "old-1", 0, start()))).unwrap();
        std::fs::write(&path, format!("{}\n", line("c1", "new-1", 0, start()))).unwrap();

        assert!(read_turn(&path, "old-1", true).is_some());
        assert!(read_turn(&path, "old-1", false).is_none());
        assert!(read_turn(&path, "new-1", true).is_some());
        assert!(read_turn(&path, "absent", true).is_none());
    }

    #[test]
    fn rotation_through_the_recorder_keeps_both_generations_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.ndjson");
        // A cap below one record forces a rotation on every append.
        let r = crate::history::HistoryRecorder::with_path_and_cap(path.clone(), 64);
        let start = |p: &str| TurnStart {
            provider: "claude".into(),
            model: "claude:sonnet".into(),
            conv_title: None,
            workspace_root: "C:/w".into(),
            prompt: p.into(),
            prompt_bytes: 0,
            prompt_truncated: false,
            input_tokens_est: None,
            estimator: None,
        };
        r.begin_turn("c1", "c1-1", start("first"), false);
        r.begin_turn("c1", "c1-2", start("second"), false);
        assert!(rotated_path(&path).exists());
        assert!(read_turn(&path, "c1-1", true).is_some());
        assert!(read_turn(&path, "c1-2", false).is_some());
        assert_eq!(read_records(&path, true).skipped, 0);
    }

    #[test]
    fn missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let out = read_records(&dir.path().join("absent.ndjson"), true);
        assert!(out.records.is_empty());
        assert_eq!(out.skipped, 0);
    }
}
