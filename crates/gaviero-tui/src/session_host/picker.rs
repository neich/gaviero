//! New-session picker (Alt+N): a folder browser over the whole disk.
//!
//! An empty path box shows the *places* — recently opened paths, the folder
//! the host was started in, the home folder and every filesystem root (each
//! drive on Windows). Typing completes folders and `.gaviero-workspace` files
//! (absolute and `~` paths as typed, anything else relative to the start
//! folder). Right enters the selected folder, Left goes to the parent of the
//! listed one (from a root: back to the places), and a listed folder always
//! shows itself first so Enter can open it.

use super::state;
use crate::widgets::path_listing::{
    display_dir_prefix, ends_with_separator, expand_tilde, is_filesystem_query,
    list_filesystem_entries,
};
use crate::widgets::prompt_input::{
    AutocompleteMode, FileAutocomplete, POPUP_ROWS, render_autocomplete,
};
use crate::widgets::text_input::TextInput;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use std::path::{Path, PathBuf};

const WORKSPACE_SUFFIX: &str = ".gaviero-workspace";
const NOT_OPENABLE: &str = "Not a folder or .gaviero-workspace file";

/// A key the picker understands, from either input path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickerKey {
    Char(char),
    Backspace,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Tab,
    Enter,
    Esc,
}

/// What the host does after a picker key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PickerOutcome {
    Stay,
    Close,
    Open(PathBuf),
}

/// The picker key a Windows key-down record is, if any.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn key_from_record(r: &super::encode::KeyRecord) -> Option<PickerKey> {
    if !r.key_down {
        return None;
    }
    Some(match r.vk {
        0x0D => PickerKey::Enter,
        0x1B => PickerKey::Esc,
        0x08 => PickerKey::Backspace,
        0x09 => PickerKey::Tab,
        0x25 => PickerKey::Left,
        0x26 => PickerKey::Up,
        0x27 => PickerKey::Right,
        0x28 => PickerKey::Down,
        0x21 => PickerKey::PageUp,
        0x22 => PickerKey::PageDown,
        _ => {
            let c = char::from_u32(u32::from(r.uchar))?;
            if c.is_control() {
                return None;
            }
            PickerKey::Char(c)
        }
    })
}

/// The picker key a crossterm key event is, if any (Unix input path).
pub(crate) fn key_from_event(k: &KeyEvent) -> Option<PickerKey> {
    Some(match k.code {
        KeyCode::Enter => PickerKey::Enter,
        KeyCode::Esc => PickerKey::Esc,
        KeyCode::Backspace => PickerKey::Backspace,
        KeyCode::Tab => PickerKey::Tab,
        KeyCode::Up => PickerKey::Up,
        KeyCode::Down => PickerKey::Down,
        KeyCode::Left => PickerKey::Left,
        KeyCode::Right => PickerKey::Right,
        KeyCode::PageUp => PickerKey::PageUp,
        KeyCode::PageDown => PickerKey::PageDown,
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => PickerKey::Char(c),
        _ => return None,
    })
}

pub(crate) struct Picker {
    pub input: TextInput,
    pub list: FileAutocomplete,
    recents: Vec<PathBuf>,
    /// Start folder, home folder and filesystem roots, shown after the
    /// recents while the path box is empty.
    places: Vec<PathBuf>,
    pub error: Option<String>,
}

impl Picker {
    pub fn new(recents: Vec<PathBuf>, cwd: &Path) -> Self {
        let mut places = Vec::new();
        places.push(gaviero_core::util::fs::canonicalize_simplified(cwd).unwrap_or_else(|_| cwd.to_path_buf()));
        places.extend(dirs::home_dir());
        places.extend(crate::platform::filesystem_roots());
        let mut picker = Picker {
            input: TextInput::new(),
            list: FileAutocomplete::new(),
            recents,
            places,
            error: None,
        };
        picker.refresh(cwd);
        picker
    }

    /// Recompute the match list for the typed text.
    ///
    /// Empty: the places (recents, start folder, home, roots). A bare name
    /// (no separator): matching recents, then folders and workspace files in
    /// `cwd`. A path: the listed folder itself when the text ends with a
    /// separator, then its folders and workspace files. A match that is
    /// exactly the typed path goes to the very top.
    pub fn refresh(&mut self, cwd: &Path) {
        let query = self.input.text.clone();
        let mut matches: Vec<String> = Vec::new();
        let push = |m: String, matches: &mut Vec<String>| {
            if !matches.contains(&m) {
                matches.push(m);
            }
        };
        if query.is_empty() {
            for p in self.recents.iter().chain(&self.places) {
                push(dir_label(p), &mut matches);
            }
        } else {
            if !query.contains(['/', '\\']) {
                let needle = query.to_lowercase();
                for p in &self.recents {
                    let label = dir_label(p);
                    if label.to_lowercase().contains(&needle) {
                        push(label, &mut matches);
                    }
                }
            }
            if ends_with_separator(&query) && resolve(&query, cwd).is_some_and(|p| p.is_dir()) {
                push(query.clone(), &mut matches);
            }
            let base = (!is_filesystem_query(&query)).then_some(cwd);
            let listed = list_filesystem_entries(&query, base, usize::MAX)
                .into_iter()
                .filter(|m| ends_with_separator(m) || m.ends_with(WORKSPACE_SUFFIX));
            for m in listed {
                push(m, &mut matches);
            }
        }
        let exact = |m: &String| {
            !query.is_empty()
                && (*m == query
                    || m.strip_suffix(['/', '\\']).is_some_and(|s| s == query))
        };
        if let Some(pos) = matches.iter().position(exact) {
            let m = matches.remove(pos);
            matches.insert(0, m);
        }
        self.list.open(AutocompleteMode::DirPath, 0, &query);
        self.list.matches = matches;
        self.list.clamp_selection();
    }

    /// Apply one key. `Open` carries a resolved folder or workspace file.
    pub fn on_key(&mut self, key: PickerKey, cwd: &Path) -> PickerOutcome {
        match key {
            PickerKey::Esc => return PickerOutcome::Close,
            PickerKey::Char(c) => {
                self.input.insert_char(c);
                self.error = None;
                self.refresh(cwd);
            }
            PickerKey::Backspace => {
                self.input.backspace();
                self.error = None;
                self.refresh(cwd);
            }
            PickerKey::Up => self.list.select_previous(),
            PickerKey::Down => self.list.select_next(),
            PickerKey::PageUp => self.list.move_selection(-(POPUP_ROWS as isize)),
            PickerKey::PageDown => self.list.move_selection(POPUP_ROWS as isize),
            PickerKey::Left => self.go_to_parent(cwd),
            PickerKey::Right => {
                if let Some(m) = self.selected().map(str::to_string)
                    && ends_with_separator(&m)
                    && m != self.input.text
                {
                    self.set_input(&m, cwd);
                }
            }
            PickerKey::Tab => {
                if let Some(m) = self.selected().map(str::to_string) {
                    self.set_input(&m, cwd);
                }
            }
            PickerKey::Enter => {
                let typed = self.input.text.clone();
                let target = self.selected().map(str::to_string).unwrap_or(typed);
                match resolve(&target, cwd) {
                    Some(path) => return PickerOutcome::Open(path),
                    None => self.error = Some(NOT_OPENABLE.to_string()),
                }
            }
        }
        PickerOutcome::Stay
    }

    fn set_input(&mut self, text: &str, cwd: &Path) {
        self.input = TextInput::new();
        self.input.insert_str(text);
        self.error = None;
        self.refresh(cwd);
    }

    /// Show the parent of the listed folder (the start folder for a bare
    /// name); from a filesystem root, or with an empty box, the places.
    fn go_to_parent(&mut self, cwd: &Path) {
        let query = self.input.text.clone();
        if query.is_empty() {
            return;
        }
        let dir_text = display_dir_prefix(&query);
        let listed = if dir_text.is_empty() {
            cwd.to_path_buf()
        } else {
            absolute(&dir_text, cwd)
        };
        let listed = gaviero_core::util::fs::canonicalize_simplified(&listed).unwrap_or(listed);
        let parent = listed.parent().map(dir_label).unwrap_or_default();
        self.set_input(&parent, cwd);
    }

    fn selected(&self) -> Option<&str> {
        if self.list.is_open() {
            self.list.matches.get(self.list.selected).map(String::as_str)
        } else {
            None
        }
    }

    /// A centered box over `area`: title, path box, error line, match list.
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let width = area.width.saturating_sub(4).min(100);
        let height = area
            .height
            .saturating_sub(2)
            .min(POPUP_ROWS as u16 + 4);
        if width < 10 || height < 4 {
            return;
        }
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        Clear.render(rect, buf);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" New session ")
            .style(Style::default().fg(Color::White).bg(Color::Rgb(35, 39, 46)));
        let inner = block.inner(rect);
        block.render(rect, buf);
        if inner.height < 2 {
            return;
        }

        // Path box: the typed text and a block caret.
        let prompt = format!("> {}", self.input.text);
        buf.set_stringn(inner.x, inner.y, &prompt, inner.width as usize, Style::default());
        let caret_x = inner.x + (2 + self.input.text.chars().count() as u16).min(inner.width - 1);
        buf[(caret_x, inner.y)].set_style(Style::default().add_modifier(Modifier::REVERSED));

        let (note, style) = match &self.error {
            Some(err) => (err.as_str(), Style::default().fg(Color::Rgb(224, 108, 117))),
            None => (
                "Enter open · → into folder · ← parent · Tab complete · Esc cancel",
                Style::default().fg(Color::Rgb(99, 109, 131)),
            ),
        };
        buf.set_stringn(inner.x, inner.y + 1, note, inner.width as usize, style);

        let list_area = Rect {
            y: inner.y + 2,
            height: inner.height - 2,
            ..inner
        };
        if list_area.height > 0 && self.list.is_open() {
            render_autocomplete(&self.list, list_area, buf);
        }
    }
}

/// A typed or listed path, resolved against `cwd`, if it is a folder or a
/// `.gaviero-workspace` file.
fn resolve(text: &str, cwd: &Path) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let path = gaviero_core::util::fs::canonicalize_simplified(&absolute(text, cwd)).ok()?;
    state::still_openable(&path).then_some(path)
}

/// `text` with `~` expanded, joined onto `cwd` unless already absolute.
fn absolute(text: &str, cwd: &Path) -> PathBuf {
    let path = PathBuf::from(expand_tilde(text));
    if path.is_absolute() { path } else { cwd.join(path) }
}

/// How the list shows a path: as displayed, with a trailing separator when
/// it is a folder (so Right / Tab browse into it).
fn dir_label(path: &Path) -> String {
    let label = path.display().to_string();
    if !ends_with_separator(&label) && path.is_dir() {
        format!("{label}{}", std::path::MAIN_SEPARATOR)
    } else {
        label
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("alpha")).unwrap();
        std::fs::create_dir(dir.path().join("beta")).unwrap();
        std::fs::write(dir.path().join("w.gaviero-workspace"), "{}").unwrap();
        std::fs::write(dir.path().join("x.txt"), "").unwrap();
        dir
    }

    fn typed(picker: &mut Picker, text: &str, cwd: &Path) {
        for c in text.chars() {
            picker.on_key(PickerKey::Char(c), cwd);
        }
    }

    fn canon(p: &Path) -> PathBuf {
        gaviero_core::util::fs::canonicalize_simplified(p).unwrap()
    }

    #[test]
    fn typing_completes_folders_and_workspace_files_only() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "a", dir.path());
        assert_eq!(picker.list.matches, vec!["alpha/".to_string()]);
        picker.on_key(PickerKey::Backspace, dir.path());
        typed(&mut picker, "w", dir.path());
        assert_eq!(picker.list.matches, vec!["w.gaviero-workspace".to_string()]);
        picker.on_key(PickerKey::Backspace, dir.path());
        typed(&mut picker, "x", dir.path());
        assert!(picker.list.matches.is_empty(), "plain files are not offered");
    }

    #[test]
    fn the_empty_box_lists_the_start_folder_home_and_roots() {
        let dir = fixture();
        let picker = Picker::new(Vec::new(), dir.path());
        let m = &picker.list.matches;
        assert_eq!(m[0], dir_label(&canon(dir.path())));
        if let Some(home) = dirs::home_dir() {
            assert!(m.contains(&dir_label(&home)));
        }
        for root in crate::platform::filesystem_roots() {
            assert!(m.contains(&dir_label(&root)), "{root:?} missing from {m:?}");
        }
    }

    #[test]
    fn right_enters_a_folder_and_lists_it_first() {
        let dir = fixture();
        let start = dir_label(&canon(dir.path()));
        let mut picker = Picker::new(Vec::new(), dir.path());
        picker.on_key(PickerKey::Right, dir.path());
        assert_eq!(picker.input.text, start);
        assert_eq!(picker.list.matches[0], start, "the folder itself comes first");
        let alpha = format!("{start}alpha{}", std::path::MAIN_SEPARATOR);
        assert!(picker.list.matches.contains(&alpha));
        assert!(!picker.list.matches.iter().any(|m| m.contains("x.txt")));

        // Right on the folder itself stays put; on a child it descends.
        picker.on_key(PickerKey::Right, dir.path());
        assert_eq!(picker.input.text, start);
        picker.list.selected = picker.list.matches.iter().position(|m| *m == alpha).unwrap();
        picker.on_key(PickerKey::Right, dir.path());
        assert_eq!(picker.input.text, alpha);
    }

    #[test]
    fn enter_on_a_browsed_folder_opens_it() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        picker.on_key(PickerKey::Right, dir.path());
        assert_eq!(
            picker.on_key(PickerKey::Enter, dir.path()),
            PickerOutcome::Open(canon(dir.path()))
        );
    }

    #[test]
    fn left_walks_up_to_the_root_then_the_places() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "alpha/", dir.path());
        picker.on_key(PickerKey::Left, dir.path());
        assert_eq!(picker.input.text, dir_label(&canon(dir.path())));
        picker.on_key(PickerKey::Left, dir.path());
        assert_eq!(picker.input.text, dir_label(canon(dir.path()).parent().unwrap()));
        // Leaving the folder the host started in is the point.
        assert!(picker.list.matches.iter().any(|m| m.ends_with(&format!(
            "{}{}",
            dir.path().file_name().unwrap().to_string_lossy(),
            std::path::MAIN_SEPARATOR
        ))));

        for _ in 0..64 {
            if picker.input.text.is_empty() {
                break;
            }
            picker.on_key(PickerKey::Left, dir.path());
        }
        assert!(picker.input.text.is_empty(), "past the root come the places");
        assert_eq!(picker.list.matches[0], dir_label(&canon(dir.path())));
    }

    #[test]
    fn a_bare_name_left_goes_to_the_start_folders_parent() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "al", dir.path());
        picker.on_key(PickerKey::Left, dir.path());
        assert_eq!(picker.input.text, dir_label(canon(dir.path()).parent().unwrap()));
    }

    #[test]
    fn enter_opens_the_selected_match() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "al", dir.path());
        let want = gaviero_core::util::fs::canonicalize_simplified(&dir.path().join("alpha")).unwrap();
        assert_eq!(
            picker.on_key(PickerKey::Enter, dir.path()),
            PickerOutcome::Open(want)
        );
    }

    #[test]
    fn enter_on_a_missing_path_sets_an_error() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "nope", dir.path());
        assert!(!picker.list.is_open());
        assert_eq!(picker.on_key(PickerKey::Enter, dir.path()), PickerOutcome::Stay);
        assert_eq!(picker.error.as_deref(), Some(NOT_OPENABLE));
        // Typing again clears it.
        picker.on_key(PickerKey::Backspace, dir.path());
        assert!(picker.error.is_none());
    }

    #[test]
    fn recents_rank_first() {
        let dir = fixture();
        let recent = dir.path().join("beta");
        let mut picker = Picker::new(vec![recent.clone()], dir.path());
        assert_eq!(picker.list.matches[0], dir_label(&recent));
        typed(&mut picker, "be", dir.path());
        assert_eq!(picker.list.matches[0], dir_label(&recent));
        picker.on_key(PickerKey::Backspace, dir.path());
        picker.on_key(PickerKey::Backspace, dir.path());
        typed(&mut picker, "zzz", dir.path());
        assert!(picker.list.matches.is_empty(), "a recent must contain the text");
    }

    #[test]
    fn an_exactly_typed_folder_beats_longer_names() {
        let dir = fixture();
        std::fs::create_dir(dir.path().join("alpha2")).unwrap();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "alpha", dir.path());
        assert_eq!(picker.list.matches[0], "alpha/");
    }

    #[test]
    fn tab_accepts_the_selection_and_esc_closes() {
        let dir = fixture();
        let mut picker = Picker::new(Vec::new(), dir.path());
        typed(&mut picker, "be", dir.path());
        picker.on_key(PickerKey::Tab, dir.path());
        assert_eq!(picker.input.text, "beta/");
        assert_eq!(picker.on_key(PickerKey::Esc, dir.path()), PickerOutcome::Close);
    }

    #[test]
    fn windows_records_map_to_picker_keys() {
        let rec = |vk, uchar, down| super::super::encode::KeyRecord {
            vk,
            scan: 0,
            uchar,
            key_down: down,
            ctrl_state: 0,
            repeat: 1,
        };
        assert_eq!(key_from_record(&rec(0x0D, 13, true)), Some(PickerKey::Enter));
        assert_eq!(key_from_record(&rec(0x25, 0, true)), Some(PickerKey::Left));
        assert_eq!(key_from_record(&rec(0x27, 0, true)), Some(PickerKey::Right));
        assert_eq!(key_from_record(&rec(0x41, 'a' as u16, true)), Some(PickerKey::Char('a')));
        assert_eq!(key_from_record(&rec(0x41, 'a' as u16, false)), None);
        assert_eq!(key_from_record(&rec(0x51, 17, true)), None, "Ctrl+Q is not text");
    }
}
