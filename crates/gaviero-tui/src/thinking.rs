//! Chain-of-thought display reduction for the agent chat.
//!
//! Every provider that emits reasoning frames it as literal `<think>` /
//! `</think>` text inside the assistant message — the in-process tool agent
//! (`agent_session/tool_agent/agent_loop.rs`), the ACP client
//! (`agent_session/registry.rs`), and the swarm executor
//! (`swarm/backend/executor.rs`) all stream those two tags through
//! `on_stream_chunk`. The TUI stores that framing verbatim in
//! `ChatMessage.content`, which is also the string it replays to the model, so
//! without a reduction the whole raw chain of thought reaches the screen.
//!
//! DeepSeek's chain of thought makes that unworkable. Chat repaints are
//! coalesced to one per 100 ms (`CHAT_STREAM_RENDER_INTERVAL`, `main.rs`), and
//! a reasoning burst appends far more than a screenful between frames: the
//! transcript scrolls faster than it can be read, and the block is still there
//! afterwards, burying the answer under thousands of lines of deliberation.
//!
//! [`collapse_thinking`] is the single reduction applied by every consumer
//! that is not the model itself:
//!
//! * the chat transcript (`panels::agent_chat`),
//! * the remote mirror DTO (`app::projection`),
//! * the post-turn memory transcript (`app::chat_memory`),
//! * the `/history` audit log (`app::observers`).
//!
//! Replay history and `/handoff` are deliberately *not* on that list: they
//! read `ChatMessage.content` directly, so the model keeps seeing exactly what
//! it produced. See `panels::agent_chat::context_messages_at`.

/// Provider framing for a reasoning block, written by the core stream
/// consumers. Both tags always sit on their own line, though
/// `panels::chat_markdown` also tolerates them inline.
pub const THINKING_OPEN: &str = "<think>";
pub const THINKING_CLOSE: &str = "</think>";

/// Characters of an in-flight reasoning block echoed under the header.
///
/// A rolling character window rather than whole lines: chain-of-thought
/// paragraphs are long, so N lines of body is N wrapped paragraphs on screen.
/// 240 characters is about three visual lines at 80 columns — enough to follow
/// what the model is doing, small enough that the ticker does not churn.
const TAIL_CHARS: usize = 240;

/// How much of a reasoning block the chat transcript shows.
///
/// Defaults to [`ThinkingDisplay::Brief`]: the detail is one `/reasoning full`
/// away, but it is not what the transcript puts in front of you by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThinkingDisplay {
    /// The raw chain of thought, exactly as the provider streamed it.
    Full,
    /// One header line per block, plus a rolling tail while it streams.
    #[default]
    Brief,
    /// Nothing at all — the block is dropped from the rendered text.
    Off,
}

impl ThinkingDisplay {
    /// The persisted name, as stored in `StoredConversation::thinking_display`.
    pub fn as_setting(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Brief => "brief",
            Self::Off => "off",
        }
    }

    /// Parse a persisted name. Unknown values (a newer build's mode, or a
    /// hand-edited file) fall back to the default rather than failing the load.
    pub fn from_setting(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "full" | "all" | "raw" => Some(Self::Full),
            "brief" | "tail" | "summary" => Some(Self::Brief),
            "off" | "none" | "hide" | "hidden" => Some(Self::Off),
            _ => None,
        }
    }
}

/// Rewrite `<think>` regions of `text` for display.
///
/// [`ThinkingDisplay::Full`], and any text carrying no framing, comes back
/// verbatim. [`ThinkingDisplay::Brief`] replaces each closed block with a
/// `▸ thinking (N lines)` header and each still-open block with a header plus
/// the tail of what has arrived so far. [`ThinkingDisplay::Off`] drops the
/// bodies entirely.
///
/// The framing tags are what makes this safe to run on every frame: text
/// outside a block — the answer, `[Tool: …]` markers interleaved by the loop —
/// passes through byte-identical, so the collapse never reorders the
/// transcript.
pub fn collapse_thinking(text: &str, mode: ThinkingDisplay) -> String {
    if mode == ThinkingDisplay::Full || !text.contains(THINKING_OPEN) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find(THINKING_OPEN) {
        out.push_str(&rest[..open]);
        let after_open = &rest[open + THINKING_OPEN.len()..];
        let (body, next, closed) = match after_open.find(THINKING_CLOSE) {
            Some(close) => (
                &after_open[..close],
                &after_open[close + THINKING_CLOSE.len()..],
                true,
            ),
            // Still streaming: everything after the open tag is body, and
            // nothing follows it yet.
            None => (after_open, "", false),
        };

        if mode == ThinkingDisplay::Brief {
            out.push_str(&brief_block(body, closed));
            // The tags sit on their own lines, so the block's trailing newline
            // is already the first character of `next`. Add one only when the
            // framing was inline and that break is missing.
            if !next.is_empty() && !next.starts_with('\n') {
                out.push('\n');
            }
        }

        if !closed {
            return out;
        }
        rest = next;
    }
    out.push_str(rest);
    out
}

/// The one-line replacement for a block: a header, and for an in-flight block
/// the tail of what has arrived so far.
fn brief_block(body: &str, closed: bool) -> String {
    let lines = body.lines().filter(|line| !line.trim().is_empty()).count();
    let plural = if lines == 1 { "" } else { "s" };
    if closed {
        return format!("▸ thinking ({lines} line{plural})");
    }
    format!(
        "▸ thinking… ({lines} line{plural} so far)\n    {}",
        tail(body)
    )
}

/// The last [`TAIL_CHARS`] characters of `body`, with whitespace runs folded so
/// the ticker stays a couple of rendered lines tall.
fn tail(body: &str) -> String {
    let flattened = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let start = flattened
        .char_indices()
        .rev()
        .nth(TAIL_CHARS.saturating_sub(1))
        .map(|(index, _)| index)
        .unwrap_or(0);
    if start == 0 {
        return flattened;
    }
    format!("…{}", &flattened[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOSED: &str = "Answer before.\n<think>\ndeliberation one\ndeliberation two\n</think>\nAnswer after.";

    #[test]
    fn full_returns_the_text_untouched() {
        assert_eq!(collapse_thinking(CLOSED, ThinkingDisplay::Full), CLOSED);
    }

    #[test]
    fn text_without_framing_is_untouched_by_every_mode() {
        let plain = "Just an answer, no reasoning.\nSecond line.";
        for mode in [
            ThinkingDisplay::Full,
            ThinkingDisplay::Brief,
            ThinkingDisplay::Off,
        ] {
            assert_eq!(collapse_thinking(plain, mode), plain, "{mode:?}");
        }
    }

    #[test]
    fn brief_collapses_a_closed_block_to_one_header() {
        let shown = collapse_thinking(CLOSED, ThinkingDisplay::Brief);
        assert_eq!(
            shown,
            "Answer before.\n▸ thinking (2 lines)\nAnswer after."
        );
        assert!(!shown.contains(THINKING_OPEN));
        assert!(!shown.contains("deliberation"));
    }

    /// The whole point of the mode: while the model is still reasoning the
    /// block is open, so the header carries a tail of the live text.
    #[test]
    fn brief_keeps_a_rolling_tail_while_streaming() {
        let partial = "Answer.\n<think>\nfirst step\nsecond step";
        let shown = collapse_thinking(partial, ThinkingDisplay::Brief);
        assert!(shown.starts_with("Answer.\n▸ thinking… (2 lines so far)\n    "), "{shown:?}");
        assert!(shown.contains("second step"), "{shown:?}");
        assert!(shown.contains("first step"), "{shown:?}");
    }

    #[test]
    fn the_streaming_tail_is_a_bounded_window() {
        let long = "x".repeat(4_000);
        let partial = format!("<think>\n{long}");
        let shown = collapse_thinking(&partial, ThinkingDisplay::Brief);
        let body = shown.split("\n    ").nth(1).expect("a tail line");
        // The window, the ellipsis marking what it dropped, and nothing else.
        assert_eq!(body.chars().count(), TAIL_CHARS + 1, "{body}");
        assert!(body.starts_with('…'), "{body}");
    }

    #[test]
    fn tail_folds_whitespace_runs_so_it_stays_a_couple_of_lines() {
        let partial = "<think>\nstep one\n\n   step two   \n";
        let shown = collapse_thinking(partial, ThinkingDisplay::Brief);
        assert!(shown.ends_with("step one step two"), "{shown:?}");
    }

    #[test]
    fn off_drops_the_block_entirely() {
        let shown = collapse_thinking(CLOSED, ThinkingDisplay::Off);
        assert!(!shown.contains("deliberation"), "{shown:?}");
        assert!(shown.contains("Answer before."), "{shown:?}");
        assert!(shown.contains("Answer after."), "{shown:?}");
    }

    /// One DeepSeek turn opens a block per tool round, so a single message
    /// carries several — each must collapse on its own.
    #[test]
    fn every_block_in_a_message_collapses() {
        let text = "a\n<think>\none\n</think>\n[Bash: ls]\nb\n<think>\ntwo\n</think>\nc";
        assert_eq!(
            collapse_thinking(text, ThinkingDisplay::Brief),
            "a\n▸ thinking (1 line)\n[Bash: ls]\nb\n▸ thinking (1 line)\nc"
        );
    }

    /// `chat_markdown` tolerates inline tags; the collapse must not assume the
    /// framing always arrives on its own lines.
    #[test]
    fn inline_framing_keeps_the_line_break() {
        let text = "before<think>hmm</think>after";
        assert_eq!(
            collapse_thinking(text, ThinkingDisplay::Brief),
            "before▸ thinking (1 line)\nafter"
        );
    }

    /// An unclosed block swallows the rest of the text: nothing can follow a
    /// block brace that has not closed.
    #[test]
    fn an_open_block_at_the_end_keeps_everything_before_it() {
        let shown = collapse_thinking("answer\n<think>\nstep", ThinkingDisplay::Brief);
        assert!(shown.starts_with("answer\n▸ thinking… (1 line so far)"), "{shown:?}");
    }

    #[test]
    fn mode_names_round_trip_and_unknown_names_are_rejected() {
        for mode in [
            ThinkingDisplay::Full,
            ThinkingDisplay::Brief,
            ThinkingDisplay::Off,
        ] {
            assert_eq!(ThinkingDisplay::from_setting(mode.as_setting()), Some(mode));
        }
        assert_eq!(ThinkingDisplay::from_setting("  FULL "), Some(ThinkingDisplay::Full));
        assert_eq!(ThinkingDisplay::from_setting("verbose"), None);
        assert_eq!(ThinkingDisplay::default(), ThinkingDisplay::Brief);
    }
}
