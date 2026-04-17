use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use crossterm::event::{MouseButton, MouseEventKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};

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

use gaviero_core::memory::MemoryStore;
use gaviero_core::repo_map::RepoMap;
use gaviero_core::session_state::{self, SessionState, TabState};
use gaviero_core::types::WriteProposal;
use gaviero_core::workspace::Workspace;
use gaviero_core::write_gate::{WriteGatePipeline, WriteMode};

mod chat_memory;
mod commands;
mod controller;
mod editing;
mod layout;
mod left_panel;
mod observers;
mod render;
mod review;
pub(crate) mod session;
mod side_panel;
mod state;

use self::observers::{TuiAcpObserver, TuiSwarmObserver, TuiWriteGateObserver};
use self::state::{
    BatchReviewState, ChangesEntry, ChangesState, DiffKind, FirstRunDialog, FirstRunStep, Focus,
    LayoutAreas, LayoutPreset, LeftPanelMode, MoveState, PanelVisibility, ReviewProposal,
    ScrollbarTarget, SidePanelMode, TreeDialog, TreeDialogKind, build_simple_diff,
};

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
    /// When true, a quit-confirmation dialog is shown (unsaved files or active agents).
    quit_confirm: bool,
    /// First-run setup dialog shown when no `.gaviero/settings.json` is found.
    first_run_dialog: Option<FirstRunDialog>,
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
    // File move state (multi-step: select source → select dest → confirm)
    move_state: Option<MoveState>,

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

    // Code graph cache — lazy build, invalidated on file changes.
    // `None` means "needs (re)build before next chat send".
    pub repo_map: Arc<tokio::sync::RwLock<Option<Arc<RepoMap>>>>,
    /// Workspace root path used for graph rebuilds (first root).
    pub graph_workspace_root: Option<std::path::PathBuf>,

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

        // Detect first run: no .gaviero/settings.json for any workspace root.
        let is_first_run = roots
            .first()
            .map(|root| !root.join(".gaviero").join("settings.json").exists())
            .unwrap_or(false);

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
        let agent_ollama_base_url = workspace
            .resolve_setting(settings::AGENT_OLLAMA_BASE_URL, None)
            .as_str()
            .unwrap_or("http://localhost:11434")
            .to_string();
        let agent_graph_budget_tokens = workspace
            .resolve_setting(settings::AGENT_GRAPH_BUDGET_TOKENS, None)
            .as_u64()
            .unwrap_or(40_000) as usize;

        let write_namespace = workspace.resolve_namespace(None);
        let read_namespaces = workspace.resolve_read_namespaces(None);

        // Open git repo for git panel (M4)
        let git_repo = workspace
            .roots()
            .first()
            .and_then(|r| gaviero_core::git::GitRepo::open(r).ok());

        // Primary workspace root for code-graph context (first root).
        let graph_workspace_root = workspace.roots().first().map(|p| p.to_path_buf());

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
            quit_confirm: false,
            first_run_dialog: if is_first_run {
                Some(FirstRunDialog {
                    step: FirstRunStep::AskSettings,
                    create_settings: false,
                })
            } else {
                None
            },
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
            move_state: None,
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
                    ollama_base_url: agent_ollama_base_url,
                    write_namespace,
                    read_namespaces,
                    graph_budget_tokens: agent_graph_budget_tokens,
                };
                cs
            },
            acp_tasks: HashMap::new(),
            memory: None,
            repo_map: Arc::new(tokio::sync::RwLock::new(None)),
            graph_workspace_root,
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
        controller::handle_event(self, event);
    }

    fn handle_action(&mut self, action: Action) {
        controller::handle_action(self, action);
    }

    // ── Chat panel actions ──────────────────────────────────────

    fn handle_chat_action(&mut self, action: Action) {
        side_panel::handle_chat_action(self, action);
    }

    // ── Git panel actions ────────────────────────────────────

    fn handle_swarm_dashboard_action(&mut self, action: Action) {
        side_panel::handle_swarm_dashboard_action(self, action);
    }

    fn handle_git_panel_action(&mut self, action: Action) {
        side_panel::handle_git_panel_action(self, action);
    }

    /// Read the HEAD version of a file (for diff display).
    fn git_head_content(&self, rel_path: &str) -> Option<String> {
        side_panel::git_head_content(self, rel_path)
    }

    /// Refresh the git panel from the repository if it's visible.
    fn refresh_git_panel(&mut self) {
        side_panel::refresh_git_panel(self);
    }

    /// Collect workspace file paths and update autocomplete matches.
    fn refresh_chat_autocomplete(&mut self) {
        side_panel::refresh_chat_autocomplete(self);
    }

    fn send_chat_message(&mut self) {
        side_panel::send_chat_message(self);
    }

    /// Handle `/swarm <task>` command — plan + execute a multi-agent task.
    fn handle_swarm_command(&mut self) {
        commands::handle_swarm_command(self);
    }

    /// Handle `/run <path.gaviero> [prompt]` — compile and execute a DSL script.
    fn handle_run_script_command(&mut self) {
        commands::handle_run_script_command(self);
    }

    /// Handle `/cswarm <task>` — coordinated tier-routed swarm (Opus → Sonnet/Haiku).
    fn handle_coordinated_swarm_command(&mut self) {
        commands::handle_coordinated_swarm_command(self);
    }

    /// Handle `/undo-swarm` — navigate to swarm dashboard and arm undo confirmation.
    fn handle_undo_swarm_command(&mut self) {
        commands::handle_undo_swarm_command(self);
    }

    /// Handle `/remember <text>` command — store text to semantic memory.
    fn handle_remember_command(&mut self) {
        commands::handle_remember_command(self);
    }

    /// Handle `/attach [path]` command.
    fn handle_attach_command(&mut self) {
        commands::handle_attach_command(self);
    }

    /// Handle `/detach [name|all]` command.
    fn handle_detach_command(&mut self) {
        commands::handle_detach_command(self);
    }

    /// Paste from clipboard into chat — checks for image first, then text.
    fn chat_paste_from_clipboard(&mut self) {
        side_panel::chat_paste_from_clipboard(self);
    }

    fn cancel_agent(&mut self) {
        side_panel::cancel_agent(self);
    }

    // ── Layout presets + fullscreen ────────────────────────────────

    fn toggle_fullscreen(&mut self) {
        layout::toggle_fullscreen(self);
    }

    fn switch_layout(&mut self, n: u8) {
        layout::switch_layout(self, n);
    }

    /// Get the effective layout constraints, honoring active preset.
    fn effective_panel_constraints(&self, total_width: u16) -> (u16, u16) {
        layout::effective_panel_constraints(self, total_width)
    }

    // ── Review mode actions ──────────────────────────────────────

    /// Handle an action while in diff review mode. Returns true if consumed.
    fn handle_review_action(&mut self, action: &Action) -> bool {
        review::handle_review_action(self, action)
    }

    /// Enter diff review mode with an owned proposal. No lock needed.
    fn enter_review_mode(&mut self, proposal: WriteProposal, source: DiffSource) {
        review::enter_review_mode(self, proposal, source);
    }

    // ── Batch review mode ────────────────────────────────────────

    /// Enter batch review mode with a set of deferred proposals.
    fn enter_batch_review(&mut self, proposals: Vec<WriteProposal>) {
        review::enter_batch_review(self, proposals);
    }

    /// Handle an action while in batch review mode. Returns true if consumed.
    fn handle_batch_review_action(&mut self, action: &Action) -> bool {
        review::handle_batch_review_action(self, action)
    }

    /// Apply all deferred writes to disk and exit review mode.
    fn finalize_batch_review(&mut self) {
        review::finalize_batch_review(self);
    }

    /// Discard all proposals and exit review mode.
    fn cancel_batch_review(&mut self) {
        review::cancel_batch_review(self);
    }

    /// Render the review file list in the left panel.
    fn render_review_file_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        review::render_review_file_list(self, frame, area, focused);
    }

    /// Render the batch review diff in the editor area.
    fn render_batch_review_diff(&mut self, frame: &mut Frame, area: Rect) {
        review::render_batch_review_diff(self, frame, area);
    }

    // ── Git Changes panel ────────────────────────────────────────

    /// Populate changes_state from `git status` + working-tree diffs.
    fn refresh_git_changes(&mut self) {
        review::refresh_git_changes(self);
    }

    /// Handle a keyboard action while in Changes mode. Returns true if consumed.
    fn handle_changes_action(&mut self, action: &Action) -> bool {
        review::handle_changes_action(self, action)
    }

    /// Render the git changes file list in the left panel.
    fn render_changes_file_list(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        review::render_changes_file_list(self, frame, area, focused);
    }

    /// Render the git changes diff in the editor area.
    fn render_changes_diff(&mut self, frame: &mut Frame, area: Rect) {
        review::render_changes_diff(self, frame, area);
    }

    // ── Find bar (Ctrl+F) ────────────────────────────────────────

    fn handle_find_bar_action(&mut self, action: Action) {
        editing::handle_find_bar_action(self, action);
    }

    /// Update the editor's search highlight from the find bar input, and jump
    /// to the first match at or after the cursor.
    fn update_find_highlight(&mut self) {
        editing::update_find_highlight(self);
    }

    /// Make sure the editor cursor is within the visible viewport.
    fn ensure_editor_cursor_visible(&mut self) {
        editing::ensure_editor_cursor_visible(self);
    }

    // ── Editor actions ───────────────────────────────────────────

    fn handle_editor_action(&mut self, action: Action) {
        editing::handle_editor_action(self, action);
    }

    // ── Clipboard ───────────────────────────────────────────────

    fn clipboard_copy(&mut self) {
        editing::clipboard_copy(self);
    }

    fn clipboard_cut(&mut self) {
        editing::clipboard_cut(self);
    }

    fn clipboard_paste(&mut self) {
        editing::clipboard_paste(self);
    }

    /// Sets text on both the internal clipboard and the system clipboard.
    /// Tries arboard first, then OSC 52 (for SSH sessions), then falls back to internal only.
    fn set_clipboard(&mut self, text: &str) -> ClipboardResult {
        editing::set_clipboard(self, text)
    }

    fn get_clipboard(&mut self) -> String {
        editing::get_clipboard(self)
    }

    fn handle_search_action(&mut self, action: Action) {
        left_panel::handle_search_action(self, action);
    }

    /// Run search using the current input text.
    fn run_search_from_input(&mut self) {
        left_panel::run_search_from_input(self);
    }

    /// Open the currently selected search result in the editor.
    fn open_selected_search_result(&mut self) {
        left_panel::open_selected_search_result(self);
    }

    fn handle_file_tree_action(&mut self, action: Action) {
        left_panel::handle_file_tree_action(self, action);
    }

    /// Determine the target directory for a new file/folder based on selected entry.
    fn selected_dir(&self) -> Option<std::path::PathBuf> {
        left_panel::selected_dir(self)
    }

    fn start_tree_dialog(&mut self, kind: TreeDialogKind) {
        left_panel::start_tree_dialog(self, kind);
    }

    // ── File move (multi-step) ────────────────────────────────────

    fn start_move(&mut self) {
        left_panel::start_move(self);
    }

    fn handle_move_key(&mut self, key: &crossterm::event::KeyEvent) {
        left_panel::handle_move_key(self, key);
    }

    fn execute_move(&mut self, src: std::path::PathBuf, dest_dir: std::path::PathBuf) {
        left_panel::execute_move(self, src, dest_dir);
    }

    fn render_move_panel_info(&self, frame: &mut Frame, tree_area: Rect) {
        left_panel::render_move_panel_info(self, frame, tree_area);
    }

    fn handle_dialog_key(&mut self, key: &crossterm::event::KeyEvent) {
        left_panel::handle_dialog_key(self, key);
    }

    fn confirm_tree_dialog(&mut self) {
        left_panel::confirm_tree_dialog(self);
    }

    /// Select a specific path in the file tree (scrolls to it).
    fn select_path_in_tree(&mut self, path: &std::path::Path) {
        left_panel::select_path_in_tree(self, path);
    }

    fn refresh_file_tree(&mut self) {
        left_panel::refresh_file_tree(self);
    }

    // ── Mouse handling ───────────────────────────────────────────

    fn handle_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        editing::handle_mouse(self, mouse);
    }

    /// Scroll a panel based on a mouse row within its scrollbar track.
    fn scroll_panel_to_row(&mut self, target: ScrollbarTarget, row: u16) {
        editing::scroll_panel_to_row(self, target, row);
    }

    /// Convert mouse (col, row) to buffer cursor position.
    fn set_cursor_from_mouse(&mut self, col: u16, row: u16) {
        editing::set_cursor_from_mouse(self, col, row);
    }

    /// Handle a bracketed paste event from the terminal.
    fn handle_paste(&mut self, text: &str) {
        editing::handle_paste(self, text);
    }

    /// Search for the selected text (or word at cursor) across the workspace.
    /// If results already exist for the same query, cycles to the next result.
    fn search_selected_in_workspace(&mut self) {
        editing::search_selected_in_workspace(self);
    }

    /// Navigate to the next search result, opening the file and jumping to the line.
    fn goto_next_search_result(&mut self) {
        editing::goto_next_search_result(self);
    }

    fn handle_file_changed(&mut self, path: &Path) {
        editing::handle_file_changed(self, path);
    }

    /// Open a file in a new buffer (or switch to existing).
    pub fn open_file(&mut self, path: &Path) {
        editing::open_file(self, path);
    }

    fn cycle_tab(&mut self, delta: i32) {
        editing::cycle_tab(self, delta);
    }

    fn close_tab(&mut self) {
        editing::close_tab(self);
    }

    /// Handle a key press while the first-run setup dialog is active.
    fn handle_first_run_key(&mut self, key: &crossterm::event::KeyEvent) {
        session::handle_first_run_key(self, key);
    }

    /// Execute first-run actions based on collected answers, then dismiss the dialog.
    fn apply_first_run(&mut self, init_memory: bool) {
        session::apply_first_run(self, init_memory);
    }

    /// Check for unsaved files and active agents before quitting.
    /// If anything needs attention, show the confirmation dialog instead.
    fn try_quit(&mut self) {
        session::try_quit(self);
    }

    fn spawn_active_terminal(&mut self) {
        editing::spawn_active_terminal(self);
    }

    fn save_current_buffer(&mut self) {
        editing::save_current_buffer(self);
    }

    fn is_current_buffer_markdown(&self) -> bool {
        editing::is_current_buffer_markdown(self)
    }

    // ── Rendering ────────────────────────────────────────────────

    pub fn render(&mut self, frame: &mut Frame) {
        render::render(self, frame);
    }

    fn render_fullscreen(&mut self, frame: &mut Frame, area: Rect, panel: Focus) {
        render::render_fullscreen(self, frame, area, panel);
    }

    fn left_panel_title(&self, fullscreen: bool) -> &'static str {
        render::left_panel_title(self, fullscreen)
    }

    fn side_panel_title(&self, fullscreen: bool) -> &'static str {
        render::side_panel_title(self, fullscreen)
    }

    fn render_left_panel_content(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        render::render_left_panel_content(self, frame, area, focused);
    }

    fn current_move_source(&self) -> Option<std::path::PathBuf> {
        render::current_move_source(self)
    }

    fn render_tab_bar(&self, frame: &mut Frame, area: Rect) {
        render::render_tab_bar(self, frame, area);
    }

    fn render_editor(&mut self, frame: &mut Frame, area: Rect) {
        render::render_editor(self, frame, area);
    }

    /// Render the find bar at the top of the editor area.
    fn render_find_bar(&self, frame: &mut Frame, area: Rect) {
        render::render_find_bar(self, frame, area);
    }

    fn render_tree_dialog(&self, frame: &mut Frame, tree_area: Rect, dialog: &TreeDialog) {
        render::render_tree_dialog(self, frame, tree_area, dialog);
    }

    fn render_status_bar(&self, frame: &mut Frame, area: Rect) {
        render::render_status_bar(self, frame, area);
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
        render::render_panel_header(frame, area, title, focused, show_cycle_arrow)
    }

    fn render_terminal(&mut self, frame: &mut Frame, area: Rect) {
        render::render_terminal(self, frame, area);
    }

    fn render_markdown_preview(&self, frame: &mut Frame, area: Rect) {
        render::render_markdown_preview(self, frame, area);
    }

    fn render_side_panel(&mut self, frame: &mut Frame, area: Rect) {
        render::render_side_panel(self, frame, area);
    }

    // ── Session state persistence ─────────────────────────────────

    fn workspace_key(&self) -> std::path::PathBuf {
        session::workspace_key(self)
    }

    /// Restore session state (open tabs, panel visibility, tree state).
    /// Call after `new()`.
    pub fn restore_session(&mut self) {
        session::restore_session(self);
    }

    /// Save session state. Call before quitting.
    pub fn save_session(&self) {
        session::save_session(self);
    }

    fn update_cursor_position(&self, frame: &mut Frame, editor_area: Rect) {
        render::update_cursor_position(self, frame, editor_area);
    }

    fn render_quit_confirm(&self, frame: &mut Frame, area: Rect) {
        render::render_quit_confirm(self, frame, area);
    }

    fn render_first_run_dialog(&self, frame: &mut Frame, area: Rect) {
        render::render_first_run_dialog(self, frame, area);
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
    std::io::stdout()
        .write_all(seq.as_bytes())
        .and_then(|_| std::io::stdout().flush())
        .is_ok()
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
    layout::parse_layout_presets(workspace)
}

/// List files in a workspace directory (up to `limit`), for the task planner.
/// Save clipboard image data as a temporary PNG file.
///
/// Encodes RGBA pixel data to PNG using the `png` crate.
/// Returns the path to the saved temporary file.
fn save_clipboard_image_as_png(img: &arboard::ImageData) -> anyhow::Result<std::path::PathBuf> {
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
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if files.len() >= limit {
                return;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            // Skip hidden dirs, build artifacts, node_modules
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "build"
            {
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
        if let Some(rel_end) = lines[content_start..]
            .iter()
            .rposition(|l| l.trim() == "```")
        {
            let content = lines[content_start..content_start + rel_end].join("\n");
            return content;
        }
    }
    src.to_string()
}
