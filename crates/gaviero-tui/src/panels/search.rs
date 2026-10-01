use std::path::{Path, PathBuf};

use crate::theme;
use crate::widgets::scroll_state::ScrollState;
use crate::widgets::text_input::TextInput;
use ratatui::{buffer::Buffer, layout::Rect, style::Style};

/// A single search result: file path + line number + matching line text.
/// File-name matches carry `line_number == 0` and an empty `line_text`.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Path relative to the workspace root it was found under, prefixed with
    /// that root's folder name in multi-folder workspaces (display only).
    pub path: PathBuf,
    /// Absolute path to open — `path` alone can't say which root it came
    /// from in a multi-folder workspace.
    pub abs_path: PathBuf,
    pub line_number: usize,
    pub line_text: String,
}

/// What the search panel matches the query against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchMode {
    /// Occurrences of the query inside file contents.
    #[default]
    Content,
    /// File names (or relative paths, when the query contains `/`).
    FileName,
}

/// State for the workspace search panel.
pub struct SearchPanelState {
    /// The interactive input field at the top of the panel.
    pub input: TextInput,
    pub mode: SearchMode,
    /// The query that produced the current result set (may lag behind `input`
    /// while a debounce timer is pending).
    pub query: String,
    pub results: Vec<SearchResult>,
    pub scroll: ScrollState,
    pub searching: bool,
    /// When true the text input has keyboard focus; when false the results
    /// list does (arrow keys navigate results).
    pub editing: bool,
    /// Searchable files, walked once and reused by every keystroke until
    /// `invalidate_file_index` (file tree / `.gitignore` change) or the
    /// roots / excludes it was built from change.
    file_index: Option<FileIndex>,
}

/// One searchable file.
#[derive(Debug, Clone)]
struct IndexedFile {
    /// Root-relative path, folder-prefixed in multi-folder workspaces.
    display: PathBuf,
    /// `display` '/'-joined and lowercased, for file-name matching.
    display_lower: String,
    abs: PathBuf,
}

struct FileIndex {
    roots: Vec<PathBuf>,
    excludes: Vec<String>,
    files: Vec<IndexedFile>,
}

impl FileIndex {
    fn build(roots: &[&Path], excludes: &[String]) -> Self {
        let mut files = Vec::new();
        for &root in roots {
            // Same label the file tree shows for each root; omitted when
            // there is only one root, where it would be noise.
            let label =
                (roots.len() > 1).then(|| root.file_name().map(Path::new).unwrap_or(root));
            let git = RootGit::open(root);
            collect_files(root, root, label, excludes, git.as_ref(), &mut files);
        }
        Self {
            roots: roots.iter().map(|r| r.to_path_buf()).collect(),
            excludes: excludes.to_vec(),
            files,
        }
    }

    fn matches(&self, roots: &[&Path], excludes: &[String]) -> bool {
        self.excludes == excludes
            && self
                .roots
                .iter()
                .map(PathBuf::as_path)
                .eq(roots.iter().copied())
    }
}

/// The git repo enclosing a workspace root, for pruning ignored folders.
struct RootGit {
    repo: gaviero_core::git::GitRepo,
    /// The root's '/'-joined path inside the repo workdir ("" at the top).
    prefix: String,
}

impl RootGit {
    fn open(root: &Path) -> Option<Self> {
        let repo = gaviero_core::git::GitRepo::open(root).ok()?;
        // Canonicalize both sides: the workspace root and libgit2's workdir
        // can differ in separators, casing, or `\\?\` prefixes on Windows.
        let workdir = std::fs::canonicalize(repo.workdir()?).ok()?;
        let root = std::fs::canonicalize(root).ok()?;
        let prefix = rel_path_string(root.strip_prefix(&workdir).ok()?);
        Some(Self { repo, prefix })
    }

    fn is_ignored(&self, rel_str: &str) -> bool {
        if self.prefix.is_empty() {
            self.repo.is_path_ignored(rel_str)
        } else {
            self.repo.is_path_ignored(&format!("{}/{}", self.prefix, rel_str))
        }
    }
}

fn collect_files(
    root: &Path,
    dir: &Path,
    label: Option<&Path>,
    excludes: &[String],
    git: Option<&RootGit>,
    out: &mut Vec<IndexedFile>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path);
        // '/'-joined for the exclude matcher: native separators (Windows
        // `\`) never match the '/'-literal pattern grammar. Uses the same
        // gitignore-style matcher as the file watcher and file list —
        // search previously had its own weaker equality/prefix variant.
        let rel_str = rel_path_string(rel);
        if crate::app::matches_exclude(&rel_str, excludes) {
            continue;
        }

        // Skip the components the file watcher always drops, plus every
        // dotfile — search policy is to never descend hidden dirs (the
        // watcher, by contrast, still reports non-`.git` dot-dirs).
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.')
            || crate::event::ALWAYS_SKIP_COMPONENTS.contains(&name_str.as_ref())
        {
            continue;
        }

        // `DirEntry::file_type` is free on most platforms; only symlinks
        // need the extra stat to learn what they point at.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let (is_dir, is_file) = if file_type.is_symlink() {
            (path.is_dir(), path.is_file())
        } else {
            (file_type.is_dir(), file_type.is_file())
        };

        if is_dir {
            // Never descend gitignored folders (`tmp/`, `/target/`, …):
            // walking them is what made search block the TUI.
            if git.is_some_and(|g| g.is_ignored(&rel_str)) {
                continue;
            }
            collect_files(root, &path, label, excludes, git, out);
        } else if is_file {
            let display = match label {
                Some(label) => label.join(rel),
                None => rel.to_path_buf(),
            };
            let display_lower = rel_path_string(&display).to_lowercase();
            out.push(IndexedFile {
                display,
                display_lower,
                abs: path,
            });
        }
    }
}

impl SearchPanelState {
    pub fn new() -> Self {
        Self {
            input: TextInput::new(),
            mode: SearchMode::default(),
            query: String::new(),
            results: Vec::new(),
            scroll: ScrollState::new(),
            searching: false,
            editing: true,
            file_index: None,
        }
    }

    /// Drop the cached file list so the next search re-walks the workspace.
    pub fn invalidate_file_index(&mut self) {
        self.file_index = None;
    }

    /// Focus the input field (called when switching to the search panel).
    pub fn focus_input(&mut self) {
        self.editing = true;
    }

    /// Switch between content and file-name search.
    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            SearchMode::Content => SearchMode::FileName,
            SearchMode::FileName => SearchMode::Content,
        };
    }

    /// Start a new search. Clears previous results.
    pub fn search(&mut self, query: &str, roots: &[&Path], excludes: &[String]) {
        self.query = query.to_string();
        self.results.clear();
        self.scroll.reset();
        self.searching = true;

        if !query.trim().is_empty() {
            if !self
                .file_index
                .as_ref()
                .is_some_and(|index| index.matches(roots, excludes))
            {
                self.file_index = Some(FileIndex::build(roots, excludes));
            }
            match self.mode {
                SearchMode::Content => self.search_contents(),
                SearchMode::FileName => self.search_file_names(),
            }
        }
        self.searching = false;
    }

    /// Run a search using the current input text. Called on every keystroke.
    pub fn search_from_input(&mut self, roots: &[&Path], excludes: &[String]) {
        let query = self.input.text.clone();
        self.search(&query, roots, excludes);
    }

    fn search_contents(&mut self) {
        let Some(index) = &self.file_index else {
            return;
        };
        let query_lower = self.query.to_lowercase();
        for file in &index.files {
            // Non-UTF-8 (binary) files fail to read and are skipped.
            let Ok(content) = std::fs::read_to_string(&file.abs) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&query_lower) {
                    self.results.push(SearchResult {
                        path: file.display.clone(),
                        abs_path: file.abs.clone(),
                        line_number: i + 1,
                        line_text: line.trim().to_string(),
                    });
                }
            }
        }
    }

    fn search_file_names(&mut self) {
        let Some(index) = &self.file_index else {
            return;
        };
        let query_lower = self.query.trim().to_lowercase();
        let mut hits: Vec<(u8, &IndexedFile)> = index
            .files
            .iter()
            .filter_map(|f| file_name_rank(&f.display_lower, &query_lower).map(|rank| (rank, f)))
            .collect();
        // Best matches first: exact name, name prefix, name substring, then
        // path-only hits; shorter paths win ties.
        hits.sort_by(|(rank_a, a), (rank_b, b)| {
            rank_a
                .cmp(rank_b)
                .then(a.display_lower.len().cmp(&b.display_lower.len()))
                .then_with(|| a.display_lower.cmp(&b.display_lower))
        });
        self.results = hits
            .into_iter()
            .map(|(_, f)| SearchResult {
                path: f.display.clone(),
                abs_path: f.abs.clone(),
                line_number: 0,
                line_text: String::new(),
            })
            .collect();
    }

    /// Get the selected result.
    pub fn selected_result(&self) -> Option<&SearchResult> {
        self.results.get(self.scroll.selected)
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer, focused: bool) {
        let bg = theme::PANEL_BG;
        let fg = theme::TEXT_FG;
        let sel_bg = if focused && !self.editing {
            theme::FOCUSED_SELECTION_BG
        } else {
            theme::DARK_BG
        };

        // Clear area
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].set_char(' ').set_style(Style::default().bg(bg));
            }
        }

        // ── Input field (row 0) ─────────────────────────────────
        let input_y = area.y;
        let input_bg = if focused && self.editing {
            theme::INPUT_BG
        } else {
            bg
        };
        let prompt = " \u{1F50D} "; // 🔍 magnifying glass + space
        let prompt_style = Style::default().fg(theme::TEXT_DIM).bg(input_bg);

        // Clear input row
        for x in area.x..area.right() {
            if input_y < buf.area().bottom() {
                buf[(x, input_y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(input_bg));
            }
        }

        // Draw prompt
        let mut x = area.x;
        for ch in prompt.chars() {
            if x < area.right() && input_y < buf.area().bottom() {
                buf[(x, input_y)].set_char(ch).set_style(prompt_style);
                x += ch.len_utf8() as u16; // emoji takes 1 cell in our buffer but let's advance properly
            }
        }
        // The emoji is wide; use a simpler prompt for reliable column math
        let prompt_cols: u16 = 3; // " > " / " @ "
        x = area.x;
        // Re-render with a simple text prompt for reliable positioning
        let prompt = match self.mode {
            SearchMode::Content => " > ",
            SearchMode::FileName => " @ ",
        };
        for ch in prompt.chars() {
            if x < area.right() && input_y < buf.area().bottom() {
                buf[(x, input_y)].set_char(ch).set_style(prompt_style);
            }
            x += 1;
        }
        let text_x = area.x + prompt_cols;

        // Draw input text or placeholder
        if self.input.is_empty() && !(focused && self.editing) {
            let hint = match self.mode {
                SearchMode::Content => "type to search in files...  (Tab: file names)",
                SearchMode::FileName => "type a file name...  (Tab: contents)",
            };
            let hint_style = Style::default().fg(theme::TEXT_DIM).bg(input_bg);
            let mut hx = text_x;
            for ch in hint.chars() {
                if hx >= area.right() {
                    break;
                }
                if input_y < buf.area().bottom() {
                    buf[(hx, input_y)].set_char(ch).set_style(hint_style);
                }
                hx += 1;
            }
        } else {
            let input_style = Style::default().fg(fg).bg(input_bg);
            let mut ix = text_x;
            for ch in self.input.text.chars() {
                if ix >= area.right() {
                    break;
                }
                if input_y < buf.area().bottom() {
                    buf[(ix, input_y)].set_char(ch).set_style(input_style);
                }
                ix += 1;
            }
        }

        // Cursor
        if focused && self.editing {
            let cursor_x = text_x + self.input.cursor as u16;
            if cursor_x < area.right() && input_y < buf.area().bottom() {
                let cursor_style = Style::default().fg(input_bg).bg(theme::TEXT_FG);
                buf[(cursor_x, input_y)].set_style(cursor_style);
            }
        }

        // ── Summary line (row 1) ────────────────────────────────
        let summary_y = area.y + 1;
        if summary_y < area.bottom() {
            let summary = match (self.mode, self.query.is_empty(), self.results.len()) {
                (SearchMode::Content, true, _) => " Searching file contents".to_string(),
                (SearchMode::FileName, true, _) => " Searching file names".to_string(),
                (SearchMode::Content, false, 0) => format!(" No results for '{}'", self.query),
                (SearchMode::FileName, false, 0) => format!(" No files matching '{}'", self.query),
                (SearchMode::Content, false, n) => format!(" {} results", n),
                (SearchMode::FileName, false, n) => format!(" {} files", n),
            };
            let summary_style = Style::default()
                .fg(if self.results.is_empty() {
                    theme::TEXT_DIM
                } else {
                    theme::WARNING
                })
                .bg(bg);
            for (i, ch) in summary.chars().enumerate() {
                let sx = area.x + i as u16;
                if sx < area.right() && summary_y < buf.area().bottom() {
                    buf[(sx, summary_y)].set_char(ch).set_style(summary_style);
                }
            }
        }

        // ── Results list (row 2+) ───────────────────────────────
        let results_start = area.y + 2;
        let viewport = (area.height as usize).saturating_sub(2);
        self.scroll.set_viewport(viewport);
        self.scroll.ensure_visible_on_render();

        for idx in self.scroll.visible_range(self.results.len(), viewport) {
            let row = idx - self.scroll.offset;
            let y = results_start + row as u16;
            if y >= area.bottom() {
                break;
            }

            let result = &self.results[idx];
            let is_selected = idx == self.scroll.selected;

            let line_bg = if is_selected { sel_bg } else { bg };

            // File path (+ line number for content matches)
            let path_str = if result.line_number == 0 {
                format!(" {}", result.path.display())
            } else {
                format!(" {}:{}", result.path.display(), result.line_number)
            };
            let path_style = Style::default().fg(theme::FOCUS_BORDER).bg(line_bg);
            let text_style = Style::default().fg(fg).bg(line_bg);

            // Clear row
            for rx in area.x..area.right() {
                buf[(rx, y)]
                    .set_char(' ')
                    .set_style(Style::default().bg(line_bg));
            }

            // Render path
            let mut rx = area.x;
            for ch in path_str.chars() {
                if rx < area.right() {
                    buf[(rx, y)].set_char(ch).set_style(path_style);
                    rx += 1;
                }
            }

            // Separator
            if rx + 2 < area.right() {
                buf[(rx, y)].set_char(' ').set_style(text_style);
                rx += 1;
            }

            // Render line text (truncated)
            for ch in result.line_text.chars() {
                if rx >= area.right() {
                    break;
                }
                buf[(rx, y)].set_char(ch).set_style(text_style);
                rx += 1;
            }
        }

        // Scrollbar
        if viewport > 0 {
            let scrollbar_area = Rect {
                x: area.x,
                y: results_start,
                width: area.width,
                height: viewport as u16,
            };
            crate::widgets::scrollbar::render_scrollbar(
                scrollbar_area,
                buf,
                self.results.len(),
                viewport,
                self.scroll.offset,
            );
        }
    }
}

/// '/'-joined relative path. Native separators (Windows `\`) never match the
/// '/'-literal exclude grammar or a '/'-containing file-name query.
fn rel_path_string(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Rank a lowercase '/'-joined path against a lowercase file-name query;
/// `None` means no match. A query containing `/` matches against the whole
/// path, otherwise only the file name is considered. Lower rank sorts first.
fn file_name_rank(rel_lower: &str, query_lower: &str) -> Option<u8> {
    if query_lower.is_empty() {
        return None;
    }
    if query_lower.contains('/') {
        return rel_lower.contains(query_lower).then_some(3);
    }
    let name = rel_lower.rsplit('/').next().unwrap_or(rel_lower);
    if name == query_lower {
        Some(0)
    } else if name.starts_with(query_lower) {
        Some(1)
    } else if name.contains(query_lower) {
        Some(2)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_rank_orders_exact_prefix_substring() {
        assert_eq!(file_name_rank("src/main.rs", "main.rs"), Some(0));
        assert_eq!(file_name_rank("src/main.rs", "mai"), Some(1));
        assert_eq!(file_name_rank("src/main.rs", "ain"), Some(2));
        assert_eq!(file_name_rank("src/main.rs", "src"), None);
        assert_eq!(file_name_rank("src/main.rs", "src/ma"), Some(3));
    }

    #[test]
    fn file_name_search_matches_names_not_contents() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/panels")).unwrap();
        std::fs::write(root.join("src/panels/search.rs"), "fn search() {}").unwrap();
        std::fs::write(root.join("src/research_notes.md"), "").unwrap();
        std::fs::write(root.join("src/other.rs"), "search").unwrap();

        let mut panel = SearchPanelState::new();
        panel.mode = SearchMode::FileName;
        panel.search("Search", &[root], &[]);

        let paths: Vec<String> = panel
            .results
            .iter()
            .map(|r| rel_path_string(&r.path))
            .collect();
        assert_eq!(paths, vec!["src/panels/search.rs", "src/research_notes.md"]);
        assert!(panel.results.iter().all(|r| r.line_number == 0));

        panel.toggle_mode();
        panel.search("search", &[root], &[]);
        assert_eq!(panel.results.len(), 2, "content mode hits search.rs and other.rs");
    }

    #[test]
    fn results_carry_the_root_they_were_found_under() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("alpha.rs"), "needle").unwrap();
        std::fs::write(b.path().join("beta.rs"), "needle").unwrap();

        let mut panel = SearchPanelState::new();
        panel.search("needle", &[a.path(), b.path()], &[]);
        let b_label = Path::new(b.path().file_name().unwrap());
        let beta = panel
            .results
            .iter()
            .find(|r| r.abs_path == b.path().join("beta.rs"))
            .expect("second-root match");
        assert_eq!(beta.path, b_label.join("beta.rs"), "prefixed with root folder name");

        panel.mode = SearchMode::FileName;
        panel.search("beta", &[a.path(), b.path()], &[]);
        assert_eq!(panel.results[0].abs_path, b.path().join("beta.rs"));

        // A '/' query can name the root folder.
        let query = format!("{}/beta", b_label.to_str().unwrap());
        panel.search(&query, &[a.path(), b.path()], &[]);
        assert_eq!(panel.results.len(), 1);
    }

    #[test]
    fn single_root_results_have_no_folder_prefix() {
        let a = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("alpha.rs"), "needle").unwrap();

        let mut panel = SearchPanelState::new();
        panel.search("needle", &[a.path()], &[]);
        assert_eq!(panel.results[0].path, Path::new("alpha.rs"));
    }

    #[test]
    fn gitignored_folders_are_not_searched() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git2::Repository::init(root).unwrap();
        std::fs::write(root.join(".gitignore"), "tmp/\n").unwrap();
        std::fs::create_dir_all(root.join("tmp/deep")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("tmp/deep/needle.rs"), "needle").unwrap();
        std::fs::write(root.join("src/needle.rs"), "needle").unwrap();

        let mut panel = SearchPanelState::new();
        panel.mode = SearchMode::FileName;
        panel.search("needle", &[root], &[]);
        let paths: Vec<String> = panel
            .results
            .iter()
            .map(|r| rel_path_string(&r.path))
            .collect();
        assert_eq!(paths, vec!["src/needle.rs"]);

        panel.toggle_mode();
        panel.search("needle", &[root], &[]);
        assert_eq!(panel.results.len(), 1, "content search skips tmp/ too");
    }

    #[test]
    fn file_index_is_reused_until_invalidated() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("first.rs"), "").unwrap();

        let mut panel = SearchPanelState::new();
        panel.mode = SearchMode::FileName;
        panel.search(".rs", &[root], &[]);
        assert_eq!(panel.results.len(), 1);

        std::fs::write(root.join("second.rs"), "").unwrap();
        panel.search(".rs", &[root], &[]);
        assert_eq!(panel.results.len(), 1, "cached index; no re-walk per keystroke");

        panel.invalidate_file_index();
        panel.search(".rs", &[root], &[]);
        assert_eq!(panel.results.len(), 2);
    }
}
