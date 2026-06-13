use std::path::PathBuf;

use ratatui::layout::Rect;

pub(super) use crate::editor::diff::{DiffKind, build_simple_diff};
use crate::editor::highlight::StyledSpan;

/// Tree-sitter highlight spans precomputed for a diff view, indexed by the
/// byte offset of each diff line in a synthetic concatenated source. Stored
/// alongside `cached_diff` so it's recomputed only when the selected file
/// changes, not on every render frame.
pub(super) struct DiffHighlightCache {
    /// `line_start_bytes[i]` is the byte offset where diff line `i` starts in
    /// the synthetic source that tree-sitter parsed.
    pub line_start_bytes: Vec<usize>,
    /// Highlight spans sorted by start_byte; the rendering loop applies them
    /// in order and the last matching span wins (matches `editor::view`).
    pub spans: Vec<StyledSpan>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Editor,
    FileTree,
    SidePanel,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LeftPanelMode {
    FileTree,
    Search,
    Review,
    Changes,
}

/// Markdown buffer preview layout, cycled with Alt+P.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MarkdownPreviewMode {
    /// Source only (no rendered pane).
    #[default]
    Off,
    /// Source and rendered preview side by side.
    Split,
    /// Rendered preview only (source hidden).
    PreviewOnly,
}

impl MarkdownPreviewMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::Split,
            Self::Split => Self::PreviewOnly,
            Self::PreviewOnly => Self::Off,
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn title_label(self) -> &'static str {
        match self {
            Self::Off => "Markdown",
            Self::Split => "Markdown · split (Alt+P)",
            Self::PreviewOnly => "Markdown · preview (Alt+P)",
        }
    }
}

#[cfg(test)]
mod preview_mode_tests {
    use super::MarkdownPreviewMode;

    #[test]
    fn cycle_off_split_preview_only_off() {
        assert_eq!(
            MarkdownPreviewMode::Off.cycle(),
            MarkdownPreviewMode::Split
        );
        assert_eq!(
            MarkdownPreviewMode::Split.cycle(),
            MarkdownPreviewMode::PreviewOnly
        );
        assert_eq!(
            MarkdownPreviewMode::PreviewOnly.cycle(),
            MarkdownPreviewMode::Off
        );
    }
}

#[derive(Clone, Debug)]
pub struct ReviewProposal {
    /// Stable identifier copied from the source `WriteProposal` so the UI can
    /// detect conflict peers across the open batch even after re-sorting.
    pub id: u64,
    /// Agent/source name carried over from `WriteProposal.source`. Rendered
    /// as a short badge so the reviewer sees which provider produced the
    /// proposal in a mixed-batch inbox.
    pub source: String,
    /// Conversation that produced this proposal. Kept for future per-conv
    /// filtering and audit; the v1 filter cycles by `source` (agent name)
    /// because providers like `claude-chat` use a constant agent_id across
    /// conversations.
    #[allow(dead_code)]
    pub conv_id: Option<String>,
    /// IDs of other proposals in the same batch that target the same path.
    /// Non-empty means this proposal is half of a conflict pair.
    pub conflicts_with: Vec<u64>,
    /// Set to true after the conflicting peer was accepted. A superseded
    /// proposal is rendered with `⊘` and cannot be applied — Unit 7 enforces
    /// pick-one resolution.
    pub superseded: bool,
    pub path: PathBuf,
    pub old_content: Option<String>,
    pub new_content: String,
    pub additions: usize,
    pub deletions: usize,
    /// True when accepting the proposal removes the file from disk rather
    /// than writing `new_content`.
    pub is_deletion: bool,
}

pub struct BatchReviewState {
    pub proposals: Vec<ReviewProposal>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub diff_scroll: usize,
    pub(super) cached_diff: Vec<(DiffKind, String)>,
    pub(super) cached_diff_index: usize,
    pub(super) cached_highlights: Option<DiffHighlightCache>,
    /// Active provider filter (matches `ReviewProposal.source`). `None` means
    /// "show everything". Cycled with `Alt+o` / `Alt+i`.
    pub filter_source: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChangesEntry {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub status_char: char,
    pub additions: usize,
    pub deletions: usize,
    /// Git unmerged path and/or `<<<<<<<` markers in the working tree.
    pub is_conflict: bool,
}

pub struct ChangesState {
    pub entries: Vec<ChangesEntry>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub diff_scroll: usize,
    pub(super) cached_diff: Vec<(DiffKind, String)>,
    pub(super) cached_diff_index: usize,
    pub(super) cached_highlights: Option<DiffHighlightCache>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidePanelMode {
    AgentChat,
    #[allow(dead_code)]
    SwarmDashboard,
    #[allow(dead_code)]
    GitPanel,
    /// Tier A / A4: memory inspection panel. Activated via `Alt+m`.
    MemoryPanel,
}

#[derive(Debug, Clone)]
pub struct PanelVisibility {
    pub file_tree: bool,
    pub side_panel: bool,
    pub terminal: bool,
}

#[derive(Debug, Clone)]
pub struct LayoutPreset {
    pub file_tree_pct: u16,
    #[allow(dead_code)]
    pub editor_pct: u16,
    pub side_panel_pct: u16,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ScrollbarTarget {
    Editor,
    /// Markdown rendered preview pane (split or preview-only).
    MarkdownPreview,
    Chat,
    LeftPanel,
    /// Diff pane while a review is active — single proposal (`diff_review`,
    /// scrolls `scroll_top`) or batch file-set (`batch_review`, scrolls
    /// `diff_scroll`). The active one is resolved at scroll time.
    ReviewDiff,
}

#[derive(Default, Clone)]
pub(super) struct LayoutAreas {
    pub tab_area: Rect,
    pub file_tree_area: Option<Rect>,
    pub left_header_area: Option<Rect>,
    pub editor_area: Rect,
    pub preview_area: Option<Rect>,
    pub side_panel_area: Option<Rect>,
    pub side_header_area: Option<Rect>,
    pub terminal_area: Option<Rect>,
    pub status_area: Rect,
}

#[derive(Debug, Clone)]
pub enum MoveState {
    SelectingSource,
    SelectingDest(PathBuf),
    Confirming(PathBuf, PathBuf),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum FirstRunStep {
    AskSettings,
    AskMemory,
}

#[derive(Debug, Clone)]
pub(super) struct FirstRunDialog {
    pub step: FirstRunStep,
    pub create_settings: bool,
}

/// Codex MCP trust prompt. Fires once before the first `/swarm` run
/// when `mcp.gavieroServer.codexTrust` is still "unknown". Answering
/// persists the choice to `.gaviero/settings.json` and resumes the
/// swarm command that was pending.
#[derive(Debug, Clone)]
pub(crate) struct CodexTrustDialog {
    /// The `/swarm <task>` description the user submitted, replayed
    /// after the prompt resolves (regardless of grant/deny).
    pub pending_task: String,
}

#[derive(Debug, Clone)]
pub enum TreeDialogKind {
    NewFile,
    NewFolder,
    Rename,
    Delete,
}

#[derive(Debug, Clone)]
pub struct TreeDialog {
    pub kind: TreeDialogKind,
    pub input: String,
    pub cursor: usize,
    pub target_dir: PathBuf,
    pub original_path: Option<PathBuf>,
}

/// State for bulk operations on selected files in the EXPLORER panel.
#[derive(Debug, Clone)]
pub enum BulkOpState {
    /// Waiting for [y/n] to confirm deletion of the listed paths.
    ConfirmDelete { paths: Vec<PathBuf> },
    /// User is navigating the file tree to pick a destination directory.
    SelectingDest { paths: Vec<PathBuf> },
    /// Waiting for [y/n] to confirm moving the listed paths to `dest_dir`.
    ConfirmMove { paths: Vec<PathBuf>, dest_dir: PathBuf },
}

impl TreeDialog {
    pub(super) fn new(kind: TreeDialogKind, target_dir: PathBuf) -> Self {
        Self {
            kind,
            input: String::new(),
            cursor: 0,
            target_dir,
            original_path: None,
        }
    }

    pub(super) fn prompt(&self) -> &str {
        match self.kind {
            TreeDialogKind::NewFile => "New file: ",
            TreeDialogKind::NewFolder => "New folder: ",
            TreeDialogKind::Rename => "Rename to: ",
            TreeDialogKind::Delete => "Delete (y/n)? ",
        }
    }

    pub(super) fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub(super) fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input.drain(self.cursor - prev..self.cursor);
            self.cursor -= prev;
        }
    }

    pub(super) fn delete(&mut self) {
        if self.cursor < self.input.len() {
            let next = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input.drain(self.cursor..self.cursor + next);
        }
    }

    pub(super) fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor -= prev;
        }
    }

    pub(super) fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            let next = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor += next;
        }
    }

    pub(super) fn move_home(&mut self) {
        self.cursor = 0;
    }

    pub(super) fn move_end(&mut self) {
        self.cursor = self.input.len();
    }
}
