//! Read-only workspace browsing for the remote `file_browser` capability
//! (protocol 1.3). Paths are explorer display paths: `<root-label>/<rel>`,
//! using the same roots and labels the desktop explorer and `@` completion
//! use. Nothing here writes: there is no save, rename, create, or delete.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use gaviero_remote::dto::ErrorCode;
use serde_json::json;

use super::remote::CommandFailure;

/// Content cap for `read_file`; keeps a worst-case `command_result` under
/// the 256 KiB frame cap.
pub(crate) const FILE_MAX_BYTES: usize = 48 * 1024;
/// One directory listing is capped so a single frame stays bounded.
pub(crate) const DIRECTORY_MAX_ENTRIES: usize = 1000;
/// Highlight spans per file.
pub(crate) const SPAN_MAX: usize = 4000;

/// Binary sniffing window.
const BINARY_WINDOW: usize = 8192;

/// Label + absolute directory, in listing order.
fn labelled_roots(app: &crate::app::App) -> Vec<(String, PathBuf)> {
    let roots = crate::app::side_panel::completion_named_roots(&app.workspace);
    let labels = crate::app::side_panel::unique_root_labels(&roots);
    let mut out: Vec<(String, PathBuf)> = roots
        .into_iter()
        .zip(labels)
        .map(|((_name, path), label)| {
            let label = if label.is_empty() {
                "workspace".to_string()
            } else {
                label
            };
            (label, path)
        })
        .collect();
    // `unique_root_labels` can collapse several nameless roots to the same
    // fallback; suffix repeats instead of producing ambiguous paths.
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (label, _) in out.iter_mut() {
        let count = counts.entry(label.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            *label = format!("{label}-{count}");
        }
    }
    out
}

/// Resolve an explorer display path (`<root-label>/<rel>`) to a canonical
/// absolute path inside a root. `None` when the path is malformed, escapes a
/// root (including through a symlink), or names nothing.
fn resolve(app: &crate::app::App, display: &str) -> Option<PathBuf> {
    if display.is_empty()
        || display.starts_with('/')
        || display.starts_with('\\')
        || Path::new(display).components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return None;
    }

    let roots = labelled_roots(app);
    // Longest label first: a label may contain `/` after disambiguation and
    // may be a prefix of another label.
    let mut indices: Vec<usize> = (0..roots.len()).collect();
    indices.sort_by_key(|&i| std::cmp::Reverse(roots[i].0.len()));

    for i in indices {
        let (label, root) = &roots[i];
        let Some(rest) = display.strip_prefix(label.as_str()) else {
            continue;
        };
        let candidate = if rest.is_empty() {
            root.clone()
        } else {
            let Some(tail) = rest.strip_prefix('/') else {
                continue;
            };
            if tail.is_empty() {
                root.clone()
            } else {
                root.join(tail)
            }
        };
        let Ok(canon_root) = root.canonicalize() else {
            continue;
        };
        let Ok(canon) = candidate.canonicalize() else {
            continue;
        };
        if canon.starts_with(&canon_root) {
            return Some(canon);
        }
    }
    None
}

/// `list_directory` handler: the roots when `path` is absent/empty, otherwise
/// one directory, directories first then case-insensitive name.
pub(crate) fn list_directory(
    app: &crate::app::App,
    path: Option<&str>,
) -> Result<serde_json::Value, CommandFailure> {
    let roots = labelled_roots(app);
    let display = path.unwrap_or("").trim_end_matches('/');

    if display.is_empty() {
        return Ok(json!({
            "path": "",
            "roots": roots
                .iter()
                .map(|(label, _)| json!({ "name": label, "path": label, "is_dir": true }))
                .collect::<Vec<_>>(),
            "entries": [],
            "truncated": false,
        }));
    }

    let Some(dir) = resolve(app, display) else {
        return Err(CommandFailure::new(
            ErrorCode::InvalidPayload,
            "path is outside the workspace",
        ));
    };

    let read = std::fs::read_dir(&dir).map_err(|e| {
        CommandFailure::new(ErrorCode::InvalidPayload, format!("cannot list {display}: {e}"))
    })?;

    let mut entries: Vec<(bool, String, serde_json::Value)> = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| {
            CommandFailure::new(ErrorCode::InvalidPayload, format!("cannot list {display}: {e}"))
        })?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let metadata = entry.metadata().map_err(|e| {
            CommandFailure::new(
                ErrorCode::InvalidPayload,
                format!("cannot stat {display}/{name}: {e}"),
            )
        })?;
        let is_dir = metadata.is_dir();
        let mut value = json!({
            "name": name,
            "path": format!("{display}/{name}"),
            "is_dir": is_dir,
        });
        if !is_dir {
            if let Some(object) = value.as_object_mut() {
                object.insert("size".to_string(), json!(metadata.len()));
            }
        }
        entries.push((is_dir, name.to_lowercase(), value));
    }

    // Directories first, then case-insensitive name.
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let truncated = entries.len() > DIRECTORY_MAX_ENTRIES;
    entries.truncate(DIRECTORY_MAX_ENTRIES);

    Ok(json!({
        "path": display,
        "roots": [],
        "entries": entries.into_iter().map(|(_, _, value)| value).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

/// `read_file` handler: one regular file as UTF-8, head-truncated at
/// [`FILE_MAX_BYTES`], with server-side tree-sitter highlight classes.
pub(crate) fn read_file(
    app: &crate::app::App,
    path: &str,
) -> Result<serde_json::Value, CommandFailure> {
    let Some(absolute) = resolve(app, path) else {
        return Err(CommandFailure::new(
            ErrorCode::InvalidPayload,
            "path is outside the workspace",
        ));
    };

    let metadata = std::fs::metadata(&absolute).map_err(|e| {
        CommandFailure::new(ErrorCode::InvalidPayload, format!("cannot stat {path}: {e}"))
    })?;
    if !metadata.is_file() {
        return Ok(json!({
            "path": path,
            "viewable": false,
            "reason": "not_a_file",
            "bytes": 0,
        }));
    }
    let full_len = metadata.len();

    let mut file = std::fs::File::open(&absolute).map_err(|e| {
        CommandFailure::new(ErrorCode::InvalidPayload, format!("cannot read {path}: {e}"))
    })?;
    let mut buffer = Vec::new();
    (&mut file)
        .take((FILE_MAX_BYTES + 1) as u64)
        .read_to_end(&mut buffer)
        .map_err(|e| {
            CommandFailure::new(ErrorCode::InvalidPayload, format!("cannot read {path}: {e}"))
        })?;

    let truncated = buffer.len() > FILE_MAX_BYTES;
    if truncated {
        buffer.truncate(FILE_MAX_BYTES);
        // Cut at a UTF-8 boundary: only the final 0-3 bytes can be partial.
        while std::str::from_utf8(&buffer).is_err() {
            buffer.pop();
        }
    }

    if looks_binary(&buffer) {
        return Ok(json!({
            "path": path,
            "viewable": false,
            "reason": "binary",
            "bytes": full_len,
        }));
    }

    let text = match std::str::from_utf8(&buffer) {
        Ok(text) => text,
        Err(_) => {
            return Ok(json!({
                "path": path,
                "viewable": false,
                "reason": "not_utf8",
                "bytes": full_len,
            }));
        }
    };

    let extension = Path::new(path).extension().and_then(|e| e.to_str());
    let language = extension.and_then(gaviero_core::tree_sitter::language_name_for_extension);

    let mut classes: Vec<String> = Vec::new();
    let mut spans: Vec<[u32; 3]> = Vec::new();
    let mut spans_truncated = false;

    if !truncated
        && let Some(lang_name) = language
        && let Some(grammar) =
            extension.and_then(gaviero_core::tree_sitter::language_for_extension)
    {
        match crate::editor::highlight::class_spans(text, lang_name, grammar) {
            Ok(found) => {
                for span in found {
                    if spans.len() >= SPAN_MAX {
                        spans_truncated = true;
                        break;
                    }
                    let index = match classes.iter().position(|c| c == &span.class) {
                        Some(index) => index,
                        None => {
                            classes.push(span.class);
                            classes.len() - 1
                        }
                    };
                    spans.push([
                        span.start_byte as u32,
                        span.end_byte as u32,
                        index as u32,
                    ]);
                }
            }
            // Highlighting is best-effort: a query/parse failure must not
            // turn a readable file into an error.
            Err(err) => eprintln!("file_browser: highlighting failed for {path}: {err}"),
        }
    }

    Ok(json!({
        "path": path,
        "viewable": true,
        "language": language,
        "bytes": full_len,
        "truncated": truncated,
        "content": text,
        "classes": classes,
        "spans": spans,
        "spans_truncated": spans_truncated,
    }))
}

/// NUL in the first 8 KiB, or more than 5% C0 controls other than `\t\n\r`.
fn looks_binary(buffer: &[u8]) -> bool {
    let window = &buffer[..buffer.len().min(BINARY_WINDOW)];
    if window.contains(&0) {
        return true;
    }
    let controls = window
        .iter()
        .filter(|byte| {
            let byte = **byte;
            byte < 0x20 && !matches!(byte, b'\t' | b'\n' | b'\r')
        })
        .count();
    controls * 100 > window.len() * 5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> (tempfile::TempDir, crate::app::App) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".gaviero")).unwrap();
        std::fs::write(dir.path().join(".gaviero/settings.json"), "{}").unwrap();
        let ws = gaviero_core::workspace::Workspace::single_folder(dir.path().to_path_buf());
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = crate::app::App::new(ws, tx);
        app.chat_state.new_conversation();
        (dir, app)
    }

    /// The temp directory name is random, so read the label from the server.
    fn root_label(app: &crate::app::App) -> String {
        let listing = list_directory(app, None).unwrap();
        listing["roots"][0]["path"].as_str().unwrap().to_string()
    }

    #[test]
    fn roots_are_labelled_and_listed() {
        let (_dir, app) = test_app();
        let listing = list_directory(&app, None).unwrap();
        let roots = listing["roots"].as_array().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0]["name"], roots[0]["path"]);
        assert_eq!(roots[0]["is_dir"], true);
        assert!(listing["entries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn directory_listing_sorts_and_skips_dot_git() {
        let (dir, app) = test_app();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "[core]").unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let label = root_label(&app);
        let listing = list_directory(&app, Some(&label)).unwrap();
        let entries = listing["entries"].as_array().unwrap();
        assert_eq!(listing["path"], label);
        assert!(
            !entries.iter().any(|e| e["name"] == ".git"),
            ".git must not be listed"
        );

        // `.gaviero` (created by the test app) is a dot-directory and sorts
        // with the other directories; compare the non-dot rows.
        let visible: Vec<&serde_json::Value> = entries
            .iter()
            .filter(|e| !e["name"].as_str().unwrap().starts_with('.'))
            .collect();
        assert_eq!(visible[0]["name"], "src");
        assert_eq!(visible[0]["is_dir"], true);
        assert_eq!(visible[1]["name"], "README.md");
        assert_eq!(visible[1]["is_dir"], false);
        assert_eq!(visible[1]["size"], 5);

        let first_file = entries.iter().position(|e| e["is_dir"] == false).unwrap();
        assert!(entries[..first_file].iter().all(|e| e["is_dir"] == true));

        let sub = list_directory(&app, Some(&format!("{label}/src"))).unwrap();
        assert_eq!(sub["path"], format!("{label}/src"));
        for entry in sub["entries"].as_array().unwrap() {
            assert!(
                entry["path"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("{label}/src/"))
            );
        }
    }

    #[test]
    fn path_resolution_rejects_traversal_and_absolute_paths() {
        let (dir, app) = test_app();
        let label = root_label(&app);
        let mut bad: Vec<String> = vec![
            "../x".to_string(),
            "/etc".to_string(),
            "C:\\Windows".to_string(),
        ];
        bad.push(format!("{label}/../../x"));
        for path in &bad {
            let err = list_directory(&app, Some(path)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidPayload, "accepted {path:?}");
        }

        // A symlink that escapes the root resolves outside it and is refused.
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join("escape");
        let made = {
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok()
            }
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(outside.path(), &link).is_ok()
            }
        };
        if made {
            let err = list_directory(&app, Some(&format!("{label}/escape"))).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidPayload);
        } else {
            eprintln!("symlink creation not permitted here; skipping the escape assertion");
        }
    }

    #[test]
    fn read_file_reports_languages_and_highlighting() {
        let (dir, app) = test_app();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() { let s = \"x\"; }\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("src/app.ts"), "const x: number = 1;\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "# Title\n\nbody café\n").unwrap();
        let label = root_label(&app);

        let rust = read_file(&app, &format!("{label}/src/main.rs")).unwrap();
        assert_eq!(rust["viewable"], true);
        assert_eq!(rust["language"], "rust");
        assert_eq!(rust["truncated"], false);
        let content = rust["content"].as_str().unwrap();
        let classes = rust["classes"].as_array().unwrap();
        assert!(classes.iter().any(|c| c == "keyword"));
        let spans = rust["spans"].as_array().unwrap();
        assert!(!spans.is_empty(), "rust file must carry spans");
        let mut previous_end = 0usize;
        for span in spans {
            let start = span[0].as_u64().unwrap() as usize;
            let end = span[1].as_u64().unwrap() as usize;
            let index = span[2].as_u64().unwrap() as usize;
            assert!(start >= previous_end, "spans must be sorted and disjoint");
            assert!(start < end && end <= content.len());
            assert!(index < classes.len());
            assert!(content.is_char_boundary(start) && content.is_char_boundary(end));
            previous_end = end;
        }

        let typescript = read_file(&app, &format!("{label}/src/app.ts")).unwrap();
        assert_eq!(typescript["language"], "typescript");
        let ts_classes = typescript["classes"].as_array().unwrap();
        let ts_spans = typescript["spans"].as_array().unwrap();
        assert!(!ts_spans.is_empty());
        assert!(ts_spans.iter().any(|span| {
            let index = span[2].as_u64().unwrap() as usize;
            let class = ts_classes[index].as_str().unwrap();
            let head = class.split('.').next().unwrap_or(class);
            matches!(head, "keyword" | "type")
        }));

        let markdown = read_file(&app, &format!("{label}/README.md")).unwrap();
        assert_eq!(markdown["language"], "markdown");
        assert!(markdown["spans"].as_array().unwrap().is_empty());
        assert_eq!(markdown["content"], "# Title\n\nbody café\n");
    }

    #[test]
    fn read_file_marks_binary_and_truncated() {
        let (dir, app) = test_app();
        std::fs::write(dir.path().join("big.txt"), "a".repeat(60_000)).unwrap();
        std::fs::write(
            dir.path().join("logo.png"),
            [0x89u8, b'P', b'N', b'G', 0, 0],
        )
        .unwrap();
        let label = root_label(&app);

        let big = read_file(&app, &format!("{label}/big.txt")).unwrap();
        assert_eq!(big["truncated"], true);
        assert_eq!(big["content"].as_str().unwrap().len(), FILE_MAX_BYTES);
        assert_eq!(big["bytes"], 60_000u64);
        assert!(big["spans"].as_array().unwrap().is_empty());

        let png = read_file(&app, &format!("{label}/logo.png")).unwrap();
        assert_eq!(png["viewable"], false);
        assert_eq!(png["reason"], "binary");
        assert_eq!(png["bytes"], 6u64);
    }

    #[test]
    fn file_result_stays_under_the_frame_cap() {
        let (dir, app) = test_app();
        let unit = "fn f() { let value = 1; }\n";
        let mut source = String::new();
        while source.len() < 40_000 {
            source.push_str(unit);
        }
        std::fs::write(dir.path().join("big.rs"), &source).unwrap();
        let label = root_label(&app);

        let result = read_file(&app, &format!("{label}/big.rs")).unwrap();
        assert_eq!(result["truncated"], false);
        let encoded = serde_json::to_vec(&result).unwrap();
        assert!(
            encoded.len() < 256 * 1024,
            "encoded result is {} bytes",
            encoded.len()
        );
        assert!(result["spans"].as_array().unwrap().len() <= SPAN_MAX);
    }
}
