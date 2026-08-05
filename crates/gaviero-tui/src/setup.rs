//! First-run setup wizard.
//!
//! Runs when a folder — or, with `--workspace`, a directory that has no
//! `.gaviero-workspace` file yet — carries no Gaviero configuration. It asks
//! for an agent profile, optionally which sub-folders join the workspace, and
//! whether to write the Claude / Codex / Cursor MCP configs, then materializes
//! `.gaviero/settings.json` (and the workspace file) before the editor starts.
//!
//! The wizard owns its own terminal session: raw mode + the alternate screen
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

/// How the TUI was launched, which decides whether the wizard asks for
/// workspace members and what it writes at the end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchMode {
    /// Single folder — writes `<root>/.gaviero/settings.json`.
    Folder,
    /// `--workspace` — additionally writes `<root>/<name>.gaviero-workspace`.
    Workspace,
}

/// Agent capability preset chosen on the first step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Profile {
    /// Everything on: shell plus the full edit tool surface, auto-approved.
    Full,
    /// No shell at all — the agent can read, search, and propose edits, but
    /// `Bash` is absent from the tool surface entirely.
    Restricted,
}

impl Profile {
    fn title(self) -> &'static str {
        match self {
            Profile::Full => "Full capabilities",
            Profile::Restricted => "Restricted (no shell)",
        }
    }

    fn detail(self) -> &'static str {
        match self {
            Profile::Full => {
                "Read/Glob/Grep, Write/Edit/MultiEdit and Bash, all auto-approved. \
                 Shell runs under an allow/deny list. Same shape as the gaviero repo itself."
            }
            Profile::Restricted => {
                "Read/Glob/Grep and Write/Edit/MultiEdit, auto-approved. Bash is not \
                 offered to the agent at all, so no command can run. File edits still \
                 pass through the Write Gate for review."
            }
        }
    }
}

/// Which wizard screen is on-stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Profile,
    Folders,
    Providers,
    Confirm,
}

/// A candidate workspace member found under the launch directory.
struct FolderCandidate {
    path: PathBuf,
    name: String,
    is_git: bool,
    selected: bool,
}

/// What the wizard decided, handed back to `main` so it can build the
/// `Workspace` and (optionally) write provider configs.
pub struct SetupOutcome {
    /// Path of the `.gaviero-workspace` file the wizard created, if any.
    /// `None` means single-folder mode — open the launch directory.
    pub workspace_file: Option<PathBuf>,
    /// User asked for Claude / Codex / Cursor MCP configs to be written.
    pub init_providers: bool,
}

/// Does this launch target still need first-run setup?
///
/// Folder mode is "configured" once `<root>/.gaviero/settings.json` exists;
/// workspace mode once any `*.gaviero-workspace` file sits in the directory.
pub fn needs_setup(root: &Path, mode: LaunchMode) -> bool {
    match mode {
        LaunchMode::Folder => !root.join(".gaviero").join("settings.json").exists(),
        LaunchMode::Workspace => existing_workspace_file(root).is_none(),
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
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|ext| ext == "gaviero-workspace")
        })
        .collect();
    matches.sort();
    matches.into_iter().next()
}

/// Run the wizard. Returns `None` when the user cancelled (Esc on the first
/// screen) — the caller then opens the folder with built-in defaults and
/// writes nothing.
pub fn run(root: &Path, mode: LaunchMode) -> Result<Option<SetupOutcome>> {
    let mut wizard = Wizard::new(root, mode);

    enable_raw_mode().context("enabling raw mode for setup wizard")?;
    let mut stdout = std::io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(e).context("entering alternate screen for setup wizard");
    }

    let result = (|| -> Result<bool> {
        let backend = ratatui::backend::CrosstermBackend::new(std::io::stdout());
        let mut terminal = ratatui::Terminal::new(backend).context("creating setup terminal")?;
        loop {
            terminal.draw(|frame| wizard.render(frame))?;
            let TermEvent::Key(key) = event::read()? else {
                continue;
            };
            if key.kind == KeyEventKind::Release {
                continue;
            }
            match wizard.handle_key(key.code) {
                Flow::Continue => {}
                Flow::Cancel => return Ok(false),
                Flow::Apply => return Ok(true),
            }
        }
    })();

    // Restore the terminal on every path before reporting anything, so an
    // error message from the wizard lands on a usable screen.
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
    let _ = disable_raw_mode();

    if !result? {
        return Ok(None);
    }
    wizard.apply().map(Some)
}

/// Write the Claude / Codex / Cursor MCP configs for every workspace root.
///
/// All folders share the primary root's endpoint — one gaviero process serves
/// the whole workspace — and Codex trust is granted, which is what makes
/// `.codex/config.toml` get written at all. Best-effort: a failing folder is
/// logged and the rest still run.
pub fn synthesize_provider_configs(
    workspace: &gaviero_core::workspace::Workspace,
) -> Vec<PathBuf> {
    use gaviero_core::mcp;

    let roots: Vec<PathBuf> = workspace.roots().iter().map(|r| r.to_path_buf()).collect();
    let Some(primary) = roots.first() else {
        return Vec::new();
    };
    let endpoint = mcp::McpEndpoint::for_workspace(primary);

    let mut written = Vec::new();
    for root in &roots {
        let overrides = mcp::McpConfigOverrides {
            codex_trust: Some(mcp::TrustConsent::Granted),
            ..Default::default()
        };
        let synth = mcp::resolve_mcp_config_synth(workspace, root, endpoint.clone(), &overrides);
        match mcp::synthesize_for_worktree(&synth) {
            Ok(paths) => written.extend(paths),
            Err(e) => tracing::warn!(
                target: "mcp_server",
                error = %e,
                root = %root.display(),
                "first-run provider config synthesis failed"
            ),
        }
    }
    written
}

/// What `handle_key` asks the event loop to do next.
enum Flow {
    Continue,
    Cancel,
    Apply,
}

struct Wizard {
    root: PathBuf,
    mode: LaunchMode,
    step: Step,
    profile: Profile,
    folders: Vec<FolderCandidate>,
    folder_cursor: usize,
    init_providers: bool,
    /// Inline complaint shown under the current step (e.g. "select a folder").
    notice: Option<String>,
}

impl Wizard {
    fn new(root: &Path, mode: LaunchMode) -> Self {
        let folders = if mode == LaunchMode::Workspace {
            discover_folders(root)
        } else {
            Vec::new()
        };
        Self {
            root: root.to_path_buf(),
            mode,
            step: Step::Profile,
            profile: Profile::Full,
            folders,
            folder_cursor: 0,
            init_providers: true,
            notice: None,
        }
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
            Step::Profile => match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.profile = Profile::Full;
                    Flow::Continue
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.profile = Profile::Restricted;
                    Flow::Continue
                }
                KeyCode::Char('1') => {
                    self.profile = Profile::Full;
                    self.advance();
                    Flow::Continue
                }
                KeyCode::Char('2') => {
                    self.profile = Profile::Restricted;
                    self.advance();
                    Flow::Continue
                }
                KeyCode::Enter => {
                    self.advance();
                    Flow::Continue
                }
                KeyCode::Esc => Flow::Cancel,
                _ => Flow::Continue,
            },
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
                        self.advance();
                    }
                    Flow::Continue
                }
                KeyCode::Esc => {
                    self.step = Step::Profile;
                    Flow::Continue
                }
                _ => Flow::Continue,
            },
            Step::Providers => match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.init_providers = true;
                    Flow::Continue
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.init_providers = false;
                    Flow::Continue
                }
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.init_providers = true;
                    self.advance();
                    Flow::Continue
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.init_providers = false;
                    self.advance();
                    Flow::Continue
                }
                KeyCode::Enter => {
                    self.advance();
                    Flow::Continue
                }
                KeyCode::Esc => {
                    self.step = if self.has_folder_step() {
                        Step::Folders
                    } else {
                        Step::Profile
                    };
                    Flow::Continue
                }
                _ => Flow::Continue,
            },
            Step::Confirm => match code {
                KeyCode::Enter => Flow::Apply,
                KeyCode::Esc => {
                    self.step = Step::Providers;
                    Flow::Continue
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => Flow::Cancel,
                _ => Flow::Continue,
            },
        }
    }

    fn advance(&mut self) {
        self.step = match self.step {
            Step::Profile if self.has_folder_step() => Step::Folders,
            Step::Profile => Step::Providers,
            Step::Folders => Step::Providers,
            Step::Providers => Step::Confirm,
            Step::Confirm => Step::Confirm,
        };
    }

    /// Folders that will end up in the workspace file. Falls back to the
    /// launch directory when there was nothing to pick.
    fn workspace_members(&self) -> Vec<PathBuf> {
        if self.folders.is_empty() {
            vec![self.root.clone()]
        } else {
            self.selected_folders().iter().map(|f| f.path.clone()).collect()
        }
    }

    fn workspace_file_path(&self) -> PathBuf {
        self.root
            .join(format!("{}.gaviero-workspace", dir_name(&self.root)))
    }

    /// Write every file the wizard promised on the confirm screen.
    fn apply(&self) -> Result<SetupOutcome> {
        match self.mode {
            LaunchMode::Folder => {
                let mut settings = base_settings(&dir_name(&self.root));
                merge_into(&mut settings, profile_settings(self.profile, self.init_providers));
                write_json_if_absent(&self.root.join(".gaviero").join("settings.json"), &settings)?;
                Ok(SetupOutcome {
                    workspace_file: None,
                    init_providers: self.init_providers,
                })
            }
            LaunchMode::Workspace => {
                let members = self.workspace_members();
                // Profile settings live at workspace level so every folder
                // inherits one policy; each folder keeps its own namespace.
                let workspace_settings = profile_settings(self.profile, self.init_providers);
                let path = self.workspace_file_path();
                write_workspace_file(&path, &members, &workspace_settings)?;
                for member in &members {
                    let settings = base_settings(&dir_name(member));
                    write_json_if_absent(&member.join(".gaviero").join("settings.json"), &settings)?;
                }
                Ok(SetupOutcome {
                    workspace_file: Some(path),
                    init_providers: self.init_providers,
                })
            }
        }
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
            match self.mode {
                LaunchMode::Folder => "  No configuration found — creating one.".to_string(),
                LaunchMode::Workspace => {
                    "  No .gaviero-workspace found — creating one.".to_string()
                }
            },
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(""));

        match self.step {
            Step::Profile => self.render_profile(&mut lines),
            Step::Folders => self.render_folders(&mut lines, inner.height),
            Step::Providers => self.render_providers(&mut lines),
            Step::Confirm => self.render_confirm(&mut lines),
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

    fn footer(&self) -> &'static str {
        match self.step {
            Step::Profile => "↑/↓ choose · 1/2 pick directly · Enter next · Esc skip setup",
            Step::Folders => "↑/↓ move · Space toggle · a all/none · Enter next · Esc back",
            Step::Providers => "↑/↓ or y/n choose · Enter next · Esc back",
            Step::Confirm => "Enter write these files · Esc back · q skip setup",
        }
    }

    fn render_profile(&self, lines: &mut Vec<Line>) {
        lines.push(Line::from(Span::styled(
            "  Agent profile",
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        for (idx, profile) in [Profile::Full, Profile::Restricted].iter().enumerate() {
            let active = *profile == self.profile;
            let marker = if active { "▸" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("  {marker} {}. {}", idx + 1, profile.title()),
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
            lines.push(Line::from(Span::styled(
                format!("       {}", profile.detail()),
                Style::default().fg(theme::TEXT_DIM),
            )));
        }
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
            let cursor = if idx == self.folder_cursor { "▸" } else { " " };
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

    fn render_providers(&self, lines: &mut Vec<Line>) {
        lines.push(Line::from(Span::styled(
            "  Coding-agent configuration",
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            "  Writes .mcp.json, .claude/settings.json, .cursor/mcp.json and",
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  .codex/config.toml so Claude Code, Cursor and Codex can reach",
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  Gaviero's read-only MCP tools. Codex needs trust granted here;",
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(Span::styled(
            "  answering no defers that to the first swarm run.",
            Style::default().fg(theme::TEXT_DIM),
        )));
        lines.push(Line::from(""));
        for (yes, label) in [
            (true, "Yes — set up Claude, Codex and Cursor now"),
            (false, "No — configure them later"),
        ] {
            let active = yes == self.init_providers;
            let marker = if active { "▸" } else { " " };
            lines.push(Line::from(Span::styled(
                format!("  {marker} {label}"),
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
    }

    fn render_confirm(&self, lines: &mut Vec<Line>) {
        lines.push(Line::from(Span::styled(
            "  Ready to write",
            Style::default()
                .fg(theme::TEXT_BRIGHT)
                .add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("  Profile: {}", self.profile.title()),
            Style::default().fg(theme::TEXT_FG),
        )));

        let mut files: Vec<String> = Vec::new();
        match self.mode {
            LaunchMode::Folder => {
                files.push(format!("{}", self.root.join(".gaviero/settings.json").display()));
            }
            LaunchMode::Workspace => {
                files.push(format!("{}", self.workspace_file_path().display()));
                for member in self.workspace_members() {
                    files.push(format!("{}", member.join(".gaviero/settings.json").display()));
                }
            }
        }
        if self.init_providers {
            let targets: Vec<PathBuf> = match self.mode {
                LaunchMode::Folder => vec![self.root.clone()],
                LaunchMode::Workspace => self.workspace_members(),
            };
            for target in targets {
                files.push(format!(
                    "{}{}{{.mcp.json, .claude/, .cursor/, .codex/}}",
                    target.display(),
                    std::path::MAIN_SEPARATOR
                ));
            }
        }

        lines.push(Line::from(""));
        for file in files {
            lines.push(Line::from(Span::styled(
                format!("    {file}"),
                Style::default().fg(theme::CODE_GREEN),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  Existing files are never overwritten.",
            Style::default().fg(theme::TEXT_DIM),
        )));
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

/// The profile-independent block: the same defaults `Workspace::ensure_settings`
/// writes, so a wizard-created file is indistinguishable from the implicit one.
fn base_settings(namespace: &str) -> serde_json::Value {
    serde_json::json!({
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
            "namespace": namespace,
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
    })
}

/// The agent + MCP block that differs per profile.
///
/// Both profiles keep the destructive-command denylist: on `Restricted` it is
/// dead weight while `Bash` is absent from `availableTools`, but it means
/// adding the tool back later does not silently arrive without guardrails.
fn profile_settings(profile: Profile, codex_trust_granted: bool) -> serde_json::Value {
    let denylist = serde_json::json!([
        "terraform destroy",
        "npm publish",
        "git push --force",
        "git push -f",
        "chmod 777",
        "mkfs.",
        "dd if="
    ]);

    let agent = match profile {
        Profile::Full => serde_json::json!({
            "availableTools": [
                "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit", "Bash",
                "AskUserQuestion", "mcp__gaviero"
            ],
            "approvedTools": [
                "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit", "Bash",
                "AskUserQuestion", "mcp__gaviero"
            ],
            "permissions": {
                "bash": {
                    "denylist": denylist,
                    "allowlist": [
                        "cargo check", "cargo test", "cargo build", "cargo clippy",
                        "git status", "git diff", "git log", "git show",
                        "ls", "cat ", "rg ", "grep ", "find ", "head ", "tail ",
                        "wc ", "pwd", "echo "
                    ]
                }
            }
        }),
        Profile::Restricted => serde_json::json!({
            "availableTools": [
                "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit",
                "AskUserQuestion", "mcp__gaviero"
            ],
            "approvedTools": [
                "Read", "Glob", "Grep", "Write", "Edit", "MultiEdit",
                "AskUserQuestion", "mcp__gaviero"
            ],
            "permissions": {
                "bash": { "denylist": denylist }
            }
        }),
    };

    let mut out = serde_json::json!({ "agent": agent });
    if codex_trust_granted {
        out["mcp"] = serde_json::json!({ "gavieroServer": { "codexTrust": "granted" } });
    }
    out
}

/// Shallow merge of top-level keys — the two settings blocks never overlap,
/// so a per-key overwrite is enough.
fn merge_into(target: &mut serde_json::Value, source: serde_json::Value) {
    let (Some(target), serde_json::Value::Object(source)) = (target.as_object_mut(), source) else {
        return;
    };
    for (key, value) in source {
        target.insert(key, value);
    }
}

fn write_json_if_absent(path: &Path, value: &serde_json::Value) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(value)?;
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    tracing::info!("first-run setup wrote {}", path.display());
    Ok(())
}

/// Write the `.gaviero-workspace` file. Folder paths are absolute: they are
/// used as roots verbatim by `Workspace::load`, which never resolves them
/// against the workspace file's directory.
fn write_workspace_file(
    path: &Path,
    members: &[PathBuf],
    settings: &serde_json::Value,
) -> Result<()> {
    let folders: Vec<serde_json::Value> = members
        .iter()
        .map(|p| serde_json::json!({ "path": p, "name": dir_name(p) }))
        .collect();
    let doc = serde_json::json!({ "folders": folders, "settings": settings });
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

    #[test]
    fn needs_setup_folder_mode_tracks_settings_file() {
        let dir = tempdir();
        assert!(needs_setup(dir.path(), LaunchMode::Folder));
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(dir.path().join(".gaviero/settings.json"), "{}").unwrap();
        assert!(!needs_setup(dir.path(), LaunchMode::Folder));
    }

    #[test]
    fn needs_setup_workspace_mode_tracks_workspace_file() {
        let dir = tempdir();
        assert!(needs_setup(dir.path(), LaunchMode::Workspace));
        std::fs::write(dir.path().join("anything.gaviero-workspace"), "{}").unwrap();
        assert!(!needs_setup(dir.path(), LaunchMode::Workspace));
        // A per-folder settings.json alone must not count as a configured
        // workspace — the workspace file is what `--workspace` needs.
        let other = tempdir();
        std::fs::create_dir_all(other.path().join(".gaviero")).unwrap();
        std::fs::write(other.path().join(".gaviero/settings.json"), "{}").unwrap();
        assert!(needs_setup(other.path(), LaunchMode::Workspace));
    }

    #[test]
    fn existing_workspace_file_prefers_dir_named_file() {
        let dir = tempdir();
        let named = dir.path().join(format!("{}.gaviero-workspace", dir_name(dir.path())));
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
    fn restricted_profile_drops_bash_from_the_tool_surface() {
        let full = profile_settings(Profile::Full, false);
        let restricted = profile_settings(Profile::Restricted, false);

        let tools = |v: &serde_json::Value, key: &str| -> Vec<String> {
            v["agent"][key]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| t.as_str().unwrap().to_string())
                .collect()
        };
        assert!(tools(&full, "availableTools").contains(&"Bash".to_string()));
        assert!(tools(&full, "approvedTools").contains(&"Bash".to_string()));
        assert!(!tools(&restricted, "availableTools").contains(&"Bash".to_string()));
        assert!(!tools(&restricted, "approvedTools").contains(&"Bash".to_string()));
        // Edits stay on the surface in both — the Write Gate reviews them.
        assert!(tools(&restricted, "availableTools").contains(&"Edit".to_string()));
        // The denylist survives so re-adding Bash is not unguarded.
        assert!(restricted["agent"]["permissions"]["bash"]["denylist"].is_array());
    }

    #[test]
    fn codex_trust_is_granted_only_when_providers_are_initialized() {
        assert_eq!(
            profile_settings(Profile::Full, true)["mcp"]["gavieroServer"]["codexTrust"],
            serde_json::json!("granted")
        );
        assert!(profile_settings(Profile::Full, false).get("mcp").is_none());
    }

    #[test]
    fn folder_apply_writes_merged_settings() {
        let dir = tempdir();
        let wizard = Wizard::new(dir.path(), LaunchMode::Folder);
        let outcome = wizard.apply().unwrap();
        assert!(outcome.workspace_file.is_none());

        let body =
            std::fs::read_to_string(dir.path().join(".gaviero/settings.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(doc["memory"]["namespace"], dir_name(dir.path()).as_str());
        assert!(doc["files"]["exclude"]["target"].as_bool().unwrap());
        assert!(doc["agent"]["availableTools"].is_array());
    }

    #[test]
    fn workspace_apply_writes_file_and_per_folder_settings() {
        let dir = tempdir();
        for name in ["alpha", "beta"] {
            std::fs::create_dir_all(dir.path().join(name).join(".git")).unwrap();
        }
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Workspace);
        wizard.profile = Profile::Restricted;
        wizard.folders[1].selected = false; // keep only `alpha`

        let outcome = wizard.apply().unwrap();
        let ws_path = outcome.workspace_file.expect("workspace file written");
        assert_eq!(
            ws_path.file_name().unwrap().to_str().unwrap(),
            format!("{}.gaviero-workspace", dir_name(dir.path()))
        );

        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&ws_path).unwrap()).unwrap();
        assert_eq!(doc["folders"].as_array().unwrap().len(), 1);
        assert_eq!(doc["folders"][0]["name"], "alpha");
        // Profile lives at workspace level, namespaces stay per folder.
        assert!(doc["settings"]["agent"]["availableTools"].is_array());
        assert!(dir.path().join("alpha/.gaviero/settings.json").exists());
        assert!(!dir.path().join("beta/.gaviero/settings.json").exists());

        // And the file we just wrote must load back as a real workspace.
        let ws = gaviero_core::workspace::Workspace::load(&ws_path).unwrap();
        assert_eq!(ws.roots().len(), 1);
    }

    #[test]
    fn workspace_apply_falls_back_to_launch_dir_when_no_subfolders() {
        let dir = tempdir();
        let wizard = Wizard::new(dir.path(), LaunchMode::Workspace);
        assert!(!wizard.has_folder_step(), "nothing to pick → skip the step");

        let outcome = wizard.apply().unwrap();
        let ws_path = outcome.workspace_file.unwrap();
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&ws_path).unwrap()).unwrap();
        assert_eq!(doc["folders"].as_array().unwrap().len(), 1);
        assert!(dir.path().join(".gaviero/settings.json").exists());
    }

    #[test]
    fn write_json_if_absent_never_clobbers() {
        let dir = tempdir();
        let path = dir.path().join(".gaviero/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{\"mine\":true}").unwrap();
        write_json_if_absent(&path, &serde_json::json!({"mine": false})).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"mine\":true}");
    }

    #[test]
    fn wizard_flow_skips_folder_step_in_folder_mode() {
        let dir = tempdir();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Folder);
        wizard.handle_key(KeyCode::Enter);
        assert_eq!(wizard.step, Step::Providers);
        wizard.handle_key(KeyCode::Enter);
        assert_eq!(wizard.step, Step::Confirm);
        assert!(matches!(wizard.handle_key(KeyCode::Enter), Flow::Apply));
    }

    #[test]
    fn folder_step_requires_a_selection() {
        let dir = tempdir();
        std::fs::create_dir_all(dir.path().join("plain")).unwrap();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Workspace);
        wizard.handle_key(KeyCode::Enter); // profile → folders
        assert_eq!(wizard.step, Step::Folders);

        wizard.handle_key(KeyCode::Enter); // nothing selected
        assert_eq!(wizard.step, Step::Folders);
        assert!(wizard.notice.is_some());

        wizard.handle_key(KeyCode::Char(' '));
        wizard.handle_key(KeyCode::Enter);
        assert_eq!(wizard.step, Step::Providers);
    }

    #[test]
    fn esc_on_first_step_cancels_setup() {
        let dir = tempdir();
        let mut wizard = Wizard::new(dir.path(), LaunchMode::Folder);
        assert!(matches!(wizard.handle_key(KeyCode::Esc), Flow::Cancel));
    }
}
