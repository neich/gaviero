//! Named model profiles for workflow scripts: `.gaviero/dsl/profiles/<name>.gaviero`.
//!
//! A profile says which client each tier means for one run, so a script can
//! ask for `tier high` without naming a provider. Discovery and nearest-scope
//! resolution match scripts ([`crate::scoped_folders`]). Only the location is
//! decided here: core has no DSL dependency, so `gaviero-dsl` reads the file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::scoped_folders::{ScopedEntry, nearest, native_roots};
use crate::scripts::SCRIPTS_SUBDIR;
use crate::workspace::Workspace;

/// Directory under `.gaviero/dsl/` holding the profile files. Reserved: the
/// script catalog does not treat it as a script folder.
pub const PROFILES_SUBDIR: &str = "profiles";

/// The profile used when a run does not choose one.
pub const DEFAULT_PROFILE_NAME: &str = "default";

const PROFILE_EXTENSION: &str = "gaviero";

/// A profile found by [`ProfileCatalog::scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub scope_level: i32,
    pub source_path: PathBuf,
}

impl ScopedEntry for Profile {
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

/// Who wins when a script binds a tier the profile also binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfilePrecedence {
    /// Chosen for this run: the profile replaces the script's bindings.
    OverridesScript,
    /// Picked up implicitly as `default`: it only supplies what the script
    /// leaves open, so a script that pins its own tiers keeps them.
    FillsGaps,
}

/// The profile a run compiles with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedProfile {
    pub name: String,
    pub path: PathBuf,
    pub precedence: ProfilePrecedence,
}

/// Profiles discovered under the workspace and global roots.
#[derive(Debug, Clone, Default)]
pub struct ProfileCatalog {
    by_name: HashMap<String, Vec<Profile>>,
    scanned: Vec<PathBuf>,
}

impl ProfileCatalog {
    /// Scan every scope; `global_scripts_dir` is the user's `.gaviero/dsl`
    /// ([`crate::scripts::ScriptCatalog::global_scripts_dir`]). A folder that
    /// cannot be listed holds no profiles.
    pub fn scan(workspace: &Workspace, global_scripts_dir: &Path) -> Self {
        let mut catalog = Self::default();
        for root in native_roots(workspace, SCRIPTS_SUBDIR, global_scripts_dir) {
            let dir = root.path.join(PROFILES_SUBDIR);
            catalog.scan_dir(&dir, root.level);
            catalog.scanned.push(dir);
        }
        catalog
    }

    fn scan_dir(&mut self, dir: &Path, scope_level: i32) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_profile = path.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case(PROFILE_EXTENSION));
            if !is_profile {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            self.by_name
                .entry(name.to_string())
                .or_default()
                .push(Profile {
                    name: name.to_string(),
                    scope_level,
                    source_path: path.clone(),
                });
        }
    }

    /// The nearest definition of `name`.
    pub fn resolve(&self, name: &str, active_repo_id: Option<&str>) -> Option<&Profile> {
        nearest(self.by_name.get(name)?, active_repo_id, SCRIPTS_SUBDIR)
    }

    /// Every profile name, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.by_name.keys().cloned().collect();
        names.sort();
        names
    }

    /// Which profile a run uses. `requested` is a catalog name or a path to a
    /// `.gaviero` file (relative paths start at `base_dir`); `None` means the
    /// `default` profile if there is one, else no profile at all.
    pub fn select(
        &self,
        requested: Option<&str>,
        active_repo_id: Option<&str>,
        base_dir: &Path,
    ) -> Result<Option<SelectedProfile>, String> {
        let Some(requested) = requested.map(str::trim).filter(|r| !r.is_empty()) else {
            return Ok(self
                .resolve(DEFAULT_PROFILE_NAME, active_repo_id)
                .map(|profile| SelectedProfile {
                    name: profile.name.clone(),
                    path: profile.source_path.clone(),
                    precedence: ProfilePrecedence::FillsGaps,
                }));
        };

        let (name, path) = if looks_like_path(requested) {
            let path = base_dir.join(requested);
            if !path.is_file() {
                return Err(format!("profile file not found: {}", path.display()));
            }
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| requested.to_string());
            (name, path)
        } else {
            let profile = self
                .resolve(requested, active_repo_id)
                .ok_or_else(|| self.unknown_profile_message(requested))?;
            (profile.name.clone(), profile.source_path.clone())
        };

        Ok(Some(SelectedProfile {
            name,
            path,
            precedence: ProfilePrecedence::OverridesScript,
        }))
    }

    fn unknown_profile_message(&self, requested: &str) -> String {
        let searched = self
            .scanned
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let names = self.names();
        if names.is_empty() {
            format!("profile `{requested}` not found: no profiles exist (searched {searched})")
        } else {
            format!(
                "profile `{requested}` not found; available: {} (searched {searched})",
                names.join(", ")
            )
        }
    }
}

fn looks_like_path(requested: &str) -> bool {
    requested.contains(['/', '\\'])
        || Path::new(requested)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(PROFILE_EXTENSION))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::scope::hash_path;
    use std::fs;

    fn write_profile(root: &Path, name: &str) -> PathBuf {
        let dir = root.join(".gaviero").join(SCRIPTS_SUBDIR).join(PROFILES_SUBDIR);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.gaviero"));
        fs::write(&path, "// profile\n").unwrap();
        path
    }

    fn write_global(global_scripts_dir: &Path, name: &str) -> PathBuf {
        let dir = global_scripts_dir.join(PROFILES_SUBDIR);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.gaviero"));
        fs::write(&path, "// profile\n").unwrap();
        path
    }

    #[test]
    fn lists_only_gaviero_files_by_stem() {
        let tmp = tempfile::tempdir().unwrap();
        write_profile(tmp.path(), "claude");
        write_profile(tmp.path(), "mixed");
        let dir = tmp.path().join(".gaviero/dsl/profiles");
        fs::write(dir.join("notes.md"), "x").unwrap();
        fs::create_dir_all(dir.join("folder.gaviero")).unwrap();

        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        assert_eq!(catalog.names(), ["claude", "mixed"]);
    }

    #[test]
    fn repo_profile_shadows_a_global_one() {
        let repo = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let local = write_profile(repo.path(), "claude");
        write_global(global.path(), "claude");

        let ws = Workspace::single_folder(repo.path().to_path_buf());
        let catalog = ProfileCatalog::scan(&ws, global.path());
        let found = catalog.resolve("claude", None).unwrap();
        assert_eq!(found.source_path, local);
    }

    #[test]
    fn active_folder_wins_between_workspace_folders() {
        let outer = tempfile::tempdir().unwrap();
        let a = outer.path().join("a");
        let b = outer.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        write_profile(&a, "dup");
        let in_b = write_profile(&b, "dup");

        let mut ws = Workspace::single_folder(a.clone());
        ws.add_root(b.clone(), None);
        let catalog = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        let active = hash_path(&b.canonicalize().unwrap());
        let selected = catalog
            .select(Some("dup"), Some(&active), Path::new("."))
            .unwrap()
            .unwrap();
        assert_eq!(selected.path, in_b);
    }

    #[test]
    fn no_request_means_the_default_profile_filling_gaps() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::single_folder(tmp.path().to_path_buf());

        let empty = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        assert_eq!(empty.select(None, None, tmp.path()).unwrap(), None);

        let default = write_profile(tmp.path(), DEFAULT_PROFILE_NAME);
        let catalog = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        let selected = catalog.select(None, None, tmp.path()).unwrap().unwrap();
        assert_eq!(selected.path, default);
        assert_eq!(selected.precedence, ProfilePrecedence::FillsGaps);
    }

    #[test]
    fn asking_for_default_by_name_overrides_like_any_other_choice() {
        let tmp = tempfile::tempdir().unwrap();
        write_profile(tmp.path(), DEFAULT_PROFILE_NAME);
        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        let selected = catalog
            .select(Some(DEFAULT_PROFILE_NAME), None, tmp.path())
            .unwrap()
            .unwrap();
        assert_eq!(selected.precedence, ProfilePrecedence::OverridesScript);
    }

    #[test]
    fn a_path_is_used_directly_and_must_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        fs::write(tmp.path().join("adhoc.gaviero"), "// p\n").unwrap();

        let selected = catalog
            .select(Some("adhoc.gaviero"), None, tmp.path())
            .unwrap()
            .unwrap();
        assert_eq!(selected.name, "adhoc");
        assert_eq!(selected.path, tmp.path().join("adhoc.gaviero"));

        let missing = catalog.select(Some("nope/x.gaviero"), None, tmp.path());
        assert!(missing.unwrap_err().contains("not found"));
    }

    #[test]
    fn an_unknown_name_lists_what_exists() {
        let tmp = tempfile::tempdir().unwrap();
        write_profile(tmp.path(), "claude");
        let ws = Workspace::single_folder(tmp.path().to_path_buf());
        let catalog = ProfileCatalog::scan(&ws, Path::new("/nonexistent"));
        let message = catalog
            .select(Some("cladue"), None, tmp.path())
            .unwrap_err();
        assert!(message.contains("cladue") && message.contains("claude"), "{message}");
    }
}
