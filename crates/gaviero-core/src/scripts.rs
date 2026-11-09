//! Catalog of named workflow scripts: `.gaviero/dsl/<name>/<name>.gaviero`.
//!
//! Discovery, scopes and name resolution are those of skills
//! ([`crate::scoped_folders`]). A script is listed without being compiled:
//! core has no DSL dependency and a scan runs on every relevant file event,
//! so the only thing read here is the optional `// description:` header.
//! `.gaviero/dsl/profiles/` is not a script: it holds the model profiles
//! ([`crate::script_profiles`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::memory::scope::SCOPE_REPO;
use crate::script_profiles::PROFILES_SUBDIR;
use crate::scoped_folders::{
    ScopedEntry, find_entry_file, folder_labels, is_under_native_dir, native_roots,
    native_scope_label, nearest, sort_nearest_first,
};
use crate::skills::catalog::ScannedDir;
use crate::skills::{SkillWarning, valid_stem};
use crate::workspace::Workspace;

/// Directory under `.gaviero/` holding one folder per script.
pub const SCRIPTS_SUBDIR: &str = "dsl";

/// The main script of the folder called `name`, matched case-insensitively.
/// Any other file in the folder is only reachable through `include`.
pub fn script_file_name(name: &str) -> String {
    format!("{name}.gaviero")
}

/// A script found by [`ScriptCatalog::scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    pub name: String,
    /// From a leading `// description: …` comment.
    pub description: Option<String>,
    pub scope_level: i32,
    pub source_path: PathBuf,
}

impl ScopedEntry for Script {
    fn entry_name(&self) -> &str {
        &self.name
    }
    fn scope_level(&self) -> i32 {
        self.scope_level
    }
    fn entry_path(&self) -> &Path {
        &self.source_path
    }
}

/// Scripts discovered under the workspace and global roots.
#[derive(Debug, Clone, Default)]
pub struct ScriptCatalog {
    by_name: HashMap<String, Vec<Script>>,
    folder_index: HashMap<PathBuf, String>,
    scanned: Vec<ScannedDir>,
    warnings: Vec<SkillWarning>,
}

impl ScriptCatalog {
    pub fn global_scripts_dir() -> PathBuf {
        crate::workspace::user_gaviero_dir()
            .unwrap_or_else(|| PathBuf::from(".gaviero"))
            .join(SCRIPTS_SUBDIR)
    }

    /// Scan every scope. Folders that cannot be listed become warnings.
    pub fn scan(workspace: &Workspace, global_dir: &Path) -> Self {
        let mut catalog = Self {
            folder_index: folder_labels(workspace),
            ..Self::default()
        };
        for root in native_roots(workspace, SCRIPTS_SUBDIR, global_dir) {
            catalog.scanned.push(ScannedDir {
                scope: root.scope,
                exists: root.path.is_dir(),
                path: root.path.clone(),
            });
            catalog.scan_dir(&root.path, root.level);
        }
        catalog
    }

    fn scan_dir(&mut self, dir: &Path, scope_level: i32) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            let name = folder.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            if name.starts_with('.') || name.eq_ignore_ascii_case(PROFILES_SUBDIR) {
                continue;
            }
            let warn = |message: String| SkillWarning {
                name: name.to_string(),
                message,
            };
            if !valid_stem(name) {
                self.warnings.push(warn("invalid script name charset".into()));
                continue;
            }
            let file_name = script_file_name(name);
            let Some(source_path) = find_entry_file(&folder, &file_name) else {
                self.warnings.push(warn(format!(
                    "script folder {} is missing {file_name}",
                    folder.display()
                )));
                continue;
            };
            let contents = match std::fs::read_to_string(&source_path) {
                Ok(c) => c,
                Err(e) => {
                    self.warnings
                        .push(warn(format!("failed to read {}: {e}", source_path.display())));
                    continue;
                }
            };
            self.by_name
                .entry(name.to_string())
                .or_default()
                .push(Script {
                    name: name.to_string(),
                    description: description_header(&contents),
                    scope_level,
                    source_path,
                });
        }
    }

    /// `global`, `workspace`, or the owning folder's display name.
    pub fn source_label(&self, script: &Script) -> String {
        native_scope_label(
            script.scope_level,
            &script.source_path,
            &self.folder_index,
            SCRIPTS_SUBDIR,
        )
    }

    /// Resolve by optional source qualifier and bare name, as `$source/name`
    /// and `$name` do for skills.
    pub fn resolve(
        &self,
        qualifier: Option<&str>,
        name: &str,
        active_repo_id: Option<&str>,
    ) -> Option<&Script> {
        let scripts = self.by_name.get(name)?;
        if let Some(q) = qualifier {
            return scripts.iter().find(|s| self.source_label(s) == q);
        }
        nearest(scripts, active_repo_id, SCRIPTS_SUBDIR)
    }

    /// Every definition whose name starts with `prefix`, nearest first.
    pub fn complete(&self, prefix: &str, active_repo_id: Option<&str>) -> Vec<&Script> {
        let mut out: Vec<&Script> = self
            .by_name
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .flat_map(|(_, scripts)| scripts.iter())
            .collect();
        sort_nearest_first(&mut out, active_repo_id, SCRIPTS_SUBDIR);
        out
    }

    /// Every directory the scan looked in, in scan order.
    pub fn scanned_dirs(&self) -> &[ScannedDir] {
        &self.scanned
    }

    /// Folders found but not listed, with the reason.
    pub fn warnings(&self) -> &[SkillWarning] {
        &self.warnings
    }

    /// True when `event_path` lies under any `*/.gaviero/dsl/` directory.
    pub fn needs_rebuild(event_path: &Path) -> bool {
        is_under_native_dir(event_path, SCRIPTS_SUBDIR)
    }
}

/// True for a script defined in a workspace folder (not the workspace layer
/// or the global directory).
pub fn is_repo_scoped(script: &Script) -> bool {
    script.scope_level == SCOPE_REPO
}

/// The value of a `// description: …` line among the script's leading
/// comments. Comments are the one place the DSL grammar leaves free.
fn description_header(contents: &str) -> Option<String> {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let comment = line.strip_prefix("//")?.trim();
        if let Some((key, value)) = comment.split_once(':')
            && key.trim().eq_ignore_ascii_case("description")
        {
            let value = value.trim();
            return (!value.is_empty()).then(|| value.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::scope::hash_path;
    use std::fs;

    fn write_script(root: &Path, name: &str, body: &str) -> PathBuf {
        let dir = root.join(".gaviero").join(SCRIPTS_SUBDIR).join(name);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(script_file_name(name));
        fs::write(&path, body).unwrap();
        path
    }

    fn scan(ws: &Workspace, global: &Path) -> ScriptCatalog {
        ScriptCatalog::scan(ws, global)
    }

    #[test]
    fn lists_scripts_with_their_description_header() {
        let tmp = tempfile::tempdir().unwrap();
        write_script(
            tmp.path(),
            "audit",
            "// A security pass\n// description: Audit the auth module\n\nagent a { }\n",
        );
        write_script(tmp.path(), "plain", "agent a { }\n");

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = scan(&ws, Path::new("/nonexistent"));

        let audit = catalog.resolve(None, "audit", None).unwrap();
        assert_eq!(audit.description.as_deref(), Some("Audit the auth module"));
        assert_eq!(catalog.resolve(None, "plain", None).unwrap().description, None);
    }

    #[test]
    fn description_is_only_read_from_the_leading_comments() {
        assert_eq!(
            description_header("agent a { }\n// description: too late\n"),
            None
        );
        assert_eq!(
            description_header("\u{feff}// Description: with bom\n").as_deref(),
            Some("with bom")
        );
    }

    #[test]
    fn folder_without_a_script_is_a_warning_not_an_entry() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join(".gaviero/dsl/empty")).unwrap();

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = scan(&ws, Path::new("/nonexistent"));

        assert!(catalog.complete("", None).is_empty());
        assert_eq!(catalog.warnings().len(), 1);
        assert!(catalog.warnings()[0].message.contains("empty.gaviero"));
    }

    #[test]
    fn only_the_file_named_after_the_folder_is_the_script() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".gaviero/dsl/audit");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("DSL.gaviero"), "agent a { }\n").unwrap();
        fs::write(dir.join("helpers.gaviero"), "agent b { }\n").unwrap();

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = scan(&ws, Path::new("/nonexistent"));
        assert!(catalog.complete("", None).is_empty());
        assert!(catalog.warnings()[0].message.contains("audit.gaviero"));

        let main = dir.join("audit.gaviero");
        fs::write(&main, "agent a { }\n").unwrap();
        let catalog = scan(&ws, Path::new("/nonexistent"));
        assert_eq!(catalog.complete("", None).len(), 1, "one script per folder");
        assert_eq!(catalog.resolve(None, "audit", None).unwrap().source_path, main);
    }

    #[test]
    fn the_profiles_folder_is_neither_a_script_nor_a_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".gaviero/dsl").join(PROFILES_SUBDIR);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("claude.gaviero"), "// profile\n").unwrap();

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = scan(&ws, Path::new("/nonexistent"));
        assert!(catalog.complete("", None).is_empty());
        assert!(catalog.warnings().is_empty(), "{:?}", catalog.warnings());
    }

    #[test]
    fn script_file_name_is_case_insensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".gaviero/dsl/lower");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("LOWER.gaviero"), "agent a { }\n").unwrap();

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        assert!(scan(&ws, Path::new("/nonexistent")).resolve(None, "lower", None).is_some());
    }

    #[test]
    fn repo_shadows_global_and_a_qualifier_reaches_the_other() {
        let repo = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let repo_path = write_script(repo.path(), "review", "// description: repo\n");
        let global_dir = global.path().join("dsl");
        fs::create_dir_all(global_dir.join("review")).unwrap();
        fs::write(
            global_dir.join("review").join(script_file_name("review")),
            "// description: global\n",
        )
        .unwrap();

        let ws = Workspace::single_folder(repo.path().to_path_buf());
        let catalog = scan(&ws, &global_dir);

        assert_eq!(catalog.resolve(None, "review", None).unwrap().source_path, repo_path);
        assert_eq!(
            catalog
                .resolve(Some("global"), "review", None)
                .unwrap()
                .description
                .as_deref(),
            Some("global")
        );
        assert_eq!(catalog.complete("rev", None).len(), 2, "both stay listable");
    }

    #[test]
    fn active_folder_wins_between_workspace_folders() {
        let outer = tempfile::tempdir().unwrap();
        let a = outer.path().join("a");
        let b = outer.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        write_script(&a, "dup", "// description: from a\n");
        let in_b = write_script(&b, "dup", "// description: from b\n");

        let mut ws = Workspace::single_folder(a.clone());
        ws.add_root(b.clone(), None);
        let catalog = scan(&ws, Path::new("/nonexistent"));

        let active = hash_path(&b.canonicalize().unwrap());
        assert_eq!(
            catalog.resolve(None, "dup", Some(&active)).unwrap().source_path,
            in_b
        );
        assert_eq!(catalog.complete("", Some(&active))[0].source_path, in_b);
    }

    #[test]
    fn rebuild_predicate_matches_only_the_script_folders() {
        assert!(ScriptCatalog::needs_rebuild(Path::new(
            "/p/.gaviero/dsl/x/x.gaviero"
        )));
        assert!(!ScriptCatalog::needs_rebuild(Path::new(
            "/p/.gaviero/skills/x/SKILL.md"
        )));
        assert!(!ScriptCatalog::needs_rebuild(Path::new("/p/src/dsl/x.rs")));
    }
}
