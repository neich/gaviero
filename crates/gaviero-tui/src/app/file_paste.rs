//! Paste files copied in the OS file manager (Explorer, Finder, Files, …)
//! into the folder selected in the EXPLORER.
//!
//! Three ways a paste can reach the explorer, and what each carries:
//! - **`Action::Paste`** — Ctrl+V / Alt+V reached gaviero as a key: the
//!   clipboard's file list (`arboard` `Get::file_list` — CF_HDROP on Windows,
//!   `text/uri-list` on X11/Wayland, file URLs on macOS), else its text.
//! - **`Event::Paste(text)`** — the terminal pasted for us (bracketed paste,
//!   also what most terminals send for a file dropped on the window): the
//!   text as paths, else the clipboard's file list (an empty payload is what
//!   Windows Terminal sends for a clipboard with no text).
//! - **Windows Ctrl+V with no event at all** — the Tick fallback
//!   (`App::maybe_windows_image_paste_fallback`): the file list only.
//!
//! Every paste asks first (`BulkOpState::ConfirmCopy`). Copies never
//! overwrite: a taken name becomes `name (copy).ext`, `name (copy 2).ext`, ….

use super::*;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Whether the explorer tree (not FIND / CHANGES / a review) has focus.
pub(super) fn explorer_focused(app: &App) -> bool {
    app.focus == Focus::FileTree && app.left_panel == LeftPanelMode::FileTree
}

/// A move or another bulk operation is in progress: a paste must not
/// replace it (a terminal paste is not gated by those modes).
fn explorer_busy(app: &App) -> bool {
    app.bulk_op_state.is_some() || app.move_state.is_some()
}

/// Open the copy question for whatever files the paste carries, or say why
/// nothing happened. `text` is the terminal's paste payload, `None` for a
/// keymap paste. Returns whether the question opened.
pub(super) fn start_paste(app: &mut App, text: Option<&str>) -> bool {
    if explorer_busy(app) {
        return false;
    }
    let Some(dest_dir) = app.selected_dir() else {
        set_status(app, "Select a folder in the explorer to paste into".to_string());
        return false;
    };
    let paths = match text {
        Some(text) => paths_from_text(text).or_else(|| clipboard_files(app)),
        None => clipboard_files(app).or_else(|| {
            let text = app.get_clipboard();
            paths_from_text(&text)
        }),
    };
    let Some(paths) = paths else {
        set_status(app, "Nothing to paste: the clipboard holds no files".to_string());
        return false;
    };
    app.bulk_op_state = Some(BulkOpState::ConfirmCopy { paths, dest_dir });
    true
}

/// The Windows Ctrl+V fallback: the clipboard's file list only, silent when
/// there is none (a text paste is the terminal's job).
pub(super) fn start_paste_clipboard_files(app: &mut App) -> bool {
    if explorer_busy(app) {
        return false;
    }
    let Some(dest_dir) = app.selected_dir() else {
        return false;
    };
    let Some(paths) = clipboard_files(app) else {
        return false;
    };
    app.bulk_op_state = Some(BulkOpState::ConfirmCopy { paths, dest_dir });
    true
}

/// Run a confirmed paste: copy, record the writes for a running turn's
/// review, refresh the tree and select the first copy.
pub(super) fn execute_copy(app: &mut App, paths: &[PathBuf], dest_dir: &Path) {
    app.bulk_op_state = None;
    let report = copy_into(paths, dest_dir);
    for file in &report.files {
        super::turn_review::note_host_write(app, file);
    }
    for failure in &report.failures {
        tracing::error!("paste into {}: {failure}", dest_dir.display());
    }
    app.refresh_file_tree();
    if let Some(first) = report.top_level.first() {
        app.select_path_in_tree(first);
    }
    let dest_name = dest_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dest_dir.display().to_string());
    let copied = report.top_level.len();
    let message = match report.failures.first() {
        None => format!("Copied {copied} item(s) → {dest_name}"),
        Some(first) => format!(
            "Copied {copied} item(s) → {dest_name}; {} problem(s), first: {first}",
            report.failures.len()
        ),
    };
    set_status(app, message);
}

fn set_status(app: &mut App, message: String) {
    app.status_message = Some((message, std::time::Instant::now()));
}

/// The clipboard's file list, keeping only paths that still exist.
fn clipboard_files(app: &mut App) -> Option<Vec<PathBuf>> {
    let files = app.clipboard.as_mut()?.get().file_list().ok()?;
    let files: Vec<PathBuf> = files.into_iter().filter(|p| p.exists()).collect();
    (!files.is_empty()).then_some(files)
}

/// Pasted text read as absolute paths of existing files or folders.
fn paths_from_text(text: &str) -> Option<Vec<PathBuf>> {
    // `\` separates Windows paths; elsewhere terminals use it to escape spaces.
    resolve_pasted_paths(text, !cfg!(windows), |p| p.is_absolute() && p.exists())
}

// ── Pure helpers ────────────────────────────────────────────────────────

/// Read pasted text as a list of paths, or `None` when any non-empty line
/// is not one (ordinary text is never taken for a file list).
///
/// Each line is first split shell-style — `'…'` / `"…"` quoting, `\`
/// escapes when `backslash_escapes` — which covers several files dropped at
/// once (`'/a b' '/c'`, `"C:\a b" C:\c`, `/a\ b`); if any token is
/// rejected, the whole line (minus one pair of surrounding quotes) is tried
/// as a single path. `file://` URIs are percent-decoded. `accept` decides
/// what counts as a path (production: absolute and existing).
pub(super) fn resolve_pasted_paths(
    text: &str,
    backslash_escapes: bool,
    accept: impl Fn(&Path) -> bool,
) -> Option<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let from_tokens = tokenize(line, backslash_escapes).and_then(|tokens| {
            let paths: Vec<PathBuf> = tokens.iter().map(|t| token_path(t)).collect();
            (!paths.is_empty() && paths.iter().all(|p| accept(p))).then_some(paths)
        });
        let paths = match from_tokens {
            Some(paths) => paths,
            None => {
                let whole = token_path(strip_matching_quotes(line));
                if !accept(&whole) {
                    return None;
                }
                vec![whole]
            }
        };
        for path in paths {
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Shell-style split; `None` on an unclosed quote or a trailing `\`.
fn tokenize(line: &str, backslash_escapes: bool) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) if c == q => quote = None,
            Some('"') if c == '\\' && backslash_escapes => current.push(chars.next()?),
            Some(_) => current.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_token = true;
            }
            None if c == '\\' && backslash_escapes => {
                current.push(chars.next()?);
                in_token = true;
            }
            None if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            None => {
                current.push(c);
                in_token = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if in_token {
        tokens.push(current);
    }
    Some(tokens)
}

fn strip_matching_quotes(s: &str) -> &str {
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

fn token_path(token: &str) -> PathBuf {
    decode_file_uri(token)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(token))
}

/// `file:///a%20b` → `/a b`; `file://localhost/…` too. Other hosts are not
/// local files. On Windows `file:///C:/x` → `C:/x`.
fn decode_file_uri(token: &str) -> Option<String> {
    let rest = token.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let decoded = String::from_utf8(percent_decode(rest)?).ok()?;
    if cfg!(windows) {
        let b = decoded.as_bytes();
        if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' {
            return Some(decoded[1..].to_string());
        }
    }
    Some(decoded)
}

fn percent_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

/// A free name for `name` in `dir`: `name` itself, else `stem (copy).ext`,
/// `stem (copy 2).ext`, …. A folder keeps its whole name as the stem.
pub(super) fn unique_dest(dir: &Path, name: &OsStr, is_dir: bool) -> PathBuf {
    let first = dir.join(name);
    if !occupied(&first) {
        return first;
    }
    let as_path = Path::new(name);
    let (stem, ext) = if is_dir {
        (name.to_string_lossy().into_owned(), None)
    } else {
        (
            as_path
                .file_stem()
                .unwrap_or(name)
                .to_string_lossy()
                .into_owned(),
            as_path.extension().map(|e| e.to_string_lossy().into_owned()),
        )
    };
    (1u32..)
        .map(|n| {
            let suffix = if n == 1 {
                " (copy)".to_string()
            } else {
                format!(" (copy {n})")
            };
            match &ext {
                Some(ext) => dir.join(format!("{stem}{suffix}.{ext}")),
                None => dir.join(format!("{stem}{suffix}")),
            }
        })
        .find(|candidate| !occupied(candidate))
        .expect("an unused copy name exists")
}

/// Taken by anything, a dangling symlink included.
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// What a paste did.
#[derive(Debug, Default)]
pub(super) struct CopyReport {
    /// The new top-level entries in the destination, in paste order.
    pub top_level: Vec<PathBuf>,
    /// Every file written (inside copied folders too).
    pub files: Vec<PathBuf>,
    /// One line per thing that could not be copied.
    pub failures: Vec<String>,
}

/// Copy each of `srcs` into `dest_dir` under a free name. Folders are copied
/// recursively; a link at the top is followed (the pasted item is what it
/// points at), links inside a folder to folders are skipped (no cycles).
/// Pasting a folder into itself or a folder inside it is refused.
pub(super) fn copy_into(srcs: &[PathBuf], dest_dir: &Path) -> CopyReport {
    let mut report = CopyReport::default();
    let dest_canon = std::fs::canonicalize(dest_dir).ok();
    for src in srcs {
        let Some(name) = src.file_name() else {
            report
                .failures
                .push(format!("{}: not a file or folder", src.display()));
            continue;
        };
        let meta = match std::fs::metadata(src) {
            Ok(meta) => meta,
            Err(e) => {
                report.failures.push(format!("{}: {e}", src.display()));
                continue;
            }
        };
        if meta.is_dir() {
            if let (Some(dest), Ok(src_canon)) = (&dest_canon, std::fs::canonicalize(src))
                && dest.starts_with(&src_canon)
            {
                report.failures.push(format!(
                    "{}: cannot paste a folder into itself",
                    src.display()
                ));
                continue;
            }
            let target = unique_dest(dest_dir, name, true);
            match copy_dir(src, &target, &mut report) {
                Ok(()) => report.top_level.push(target),
                Err(e) => report.failures.push(format!("{}: {e}", src.display())),
            }
        } else {
            let target = unique_dest(dest_dir, name, false);
            match std::fs::copy(src, &target) {
                Ok(_) => {
                    report.files.push(target.clone());
                    report.top_level.push(target);
                }
                Err(e) => report.failures.push(format!("{}: {e}", src.display())),
            }
        }
    }
    report
}

fn copy_dir(src: &Path, target: &Path, report: &mut CopyReport) -> std::io::Result<()> {
    std::fs::create_dir(target)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = target.join(entry.file_name());
        let file_type = entry.file_type()?;
        let result = if file_type.is_dir() {
            copy_dir(&from, &to, report)
        } else if file_type.is_symlink() && !from.is_file() {
            report.failures.push(format!(
                "{}: link to a folder (or a dangling link) skipped",
                from.display()
            ));
            continue;
        } else {
            std::fs::copy(&from, &to).map(|_| report.files.push(to.clone()))
        };
        if let Err(e) = result {
            report.failures.push(format!("{}: {e}", from.display()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn accept_set(paths: &[&str]) -> impl Fn(&Path) -> bool {
        let set: HashSet<PathBuf> = paths.iter().map(PathBuf::from).collect();
        move |p| set.contains(p)
    }

    fn pb(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn ordinary_text_is_not_a_file_list() {
        let accept = accept_set(&["/a"]);
        assert_eq!(resolve_pasted_paths("hello world", true, &accept), None);
        assert_eq!(resolve_pasted_paths("", true, &accept), None);
        // One stray line spoils the whole paste.
        assert_eq!(resolve_pasted_paths("/a\nnot a path", true, &accept), None);
    }

    #[test]
    fn one_path_per_line_crlf_and_blank_lines() {
        let accept = accept_set(&["/a", "/b/c.txt"]);
        assert_eq!(
            resolve_pasted_paths("/a\r\n\r\n  /b/c.txt  \r\n", true, &accept),
            Some(pb(&["/a", "/b/c.txt"]))
        );
    }

    #[test]
    fn a_path_with_spaces_unquoted_is_taken_whole() {
        let accept = accept_set(&["/home/u/a b.txt"]);
        assert_eq!(
            resolve_pasted_paths("/home/u/a b.txt", true, &accept),
            Some(pb(&["/home/u/a b.txt"]))
        );
    }

    #[test]
    fn several_dropped_files_quoted_on_one_line() {
        let accept = accept_set(&["/x/a b", "/x/c", "/x/it's"]);
        assert_eq!(
            resolve_pasted_paths("'/x/a b' '/x/c' \"/x/it's\"", true, &accept),
            Some(pb(&["/x/a b", "/x/c", "/x/it's"]))
        );
    }

    #[test]
    fn unix_backslash_escaped_spaces() {
        let accept = accept_set(&["/x/a b.txt", "/x/c"]);
        assert_eq!(
            resolve_pasted_paths(r"/x/a\ b.txt /x/c", true, &accept),
            Some(pb(&["/x/a b.txt", "/x/c"]))
        );
    }

    #[test]
    fn windows_backslashes_are_separators_not_escapes() {
        let accept = accept_set(&[r"C:\a b\x.txt", r"C:\y"]);
        assert_eq!(
            resolve_pasted_paths(r#""C:\a b\x.txt" C:\y"#, false, &accept),
            Some(pb(&[r"C:\a b\x.txt", r"C:\y"]))
        );
        // Unquoted with a space: the whole line.
        assert_eq!(
            resolve_pasted_paths(r"C:\a b\x.txt", false, &accept),
            Some(pb(&[r"C:\a b\x.txt"]))
        );
    }

    #[test]
    fn an_apostrophe_in_a_windows_name_falls_back_to_the_line() {
        let accept = accept_set(&[r"C:\O'Brien.txt"]);
        assert_eq!(
            resolve_pasted_paths(r"C:\O'Brien.txt", false, &accept),
            Some(pb(&[r"C:\O'Brien.txt"]))
        );
    }

    #[test]
    fn file_uris_are_decoded() {
        let accept = accept_set(&["/x/a b,c/ñ.pdf", "/y"]);
        assert_eq!(
            resolve_pasted_paths(
                "file:///x/a%20b%2Cc/%C3%B1.pdf\nfile://localhost/y",
                true,
                &accept
            ),
            Some(pb(&["/x/a b,c/ñ.pdf", "/y"]))
        );
        // Another host is not a local file, and broken escapes are not decoded.
        assert_eq!(decode_file_uri("file://server/share/x"), None);
        assert_eq!(decode_file_uri("file:///a%2"), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_file_uri_drops_the_slash_before_the_drive() {
        assert_eq!(decode_file_uri("file:///C:/a%20b").as_deref(), Some("C:/a b"));
    }

    #[test]
    fn duplicates_are_listed_once() {
        let accept = accept_set(&["/a"]);
        assert_eq!(resolve_pasted_paths("/a\n/a", true, &accept), Some(pb(&["/a"])));
    }

    #[test]
    fn unique_dest_never_reuses_a_taken_name() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert_eq!(unique_dest(d, OsStr::new("x.txt"), false), d.join("x.txt"));
        std::fs::write(d.join("x.txt"), "").unwrap();
        assert_eq!(unique_dest(d, OsStr::new("x.txt"), false), d.join("x (copy).txt"));
        std::fs::write(d.join("x (copy).txt"), "").unwrap();
        assert_eq!(unique_dest(d, OsStr::new("x.txt"), false), d.join("x (copy 2).txt"));
        std::fs::create_dir(d.join("lib.d")).unwrap();
        assert_eq!(unique_dest(d, OsStr::new("lib.d"), true), d.join("lib.d (copy)"));
        std::fs::write(d.join(".env"), "").unwrap();
        assert_eq!(unique_dest(d, OsStr::new(".env"), false), d.join(".env (copy)"));
    }

    #[test]
    fn copy_into_copies_files_and_folders_without_overwriting() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("a.txt"), "new").unwrap();
        std::fs::create_dir_all(src.path().join("pkg/sub")).unwrap();
        std::fs::write(src.path().join("pkg/sub/m.rs"), "fn m() {}").unwrap();
        std::fs::write(dest.path().join("a.txt"), "old").unwrap();

        let report = copy_into(
            &[src.path().join("a.txt"), src.path().join("pkg")],
            dest.path(),
        );
        assert!(report.failures.is_empty(), "{:?}", report.failures);
        assert_eq!(
            report.top_level,
            vec![dest.path().join("a (copy).txt"), dest.path().join("pkg")]
        );
        assert_eq!(std::fs::read_to_string(dest.path().join("a.txt")).unwrap(), "old");
        assert_eq!(
            std::fs::read_to_string(dest.path().join("a (copy).txt")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(dest.path().join("pkg/sub/m.rs")).unwrap(),
            "fn m() {}"
        );
        assert_eq!(report.files.len(), 2);
        // The sources are untouched.
        assert!(src.path().join("pkg/sub/m.rs").exists());
    }

    #[test]
    fn pasting_into_the_same_folder_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("n.md"), "x").unwrap();
        let report = copy_into(&[dir.path().join("n.md")], dir.path());
        assert_eq!(report.top_level, vec![dir.path().join("n (copy).md")]);
    }

    #[test]
    fn a_folder_cannot_be_pasted_into_itself() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("f");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        for dest in [folder.clone(), folder.join("inner")] {
            let report = copy_into(std::slice::from_ref(&folder), &dest);
            assert!(report.top_level.is_empty());
            assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
            assert!(report.failures[0].contains("into itself"));
        }
        assert!(!folder.join("f").exists());
        assert!(!folder.join("inner/f").exists());
    }

    #[test]
    fn a_missing_source_is_reported_not_fatal() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("ok.txt"), "").unwrap();
        let report = copy_into(
            &[src.path().join("gone.txt"), src.path().join("ok.txt")],
            dest.path(),
        );
        assert_eq!(report.top_level, vec![dest.path().join("ok.txt")]);
        assert_eq!(report.failures.len(), 1);
    }

    // ── App wiring ──────────────────────────────────────────────────────

    fn test_app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(dir.path().join(".gaviero/settings.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join("dest")).unwrap();
        let ws = gaviero_core::workspace::Workspace::single_folder(dir.path().to_path_buf());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(ws, tx);
        // Never consult the developer's real clipboard.
        app.clipboard = None;
        app.refresh_file_tree();
        app.select_path_in_tree(&dir.path().join("dest"));
        app.focus = Focus::FileTree;
        app.left_panel = LeftPanelMode::FileTree;
        (dir, app)
    }

    fn quoted(path: &Path) -> String {
        format!("\"{}\"", path.display())
    }

    #[test]
    fn pasted_paths_open_the_copy_question_for_the_selected_folder() {
        let (ws, mut app) = test_app();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("in.txt");
        std::fs::write(&file, "payload").unwrap();

        assert!(explorer_focused(&app));
        assert!(start_paste(&mut app, Some(&quoted(&file))));
        let Some(BulkOpState::ConfirmCopy { paths, dest_dir }) = app.bulk_op_state.clone() else {
            panic!("expected ConfirmCopy, got {:?}", app.bulk_op_state);
        };
        assert_eq!(paths, vec![file.clone()]);
        assert_eq!(
            std::fs::canonicalize(&dest_dir).unwrap(),
            std::fs::canonicalize(ws.path().join("dest")).unwrap()
        );
        assert!(super::super::key_route::KeyContext::of(&app).modal().is_some());

        execute_copy(&mut app, &paths, &dest_dir);
        assert!(app.bulk_op_state.is_none());
        assert_eq!(
            std::fs::read_to_string(ws.path().join("dest/in.txt")).unwrap(),
            "payload"
        );
        let (status, _) = app.status_message.clone().unwrap();
        assert!(status.starts_with("Copied 1 item(s) → dest"), "{status}");
    }

    #[test]
    fn plain_text_pasted_into_the_explorer_copies_nothing() {
        let (_ws, mut app) = test_app();
        assert!(!start_paste(&mut app, Some("just some words")));
        assert!(app.bulk_op_state.is_none());
        let (status, _) = app.status_message.clone().unwrap();
        assert!(status.starts_with("Nothing to paste"), "{status}");
        // The Windows fallback stays silent without a clipboard file list.
        app.status_message = None;
        assert!(!start_paste_clipboard_files(&mut app));
        assert!(app.status_message.is_none());
    }

    #[test]
    fn keys_drive_the_copy_question_and_a_second_ctrl_v_does_not_answer_it() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = |code, mods| crate::event::Event::Key(KeyEvent::new(code, mods));
        let (ws, mut app) = test_app();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("k.txt");
        std::fs::write(&file, "k").unwrap();
        app.internal_clipboard = quoted(&file);

        // Ctrl+V in the explorer: the keymap paste reads the clipboard text.
        let ctrl_v = || key(KeyCode::Char('v'), KeyModifiers::CONTROL);
        super::super::controller::handle_event(&mut app, ctrl_v());
        assert!(matches!(app.bulk_op_state, Some(BulkOpState::ConfirmCopy { .. })));
        // The same gesture again leaves the question open.
        super::super::controller::handle_event(&mut app, ctrl_v());
        assert!(matches!(app.bulk_op_state, Some(BulkOpState::ConfirmCopy { .. })));
        // A terminal paste while it is open is dropped by the modal gate.
        super::super::controller::handle_event(
            &mut app,
            crate::event::Event::Paste(quoted(&file)),
        );
        super::super::controller::handle_event(&mut app, key(KeyCode::Char('y'), KeyModifiers::NONE));
        assert!(app.bulk_op_state.is_none());
        assert_eq!(std::fs::read_to_string(ws.path().join("dest/k.txt")).unwrap(), "k");
        assert!(!ws.path().join("dest/k (copy).txt").exists());

        // Esc cancels without copying.
        super::super::controller::handle_event(&mut app, ctrl_v());
        assert!(app.bulk_op_state.is_some());
        super::super::controller::handle_event(&mut app, key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.bulk_op_state.is_none());
        assert!(!ws.path().join("dest/k (copy).txt").exists());
    }

    #[test]
    fn a_paste_never_replaces_a_move_in_progress() {
        let (ws, mut app) = test_app();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("in.txt");
        std::fs::write(&file, "").unwrap();
        let picking = BulkOpState::SelectingDest {
            paths: vec![ws.path().join("dest")],
        };
        app.bulk_op_state = Some(picking);
        assert!(!start_paste(&mut app, Some(&quoted(&file))));
        assert!(matches!(
            app.bulk_op_state,
            Some(BulkOpState::SelectingDest { .. })
        ));
    }

    #[test]
    fn a_terminal_paste_in_the_explorer_reaches_the_copy_question() {
        let (_ws, mut app) = test_app();
        let outside = tempfile::tempdir().unwrap();
        let file = outside.path().join("dropped.bin");
        std::fs::write(&file, [0u8, 1, 2]).unwrap();
        app.handle_paste(&quoted(&file));
        assert!(
            matches!(app.bulk_op_state, Some(BulkOpState::ConfirmCopy { .. })),
            "{:?}",
            app.bulk_op_state
        );
    }
}
