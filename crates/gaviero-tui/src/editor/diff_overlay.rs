use ratatui::{
    buffer::Buffer as RataBuf,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use gaviero_core::types::{HunkStatus, HunkType, WriteProposal};

use crate::theme::Theme;

/// Where the diff originated from.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffSource {
    /// From the AI agent (ACP pipeline) — user can accept/reject/finalize.
    Acp,
    /// From an external tool or editor — read-only display, dismiss only.
    External,
}

/// State for the diff review overlay.
///
/// Owns a local copy of the `WriteProposal` so that rendering and user
/// actions never need to lock the write gate.
pub struct DiffReviewState {
    /// The proposal being reviewed (owned, lock-free).
    pub proposal: WriteProposal,
    /// Where this diff came from.
    pub source: DiffSource,
    /// Index of the currently focused hunk.
    pub current_hunk: usize,
    /// Scroll offset (top visible line in the merged view).
    pub scroll_top: usize,
    /// Pending `]` or `[` key for two-key hunk navigation.
    pub pending_bracket: Option<char>,
    /// Cached diff lines — rebuilt only when proposal hunks change.
    cached_lines: Vec<DiffLine>,
    /// Whether the cache needs rebuilding.
    cache_dirty: bool,
}

impl DiffReviewState {
    pub fn new(proposal: WriteProposal, source: DiffSource) -> Self {
        let cached_lines = build_diff_lines(&proposal);
        Self {
            proposal,
            source,
            current_hunk: 0,
            scroll_top: 0,
            pending_bracket: None,
            cached_lines,
            cache_dirty: false,
        }
    }

    /// Get the cached diff lines, rebuilding if dirty.
    pub(crate) fn diff_lines(&mut self) -> &[DiffLine] {
        if self.cache_dirty {
            self.cached_lines = build_diff_lines(&self.proposal);
            self.cache_dirty = false;
        }
        &self.cached_lines
    }

    /// Mark cache as needing rebuild (call after hunk status changes).
    fn invalidate_cache(&mut self) {
        self.cache_dirty = true;
    }

    pub fn hunk_count(&self) -> usize {
        self.proposal.structural_hunks.len()
    }

    pub fn next_hunk(&mut self) {
        let total = self.hunk_count();
        if total > 0 && self.current_hunk < total - 1 {
            self.current_hunk += 1;
        }
    }

    pub fn prev_hunk(&mut self) {
        if self.current_hunk > 0 {
            self.current_hunk -= 1;
        }
    }

    /// Accept the hunk at `index` (local mutation, no lock).
    pub fn accept_hunk(&mut self, index: usize) {
        if let Some(hunk) = self.proposal.structural_hunks.get_mut(index) {
            hunk.status = HunkStatus::Accepted;
        }
        update_proposal_status(&mut self.proposal);
        self.invalidate_cache();
    }

    /// Reject the hunk at `index` (local mutation, no lock).
    pub fn reject_hunk(&mut self, index: usize) {
        if let Some(hunk) = self.proposal.structural_hunks.get_mut(index) {
            hunk.status = HunkStatus::Rejected;
        }
        update_proposal_status(&mut self.proposal);
        self.invalidate_cache();
    }

    /// Accept all hunks.
    pub fn accept_all(&mut self) {
        for hunk in &mut self.proposal.structural_hunks {
            hunk.status = HunkStatus::Accepted;
        }
        update_proposal_status(&mut self.proposal);
        self.invalidate_cache();
    }

    /// Reject all hunks.
    pub fn reject_all(&mut self) {
        for hunk in &mut self.proposal.structural_hunks {
            hunk.status = HunkStatus::Rejected;
        }
        update_proposal_status(&mut self.proposal);
        self.invalidate_cache();
    }

    /// Whether this diff allows interactive accept/reject.
    pub fn is_interactive(&self) -> bool {
        self.source == DiffSource::Acp
    }
}

fn update_proposal_status(proposal: &mut WriteProposal) {
    use gaviero_core::types::ProposalStatus;
    let all_accepted = proposal
        .structural_hunks
        .iter()
        .all(|h| h.status == HunkStatus::Accepted);
    let all_rejected = proposal
        .structural_hunks
        .iter()
        .all(|h| h.status == HunkStatus::Rejected);
    let any_accepted = proposal
        .structural_hunks
        .iter()
        .any(|h| h.status == HunkStatus::Accepted);

    if all_accepted {
        proposal.status = ProposalStatus::Accepted;
    } else if all_rejected {
        proposal.status = ProposalStatus::Rejected;
    } else if any_accepted {
        proposal.status = ProposalStatus::PartiallyAccepted;
    } else {
        proposal.status = ProposalStatus::Pending;
    }
}

/// A line in the merged diff view.
#[derive(Clone, Debug)]
pub(crate) struct DiffLine {
    /// The text content (without trailing newline).
    text: String,
    /// What kind of line this is.
    kind: DiffLineKind,
    /// Index of the hunk this line belongs to (None for context lines).
    hunk_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
enum DiffLineKind {
    Context,
    Added,
    Removed,
    /// A separator bar showing function grouping info.
    NodeBar(String), // content of the bar
}

/// Build the merged diff view lines from a proposal.
fn build_diff_lines(proposal: &WriteProposal) -> Vec<DiffLine> {
    let original_lines: Vec<&str> = proposal.original_content.lines().collect();
    let mut lines = Vec::new();
    let mut orig_idx = 0;

    for (hunk_i, shunk) in proposal.structural_hunks.iter().enumerate() {
        let dh = &shunk.diff_hunk;

        // Context lines before this hunk
        while orig_idx < dh.original_range.0 && orig_idx < original_lines.len() {
            lines.push(DiffLine {
                text: original_lines[orig_idx].to_string(),
                kind: DiffLineKind::Context,
                hunk_index: None,
            });
            orig_idx += 1;
        }

        // Removed / original lines from this hunk
        match dh.hunk_type {
            HunkType::Removed | HunkType::Modified => {
                for line in dh.original_text.lines() {
                    lines.push(DiffLine {
                        text: line.to_string(),
                        kind: DiffLineKind::Removed,
                        hunk_index: Some(hunk_i),
                    });
                }
            }
            HunkType::Added => {}
        }

        // Added / proposed lines from this hunk
        match dh.hunk_type {
            HunkType::Added | HunkType::Modified => {
                for line in dh.proposed_text.lines() {
                    lines.push(DiffLine {
                        text: line.to_string(),
                        kind: DiffLineKind::Added,
                        hunk_index: Some(hunk_i),
                    });
                }
            }
            HunkType::Removed => {}
        }

        orig_idx = dh.original_range.1;

        // Node grouping bar (if there's an enclosing node)
        if let Some(ref node) = shunk.enclosing_node {
            let name = node.name.as_deref().unwrap_or("<anonymous>");
            // Only show bar at the last hunk in this node group
            let is_last_in_group = proposal
                .structural_hunks
                .get(hunk_i + 1)
                .map(|next| {
                    next.enclosing_node.as_ref().and_then(|n| n.name.as_deref()) != Some(name)
                })
                .unwrap_or(true);

            if is_last_in_group {
                let status_char = match shunk.status {
                    HunkStatus::Accepted => '✓',
                    HunkStatus::Rejected => '✗',
                    HunkStatus::Pending => '?',
                };
                let bar = format!(
                    "── {} {} ── [{}] [a]ccept fn  [r]eject fn ──",
                    node.kind.replace('_', " "),
                    name,
                    status_char,
                );
                lines.push(DiffLine {
                    text: bar,
                    kind: DiffLineKind::NodeBar(name.to_string()),
                    hunk_index: Some(hunk_i),
                });
            }
        }
    }

    // Remaining context lines
    while orig_idx < original_lines.len() {
        lines.push(DiffLine {
            text: original_lines[orig_idx].to_string(),
            kind: DiffLineKind::Context,
            hunk_index: None,
        });
        orig_idx += 1;
    }

    lines
}

/// Render the diff overlay into the given area.
pub fn render_diff_overlay(
    area: Rect,
    buf: &mut RataBuf,
    state: &mut DiffReviewState,
    theme: &Theme,
) {
    if area.width < 6 || area.height == 0 {
        return;
    }

    // Ensure cache is fresh, then borrow immutably for the rest of rendering
    let _ = state.diff_lines();
    let diff_lines = &state.cached_lines;
    let proposal = &state.proposal;

    // Auto-scroll to keep current hunk visible
    let current_hunk_first_line = diff_lines
        .iter()
        .position(|l| l.hunk_index == Some(state.current_hunk))
        .unwrap_or(0);
    let scroll_top = if current_hunk_first_line < state.scroll_top {
        current_hunk_first_line
    } else if current_hunk_first_line >= state.scroll_top + area.height as usize {
        current_hunk_first_line.saturating_sub(area.height as usize / 3)
    } else {
        state.scroll_top
    };

    let gutter_w: u16 = 5; // " + │ " or " - │ " or "   │ "
    let _code_w = area.width.saturating_sub(gutter_w);

    for row in 0..area.height as usize {
        let line_idx = scroll_top + row;
        if line_idx >= diff_lines.len() {
            break;
        }

        let dl = &diff_lines[line_idx];
        let y = area.y + row as u16;
        let is_current_hunk = dl.hunk_index == Some(state.current_hunk);

        // Gutter
        let (gutter_str, gutter_style) = match &dl.kind {
            DiffLineKind::Added => (
                " + │ ",
                Style::default()
                    .fg(Color::Rgb(80, 200, 80))
                    .add_modifier(Modifier::BOLD),
            ),
            DiffLineKind::Removed => (
                " - │ ",
                Style::default()
                    .fg(Color::Rgb(220, 80, 80))
                    .add_modifier(Modifier::BOLD),
            ),
            DiffLineKind::Context => ("   │ ", Style::default().fg(Color::Rgb(99, 109, 131))),
            DiffLineKind::NodeBar(_) => ("   │ ", Style::default().fg(Color::Rgb(97, 175, 239))),
        };

        for (i, ch) in gutter_str.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() {
                buf[(x, y)].set_char(ch).set_style(gutter_style);
            }
        }

        // Line content
        let line_style = match &dl.kind {
            DiffLineKind::Added => {
                let hunk_status = dl
                    .hunk_index
                    .and_then(|i| proposal.structural_hunks.get(i))
                    .map(|h| &h.status);
                match hunk_status {
                    Some(HunkStatus::Accepted) => theme.default_style().bg(Color::Rgb(45, 74, 48)),
                    Some(HunkStatus::Rejected) => theme
                        .default_style()
                        .fg(Color::Rgb(120, 120, 120))
                        .add_modifier(Modifier::CROSSED_OUT)
                        .bg(Color::Rgb(55, 45, 45)),
                    _ => theme.default_style().bg(Color::Rgb(40, 65, 42)),
                }
            }
            DiffLineKind::Removed => {
                let hunk_status = dl
                    .hunk_index
                    .and_then(|i| proposal.structural_hunks.get(i))
                    .map(|h| &h.status);
                match hunk_status {
                    Some(HunkStatus::Accepted) => theme
                        .default_style()
                        .fg(Color::Rgb(120, 120, 120))
                        .add_modifier(Modifier::CROSSED_OUT)
                        .bg(Color::Rgb(55, 45, 45)),
                    Some(HunkStatus::Rejected) => theme.default_style().bg(Color::Rgb(74, 45, 45)),
                    _ => theme.default_style().bg(Color::Rgb(65, 40, 40)),
                }
            }
            DiffLineKind::Context => theme.default_style(),
            DiffLineKind::NodeBar(_) => Style::default()
                .fg(Color::Rgb(97, 175, 239))
                .add_modifier(Modifier::DIM),
        };

        // Highlight current hunk with a brighter left edge
        let line_style = if is_current_hunk && dl.kind != DiffLineKind::Context {
            line_style.add_modifier(Modifier::BOLD)
        } else {
            line_style
        };

        let code_start = area.x + gutter_w;
        for (i, ch) in dl.text.chars().enumerate() {
            let x = code_start + i as u16;
            if x >= area.right() {
                break;
            }
            buf[(x, y)].set_char(ch).set_style(line_style);
        }
        // Fill remaining width with background
        let text_end = code_start + dl.text.len() as u16;
        for x in text_end..area.right() {
            buf[(x, y)].set_style(line_style);
        }
    }
}

/// Find which hunk index is at a given row in the diff view.
pub fn hunk_at_row(state: &mut DiffReviewState, row: usize) -> Option<usize> {
    let _ = state.diff_lines(); // ensure cache is fresh
    let line_idx = state.scroll_top + row;
    state.cached_lines.get(line_idx).and_then(|l| l.hunk_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gaviero_core::types::*;

    fn make_proposal(original: &str, proposed: &str) -> WriteProposal {
        use gaviero_core::diff_engine::compute_hunks;

        let hunks = compute_hunks(original, proposed);
        let structural_hunks = hunks
            .into_iter()
            .map(|h| StructuralHunk {
                description: String::new(),
                enclosing_node: None,
                status: HunkStatus::Pending,
                diff_hunk: h,
            })
            .collect();

        WriteProposal {
            id: 1,
            source: "test".into(),
            file_path: "test.rs".into(),
            original_content: original.into(),
            proposed_content: proposed.into(),
            structural_hunks,
            status: ProposalStatus::Pending,
        }
    }

    #[test]
    fn test_build_diff_lines_modification() {
        let proposal = make_proposal("aaa\nbbb\nccc\n", "aaa\nBBB\nccc\n");
        let lines = build_diff_lines(&proposal);

        // Should have: context(aaa), removed(bbb), added(BBB), context(ccc)
        assert!(
            lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Context && l.text == "aaa")
        );
        assert!(
            lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Removed && l.text == "bbb")
        );
        assert!(
            lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Added && l.text == "BBB")
        );
        assert!(
            lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Context && l.text == "ccc")
        );
    }

    #[test]
    fn test_build_diff_lines_addition() {
        let proposal = make_proposal("aaa\nccc\n", "aaa\nbbb\nccc\n");
        let lines = build_diff_lines(&proposal);
        assert!(
            lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Added && l.text == "bbb")
        );
    }

    #[test]
    fn test_hunk_navigation() {
        let proposal = make_proposal("aaa\nbbb\nccc\nddd\n", "aaa\nBBB\nccc\nDDD\n");
        let mut state = DiffReviewState::new(proposal, DiffSource::Acp);
        assert_eq!(state.current_hunk, 0);
        state.next_hunk();
        assert_eq!(state.current_hunk, 1);
        state.next_hunk(); // at end, shouldn't go further
        assert_eq!(state.current_hunk, 1);
        state.prev_hunk();
        assert_eq!(state.current_hunk, 0);
    }

    #[test]
    fn test_accept_reject_local() {
        let proposal = make_proposal("aaa\nbbb\nccc\nddd\n", "aaa\nBBB\nccc\nDDD\n");
        let mut state = DiffReviewState::new(proposal, DiffSource::Acp);
        state.accept_hunk(0);
        assert_eq!(
            state.proposal.structural_hunks[0].status,
            HunkStatus::Accepted
        );
        state.reject_hunk(1);
        assert_eq!(
            state.proposal.structural_hunks[1].status,
            HunkStatus::Rejected
        );
        assert_eq!(state.proposal.status, ProposalStatus::PartiallyAccepted);
    }
}
