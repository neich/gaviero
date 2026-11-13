//! First-run setup screen.
//!
//! Runs, at every launch, while the launch target is unconfigured:
//! - a folder without `.gaviero/settings.json`;
//! - a `.gaviero-workspace` file (given directly, or found by `--workspace`)
//!   without `.gaviero/settings.json` beside it — the workspace layer;
//! - `--workspace` on a directory without a `.gaviero-workspace` file, which
//!   first picks the member folders and writes that file.
//!
//! Every mode then offers the same three choices for that one
//! `.gaviero/settings.json` (the folder's, or the one beside the workspace
//! file, which every member inherits):
//! 1. **Editor only** (preselected; also Esc) — write no settings and open an
//!    editor-only session: explorer, find, editor and terminal; no agents,
//!    memory or code graph. Settings come from the global file.
//! 2. **Minimal settings** — `.gaviero/settings.json` containing `{}`.
//! 3. **Full settings** — `.gaviero/settings.json` as a byte-for-byte copy of
//!    `$GAVIERO_ROOT/.gaviero/settings.json` as it is right now; when that
//!    file does not exist, gaviero's built-in full settings instead.
//!
//! Only the highlighted option's description is drawn, so all three titles
//! fit even a short terminal pane.
//!
//! The screen owns its own terminal session: raw mode + the alternate screen
//! are entered and left inside [`run`], so the caller gets a pristine terminal
//! back. That matters because the C1 memory-migration consent prompt that runs
//! after it reads plain stdin.

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event as TermEvent, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::path::{Path, PathBuf};

use crate::theme;

/// Directory names never offered as workspace members — build output and
/// dependency caches, mirroring the `files.exclude` defaults.
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "venv",
    "__pycache__",
];

/// Exact content of the "minimal settings" file.
const MINIMAL_SETTINGS: &str = "{}\n";

/// How the TUI was launched, which decides what the screen offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    /// Single folder — settings go to `<root>/.gaviero/settings.json`.
    Folder,
    /// `--workspace` on a directory without a workspace file — picks the
    /// members and writes `<root>/<name>.gaviero-workspace`; settings go to
    /// `<root>/.gaviero/settings.json`, beside it.
    Workspace,
    /// An existing `.gaviero-workspace` file — settings go to
    /// `.gaviero/settings.json` beside it.
    OpenWorkspace(PathBuf),
}

impl LaunchMode {
    /// Pick the mode for a launch: an existing workspace file (given directly
    /// or found by `--workspace`) wins, then `--workspace`, else folder mode.
    pub fn for_launch(workspace_file: Option<&Path>, workspace_flag: bool) -> Self {
        match workspace_file {
            Some(file) => LaunchMode::OpenWorkspace(file.to_path_buf()),
            None if workspace_flag => LaunchMode::Workspace,
            None => LaunchMode::Folder,
        }
    }
}

/// One option on the choice screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    /// Write no settings; open an editor-only session.
    EditorOnly,
    /// `.gaviero/settings.json` containing `{}`.
    Minimal,
    /// `.gaviero/settings.json` copied byte for byte from the global file,
    /// or gaviero's built-in full settings when there is none.
    Full,
}

impl Choice {
    fn title(self) -> &'static str {
        match self {
            Choice::EditorOnly => "Editor only",
            Choice::Minimal => "Minimal settings",
            Choice::Full => "Full settings",
        }
    }
}

/// Which screen is on-stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    /// Workspace members (`--workspace` with sub-directories only).
    Folders,
    /// The settings choice.
    Choice,
}

/// A candidate workspace member found under the launch directory.
struct FolderCandidate {
    path: PathBuf,
    name: String,
    is_git: bool,
    selected: bool,
}

/// What the screen decided, handed back to `main`.
#[derive(Debug, PartialEq, Eq)]
pub struct SetupOutcome {
    /// The `.gaviero-workspace` file to open — the one opened, or the one
    /// just written. `None` means single-folder mode: open the launch
    /// directory.
    pub workspace_file: Option<PathBuf>,
    /// Open an editor-only session: no agents, memory, code graph, MCP or
    /// remote, and no settings written.
    pub editor_only: bool,
}

/// `.gaviero/settings.json` under `dir`.
fn settings_file(dir: &Path) -> PathBuf {
    dir.join(".gaviero").join("settings.json")
}

/// Does this launch target still need the setup screen?
///
/// A folder is "configured" once `<root>/.gaviero/settings.json` exists; a
/// workspace file once `.gaviero/settings.json` sits beside it; `--workspace`
/// on a directory needs the screen while it has no workspace file at all.
pub fn needs_setup(root: &Path, mode: &LaunchMode) -> bool {
    match mode {
        LaunchMode::Folder => !settings_file(root).exists(),
        LaunchMode::Workspace => existing_workspace_file(root).is_none(),
        LaunchMode::OpenWorkspace(file) => {
            !settings_file(file.parent().unwrap_or(Path::new("."))).exists()
        }
    }
}

/// The `.gaviero-workspace` file already present in `root`, if any.
/// `<dirname>.gaviero-workspace` wins; otherwise the first match by name.
pub fn existing_workspace_file(root: &Path) -> Option<PathBuf> {
    let preferred = root.join(format!("{}.gaviero-workspace", dir_name(root)));
    if preferred.is_file() {
        return Some(preferred);
    }
    let mut matches: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "gaviero-workspace"))
        .collect();
    matches.sort();
    matches.into_iter().next()
}

/// Run the setup screen and write whatever was chosen. `root` is the launch
/// directory (ignored for [`LaunchMode::OpenWorkspace`], whose directory is
/// the workspace file's).
pub fn run(root: &Path, mode: LaunchMode) -> Result<SetupOutcome> {
    let mut wizard = Wizard::new(root, mode, gaviero_core::workspace::user_settings_path());
    match run_screen(&mut wizard)? {
        Flow::EditorOnly => Ok(wizard.untouched_outcome()),
        _ => wizard.apply(),
    }
}

/// A full-screen, pre-editor screen driven by [`run_screen`].
trait Screen {
    type Out;
    fn draw(&self, frame: &mut Frame);
    /// `Some` ends the screen with that result.
    fn key(&mut self, code: KeyCode) -> Option<Self::Out>;
}

impl Screen for Wizard {
    type Out = Flow;
    fn draw(&self, frame: &mut Frame) {
        self.render(frame);
    }
    fn key(&mut self, code: KeyCode) -> Option<Flow> {
        match self.handle_key(code) {
            Flow::Continue => None,
            done => Some(done),
        }
    }
}

/// Run `screen` in its own raw-mode + alternate-screen session and hand the
/// terminal back pristine, so plain-stdin prompts after it still work.
fn run_screen<S: Screen>(screen: &mut S) -> Result<S::Out> {
    enable_raw_mode().context("enabling raw mode for setup screen")?;
    let mut stdout = std::io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e).context("entering alternate screen for setup screen");
    }

    let control = std::env::var_os(crate::session_host::CONTROL_ENV).map(PathBuf::from);
    let result = (|| -> Result<S::Out> {
        let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
        let mut terminal = ratatui::Terminal::new(backend).context("creating setup terminal")?;
        loop {
            terminal.draw(|frame| screen.draw(frame))?;
            // Under a session host, a tab still at this screen when the
            // host closes it (or quits) must go at once, not sit here until
            // it is killed.
            if let Some(control) = &control {
                while !event::poll(CONTROL_POLL)? {
                    if host_closed_session(control) {
                        return Err(HostClosed.into());
                    }
                }
            }
            let TermEvent::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            if let Some(out) = screen.key(key.code) {
                return Ok(out);
            }
        }
    })();

    // Restore the terminal on every path before reporting anything, so an
    // error message lands on a usable screen.
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();
    result
}

/// How often a pre-editor screen under a session host checks its control
/// file while no key arrives.
const CONTROL_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// The session host closed this tab or is quitting while a pre-editor
/// screen was still asking: nothing was chosen or written, and the process
/// should end quietly (`main` exits 0 on it).
#[derive(Debug)]
pub struct HostClosed;

impl std::fmt::Display for HostClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the session host closed this tab")
    }
}

impl std::error::Error for HostClosed {}

/// The host wrote `exit` (tab closed / host quitting) or `quit` into the
/// control file (`App::poll_session_control` reads the same file once the
/// editor runs). Either ends a screen that has nothing to save.
fn host_closed_session(control: &Path) -> bool {
    std::fs::read_to_string(control).is_ok_and(|text| matches!(text.trim(), "exit" | "quit"))
}

// ── Folder that holds a workspace file ─────────────────────────

/// How to open a directory launch whose directory holds a
/// `.gaviero-workspace` file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderLaunch {
    /// Open that workspace file instead (multi-folder mode).
    Workspace(PathBuf),
    /// Open the directory as a plain single folder, as asked.
    Folder,
}

/// Ask whether `gaviero <dir>` should open the `.gaviero-workspace` file
/// found in `<dir>` or the directory itself. Needs a TTY.
pub fn ask_folder_launch(root: &Path, workspace_file: &Path) -> Result<FolderLaunch> {
    let mut prompt = WorkspacePrompt::new(root, workspace_file);
    run_screen(&mut prompt)
}

/// The two-option screen behind [`ask_folder_launch`]. Opening the workspace
/// is preselected; Esc opens the folder exactly as the command asked.
struct WorkspacePrompt {
    root: PathBuf,
    workspace_file: PathBuf,
    /// 0 = workspace, 1 = plain folder.
    cursor: usize,
}

impl WorkspacePrompt {
    fn new(root: &Path, workspace_file: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            workspace_file: workspace_file.to_path_buf(),
            cursor: 0,
        }
    }

    fn pick(&self, idx: usize) -> FolderLaunch {
        if idx == 0 {
            FolderLaunch::Workspace(self.workspace_file.clone())
        } else {
            FolderLaunch::Folder
        }
    }

    fn file_name(&self) -> String {
        self.workspace_file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("the workspace file")
            .to_string()
    }

    /// Member folders listed in the workspace file (empty if unreadable).
    fn member_names(&self) -> Vec<String> {
        gaviero_core::workspace::Workspace::load(&self.workspace_file)
            .map(|ws| {
                ws.folders()
                    .iter()
                    .map(|f| f.display_name().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Screen for WorkspacePrompt {
    type Out = FolderLaunch;

    fn key(&mut self, code: KeyCode) -> Option<FolderLaunch> {
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = 0;
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = 1;
                None
            }
            KeyCode::Char('1') => Some(self.pick(0)),
            KeyCode::Char('2') => Some(self.pick(1)),
            KeyCode::Enter => Some(self.pick(self.cursor)),
            KeyCode::Esc => Some(FolderLaunch::Folder),
            _ => None,
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let block = Block::default()
            .title(" Gaviero — workspace found ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::FOCUS_BORDER))
            .style(Style::default().bg(theme::PANEL_BG).fg(theme::TEXT_FG));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let dim = Style::default().fg(theme::TEXT_DIM);
        let file_name = self.file_name();
        let members = self.member_names();
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(format!("  {}", self.root.display()), dim)),
            Line::from(Span::styled(
                format!("  This folder holds {file_name}. Open the workspace instead?"),
                dim,
            )),
            Line::from(""),
        ];
        let titles = [
            format!("Open workspace {file_name}"),
            "Open this folder as a plain folder".to_string(),
        ];
        for (idx, title) in titles.iter().enumerate() {
            let active = idx == self.cursor;
            let marker = if active { "▸" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("  {marker} {}. {title}", idx + 1),
                Style::default()
                    .fg(if active { theme::ACCENT } else { theme::TEXT_FG })
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            )));
        }
        lines.push(Line::from(""));
        let detail: Vec<(String, ratatui::style::Color)> = if self.cursor == 0 {
            let mut d = vec![(
                "Each member keeps its own memory, code graph and .gaviero/settings.json."
                    .to_string(),
                theme::TEXT_DIM,
            )];
            if !members.is_empty() {
                d.push((format!("Members: {}", members.join(", ")), theme::TEXT_DIM));
            }
            d
        } else {
            vec![
                (
                    "Memory, code graph and turn history go to this folder's own .gaviero/,"
                        .to_string(),
                    theme::WARNING,
                ),
                (
                    "separate from the workspace's — memories do not carry over either way."
                        .to_string(),
                    theme::WARNING,
                ),
                (
                    "Member folders' .gaviero/settings.json files are not applied.".to_string(),
                    theme::WARNING,
                ),
                (
                    "The GIT panel still shows a tab per repository inside the folder."
                        .to_string(),
                    theme::TEXT_DIM,
                ),
            ]
        };
        for (text, color) in detail {
            lines.push(Line::from(Span::styled(
                format!("    {text}"),
                Style::default().fg(color),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ↑/↓ choose · 1-2 pick · Enter open · Esc plain folder · `gaviero --folder` skips this",
            dim,
        )));

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            },
        );
    }
}

/// What `handle_key` asks the event loop to do next.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    /// Esc: open editor-only without writing anything at all.
    EditorOnly,
    /// Apply the chosen option (editor-only included: with `--workspace` it
    /// still writes the workspace file the folder step built).
    Apply,
}

struct Wizard {
    root: PathBuf,
    mode: LaunchMode,
    step: Step,
    choice: Choice,
    folders: Vec<FolderCandidate>,
    folder_cursor: usize,
    /// Where the global settings file is looked for (`None` only when no
    /// root resolves). Shown on screen so a missing file is explained.
    global_path: Option<PathBuf>,
    /// The global settings file, when it exists — what [`Choice::Full`]
    /// copies.
    global: Option<PathBuf>,
    /// The global file holds a non-empty `remote.relay.secret`, which a copy
    /// would carry into the folder.
    global_has_secret: bool,
    /// Inline complaint shown under the current step (e.g. "select a folder").
    notice: Option<String>,
}

impl Wizard {
    /// `global` is the global settings path; only an existing file counts.
    fn new(root: &Path, mode: LaunchMode, global_path: Option<PathBuf>) -> Self {
        // An opened workspace file's settings live beside it.
        let root = match &mode {
            LaunchMode::OpenWorkspace(file) => file.parent().unwrap_or(root),
            _ => root,
        };
        let global = global_path.clone().filter(|p| p.is_file());
        tracing::info!(
            "first-run setup: global settings {} ({})",
            global_path
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<no root>".to_string()),
            if global.is_some() { "found" } else { "missing" }
        );
        let global_has_secret = global
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .is_some_and(|doc| {
                gaviero_core::workspace::dot_get(&doc, "remote.relay.secret")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty())
            });
        let folders = if mode == LaunchMode::Workspace {
            discover_folders(root)
        } else {
            Vec::new()
        };
        let mut wizard = Self {
            root: root.to_path_buf(),
            mode,
            step: Step::Choice,
            choice: Choice::Minimal,
            folders,
            folder_cursor: 0,
            global_path,
            global,
            global_has_secret,
            notice: None,
        };
        wizard.step = if wizard.has_folder_step() {
            Step::Folders
        } else {
            Step::Choice
        };
        wizard.choice = wizard.choices()[0];
        wizard
    }

    /// Options on the choice screen, in display order (the first is
    /// preselected). The same in every mode.
    fn choices(&self) -> Vec<Choice> {
        vec![Choice::EditorOnly, Choice::Minimal, Choice::Full]
    }

    /// Esc: open editor-only and write nothing — the opened workspace file
    /// if there is one, else the launch directory as a single folder.
    fn untouched_outcome(&self) -> SetupOutcome {
        SetupOutcome {
            workspace_file: match &self.mode {
                LaunchMode::OpenWorkspace(file) => Some(file.clone()),
                _ => None,
            },
            editor_only: true,
        }
    }

    fn move_choice(&mut self, delta: isize) {
        let list = self.choices();
        let at = list.iter().position(|c| *c == self.choice).unwrap_or(0);
        let next = (at as isize + delta).clamp(0, list.len() as isize - 1) as usize;
        self.choice = list[next];
    }

    /// The flow that commits the current choice.
    fn commit(&self) -> Flow {
        Flow::Apply
    }

    /// Workspace mode with no sub-directories has nothing to pick, so the
    /// folder step is skipped and the launch directory becomes the sole member.
    fn has_folder_step(&self) -> bool {
        self.mode == LaunchMode::Workspace && !self.folders.is_empty()
    }

    fn selected_folders(&self) -> Vec<&FolderCandidate> {
        self.folders.iter().filter(|f| f.selected).collect()
    }

    fn handle_key(&mut self, code: KeyCode) -> Flow {
        self.notice = None;
        match self.step {
            Step::Folders => match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.folder_cursor = self.folder_cursor.saturating_sub(1);
                    Flow::Continue
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.folder_cursor + 1 < self.folders.len() {
                        self.folder_cursor += 1;
                    }
                    Flow::Continue
                }
                KeyCode::Char(' ') => {
                    if let Some(f) = self.folders.get_mut(self.folder_cursor) {
                        f.selected = !f.selected;
                    }
                    Flow::Continue
                }
                KeyCode::Char('a') | KeyCode::Char('A') => {
                    let all_on = self.folders.iter().all(|f| f.selected);
                    for f in &mut self.folders {
                        f.selected = !all_on;
                    }
                    Flow::Continue
                }
                KeyCode::Enter => {
                    if self.selected_folders().is_empty() {
                        self.notice =
                            Some("Select at least one folder (Space toggles).".to_string());
                    } else {
                        self.step = Step::Choice;
                    }
                    Flow::Continue
                }
                // Nothing written: open the directory editor-only.
                KeyCode::Esc => Flow::EditorOnly,
                _ => Flow::Continue,
            },
            Step::Choice => match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.move_choice(-1);
                    Flow::Continue
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.move_choice(1);
                    Flow::Continue
                }
                KeyCode::Char(c @ '1'..='9') => {
                    let pick = c as usize - '1' as usize;
                    match self.choices().get(pick).copied() {
                        Some(choice) => {
                            self.choice = choice;
                            self.commit()
                        }
                        None => Flow::Continue,
                    }
                }
                KeyCode::Enter => self.commit(),
                KeyCode::Esc if self.has_folder_step() => {
                    self.step = Step::Folders;
                    Flow::Continue
                }
                KeyCode::Esc => Flow::EditorOnly,
                _ => Flow::Continue,
            },
        }
    }

    /// Folders that will end up in the workspace file. Falls back to the
    /// launch directory when there was nothing to pick.
    fn workspace_members(&self) -> Vec<PathBuf> {
        if self.folders.is_empty() {
            vec![self.root.clone()]
        } else {
            self.selected_folders()
                .iter()
                .map(|f| f.path.clone())
                .collect()
        }
    }

    fn workspace_file_path(&self) -> PathBuf {
        self.root
            .join(format!("{}.gaviero-workspace", dir_name(&self.root)))
    }

    /// The one `.gaviero/settings.json` a settings choice writes: the
    /// folder's own, or the workspace layer beside the workspace file —
    /// which every member folder inherits.
    fn settings_target(&self) -> PathBuf {
        settings_file(&self.root)
    }

    /// The `.gaviero-workspace` file this launch opens or creates.
    fn launch_workspace_file(&self) -> Option<PathBuf> {
        match &self.mode {
            LaunchMode::Folder => None,
            LaunchMode::Workspace => Some(self.workspace_file_path()),
            LaunchMode::OpenWorkspace(file) => Some(file.clone()),
        }
    }

    /// Every file the current choice writes, for the confirm list.
    fn files_written(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if self.mode == LaunchMode::Workspace {
            files.push(self.workspace_file_path());
        }
        if self.choice != Choice::EditorOnly {
            files.push(self.settings_target());
        }
        files
    }

    /// Write every file the current choice promises.
    fn apply(&self) -> Result<SetupOutcome> {
        if self.mode == LaunchMode::Workspace {
            write_workspace_file(&self.workspace_file_path(), &self.workspace_members())?;
        }
        let workspace_file = self.launch_workspace_file();
        let content: Vec<u8> = match (self.choice, self.global.as_deref()) {
            (Choice::EditorOnly, _) => {
                return Ok(SetupOutcome {
                    workspace_file,
                    editor_only: true,
                });
            }
            (Choice::Minimal, _) => MINIMAL_SETTINGS.as_bytes().to_vec(),
            (Choice::Full, Some(global)) => {
                std::fs::read(global).with_context(|| format!("reading {}", global.display()))?
            }
            (Choice::Full, None) => {
                // A workspace layer names no namespace, so each member keeps
                // its own (the folder name).
                let namespace = match self.mode {
                    LaunchMode::Folder => Some(dir_name(&self.root)),
                    _ => None,
                };
                let mut body =
                    serde_json::to_string_pretty(&builtin_full_settings(namespace.as_deref()))?;
                body.push('\n');
                body.into_bytes()
            }
        };
        write_if_absent(&self.settings_target(), &content)?;
        Ok(SetupOutcome {
            workspace_file,
            editor_only: false,
        })
    }

    // ── Rendering ──────────────────────────────────────────────

    fn render(&self, frame: &mut Frame) {
        let area = frame.area();
        let block = Block::default()
            .title(" Gaviero — first-time setup ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::FOCUS_BORDER))
            .style(Style::default().bg(theme::PANEL_BG).fg(theme::TEXT_FG));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(
            format!("  {}", self.root.display()),
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(Span::styled(
            match &self.mode {
                LaunchMode::Folder => {
                    "  No .gaviero/settings.json here. How should gaviero open this folder?"
                        .to_string()
                }
                LaunchMode::Workspace => "  No .gaviero-workspace found — creating one.".to_string(),
                LaunchMode::OpenWorkspace(file) => format!(
                    "  No .gaviero/settings.json beside {}. How should gaviero open this workspace?",
                    file.file_name().and_then(|n| n.to_str()).unwrap_or("the workspace file")
                ),
            },
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(""));

        match self.step {
            Step::Folders => self.render_folders(&mut lines, inner.height),
            Step::Choice => self.render_choices(&mut lines),
        }

        if let Some(notice) = &self.notice {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {notice}"),
                Style::default().fg(theme::WARNING),
            )));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  {}", self.footer()),
            Style::default().fg(theme::TEXT_DIM),
        )));

        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                x: inner.x,
                y: inner.y + 1,
                width: inner.width,
                height: inner.height.saturating_sub(1),
            },
        );
    }

    fn footer(&self) -> String {
        match self.step {
            Step::Folders => {
                "↑/↓ move · Space toggle · a all/none · Enter next · Esc editor only".to_string()
            }
            Step::Choice => {
                let esc = if self.has_folder_step() {
                    "Esc back"
                } else {
                    "Esc editor only"
                };
                format!(
                    "↑/↓ choose · 1-{} pick · Enter open · {esc}",
                    self.choices().len()
                )
            }
        }
    }

    /// Description of one option, drawn under the list for the highlighted
    /// one only.
    fn choice_detail(&self, choice: Choice) -> Vec<(String, ratatui::style::Color)> {
        let dim = theme::TEXT_DIM;
        let looked_at = self
            .global_path
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "$GAVIERO_ROOT/.gaviero/settings.json".to_string());
        let inherited = match &self.global {
            Some(path) => format!("the global {}", path.display()),
            None => "gaviero's built-in defaults (no global settings file)".to_string(),
        };
        match choice {
            Choice::EditorOnly => vec![
                (
                    "Explorer, find, editor and terminal. No agents, memory or code graph."
                        .to_string(),
                    dim,
                ),
                (
                    if self.mode == LaunchMode::Workspace {
                        "Only the .gaviero-workspace file is written; no .gaviero/.".to_string()
                    } else {
                        "No .gaviero/ is created; nothing is written.".to_string()
                    },
                    dim,
                ),
                (format!("Settings come from {inherited}."), dim),
            ],
            Choice::Minimal => vec![
                ("Writes .gaviero/settings.json containing {}.".to_string(), dim),
                (format!("Every setting is inherited from {inherited}."), dim),
            ],
            Choice::Full => match &self.global {
                Some(path) => {
                    let mut lines = vec![
                        (
                            "Writes .gaviero/settings.json as an exact copy of".to_string(),
                            dim,
                        ),
                        (format!("{} (as it is now).", path.display()), dim),
                    ];
                    if self.global_has_secret {
                        lines.push((
                            "⚠ The copy includes remote.relay.secret — do not commit it."
                                .to_string(),
                            theme::WARNING,
                        ));
                    }
                    lines
                }
                None => vec![
                    (
                        "Writes gaviero's built-in full settings: every agent tool, auto-approved,"
                            .to_string(),
                        dim,
                    ),
                    (
                        "a bash allow/deny list, default excludes and panel layouts.".to_string(),
                        dim,
                    ),
                    (format!("No global settings file at {looked_at}."), theme::WARNING),
                ],
            },
        }
    }

    fn render_choices(&self, lines: &mut Vec<Line>) {
        if self.mode != LaunchMode::Folder {
            lines.push(Line::from(Span::styled(
                "  Workspace settings — every member folder inherits them",
                Style::default()
                    .fg(theme::TEXT_BRIGHT)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
        }
        for (idx, choice) in self.choices().iter().enumerate() {
            let active = *choice == self.choice;
            let marker = if active { "▸" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("  {marker} {}. {}", idx + 1, choice.title()),
                Style::default()
                    .fg(if active {
                        theme::ACCENT
                    } else {
                        theme::TEXT_FG
                    })
                    .add_modifier(if active {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            )));
        }
        lines.push(Line::from(""));
        for (text, color) in self.choice_detail(self.choice) {
            lines.push(Line::from(Span::styled(
                format!("    {text}"),
                Style::default().fg(color),
            )));
        }

        let files = self.files_written();
        if files.is_empty() {
            return;
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Writes:",
            Style::default().fg(theme::TEXT_DIM),
        )));
        for file in files {
            lines.push(Line::from(Span::styled(
                format!("    {}", file.display()),
                Style::default().fg(theme::CODE_GREEN),
            )));
        }
        lines.push(Line::from(Span::styled(
            "  Existing files are never overwritten.",
            Style::default().fg(theme::TEXT_DIM),
        )));
    }

    fn render_folders(&self, lines: &mut Vec<Line>, height: u16) {
        lines.push(Line::from(Span::styled(
            "  Workspace folders",
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Git repositories are pre-selected.",
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(""));

        // Keep the cursor on-screen when there are more folders than rows.
        let visible = (height as usize).saturating_sub(10).max(1);
        let start = self.folder_cursor.saturating_sub(visible.saturating_sub(1));
        for (idx, folder) in self.folders.iter().enumerate().skip(start).take(visible) {
            let cursor = if idx == self.folder_cursor {
                "▸"
            } else {
                " "
            };
            let check = if folder.selected { "[x]" } else { "[ ]" };
            let tag = if folder.is_git { "  (git)" } else { "" };
            lines.push(Line::from(Span::styled(
                format!("  {cursor} {check} {}{tag}", folder.name),
                Style::default().fg(if idx == self.folder_cursor {
                    theme::ACCENT
                } else if folder.selected {
                    theme::TEXT_BRIGHT
                } else {
                    theme::TEXT_FG
                }),
            )));
        }
        if self.folders.len() > visible {
            lines.push(Line::from(Span::styled(
                format!(
                    "  … {} of {} shown",
                    visible.min(self.folders.len()),
                    self.folders.len()
                ),
                Style::default().fg(theme::TEXT_DIM),
            )));
        }
    }
}

// ── Filesystem helpers ─────────────────────────────────────────

fn dir_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string()
}

/// Immediate sub-directories of `root`, hidden and build dirs dropped,
/// sorted by name, with git repositories pre-selected.
fn discover_folders(root: &Path) -> Vec<FolderCandidate> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut folders: Vec<FolderCandidate> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                return None;
            }
            let path = e.path();
            let is_git = path.join(".git").exists();
            Some(FolderCandidate {
                name,
                is_git,
                selected: is_git,
                path,
            })
        })
        .collect();
    folders.sort_by(|a, b| a.name.cmp(&b.name));
    folders
}

/// What [`Choice::Full`] writes when there is no global settings file: the
/// default excludes, layouts and (for a single folder) its memory namespace,
/// plus every agent tool auto-approved with a bash allow/deny list. A
/// workspace layer gets no namespace, so each member keeps its own. No `mcp__gaviero`
/// entry is needed — gaviero's own MCP server is approved for every provider
/// by default.
fn builtin_full_settings(namespace: Option<&str>) -> serde_json::Value {
    let mut settings = serde_json::json!({
        "agent": {
            "availableTools": [
                "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit", "Bash",
                "AskUserQuestion"
            ],
            "approvedTools": [
                "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit", "Bash",
                "AskUserQuestion"
            ],
            "permissions": {
                "bash": {
                    "denylist": [
                        "terraform destroy",
                        "npm publish",
                        "git push --force",
                        "git push -f",
                        "chmod 777",
                        "mkfs.",
                        "dd if="
                    ],
                    "allowlist": [
                        "cargo check", "cargo test", "cargo build", "cargo clippy",
                        "git status", "git diff", "git log", "git show",
                        "ls", "cat ", "rg ", "grep ", "find ", "head ", "tail ",
                        "wc ", "pwd", "echo "
                    ]
                }
            }
        },
        "files": {
            "exclude": {
                ".DS_Store": true,
                ".cache": true,
                ".gradle": true,
                ".idea": true,
                ".mvn": true,
                ".mypy_cache": true,
                ".next": true,
                ".nuxt": true,
                ".parcel-cache": true,
                ".pytest_cache": true,
                ".tox": true,
                ".venv": true,
                "Thumbs.db": true,
                "__pycache__": true,
                "build": true,
                "coverage": true,
                "dist": true,
                "node_modules": true,
                "out": true,
                "target": true,
                "venv": true
            }
        },
        "git": {
            "treeAllowList": ["config", "description", "HEAD", "hooks", "info"]
        },
        "memory": {
            "readNamespaces": []
        },
        "panels": {
            "fileTree": { "width": 25 },
            "sidePanel": { "width": 50 },
            "layouts": {
                "1": [15, 60, 25],
                "2": [15, 40, 45],
                "3": [0, 100, 0],
                "4": [0, 60, 40]
            }
        }
    });
    if let Some(namespace) = namespace {
        settings["memory"]["namespace"] = serde_json::json!(namespace);
    }
    settings
}

/// Write `bytes` to `path` unless something is already there.
fn write_if_absent(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    tracing::info!("first-run setup wrote {}", path.display());
    Ok(())
}

/// Write the `.gaviero-workspace` file. Folder paths are absolute: they are
/// used as roots verbatim by `Workspace::load`, which never resolves them
/// against the workspace file's directory. `settings` stays empty — settings
/// embedded in the workspace file are not read.
fn write_workspace_file(path: &Path, members: &[PathBuf]) -> Result<()> {
    let folders: Vec<serde_json::Value> = members
        .iter()
        .map(|p| serde_json::json!({ "path": p, "name": dir_name(p) }))
        .collect();
    let doc = serde_json::json!({ "folders": folders, "settings": {} });
    let body = serde_json::to_string_pretty(&doc)?;
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    tracing::info!("first-run setup wrote {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// A tab still at a launch prompt leaves when the host writes `exit`
    /// (tab closed, host quitting) or `quit`; no file or anything else
    /// keeps it asking.
    #[test]
    fn launch_prompt_ends_when_the_host_closes_the_tab() {
        let dir = tempdir();
        let control = dir.path().join("1.control");
        assert!(!host_closed_session(&control));
        std::fs::write(&control, "").unwrap();
        assert!(!host_closed_session(&control));
        std::fs::write(&control, "exit\n").unwrap();
        assert!(host_closed_session(&control));
        std::fs::write(&control, "quit").unwrap();
        assert!(host_closed_session(&control));
        let err: anyhow::Error = HostClosed.into();
        assert!(err.context("asking").is::<HostClosed>());
    }

    /// A global settings file with arbitrary formatting (to prove the copy
    /// is byte-exact) and a relay secret.
    const GLOBAL: &str =
        "{\n  \"remote\": {\"relay\": {\"host\": \"r.example\", \"secret\": \"s3cret\"}},\n  \"agent\":{\"model\":\"claude:opus\"}\n}\n";

    fn global_file(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("global-settings.json");
        std::fs::write(&path, body).unwrap();
        path
    }

    fn read(path: PathBuf) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn needs_setup_folder_mode_tracks_settings_file() {
        let dir = tempdir();
        assert!(needs_setup(dir.path(), &LaunchMode::Folder));
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(dir.path().join(".gaviero/settings.json"), "{}").unwrap();
        assert!(!needs_setup(dir.path(), &LaunchMode::Folder));
    }

    #[test]
    fn needs_setup_workspace_mode_tracks_workspace_file() {
        let dir = tempdir();
        assert!(needs_setup(dir.path(), &LaunchMode::Workspace));
        std::fs::write(dir.path().join("anything.gaviero-workspace"), "{}").unwrap();
        assert!(!needs_setup(dir.path(), &LaunchMode::Workspace));
        // A per-folder settings.json alone must not count as a configured
        // workspace — the workspace file is what `--workspace` needs.
        let other = tempdir();
        std::fs::create_dir_all(other.path().join(".gaviero")).unwrap();
        std::fs::write(other.path().join(".gaviero/settings.json"), "{}").unwrap();
        assert!(needs_setup(other.path(), &LaunchMode::Workspace));
    }

    #[test]
    fn workspace_prompt_preselects_the_workspace_and_esc_keeps_the_folder() {
        let dir = tempdir();
        let file = workspace_file(dir.path());
        let mut prompt = WorkspacePrompt::new(dir.path(), &file);
        assert_eq!(prompt.member_names(), ["app"]);

        assert_eq!(
            prompt.key(KeyCode::Enter),
            Some(FolderLaunch::Workspace(file.clone()))
        );
        assert_eq!(prompt.key(KeyCode::Down), None);
        assert_eq!(prompt.key(KeyCode::Enter), Some(FolderLaunch::Folder));
        assert_eq!(prompt.key(KeyCode::Char('k')), None);
        assert_eq!(
            prompt.key(KeyCode::Enter),
            Some(FolderLaunch::Workspace(file.clone()))
        );
        assert_eq!(prompt.key(KeyCode::Char('2')), Some(FolderLaunch::Folder));
        assert_eq!(
            prompt.key(KeyCode::Char('1')),
            Some(FolderLaunch::Workspace(file))
        );
        assert_eq!(prompt.key(KeyCode::Esc), Some(FolderLaunch::Folder));
        assert_eq!(prompt.key(KeyCode::Char('x')), None);
    }

    #[test]
    fn workspace_prompt_draws_both_options_and_the_folder_warning() {
        let dir = tempdir();
        let file = workspace_file(dir.path());
        let mut prompt = WorkspacePrompt::new(dir.path(), &file);
        let render = |prompt: &WorkspacePrompt| -> String {
            let backend = ratatui::backend::TestBackend::new(110, 20);
            let mut terminal = ratatui::Terminal::new(backend).unwrap();
            terminal.draw(|f| prompt.draw(f)).unwrap();
            let buf = terminal.backend().buffer().clone();
            buf.content().iter().map(|c| c.symbol()).collect()
        };
        let screen = render(&prompt);
        assert!(screen.contains("Open workspace ws.gaviero-workspace"), "{screen}");
        assert!(screen.contains("Open this folder as a plain folder"));
        assert!(screen.contains("Members: app"));
        prompt.key(KeyCode::Down);
        assert!(render(&prompt).contains("memories do not carry over"));
    }

    /// `<dir>/ws.gaviero-workspace` listing `<dir>/app` (which may have its
    /// own settings) — the workspace layer is `<dir>/.gaviero/settings.json`.
    fn workspace_file(dir: &Path) -> PathBuf {
        let app = dir.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let file = dir.join("ws.gaviero-workspace");
        write_workspace_file(&file, &[app]).unwrap();
        file
    }

    #[test]
    fn an_opened_workspace_file_needs_settings_beside_it() {
        let dir = tempdir();
        let file = workspace_file(dir.path());
        let mode = LaunchMode::for_launch(Some(&file), false);
        assert_eq!(mode, LaunchMode::OpenWorkspace(file.clone()));
        assert!(needs_setup(&file, &mode));
        // A member's own settings do not configure the workspace.
        std::fs::create_dir_all(dir.path().join("app/.gaviero")).unwrap();
        std::fs::write(dir.path().join("app/.gaviero/settings.json"), "{}").unwrap();
        assert!(needs_setup(&file, &mode));
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(dir.path().join(".gaviero/settings.json"), "{}").unwrap();
        assert!(!needs_setup(&file, &mode));
        // `--workspace` finding that file resolves to the same mode.
        assert_eq!(
            LaunchMode::for_launch(Some(&file), true),
            LaunchMode::OpenWorkspace(file)
        );
        assert_eq!(LaunchMode::for_launch(None, true), LaunchMode::Workspace);
        assert_eq!(LaunchMode::for_launch(None, false), LaunchMode::Folder);
    }

    #[test]
    fn an_opened_workspace_offers_three_choices_and_writes_beside_the_file() {
        let dir = tempdir();
        let file = workspace_file(dir.path());
        let global = global_file(dir.path(), GLOBAL);

        let mut wizard = Wizard::new(&file, LaunchMode::OpenWorkspace(file.clone()), Some(global));
        assert_eq!(wizard.root, dir.path());
        assert_eq!(wizard.step, Step::Choice);
        assert_eq!(wizard.choice, Choice::EditorOnly);
        assert_eq!(wizard.choices().len(), 3);
        assert_eq!(wizard.handle_key(KeyCode::Char('3')), Flow::Apply);
        let outcome = wizard.apply().unwrap();
        assert_eq!(
            outcome,
            SetupOutcome {
                workspace_file: Some(file.clone()),
                editor_only: false
            }
        );
        assert_eq!(read(dir.path().join(".gaviero/settings.json")), GLOBAL);
        assert!(!dir.path().join("app/.gaviero").exists());
    }

    #[test]
    fn esc_on_an_opened_workspace_opens_it_editor_only_and_writes_nothing() {
        let dir = tempdir();
        let file = workspace_file(dir.path());
        let mut wizard = Wizard::new(&file, LaunchMode::OpenWorkspace(file.clone()), None);
        assert_eq!(wizard.handle_key(KeyCode::Esc), Flow::EditorOnly);
        assert_eq!(
            wizard.untouched_outcome(),
            SetupOutcome {
                workspace_file: Some(file.clone()),
                editor_only: true
            }
        );
        // Enter on the preselected editor-only option is the same.
        assert_eq!(wizard.handle_key(KeyCode::Enter), Flow::Apply);
        assert_eq!(wizard.apply().unwrap(), wizard.untouched_outcome());
        assert!(!dir.path().join(".gaviero").exists());
    }

    #[test]
    fn existing_workspace_file_prefers_dir_named_file() {
        let dir = tempdir();
        let named = dir
            .path()
            .join(format!("{}.gaviero-workspace", dir_name(dir.path())));
        std::fs::write(dir.path().join("aaa.gaviero-workspace"), "{}").unwrap();
        std::fs::write(&named, "{}").unwrap();
        assert_eq!(existing_workspace_file(dir.path()), Some(named));
    }

    #[test]
    fn discover_folders_preselects_git_repos_and_skips_noise() {
        let dir = tempdir();
        for name in ["alpha", "beta", "node_modules", ".hidden"] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("alpha/.git")).unwrap();
        std::fs::write(dir.path().join("loose.txt"), "x").unwrap();

        let found = discover_folders(dir.path());
        let names: Vec<&str> = found.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        assert!(found[0].selected, "git repo should be pre-selected");
        assert!(!found[1].selected, "plain folder should start unselected");
    }

    #[test]
    fn folder_mode_always_offers_three_choices_with_editor_only_preselected() {
        let dir = tempdir();
        let all = vec![Choice::EditorOnly, Choice::Minimal, Choice::Full];
        let without = Wizard::new(dir.path(), LaunchMode::Folder, None);
        assert_eq!(without.choice, Choice::EditorOnly);
        assert_eq!(without.choices(), all);

        // A path that does not exist counts as no global file.
        let missing = Wizard::new(
            dir.path(),
            LaunchMode::Folder,
            Some(dir.path().join("nope.json")),
        );
        assert!(missing.global.is_none());
        assert_eq!(missing.choices(), all);

        let global = global_file(dir.path(), GLOBAL);
        let with = Wizard::new(dir.path(), LaunchMode::Folder, Some(global));
        assert_eq!(with.choices(), all);
        assert!(with.global_has_secret);
    }

    #[test]
    fn full_without_a_global_file_writes_builtin_full_settings() {
        let dir = tempdir();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let mut wizard = Wizard::new(&root, LaunchMode::Folder, Some(dir.path().join("nope.json")));
        assert_eq!(wizard.handle_key(KeyCode::Char('3')), Flow::Apply);
        wizard.apply().unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&read(root.join(".gaviero/settings.json"))).unwrap();
        assert_eq!(doc["memory"]["namespace"], "proj");
        assert!(
            doc["agent"]["availableTools"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("Bash"))
        );
        assert!(doc["agent"]["permissions"]["bash"]["denylist"].is_array());
        assert!(doc["files"]["exclude"]["target"].as_bool().unwrap());
    }

    /// Draw the screen into a `width`×`height` buffer and return its text.
    fn screen(wizard: &Wizard, width: u16, height: u16) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|frame| wizard.render(frame)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn all_three_options_fit_a_short_narrow_pane() {
        let dir = tempdir();
        let root = dir.path().join("a-project-folder-with-a-fairly-long-name");
        std::fs::create_dir_all(&root).unwrap();
        let global = global_file(dir.path(), GLOBAL);
        for global in [Some(global), None] {
            let mut wizard = Wizard::new(&root, LaunchMode::Folder, global);
            for (width, height) in [(60, 12), (80, 24)] {
                // Whichever option is highlighted.
                for _ in 0..3 {
                    let text = screen(&wizard, width, height);
                    for title in ["1. Editor only", "2. Minimal settings", "3. Full settings"] {
                        assert!(text.contains(title), "{title:?} missing at {width}x{height}:\n{text}");
                    }
                    wizard.handle_key(KeyCode::Down);
                }
                wizard.choice = Choice::EditorOnly;
            }
        }
    }

    #[test]
    fn enter_and_esc_on_the_default_open_editor_only_and_write_nothing() {
        let dir = tempdir();
        let editor_only = SetupOutcome {
            workspace_file: None,
            editor_only: true,
        };
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Folder, None);
        assert_eq!(wizard.handle_key(KeyCode::Enter), Flow::Apply);
        assert_eq!(wizard.apply().unwrap(), editor_only);
        wizard.handle_key(KeyCode::Down);
        assert_eq!(wizard.handle_key(KeyCode::Esc), Flow::EditorOnly);
        assert_eq!(wizard.untouched_outcome(), editor_only);
        assert!(!dir.path().join(".gaviero").exists());
    }

    #[test]
    fn minimal_writes_an_empty_object() {
        let dir = tempdir();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Folder, None);
        assert_eq!(wizard.handle_key(KeyCode::Char('2')), Flow::Apply);
        assert_eq!(wizard.choice, Choice::Minimal);
        let outcome = wizard.apply().unwrap();
        assert!(!outcome.editor_only);
        assert!(outcome.workspace_file.is_none());
        assert_eq!(read(dir.path().join(".gaviero/settings.json")), "{}\n");
    }

    #[test]
    fn copy_is_byte_for_byte_including_the_relay_secret() {
        let dir = tempdir();
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        let global = global_file(dir.path(), GLOBAL);
        let mut wizard = Wizard::new(&root, LaunchMode::Folder, Some(global));
        assert_eq!(wizard.handle_key(KeyCode::Char('3')), Flow::Apply);
        wizard.apply().unwrap();
        assert_eq!(read(root.join(".gaviero/settings.json")), GLOBAL);
    }

    #[test]
    fn copy_of_a_global_file_without_secret_has_no_warning() {
        let dir = tempdir();
        let global = global_file(dir.path(), "{\"remote\":{\"relay\":{\"secret\":\"  \"}}}");
        let wizard = Wizard::new(dir.path(), LaunchMode::Folder, Some(global));
        assert!(!wizard.global_has_secret);
    }

    #[test]
    fn write_if_absent_never_clobbers() {
        let dir = tempdir();
        let path = dir.path().join(".gaviero/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"mine\":true}").unwrap();
        write_if_absent(&path, b"{}\n").unwrap();
        assert_eq!(read(path), "{\"mine\":true}");
    }

    #[test]
    fn creating_a_workspace_writes_its_settings_beside_the_workspace_file() {
        let dir = tempdir();
        for name in ["alpha", "beta"] {
            std::fs::create_dir_all(dir.path().join(name).join(".git")).unwrap();
        }
        let global = global_file(dir.path(), GLOBAL);
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Workspace, Some(global));
        assert_eq!(wizard.step, Step::Folders);
        wizard.folders[1].selected = false; // keep only `alpha`
        wizard.handle_key(KeyCode::Enter);
        assert_eq!(wizard.step, Step::Choice);
        assert_eq!(wizard.choices().len(), 3);
        assert_eq!(wizard.handle_key(KeyCode::Char('3')), Flow::Apply);

        let outcome = wizard.apply().unwrap();
        let ws_path = outcome.workspace_file.expect("workspace file written");
        assert_eq!(
            ws_path.file_name().unwrap().to_str().unwrap(),
            format!("{}.gaviero-workspace", dir_name(dir.path()))
        );
        let doc: serde_json::Value = serde_json::from_str(&read(ws_path.clone())).unwrap();
        assert_eq!(doc["folders"].as_array().unwrap().len(), 1);
        assert_eq!(doc["folders"][0]["name"], "alpha");
        assert_eq!(doc["settings"], serde_json::json!({}));
        // One workspace layer beside the file; members inherit it.
        assert_eq!(read(dir.path().join(".gaviero/settings.json")), GLOBAL);
        assert!(!dir.path().join("alpha/.gaviero").exists());
        // So the next launch on that file goes straight to the editor.
        assert!(!needs_setup(&ws_path, &LaunchMode::OpenWorkspace(ws_path.clone())));

        // And the file we just wrote must load back as a real workspace.
        let ws = gaviero_core::workspace::Workspace::load(&ws_path).unwrap();
        assert_eq!(ws.roots().len(), 1);
    }

    #[test]
    fn creating_a_workspace_editor_only_writes_just_the_workspace_file() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.path().join("alpha/.git")).unwrap();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Workspace, None);
        wizard.handle_key(KeyCode::Enter); // alpha is pre-selected
        assert_eq!(wizard.handle_key(KeyCode::Char('1')), Flow::Apply);
        let outcome = wizard.apply().unwrap();
        assert!(outcome.editor_only);
        let ws_path = outcome.workspace_file.expect("workspace file written");
        assert!(ws_path.is_file());
        assert!(!dir.path().join(".gaviero").exists());
        // Next launch on that file asks again.
        assert!(needs_setup(&ws_path, &LaunchMode::OpenWorkspace(ws_path.clone())));
    }

    #[test]
    fn builtin_full_settings_name_a_namespace_only_for_a_single_folder() {
        let dir = tempdir();
        let file = workspace_file(dir.path());
        let mut wizard = Wizard::new(
            &file,
            LaunchMode::OpenWorkspace(file.clone()),
            Some(dir.path().join("nope.json")),
        );
        wizard.choice = Choice::Full;
        wizard.apply().unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&read(dir.path().join(".gaviero/settings.json"))).unwrap();
        assert!(doc["memory"].get("namespace").is_none());
        assert!(doc["agent"]["availableTools"].is_array());
    }

    #[test]
    fn workspace_esc_goes_back_then_opens_editor_only() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.path().join("plain")).unwrap();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Workspace, None);
        wizard.handle_key(KeyCode::Enter); // nothing selected
        assert_eq!(wizard.step, Step::Folders);
        assert!(wizard.notice.is_some());
        wizard.handle_key(KeyCode::Char(' '));
        wizard.handle_key(KeyCode::Enter);
        assert_eq!(wizard.step, Step::Choice);
        assert_eq!(wizard.handle_key(KeyCode::Esc), Flow::Continue);
        assert_eq!(wizard.step, Step::Folders);
        assert_eq!(wizard.handle_key(KeyCode::Esc), Flow::EditorOnly);
    }

    #[test]
    fn workspace_apply_falls_back_to_launch_dir_when_no_subfolders() {
        let dir = tempdir();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Workspace, None);
        assert!(!wizard.has_folder_step(), "nothing to pick → skip the step");
        assert_eq!(wizard.step, Step::Choice);
        assert_eq!(wizard.handle_key(KeyCode::Char('2')), Flow::Apply);

        let outcome = wizard.apply().unwrap();
        let ws_path = outcome.workspace_file.unwrap();
        let doc: serde_json::Value = serde_json::from_str(&read(ws_path)).unwrap();
        assert_eq!(doc["folders"].as_array().unwrap().len(), 1);
        assert_eq!(read(dir.path().join(".gaviero/settings.json")), "{}\n");
    }
}
