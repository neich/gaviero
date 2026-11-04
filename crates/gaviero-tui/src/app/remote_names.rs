//! Machine-unique workspace display names for the remote sidecar.
//!
//! The phone lists every instance on a machine by `workspace.display_name`,
//! which is the workspace root's folder name. Two workspaces in folders with
//! the same name (`…\git\gaviero\gaviero` and `…\Dropbox\gaviero`) would
//! both read "gaviero". Each workspace that starts a sidecar records itself
//! under `~/.gaviero/remote/workspaces/<workspace_id>.json` (id + root, local
//! to this machine, never sent over the wire), and a name that collides with
//! another recorded workspace gains the nearest parent folder that tells them
//! apart: `gaviero (git)` / `gaviero (Dropbox)`.
//!
//! Every instance writes its own record before it reads the others, so of
//! two instances starting at the same moment at least one sees the other and
//! the two names still differ. Records persist, so a name stays the same
//! across restarts and the phone's list (which keeps offline entries) never
//! shows two identical names. A record whose root no longer exists is
//! ignored; deleting a record file forgets that workspace.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Subdirectory of the machine remote state dir holding one record per
/// workspace.
const DIR: &str = "workspaces";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    workspace_id: String,
    root: PathBuf,
}

/// The workspace's plain name: its root's folder name.
pub fn base_name(root: &Path) -> String {
    root.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".to_string())
}

/// Record this workspace (best-effort: a write failure only means other
/// instances cannot see it yet).
pub fn record(machine_dir: &Path, workspace_id: &str, root: &Path) {
    let dir = machine_dir.join(DIR);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let record = Record {
        workspace_id: workspace_id.to_string(),
        root: root.to_path_buf(),
    };
    let Ok(body) = serde_json::to_vec_pretty(&record) else {
        return;
    };
    let dest = dir.join(format!("{}.json", sanitize(workspace_id)));
    let tmp = dest.with_extension("json.tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, &dest);
    }
}

/// This workspace's display name given every recorded workspace on the
/// machine. Read-only.
pub fn unique_name(machine_dir: &Path, workspace_id: &str, root: &Path) -> String {
    let others: Vec<PathBuf> = read_records(machine_dir)
        .into_iter()
        .filter(|r| r.workspace_id != workspace_id && r.root.exists())
        .map(|r| r.root)
        .collect();
    disambiguate(root, &others)
}

fn read_records(machine_dir: &Path) -> Vec<Record> {
    let Ok(entries) = std::fs::read_dir(machine_dir.join(DIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|text| serde_json::from_str(&text).ok())
        .collect()
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// `root`'s base name, qualified when another root has the same base name
/// (case-insensitive — Windows folder names). The qualifier is the shortest
/// run of parent folders, nearest first and skipping folders named like the
/// base itself, that no colliding root shares: `git` for
/// `…\git\gaviero\gaviero` against `…\Dropbox\gaviero`.
fn disambiguate(root: &Path, others: &[PathBuf]) -> String {
    let base = base_name(root);
    let key = base.to_lowercase();
    let colliding: Vec<&PathBuf> = others
        .iter()
        .filter(|o| base_name(o).to_lowercase() == key)
        .collect();
    if colliding.is_empty() {
        return base;
    }
    let own = qualifiers(root, &key);
    let theirs: Vec<Vec<String>> = colliding.iter().map(|o| qualifiers(o, &key)).collect();
    for k in 1..=own.len() {
        let mine = qualifier(&own, k);
        if theirs.iter().all(|t| qualifier(t, k).to_lowercase() != mine.to_lowercase()) {
            return format!("{base} ({mine})");
        }
    }
    // Same folder names all the way up (e.g. another drive): the full path
    // is the only thing left that differs.
    format!("{base} ({})", root.display())
}

/// Parent folder names, nearest first, without folders named like the base.
fn qualifiers(root: &Path, base_key: &str) -> Vec<String> {
    let mut names: Vec<String> = root
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| match c {
                    Component::Normal(n) => Some(n.to_string_lossy().to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    names.reverse();
    names.retain(|n| n.to_lowercase() != base_key);
    names
}

/// The `k` nearest qualifiers in path order (`work/git` for k = 2).
fn qualifier(names: &[String], k: usize) -> String {
    let mut picked: Vec<&str> = names.iter().take(k).map(String::as_str).collect();
    picked.reverse();
    picked.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn a_name_without_collision_stays_plain() {
        assert_eq!(disambiguate(&p("/home/u/git/gaviero"), &[p("/home/u/git/hcol")]), "gaviero");
        assert_eq!(disambiguate(&p("/home/u/git/gaviero"), &[]), "gaviero");
    }

    #[test]
    fn colliding_names_take_the_nearest_differing_parent() {
        let repo = p("/home/u/Dropbox/work/git/gaviero/gaviero");
        let notes = p("/home/u/Dropbox/gaviero");
        assert_eq!(disambiguate(&repo, std::slice::from_ref(&notes)), "gaviero (git)");
        assert_eq!(disambiguate(&notes, std::slice::from_ref(&repo)), "gaviero (Dropbox)");
    }

    #[test]
    fn a_shared_nearest_parent_extends_the_qualifier() {
        let a = p("/home/u/one/src/app");
        let b = p("/home/u/two/src/app");
        assert_eq!(disambiguate(&a, std::slice::from_ref(&b)), "app (one/src)");
        assert_eq!(disambiguate(&b, std::slice::from_ref(&a)), "app (two/src)");
    }

    #[test]
    fn collisions_ignore_case() {
        let a = p("/home/u/x/Gaviero");
        let b = p("/home/u/y/gaviero");
        assert_eq!(disambiguate(&a, std::slice::from_ref(&b)), "Gaviero (x)");
    }

    #[test]
    fn records_round_trip_and_skip_self_and_missing_roots() {
        let machine = tempfile::tempdir().unwrap();
        let roots = tempfile::tempdir().unwrap();
        let repo = roots.path().join("git").join("gaviero");
        let notes = roots.path().join("notes").join("gaviero");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&notes).unwrap();

        record(machine.path(), "repo", &repo);
        // Alone on the machine (its own record is not a collision).
        assert_eq!(unique_name(machine.path(), "repo", &repo), "gaviero");

        record(machine.path(), "notes", &notes);
        assert_eq!(unique_name(machine.path(), "repo", &repo), "gaviero (git)");
        assert_eq!(unique_name(machine.path(), "notes", &notes), "gaviero (notes)");

        // A recorded workspace whose folder is gone no longer qualifies.
        std::fs::remove_dir_all(&notes).unwrap();
        assert_eq!(unique_name(machine.path(), "repo", &repo), "gaviero");
    }
}
