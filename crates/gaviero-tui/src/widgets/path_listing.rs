//! Filesystem path completion shared by the chat (`@`, `/add-dir`), the
//! SWARM form and the session host's new-session picker.

/// A typed path the filesystem should complete rather than the workspace
/// file list: rooted (`/`, `\`), home-relative (`~`), or a Windows drive
/// path (`C:\`, `C:/`).
pub fn is_filesystem_query(query: &str) -> bool {
    let b = query.as_bytes();
    query.starts_with(['/', '\\'])
        || query == "~"
        || query.starts_with("~/")
        || query.starts_with("~\\")
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'/' | b'\\'))
}

pub fn ends_with_separator(path: &str) -> bool {
    path.ends_with(['/', '\\'])
}

/// Every directory entry matching the user's typed prefix, at most
/// `max_entries` of them.
///
/// The query is split into a parent directory (everything up to and
/// including the last `/` or `\`) and a filename prefix; entries in the
/// parent whose names start with the prefix are returned as the typed parent
/// plus the name, with a trailing separator for subdirectories. `~` is
/// expanded against `$HOME`. With a `base`, the query is relative to it and
/// needs no separator; without one, a query without a separator lists
/// nothing.
pub fn list_filesystem_entries(
    query: &str,
    base: Option<&std::path::Path>,
    max_entries: usize,
) -> Vec<String> {
    let expanded = expand_tilde(query);
    let (parent, prefix) = match expanded.rfind(['/', '\\']) {
        Some(idx) => (&expanded[..=idx], &expanded[idx + 1..]),
        None if base.is_some() => ("", expanded.as_str()),
        None => return Vec::new(),
    };
    let dir = match base {
        Some(base) => base.join(parent),
        None if parent.is_empty() => std::path::PathBuf::from("/"),
        None => std::path::PathBuf::from(parent),
    };
    let entries = match std::fs::read_dir(&dir) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let mut matches: Vec<String> = Vec::new();
    let display_prefix = display_dir_prefix(query);
    // A completed folder ends with the separator the user is typing.
    let separator = if display_prefix.ends_with('\\') { "\\" } else { "/" };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !prefix.is_empty() && !name_str.starts_with(prefix) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let suffix = if is_dir { separator } else { "" };
        matches.push(format!("{}{}{}", display_prefix, name_str, suffix));
        if matches.len() >= max_entries {
            break;
        }
    }
    matches.sort();
    matches
}

pub fn expand_tilde(query: &str) -> String {
    if let Some(rest) = query.strip_prefix("~/").or_else(|| query.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return format!("{}/{}", home.display(), rest);
        }
    } else if query == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home.display().to_string();
    }
    query.to_string()
}

pub fn display_dir_prefix(query: &str) -> String {
    match query.rfind(['/', '\\']) {
        Some(idx) => query[..=idx].to_string(),
        None => String::new(),
    }
}
