use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;

use crate::editor::buffer::Buffer;
use crate::editor::diff_overlay::{self, DiffReviewState, DiffSource};
use crate::editor::highlight::{HighlightConfig, load_highlight_config};
use crate::editor::view::EditorView;
use crate::event::Event;
use crate::keymap::{Action, Keymap};
use crate::panels::agent_chat::AgentChatState;
use crate::panels::file_tree::FileTreeState;
use crate::panels::status_bar::StatusBar;
use crate::theme::{self, Theme};
use crate::widgets::tabs::TabBar;

use gaviero_core::acp::client::AcpPipeline;
use gaviero_core::memory::MemoryStore;
use gaviero_core::observer::{AcpObserver, WriteGateObserver};
use gaviero_core::session_state::{self, SessionState, TabState};
use gaviero_core::types::WriteProposal;
use gaviero_core::workspace::Workspace;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};

// ── Focus & panel types ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Editor,
    FileTree,
    SidePanel,
    Terminal,
}


/// What the left panel shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LeftPanelMode {
    FileTree,
    Search,
    Review,
    /// Git working-directory changes browser (cyclable via F7).
    Changes,
}

// ── Batch Review ─────────────────────────────────────────────────

/// A single proposed file change for batch review.
#[derive(Clone, Debug)]
pub struct ReviewProposal {
    pub path: PathBuf,
    pub old_content: Option<String>,
    pub new_content: String,
    pub additions: usize,
    pub deletions: usize,
}

/// State for the batch review mode entered after an agent response completes.
pub struct BatchReviewState {
    pub proposals: Vec<ReviewProposal>,
    pub selected_index: usize,
    /// Scroll offset for the file list in the left panel.
    pub scroll_offset: usize,
    /// Scroll offset for the diff view in the editor panel.
    pub diff_scroll: usize,
    /// Cached diff lines for the currently selected file.
    /// Recomputed only when `selected_index` changes.
    cached_diff: Vec<(DiffKind, String)>,
    cached_diff_index: usize,
}

#[derive(Clone, Debug)]
enum DiffKind {
    Context,
    Added,
    Removed,
}

/// Build a simple line-level diff using longest common subsequence.
fn build_simple_diff<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(DiffKind, String)> {
    // LCS-based diff
    let m = old.len();
    let n = new.len();

    // Build LCS table
    let mut dp = vec![vec![0u32; n + 1]; m + 1];
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if old[i - 1] == new[j - 1] {
                dp[i - 1][j - 1] + 1
            } else {
                dp[i - 1][j].max(dp[i][j - 1])
            };
        }
    }

    // Backtrack to produce diff
    let mut result = Vec::new();
    let (mut i, mut j) = (m, n);
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            result.push((DiffKind::Context, old[i - 1].to_string()));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            result.push((DiffKind::Added, new[j - 1].to_string()));
            j -= 1;
        } else {
            result.push((DiffKind::Removed, old[i - 1].to_string()));
            i -= 1;
        }
    }

    result.reverse();
    result
}

// ── Git Changes Panel ───────────────────────────────────────────

/// A single changed file from `git status`.
#[derive(Clone, Debug)]
pub struct ChangesEntry {
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub status_char: char,
    pub additions: usize,
    pub deletions: usize,
}

/// State for the git-changes left-panel mode.
pub struct ChangesState {
    pub entries: Vec<ChangesEntry>,
    pub selected_index: usize,
    pub scroll_offset: usize,
    /// Scroll offset for the diff view in the editor panel.
    pub diff_scroll: usize,
    /// Cached diff lines for the currently selected file.
    cached_diff: Vec<(DiffKind, String)>,
    cached_diff_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SidePanelMode {
    AgentChat,
    #[allow(dead_code)] // Wired but not yet launchable from TUI
    SwarmDashboard,
    #[allow(dead_code)]
    GitPanel,
}

#[derive(Debug, Clone)]
pub struct PanelVisibility {
    pub file_tree: bool,
    pub side_panel: bool,
    pub terminal: bool,
}

/// A named layout preset: percentage widths for each panel column.
/// Panels with 0% are hidden.
#[derive(Debug, Clone)]
pub struct LayoutPreset {
    pub file_tree_pct: u16,
    #[allow(dead_code)]
    pub editor_pct: u16,
    pub side_panel_pct: u16,
}

/// Which panel's scrollbar is being dragged.
#[derive(Debug, Clone, Copy)]
enum ScrollbarTarget {
    Editor,
    Chat,
    LeftPanel,
}

/// Cached layout areas from last render, used for mouse hit-testing.
#[derive(Default, Clone)]
struct LayoutAreas {
    tab_area: Rect,
    file_tree_area: Option<Rect>,
    left_header_area: Option<Rect>,
    editor_area: Rect,
    side_panel_area: Option<Rect>,
    side_header_area: Option<Rect>,
    terminal_area: Option<Rect>,
    status_area: Rect,
}

// ── TUI observer (bridges write gate → event channel) ────────────

struct TuiWriteGateObserver {
    tx: mpsc::UnboundedSender<Event>,
}

impl WriteGateObserver for TuiWriteGateObserver {
    fn on_proposal_created(&self, proposal: &WriteProposal) {
        let _ = self.tx.send(Event::ProposalCreated(Box::new(proposal.clone())));
    }
    fn on_proposal_updated(&self, proposal_id: u64) {
        let _ = self.tx.send(Event::ProposalUpdated(proposal_id));
    }
    fn on_proposal_finalized(&self, path: &str) {
        let _ = self.tx.send(Event::ProposalFinalized(path.to_string()));
    }
}

// ── TUI observer (bridges swarm → event channel) ────────────

#[allow(dead_code)] // Wired but swarm launch not yet exposed in TUI
pub struct TuiSwarmObserver {
    pub tx: mpsc::UnboundedSender<Event>,
}

impl gaviero_core::observer::SwarmObserver for TuiSwarmObserver {
    fn on_phase_changed(&self, phase: &str) {
        let _ = self.tx.send(Event::SwarmPhaseChanged(phase.to_string()));
    }
    fn on_agent_state_changed(&self, id: &str, status: &gaviero_core::swarm::models::AgentStatus, detail: &str) {
        let _ = self.tx.send(Event::SwarmAgentStateChanged {
            id: id.to_string(),
            status: status.clone(),
            detail: detail.to_string(),
        });
    }
    fn on_tier_started(&self, current: usize, total: usize) {
        let _ = self.tx.send(Event::SwarmTierStarted { current, total });
    }
    fn on_merge_conflict(&self, branch: &str, files: &[String]) {
        let _ = self.tx.send(Event::SwarmMergeConflict {
            branch: branch.to_string(),
            files: files.to_vec(),
        });
    }
    fn on_completed(&self, result: &gaviero_core::swarm::models::SwarmResult) {
        let _ = self.tx.send(Event::SwarmCompleted(Box::new(result.clone())));
    }
    fn on_coordination_started(&self, prompt: &str) {
        let _ = self.tx.send(Event::SwarmCoordinationStarted(prompt.to_string()));
    }
    fn on_coordination_complete(&self, dag: &gaviero_core::swarm::coordinator::TaskDAG) {
        let _ = self.tx.send(Event::SwarmCoordinationComplete {
            unit_count: dag.units.len(),
            summary: dag.plan_summary.clone(),
        });
    }
    fn on_tier_dispatch(&self, unit_id: &str, tier: gaviero_core::types::ModelTier, backend: &str) {
        let _ = self.tx.send(Event::SwarmTierDispatch {
            unit_id: unit_id.to_string(),
            tier,
            backend: backend.to_string(),
        });
    }
    fn on_cost_update(&self, estimate: &gaviero_core::swarm::verify::CostEstimate) {
        let _ = self.tx.send(Event::SwarmCostUpdate(estimate.clone()));
    }
}

// ── TUI observer (bridges ACP agent → event channel) ────────────

pub struct TuiAcpObserver {
    pub tx: mpsc::UnboundedSender<Event>,
    pub conv_id: String,
}

impl AcpObserver for TuiAcpObserver {
    fn on_stream_chunk(&self, text: &str) {
        let _ = self.tx.send(Event::StreamChunk {
            conv_id: self.conv_id.clone(),
            text: text.to_string(),
        });
    }
    fn on_tool_call_started(&self, tool_name: &str) {
        let _ = self.tx.send(Event::ToolCallStarted {
            conv_id: self.conv_id.clone(),
            tool_name: tool_name.to_string(),
        });
    }
    fn on_streaming_status(&self, status: &str) {
        let _ = self.tx.send(Event::StreamingStatus {
            conv_id: self.conv_id.clone(),
            status: status.to_string(),
        });
    }
    fn on_message_complete(&self, role: &str, content: &str) {
        let _ = self.tx.send(Event::MessageComplete {
            conv_id: self.conv_id.clone(),
            role: role.to_string(),
            content: content.to_string(),
        });
    }
    fn on_proposal_deferred(&self, path: &std::path::Path, old_content: Option<&str>, new_content: &str) {
        // Compute addition/deletion line counts for compact summary
        let old_lines = old_content.map(|s| s.lines().count()).unwrap_or(0);
        let new_lines = new_content.lines().count();
        let additions = new_lines.saturating_sub(old_lines);
        let deletions = old_lines.saturating_sub(new_lines);
        let _ = self.tx.send(Event::FileProposalDeferred {
            conv_id: self.conv_id.clone(),
            path: path.to_path_buf(),
            additions,
            deletions,
        });
    }
}

// ── Inline dialog for file tree operations ──────────────────────

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
    /// The parent directory where the new item will be created.
    pub target_dir: std::path::PathBuf,
    /// For rename/delete: the original path.
    pub original_path: Option<std::path::PathBuf>,
}

impl TreeDialog {
    fn new(kind: TreeDialogKind, target_dir: std::path::PathBuf) -> Self {
        Self {
            kind,
            input: String::new(),
            cursor: 0,
            target_dir,
            original_path: None,
        }
    }

    fn prompt(&self) -> &str {
        match self.kind {
            TreeDialogKind::NewFile => "New file: ",
            TreeDialogKind::NewFolder => "New folder: ",
            TreeDialogKind::Rename => "Rename to: ",
            TreeDialogKind::Delete => "Delete (y/n)? ",
        }
    }

    fn insert_char(&mut self, ch: char) {
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    fn backspace(&mut self) {
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

    fn delete(&mut self) {
        if self.cursor < self.input.len() {
            let next = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.input.drain(self.cursor..self.cursor + next);
        }
    }

    fn move_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor -= prev;
        }
    }

    fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            let next = self.input[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor += next;
        }
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.input.len();
    }
}

// ── Constants ────────────────────────────────────────────────────


/// Gutter column width used for mouse hit-testing in diff review mode.
const DIFF_GUTTER_WIDTH: u16 = 5;

// ── App ──────────────────────────────────────────────────────────

pub struct App {
    pub workspace: Workspace,
    pub buffers: Vec<Buffer>,
    pub active_buffer: usize,
    pub file_tree: FileTreeState,
    pub search_panel: crate::panels::search::SearchPanelState,
    pub swarm_dashboard: crate::panels::swarm_dashboard::SwarmDashboardState,
    pub left_panel: LeftPanelMode,
    pub side_panel: SidePanelMode,
    pub focus: Focus,
    pub panel_visible: PanelVisibility,
    pub should_quit: bool,
    /// When true, the main loop should call `terminal.clear()` before the next draw
    /// to force a full redraw and fix any terminal state corruption.
    pub needs_full_redraw: bool,
    #[allow(dead_code)]
    pub event_tx: mpsc::UnboundedSender<Event>,
    pub theme: Theme,
    highlight_configs: HashMap<String, HighlightConfig>,
    indent_query_cache: gaviero_core::indent::config::IndentQueryCache,
    layout: LayoutAreas,

    // Configurable panel sizes
    file_tree_width: u16,
    side_panel_width: u16,
    terminal_split_percent: u16,

    // Layout presets (0-9) and fullscreen toggle
    layout_presets: Vec<LayoutPreset>,
    active_preset: Option<usize>,
    fullscreen_panel: Option<Focus>,
    /// Saved visibility before fullscreen toggle.
    pre_fullscreen: Option<PanelVisibility>,

    // Clipboard (system + internal fallback)
    clipboard: Option<arboard::Clipboard>,
    internal_clipboard: String,

    // Mouse state
    mouse_dragging: bool,
    scrollbar_dragging: Option<ScrollbarTarget>,
    last_click: Option<(u16, u16, std::time::Instant)>, // (col, row, time) for double-click

    // Transient status message (shown for a few seconds, then cleared)
    status_message: Option<(String, std::time::Instant)>,

    // File tree dialog (new file/folder, rename, delete)
    tree_dialog: Option<TreeDialog>,

    // Editor find bar (Ctrl+F)
    pub find_bar_active: bool,
    pub find_input: crate::widgets::text_input::TextInput,

    // Markdown preview
    pub preview_visible: bool,
    pub preview_scroll: usize,

    // Write gate
    pub write_gate: Arc<Mutex<WriteGatePipeline>>,
    /// Unified diff review state — owns the proposal locally (no lock needed).
    pub diff_review: Option<DiffReviewState>,
    /// Batch review state — entered after agent response with deferred writes.
    pub batch_review: Option<BatchReviewState>,
    /// Git changes panel state — populated when cycling to Changes mode via F7.
    pub changes_state: Option<ChangesState>,

    // Agent chat
    pub chat_state: AgentChatState,
    acp_tasks: HashMap<String, tokio::task::JoinHandle<()>>,

    // Memory
    pub memory: Option<Arc<MemoryStore>>,

    // Git panel (M4)
    pub git_panel: crate::panels::git_panel::GitPanelState,
    pub git_repo: Option<gaviero_core::git::GitRepo>,

    // Terminal (M4) — managed by TerminalManager in gaviero-core
    pub terminal_manager: gaviero_core::terminal::TerminalManager,
    /// Terminal panel text selection state.
    pub terminal_selection: crate::panels::terminal::TerminalSelectionState,
}

impl App {
    pub fn new(workspace: Workspace, event_tx: mpsc::UnboundedSender<Event>) -> Self {
        let excludes = parse_exclude_patterns(&workspace);
        let git_allow = parse_git_allow_list(&workspace);
        let roots: Vec<&Path> = workspace.roots();
        let file_tree = FileTreeState::from_roots(&roots, &excludes, &git_allow);

        let theme = Theme::load(Path::new("themes/default.toml"))
            .unwrap_or_else(|_| Theme::builtin_default());

        // Read panel sizes from settings
        use gaviero_core::workspace::settings;
        let file_tree_width = workspace
            .resolve_setting(settings::FILE_TREE_WIDTH, None)
            .as_u64()
            .unwrap_or(30) as u16;
        let side_panel_width = workspace
            .resolve_setting(settings::SIDE_PANEL_WIDTH, None)
            .as_u64()
            .unwrap_or(40) as u16;
        let terminal_split_percent = workspace
            .resolve_setting(settings::TERMINAL_SPLIT_PERCENT, None)
            .as_u64()
            .unwrap_or(30) as u16;

        let layout_presets = parse_layout_presets(&workspace);

        // Read agent settings before workspace is moved
        let agent_model = workspace
            .resolve_setting(settings::AGENT_MODEL, None)
            .as_str()
            .unwrap_or("sonnet")
            .to_string();
        let agent_effort = workspace
            .resolve_setting(settings::AGENT_EFFORT, None)
            .as_str()
            .unwrap_or("off")
            .to_string();
        let agent_max_tokens = workspace
            .resolve_setting(settings::AGENT_MAX_TOKENS, None)
            .as_u64()
            .unwrap_or(16384) as u32;

        let write_namespace = workspace.resolve_namespace(None);
        let read_namespaces = workspace.resolve_read_namespaces(None);

        // Open git repo for git panel (M4)
        let git_repo = workspace.roots().first()
            .and_then(|r| gaviero_core::git::GitRepo::open(r).ok());

        let observer = TuiWriteGateObserver {
            tx: event_tx.clone(),
        };
        let write_gate = Arc::new(Mutex::new(WriteGatePipeline::new(
            WriteMode::Interactive,
            Box::new(observer),
        )));

        Self {
            workspace,
            buffers: Vec::new(),
            active_buffer: 0,
            file_tree,
            search_panel: crate::panels::search::SearchPanelState::new(),
            swarm_dashboard: crate::panels::swarm_dashboard::SwarmDashboardState::new(),
            left_panel: LeftPanelMode::FileTree,
            side_panel: SidePanelMode::AgentChat,
            focus: Focus::FileTree,
            panel_visible: PanelVisibility {
                file_tree: true,
                side_panel: false,
                terminal: false,
            },
            should_quit: false,
            needs_full_redraw: false,
            event_tx,
            theme,
            highlight_configs: HashMap::new(),
            indent_query_cache: gaviero_core::indent::config::IndentQueryCache::new(),
            layout: LayoutAreas::default(),
            file_tree_width,
            side_panel_width,
            terminal_split_percent,
            layout_presets,
            active_preset: None,
            fullscreen_panel: None,
            pre_fullscreen: None,
            clipboard: match arboard::Clipboard::new() {
                Ok(cb) => Some(cb),
                Err(e) => {
                    tracing::warn!("System clipboard unavailable: {}", e);
                    None
                }
            },
            internal_clipboard: String::new(),
            mouse_dragging: false,
            scrollbar_dragging: None,
            last_click: None,
            status_message: None,
            tree_dialog: None,
            find_bar_active: false,
            find_input: crate::widgets::text_input::TextInput::new(),
            preview_visible: false,
            preview_scroll: 0,
            write_gate,
            diff_review: None,
            batch_review: None,
            changes_state: None,
            chat_state: {
                let mut cs = AgentChatState::new();
                cs.agent_settings = crate::panels::agent_chat::AgentSettings {
                    model: agent_model,
                    effort: agent_effort,
                    max_tokens: agent_max_tokens,
                    write_namespace,
                    read_namespaces,
                };
                cs
            },
            acp_tasks: HashMap::new(),
            memory: None,
            git_panel: crate::panels::git_panel::GitPanelState::new(),
            git_repo,
            terminal_manager: gaviero_core::terminal::TerminalManager::new(
                gaviero_core::terminal::TerminalConfig::default(),
            ),
            terminal_selection: crate::panels::terminal::TerminalSelectionState::default(),
        }
    }

    /// Handle an incoming event.
    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                // Terminal intercepts raw keys when focused
                if self.focus == Focus::Terminal {
                    if let Some(inst) = self.terminal_manager.active_instance() {
                        if inst.spawned {
                            use crate::panels::terminal::{is_terminal_escape_key, key_event_to_bytes};
                            if is_terminal_escape_key(&key) {
                                let action = Keymap::resolve(&key);
                                self.handle_action(action);
                            } else {
                                let bytes = key_event_to_bytes(&key);
                                if !bytes.is_empty() {
                                    self.terminal_selection.clear();
                                    let inst = self.terminal_manager.active_instance_mut().unwrap();
                                    // Reset scrollback to live view when user types
                                    inst.screen_mut().set_scrollback(0);
                                    inst.write_input(&bytes);
                                }
                            }
                            return;
                        }
                    }
                }

                // If a tree dialog is active, route raw keys there
                if self.tree_dialog.is_some() {
                    self.handle_dialog_key(&key);
                    return;
                }

                // Esc closes the find bar if active
                if self.find_bar_active && key.code == crossterm::event::KeyCode::Esc {
                    self.find_bar_active = false;
                    if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                        buf.set_search_highlight(None);
                    }
                    return;
                }

                let action = Keymap::resolve(&key);
                self.handle_action(action);
            }
            Event::Paste(text) => {
                // Forward paste to terminal if focused
                if self.focus == Focus::Terminal {
                    if let Some(inst) = self.terminal_manager.active_instance_mut() {
                        if inst.spawned {
                            inst.write_input(text.as_bytes());
                            return;
                        }
                    }
                }
                self.handle_paste(&text);
            }
            Event::Mouse(mouse) => self.handle_mouse(mouse),
            Event::Resize(_w, _h) => {
                self.needs_full_redraw = true;
            }
            Event::FileChanged(path) => self.handle_file_changed(&path),
            Event::FileTreeChanged => {
                self.refresh_file_tree();
                self.refresh_git_panel();
            }
            Event::ProposalCreated(proposal) => {
                self.enter_review_mode(*proposal, DiffSource::Acp);
            }
            Event::ProposalUpdated(_id) => {} // Local state is authoritative
            Event::ProposalFinalized(path_str) => {
                // Open a tab for the file if not already open
                let path = std::path::PathBuf::from(&path_str);
                if path.exists() {
                    self.open_file(&path);
                }
                self.refresh_file_tree();
            }

            // ACP agent events — swarm agents route to dashboard, others to chat
            Event::StreamChunk { conv_id, text } => {
                if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                    self.swarm_dashboard.append_stream_chunk(agent_id, &text);
                } else {
                    self.chat_state.append_stream_chunk_to(&conv_id, &text);
                }
            }
            Event::ToolCallStarted { conv_id, tool_name } => {
                if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                    self.swarm_dashboard.add_tool_call(agent_id, &tool_name);
                } else {
                    self.chat_state.add_tool_call_to(&conv_id, &tool_name);
                }
            }
            Event::StreamingStatus { conv_id, status } => {
                if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                    self.swarm_dashboard.set_streaming_status(agent_id, &status);
                } else if let Some(idx) = self.chat_state.find_conv_idx(&conv_id) {
                    self.chat_state.conversations[idx].streaming_status = status;
                }
            }
            Event::MessageComplete { conv_id, role, content } => {
                if conv_id.is_empty() {
                    // Swarm/background task messages without a conversation —
                    // route to chat as a system message so they're visible.
                    self.chat_state.add_system_message(&content);
                    // Also show in the dashboard if a swarm run failed
                    if self.swarm_dashboard.phase == "failed" {
                        self.swarm_dashboard.status_message = content.clone();
                    }
                    tracing::warn!("Swarm message: {}", content);
                } else {
                    self.chat_state.finalize_message_to(&conv_id, &role, &content);
                    // Collapse <file> blocks in the assistant message for cleaner display
                    if role == "assistant" {
                        self.chat_state.collapse_file_blocks_in(&conv_id);
                    }
                }
            }

            // Deferred file proposal — swarm agents to dashboard, others to chat
            Event::FileProposalDeferred { conv_id, path, additions, deletions } => {
                tracing::debug!(
                    "Deferred proposal: {} (+{} -{})",
                    path.display(), additions, deletions
                );
                if let Some(agent_id) = conv_id.strip_prefix("swarm-") {
                    self.swarm_dashboard.add_file_change(
                        agent_id,
                        &path.to_string_lossy(),
                        additions,
                        deletions,
                    );
                } else {
                    self.chat_state.append_deferred_summary(&conv_id, &path, additions, deletions);
                }
            }

            // ACP task completed with deferred proposals — enter batch review
            Event::AcpTaskCompleted { conv_id, proposals } => {
                tracing::info!(
                    "ACP task completed for conv {} with {} proposals",
                    conv_id, proposals.len()
                );
                if proposals.is_empty() {
                    self.status_message = Some((
                        "Agent finished — no file changes".to_string(),
                        std::time::Instant::now(),
                    ));
                } else {
                    self.enter_batch_review(proposals);
                }
            }

            // Swarm events
            Event::SwarmPhaseChanged(phase) => {
                self.swarm_dashboard.set_phase(&phase);
                // Clear status message when transitioning to active phases
                if phase == "running" || phase == "merging" || phase == "verifying" {
                    self.swarm_dashboard.status_message.clear();
                } else if phase == "validating" {
                    self.swarm_dashboard.status_message = "Validating scopes and dependencies...".into();
                } else if phase == "failed" {
                    // Set a placeholder — the actual error arrives via MessageComplete right after
                    if self.swarm_dashboard.status_message.is_empty() {
                        self.swarm_dashboard.status_message = "Run failed — waiting for error details...".into();
                    }
                } else if phase == "reverted" {
                    self.swarm_dashboard.result = None; // prevent double-undo
                    self.swarm_dashboard.diff_agent = None;
                    self.swarm_dashboard.status_message = "Swarm reverted to pre-run state.".into();
                } else if phase.starts_with("revert failed") || phase.starts_with("revert panicked") {
                    self.swarm_dashboard.status_message = format!("Undo failed: {}", &phase["revert ".len()..]);
                }
            }
            Event::SwarmAgentStateChanged { id, status, detail } => {
                self.swarm_dashboard.update_agent(&id, &status, &detail);
            }
            Event::SwarmTierStarted { current, total } => {
                self.swarm_dashboard.set_tier(current, total);
            }
            Event::SwarmCompleted(result) => {
                self.swarm_dashboard.set_phase("completed");
                self.swarm_dashboard.status_message.clear();
                self.swarm_dashboard.set_result(*result);
            }
            Event::SwarmMergeConflict { branch, files } => {
                self.status_message = Some((
                    format!("Merge conflict in {}: {}", branch, files.join(", ")),
                    std::time::Instant::now(),
                ));
            }

            // Coordination lifecycle events
            Event::SwarmCoordinationStarted(_prompt) => {
                self.swarm_dashboard.set_phase("coordinating");
                self.swarm_dashboard.status_message = "Opus is decomposing the task...".into();
            }
            Event::SwarmCoordinationComplete { unit_count, summary: _ } => {
                self.swarm_dashboard.set_phase(&format!("planned ({} agents)", unit_count));
                self.swarm_dashboard.status_message = format!("Plan ready: {} agents, starting execution...", unit_count);
            }
            Event::SwarmTierDispatch { unit_id, tier, backend } => {
                self.swarm_dashboard.set_tier_dispatch(&unit_id, tier, &backend);
            }
            Event::SwarmCostUpdate(estimate) => {
                self.swarm_dashboard.set_cost(estimate.estimated_usd);
            }
            Event::SwarmDslPlanReady(plan_path) => {
                self.swarm_dashboard.set_phase("plan ready");
                let workspace_root = self.workspace.roots().first()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                let rel = plan_path
                    .strip_prefix(&workspace_root)
                    .unwrap_or(&plan_path)
                    .display()
                    .to_string();
                self.swarm_dashboard.status_message = format!("Plan saved: {} — review and /run it", rel);
                self.chat_state.add_system_message(&format!(
                    "Plan saved to `{}`.\nReview it (it's open in the editor), then run it with:\n  /run {}",
                    rel, rel,
                ));
                self.open_file(&plan_path);
                self.focus = Focus::Editor;
            }

            Event::MemoryReady(store) => {
                self.memory = Some(store);
                self.status_message = Some(("Memory ready".to_string(), std::time::Instant::now()));
            }

            Event::Terminal(term_event) => {
                if matches!(&term_event, gaviero_core::terminal::TerminalEvent::PtyOutput { .. }) {
                    self.terminal_selection.clear();
                }
                self.terminal_manager.process_event(term_event);
            }
            Event::Tick => {
                self.terminal_manager.tick();
                if self.chat_state.active_conv_streaming() {
                    self.chat_state.tick_count = self.chat_state.tick_count.wrapping_add(1);
                }
            }
        }
    }

    fn handle_action(&mut self, action: Action) {
        // If in diff review mode, route actions there first
        if self.diff_review.is_some() {
            if self.handle_review_action(&action) {
                return;
            }
        }

        // If in batch review mode, route actions there first
        if self.batch_review.is_some() {
            if self.handle_batch_review_action(&action) {
                return;
            }
        }

        // Terminal resize: Alt+Up/Down when terminal is focused
        if self.panel_visible.terminal {
            match action {
                Action::MoveLineUp if self.focus == Focus::Terminal => {
                    self.terminal_split_percent = (self.terminal_split_percent + theme::TERMINAL_RESIZE_STEP).min(theme::TERMINAL_MAX_PERCENT);
                    return;
                }
                Action::MoveLineDown if self.focus == Focus::Terminal => {
                    self.terminal_split_percent = self.terminal_split_percent.saturating_sub(theme::TERMINAL_RESIZE_STEP).max(theme::TERMINAL_MIN_PERCENT);
                    return;
                }
                _ => {}
            }
        }

        // Terminal scrollback: Shift+PageUp/PageDown when terminal is focused
        if self.focus == Focus::Terminal {
            match action {
                Action::PageUp => {
                    if let Some(inst) = self.terminal_manager.active_instance_mut() {
                        let current = inst.screen().scrollback();
                        let page = inst.screen().size().0 as usize; // rows
                        inst.screen_mut().set_scrollback(current + page);
                    }
                    return;
                }
                Action::PageDown => {
                    if let Some(inst) = self.terminal_manager.active_instance_mut() {
                        let current = inst.screen().scrollback();
                        let page = inst.screen().size().0 as usize;
                        inst.screen_mut().set_scrollback(current.saturating_sub(page));
                    }
                    return;
                }
                _ => {}
            }
        }

        // Terminal paste: Ctrl+V when terminal is focused — send internal clipboard with bracketed-paste wrappers
        if self.focus == Focus::Terminal {
            if let Action::Paste = action {
                let text = self.get_clipboard();
                if !text.is_empty() {
                    if let Some(inst) = self.terminal_manager.active_instance_mut() {
                        if inst.spawned {
                            // Bracketed paste: \x1b[200~ ... \x1b[201~
                            let mut payload = b"\x1b[200~".to_vec();
                            payload.extend_from_slice(text.as_bytes());
                            payload.extend_from_slice(b"\x1b[201~");
                            inst.write_input(&payload);
                        }
                    }
                }
                return;
            }
        }

        // Chat input area resize: Alt+Up/Down when side panel is focused
        if self.focus == Focus::SidePanel {
            match action {
                Action::MoveLineUp => {
                    // Grow input area
                    let current = self.chat_state.input_area_rows.max(3);
                    self.chat_state.input_area_rows = (current + 1).min(30);
                    return;
                }
                Action::MoveLineDown => {
                    // Shrink input area (0 = auto-size)
                    let current = self.chat_state.input_area_rows;
                    if current <= 3 {
                        self.chat_state.input_area_rows = 0; // back to auto
                    } else {
                        self.chat_state.input_area_rows = current - 1;
                    }
                    return;
                }
                _ => {}
            }
        }

        match action {
            Action::Quit => {
                // Dismiss swarm diff overlay first
                if self.focus == Focus::SidePanel
                    && matches!(self.side_panel, SidePanelMode::SwarmDashboard)
                    && self.swarm_dashboard.diff_agent.is_some()
                {
                    self.swarm_dashboard.close_diff();
                } else if self.focus == Focus::SidePanel
                    && matches!(self.side_panel, SidePanelMode::SwarmDashboard)
                    && self.swarm_dashboard.pending_undo_confirm
                {
                    self.swarm_dashboard.pending_undo_confirm = false;
                } else if self.diff_review.is_some() {
                    // q in review mode dismisses the overlay
                    self.diff_review = None;
                } else {
                    self.should_quit = true;
                }
            }
            Action::ToggleFileTree => {
                self.panel_visible.file_tree = !self.panel_visible.file_tree;
                if !self.panel_visible.file_tree && self.focus == Focus::FileTree {
                    self.focus = Focus::Editor;
                }
            }
            Action::ToggleSidePanel => {
                self.panel_visible.side_panel = !self.panel_visible.side_panel;
                if !self.panel_visible.side_panel && self.focus == Focus::SidePanel {
                    self.focus = Focus::Editor;
                }
            }
            Action::ToggleSwarmDashboard => {
                if !self.panel_visible.side_panel {
                    self.panel_visible.side_panel = true;
                }
                self.side_panel = SidePanelMode::SwarmDashboard;
                self.focus = Focus::SidePanel;
            }
            Action::SetSideModeChat => {
                self.panel_visible.side_panel = true;
                self.side_panel = SidePanelMode::AgentChat;
                self.focus = Focus::SidePanel;
            }
            Action::SetSideModeSwarm => {
                self.panel_visible.side_panel = true;
                self.side_panel = SidePanelMode::SwarmDashboard;
                self.focus = Focus::SidePanel;
            }
            Action::SetSideModeGit => {
                self.panel_visible.side_panel = true;
                self.side_panel = SidePanelMode::GitPanel;
                self.focus = Focus::SidePanel;
                self.refresh_git_panel();
            }
            Action::ToggleTerminal => {
                // Ctrl+J is bound to ToggleTerminal, but many terminals also send Ctrl+J
                // (LF = 0x0A) for Shift+Enter. Redirect to newline when the chat input has focus.
                if self.focus == Focus::SidePanel
                    && matches!(self.side_panel, SidePanelMode::AgentChat)
                {
                    if !self.chat_state.active_conv_streaming() {
                        self.chat_state.insert_char('\n');
                    }
                    return;
                }
                self.panel_visible.terminal = !self.panel_visible.terminal;
                if self.panel_visible.terminal {
                    self.spawn_active_terminal();
                    self.focus = Focus::Terminal;
                } else if self.focus == Focus::Terminal {
                    self.focus = Focus::Editor;
                }
            }
            Action::NewTerminal => {
                let root = self.workspace.roots().first()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                match self.terminal_manager.create_tab(&root) {
                    Ok(id) => {
                        self.terminal_manager.switch_tab(id);
                        self.panel_visible.terminal = true;
                        self.focus = Focus::Terminal;
                    }
                    Err(e) => {
                        self.status_message = Some((format!("Terminal: {}", e), std::time::Instant::now()));
                    }
                }
            }
            Action::CloseTerminal => {
                if self.focus == Focus::Terminal {
                    if let Some(id) = self.terminal_manager.active_tab() {
                        if self.terminal_manager.tab_count() > 1 {
                            self.terminal_manager.close_tab(id);
                        } else {
                            // Last terminal — close it and hide the panel
                            self.terminal_manager.close_tab(id);
                            self.panel_visible.terminal = false;
                            self.focus = Focus::Editor;
                        }
                    }
                }
            }
            Action::NewTab => {
                if self.focus == Focus::SidePanel {
                    self.chat_state.new_conversation();
                } else {
                    self.buffers.push(Buffer::empty());
                    self.active_buffer = self.buffers.len() - 1;
                    self.focus = Focus::Editor;
                }
            }
            Action::FocusLeftPanel => {
                if !self.panel_visible.file_tree {
                    self.panel_visible.file_tree = true;
                }
                self.focus = Focus::FileTree;
            }
            Action::FocusEditor => {
                self.focus = Focus::Editor;
            }
            Action::FocusSidePanel => {
                if !self.panel_visible.side_panel {
                    self.panel_visible.side_panel = true;
                }
                self.focus = Focus::SidePanel;
            }
            Action::FocusTerminal => {
                if !self.panel_visible.terminal {
                    self.panel_visible.terminal = true;
                    self.spawn_active_terminal();
                }
                self.focus = Focus::Terminal;
            }
            Action::CycleTabForward => {
                if self.focus == Focus::SidePanel {
                    self.chat_state.next_conversation();
                } else {
                    self.cycle_tab(1);
                }
            }
            Action::CycleTabBack => {
                if self.focus == Focus::SidePanel {
                    self.chat_state.prev_conversation();
                } else {
                    self.cycle_tab(-1);
                }
            }
            // Left panel mode switching (Alt+E/F/C)
            Action::SetLeftModeExplorer => {
                if !self.panel_visible.file_tree {
                    self.panel_visible.file_tree = true;
                }
                self.left_panel = LeftPanelMode::FileTree;
                self.focus = Focus::FileTree;
            }
            Action::SetLeftModeFind => {
                if !self.panel_visible.file_tree {
                    self.panel_visible.file_tree = true;
                }
                self.left_panel = LeftPanelMode::Search;
                self.focus = Focus::FileTree;
                self.search_panel.focus_input();
            }
            Action::SetLeftModeChanges => {
                if !self.panel_visible.file_tree {
                    self.panel_visible.file_tree = true;
                }
                self.left_panel = LeftPanelMode::Changes;
                self.focus = Focus::FileTree;
                self.refresh_git_changes();
            }
            Action::CloseTab => {
                if self.focus == Focus::Terminal {
                    self.handle_action(Action::CloseTerminal);
                } else {
                    self.close_tab();
                }
            }
            Action::Save => self.save_current_buffer(),
            Action::TogglePreview => {
                self.preview_visible = !self.preview_visible;
                self.preview_scroll = 0;
            }
            Action::ToggleFullscreen => self.toggle_fullscreen(),
            Action::SwitchLayout(n) => self.switch_layout(n),
            Action::FindInBuffer => {
                self.find_bar_active = true;
                self.find_input.select_all();
                self.focus = Focus::Editor;
                return;
            }
            Action::SearchInWorkspace => {
                // F3: if find bar is active, go to next match; otherwise workspace search
                if self.find_bar_active {
                    if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                        buf.find_next_match();
                    }
                    self.ensure_editor_cursor_visible();
                    return;
                }
                self.search_selected_in_workspace();
                return;
            }

            _ if self.focus == Focus::FileTree => {
                match self.left_panel {
                    LeftPanelMode::FileTree => self.handle_file_tree_action(action),
                    LeftPanelMode::Search => self.handle_search_action(action),
                    LeftPanelMode::Review => {} // handled by handle_batch_review_action above
                    LeftPanelMode::Changes => { self.handle_changes_action(&action); }
                }
            }
            _ if self.focus == Focus::Editor && self.find_bar_active => {
                self.handle_find_bar_action(action);
            }
            _ if self.focus == Focus::Editor => {
                // Block normal editing when in review mode
                if self.diff_review.is_none() {
                    self.handle_editor_action(action);
                }
            }
            _ if self.focus == Focus::SidePanel => match self.side_panel {
                SidePanelMode::AgentChat => self.handle_chat_action(action),
                SidePanelMode::GitPanel => self.handle_git_panel_action(action),
                SidePanelMode::SwarmDashboard => self.handle_swarm_dashboard_action(action)
            },
            _ => {}
        }
    }

    // ── Chat panel actions ──────────────────────────────────────

    fn handle_chat_action(&mut self, action: Action) {
        // Clear mouse text selection on any keyboard input
        self.chat_state.clear_text_selection();

        // Renaming mode: input edits the conversation title
        if self.chat_state.renaming {
            match action {
                Action::Enter => self.chat_state.confirm_rename(),
                Action::Quit => self.chat_state.cancel_rename(),
                Action::InsertChar(ch) => self.chat_state.insert_char(ch),
                Action::Backspace => self.chat_state.backspace(),
                Action::Delete => self.chat_state.text_input.delete(),
                Action::CursorLeft => self.chat_state.text_input.move_left(),
                Action::CursorRight => self.chat_state.text_input.move_right(),
                Action::Home => self.chat_state.text_input.move_home(),
                Action::End => self.chat_state.text_input.move_end(),
                _ => {}
            }
            return;
        }

        // Browse mode: navigate messages and copy
        if self.chat_state.browse_mode {
            match action {
                Action::CursorUp => self.chat_state.browse_up(),
                Action::CursorDown => self.chat_state.browse_down(),
                Action::Copy => {
                    // Copy the browsed message content to clipboard
                    if let Some(text) = self.chat_state.browsed_message_content() {
                        self.set_clipboard(&text);
                    }
                    self.chat_state.exit_browse_mode();
                }
                Action::Quit | Action::Enter => {
                    self.chat_state.exit_browse_mode();
                }
                // Any other key exits browse mode and is processed normally below
                _ => {
                    self.chat_state.exit_browse_mode();
                    // Fall through to normal handling
                    self.handle_chat_action(action);
                    return;
                }
            }
            return;
        }

        let ac_active = self.chat_state.autocomplete.active
            && !self.chat_state.autocomplete.matches.is_empty();

        match action {
            // F2: rename active conversation
            Action::Rename => {
                self.chat_state.start_rename();
                return;
            }

            // When autocomplete is showing, Tab/Enter accepts, Up/Down navigate
            Action::Tab if ac_active => {
                self.chat_state.accept_autocomplete();
                return;
            }
            Action::Enter if ac_active => {
                self.chat_state.accept_autocomplete();
                return;
            }
            Action::CursorUp if ac_active => {
                self.chat_state.autocomplete_up();
                return;
            }
            Action::CursorDown if ac_active => {
                self.chat_state.autocomplete_down();
                return;
            }
            Action::Quit if ac_active => {
                // Escape dismisses autocomplete first
                self.chat_state.autocomplete.reset();
                return;
            }

            Action::Enter => {
                if !self.chat_state.text_input.text.is_empty() && !self.chat_state.active_conv_streaming() {
                    // Block agent calls while in batch review mode
                    if self.batch_review.is_some() {
                        self.status_message = Some((
                            "Exit review first (f: apply, Esc: discard)".to_string(),
                            std::time::Instant::now(),
                        ));
                        return;
                    }

                    // Always scroll to bottom when user submits input
                    self.chat_state.scroll_pinned_to_bottom = true;

                    // Handle commands that need app-level access
                    if self.chat_state.text_input.text.trim().starts_with("/cswarm") {
                        self.handle_coordinated_swarm_command();
                    } else if self.chat_state.text_input.text.trim() == "/undo-swarm" {
                        self.handle_undo_swarm_command();
                    } else if self.chat_state.text_input.text.trim().starts_with("/swarm") {
                        self.handle_swarm_command();
                    } else if self.chat_state.text_input.text.trim().starts_with("/remember") {
                        self.handle_remember_command();
                    } else if self.chat_state.text_input.text.trim().starts_with("/attach") {
                        self.handle_attach_command();
                    } else if self.chat_state.text_input.text.trim().starts_with("/detach") {
                        self.handle_detach_command();
                    } else if self.chat_state.text_input.text.trim().starts_with("/run") {
                        self.handle_run_script_command();
                    } else if !self.chat_state.process_slash_command() {
                        self.send_chat_message();
                    }
                }
            }
            Action::AltEnter => {
                self.chat_state.insert_char('\n');
            }
            Action::InsertChar(ch) => {
                if !self.chat_state.active_conv_streaming() {
                    self.chat_state.insert_char(ch);
                    // Update autocomplete matches after typing
                    self.refresh_chat_autocomplete();
                }
            }
            Action::Backspace => {
                self.chat_state.backspace();
                self.refresh_chat_autocomplete();
            }
            Action::Delete => self.chat_state.text_input.delete(),
            Action::Undo => self.chat_state.text_input.undo(),
            Action::Redo => self.chat_state.text_input.redo(),
            Action::SelectAll => self.chat_state.text_input.select_all(),
            Action::DeleteWordBack => {
                self.chat_state.delete_word_back();
                self.refresh_chat_autocomplete();
            }
            Action::CursorLeft => self.chat_state.text_input.move_left(),
            Action::CursorRight => self.chat_state.text_input.move_right(),
            Action::WordLeft => self.chat_state.text_input.move_word_left(),
            Action::WordRight => self.chat_state.text_input.move_word_right(),
            Action::SelectLeft => self.chat_state.text_input.select_left(),
            Action::SelectRight => self.chat_state.text_input.select_right(),
            Action::SelectWordLeft => self.chat_state.text_input.select_word_left(),
            Action::SelectWordRight => self.chat_state.text_input.select_word_right(),
            Action::Home => self.chat_state.text_input.move_home(),
            Action::End => self.chat_state.text_input.move_end(),
            Action::CursorUp => {
                let streaming = self.chat_state.active_conv_streaming();
                if streaming {
                    // During streaming, Up/Down always scroll the chat history
                    self.chat_state.scroll_offset = self.chat_state.scroll_offset.saturating_sub(1);
                    self.chat_state.user_scrolled_during_stream = true;
                } else {
                    // Compute the visual line widths for the input area
                    let prompt_len = 2; // "> "
                    let panel_w = self.layout.side_panel_area.map(|a| a.width).unwrap_or(40).saturating_sub(2) as usize; // -2 for borders
                    let first_w = panel_w.saturating_sub(prompt_len);
                    let has_visual_lines = !self.chat_state.text_input.text.is_empty()
                        && (self.chat_state.input_is_multiline()
                            || self.chat_state.input_wraps_visually(first_w, panel_w));

                    if has_visual_lines {
                        if !self.chat_state.move_up_visual(first_w, panel_w) {
                            // Reached top of visual text — same logic as single-line case
                            if self.chat_state.history_index.is_some() || self.chat_state.text_input.text.is_empty() {
                                self.chat_state.history_up();
                            } else {
                                self.chat_state.scroll_offset = self.chat_state.scroll_offset.saturating_sub(1);
                            }
                        }
                    } else if self.chat_state.history_index.is_some() || self.chat_state.text_input.text.is_empty() {
                        self.chat_state.history_up();
                    } else {
                        self.chat_state.scroll_offset = self.chat_state.scroll_offset.saturating_sub(1);
                    }
                }
            }
            Action::CursorDown => {
                let streaming = self.chat_state.active_conv_streaming();
                if streaming {
                    self.chat_state.scroll_offset += 1;
                    // Don't mark user_scrolled — scrolling down towards bottom
                    // is consistent with wanting to follow output
                } else {
                    let prompt_len = 2;
                    let panel_w = self.layout.side_panel_area.map(|a| a.width).unwrap_or(40).saturating_sub(2) as usize;
                    let first_w = panel_w.saturating_sub(prompt_len);
                    let has_visual_lines = !self.chat_state.text_input.text.is_empty()
                        && (self.chat_state.input_is_multiline()
                            || self.chat_state.input_wraps_visually(first_w, panel_w));

                    if has_visual_lines {
                        if !self.chat_state.move_down_visual(first_w, panel_w) {
                            // Reached bottom of visual text — same logic as single-line case
                            if self.chat_state.history_index.is_some() {
                                self.chat_state.history_down();
                            } else {
                                self.chat_state.scroll_offset += 1;
                            }
                        }
                    } else if self.chat_state.history_index.is_some() {
                        self.chat_state.history_down();
                    } else {
                        self.chat_state.scroll_offset += 1;
                    }
                }
            }
            Action::PageUp => {
                self.chat_state.scroll_offset = self.chat_state.scroll_offset.saturating_sub(20);
                if self.chat_state.active_conv_streaming() {
                    self.chat_state.user_scrolled_during_stream = true;
                }
            }
            Action::PageDown => {
                self.chat_state.scroll_offset = self.chat_state.scroll_offset.saturating_add(20);
            }
            Action::Quit => {
                if !self.chat_state.text_input.text.is_empty() {
                    self.chat_state.text_input.clear();
                    self.chat_state.autocomplete.reset();
                } else {
                    self.focus = Focus::Editor;
                }
            }
            Action::Paste => {
                if !self.chat_state.active_conv_streaming() {
                    self.chat_paste_from_clipboard();
                }
            }
            Action::Copy => {
                if self.chat_state.active_conv_streaming() {
                    self.cancel_agent();
                } else {
                    // Enter browse mode to select and copy messages
                    self.chat_state.enter_browse_mode();
                }
            }
            _ => {}
        }
    }

    // ── Git panel actions ────────────────────────────────────

    fn handle_swarm_dashboard_action(&mut self, action: Action) {
        use crate::panels::swarm_dashboard::DashboardFocus;

        // ── Actions that need access to multiple self fields ──────────────
        // These are handled before the `dash` reborrow to avoid borrow conflicts.

        match action {
            Action::Enter => {
                if self.swarm_dashboard.diff_agent.is_some() {
                    self.swarm_dashboard.close_diff();
                    return;
                }
                let agent = self.swarm_dashboard.agents
                    .get(self.swarm_dashboard.scroll.selected);
                let branch = agent.and_then(|a| a.branch.clone());
                let agent_id = agent.map(|a| a.id.clone());
                let is_completed = agent.map(|a| matches!(a.status, gaviero_core::swarm::models::AgentStatus::Completed)).unwrap_or(false);

                if !is_completed {
                    return;
                }
                match (branch, agent_id) {
                    (Some(branch), Some(agent_id)) => {
                        let root = self.workspace.roots().first()
                            .map(|p| p.to_path_buf())
                            .unwrap_or_else(|| std::path::PathBuf::from("."));
                        let pre_sha = self.swarm_dashboard.result.as_ref()
                            .map(|r| r.pre_swarm_sha.clone())
                            .unwrap_or_default();
                        let diff_text = gaviero_core::git::diff_branch_vs_sha(&root, &pre_sha, &branch)
                            .unwrap_or_default();
                        self.swarm_dashboard.show_diff(agent_id, diff_text);
                    }
                    (_, Some(id)) => {
                        self.swarm_dashboard.status_message = format!(
                            "No diff available for '{}' (agent ran without worktree isolation)", id
                        );
                    }
                    _ => {}
                }
                return;
            }
            Action::InsertChar('u') => {
                let can_undo = self.swarm_dashboard.result.as_ref()
                    .map(|r| !r.pre_swarm_sha.is_empty())
                    .unwrap_or(false);
                if !can_undo { return; }

                if self.swarm_dashboard.pending_undo_confirm {
                    self.swarm_dashboard.pending_undo_confirm = false;
                    let result = match self.swarm_dashboard.result.clone() {
                        Some(r) => r,
                        None => return,
                    };
                    let root = self.workspace.roots().first()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                    let tx = self.event_tx.clone();
                    tokio::task::spawn(async move {
                        let revert_result = tokio::task::spawn_blocking(move || {
                            gaviero_core::swarm::pipeline::revert_swarm(&root, &result)
                        }).await;
                        match revert_result {
                            Ok(Ok(())) => { let _ = tx.send(Event::SwarmPhaseChanged("reverted".to_string())); }
                            Ok(Err(e)) => { let _ = tx.send(Event::SwarmPhaseChanged(format!("revert failed: {}", e))); }
                            Err(e) => { let _ = tx.send(Event::SwarmPhaseChanged(format!("revert panicked: {}", e))); }
                        }
                    });
                } else {
                    self.swarm_dashboard.pending_undo_confirm = true;
                }
                return;
            }
            _ => {}
        }

        // ── Remaining actions using `dash` shorthand ───────────────────────

        let dash = &mut self.swarm_dashboard;
        let agent_count = dash.agents.len();

        // Helper: reset detail pane when table selection changes
        macro_rules! reset_detail {
            ($dash:expr) => {
                $dash.detail_scroll = 0;
                $dash.detail_auto_scroll = true;
            };
        }

        match action {
            // Tab toggles focus between Table and Detail
            Action::InsertChar('\t') => {
                dash.cycle_focus();
            }

            // Up/Down are focus-aware; when diff overlay is open, scroll the diff
            Action::CursorUp | Action::InsertChar('k') => {
                if let Some(ref mut diff) = dash.diff_agent {
                    diff.scroll = diff.scroll.saturating_sub(1);
                } else {
                    match dash.focus {
                        DashboardFocus::Table => {
                            let prev = dash.scroll.selected;
                            dash.scroll.move_up();
                            if dash.scroll.selected != prev { reset_detail!(dash); }
                        }
                        DashboardFocus::Detail => {
                            dash.detail_auto_scroll = false;
                            dash.detail_scroll = dash.detail_scroll.saturating_sub(1);
                        }
                    }
                }
            }
            Action::CursorDown | Action::InsertChar('j') => {
                if let Some(ref mut diff) = dash.diff_agent {
                    diff.scroll = diff.scroll.saturating_add(1);
                } else {
                    match dash.focus {
                        DashboardFocus::Table => {
                            let prev = dash.scroll.selected;
                            dash.scroll.move_down(agent_count);
                            if dash.scroll.selected != prev { reset_detail!(dash); }
                        }
                        DashboardFocus::Detail => {
                            if let Some(agent) = dash.agents.get(dash.scroll.selected) {
                                let w = dash.detail_rect.width.saturating_sub(1) as usize;
                                let total = crate::panels::swarm_dashboard::count_display_lines(&agent.activity, w);
                                dash.detail_scroll = (dash.detail_scroll + 1).min(total.saturating_sub(1));
                            }
                        }
                    }
                }
            }

            // PageUp/PageDown — always scroll the focused pane by page
            Action::PageUp => match dash.focus {
                DashboardFocus::Table => {
                    dash.scroll.selected = dash.scroll.selected.saturating_sub(10);
                    dash.scroll.ensure_visible();
                    reset_detail!(dash);
                }
                DashboardFocus::Detail => {
                    dash.detail_auto_scroll = false;
                    dash.detail_scroll = dash.detail_scroll.saturating_sub(10);
                }
            },
            Action::PageDown => match dash.focus {
                DashboardFocus::Table => {
                    dash.scroll.selected = (dash.scroll.selected + 10).min(agent_count.saturating_sub(1));
                    dash.scroll.ensure_visible();
                    reset_detail!(dash);
                }
                DashboardFocus::Detail => {
                    if let Some(agent) = dash.agents.get(dash.scroll.selected) {
                        let w = dash.detail_rect.width.saturating_sub(1) as usize;
                        let total = crate::panels::swarm_dashboard::count_display_lines(&agent.activity, w);
                        dash.detail_scroll = (dash.detail_scroll + 10).min(total.saturating_sub(1));
                    }
                }
            },

            // Home/End
            Action::Home => {
                match dash.focus {
                    DashboardFocus::Table => {
                        dash.scroll.reset();
                    }
                    DashboardFocus::Detail => {
                        dash.detail_scroll = 0;
                        dash.detail_auto_scroll = false;
                    }
                }
            }
            Action::End => {
                match dash.focus {
                    DashboardFocus::Table => {
                        dash.scroll.selected = agent_count.saturating_sub(1);
                        dash.scroll.ensure_visible();
                    }
                    DashboardFocus::Detail => {
                        dash.detail_auto_scroll = true;
                    }
                }
            }

            // Toggle auto-scroll (detail pane)
            Action::InsertChar('f') => {
                dash.detail_auto_scroll = !dash.detail_auto_scroll;
            }
            _ => {}
        }
    }

    fn handle_git_panel_action(&mut self, action: Action) {
        use crate::panels::git_panel::GitRegion;

        // Branch picker intercepts all input when open
        if self.git_panel.branch_picker_open {
            match action {
                Action::CursorUp | Action::InsertChar('k') => self.git_panel.branch_picker_up(),
                Action::CursorDown | Action::InsertChar('j') => self.git_panel.branch_picker_down(),
                Action::Enter => {
                    if let Some(name) = self.git_panel.selected_branch_name() {
                        if let Some(repo) = &self.git_repo {
                            if let Err(e) = repo.checkout(&name) {
                                self.git_panel.error_message = Some(format!("{}", e));
                            }
                            self.git_panel.refresh(repo);
                        }
                    }
                    self.git_panel.close_branch_picker();
                }
                Action::Quit => self.git_panel.close_branch_picker(),
                Action::Backspace => self.git_panel.branch_picker_backspace(),
                Action::InsertChar(ch) => self.git_panel.branch_picker_insert(ch),
                _ => {}
            }
            return;
        }

        match action {
            Action::CursorUp | Action::InsertChar('k') => self.git_panel.move_up(),
            Action::CursorDown | Action::InsertChar('j') => self.git_panel.move_down(),
            Action::Tab => self.git_panel.cycle_region(),

            Action::InsertChar('s') if self.git_panel.region != GitRegion::CommitInput => {
                if let Some(path) = self.git_panel.selected_path().map(|s| s.to_string()) {
                    if let Some(repo) = &self.git_repo {
                        if let Err(e) = repo.stage_file(&path) {
                            self.git_panel.error_message = Some(format!("{}", e));
                        }
                        self.git_panel.refresh(repo);
                    }
                }
            }
            Action::InsertChar('u') if self.git_panel.region != GitRegion::CommitInput => {
                if let Some(path) = self.git_panel.selected_path().map(|s| s.to_string()) {
                    if let Some(repo) = &self.git_repo {
                        if let Err(e) = repo.unstage_file(&path) {
                            self.git_panel.error_message = Some(format!("{}", e));
                        }
                        self.git_panel.refresh(repo);
                    }
                }
            }
            Action::InsertChar('d') if self.git_panel.region != GitRegion::CommitInput => {
                if let Some(path) = self.git_panel.selected_path().map(|s| s.to_string()) {
                    if let Some(repo) = &self.git_repo {
                        if let Err(e) = repo.discard_changes(&path) {
                            self.git_panel.error_message = Some(format!("{}", e));
                        }
                        self.git_panel.refresh(repo);
                        self.refresh_file_tree();
                    }
                }
            }
            Action::InsertChar('c') if self.git_panel.region != GitRegion::CommitInput => {
                // Commit staged files
                if !self.git_panel.commit_input.is_empty() {
                    if let Some(repo) = &self.git_repo {
                        match repo.commit(&self.git_panel.commit_input.text) {
                            Ok(_) => {
                                self.git_panel.commit_input.clear();
                                self.git_panel.refresh(repo);
                                self.refresh_file_tree();
                            }
                            Err(e) => {
                                self.git_panel.error_message = Some(format!("{}", e));
                            }
                        }
                    }
                } else {
                    self.git_panel.region = GitRegion::CommitInput;
                }
            }
            Action::InsertChar('a') if self.git_panel.region != GitRegion::CommitInput => {
                // Amend last commit
                if !self.git_panel.commit_input.is_empty() {
                    if let Some(repo) = &self.git_repo {
                        match repo.amend(&self.git_panel.commit_input.text) {
                            Ok(_) => {
                                self.git_panel.commit_input.clear();
                                self.git_panel.refresh(repo);
                            }
                            Err(e) => {
                                self.git_panel.error_message = Some(format!("{}", e));
                            }
                        }
                    }
                }
            }

            // In CommitInput region, type commit message
            Action::InsertChar(ch) if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.insert_char(ch);
            }
            Action::Backspace if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.backspace();
            }
            Action::CursorLeft if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.move_left();
            }
            Action::CursorRight if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.move_right();
            }
            Action::WordLeft if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.move_word_left();
            }
            Action::WordRight if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.move_word_right();
            }
            Action::SelectLeft if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.select_left();
            }
            Action::SelectRight if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.select_right();
            }
            Action::SelectWordLeft if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.select_word_left();
            }
            Action::SelectWordRight if self.git_panel.region == GitRegion::CommitInput => {
                self.git_panel.commit_input.select_word_right();
            }
            Action::Enter if self.git_panel.region == GitRegion::CommitInput => {
                // Enter in commit input → commit
                if !self.git_panel.commit_input.is_empty() {
                    if let Some(repo) = &self.git_repo {
                        match repo.commit(&self.git_panel.commit_input.text) {
                            Ok(_) => {
                                self.git_panel.commit_input.clear();
                                self.git_panel.refresh(repo);
                                self.refresh_file_tree();
                            }
                            Err(e) => {
                                self.git_panel.error_message = Some(format!("{}", e));
                            }
                        }
                    }
                }
            }

            // Enter on a file (not in commit input) → show diff in editor
            Action::Enter => {
                if let Some(rel_path) = self.git_panel.selected_path().map(|s| s.to_string()) {
                    let root = self.workspace.roots().first().map(|p| p.to_path_buf());
                    if let Some(root) = root {
                        let abs_path = root.join(&rel_path);
                        if abs_path.exists() {
                            // Read HEAD version for diff
                            let staged = self.git_panel.region == GitRegion::Staged;
                            let original = self.git_head_content(&rel_path).unwrap_or_default();
                            let current = std::fs::read_to_string(&abs_path).unwrap_or_default();

                            if original != current {
                                let proposal = WriteGatePipeline::build_proposal(
                                    0, "git-diff", &abs_path, &original, &current,
                                );
                                self.open_file(&abs_path);
                                self.focus = Focus::Editor;
                                let source = if staged { DiffSource::Acp } else { DiffSource::Acp };
                                self.diff_review = Some(DiffReviewState::new(proposal, source));
                            } else {
                                self.open_file(&abs_path);
                                self.focus = Focus::Editor;
                            }
                        }
                    }
                }
            }

            // Branch picker
            Action::InsertChar('b') if self.git_panel.region != GitRegion::CommitInput => {
                self.git_panel.toggle_branch_picker();
            }

            Action::Quit => {
                if self.git_panel.branch_picker_open {
                    self.git_panel.close_branch_picker();
                } else if self.git_panel.region == GitRegion::CommitInput && !self.git_panel.commit_input.is_empty() {
                    self.git_panel.commit_input.clear();
                } else {
                    self.focus = Focus::Editor;
                }
            }

            _ => {}
        }
    }

    /// Read the HEAD version of a file (for diff display).
    fn git_head_content(&self, rel_path: &str) -> Option<String> {
        let repo = self.git_repo.as_ref()?;
        let head = repo.head_file_content(rel_path).ok()?;
        Some(head)
    }

    /// Refresh the git panel from the repository if it's visible.
    fn refresh_git_panel(&mut self) {
        if let Some(repo) = &self.git_repo {
            self.git_panel.refresh(repo);
        }
    }

    /// Collect workspace file paths and update autocomplete matches.
    fn refresh_chat_autocomplete(&mut self) {
        if !self.chat_state.autocomplete.active {
            return;
        }
        let root = self.workspace.roots().first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        // Detect whether we are completing the path argument of a /run command.
        // The text before the '@' (trimmed) will equal "/run" in that case.
        let at_pos = self.chat_state.autocomplete.at_pos;
        let is_run_path_context = {
            let text = &self.chat_state.text_input.text;
            at_pos <= text.len() && text[..at_pos].trim() == "/run"
        };

        // Collect file paths from the file tree entries (already loaded).
        // In run-path context, restrict to .gaviero files only.
        let files: Vec<String> = self.file_tree.entries.iter()
            .filter(|e| !e.is_dir)
            .filter_map(|e| {
                e.path.strip_prefix(&root).ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .filter(|f| !is_run_path_context || f.ends_with(".gaviero"))
            .collect();

        self.chat_state.update_autocomplete_matches(&files);
    }

    fn send_chat_message(&mut self) {
        let conv_id = self.chat_state.conversations[self.chat_state.active_conv].id.clone();
        let prompt = self.chat_state.take_input();
        self.chat_state.add_user_message(&prompt);
        self.chat_state.conversations[self.chat_state.active_conv].is_streaming = true;
        self.chat_state.conversations[self.chat_state.active_conv].streaming_status = "Connecting...".to_string();
        self.chat_state.conversations[self.chat_state.active_conv].streaming_started_at = Some(std::time::Instant::now());

        let tx = self.event_tx.clone();
        let wg = self.write_gate.clone();
        let root = self.workspace.roots().first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        // Resolve @file references: read file contents
        let refs = crate::panels::agent_chat::parse_file_references(&prompt);
        let mut file_refs: Vec<(String, String)> = Vec::new();
        for rel_path in &refs {
            let abs_path = root.join(rel_path);
            if let Ok(content) = std::fs::read_to_string(&abs_path) {
                file_refs.push((rel_path.clone(), content));
            }
        }

        // Take attachments and split by kind:
        // - Text attachments: read contents and add to file_refs
        // - Image attachments: pass as file paths to the CLI
        let attachments = self.chat_state.take_attachments();
        let mut cli_file_attachments: Vec<std::path::PathBuf> = Vec::new();
        for attach in &attachments {
            match attach.kind {
                crate::panels::agent_chat::AttachmentKind::Text => {
                    if let Ok(content) = std::fs::read_to_string(&attach.path) {
                        file_refs.push((attach.display_name.clone(), content));
                    }
                }
                crate::panels::agent_chat::AttachmentKind::Image => {
                    cli_file_attachments.push(attach.path.clone());
                }
            }
        }

        // Collect conversation history for multi-turn context
        let context: Vec<(String, String)> = self.chat_state.context_messages()
            .into_iter()
            .rev()
            .skip(1) // skip the just-added user message
            .rev()
            .map(|(r, c)| (r.to_string(), c.to_string()))
            .collect();

        // Read effective model and options for this conversation
        let model = self.chat_state.effective_model().to_string();
        let effort = self.chat_state.effective_effort().to_string();
        let max_tokens = self.chat_state.agent_settings.max_tokens;

        let options = gaviero_core::acp::session::AgentOptions {
            effort,
            max_tokens,
        };

        let memory = self.memory.clone();
        let read_ns = self.chat_state.agent_settings.read_namespaces.clone();

        let conv_id_clone = conv_id.clone();
        let task = tokio::spawn(async move {
            // Switch write gate to Deferred mode so file proposals are collected, not applied
            {
                let mut gate = wg.lock().await;
                tracing::info!("Write gate mode before: {:?}, switching to Deferred", gate.mode());
                gate.set_mode(WriteMode::Deferred);
            }

            // Enrich prompt with memory context (if memory is available).
            // Cap at 5s to avoid delaying the prompt if the memory server is slow.
            let enriched_prompt = if let Some(ref mem) = memory {
                let ctx = match tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    mem.search_context(&read_ns, &prompt, 5),
                ).await {
                    Ok(ctx) => ctx,
                    Err(_) => {
                        tracing::warn!("Memory search timed out after 5s, proceeding without context");
                        String::new()
                    }
                };
                if ctx.is_empty() { prompt.clone() } else { format!("{}\n\n{}", ctx, prompt) }
            } else {
                prompt.clone()
            };

            let observer = TuiAcpObserver {
                tx: tx.clone(),
                conv_id: conv_id_clone.clone(),
            };
            let pipeline = AcpPipeline::new(wg.clone(), Box::new(observer), model, root, "claude-chat", options);
            if let Err(e) = pipeline.send_prompt(&enriched_prompt, &file_refs, &context, &cli_file_attachments).await {
                tracing::error!("send_prompt error: {}", e);
                let _ = tx.send(Event::MessageComplete {
                    conv_id: conv_id.clone(),
                    role: "system".to_string(),
                    content: format!("Error: {}", e),
                });
            }

            // Drain deferred proposals and fire AcpTaskCompleted if any exist
            let proposals = {
                let mut gate = wg.lock().await;
                let proposals = gate.take_pending_proposals();
                tracing::info!(
                    "Draining deferred proposals: count={}, switching back to Interactive",
                    proposals.len()
                );
                gate.set_mode(WriteMode::Interactive);
                proposals
            };
            if !proposals.is_empty() {
                tracing::info!("Sending AcpTaskCompleted with {} proposals", proposals.len());
                let _ = tx.send(Event::AcpTaskCompleted {
                    conv_id: conv_id_clone,
                    proposals,
                });
            } else {
                tracing::info!("No deferred proposals — skipping AcpTaskCompleted");
            }
        });
        self.acp_tasks.insert(
            self.chat_state.conversations[self.chat_state.active_conv].id.clone(),
            task,
        );
    }

    /// Handle `/swarm <task>` command — plan + execute a multi-agent task.
    fn handle_swarm_command(&mut self) {
        let input = self.chat_state.take_input();
        let task_desc = input
            .trim()
            .strip_prefix("/swarm")
            .unwrap_or("")
            .trim()
            .to_string();

        if task_desc.is_empty() {
            self.chat_state.add_system_message(
                "Usage: /swarm <task description>\n\
                 Plans and executes a multi-agent task with git worktree isolation.\n\
                 Example: /swarm Refactor the auth module and update all tests",
            );
            return;
        }

        self.chat_state.add_user_message(&format!("/swarm {}", task_desc));
        self.chat_state.add_system_message(&format!(
            "Planning swarm task: {}\nSwitch to SWARM panel (Ctrl+Shift+P) to monitor progress.",
            task_desc
        ));

        // Switch to swarm dashboard, reset for new run
        self.side_panel = SidePanelMode::SwarmDashboard;
        self.panel_visible.side_panel = true;
        self.swarm_dashboard.reset("planning");
        self.swarm_dashboard.status_message = format!("Planning task: {}...", task_desc.chars().take(60).collect::<String>());

        let tx = self.event_tx.clone();
        let root = self.workspace.roots().first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let model = self.chat_state.effective_model().to_string();
        let write_ns = self.chat_state.agent_settings.write_namespace.clone();
        let read_ns = self.chat_state.agent_settings.read_namespaces.clone();
        let memory = self.memory.clone();

        tokio::spawn(async move {
            use gaviero_core::swarm::{pipeline, planner};

            // Search memory for planning context
            let memory_ctx = if let Some(ref mem) = memory {
                mem.search_context(&read_ns, &task_desc, 5).await
            } else {
                String::new()
            };

            // Step 1: Plan the task
            let file_list = list_workspace_files(&root, 200);
            let work_units = match planner::plan_task(
                &task_desc,
                &root,
                &model,
                &file_list,
                &memory_ctx,
            ).await {
                Ok(units) => units,
                Err(e) => {
                    let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                    let _ = tx.send(Event::MessageComplete {
                        conv_id: String::new(),
                        role: "system".to_string(),
                        content: format!("Swarm planning failed: {}", e),
                    });
                    return;
                }
            };

            let unit_count = work_units.len();
            let _ = tx.send(Event::SwarmPhaseChanged(format!("planned ({} agents)", unit_count)));

            // Step 2: Execute
            let plan = gaviero_core::swarm::plan::CompiledPlan::from_work_units(work_units, None);
            let config = pipeline::SwarmConfig {
                max_parallel: unit_count.min(4), // cap at 4 parallel agents
                workspace_root: root,
                model: model.clone(),
                use_worktrees: unit_count > 1,
                read_namespaces: read_ns,
                write_namespace: write_ns,
                context_files: vec![],
            };

            let observer = TuiSwarmObserver { tx: tx.clone() };
            let tx2 = tx.clone();
            let make_obs = move |agent_id: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
                Box::new(TuiAcpObserver {
                    tx: tx2.clone(),
                    conv_id: format!("swarm-{}", agent_id),
                })
            };

            match pipeline::execute(&plan, &config, None, memory, &observer, make_obs).await {
                Ok(result) => {
                    let _ = tx.send(Event::SwarmCompleted(Box::new(result)));
                }
                Err(e) => {
                    let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                    let _ = tx.send(Event::MessageComplete {
                        conv_id: String::new(),
                        role: "system".to_string(),
                        content: format!("Swarm execution failed: {}", e),
                    });
                }
            }
        });
    }

    /// Handle `/run <path.gaviero> [prompt]` — compile and execute a DSL script.
    fn handle_run_script_command(&mut self) {
        let input = self.chat_state.take_input();
        let rest = input.trim().strip_prefix("/run").unwrap_or("").trim();

        // Split: first whitespace-delimited token is the path, rest is the runtime prompt.
        let (raw_path_token, runtime_prompt) = match rest.find(|c: char| c.is_ascii_whitespace()) {
            Some(idx) => {
                let path_tok = &rest[..idx];
                let remainder = rest[idx..].trim();
                let rp = if remainder.is_empty() { None } else { Some(remainder.to_string()) };
                (path_tok, rp)
            }
            None => (rest, None),
        };

        // Strip optional '@' prefix (used by autocomplete).
        let script_path = raw_path_token.strip_prefix('@').unwrap_or(raw_path_token).to_string();

        if script_path.is_empty() {
            self.chat_state.add_system_message(
                "Usage: /run <path.gaviero> [prompt]\n\
                 Compiles and executes a .gaviero DSL script.\n\
                 Use {{PROMPT}} in agent prompts for runtime substitution.\n\
                 Example: /run workflows/security_audit.gaviero\n\
                 Example: /run @workflows/tdd.gaviero implement OAuth login",
            );
            return;
        }

        // Resolve path relative to workspace root
        let root = self.workspace.roots().first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let resolved = if std::path::Path::new(&script_path).is_absolute() {
            std::path::PathBuf::from(&script_path)
        } else {
            root.join(&script_path)
        };

        // Read the script file
        let raw = match std::fs::read_to_string(&resolved) {
            Ok(s) => s,
            Err(e) => {
                self.chat_state.add_system_message(
                    &format!("Cannot read {}: {}", resolved.display(), e),
                );
                return;
            }
        };

        // If the file is markdown-wrapped (LLM output with ```gaviero fences),
        // extract just the DSL block so the lexer doesn't choke on prose.
        let source = extract_gaviero_block(&raw);

        // Compile synchronously (fast, no LLM call)
        let filename = resolved.display().to_string();
        let compiled = match gaviero_dsl::compile(&source, &filename, None, runtime_prompt.as_deref()) {
            Ok(c) => c,
            Err(report) => {
                self.chat_state.add_system_message(
                    &format!("DSL compilation failed:\n{}", report),
                );
                return;
            }
        };

        let unit_count = compiled.graph.node_count();
        let display_cmd = match &runtime_prompt {
            Some(rp) => format!("/run {} {}", script_path, rp),
            None => format!("/run {}", script_path),
        };
        self.chat_state.add_user_message(&display_cmd);
        self.chat_state.add_system_message(&format!(
            "Compiled {} → {} agent(s). Executing...\n\
             Switch to SWARM panel (Ctrl+Shift+P) to monitor progress.",
            script_path, unit_count
        ));

        // Switch to swarm dashboard
        self.side_panel = SidePanelMode::SwarmDashboard;
        self.panel_visible.side_panel = true;
        self.swarm_dashboard.reset("compiled");
        self.swarm_dashboard.status_message = format!(
            "Script: {} ({} agents)",
            script_path, unit_count
        );

        let tx = self.event_tx.clone();
        let model = self.chat_state.effective_model().to_string();
        let write_ns = self.chat_state.agent_settings.write_namespace.clone();
        let read_ns = self.chat_state.agent_settings.read_namespaces.clone();
        let memory = self.memory.clone();

        tokio::spawn(async move {
            use gaviero_core::swarm::pipeline;

            // Script's max_parallel overrides the TUI default when declared.
            let effective_max_parallel = compiled.max_parallel
                .unwrap_or_else(|| unit_count.min(4));

            let config = pipeline::SwarmConfig {
                max_parallel: effective_max_parallel,
                workspace_root: root,
                model,
                use_worktrees: effective_max_parallel > 1,
                read_namespaces: read_ns,
                write_namespace: write_ns,
                context_files: vec![],
            };

            let observer = TuiSwarmObserver { tx: tx.clone() };
            let tx2 = tx.clone();
            let make_obs = move |agent_id: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
                Box::new(TuiAcpObserver {
                    tx: tx2.clone(),
                    conv_id: format!("swarm-{}", agent_id),
                })
            };

            match pipeline::execute(&compiled, &config, None, memory, &observer, make_obs).await {
                Ok(result) => {
                    let _ = tx.send(Event::SwarmCompleted(Box::new(result)));
                }
                Err(e) => {
                    let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                    let _ = tx.send(Event::MessageComplete {
                        conv_id: String::new(),
                        role: "system".to_string(),
                        content: format!("Script execution failed: {}", e),
                    });
                }
            }
        });
    }

    /// Handle `/cswarm <task>` — coordinated tier-routed swarm (Opus → Sonnet/Haiku).
    fn handle_coordinated_swarm_command(&mut self) {
        let input = self.chat_state.take_input();
        let task_desc = input
            .trim()
            .strip_prefix("/cswarm")
            .unwrap_or("")
            .trim()
            .to_string();

        if task_desc.is_empty() {
            self.chat_state.add_system_message(
                "Usage: /cswarm <task description>\n\
                 Coordinated tier-routed swarm: Opus plans, Sonnet/Haiku execute.\n\
                 Example: /cswarm Refactor the auth module to use the strategy pattern",
            );
            return;
        }

        self.chat_state.add_user_message(&format!("/cswarm {}", task_desc));
        self.chat_state.add_system_message(&format!(
            "Coordinated swarm: {}\nOpus will produce a .gaviero plan file for review.\n\
             Switch to SWARM panel (Ctrl+Shift+P) to monitor.",
            task_desc
        ));

        self.side_panel = SidePanelMode::SwarmDashboard;
        self.panel_visible.side_panel = true;
        self.swarm_dashboard.reset("coordinating");
        self.swarm_dashboard.status_message = format!("Opus planning (DSL): {}...", task_desc.chars().take(60).collect::<String>());

        let tx = self.event_tx.clone();
        let root = self.workspace.roots().first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        // Inline @file references so the coordinator receives file contents directly.
        // Subagents run in isolated git worktrees and cannot access files outside of
        // tracked paths (e.g. tmp/ dirs). Inlining ensures the coordinator can embed
        // relevant content into coordinator_instructions without requiring file access.
        // Also collect context_files so the same content can be physically injected into
        // each worktree, allowing agents to use the Read tool on them directly.
        let (task_desc, context_files) = {
            use crate::panels::agent_chat::parse_file_references;
            let refs = parse_file_references(&task_desc);
            let mut enriched = task_desc.clone();
            let mut ctx_files: Vec<(String, String)> = Vec::new();
            for rel_path in &refs {
                let abs_path = root.join(rel_path);
                if let Ok(content) = std::fs::read_to_string(&abs_path) {
                    let tag = format!("@{}", rel_path);
                    let replacement = format!(
                        "\n[File: {}]\n{}\n[End of file: {}]",
                        rel_path, content, rel_path
                    );
                    enriched = enriched.replace(&tag, &replacement);
                    ctx_files.push((rel_path.clone(), content));
                    tracing::debug!("Inlined @{} into cswarm prompt ({} bytes)", rel_path, ctx_files.last().unwrap().1.len());
                } else {
                    tracing::warn!("Could not read @{} for cswarm prompt", rel_path);
                }
            }
            (enriched, ctx_files)
        };
        let write_ns = self.chat_state.agent_settings.write_namespace.clone();
        let read_ns = self.chat_state.agent_settings.read_namespaces.clone();
        let memory = self.memory.clone();

        tokio::spawn(async move {
            use gaviero_core::swarm::{pipeline, coordinator};

            let config = pipeline::SwarmConfig {
                max_parallel: 4,
                workspace_root: root.clone(),
                model: "opus".into(),
                use_worktrees: true,
                read_namespaces: read_ns,
                write_namespace: write_ns,
                context_files,
            };

            let coord_config = coordinator::CoordinatorConfig::default();

            let observer = TuiSwarmObserver { tx: tx.clone() };
            let tx2 = tx.clone();
            let make_obs = move |agent_id: &str| -> Box<dyn gaviero_core::observer::AcpObserver> {
                Box::new(TuiAcpObserver {
                    tx: tx2.clone(),
                    conv_id: format!("swarm-{}", agent_id),
                })
            };

            match pipeline::plan_coordinated(
                &task_desc,
                &config,
                coord_config,
                memory,
                &observer,
                make_obs,
            ).await {
                Ok(dsl_text) => {
                    // Write DSL to tmp/ so user can review/edit before running
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let plan_filename = format!("gaviero_plan_{}.gaviero", timestamp);
                    let plan_path = root.join("tmp").join(&plan_filename);
                    if let Err(e) = std::fs::create_dir_all(plan_path.parent().unwrap()) {
                        let _ = tx.send(Event::MessageComplete {
                            conv_id: String::new(),
                            role: "system".to_string(),
                            content: format!("Failed to create tmp/ directory: {}", e),
                        });
                        return;
                    }
                    match std::fs::write(&plan_path, &dsl_text) {
                        Ok(()) => {
                            let _ = tx.send(Event::SwarmDslPlanReady(plan_path));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::MessageComplete {
                                conv_id: String::new(),
                                role: "system".to_string(),
                                content: format!("Failed to write plan file: {}", e),
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Event::SwarmPhaseChanged("failed".to_string()));
                    let _ = tx.send(Event::MessageComplete {
                        conv_id: String::new(),
                        role: "system".to_string(),
                        content: format!("Coordinated swarm planning failed: {}", e),
                    });
                }
            }
        });
    }

    /// Handle `/undo-swarm` — navigate to swarm dashboard and arm undo confirmation.
    fn handle_undo_swarm_command(&mut self) {
        self.chat_state.take_input();

        let has_result = self.swarm_dashboard.result.as_ref()
            .map(|r| !r.pre_swarm_sha.is_empty())
            .unwrap_or(false);

        if !has_result {
            self.chat_state.add_system_message(
                "No undoable swarm result found. Run /cswarm first."
            );
            return;
        }

        // Switch to swarm dashboard and arm confirmation
        self.side_panel = SidePanelMode::SwarmDashboard;
        self.panel_visible.side_panel = true;
        self.focus = Focus::SidePanel;
        self.swarm_dashboard.pending_undo_confirm = true;
        self.chat_state.add_system_message(
            "Swarm dashboard: press u to confirm undo all changes, Esc to cancel."
        );
    }

    /// Handle `/remember <text>` command — store text to semantic memory.
    fn handle_remember_command(&mut self) {
        let input = self.chat_state.take_input();
        let text = input.trim().strip_prefix("/remember").unwrap_or("").trim();

        if text.is_empty() {
            self.chat_state.add_system_message(
                "Usage: /remember <text to remember>\n\
                 Stores text to semantic memory for future retrieval.",
            );
            return;
        }

        self.chat_state.add_user_message(&input);

        let Some(ref memory) = self.memory else {
            self.chat_state.add_system_message(
                "Memory is not available (initialization may still be in progress).",
            );
            return;
        };

        let mem = memory.clone();
        let ns = self.chat_state.agent_settings.write_namespace.clone();
        let content = text.to_string();
        let key = format!(
            "user:{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let tx = self.event_tx.clone();
        let conv_id = self.chat_state.conversations[self.chat_state.active_conv].id.clone();

        tokio::spawn(async move {
            match mem.store(&ns, &key, &content, None).await {
                Ok(_) => {
                    let _ = tx.send(Event::MessageComplete {
                        conv_id,
                        role: "system".to_string(),
                        content: format!("Remembered: \"{}\"", content),
                    });
                }
                Err(e) => {
                    let _ = tx.send(Event::MessageComplete {
                        conv_id,
                        role: "system".to_string(),
                        content: format!("Failed to store memory: {}", e),
                    });
                }
            }
        });
    }

    /// Handle `/attach [path]` command.
    fn handle_attach_command(&mut self) {
        use crate::panels::agent_chat::AttachmentKind;

        let input = self.chat_state.take_input();
        let arg = input
            .trim()
            .strip_prefix("/attach")
            .unwrap_or("")
            .trim()
            .to_string();

        self.chat_state.add_user_message(&input);

        if arg.is_empty() {
            // List current attachments
            if self.chat_state.attachments.is_empty() {
                self.chat_state.add_system_message(
                    "No attachments.\n\
                     Usage: /attach <path>  — attach a file\n\
                     Ctrl+V pastes clipboard images.\n\
                     /detach <name>         — remove an attachment\n\
                     /detach all            — remove all attachments",
                );
            } else {
                let list: Vec<String> = self
                    .chat_state
                    .attachments
                    .iter()
                    .map(|a| {
                        let kind = if a.kind == AttachmentKind::Image {
                            "image"
                        } else {
                            "text"
                        };
                        format!("  {} ({})", a.display_name, kind)
                    })
                    .collect();
                self.chat_state.add_system_message(&format!(
                    "Attachments:\n{}\n\nUse /detach <name> or /detach all to remove.",
                    list.join("\n")
                ));
            }
            return;
        }

        // Resolve path (relative to workspace root or absolute)
        let root = self
            .workspace
            .roots()
            .first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let path = if std::path::Path::new(&arg).is_absolute() {
            std::path::PathBuf::from(&arg)
        } else {
            root.join(&arg)
        };

        if !path.exists() {
            self.chat_state
                .add_system_message(&format!("File not found: {}", path.display()));
            return;
        }

        if !path.is_file() {
            self.chat_state
                .add_system_message(&format!("Not a file: {}", path.display()));
            return;
        }

        // Determine kind from extension
        let kind = match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .as_deref()
        {
            Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg") => {
                AttachmentKind::Image
            }
            _ => AttachmentKind::Text,
        };

        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| arg.clone());

        self.chat_state.add_attachment(path, kind);
        self.chat_state
            .add_system_message(&format!("Attached: {}", display_name));
    }

    /// Handle `/detach [name|all]` command.
    fn handle_detach_command(&mut self) {
        let input = self.chat_state.take_input();
        let arg = input
            .trim()
            .strip_prefix("/detach")
            .unwrap_or("")
            .trim()
            .to_string();

        self.chat_state.add_user_message(&input);

        if arg.is_empty() {
            self.chat_state
                .add_system_message("Usage: /detach <name> or /detach all");
            return;
        }

        if arg == "all" {
            let count = self.chat_state.attachments.len();
            self.chat_state.attachments.clear();
            self.chat_state
                .add_system_message(&format!("Removed {} attachment(s).", count));
        } else if self.chat_state.remove_attachment(&arg) {
            self.chat_state
                .add_system_message(&format!("Removed: {}", arg));
        } else {
            self.chat_state
                .add_system_message(&format!("No attachment named: {}", arg));
        }
    }

    /// Paste from clipboard into chat — checks for image first, then text.
    fn chat_paste_from_clipboard(&mut self) {
        // Try image first
        if let Some(cb) = &mut self.clipboard {
            if let Ok(img) = cb.get_image() {
                if img.width > 0 && img.height > 0 {
                    // Save clipboard image as temp PNG
                    match save_clipboard_image_as_png(&img) {
                        Ok(path) => {
                            let display_name = path
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| "clipboard.png".to_string());
                            self.chat_state.add_attachment(
                                path,
                                crate::panels::agent_chat::AttachmentKind::Image,
                            );
                            self.chat_state.add_system_message(&format!(
                                "Pasted clipboard image: {} ({}x{})",
                                display_name, img.width, img.height
                            ));
                            return;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to save clipboard image: {}", e);
                            // Fall through to text paste
                        }
                    }
                }
            }
        }

        // Fall back to text paste into input (preserves newlines)
        let text = self.get_clipboard();
        if !text.is_empty() {
            self.chat_state.insert_str(&text);
            self.refresh_chat_autocomplete();
        }
    }

    fn cancel_agent(&mut self) {
        let conv_id = self.chat_state.conversations[self.chat_state.active_conv].id.clone();
        if let Some(task) = self.acp_tasks.remove(&conv_id) {
            task.abort();
            self.chat_state.conversations[self.chat_state.active_conv].is_streaming = false;
            self.chat_state.conversations[self.chat_state.active_conv].streaming_started_at = None;
            self.chat_state.finalize_message("system", "Cancelled by user.");
        }
    }

    // ── Layout presets + fullscreen ────────────────────────────────

    fn toggle_fullscreen(&mut self) {
        if self.fullscreen_panel.is_some() {
            // Exit fullscreen
            self.fullscreen_panel = None;
        } else {
            // Enter fullscreen for the focused panel
            self.fullscreen_panel = Some(self.focus);
        }
    }

    fn switch_layout(&mut self, n: u8) {
        let idx = n as usize;
        tracing::debug!("switch_layout: n={}, presets_len={}", n, self.layout_presets.len());
        if idx >= self.layout_presets.len() {
            return;
        }

        // Exit fullscreen if active
        if self.fullscreen_panel.is_some() {
            self.fullscreen_panel = None;
            self.pre_fullscreen = None;
        }

        let preset = &self.layout_presets[idx];
        self.active_preset = Some(idx);

        // Apply visibility: 0% means hidden
        self.panel_visible.file_tree = preset.file_tree_pct > 0;
        self.panel_visible.side_panel = preset.side_panel_pct > 0;

        let label = format!(
            "Layout {} (tree {}%  editor {}%  side {}%)",
            idx + 1,
            preset.file_tree_pct,
            preset.editor_pct,
            preset.side_panel_pct,
        );
        self.status_message = Some((label, std::time::Instant::now()));
    }

    /// Get the effective layout constraints, honoring active preset.
    fn effective_panel_constraints(&self, total_width: u16) -> (u16, u16) {
        if let Some(idx) = self.active_preset {
            if let Some(preset) = self.layout_presets.get(idx) {
                let ft_w = if preset.file_tree_pct > 0 {
                    (total_width as u32 * preset.file_tree_pct as u32 / 100) as u16
                } else {
                    0
                };
                let sp_w = if preset.side_panel_pct > 0 {
                    (total_width as u32 * preset.side_panel_pct as u32 / 100) as u16
                } else {
                    0
                };
                // Only clamp to minimum 1 for panels that are actually visible
                let ft_w = if preset.file_tree_pct > 0 { ft_w.max(1) } else { 0 };
                let sp_w = if preset.side_panel_pct > 0 { sp_w.max(1) } else { 0 };
                return (ft_w, sp_w);
            }
        }
        (self.file_tree_width, self.side_panel_width)
    }

    // ── Review mode actions ──────────────────────────────────────

    /// Handle an action while in diff review mode. Returns true if consumed.
    fn handle_review_action(&mut self, action: &Action) -> bool {
        let review = match &mut self.diff_review {
            Some(r) => r,
            None => return false,
        };
        let is_interactive = review.is_interactive();

        match action {
            // Two-key hunk navigation: ]h / [h
            Action::InsertChar(']') => {
                review.pending_bracket = Some(']');
                return true;
            }
            Action::InsertChar('[') => {
                review.pending_bracket = Some('[');
                return true;
            }
            Action::InsertChar('h') if review.pending_bracket.is_some() => {
                let bracket = review.pending_bracket.take().unwrap();
                match bracket {
                    ']' => review.next_hunk(),
                    '[' => review.prev_hunk(),
                    _ => {}
                }
                return true;
            }
            // Accept current hunk (ACP only)
            Action::InsertChar('a') if is_interactive => {
                review.pending_bracket = None;
                let idx = review.current_hunk;
                review.accept_hunk(idx);
                return true;
            }
            // Reject current hunk (ACP only)
            Action::InsertChar('r') if is_interactive => {
                review.pending_bracket = None;
                let idx = review.current_hunk;
                review.reject_hunk(idx);
                return true;
            }
            // Accept all (ACP only)
            Action::InsertChar('A') if is_interactive => {
                review.pending_bracket = None;
                review.accept_all();
                return true;
            }
            // Reject all (ACP only)
            Action::InsertChar('R') if is_interactive => {
                review.pending_bracket = None;
                review.reject_all();
                return true;
            }
            // Finalize (ACP only): accept all, write to disk, clean up
            Action::InsertChar('f') if is_interactive => {
                let review = self.diff_review.take().unwrap();
                let mut proposal = review.proposal;
                // Accept remaining pending hunks
                for hunk in &mut proposal.structural_hunks {
                    if hunk.status == gaviero_core::types::HunkStatus::Pending {
                        hunk.status = gaviero_core::types::HunkStatus::Accepted;
                    }
                }
                let content = gaviero_core::write_gate::assemble_final_content(&proposal);
                let path = proposal.file_path.clone();

                if let Err(e) = std::fs::write(&path, &content) {
                    tracing::error!("Failed to write finalized file {}: {}", path.display(), e);
                } else {
                    for buf in &mut self.buffers {
                        if buf.path.as_deref() == Some(path.as_path()) {
                            let _ = buf.reload();
                        }
                    }
                }

                // Remove from write gate (fire-and-forget bookkeeping)
                let wg = self.write_gate.clone();
                let id = proposal.id;
                tokio::spawn(async move {
                    let mut gate = wg.lock().await;
                    gate.finalize(id);
                });
                return true;
            }
            // Dismiss review
            Action::InsertChar('q') | Action::Quit => {
                self.diff_review = None;
                return true;
            }
            // Scroll: arrows / j/k for single line, J/K / PgDn/PgUp for page
            Action::CursorDown | Action::InsertChar('j') => {
                review.pending_bracket = None;
                review.scroll_top += 1;
                return true;
            }
            Action::CursorUp | Action::InsertChar('k') => {
                review.pending_bracket = None;
                review.scroll_top = review.scroll_top.saturating_sub(1);
                return true;
            }
            Action::InsertChar('J') | Action::PageDown => {
                review.pending_bracket = None;
                review.scroll_top += theme::DIFF_PAGE_SCROLL;
                return true;
            }
            Action::InsertChar('K') | Action::PageUp => {
                review.pending_bracket = None;
                review.scroll_top = review.scroll_top.saturating_sub(theme::DIFF_PAGE_SCROLL);
                return true;
            }
            // Clear pending bracket on any other key
            _ => {
                if review.pending_bracket.is_some() {
                    review.pending_bracket = None;
                }
                return false;
            }
        }
    }

    /// Enter diff review mode with an owned proposal. No lock needed.
    fn enter_review_mode(&mut self, proposal: WriteProposal, source: DiffSource) {
        if self.diff_review.is_some() {
            return;
        }
        let path = proposal.file_path.clone();
        self.open_file(&path);
        self.focus = Focus::Editor;
        self.diff_review = Some(DiffReviewState::new(proposal, source));
    }

    // ── Batch review mode ────────────────────────────────────────

    /// Enter batch review mode with a set of deferred proposals.
    fn enter_batch_review(&mut self, proposals: Vec<WriteProposal>) {
        if proposals.is_empty() {
            return;
        }

        let review_proposals: Vec<ReviewProposal> = proposals.into_iter().map(|p| {
            let old_lines = p.original_content.lines().count();
            let new_lines = p.proposed_content.lines().count();
            let additions = new_lines.saturating_sub(old_lines) +
                p.structural_hunks.iter()
                    .map(|h| h.diff_hunk.proposed_text.lines().count())
                    .sum::<usize>().min(new_lines);
            let deletions = old_lines.saturating_sub(new_lines) +
                p.structural_hunks.iter()
                    .map(|h| h.diff_hunk.original_text.lines().count())
                    .sum::<usize>().min(old_lines);
            let old_content = if p.original_content.is_empty() {
                None
            } else {
                Some(p.original_content.clone())
            };
            ReviewProposal {
                path: p.file_path,
                old_content,
                new_content: p.proposed_content,
                additions,
                deletions,
            }
        }).collect();

        // Pre-compute diff for the first file
        let initial_diff = if let Some(p) = review_proposals.first() {
            let old_lines: Vec<&str> = p.old_content.as_deref().unwrap_or("").lines().collect();
            let new_lines: Vec<&str> = p.new_content.lines().collect();
            build_simple_diff(&old_lines, &new_lines)
        } else {
            Vec::new()
        };

        self.batch_review = Some(BatchReviewState {
            proposals: review_proposals,
            selected_index: 0,
            scroll_offset: 0,
            diff_scroll: 0,
            cached_diff: initial_diff,
            cached_diff_index: 0,
        });
        self.left_panel = LeftPanelMode::Review;
        self.panel_visible.file_tree = true;
        self.focus = Focus::FileTree;
    }

    /// Handle an action while in batch review mode. Returns true if consumed.
    fn handle_batch_review_action(&mut self, action: &Action) -> bool {
        if self.batch_review.is_none() {
            return false;
        }

        match action {
            // ── Global review actions (work regardless of focus) ──
            Action::InsertChar('f') => {
                self.finalize_batch_review();
                true
            }
            Action::Quit => {
                self.cancel_batch_review();
                true
            }

            // ── Focus-dependent navigation ──
            Action::CursorDown | Action::InsertChar('j') => {
                let br = self.batch_review.as_mut().unwrap();
                if self.focus == Focus::FileTree {
                    if br.selected_index + 1 < br.proposals.len() {
                        br.selected_index += 1;
                        br.diff_scroll = 0;
                    }
                } else {
                    br.diff_scroll += 1;
                }
                true
            }
            Action::CursorUp | Action::InsertChar('k') => {
                let br = self.batch_review.as_mut().unwrap();
                if self.focus == Focus::FileTree {
                    if br.selected_index > 0 {
                        br.selected_index -= 1;
                        br.diff_scroll = 0;
                    }
                } else {
                    br.diff_scroll = br.diff_scroll.saturating_sub(1);
                }
                true
            }
            // Page-size scrolling for diff
            Action::InsertChar('J') | Action::PageDown => {
                let br = self.batch_review.as_mut().unwrap();
                br.diff_scroll += theme::DIFF_PAGE_SCROLL;
                true
            }
            Action::InsertChar('K') | Action::PageUp => {
                let br = self.batch_review.as_mut().unwrap();
                br.diff_scroll = br.diff_scroll.saturating_sub(theme::DIFF_PAGE_SCROLL);
                true
            }

            // Accept current file: write to disk, remove from list
            Action::InsertChar('a') => {
                let br = self.batch_review.as_mut().unwrap();
                if let Some(proposal) = br.proposals.get(br.selected_index) {
                    let path = proposal.path.clone();
                    let content = proposal.new_content.clone();
                    if let Err(e) = std::fs::write(&path, &content) {
                        tracing::error!("Failed to write {}: {}", path.display(), e);
                    } else {
                        // Reload buffer if open
                        for buf in &mut self.buffers {
                            if buf.path.as_deref() == Some(path.as_path()) {
                                let _ = buf.reload();
                            }
                        }
                    }
                    let br = self.batch_review.as_mut().unwrap();
                    br.proposals.remove(br.selected_index);
                    if br.selected_index >= br.proposals.len() && br.selected_index > 0 {
                        br.selected_index -= 1;
                    }
                    br.diff_scroll = 0;
                    br.cached_diff = Vec::new();
                    br.cached_diff_index = usize::MAX;
                    if br.proposals.is_empty() {
                        self.batch_review = None;
                        self.left_panel = LeftPanelMode::FileTree;
                        self.status_message = Some((
                            "All files reviewed".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                }
                true
            }
            // Reject current file: discard, remove from list
            Action::InsertChar('r') => {
                let br = self.batch_review.as_mut().unwrap();
                if !br.proposals.is_empty() {
                    br.proposals.remove(br.selected_index);
                    if br.selected_index >= br.proposals.len() && br.selected_index > 0 {
                        br.selected_index -= 1;
                    }
                    br.diff_scroll = 0;
                    br.cached_diff = Vec::new();
                    br.cached_diff_index = usize::MAX;
                    if br.proposals.is_empty() {
                        self.batch_review = None;
                        self.left_panel = LeftPanelMode::FileTree;
                        self.status_message = Some((
                            "All files reviewed — no changes applied".to_string(),
                            std::time::Instant::now(),
                        ));
                    }
                }
                true
            }

            // Let focus-switching actions (Ctrl+arrows) pass through
            _ => false,
        }
    }

    /// Apply all deferred writes to disk and exit review mode.
    fn finalize_batch_review(&mut self) {
        let br = match self.batch_review.take() {
            Some(r) => r,
            None => return,
        };

        let mut written = Vec::new();
        for proposal in &br.proposals {
            if let Err(e) = std::fs::write(&proposal.path, &proposal.new_content) {
                tracing::error!("Failed to write {}: {}", proposal.path.display(), e);
            } else {
                written.push(proposal.path.clone());
            }
        }

        // Open written files as editor tabs and reload any already-open buffers
        for path in &written {
            // Reload existing buffer if open
            for buf in &mut self.buffers {
                if buf.path.as_deref() == Some(path.as_path()) {
                    let _ = buf.reload();
                }
            }
        }

        self.left_panel = LeftPanelMode::FileTree;
        self.status_message = Some((
            format!("{} file(s) written", written.len()),
            std::time::Instant::now(),
        ));
    }

    /// Discard all proposals and exit review mode.
    fn cancel_batch_review(&mut self) {
        let n = self.batch_review.as_ref().map(|br| br.proposals.len()).unwrap_or(0);
        self.batch_review = None;
        self.left_panel = LeftPanelMode::FileTree;
        self.status_message = Some((
            format!("Review discarded — {} file(s) not written", n),
            std::time::Instant::now(),
        ));
    }

    /// Render the review file list in the left panel.
    fn render_review_file_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        use ratatui::style::Modifier;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Widget};

        let border_style = if focused {
            Style::default().fg(theme::FOCUS_BORDER)
        } else {
            Style::default().fg(theme::TEXT_DIM)
        };

        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, frame.buffer_mut());

        let br = match &mut self.batch_review {
            Some(r) => r,
            None => return,
        };

        let visible = inner.height as usize;

        // Auto-scroll to keep selected_index visible
        if visible > 0 {
            if br.selected_index < br.scroll_offset {
                br.scroll_offset = br.selected_index;
            } else if br.selected_index >= br.scroll_offset + visible {
                br.scroll_offset = br.selected_index - visible + 1;
            }
        }

        let scroll = br.scroll_offset;

        for (row, (i, proposal)) in br.proposals.iter().enumerate()
            .skip(scroll)
            .take(visible)
            .enumerate()
        {
            let _ = row; // silence warning, we use i for selection check
            let y = inner.y + row as u16;
            if y >= inner.bottom() { break; }

            let is_selected = i == br.selected_index;

            // Extract filename from path
            let filename = proposal.path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");

            // Build the line: " filename  +N -M"
            let name_style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme::SELECTION_BG)
            } else {
                Style::default().fg(theme::TEXT_FG)
            };

            let adds = format!(" +{}", proposal.additions);
            let dels = format!(" -{}", proposal.deletions);

            let spans = vec![
                Span::styled(format!(" {}", filename), name_style),
                Span::styled(adds, Style::default().fg(theme::SUCCESS)),
                Span::styled(dels, Style::default().fg(theme::ERROR)),
            ];

            let line = Line::from(spans);
            let line_area = Rect { x: inner.x, y, width: inner.width, height: 1 };

            // Fill background for selected row
            if is_selected {
                for x in inner.x..inner.right() {
                    frame.buffer_mut()[(x, y)].set_bg(theme::SELECTION_BG);
                }
            }

            Widget::render(line, line_area, frame.buffer_mut());
        }

        // Scrollbar
        crate::widgets::scrollbar::render_scrollbar(
            inner,
            frame.buffer_mut(),
            br.proposals.len(),
            visible,
            scroll,
        );
    }

    /// Render the batch review diff in the editor area.
    fn render_batch_review_diff(&mut self, frame: &mut Frame, area: Rect) {
        use ratatui::style::Modifier;

        let br = match &mut self.batch_review {
            Some(r) => r,
            None => return,
        };

        let proposal = match br.proposals.get(br.selected_index) {
            Some(p) => p,
            None => return,
        };

        // Recompute diff only when selection changes
        if br.cached_diff_index != br.selected_index {
            let old_lines: Vec<&str> = proposal.old_content
                .as_deref()
                .unwrap_or("")
                .lines()
                .collect();
            let new_lines: Vec<&str> = proposal.new_content.lines().collect();
            br.cached_diff = build_simple_diff(&old_lines, &new_lines);
            br.cached_diff_index = br.selected_index;
        }

        let diff_lines = &br.cached_diff;

        let gutter_w = theme::DIFF_GUTTER_WIDTH;
        // Clamp diff scroll to valid range
        let max_scroll = diff_lines.len().saturating_sub(1);
        if br.diff_scroll > max_scroll {
            br.diff_scroll = max_scroll;
        }
        let scroll = br.diff_scroll;

        // Show file path as header
        let header = format!(" {} ", proposal.path.display());
        let header_style = Style::default()
            .fg(theme::FOCUS_BORDER)
            .add_modifier(Modifier::BOLD);
        for (i, ch) in header.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() && area.y < area.bottom() {
                frame.buffer_mut()[(x, area.y)]
                    .set_char(ch)
                    .set_style(header_style);
            }
        }

        // Render diff lines starting from row 1 (after header)
        let content_height = area.height.saturating_sub(1) as usize;
        for row in 0..content_height {
            let line_idx = scroll + row;
            if line_idx >= diff_lines.len() {
                break;
            }

            let y = area.y + 1 + row as u16;
            if y >= area.bottom() { break; }

            let (kind, text) = &diff_lines[line_idx];

            // Gutter
            let (gutter_str, gutter_style, line_style) = match kind {
                DiffKind::Added => (
                    " + │ ",
                    Style::default().fg(theme::SUCCESS).add_modifier(Modifier::BOLD),
                    Style::default().fg(theme::SUCCESS).bg(theme::DIFF_ADD_LINE_BG),
                ),
                DiffKind::Removed => (
                    " - │ ",
                    Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD),
                    Style::default().fg(theme::ERROR).bg(theme::DIFF_REM_LINE_BG),
                ),
                DiffKind::Context => (
                    "   │ ",
                    Style::default().fg(theme::TEXT_DIM),
                    Style::default().fg(theme::TEXT_FG),
                ),
            };

            for (i, ch) in gutter_str.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, y)]
                        .set_char(ch)
                        .set_style(gutter_style);
                }
            }

            for (i, ch) in text.chars().enumerate() {
                let x = area.x + gutter_w + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, y)]
                        .set_char(ch)
                        .set_style(line_style);
                }
            }
        }
    }

    // ── Git Changes panel ────────────────────────────────────────

    /// Populate changes_state from `git status` + working-tree diffs.
    fn refresh_git_changes(&mut self) {
        let root = match self.workspace.roots().first() {
            Some(r) => r.to_path_buf(),
            None => {
                self.changes_state = None;
                return;
            }
        };
        let repo = match gaviero_core::git::GitRepo::open(&root) {
            Ok(r) => r,
            Err(_) => {
                self.changes_state = None;
                return;
            }
        };
        let entries = match repo.file_status() {
            Ok(e) => e,
            Err(_) => {
                self.changes_state = None;
                return;
            }
        };

        let workdir = repo.workdir().unwrap_or(&root).to_path_buf();

        let git_entries: Vec<ChangesEntry> = entries
            .into_iter()
            .filter(|e| !e.staged) // show working tree changes
            .map(|e| {
                let abs_path = workdir.join(&e.path);
                let old_content = repo.head_file_content(&e.path).unwrap_or_default();
                let new_content =
                    std::fs::read_to_string(&abs_path).unwrap_or_default();

                let old_lines: Vec<&str> = old_content.lines().collect();
                let new_lines: Vec<&str> = new_content.lines().collect();
                let diff = build_simple_diff(&old_lines, &new_lines);
                let additions = diff.iter().filter(|(k, _)| matches!(k, DiffKind::Added)).count();
                let deletions = diff.iter().filter(|(k, _)| matches!(k, DiffKind::Removed)).count();

                ChangesEntry {
                    rel_path: e.path,
                    abs_path,
                    status_char: e.status.marker(),
                    additions,
                    deletions,
                }
            })
            .collect();

        if git_entries.is_empty() {
            self.changes_state = None;
            return;
        }

        self.changes_state = Some(ChangesState {
            entries: git_entries,
            selected_index: 0,
            scroll_offset: 0,
            diff_scroll: 0,
            cached_diff: Vec::new(),
            cached_diff_index: usize::MAX,
        });
    }

    /// Handle a keyboard action while in Changes mode. Returns true if consumed.
    fn handle_changes_action(&mut self, action: &Action) -> bool {
        let cs = match &mut self.changes_state {
            Some(s) => s,
            None => return false,
        };

        match action {
            Action::CursorDown | Action::InsertChar('j') => {
                if cs.selected_index + 1 < cs.entries.len() {
                    cs.selected_index += 1;
                    cs.diff_scroll = 0;
                }
                true
            }
            Action::CursorUp | Action::InsertChar('k') => {
                if cs.selected_index > 0 {
                    cs.selected_index -= 1;
                    cs.diff_scroll = 0;
                }
                true
            }
            Action::InsertChar('J') | Action::PageDown => {
                cs.diff_scroll += theme::DIFF_PAGE_SCROLL;
                true
            }
            Action::InsertChar('K') | Action::PageUp => {
                cs.diff_scroll = cs.diff_scroll.saturating_sub(theme::DIFF_PAGE_SCROLL);
                true
            }
            Action::Enter => {
                // Open selected file in the editor
                if let Some(entry) = cs.entries.get(cs.selected_index) {
                    let path = entry.abs_path.clone();
                    self.open_file(&path);
                    self.left_panel = LeftPanelMode::FileTree;
                    self.focus = Focus::Editor;
                }
                true
            }
            Action::Quit => {
                // Esc: go back to FileTree
                self.left_panel = LeftPanelMode::FileTree;
                self.changes_state = None;
                true
            }
            Action::InsertChar('R') => {
                // Refresh changes from git
                self.refresh_git_changes();
                true
            }
            _ => false,
        }
    }

    /// Render the git changes file list in the left panel.
    fn render_changes_file_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        use ratatui::style::Modifier;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Block, Borders, Widget};

        let border_style = if focused {
            Style::default().fg(theme::FOCUS_BORDER)
        } else {
            Style::default().fg(theme::TEXT_DIM)
        };

        let block = Block::default()
            .borders(Borders::RIGHT)
            .border_style(border_style);
        let inner = block.inner(area);
        block.render(area, frame.buffer_mut());

        let cs = match &mut self.changes_state {
            Some(s) => s,
            None => {
                // Empty state
                let msg = " No changes";
                let y = inner.y;
                if y < inner.bottom() {
                    for (i, ch) in msg.chars().enumerate() {
                        let x = inner.x + i as u16;
                        if x < inner.right() {
                            frame.buffer_mut()[(x, y)]
                                .set_char(ch)
                                .set_style(Style::default().fg(theme::TEXT_DIM));
                        }
                    }
                }
                return;
            }
        };

        let visible = inner.height as usize;

        // Auto-scroll to keep selected_index visible
        if visible > 0 {
            if cs.selected_index < cs.scroll_offset {
                cs.scroll_offset = cs.selected_index;
            } else if cs.selected_index >= cs.scroll_offset + visible {
                cs.scroll_offset = cs.selected_index - visible + 1;
            }
        }

        let scroll = cs.scroll_offset;

        for (row, (i, entry)) in cs.entries.iter().enumerate()
            .skip(scroll)
            .take(visible)
            .enumerate()
        {
            let _ = row;
            let y = inner.y + row as u16;
            if y >= inner.bottom() { break; }

            let is_selected = i == cs.selected_index;

            let filename = std::path::Path::new(&entry.rel_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");

            let status_style = match entry.status_char {
                'M' => Style::default().fg(theme::WARNING),
                'A' | '?' => Style::default().fg(theme::SUCCESS),
                'D' => Style::default().fg(theme::ERROR),
                _ => Style::default().fg(theme::TEXT_DIM),
            };

            let name_style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme::SELECTION_BG)
            } else {
                Style::default().fg(theme::TEXT_FG)
            };

            let adds = format!(" +{}", entry.additions);
            let dels = format!(" -{}", entry.deletions);

            let spans = vec![
                Span::styled(format!(" {} ", entry.status_char), status_style),
                Span::styled(filename.to_string(), name_style),
                Span::styled(adds, Style::default().fg(theme::SUCCESS)),
                Span::styled(dels, Style::default().fg(theme::ERROR)),
            ];

            let line = Line::from(spans);
            let line_area = Rect { x: inner.x, y, width: inner.width, height: 1 };

            if is_selected {
                for x in inner.x..inner.right() {
                    frame.buffer_mut()[(x, y)].set_bg(theme::SELECTION_BG);
                }
            }

            Widget::render(line, line_area, frame.buffer_mut());
        }

        // Scrollbar
        crate::widgets::scrollbar::render_scrollbar(
            inner,
            frame.buffer_mut(),
            cs.entries.len(),
            visible,
            scroll,
        );
    }

    /// Render the git changes diff in the editor area.
    fn render_changes_diff(&mut self, frame: &mut Frame, area: Rect) {
        use ratatui::style::Modifier;

        let cs = match &mut self.changes_state {
            Some(s) => s,
            None => return,
        };

        let entry = match cs.entries.get(cs.selected_index) {
            Some(e) => e,
            None => return,
        };

        // Recompute diff only when selection changes
        if cs.cached_diff_index != cs.selected_index {
            let root = self.workspace.roots().first().map(|r| r.to_path_buf());
            let old_content = root
                .as_ref()
                .and_then(|r| gaviero_core::git::GitRepo::open(r).ok())
                .and_then(|repo| repo.head_file_content(&entry.rel_path).ok())
                .unwrap_or_default();
            let new_content =
                std::fs::read_to_string(&entry.abs_path).unwrap_or_default();

            let old_lines: Vec<&str> = old_content.lines().collect();
            let new_lines: Vec<&str> = new_content.lines().collect();
            cs.cached_diff = build_simple_diff(&old_lines, &new_lines);
            cs.cached_diff_index = cs.selected_index;
        }

        let diff_lines = &cs.cached_diff;

        let gutter_w = theme::DIFF_GUTTER_WIDTH;
        let max_scroll = diff_lines.len().saturating_sub(1);
        if cs.diff_scroll > max_scroll {
            cs.diff_scroll = max_scroll;
        }
        let scroll = cs.diff_scroll;

        // Show file path as header
        let header = format!(" {} ", entry.rel_path);
        let header_style = Style::default()
            .fg(theme::FOCUS_BORDER)
            .add_modifier(Modifier::BOLD);
        for (i, ch) in header.chars().enumerate() {
            let x = area.x + i as u16;
            if x < area.right() && area.y < area.bottom() {
                frame.buffer_mut()[(x, area.y)]
                    .set_char(ch)
                    .set_style(header_style);
            }
        }

        // Render diff lines starting from row 1 (after header)
        let content_height = area.height.saturating_sub(1) as usize;
        for row in 0..content_height {
            let line_idx = scroll + row;
            if line_idx >= diff_lines.len() {
                break;
            }

            let y = area.y + 1 + row as u16;
            if y >= area.bottom() { break; }

            let (kind, text) = &diff_lines[line_idx];

            let (gutter_str, gutter_style, line_style) = match kind {
                DiffKind::Added => (
                    " + │ ",
                    Style::default().fg(theme::SUCCESS).add_modifier(Modifier::BOLD),
                    Style::default().fg(theme::SUCCESS).bg(theme::DIFF_ADD_LINE_BG),
                ),
                DiffKind::Removed => (
                    " - │ ",
                    Style::default().fg(theme::ERROR).add_modifier(Modifier::BOLD),
                    Style::default().fg(theme::ERROR).bg(theme::DIFF_REM_LINE_BG),
                ),
                DiffKind::Context => (
                    "   │ ",
                    Style::default().fg(theme::TEXT_DIM),
                    Style::default().fg(theme::TEXT_FG),
                ),
            };

            for (i, ch) in gutter_str.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, y)]
                        .set_char(ch)
                        .set_style(gutter_style);
                }
            }

            for (i, ch) in text.chars().enumerate() {
                let x = area.x + gutter_w + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, y)]
                        .set_char(ch)
                        .set_style(line_style);
                }
            }
        }
    }

    // ── Find bar (Ctrl+F) ────────────────────────────────────────

    fn handle_find_bar_action(&mut self, action: Action) {
        match action {
            Action::InsertChar(ch) => {
                self.find_input.insert_char(ch);
                self.update_find_highlight();
            }
            Action::Backspace => {
                self.find_input.backspace();
                self.update_find_highlight();
            }
            Action::Delete => {
                self.find_input.delete();
                self.update_find_highlight();
            }
            Action::DeleteWordBack => {
                self.find_input.delete_word_back();
                self.update_find_highlight();
            }
            Action::CursorLeft => self.find_input.move_left(),
            Action::CursorRight => self.find_input.move_right(),
            Action::WordLeft => self.find_input.move_word_left(),
            Action::WordRight => self.find_input.move_word_right(),
            Action::SelectLeft => self.find_input.select_left(),
            Action::SelectRight => self.find_input.select_right(),
            Action::SelectWordLeft => self.find_input.select_word_left(),
            Action::SelectWordRight => self.find_input.select_word_right(),
            Action::Home => self.find_input.move_home(),
            Action::End => self.find_input.move_end(),
            Action::SelectAll => self.find_input.select_all(),
            Action::Paste => {
                let text = self.get_clipboard();
                if !text.is_empty() {
                    self.find_input.insert_str(&text);
                    self.update_find_highlight();
                }
            }
            Action::Enter | Action::CursorDown => {
                // Enter / Down: go to next match
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    buf.find_next_match();
                }
                self.ensure_editor_cursor_visible();
            }
            Action::CursorUp => {
                // Up: go to previous match
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    buf.find_prev_match();
                }
                self.ensure_editor_cursor_visible();
            }
            Action::Quit => {
                // Escape: close find bar, clear highlights
                self.find_bar_active = false;
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    buf.set_search_highlight(None);
                }
            }
            Action::FindInBuffer => {
                // Ctrl+F again: close
                self.find_bar_active = false;
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    buf.set_search_highlight(None);
                }
            }
            _ => {}
        }
    }

    /// Update the editor's search highlight from the find bar input, and jump
    /// to the first match at or after the cursor.
    fn update_find_highlight(&mut self) {
        let query = self.find_input.text.clone();
        if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
            if query.is_empty() {
                buf.set_search_highlight(None);
            } else {
                buf.set_search_highlight(Some(query));
                buf.find_next_match();
            }
        }
        self.ensure_editor_cursor_visible();
    }

    /// Make sure the editor cursor is within the visible viewport.
    fn ensure_editor_cursor_visible(&mut self) {
        let area = self.layout.editor_area;
        if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
            let line_count = buf.line_count();
            let gutter_w = gutter_width(line_count) as usize;
            let vp_h = area.height as usize;
            let vp_w = (area.width as usize).saturating_sub(gutter_w);
            buf.ensure_cursor_visible(vp_h, vp_w);
        }
    }

    // ── Editor actions ───────────────────────────────────────────

    fn handle_editor_action(&mut self, action: Action) {
        // Clipboard actions need &mut self, handle separately
        match action {
            Action::Copy => { self.clipboard_copy(); return; }
            Action::Cut => { self.clipboard_cut(); }
            Action::Paste => { self.clipboard_paste(); }
            Action::SelectAll => {
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    buf.select_all();
                }
            }
            Action::FormatBuffer => {
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    let msg = if buf.selection_range().is_some() {
                        buf.format_selection()
                    } else {
                        buf.format()
                    };
                    self.status_message = Some((msg, std::time::Instant::now()));
                }
            }
            Action::CycleFormatLevel => {
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    let msg = buf.cycle_format_level();
                    self.status_message = Some((msg, std::time::Instant::now()));
                }
            }
            _ => {
                let area = self.layout.editor_area;
                let Some(buf) = self.buffers.get_mut(self.active_buffer) else {
                    return;
                };
                let line_count = buf.line_count();
                let gutter_w = gutter_width(line_count) as usize;
                let vp_h = area.height as usize;
                let vp_w = (area.width as usize).saturating_sub(gutter_w);
                match action {
                    Action::Tab => buf.insert_tab(),
                    Action::InsertChar(ch) => buf.insert_char(ch),
                    Action::Backspace => { buf.backspace(); }
                    Action::Delete => { buf.delete(); }
                    Action::Enter => buf.insert_newline(),
                    Action::CursorUp => buf.move_cursor_up(),
                    Action::CursorDown => buf.move_cursor_down(),
                    Action::CursorLeft => buf.move_cursor_left(),
                    Action::CursorRight => buf.move_cursor_right(),
                    Action::WordLeft => buf.move_word_left(),
                    Action::WordRight => buf.move_word_right(),
                    Action::SelectLeft => buf.select_left(),
                    Action::SelectRight => buf.select_right(),
                    Action::SelectUp => buf.select_up(),
                    Action::SelectDown => buf.select_down(),
                    Action::SelectWordLeft => buf.select_word_left(),
                    Action::SelectWordRight => buf.select_word_right(),
                    Action::PageUp => buf.page_up(vp_h),
                    Action::PageDown => buf.page_down(vp_h),
                    Action::Home => buf.move_cursor_home(),
                    Action::End => buf.move_cursor_end(),
                    Action::Undo => { buf.undo(); }
                    Action::Redo => { buf.redo(); }
                    Action::DeleteLine => buf.delete_line(),
                    Action::DuplicateLine => buf.duplicate_line(),
                    Action::MoveLineUp => buf.move_line_up(),
                    Action::MoveLineDown => buf.move_line_down(),
                    Action::GoToLineEnd => buf.move_cursor_end(),
                    Action::DeleteToLineEnd => buf.delete_to_line_end(),
                    Action::DeleteWordBack => buf.delete_word_back(),
                    _ => {}
                }
                buf.ensure_cursor_visible(vp_h, vp_w);
            }
        }

        let area = self.layout.editor_area;
        if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
            let line_count = buf.line_count();
            let gutter_w = gutter_width(line_count) as usize;
            let vp_h = area.height as usize;
            let vp_w = (area.width as usize).saturating_sub(gutter_w);
            buf.ensure_cursor_visible(vp_h, vp_w);
        }
    }

    // ── Clipboard ───────────────────────────────────────────────

    fn clipboard_copy(&mut self) {
        let Some(buf) = self.buffers.get(self.active_buffer) else { return };
        let text = buf.selected_text();
        if text.is_empty() { return; }
        let n = text.chars().count();
        let suffix = if n == 1 { "" } else { "s" };
        let msg = match self.set_clipboard(&text) {
            ClipboardResult::System  => format!("Copied {} char{}", n, suffix),
            ClipboardResult::Osc52   => format!("Copied {} char{} (via terminal)", n, suffix),
            ClipboardResult::Unavailable => format!("Copied {} char{} (internal only — terminal does not support OSC 52)", n, suffix),
        };
        self.status_message = Some((msg, std::time::Instant::now()));
    }

    fn clipboard_cut(&mut self) {
        let Some(buf) = self.buffers.get_mut(self.active_buffer) else { return };
        let text = buf.delete_selection();
        if text.is_empty() { return; }
        let n = text.chars().count();
        let suffix = if n == 1 { "" } else { "s" };
        let msg = match self.set_clipboard(&text) {
            ClipboardResult::System  => format!("Cut {} char{}", n, suffix),
            ClipboardResult::Osc52   => format!("Cut {} char{} (via terminal)", n, suffix),
            ClipboardResult::Unavailable => format!("Cut {} char{} (internal only — terminal does not support OSC 52)", n, suffix),
        };
        self.status_message = Some((msg, std::time::Instant::now()));
    }

    fn clipboard_paste(&mut self) {
        let text = self.get_clipboard();
        if text.is_empty() { return; }
        if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
            buf.paste_text(&text);
        }
    }

    /// Sets text on both the internal clipboard and the system clipboard.
    /// Tries arboard first, then OSC 52 (for SSH sessions), then falls back to internal only.
    fn set_clipboard(&mut self, text: &str) -> ClipboardResult {
        self.internal_clipboard = text.to_string();
        if let Some(cb) = &mut self.clipboard {
            if cb.set_text(text).is_ok() {
                return ClipboardResult::System;
            }
            tracing::warn!("arboard set_text failed, falling back to OSC 52");
        }
        // OSC 52 fallback: works over SSH when the terminal emulator supports it
        if osc52_copy(text) {
            ClipboardResult::Osc52
        } else {
            ClipboardResult::Unavailable
        }
    }

    fn get_clipboard(&mut self) -> String {
        // Try system clipboard first, fall back to internal
        if let Some(cb) = &mut self.clipboard {
            if let Ok(text) = cb.get_text() {
                return text;
            }
        }
        self.internal_clipboard.clone()
    }

    fn handle_search_action(&mut self, action: Action) {
        if self.search_panel.editing {
            // ── Input mode: typing updates the query ────────────
            match action {
                Action::InsertChar(ch) => {
                    self.search_panel.input.insert_char(ch);
                    self.run_search_from_input();
                }
                Action::Backspace => {
                    self.search_panel.input.backspace();
                    self.run_search_from_input();
                }
                Action::Delete => {
                    self.search_panel.input.delete();
                    self.run_search_from_input();
                }
                Action::DeleteWordBack => {
                    self.search_panel.input.delete_word_back();
                    self.run_search_from_input();
                }
                Action::CursorLeft => self.search_panel.input.move_left(),
                Action::CursorRight => self.search_panel.input.move_right(),
                Action::WordLeft => self.search_panel.input.move_word_left(),
                Action::WordRight => self.search_panel.input.move_word_right(),
                Action::SelectLeft => self.search_panel.input.select_left(),
                Action::SelectRight => self.search_panel.input.select_right(),
                Action::SelectWordLeft => self.search_panel.input.select_word_left(),
                Action::SelectWordRight => self.search_panel.input.select_word_right(),
                Action::Home => self.search_panel.input.move_home(),
                Action::End => self.search_panel.input.move_end(),
                Action::SelectAll => self.search_panel.input.select_all(),
                Action::Paste => {
                    let text = self.get_clipboard();
                    if !text.is_empty() {
                        self.search_panel.input.insert_str(&text);
                        self.run_search_from_input();
                    }
                }
                Action::CursorDown | Action::Enter => {
                    // Move focus to results list (if there are results)
                    if !self.search_panel.results.is_empty() {
                        self.search_panel.editing = false;
                    }
                    if action == Action::Enter {
                        self.open_selected_search_result();
                    }
                }
                Action::Quit => {
                    if !self.search_panel.input.is_empty() {
                        // First Esc clears the input
                        self.search_panel.input.clear();
                        self.search_panel.results.clear();
                        self.search_panel.query.clear();
                        self.search_panel.scroll.reset();
                    } else {
                        // Second Esc goes back to file tree
                        self.left_panel = LeftPanelMode::FileTree;
                    }
                }
                _ => {}
            }
        } else {
            // ── Results mode: navigate and open ─────────────────
            match action {
                Action::CursorDown => {
                    let count = self.search_panel.results.len();
                    self.search_panel.scroll.move_down(count);
                }
                Action::CursorUp => {
                    if self.search_panel.scroll.selected == 0 {
                        // At top of results → go back to input
                        self.search_panel.editing = true;
                    } else {
                        self.search_panel.scroll.move_up();
                    }
                }
                Action::Enter => {
                    self.open_selected_search_result();
                }
                Action::InsertChar(ch) => {
                    // Start typing → switch back to input mode
                    self.search_panel.editing = true;
                    self.search_panel.input.insert_char(ch);
                    self.run_search_from_input();
                }
                Action::Backspace => {
                    self.search_panel.editing = true;
                    self.search_panel.input.backspace();
                    self.run_search_from_input();
                }
                Action::Quit => {
                    // Escape from results goes back to input
                    self.search_panel.editing = true;
                }
                _ => {}
            }
        }
    }

    /// Run search using the current input text.
    fn run_search_from_input(&mut self) {
        let roots = self.workspace.roots();
        let excludes: Vec<String> = self.file_tree.exclude_patterns.clone();
        self.search_panel.search_from_input(&roots, &excludes);
    }

    /// Open the currently selected search result in the editor.
    fn open_selected_search_result(&mut self) {
        if let Some(result) = self.search_panel.selected_result().cloned() {
            let root = self.workspace.roots().first()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            let abs_path = root.join(&result.path);
            if abs_path.exists() {
                self.open_file(&abs_path);
                self.focus = Focus::Editor;
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    let target_line = result.line_number.saturating_sub(1);
                    let max_line = buf.line_count().saturating_sub(1);
                    buf.cursor.line = target_line.min(max_line);
                    buf.cursor.col = 0;
                    buf.cursor.anchor = None;
                    buf.scroll.top_line = target_line.saturating_sub(10);
                }
            }
        }
    }

    fn handle_file_tree_action(&mut self, action: Action) {
        match action {
            Action::CursorDown | Action::InsertChar('j') => self.file_tree.move_down(),
            Action::CursorUp | Action::InsertChar('k') => self.file_tree.move_up(),
            Action::Enter => {
                if self.file_tree.selected_is_file() {
                    if let Some(path) = self.file_tree.selected_path() {
                        let path = path.to_path_buf();
                        self.open_file(&path);
                        self.focus = Focus::Editor;
                    }
                } else {
                    self.file_tree.toggle_expand();
                }
            }
            // n = new file, N = new folder, r = rename, d = delete
            Action::InsertChar('n') => self.start_tree_dialog(TreeDialogKind::NewFile),
            Action::InsertChar('N') => self.start_tree_dialog(TreeDialogKind::NewFolder),
            Action::InsertChar('r') => self.start_tree_dialog(TreeDialogKind::Rename),
            Action::InsertChar('d') | Action::Delete => self.start_tree_dialog(TreeDialogKind::Delete),
            _ => {}
        }
    }

    /// Determine the target directory for a new file/folder based on selected entry.
    fn selected_dir(&self) -> Option<std::path::PathBuf> {
        let entry = self.file_tree.entries.get(self.file_tree.scroll.selected)?;
        if entry.is_dir {
            Some(entry.path.clone())
        } else {
            entry.path.parent().map(|p| p.to_path_buf())
        }
    }

    fn start_tree_dialog(&mut self, kind: TreeDialogKind) {
        let Some(target_dir) = self.selected_dir() else { return };

        let mut dialog = TreeDialog::new(kind.clone(), target_dir);

        // For rename, pre-fill with current name
        if matches!(kind, TreeDialogKind::Rename) {
            if let Some(entry) = self.file_tree.entries.get(self.file_tree.scroll.selected) {
                dialog.original_path = Some(entry.path.clone());
                dialog.input = entry.name.clone();
                dialog.cursor = dialog.input.len();
            }
        }

        // For delete, store the path (no text input needed, uses y/n confirmation)
        if matches!(kind, TreeDialogKind::Delete) {
            if let Some(entry) = self.file_tree.entries.get(self.file_tree.scroll.selected) {
                dialog.original_path = Some(entry.path.clone());
            }
        }

        self.tree_dialog = Some(dialog);
    }

    fn handle_dialog_key(&mut self, key: &crossterm::event::KeyEvent) {
        use crossterm::event::{KeyCode, KeyModifiers};

        // Delete confirmation uses simple y/n keys, no text editing
        if self.tree_dialog.as_ref().is_some_and(|d| matches!(d.kind, TreeDialogKind::Delete)) {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_tree_dialog();
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.tree_dialog = None;
                }
                _ => {}
            }
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                self.tree_dialog = None;
            }
            KeyCode::Enter => {
                self.confirm_tree_dialog();
            }
            KeyCode::Backspace => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.backspace();
                }
            }
            KeyCode::Delete => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.delete();
                }
            }
            KeyCode::Left => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.move_left();
                }
            }
            KeyCode::Right => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.move_right();
                }
            }
            KeyCode::Home => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.move_home();
                }
            }
            KeyCode::End => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.move_end();
                }
            }
            KeyCode::Char('u') if ctrl => {
                // Ctrl+U clears the input
                if let Some(ref mut d) = self.tree_dialog {
                    d.input.clear();
                    d.cursor = 0;
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let Some(ref mut d) = self.tree_dialog {
                    d.insert_char(c);
                }
            }
            _ => {}
        }
    }

    fn confirm_tree_dialog(&mut self) {
        let Some(dialog) = self.tree_dialog.take() else { return };

        match dialog.kind {
            TreeDialogKind::NewFile => {
                let name = dialog.input.trim();
                if name.is_empty() { return; }
                let path = dialog.target_dir.join(name);
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&path, "") {
                    tracing::error!("Failed to create file {}: {}", path.display(), e);
                } else {
                    self.refresh_file_tree();
                    self.select_path_in_tree(&path);
                    self.open_file(&path);
                    self.focus = Focus::Editor;
                }
            }
            TreeDialogKind::NewFolder => {
                let name = dialog.input.trim();
                if name.is_empty() { return; }
                let path = dialog.target_dir.join(name);
                if let Err(e) = std::fs::create_dir_all(&path) {
                    tracing::error!("Failed to create folder {}: {}", path.display(), e);
                } else {
                    self.refresh_file_tree();
                    self.select_path_in_tree(&path);
                }
            }
            TreeDialogKind::Rename => {
                let name = dialog.input.trim();
                if name.is_empty() { return; }
                if let Some(original) = dialog.original_path {
                    let new_path = original.parent()
                        .map(|p| p.join(name))
                        .unwrap_or_else(|| std::path::PathBuf::from(name));
                    if let Err(e) = std::fs::rename(&original, &new_path) {
                        tracing::error!("Failed to rename: {}", e);
                    } else {
                        // Update any open buffer paths
                        for buf in &mut self.buffers {
                            if buf.path.as_deref() == Some(&original) {
                                buf.path = Some(new_path.clone());
                            }
                        }
                        self.refresh_file_tree();
                    }
                }
            }
            TreeDialogKind::Delete => {
                // Confirmation already handled by handle_dialog_key (y/n)
                if let Some(path) = dialog.original_path {
                    let result = if path.is_dir() {
                        std::fs::remove_dir_all(&path)
                    } else {
                        std::fs::remove_file(&path)
                    };
                    if let Err(e) = result {
                        tracing::error!("Failed to delete {}: {}", path.display(), e);
                    } else {
                        // Close any open buffer for this file
                        self.buffers.retain(|b| b.path.as_deref() != Some(&path));
                        if self.active_buffer >= self.buffers.len() && !self.buffers.is_empty() {
                            self.active_buffer = self.buffers.len() - 1;
                        }
                        self.refresh_file_tree();
                    }
                }
            }
        }
    }

    /// Select a specific path in the file tree (scrolls to it).
    fn select_path_in_tree(&mut self, path: &std::path::Path) {
        for (i, entry) in self.file_tree.entries.iter().enumerate() {
            if entry.path == path {
                self.file_tree.scroll.selected = i;
                return;
            }
        }
    }

    fn refresh_file_tree(&mut self) {
        let excludes = parse_exclude_patterns(&self.workspace);
        let git_allow = parse_git_allow_list(&self.workspace);
        let roots: Vec<&std::path::Path> = self.workspace.roots();
        let expanded = self.file_tree.expanded_paths();
        let selected = self.file_tree.scroll.selected;
        self.file_tree = FileTreeState::from_roots(&roots, &excludes, &git_allow);
        self.file_tree.restore_expanded(&expanded);
        self.file_tree.scroll.selected = selected.min(self.file_tree.entries.len().saturating_sub(1));
    }

    // ── Mouse handling ───────────────────────────────────────────

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        let col = mouse.column;
        let row = mouse.row;

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // If in interactive review mode and click is in gutter, toggle hunk
                if let Some(ref mut review) = self.diff_review {
                    if review.is_interactive()
                        && self.layout.editor_area.contains((col, row).into())
                        && col < self.layout.editor_area.x + DIFF_GUTTER_WIDTH
                    {
                        let relative_row = (row - self.layout.editor_area.y) as usize;
                        if let Some(hunk_idx) = diff_overlay::hunk_at_row(
                            review,
                            relative_row,
                        ) {
                            let current = review.proposal.structural_hunks
                                .get(hunk_idx)
                                .map(|h| h.status.clone());
                            match current {
                                Some(gaviero_core::types::HunkStatus::Accepted) => {
                                    review.reject_hunk(hunk_idx);
                                }
                                _ => {
                                    review.accept_hunk(hunk_idx);
                                }
                            }
                            return;
                        }
                    }
                }

                // ── Header arrow click: cycle left panel mode ──
                if let Some(hdr) = self.layout.left_header_area {
                    if hdr.contains((col, row).into()) {
                        // Click on the arrow region (last 3 columns) cycles panel
                        let arrow_zone = hdr.x + hdr.width.saturating_sub(3);
                        if col >= arrow_zone {
                            self.focus = Focus::FileTree;
                            self.left_panel = match self.left_panel {
                                LeftPanelMode::FileTree => LeftPanelMode::Search,
                                LeftPanelMode::Search => LeftPanelMode::Review,
                                LeftPanelMode::Review => LeftPanelMode::Changes,
                                LeftPanelMode::Changes => LeftPanelMode::FileTree,
                            };
                            return;
                        }
                        // Click elsewhere on header just focuses the panel
                        self.focus = Focus::FileTree;
                        return;
                    }
                }

                // ── Header arrow click: cycle side panel mode ──
                if let Some(hdr) = self.layout.side_header_area {
                    if hdr.contains((col, row).into()) {
                        let arrow_zone = hdr.x + hdr.width.saturating_sub(3);
                        if col >= arrow_zone {
                            self.focus = Focus::SidePanel;
                            self.side_panel = match self.side_panel {
                                SidePanelMode::AgentChat => SidePanelMode::SwarmDashboard,
                                SidePanelMode::SwarmDashboard => SidePanelMode::GitPanel,
                                SidePanelMode::GitPanel => SidePanelMode::AgentChat,
                            };
                            return;
                        }
                        self.focus = Focus::SidePanel;
                        return;
                    }
                }

                if let Some(area) = self.layout.file_tree_area {
                    if area.contains((col, row).into()) {
                        self.focus = Focus::FileTree;

                        // Check if click is on the scrollbar (rightmost column)
                        let scrollbar_x = area.x + area.width.saturating_sub(1);
                        if col == scrollbar_x {
                            self.scrollbar_dragging = Some(ScrollbarTarget::LeftPanel);
                            self.scroll_panel_to_row(ScrollbarTarget::LeftPanel, row);
                            return;
                        }

                        let relative_row = (row - area.y) as usize;

                        match self.left_panel {
                            LeftPanelMode::FileTree => {
                                self.file_tree.click_row(relative_row);
                                let is_file = self.file_tree.selected_is_file();
                                if is_file {
                                    if let Some(path) = self.file_tree.selected_path() {
                                        let path = path.to_path_buf();
                                        self.open_file(&path);
                                        self.focus = Focus::Editor;
                                    }
                                } else {
                                    self.file_tree.toggle_expand();
                                }
                            }
                            LeftPanelMode::Search => {
                                if relative_row == 0 {
                                    // Click on input row → focus input
                                    self.search_panel.editing = true;
                                    return;
                                }
                                // +2 to skip input + summary lines
                                let idx = self.search_panel.scroll.offset + relative_row.saturating_sub(2);
                                if idx < self.search_panel.results.len() {
                                    self.search_panel.scroll.selected = idx;
                                    // Open the result
                                    let result = self.search_panel.results[idx].clone();
                                    let root = self.workspace.roots().first()
                                        .map(|p| p.to_path_buf())
                                        .unwrap_or_default();
                                    let abs_path = root.join(&result.path);
                                    if abs_path.exists() {
                                        self.open_file(&abs_path);
                                        self.focus = Focus::Editor;
                                        if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                                            let target = result.line_number.saturating_sub(1);
                                            let max = buf.line_count().saturating_sub(1);
                                            buf.cursor.line = target.min(max);
                                            buf.cursor.col = 0;
                                            buf.cursor.anchor = None;
                                            buf.scroll.top_line = target.saturating_sub(10);
                                        }
                                    }
                                }
                            }
                            LeftPanelMode::Review => {
                                if let Some(ref mut br) = self.batch_review {
                                    let idx = br.scroll_offset + relative_row;
                                    if idx < br.proposals.len() {
                                        br.selected_index = idx;
                                        br.diff_scroll = 0;
                                    }
                                }
                            }
                            LeftPanelMode::Changes => {
                                if let Some(ref mut cs) = self.changes_state {
                                    let idx = cs.scroll_offset + relative_row;
                                    if idx < cs.entries.len() {
                                        cs.selected_index = idx;
                                        cs.diff_scroll = 0;
                                    }
                                }
                            }
                        }
                        return;
                    }
                }
                if self.layout.editor_area.contains((col, row).into()) {
                    self.focus = Focus::Editor;

                    // Check if click is on the scrollbar (rightmost column of editor area)
                    let scrollbar_x = self.layout.editor_area.x
                        + self.layout.editor_area.width.saturating_sub(1);
                    if col == scrollbar_x {
                        self.scrollbar_dragging = Some(ScrollbarTarget::Editor);
                        self.scroll_panel_to_row(ScrollbarTarget::Editor, row);
                        return;
                    }

                    if self.diff_review.is_none() {
                        // Detect double-click (same position within 400ms)
                        let is_double_click = self.last_click
                            .map(|(lc, lr, lt)| {
                                lc == col && lr == row && lt.elapsed().as_millis() < 400
                            })
                            .unwrap_or(false);

                        if is_double_click {
                            // Double-click: select word at cursor
                            self.last_click = None;
                            if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                                buf.select_word_at_cursor();
                            }
                        } else {
                            self.last_click = Some((col, row, std::time::Instant::now()));
                            self.set_cursor_from_mouse(col, row);
                            if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                                buf.cursor.anchor = None;
                            }
                            self.mouse_dragging = true;
                        }
                    }
                    return;
                }
                if let Some(area) = self.layout.side_panel_area {
                    if area.contains((col, row).into()) {
                        self.focus = Focus::SidePanel;

                        // Swarm dashboard: click on table row selects agent, click on detail focuses it
                        if self.side_panel == SidePanelMode::SwarmDashboard {
                            use crate::panels::swarm_dashboard::DashboardFocus;
                            let dash = &mut self.swarm_dashboard;
                            let pos = ratatui::layout::Position::new(col, row);
                            if dash.table_rect.contains(pos) {
                                dash.focus = DashboardFocus::Table;
                                let clicked_row = (row - dash.table_rect.y) as usize;
                                let idx = dash.scroll.offset + clicked_row;
                                if idx < dash.agents.len() {
                                    dash.scroll.selected = idx;
                                    dash.detail_scroll = 0;
                                    dash.detail_auto_scroll = true;
                                }
                            } else if dash.detail_rect.contains(pos) {
                                dash.focus = DashboardFocus::Detail;
                            }
                            return;
                        }

                        // Check if click is on chat conversation tabs (first row inside left border)
                        if self.side_panel == SidePanelMode::AgentChat && row == area.y {
                            let tab_area_x = area.x + 1; // skip left border
                            if let Some(idx) = self.chat_state.conv_tab_at_x(col, tab_area_x) {
                                if idx == self.chat_state.conversations.len() {
                                    // "+" button
                                    self.chat_state.new_conversation();
                                } else if idx != self.chat_state.active_conv {
                                    self.chat_state.switch_conversation(idx);
                                }
                                self.needs_full_redraw = true;
                                return;
                            }
                        }

                        // Check if click is on the scrollbar (rightmost column)
                        let scrollbar_x = area.x + area.width.saturating_sub(1);
                        if col == scrollbar_x {
                            self.scrollbar_dragging = Some(ScrollbarTarget::Chat);
                            self.scroll_panel_to_row(ScrollbarTarget::Chat, row);
                            return;
                        }

                        // Start text selection in the chat conversation area
                        if self.side_panel == SidePanelMode::AgentChat {
                            if let Some((line, ci)) = self.chat_state.screen_to_text_pos(col, row) {
                                self.chat_state.start_text_selection(line, ci);
                                self.needs_full_redraw = true;
                            } else {
                                self.chat_state.clear_text_selection();
                            }
                        }
                        return;
                    }
                }
                if let Some(area) = self.layout.terminal_area {
                    if area.contains((col, row).into()) {
                        self.focus = Focus::Terminal;
                        // Clear previous selection on new click
                        self.terminal_selection.clear();
                        // Convert absolute screen coords to vt100 screen coords.
                        // Border/tab line is at area.y; content starts at area.y + 1.
                        let content_y_start = area.y + 1;
                        if row >= content_y_start && row < area.y + area.height {
                            let vt_row = row - content_y_start;
                            let vt_col = col.saturating_sub(area.x);
                            self.terminal_selection.start(vt_row, vt_col);
                            self.needs_full_redraw = true;
                        }
                        return;
                    }
                }
                if self.layout.tab_area.contains((col, row).into()) {
                    let titles: Vec<(String, bool)> = self
                        .buffers
                        .iter()
                        .map(|b| (b.display_name().to_string(), b.modified))
                        .collect();
                    let tab_bar = TabBar {
                        titles: &titles,
                        active: self.active_buffer,
                    };
                    if let Some(idx) = tab_bar.tab_at_x(col, self.layout.tab_area.x) {
                        if idx < self.buffers.len() && idx != self.active_buffer {
                            self.active_buffer = idx;
                            self.focus = Focus::Editor;
                            self.needs_full_redraw = true;
                        }
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if let Some(area) = self.layout.file_tree_area {
                    if area.contains((col, row).into()) {
                        match self.left_panel {
                            LeftPanelMode::FileTree => self.file_tree.scroll_up(3),
                            LeftPanelMode::Search => self.search_panel.scroll.scroll_up(3),
                            LeftPanelMode::Review => {
                                if let Some(ref mut br) = self.batch_review {
                                    br.scroll_offset = br.scroll_offset.saturating_sub(3);
                                }
                            }
                            LeftPanelMode::Changes => {
                                if let Some(ref mut cs) = self.changes_state {
                                    cs.scroll_offset = cs.scroll_offset.saturating_sub(3);
                                }
                            }
                        }
                    }
                }
                if let Some(area) = self.layout.side_panel_area {
                    if area.contains((col, row).into()) {
                        match self.side_panel {
                            SidePanelMode::SwarmDashboard => {
                                let dash = &mut self.swarm_dashboard;
                                let pos = ratatui::layout::Position::new(col, row);
                                if dash.table_rect.contains(pos) {
                                    dash.scroll.scroll_up(1);
                                } else if dash.detail_rect.contains(pos) {
                                    dash.detail_auto_scroll = false;
                                    dash.detail_scroll = dash.detail_scroll.saturating_sub(3);
                                }
                            }
                            _ => {
                                self.chat_state.scroll_offset =
                                    self.chat_state.scroll_offset.saturating_sub(3);
                            }
                        }
                    }
                }
                if self.layout.editor_area.contains((col, row).into()) {
                    if let Some(ref mut br) = self.batch_review {
                        br.diff_scroll = br.diff_scroll.saturating_sub(3);
                    } else if let Some(ref mut cs) = self.changes_state {
                        if self.left_panel == LeftPanelMode::Changes {
                            cs.diff_scroll = cs.diff_scroll.saturating_sub(3);
                        }
                    } else if let Some(ref mut review) = self.diff_review {
                        review.scroll_top = review.scroll_top.saturating_sub(3);
                    } else if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                        buf.scroll.top_line = buf.scroll.top_line.saturating_sub(3);
                    }
                }
                if let Some(area) = self.layout.terminal_area {
                    if area.contains((col, row).into()) {
                        if let Some(inst) = self.terminal_manager.active_instance_mut() {
                            let current = inst.screen().scrollback();
                            inst.screen_mut().set_scrollback(current + 3);
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(area) = self.layout.file_tree_area {
                    if area.contains((col, row).into()) {
                        match self.left_panel {
                            LeftPanelMode::FileTree => self.file_tree.scroll_down(3),
                            LeftPanelMode::Search => {
                                let count = self.search_panel.results.len();
                                self.search_panel.scroll.scroll_down(3, count);
                            }
                            LeftPanelMode::Review => {
                                if let Some(ref mut br) = self.batch_review {
                                    let max = br.proposals.len().saturating_sub(1);
                                    br.scroll_offset = (br.scroll_offset + 3).min(max);
                                }
                            }
                            LeftPanelMode::Changes => {
                                if let Some(ref mut cs) = self.changes_state {
                                    let max = cs.entries.len().saturating_sub(1);
                                    cs.scroll_offset = (cs.scroll_offset + 3).min(max);
                                }
                            }
                        }
                    }
                }
                if let Some(area) = self.layout.side_panel_area {
                    if area.contains((col, row).into()) {
                        match self.side_panel {
                            SidePanelMode::SwarmDashboard => {
                                let dash = &mut self.swarm_dashboard;
                                let pos = ratatui::layout::Position::new(col, row);
                                if dash.table_rect.contains(pos) {
                                    dash.scroll.scroll_down(1, dash.agents.len());
                                } else if dash.detail_rect.contains(pos) {
                                    if let Some(agent) = dash.agents.get(dash.scroll.selected) {
                                        let w = dash.detail_rect.width.saturating_sub(1) as usize;
                                        let total = crate::panels::swarm_dashboard::count_display_lines(&agent.activity, w);
                                        dash.detail_scroll = (dash.detail_scroll + 3).min(total.saturating_sub(1));
                                    }
                                }
                            }
                            _ => {
                                self.chat_state.scroll_offset =
                                    self.chat_state.scroll_offset.saturating_add(3);
                            }
                        }
                    }
                }
                if self.layout.editor_area.contains((col, row).into()) {
                    if let Some(ref mut br) = self.batch_review {
                        br.diff_scroll += 3;
                    } else if self.left_panel == LeftPanelMode::Changes {
                        if let Some(ref mut cs) = self.changes_state {
                            cs.diff_scroll += 3;
                        }
                    } else if let Some(ref mut review) = self.diff_review {
                        review.scroll_top += 3;
                    } else if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                        let max = buf.line_count().saturating_sub(1);
                        buf.scroll.top_line = (buf.scroll.top_line + 3).min(max);
                    }
                }
                if let Some(area) = self.layout.terminal_area {
                    if area.contains((col, row).into()) {
                        if let Some(inst) = self.terminal_manager.active_instance_mut() {
                            let current = inst.screen().scrollback();
                            inst.screen_mut().set_scrollback(current.saturating_sub(3));
                        }
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(target) = self.scrollbar_dragging {
                    self.scroll_panel_to_row(target, row);
                    return;
                }
                // Chat text selection drag
                if self.chat_state.chat_dragging {
                    if let Some((line, ci)) = self.chat_state.screen_to_text_pos(col, row) {
                        self.chat_state.extend_text_selection(line, ci);
                        self.needs_full_redraw = true;
                    }
                    return;
                }
                // Terminal text selection drag
                if self.terminal_selection.dragging {
                    if let Some(area) = self.layout.terminal_area {
                        let content_y_start = area.y + 1;
                        if row >= content_y_start && row < area.y + area.height {
                            let vt_row = row - content_y_start;
                            let vt_col = col.saturating_sub(area.x);
                            self.terminal_selection.extend(vt_row, vt_col);
                            self.needs_full_redraw = true;
                        }
                    }
                    return;
                }
                if self.mouse_dragging && self.layout.editor_area.contains((col, row).into()) {
                    if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                        // Set anchor on first drag if not set
                        if buf.cursor.anchor.is_none() {
                            buf.cursor.anchor = Some((buf.cursor.line, buf.cursor.col));
                        }
                    }
                    // Move cursor to drag position (extends selection via anchor)
                    self.set_cursor_from_mouse(col, row);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // Copy terminal text selection to clipboard on mouse release
                if self.terminal_selection.dragging {
                    if let Some(inst) = self.terminal_manager.active_instance() {
                        if let Some(text) = self.terminal_selection.extract_text(inst.screen()) {
                            self.set_clipboard(&text);
                        }
                    }
                    self.terminal_selection.dragging = false;
                    // Keep selection highlight visible until next click
                }
                // Copy chat text selection to clipboard on mouse release
                if self.chat_state.chat_dragging {
                    if let Some(text) = self.chat_state.selected_chat_text() {
                        self.set_clipboard(&text);
                    }
                    self.chat_state.chat_dragging = false;
                    // Keep selection highlight visible until next click
                }
                self.mouse_dragging = false;
                self.scrollbar_dragging = None;
            }
            _ => {}
        }
    }

    /// Scroll a panel based on a mouse row within its scrollbar track.
    fn scroll_panel_to_row(&mut self, target: ScrollbarTarget, row: u16) {
        match target {
            ScrollbarTarget::Editor => {
                let area = self.layout.editor_area;
                let Some(buf) = self.buffers.get_mut(self.active_buffer) else { return };
                let track_height = area.height as usize;
                if track_height == 0 { return; }
                let total = buf.line_count();
                if total <= track_height { return; }
                let max_scroll = total.saturating_sub(track_height);
                let row_in_track = row.saturating_sub(area.y) as usize;
                let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                buf.scroll.top_line = (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
            }
            ScrollbarTarget::Chat => {
                let Some(area) = self.layout.side_panel_area else { return };
                // The conversation area is the side panel minus header (1 row) and input (3 rows)
                let conv_height = area.height.saturating_sub(4) as usize;
                if conv_height == 0 { return; }
                // Estimate total lines from current scroll state
                // The chat auto-scrolls, so we use scroll_offset + viewport as an estimate
                let total = self.chat_state.scroll_offset + conv_height + 10;
                if total <= conv_height { return; }
                let max_scroll = total.saturating_sub(conv_height);
                let row_in_track = row.saturating_sub(area.y + 1) as usize; // +1 for header
                let fraction = row_in_track as f64 / conv_height.saturating_sub(1).max(1) as f64;
                self.chat_state.scroll_offset = (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
            }
            ScrollbarTarget::LeftPanel => {
                let Some(area) = self.layout.file_tree_area else { return };
                let track_height = area.height as usize;
                if track_height == 0 { return; }
                match self.left_panel {
                    LeftPanelMode::FileTree => {
                        let total = self.file_tree.entries.len();
                        if total <= track_height { return; }
                        let max_scroll = total.saturating_sub(track_height);
                        let row_in_track = row.saturating_sub(area.y) as usize;
                        let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                        self.file_tree.scroll.offset = (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                    }
                    LeftPanelMode::Search => {
                        let total = self.search_panel.results.len();
                        let viewport = track_height.saturating_sub(2); // input + summary rows
                        if total <= viewport { return; }
                        let max_scroll = total.saturating_sub(viewport);
                        let row_in_track = row.saturating_sub(area.y) as usize;
                        let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                        self.search_panel.scroll.offset = (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                    }
                    LeftPanelMode::Review => {
                        if let Some(ref mut br) = self.batch_review {
                            let total = br.proposals.len();
                            if total <= track_height { return; }
                            let max_scroll = total.saturating_sub(track_height);
                            let row_in_track = row.saturating_sub(area.y) as usize;
                            let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                            br.scroll_offset = (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                        }
                    }
                    LeftPanelMode::Changes => {
                        if let Some(ref mut cs) = self.changes_state {
                            let total = cs.entries.len();
                            if total <= track_height { return; }
                            let max_scroll = total.saturating_sub(track_height);
                            let row_in_track = row.saturating_sub(area.y) as usize;
                            let fraction = row_in_track as f64 / track_height.saturating_sub(1).max(1) as f64;
                            cs.scroll_offset = (fraction * max_scroll as f64).round().min(max_scroll as f64) as usize;
                        }
                    }
                }
            }
        }
    }

    /// Convert mouse (col, row) to buffer cursor position.
    fn set_cursor_from_mouse(&mut self, col: u16, row: u16) {
        let Some(buf) = self.buffers.get_mut(self.active_buffer) else { return };
        let area = self.layout.editor_area;
        let gutter_w = gutter_width(buf.line_count());
        if col >= area.x + gutter_w {
            let visual_col = (col - area.x - gutter_w) as usize + buf.scroll.left_col;
            let click_line = (row - area.y) as usize + buf.scroll.top_line;
            let max_line = buf.line_count().saturating_sub(1);
            buf.cursor.line = click_line.min(max_line);
            let char_col = buf.visual_to_char_col(buf.cursor.line, visual_col);
            let line_len = buf.line_len(buf.cursor.line);
            buf.cursor.col = char_col.min(line_len);
        }
    }

    /// Handle a bracketed paste event from the terminal.
    fn handle_paste(&mut self, text: &str) {
        if self.diff_review.is_some() {
            return;
        }
        match self.focus {
            Focus::Editor => {
                if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                    buf.paste_text(text);
                    let area = self.layout.editor_area;
                    let line_count = buf.line_count();
                    let gutter_w = gutter_width(line_count) as usize;
                    let vp_h = area.height as usize;
                    let vp_w = (area.width as usize).saturating_sub(gutter_w);
                    buf.ensure_cursor_visible(vp_h, vp_w);
                }
            }
            Focus::SidePanel => {
                // Paste into chat input (preserves newlines)
                self.chat_state.insert_str(text);
            }
            _ => {}
        }
    }

    /// Search for the selected text (or word at cursor) across the workspace.
    /// If results already exist for the same query, cycles to the next result.
    fn search_selected_in_workspace(&mut self) {
        // Get the query from selection or word at cursor
        let query = if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
            let selected = buf.selected_text();
            if selected.is_empty() {
                buf.select_word_at_cursor()
            } else {
                selected
            }
        } else {
            String::new()
        };

        // If we have existing results and query matches, cycle to next
        if !self.search_panel.results.is_empty()
            && (query.trim().is_empty() || query.trim() == self.search_panel.query)
        {
            self.goto_next_search_result();
            return;
        }

        if query.trim().is_empty() {
            return;
        }

        // Populate the input field with the query
        self.search_panel.input.clear();
        self.search_panel.input.insert_str(&query);
        self.search_panel.editing = false; // focus results for navigation

        let roots = self.workspace.roots();
        let excludes: Vec<String> = self.file_tree.exclude_patterns.clone();
        self.search_panel.search(&query, &roots, &excludes);

        // Switch left panel to search
        self.left_panel = LeftPanelMode::Search;
        if !self.panel_visible.file_tree {
            self.panel_visible.file_tree = true;
        }

        let count = self.search_panel.results.len();
        if count > 0 {
            self.goto_next_search_result();
        }
        self.status_message = Some((
            format!("Found {} results for '{}'", count, query),
            std::time::Instant::now(),
        ));
    }

    /// Navigate to the next search result, opening the file and jumping to the line.
    fn goto_next_search_result(&mut self) {
        if self.search_panel.results.is_empty() {
            return;
        }

        // Advance to next result (wrap around)
        let count = self.search_panel.results.len();
        self.search_panel.scroll.selected = (self.search_panel.scroll.selected + 1) % count;
        self.search_panel.scroll.ensure_visible();

        let result = self.search_panel.results[self.search_panel.scroll.selected].clone();
        let root = self.workspace.roots().first()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        let abs_path = root.join(&result.path);

        if abs_path.exists() {
            self.open_file(&abs_path);
            self.focus = Focus::Editor;
            if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
                let target = result.line_number.saturating_sub(1);
                let max = buf.line_count().saturating_sub(1);
                buf.cursor.line = target.min(max);
                buf.cursor.col = 0;
                buf.cursor.anchor = None;
                buf.scroll.top_line = target.saturating_sub(10);
                // Set search highlight so the editor shows all matches
                buf.set_search_highlight(Some(self.search_panel.query.clone()));
            }
        }

        let idx = self.search_panel.scroll.selected + 1;
        let total = self.search_panel.results.len();
        self.status_message = Some((
            format!("Result {}/{}: {}:{}", idx, total, result.path.display(), result.line_number),
            std::time::Instant::now(),
        ));
    }

    fn handle_file_changed(&mut self, path: &Path) {
        if self.diff_review.is_some() {
            return;
        }

        let buf_idx = self.buffers.iter().position(|b| {
            b.path.as_deref() == Some(path) && !b.modified
        });
        let Some(buf_idx) = buf_idx else { return };

        let new_content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };

        let old_content = self.buffers[buf_idx].text.to_string();
        if old_content == new_content {
            return;
        }

        let proposal = WriteGatePipeline::build_proposal(
            0, "external", path, &old_content, &new_content,
        );

        // Reload buffer to show new content
        let _ = self.buffers[buf_idx].reload();

        // Show diff overlay (read-only)
        self.active_buffer = buf_idx;
        self.focus = Focus::Editor;
        self.diff_review = Some(DiffReviewState::new(proposal, DiffSource::External));
    }

    /// Open a file in a new buffer (or switch to existing).
    pub fn open_file(&mut self, path: &Path) {
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.path.as_deref() == Some(path) {
                self.active_buffer = i;
                self.needs_full_redraw = true;
                return;
            }
        }

        match Buffer::open(path) {
            Ok(mut buf) => {
                // Apply indent settings from workspace
                use gaviero_core::workspace::settings;
                let lang = buf.lang_name.as_deref();
                let tab_size = if let Some(lang) = lang {
                    self.workspace.resolve_language_setting(settings::TAB_SIZE, lang, None)
                } else {
                    self.workspace.resolve_setting(settings::TAB_SIZE, None)
                };
                let insert_spaces = if let Some(lang) = lang {
                    self.workspace.resolve_language_setting(settings::INSERT_SPACES, lang, None)
                } else {
                    self.workspace.resolve_setting(settings::INSERT_SPACES, None)
                };
                let tw = tab_size.as_u64().unwrap_or(4) as u8;
                buf.tab_width = tw;
                buf.indent_unit = if insert_spaces.as_bool().unwrap_or(true) {
                    " ".repeat(tw as usize)
                } else {
                    "\t".to_string()
                };

                if let (Some(lang_name), Some(language)) = (&buf.lang_name, &buf.language) {
                    if !self.highlight_configs.contains_key(lang_name) {
                        match load_highlight_config(language.clone(), lang_name) {
                            Ok(config) => {
                                tracing::info!("Loaded highlight config for {}", lang_name);
                                self.highlight_configs.insert(lang_name.clone(), config);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to load highlights for {}: {}",
                                    lang_name,
                                    e
                                );
                            }
                        }
                    }
                    // Load indent query for this language
                    buf.indent_query = self.indent_query_cache.get_or_load(lang_name, language);
                }
                self.buffers.push(buf);
                self.active_buffer = self.buffers.len() - 1;
                self.needs_full_redraw = true;
            }
            Err(e) => {
                tracing::error!("Failed to open file {}: {}", path.display(), e);
            }
        }
    }

    fn cycle_tab(&mut self, delta: i32) {
        if self.buffers.is_empty() {
            return;
        }
        let len = self.buffers.len() as i32;
        self.active_buffer = ((self.active_buffer as i32 + delta).rem_euclid(len)) as usize;
        self.needs_full_redraw = true;
    }

    fn close_tab(&mut self) {
        if self.buffers.is_empty() {
            return;
        }
        self.buffers.remove(self.active_buffer);
        if self.active_buffer >= self.buffers.len() && !self.buffers.is_empty() {
            self.active_buffer = self.buffers.len() - 1;
        }
    }

    fn spawn_active_terminal(&mut self) {
        // If no tabs exist, create one (lazy)
        if self.terminal_manager.is_empty() {
            let root = self.workspace.roots().first()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            self.terminal_manager.create_tab_lazy(&root);
        }
        // Set viewport dimensions from layout
        let rows = self.layout.terminal_area.map(|a| a.height.saturating_sub(2)).unwrap_or(24).max(2);
        let cols = self.layout.terminal_area.map(|a| a.width).unwrap_or(80).max(10);
        self.terminal_manager.handle_resize(rows, cols);
        // Ensure the active tab is spawned
        if let Err(e) = self.terminal_manager.ensure_active_spawned() {
            self.status_message = Some((format!("Terminal: {}", e), std::time::Instant::now()));
        }
    }

    fn save_current_buffer(&mut self) {
        if let Some(buf) = self.buffers.get_mut(self.active_buffer) {
            if let Err(e) = buf.save() {
                tracing::error!("Save failed: {}", e);
            }
        }
    }

    fn is_current_buffer_markdown(&self) -> bool {
        self.buffers
            .get(self.active_buffer)
            .and_then(|b| b.lang_name.as_deref())
            == Some("markdown")
    }

    // ── Rendering ────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame) {
        let size = frame.area();

        let main_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(1),
            ])
            .split(size);

        let tab_area = main_layout[0];
        let main_area = main_layout[1];
        let status_area = main_layout[2];

        self.layout.tab_area = tab_area;
        self.layout.status_area = status_area;

        self.render_tab_bar(frame, tab_area);

        // Fullscreen mode: render only the focused panel in the entire main area
        if let Some(fs_panel) = self.fullscreen_panel {
            self.render_fullscreen(frame, main_area, fs_panel);
            self.render_status_bar(frame, status_area);
            if fs_panel == Focus::Editor {
                self.update_cursor_position(frame, self.layout.editor_area);
            }
            return;
        }

        // If terminal is visible, split main area vertically first:
        // top = panels (file tree + editor + side panel), bottom = terminal (full width)
        self.layout.terminal_area = None;
        let (panels_area, terminal_area) = if self.panel_visible.terminal {
            let term_pct = self.terminal_split_percent;
            let v_split = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(100 - term_pct),
                    Constraint::Percentage(term_pct),
                ])
                .split(main_area);
            self.layout.terminal_area = Some(v_split[1]);
            (v_split[0], Some(v_split[1]))
        } else {
            (main_area, None)
        };

        let (eff_ft_w, eff_sp_w) = self.effective_panel_constraints(panels_area.width);

        let mut constraints = Vec::new();
        if self.panel_visible.file_tree {
            constraints.push(Constraint::Length(eff_ft_w));
        }
        constraints.push(Constraint::Min(20));
        if self.panel_visible.side_panel {
            constraints.push(Constraint::Length(eff_sp_w));
        }

        let h_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(panels_area);

        let mut panel_idx = 0;

        self.layout.file_tree_area = None;
        self.layout.left_header_area = None;
        if self.panel_visible.file_tree {
            let full_area = h_layout[panel_idx];
            let left_focused = self.focus == Focus::FileTree;
            let title = match self.left_panel {
                LeftPanelMode::FileTree => "EXPLORER",
                LeftPanelMode::Search => "SEARCH",
                LeftPanelMode::Review => "REVIEW",
                LeftPanelMode::Changes => "CHANGES",
            };
            let (header_area, content_area) = Self::render_panel_header(frame, full_area, title, left_focused, true);
            self.layout.left_header_area = Some(header_area);
            self.layout.file_tree_area = Some(content_area);

            match self.left_panel {
                LeftPanelMode::FileTree => {
                    self.file_tree.render(content_area, frame.buffer_mut(), left_focused);
                    if let Some(ref dialog) = self.tree_dialog {
                        self.render_tree_dialog(frame, content_area, dialog);
                    }
                }
                LeftPanelMode::Search => {
                    self.search_panel.render(content_area, frame.buffer_mut(), left_focused);
                }
                LeftPanelMode::Review => {
                    self.render_review_file_list(frame, content_area, left_focused);
                }
                LeftPanelMode::Changes => {
                    self.render_changes_file_list(frame, content_area, left_focused);
                }
            }

            panel_idx += 1;
        }

        let editor_full_area = h_layout[panel_idx];

        // Editor panel header
        let editor_focused = self.focus == Focus::Editor;
        let editor_title = self.buffers.get(self.active_buffer)
            .map(|b| b.display_name().to_string())
            .unwrap_or_else(|| "EDITOR".to_string());
        let (_editor_header, editor_content) = Self::render_panel_header(frame, editor_full_area, &editor_title, editor_focused, false);

        // If markdown preview is visible, split editor area horizontally
        let (actual_editor_area, preview_area) = if self.preview_visible && self.is_current_buffer_markdown() {
            let split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(editor_content);
            (split[0], Some(split[1]))
        } else {
            (editor_content, None)
        };

        self.layout.editor_area = actual_editor_area;
        self.render_editor(frame, actual_editor_area);

        // Render markdown preview if visible
        if let Some(parea) = preview_area {
            self.render_markdown_preview(frame, parea);
        }
        panel_idx += 1;

        self.layout.side_panel_area = None;
        self.layout.side_header_area = None;
        if self.panel_visible.side_panel && panel_idx < h_layout.len() {
            let full_area = h_layout[panel_idx];
            let side_focused = self.focus == Focus::SidePanel;
            let side_title = match self.side_panel {
                SidePanelMode::AgentChat => "AGENT CHAT",
                SidePanelMode::SwarmDashboard => "SWARM",
                SidePanelMode::GitPanel => "GIT",
            };
            let (side_header, content_area) = Self::render_panel_header(frame, full_area, side_title, side_focused, true);
            self.layout.side_header_area = Some(side_header);
            self.layout.side_panel_area = Some(content_area);
            self.render_side_panel(frame, content_area);
        }

        // Render terminal (full width, below all panels)
        if let Some(term_area) = terminal_area {
            self.render_terminal(frame, term_area);
        }

        self.render_status_bar(frame, status_area);
        self.update_cursor_position(frame, self.layout.editor_area);
    }

    fn render_fullscreen(&mut self, frame: &mut Frame, area: Rect, panel: Focus) {
        let title = match panel {
            Focus::FileTree => "EXPLORER (fullscreen)",
            Focus::Editor => {
                // Use buffer name — we'll set it after the match
                ""
            }
            Focus::SidePanel => "AGENT CHAT (fullscreen)",
            Focus::Terminal => "TERMINAL (fullscreen)",
        };

        let title = if panel == Focus::Editor {
            self.buffers.get(self.active_buffer)
                .map(|b| format!("{} (fullscreen)", b.display_name()))
                .unwrap_or_else(|| "EDITOR (fullscreen)".to_string())
        } else {
            title.to_string()
        };

        let (_header, content) = Self::render_panel_header(frame, area, &title, true, false);

        match panel {
            Focus::FileTree => {
                self.layout.file_tree_area = Some(content);
                match self.left_panel {
                    LeftPanelMode::FileTree => {
                        self.file_tree.render(content, frame.buffer_mut(), true);
                        if let Some(ref dialog) = self.tree_dialog {
                            self.render_tree_dialog(frame, content, dialog);
                        }
                    }
                    LeftPanelMode::Search => {
                        self.search_panel.render(content, frame.buffer_mut(), true);
                    }
                    LeftPanelMode::Review => {
                        self.render_review_file_list(frame, content, true);
                    }
                    LeftPanelMode::Changes => {
                        self.render_changes_file_list(frame, content, true);
                    }
                }
            }
            Focus::Editor => {
                self.layout.editor_area = content;
                self.render_editor(frame, content);
            }
            Focus::SidePanel => {
                self.layout.side_panel_area = Some(content);
                self.render_side_panel(frame, content);
            }
            Focus::Terminal => {
                self.render_terminal(frame,content);
            }
        }
    }

    fn render_tab_bar(&self, frame: &mut Frame, area: Rect) {
        let titles: Vec<(String, bool)> = self
            .buffers
            .iter()
            .map(|b| (b.display_name().to_string(), b.modified))
            .collect();
        let tab_bar = TabBar {
            titles: &titles,
            active: self.active_buffer,
        };
        tab_bar.render(area, frame.buffer_mut());
    }

    fn render_editor(&mut self, frame: &mut Frame, area: Rect) {
        // If in batch review mode, render the batch diff viewer
        if self.batch_review.is_some() {
            self.render_batch_review_diff(frame, area);
            return;
        }

        // If in Changes mode, render the git diff viewer
        if self.left_panel == LeftPanelMode::Changes && self.changes_state.is_some() {
            self.render_changes_diff(frame, area);
            return;
        }

        // If in diff review mode, render the overlay instead of the normal editor
        if let Some(ref mut review) = self.diff_review {
            diff_overlay::render_diff_overlay(
                area,
                frame.buffer_mut(),
                review,
                &self.theme,
            );
            return;
        }

        if self.buffers.is_empty() {
            let msg = " Press Ctrl+\\ to focus file tree, then Enter to open a file";
            let style = self.theme.default_style();
            let y = area.y + area.height / 2;
            if y < area.bottom() {
                for (i, ch) in msg.chars().enumerate() {
                    let x = area.x + i as u16;
                    if x < area.right() {
                        frame.buffer_mut()[(x, y)].set_char(ch).set_style(style);
                    }
                }
            }
            return;
        }

        // ── Find bar ─────────────────────────────────────────
        let editor_area = if self.find_bar_active {
            self.render_find_bar(frame, area);
            // Shrink editor area to make room for the find bar at top
            Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: area.height.saturating_sub(1),
            }
        } else {
            area
        };

        let buf = &self.buffers[self.active_buffer];
        let highlight_config = buf
            .lang_name
            .as_ref()
            .and_then(|name| self.highlight_configs.get(name));

        let view = EditorView::new(buf, &self.theme, highlight_config, self.focus == Focus::Editor);
        view.render(editor_area, frame.buffer_mut());
    }

    /// Render the find bar at the top of the editor area.
    fn render_find_bar(&self, frame: &mut Frame, area: Rect) {
        let bar_y = area.y;
        let buf = frame.buffer_mut();
        let bg = theme::INPUT_BG;
        let fg = theme::TEXT_FG;
        let label_fg = theme::FOCUS_BORDER;

        // Clear the bar row
        for x in area.x..area.right() {
            if bar_y < buf.area().bottom() {
                buf[(x, bar_y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        // Label: " Find: "
        let label = " Find: ";
        let label_style = Style::default().fg(label_fg).bg(bg).add_modifier(Modifier::BOLD);
        let mut x = area.x;
        for ch in label.chars() {
            if x < area.right() && bar_y < buf.area().bottom() {
                buf[(x, bar_y)].set_char(ch).set_style(label_style);
            }
            x += 1;
        }
        let text_start = x;

        // Input text
        let input_style = Style::default().fg(fg).bg(bg);
        for ch in self.find_input.text.chars() {
            if x >= area.right() { break; }
            if bar_y < buf.area().bottom() {
                buf[(x, bar_y)].set_char(ch).set_style(input_style);
            }
            x += 1;
        }

        // Cursor
        if self.find_bar_active {
            let cursor_x = text_start + self.find_input.cursor as u16;
            if cursor_x < area.right() && bar_y < buf.area().bottom() {
                let cursor_style = Style::default().fg(bg).bg(fg);
                buf[(cursor_x, bar_y)].set_style(cursor_style);
            }
        }

        // Match count indicator (right-aligned)
        if let Some(editor_buf) = self.buffers.get(self.active_buffer) {
            let total = editor_buf.search_match_count();
            if total > 0 {
                let current = editor_buf.current_match_index();
                let indicator = format!(" {}/{} ", current, total);
                let ind_style = Style::default().fg(theme::TEXT_DIM).bg(bg);
                let ind_start = area.right().saturating_sub(indicator.len() as u16);
                for (i, ch) in indicator.chars().enumerate() {
                    let ix = ind_start + i as u16;
                    if ix < area.right() && bar_y < buf.area().bottom() {
                        buf[(ix, bar_y)].set_char(ch).set_style(ind_style);
                    }
                }
            } else if !self.find_input.is_empty() {
                let indicator = " No matches ";
                let ind_style = Style::default().fg(theme::ERROR).bg(bg);
                let ind_start = area.right().saturating_sub(indicator.len() as u16);
                for (i, ch) in indicator.chars().enumerate() {
                    let ix = ind_start + i as u16;
                    if ix < area.right() && bar_y < buf.area().bottom() {
                        buf[(ix, bar_y)].set_char(ch).set_style(ind_style);
                    }
                }
            }
        }
    }

    fn render_tree_dialog(&self, frame: &mut Frame, tree_area: Rect, dialog: &TreeDialog) {
        // Render a 2-line dialog at the bottom of the file tree panel
        let dialog_height: u16 = 2;
        if tree_area.height < dialog_height + 2 {
            return;
        }

        let y = tree_area.bottom() - dialog_height;
        let dialog_area = Rect {
            x: tree_area.x,
            y,
            width: tree_area.width.saturating_sub(1), // exclude right border
            height: dialog_height,
        };

        // Background
        let bg_style = Style::default()
            .fg(theme::TEXT_BRIGHT)
            .bg(theme::INPUT_BG);

        // Clear dialog area
        for row in 0..dialog_area.height {
            for col in 0..dialog_area.width {
                let cx = dialog_area.x + col;
                let cy = dialog_area.y + row;
                if cx < frame.area().right() && cy < frame.area().bottom() {
                    frame.buffer_mut()[(cx, cy)].set_char(' ').set_style(bg_style);
                }
            }
        }

        // Top line: separator
        let sep_style = Style::default()
            .fg(theme::FOCUS_BORDER)
            .bg(theme::INPUT_BG);
        for col in 0..dialog_area.width {
            let cx = dialog_area.x + col;
            if cx < frame.area().right() {
                frame.buffer_mut()[(cx, y)].set_char('─').set_style(sep_style);
            }
        }

        // Bottom line: prompt + input
        let input_y = y + 1;
        let prompt = dialog.prompt();
        let prompt_style = Style::default()
            .fg(theme::FOCUS_BORDER)
            .bg(theme::INPUT_BG);

        let mut x = dialog_area.x;
        for ch in prompt.chars() {
            if x < dialog_area.x + dialog_area.width {
                frame.buffer_mut()[(x, input_y)].set_char(ch).set_style(prompt_style);
                x += 1;
            }
        }

        // Input text (or static filename for delete confirmation)
        let input_start_x = x;
        let is_delete = matches!(dialog.kind, TreeDialogKind::Delete);
        let display_text = if is_delete {
            // Show the filename as non-editable text
            dialog.original_path.as_ref()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or(&dialog.input)
        } else {
            &dialog.input
        };
        for ch in display_text.chars() {
            if x < dialog_area.x + dialog_area.width {
                frame.buffer_mut()[(x, input_y)].set_char(ch).set_style(bg_style);
                x += 1;
            }
        }

        // Position cursor in dialog (not for delete confirmation)
        if !is_delete {
            let cursor_x = input_start_x + dialog.input[..dialog.cursor].chars().count() as u16;
            if cursor_x < dialog_area.x + dialog_area.width && input_y < frame.area().bottom() {
                frame.set_cursor_position((cursor_x, input_y));
            }
        }
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        // Diff review mode — unified for ACP and external sources
        if let Some(ref review) = self.diff_review {
            let proposal = &review.proposal;
            let total = proposal.structural_hunks.len();
            let filename = proposal
                .file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("?");

            let (status_text, style) = match review.source {
                DiffSource::External => {
                    let current = review.current_hunk + 1;
                    (
                        format!(
                            " CHANGED  {} modified externally [{}/{}]  |  ]h/[h: navigate  q: dismiss",
                            filename, current, total,
                        ),
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme::FOCUS_BORDER),
                    )
                }
                DiffSource::Acp => {
                    let accepted = proposal
                        .structural_hunks
                        .iter()
                        .filter(|h| h.status == gaviero_core::types::HunkStatus::Accepted)
                        .count();
                    (
                        format!(
                            " REVIEW  {} [{}/{} accepted] from {}  |  a/r: hunk  A/R: all  ]h/[h: nav  f: finalize  q: dismiss",
                            filename, accepted, total, proposal.source
                        ),
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme::WARNING),
                    )
                }
            };

            for (i, ch) in status_text.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, area.y)]
                        .set_char(ch)
                        .set_style(style);
                }
            }
            for x in (area.x + status_text.len() as u16)..area.right() {
                frame.buffer_mut()[(x, area.y)].set_style(style);
            }
            return;
        }

        // Batch review mode status bar
        if let Some(ref br) = self.batch_review {
            let n = br.proposals.len();
            let current = br.selected_index + 1;
            let status_text = format!(
                " REVIEW ({} files)  [{}/{}]  |  a: accept  r: reject  f: apply all  Esc: discard  Ctrl+←→: panel",
                n, current, n,
            );
            let style = Style::default()
                .fg(Color::Black)
                .bg(theme::WARNING);
            for (i, ch) in status_text.chars().enumerate() {
                let x = area.x + i as u16;
                if x < area.right() {
                    frame.buffer_mut()[(x, area.y)]
                        .set_char(ch)
                        .set_style(style);
                }
            }
            for x in (area.x + status_text.len() as u16)..area.right() {
                frame.buffer_mut()[(x, area.y)].set_style(style);
            }
            return;
        }

        let focus_label = match self.focus {
            Focus::Editor => "EDIT",
            Focus::FileTree => match self.left_panel {
                LeftPanelMode::FileTree => "TREE",
                LeftPanelMode::Search => "FIND",
                LeftPanelMode::Review => "REVIEW",
                LeftPanelMode::Changes => "CHANGES",
            },
            Focus::SidePanel => "CHAT",
            Focus::Terminal => "TERM",
        };

        // Build model info string with context percentage
        let model = self.chat_state.effective_model().to_string();
        let effort = self.chat_state.effective_effort();
        let (_chars, ctx_pct) = self.chat_state.estimate_context();
        let model_info = format!("{}|{} ctx:{}%", model, effort, ctx_pct);

        // Show context-relevant info based on focused panel
        let current_buffer = if self.focus == Focus::Editor {
            self.buffers.get(self.active_buffer)
        } else {
            None
        };

        // Show transient status message if recent (< 3 seconds)
        let transient_msg = self.status_message.as_ref().and_then(|(msg, when)| {
            if when.elapsed().as_secs() < 3 { Some(msg.as_str()) } else { None }
        });

        let context_info = if let Some(msg) = transient_msg {
            msg.to_string()
        } else {
            match self.focus {
                Focus::FileTree => match self.left_panel {
                    LeftPanelMode::FileTree => "n: new file  N: new folder  r: rename  d: delete  Enter: open  F7: search".to_string(),
                    LeftPanelMode::Review => {
                        let n = self.batch_review.as_ref().map(|br| br.proposals.len()).unwrap_or(0);
                        format!("REVIEW ({} files)  f: apply all  Esc: discard  ↑↓: navigate", n)
                    }
                    LeftPanelMode::Search => {
                        if self.search_panel.editing {
                            "Type to search  ↓/Enter: results  Esc: clear/back".to_string()
                        } else {
                            let count = self.search_panel.results.len();
                            format!("{} results  Enter: open  ↑: input  Esc: input  F7: cycle", count)
                        }
                    }
                    LeftPanelMode::Changes => {
                        let n = self.changes_state.as_ref().map(|cs| cs.entries.len()).unwrap_or(0);
                        format!("{} changed files  ↑↓: navigate  Enter: open  R: refresh  Esc: back  F7: cycle", n)
                    }
                },
                Focus::SidePanel => {
                    let conv_count = self.chat_state.conversations.len();
                    let conv_idx = self.chat_state.active_conv + 1;
                    format!("Chat {}/{}  F2: rename  Ctrl+T: new  /help: commands", conv_idx, conv_count)
                }
                Focus::Terminal => "Terminal (M4)".to_string(),
                Focus::Editor => String::new(), // buffer info used instead
            }
        };

        let status = StatusBar {
            buffer: current_buffer,
            theme: &self.theme,
            focus_label,
            model_info: &model_info,
            context_info: &context_info,
        };
        status.render(area, frame.buffer_mut());
    }

    /// Render a 1-line panel header. Returns `(header_area, content_area)`.
    /// Active panel gets a bright colored bar; inactive gets a subtle dark bar.
    /// When `show_cycle_arrow` is true, a clickable "▸" arrow is drawn on the
    /// right side of the header to allow cycling through sub-panels.
    fn render_panel_header(
        frame: &mut Frame,
        area: Rect,
        title: &str,
        focused: bool,
        show_cycle_arrow: bool,
    ) -> (Rect, Rect) {
        if area.height < 2 {
            return (Rect::default(), area);
        }

        let header_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        let content_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };

        let (bg, fg) = if focused {
            (theme::FOCUSED_SELECTION_BG, theme::PANEL_HEADER_FOCUSED_FG)
        } else {
            (theme::CODE_BLOCK_BG, theme::PANEL_HEADER_UNFOCUSED_FG)
        };
        let style = Style::default().fg(fg).bg(bg);

        let buf = frame.buffer_mut();
        // Fill header background
        for col in 0..header_area.width {
            let cx = header_area.x + col;
            if cx < buf.area().right() && header_area.y < buf.area().bottom() {
                buf[(cx, header_area.y)].set_char(' ').set_style(style);
            }
        }

        // Write title (left-aligned with padding)
        let padded = format!(" {}", title);
        for (i, ch) in padded.chars().enumerate() {
            let cx = header_area.x + i as u16;
            if cx < header_area.x + header_area.width && cx < buf.area().right() {
                buf[(cx, header_area.y)].set_char(ch).set_style(style);
            }
        }

        // Draw cycle arrow on the right side
        if show_cycle_arrow && header_area.width >= 4 {
            let arrow_x = header_area.x + header_area.width - 2;
            if arrow_x < buf.area().right() && header_area.y < buf.area().bottom() {
                let arrow_style = Style::default()
                    .fg(if focused { theme::PANEL_HEADER_FOCUSED_FG } else { theme::PANEL_HEADER_UNFOCUSED_FG })
                    .bg(bg);
                buf[(arrow_x, header_area.y)].set_char('▸').set_style(arrow_style);
            }
        }

        (header_area, content_area)
    }

    fn render_terminal(&mut self, frame: &mut Frame, area: Rect) {
        if area.height < 2 {
            return;
        }

        // Lazy-spawn: if the active terminal hasn't been spawned yet, do it now
        let needs_spawn = self.terminal_manager.active_instance().map_or(true, |i| !i.spawned);
        if needs_spawn {
            self.spawn_active_terminal();
        }

        let focused = self.focus == Focus::Terminal;
        let tab_count = self.terminal_manager.tab_count();
        let active_idx = self.terminal_manager.active_tab_index();

        // Render tab bar in the border line if multiple terminals
        if tab_count > 1 {
            let buf = frame.buffer_mut();
            let border_fg = if focused {
                theme::FOCUS_BORDER
            } else {
                theme::BORDER_DIM
            };
            let border_style = Style::default().fg(border_fg);
            let active_style = Style::default()
                .fg(theme::NUMERIC_ORANGE)
                .add_modifier(ratatui::style::Modifier::BOLD);

            // Draw "─" across the entire top line first
            for col in 0..area.width {
                let cx = area.x + col;
                if cx < buf.area().right() && area.y < buf.area().bottom() {
                    buf[(cx, area.y)].set_char('─').set_style(border_style);
                }
            }

            // Draw terminal tabs
            let mut x = area.x + 1;
            for i in 0..tab_count {
                let label = format!(" Term {} ", i + 1);
                let style = if i == active_idx { active_style } else { border_style };
                for ch in label.chars() {
                    if x < area.x + area.width && x < buf.area().right() && area.y < buf.area().bottom() {
                        buf[(x, area.y)].set_char(ch).set_style(style);
                    }
                    x += 1;
                }
                // Separator
                if x < area.x + area.width && x < buf.area().right() {
                    buf[(x, area.y)].set_char('│').set_style(border_style);
                }
                x += 1;
            }
        }

        // Resize and render the active terminal
        let content_rows = area.height.saturating_sub(1);
        let selection = self.terminal_selection.clone();
        if let Some(inst) = self.terminal_manager.active_instance_mut() {
            inst.resize(content_rows, area.width);
            let screen = inst.screen();
            if tab_count > 1 {
                let content_area = Rect {
                    x: area.x,
                    y: area.y + 1,
                    width: area.width,
                    height: content_rows,
                };
                crate::panels::terminal::render_terminal_screen(screen, content_area, frame.buffer_mut(), focused, &selection);
            } else {
                crate::panels::terminal::render_terminal_with_border(screen, area, frame.buffer_mut(), focused, &selection);
            }
        }
    }

    fn render_markdown_preview(&self, frame: &mut Frame, area: Rect) {
        use ratatui::widgets::{Block, Borders};
        use crate::editor::markdown;

        // Draw border
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(theme::BORDER_DIM))
            .title(" Preview (Ctrl+M) ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(buf) = self.buffers.get(self.active_buffer) {
            let source = buf.text.to_string();
            markdown::render_markdown_preview(
                &source,
                inner,
                frame.buffer_mut(),
                &self.theme,
                self.preview_scroll,
            );
        }
    }

    fn render_side_panel(&mut self, frame: &mut Frame, area: Rect) {
        match self.side_panel {
            SidePanelMode::AgentChat => {
                self.chat_state.render(
                    area,
                    frame.buffer_mut(),
                    self.focus == Focus::SidePanel,
                    &self.theme,
                );
            }
            SidePanelMode::SwarmDashboard => {
                self.swarm_dashboard.render(
                    area,
                    frame.buffer_mut(),
                    self.focus == Focus::SidePanel,
                );
            }
            SidePanelMode::GitPanel => {
                self.git_panel.render(
                    area,
                    frame.buffer_mut(),
                    self.focus == Focus::SidePanel,
                    &self.theme,
                );
            }
        }
    }

    // ── Session state persistence ─────────────────────────────────

    fn workspace_key(&self) -> std::path::PathBuf {
        self.workspace
            .roots()
            .first()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    }

    /// Restore session state (open tabs, panel visibility, tree state).
    /// Call after `new()`.
    pub fn restore_session(&mut self) {
        let key = self.workspace_key();
        let state = session_state::load_session(&key);

        // Restore panel visibility
        self.panel_visible.file_tree = state.panels.file_tree;
        self.panel_visible.side_panel = state.panels.side_panel;
        self.panel_visible.terminal = state.panels.terminal;

        // Restore tree expanded state
        self.file_tree.restore_expanded(&state.tree_expanded);
        if state.tree_selected < self.file_tree.entries.len() {
            self.file_tree.scroll.selected = state.tree_selected;
        }

        // Restore open tabs
        for tab in &state.tabs {
            let path = std::path::Path::new(&tab.path);
            if path.exists() {
                self.open_file(path);
                // Restore cursor/scroll for the just-opened buffer
                if let Some(buf) = self.buffers.last_mut() {
                    let max_line = buf.text.len_lines().saturating_sub(1);
                    buf.cursor.line = tab.cursor_line.min(max_line);
                    buf.cursor.col = tab.cursor_col;
                    buf.scroll.top_line = tab.scroll_top.min(max_line);
                }
            }
        }

        // Restore active tab
        if state.active_tab < self.buffers.len() {
            self.active_buffer = state.active_tab;
        }

        // Restore terminal split percentage
        if let Some(pct) = state.terminal_split_percent {
            self.terminal_split_percent = pct.clamp(10, 80);
        }

        // Restore terminal tabs
        if let Some(term_state) = &state.terminal_session {
            self.terminal_manager.restore_state(term_state);
        }

        // Restore active layout preset
        if let Some(preset_idx) = state.active_preset {
            self.switch_layout(preset_idx as u8);
        }

        // Restore chat conversations
        self.chat_state.load_conversations(&key);

        // Set initial focus
        if !self.buffers.is_empty() {
            self.focus = Focus::Editor;
        } else if self.panel_visible.file_tree {
            self.focus = Focus::FileTree;
        }
    }

    /// Save session state. Call before quitting.
    pub fn save_session(&self) {
        let key = self.workspace_key();

        let tabs: Vec<TabState> = self
            .buffers
            .iter()
            .filter_map(|buf| {
                buf.path.as_ref().map(|p| TabState {
                    path: p.to_string_lossy().to_string(),
                    cursor_line: buf.cursor.line,
                    cursor_col: buf.cursor.col,
                    scroll_top: buf.scroll.top_line,
                })
            })
            .collect();

        let state = SessionState {
            tabs,
            active_tab: self.active_buffer,
            panels: session_state::PanelState {
                file_tree: self.panel_visible.file_tree,
                side_panel: self.panel_visible.side_panel,
                terminal: self.panel_visible.terminal,
            },
            tree_expanded: self.file_tree.expanded_paths(),
            tree_selected: self.file_tree.scroll.selected,
            active_preset: self.active_preset,
            terminal_split_percent: Some(self.terminal_split_percent),
            terminal_session: Some(self.terminal_manager.save_state()),
        };

        if let Err(e) = session_state::save_session(&key, &state) {
            tracing::warn!("Failed to save session state: {}", e);
        }

        // Save chat conversations
        self.chat_state.save_conversations(&key);
    }

    fn update_cursor_position(&self, frame: &mut Frame, editor_area: Rect) {
        // No cursor in review/diff mode
        if self.diff_review.is_some() {
            return;
        }
        if self.focus != Focus::Editor || self.buffers.is_empty() {
            return;
        }

        let buf = &self.buffers[self.active_buffer];
        let gutter_w = gutter_width(buf.line_count());
        let cursor_line = buf.cursor.line;
        let cursor_col = buf.cursor.col;
        let top = buf.scroll.top_line;
        let left = buf.scroll.left_col;

        if cursor_line >= top && cursor_line < top + editor_area.height as usize {
            if cursor_col >= left {
                let x = editor_area.x + gutter_w + (cursor_col - left) as u16;
                let y = editor_area.y + (cursor_line - top) as u16;
                if x < editor_area.right() && y < editor_area.bottom() {
                    frame.set_cursor_position((x, y));
                }
            }
        }
    }
}

/// Result of a clipboard write attempt.
enum ClipboardResult {
    /// Written to the OS/display-server clipboard via arboard.
    System,
    /// Written via OSC 52 escape sequence (SSH-friendly, terminal must support it).
    Osc52,
    /// Neither arboard nor OSC 52 worked; text is in the internal clipboard only.
    Unavailable,
}

/// Write `text` to the terminal emulator's clipboard via the OSC 52 escape sequence.
/// Returns `true` on success.  Works over SSH when the terminal supports OSC 52
/// (kitty, iTerm2, WezTerm, alacritty, etc.).
fn osc52_copy(text: &str) -> bool {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    // OSC 52 ; c ; <base64> ST
    let seq = format!("\x1b]52;c;{}\x07", encoded);
    use std::io::Write as _;
    std::io::stdout().write_all(seq.as_bytes()).and_then(|_| std::io::stdout().flush()).is_ok()
}

fn gutter_width(line_count: usize) -> u16 {
    let digits = if line_count == 0 {
        1
    } else {
        ((line_count as f64).log10().floor() as u16) + 1
    };
    digits + 2
}

/// Replace `<file path="...">...</file>` blocks with a short summary for chat display.
pub(crate) fn collapse_file_blocks(text: &str) -> String {
    let mut result = String::new();
    let mut search_from = 0;

    loop {
        let Some(tag_start) = text[search_from..].find("<file path=\"") else {
            result.push_str(&text[search_from..]);
            break;
        };
        let tag_start = search_from + tag_start;

        // Copy text before the tag
        result.push_str(&text[search_from..tag_start]);

        let after_attr = tag_start + "<file path=\"".len();
        let Some(quote_end) = text[after_attr..].find('"') else {
            result.push_str(&text[tag_start..]);
            break;
        };
        let path_str = &text[after_attr..after_attr + quote_end];

        // Find closing </file>
        let Some(close_pos) = text[after_attr..].find("</file>") else {
            result.push_str(&text[tag_start..]);
            break;
        };
        let after_close = after_attr + close_pos + "</file>".len();

        // Count lines in the file content
        let content_start = after_attr + quote_end + 1; // skip the >
        let content_end = after_attr + close_pos;
        let line_count = text[content_start..content_end].lines().count();

        result.push_str(&format!("[wrote {} ({} lines)]", path_str, line_count));
        search_from = after_close;
    }

    result
}

fn parse_exclude_patterns(workspace: &Workspace) -> Vec<String> {
    use gaviero_core::workspace::settings;
    let val = workspace.resolve_setting(settings::FILES_EXCLUDE, None);
    let mut patterns = Vec::new();
    if let Some(obj) = val.as_object() {
        for (pattern, enabled) in obj {
            if enabled.as_bool().unwrap_or(false) {
                patterns.push(pattern.clone());
            }
        }
    }
    patterns
}

fn parse_git_allow_list(workspace: &Workspace) -> Vec<String> {
    use gaviero_core::workspace::settings;
    let val = workspace.resolve_setting(settings::GIT_TREE_ALLOW_LIST, None);
    let mut items = Vec::new();
    if let Some(arr) = val.as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                items.push(s.to_string());
            }
        }
    }
    items
}

/// Parse layout presets from `panels.layouts` setting.
/// Format: `{ "0": [15, 70, 15], "1": [0, 100, 0], ... }`
/// Each value is `[fileTree%, editor%, sidePanel%]`.
///
/// Built-in defaults (used when the setting is absent or incomplete):
///   0 → standard     [15, 60, 25]
///   1 → chat focused [15, 40, 45]
///   2 → full editor  [ 0,100,  0]
///   3 → code+notes   [ 0, 60, 40]
fn parse_layout_presets(workspace: &Workspace) -> Vec<LayoutPreset> {
    const DEFAULTS: &[(u16, u16, u16)] = &[
        (15, 60, 25),
        (15, 40, 45),
        ( 0,100,  0),
        ( 0, 60, 40)
    ];

    let val = workspace.resolve_setting("panels.layouts", None);
    tracing::info!("Layout presets setting: {}", val);
    let mut presets: Vec<LayoutPreset> = DEFAULTS
        .iter()
        .map(|&(ft, ed, sp)| LayoutPreset { file_tree_pct: ft, editor_pct: ed, side_panel_pct: sp })
        .collect();

    if let Some(obj) = val.as_object() {
        // Settings keys are 1-based ("1"–"6" etc.) matching the digit the user presses.
        // SwitchLayout(n) uses 0-based index, so key "k" → index k-1.
        for k in 1..=9u8 {
            let key = k.to_string();
            if let Some(arr) = obj.get(&key).and_then(|v| v.as_array()) {
                if arr.len() >= 3 {
                    let ft = arr[0].as_u64().unwrap_or(0) as u16;
                    let ed = arr[1].as_u64().unwrap_or(100) as u16;
                    let sp = arr[2].as_u64().unwrap_or(0) as u16;
                    let idx = (k - 1) as usize;
                    while presets.len() <= idx {
                        presets.push(LayoutPreset { file_tree_pct: 0, editor_pct: 100, side_panel_pct: 0 });
                    }
                    presets[idx] = LayoutPreset { file_tree_pct: ft, editor_pct: ed, side_panel_pct: sp };
                }
            }
        }
    }

    presets
}

/// List files in a workspace directory (up to `limit`), for the task planner.
/// Save clipboard image data as a temporary PNG file.
///
/// Encodes RGBA pixel data to PNG using the `png` crate.
/// Returns the path to the saved temporary file.
fn save_clipboard_image_as_png(
    img: &arboard::ImageData,
) -> anyhow::Result<std::path::PathBuf> {
    use std::io::BufWriter;

    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("gaviero")
        .join("attachments");
    std::fs::create_dir_all(&cache_dir)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let path = cache_dir.join(format!("clipboard_{}.png", timestamp));

    let file = std::fs::File::create(&path)?;
    let writer = BufWriter::new(file);

    let mut encoder = png::Encoder::new(writer, img.width as u32, img.height as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);

    let mut png_writer = encoder.write_header()?;
    png_writer.write_image_data(&img.bytes)?;

    Ok(path)
}

fn list_workspace_files(root: &std::path::Path, limit: usize) -> Vec<String> {
    let mut files = Vec::new();
    let walker = std::fs::read_dir(root);
    fn walk(dir: &std::path::Path, prefix: &str, files: &mut Vec<String>, limit: usize) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            if files.len() >= limit { return; }
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden dirs, build artifacts, node_modules
            if name.starts_with('.') || name == "target" || name == "node_modules" || name == "build" {
                continue;
            }
            let path = format!("{}{}", prefix, name);
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(&entry.path(), &format!("{}/", path), files, limit);
            } else {
                files.push(path);
            }
        }
    }
    if walker.is_ok() {
        walk(root, "", &mut files, limit);
    }
    files
}

/// If `src` is markdown-wrapped (i.e. the LLM emitted ```gaviero fences around the
/// DSL), extract and return only the content of the first such block.  If no fences
/// are found, return the original string unchanged.
///
/// Uses the *last* bare ``` line as the closing fence, because DSL prompt strings
/// can contain inner ```cpp / ``` blocks that would otherwise truncate the extraction.
fn extract_gaviero_block(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let fence_start = lines.iter().position(|l| {
        let t = l.trim();
        t == "```gaviero" || t.starts_with("```gaviero ")
    });
    if let Some(start_idx) = fence_start {
        let content_start = start_idx + 1;
        // Use the *last* bare ``` in the file as the closing fence so that inner
        // code blocks inside prompt strings don't cause early termination.
        if let Some(rel_end) = lines[content_start..].iter().rposition(|l| l.trim() == "```") {
            let content = lines[content_start..content_start + rel_end].join("\n");
            return content;
        }
    }
    src.to_string()
}
