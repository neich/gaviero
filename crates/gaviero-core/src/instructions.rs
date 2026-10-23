//! `GAVIERO.md` — one provider-neutral agent instruction file.
//!
//! When a project `GAVIERO.md` is in scope for an agent's workspace roots,
//! gaviero owns project instructions for every provider:
//!
//! * each provider's native discovery is switched off —
//!   Claude Code: `claudeMdExcludes` passed through `--settings`
//!   ([`claude_settings_arg`]); Codex: `project_doc_max_bytes=0`
//!   ([`CODEX_DISABLE_PROJECT_DOCS`]); dsh: the `agent-instructions` plugin row
//!   disabled by a patch overlay ([`DSH_DISABLE_INSTRUCTIONS_PATCH`]); Cursor
//!   (no flag exists): a managed `.cursorignore` block ([`sync_cursorignore`]);
//! * the `GAVIERO.md` text is appended to the system prompt
//!   ([`append_to_system_prompt`]). DeepSeek in-process and Ollama have no
//!   native discovery, so they only get the injection.
//!
//! With no `GAVIERO.md` in scope nothing changes: providers load their own
//! `CLAUDE.md` / `AGENTS.md` as before.
//!
//! There is no file watcher. Discovery runs each time a session is
//! constructed, and the TUI spawns a fresh provider session every turn, so an
//! edited `GAVIERO.md` takes effect on the next turn — explicitly after
//! `/reset` or `/compact`.
//!
//! Scope, per workspace root, broadest first (a later file wins):
//! * `$GAVIERO_ROOT/.gaviero/GAVIERO.md` — the user-wide baseline, loaded even
//!   when the repository has none (it takes over `~/.claude/CLAUDE.md`);
//! * the directory holding an enclosing `*.gaviero-workspace` file;
//! * the repository root down to the workspace root, stopping at the first
//!   directory holding `.git` (only the root itself when there is none).
//!
//! Only the project/workspace layers above switch provider-native discovery
//! off; the user-wide file alone does not. A `GAVIERO.md` further down the tree
//! is folder-scoped, like a nested `CLAUDE.md`: it is listed by path (honouring
//! `.gitignore`) for the agent to read before working in that folder, and on
//! its own does not change how a provider loads its own files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The instruction file name.
pub const INSTRUCTIONS_FILE: &str = "GAVIERO.md";

/// Inlined bytes across every file; the rest is cut with a pointer to the
/// file. Same budget dsh uses for its own instruction discovery.
const MAX_INLINE_BYTES: usize = 64 * 1024;

/// Nested files listed by path; more are summarised as a count.
const MAX_NESTED: usize = 64;

/// Claude Code memory files skipped while `GAVIERO.md` is in scope. Claude
/// 2.1.306 also loads `AGENTS.md`, and `claudeMdExcludes` covers it (probed
/// 2026-10-23). Patterns match absolute paths, so the user-level
/// `~/.claude/CLAUDE.md` is excluded too.
pub const CLAUDE_NATIVE_EXCLUDES: &[&str] = &[
    "**/CLAUDE.md",
    "**/CLAUDE.local.md",
    "**/.claude/rules/**",
    "**/AGENTS.md",
];

/// Codex `--config` override that turns off `AGENTS.md` /
/// `AGENTS.override.md` / fallback-name discovery (probed on codex-cli
/// 0.163.0).
pub const CODEX_DISABLE_PROJECT_DOCS: &str = "project_doc_max_bytes=0";

/// dsh patch overlay disabling its `AGENTS.md` / `CLAUDE.md` discovery
/// plugin (`@deepseek-ai/dsh-agent-instructions`, row id
/// `agent-instructions` in `dsh --profile acp --dump-config`).
pub const DSH_DISABLE_INSTRUCTIONS_PATCH: &str = "\
# Written by gaviero; regenerated on every dsh: launch. Do not edit.
# GAVIERO.md is in scope: gaviero injects it, so dsh's own AGENTS.md /
# CLAUDE.md discovery is switched off.
- id: agent-instructions
  disabled: true
";
pub const DSH_DISABLE_INSTRUCTIONS_PATCH_FILE: &str = "gaviero-no-instructions.patch.yml";

const CURSORIGNORE_FILE: &str = ".cursorignore";
const CURSORIGNORE_BEGIN: &str = "# >>> gaviero: GAVIERO.md replaces these (managed block)";
const CURSORIGNORE_END: &str = "# <<< gaviero";
/// Cursor reads `AGENTS.md` unconditionally and `CLAUDE.md` /
/// `CLAUDE.local.md` with third-party extensibility on. `.cursorignore`
/// excludes them from file access; whether the rule loader honours it as well
/// is not guaranteed, which is why the injected block says to ignore a native
/// file if one still appears. Gitignore syntax: a bare name matches at every
/// depth.
const CURSOR_IGNORED: &[&str] = &[
    "AGENTS.md",
    "CLAUDE.md",
    "CLAUDE.local.md",
    ".cursorrules",
    ".cursor/rules/",
];

/// The `GAVIERO.md` files in scope for one agent session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectInstructions {
    /// Inlined files, broadest first (a later file wins).
    pub inline: Vec<(PathBuf, String)>,
    /// Files below a root, referenced by path and scoped to their folder.
    pub nested: Vec<PathBuf>,
    /// Nested files beyond [`MAX_NESTED`].
    pub nested_overflow: usize,
    /// A project- or workspace-level file was inlined, so the caller should
    /// switch provider-native discovery off. The user-wide
    /// `$GAVIERO_ROOT/.gaviero/GAVIERO.md` alone never sets this.
    pub replaces_native: bool,
}

impl ProjectInstructions {
    /// Find every `GAVIERO.md` in scope for `primary` + `additional` roots.
    /// `None` when there is none — the caller leaves providers alone.
    pub fn discover(primary: &Path, additional: &[PathBuf]) -> Option<Self> {
        Self::discover_with_global(primary, additional, global_file().as_deref())
    }

    /// [`Self::discover`] with an explicit user-wide file, so tests do not
    /// depend on `$GAVIERO_ROOT`.
    fn discover_with_global(
        primary: &Path,
        additional: &[PathBuf],
        global: Option<&Path>,
    ) -> Option<Self> {
        let roots = roots(primary, additional);
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut out = Self::default();

        // Broadest first: the user-wide baseline, then each root's workspace
        // and repository layers from its outermost directory down.
        if let Some(file) = global {
            out.push_inline(file, &mut seen, false);
        }
        for root in &roots {
            for dir in project_dirs(root) {
                out.push_inline(&dir.join(INSTRUCTIONS_FILE), &mut seen, true);
            }
        }

        for root in &roots {
            for file in nested_files(root) {
                let key = canonical(&file);
                if seen.contains(&key) {
                    continue;
                }
                seen.push(key);
                if out.nested.len() < MAX_NESTED {
                    out.nested.push(file);
                } else {
                    out.nested_overflow += 1;
                }
            }
        }

        (!out.inline.is_empty() || !out.nested.is_empty()).then_some(out)
    }

    fn push_inline(&mut self, file: &Path, seen: &mut Vec<PathBuf>, project: bool) {
        if !file.is_file() {
            return;
        }
        let key = canonical(file);
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
        if project {
            self.replaces_native = true;
        }
        match std::fs::read_to_string(file) {
            Ok(body) => self.inline.push((file.to_path_buf(), body)),
            Err(e) => tracing::warn!(
                target: "instructions",
                path = %file.display(),
                error = %e,
                "unreadable GAVIERO.md skipped"
            ),
        }
    }

    /// The `<project_instructions>` block appended to the system prompt.
    pub fn render(&self) -> String {
        let mut out = String::from(
            "<project_instructions source=\"GAVIERO.md\">\n\
             These are this project's agent instructions and take precedence over \
             CLAUDE.md, AGENTS.md, .cursor/rules and similar provider files. ",
        );
        if self.replaces_native {
            out.push_str(
                "Gaviero disables those files where the provider supports it; if a \
                 provider still loads one, ignore it in favour of this file. ",
            );
        } else {
            out.push_str(
                "Provider-native files are still loaded here; where they conflict, \
                 this file wins. ",
            );
        }
        out.push_str(
            "Files below are ordered broadest first, so a later file wins where \
             they disagree.\n",
        );
        let mut budget = MAX_INLINE_BYTES;
        for (path, body) in &self.inline {
            let body = body.trim();
            if body.is_empty() {
                continue;
            }
            out.push_str(&format!("\n<instructions path=\"{}\">\n", path.display()));
            if body.len() <= budget {
                out.push_str(body);
                budget -= body.len();
            } else {
                out.push_str(truncate_at_char_boundary(body, budget));
                out.push_str(&format!(
                    "\n[truncated — read {} for the rest]",
                    path.display()
                ));
                budget = 0;
            }
            out.push_str("\n</instructions>\n");
        }
        if !self.nested.is_empty() {
            out.push_str(
                "\nFolder-scoped instruction files. Before reading or changing files \
                 under one of these folders, read its GAVIERO.md and follow it; it \
                 takes precedence over the instructions above for that folder:\n",
            );
            for path in &self.nested {
                out.push_str(&format!("- {}\n", path.display()));
            }
            if self.nested_overflow > 0 {
                out.push_str(&format!(
                    "- … and {} more GAVIERO.md files deeper in the tree\n",
                    self.nested_overflow
                ));
            }
        }
        out.push_str("</project_instructions>");
        out
    }
}

/// Whether a project- or workspace-level `GAVIERO.md` is in scope, i.e.
/// whether provider-native instruction discovery should be switched off. A
/// user-wide file, or a nested folder file on its own, does not count.
pub fn is_active(primary: &Path, additional: &[PathBuf]) -> bool {
    roots(primary, additional).iter().any(|root| {
        project_dirs(root)
            .iter()
            .any(|dir| dir.join(INSTRUCTIONS_FILE).is_file())
    })
}

/// `system_prompt` plus the `GAVIERO.md` block, or unchanged when none is in
/// scope.
pub fn append_to_system_prompt(
    system_prompt: String,
    primary: &Path,
    additional: &[PathBuf],
) -> String {
    append_with_global(system_prompt, primary, additional, global_file().as_deref())
}

fn append_with_global(
    mut system_prompt: String,
    primary: &Path,
    additional: &[PathBuf],
    global: Option<&Path>,
) -> String {
    if let Some(found) = ProjectInstructions::discover_with_global(primary, additional, global) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&found.render());
    }
    system_prompt
}

/// `--settings` JSON for Claude Code that skips its native memory files.
/// A flag-level layer: merged over project/user settings, so the synthesized
/// `.claude/settings.json` permissions still apply.
pub fn claude_settings_arg() -> String {
    serde_json::json!({ "claudeMdExcludes": CLAUDE_NATIVE_EXCLUDES }).to_string()
}

/// Add or remove gaviero's managed block in `<root>/.cursorignore` so Cursor
/// skips `AGENTS.md` / `CLAUDE.md` exactly while a project `GAVIERO.md` is in
/// scope for `root`. Lines outside the block are never touched; a file that
/// held only the block is removed. Returns the path when the file changed.
///
/// Run for workspace roots, not swarm worktrees: a worktree inherits the
/// block only if `.cursorignore` is committed.
pub fn sync_cursorignore(root: &Path) -> Result<Option<PathBuf>> {
    write_cursorignore(root, is_active(root, &[]))
}

/// Sync every workspace root with one decision, so a `GAVIERO.md` in a sibling
/// folder also switches Cursor off in the others. Returns the changed paths;
/// a failing root is logged and skipped (best-effort startup work).
pub fn sync_cursorignore_roots(roots: &[PathBuf]) -> Vec<PathBuf> {
    let Some((primary, rest)) = roots.split_first() else {
        return Vec::new();
    };
    let active = is_active(primary, rest);
    let mut changed = Vec::new();
    for root in roots {
        match write_cursorignore(root, active) {
            Ok(Some(path)) => changed.push(path),
            Ok(None) => {}
            Err(e) => tracing::warn!(
                target: "instructions",
                path = %root.join(CURSORIGNORE_FILE).display(),
                error = %e,
                "could not sync .cursorignore"
            ),
        }
    }
    changed
}

fn write_cursorignore(root: &Path, active: bool) -> Result<Option<PathBuf>> {
    let path = root.join(CURSORIGNORE_FILE);
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let next = cursorignore_with_block(existing.as_deref(), active);
    match (existing.as_deref(), next.as_deref()) {
        (old, new) if old == new => Ok(None),
        (_, None) => {
            std::fs::remove_file(&path)
                .with_context(|| format!("removing {}", path.display()))?;
            Ok(Some(path))
        }
        (_, Some(body)) => {
            std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
            Ok(Some(path))
        }
    }
}

/// `existing` with the managed block stripped, then re-added when `active`.
/// `None` means "no file".
fn cursorignore_with_block(existing: Option<&str>, active: bool) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    let mut in_block = false;
    for line in existing.unwrap_or("").lines() {
        if line.trim_end() == CURSORIGNORE_BEGIN {
            in_block = true;
        } else if in_block && line.trim_end() == CURSORIGNORE_END {
            in_block = false;
        } else if !in_block {
            kept.push(line);
        }
    }
    while kept.last().is_some_and(|l| l.trim().is_empty()) {
        kept.pop();
    }
    let mut out = kept.join("\n");
    if active {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(CURSORIGNORE_BEGIN);
        out.push('\n');
        for name in CURSOR_IGNORED {
            out.push_str(name);
            out.push('\n');
        }
        out.push_str(CURSORIGNORE_END);
    }
    if out.is_empty() {
        // Nothing of ours or the user's is left. Keep a pre-existing empty
        // file as it was rather than deleting something we did not create.
        return existing.filter(|s| s.trim().is_empty()).map(str::to_string);
    }
    out.push('\n');
    Some(out)
}

fn roots(primary: &Path, additional: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in std::iter::once(primary).chain(additional.iter().map(PathBuf::as_path)) {
        if root.as_os_str().is_empty() || out.iter().any(|r| r == root) {
            continue;
        }
        out.push(root.to_path_buf());
    }
    out
}

/// Directories whose `GAVIERO.md` is inlined for `root`, outermost first: the
/// enclosing workspace directory when there is one, then the repository root
/// down to `root`.
fn project_dirs(root: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut git_root_found = false;
    for dir in root.ancestors() {
        chain.push(dir.to_path_buf());
        if dir.join(".git").exists() {
            git_root_found = true;
            break;
        }
    }
    if !git_root_found {
        chain.truncate(1);
    }
    let top = chain.last().cloned().unwrap_or_else(|| root.to_path_buf());
    if let Some(ws_dir) = top.ancestors().find(|d| holds_workspace_file(d))
        && !chain.iter().any(|d| d == ws_dir)
    {
        chain.push(ws_dir.to_path_buf());
    }
    chain.reverse();
    chain
}

/// User-wide instruction file: `$GAVIERO_ROOT/.gaviero/GAVIERO.md`, beside the
/// user settings and skills.
fn global_file() -> Option<PathBuf> {
    crate::workspace::user_gaviero_dir().map(|dir| dir.join(INSTRUCTIONS_FILE))
}

fn holds_workspace_file(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "gaviero-workspace")
                && e.file_type().is_ok_and(|t| t.is_file())
        })
    })
}

/// `GAVIERO.md` files strictly below `root`, honouring `.gitignore`, sorted.
fn nested_files(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = ignore::WalkBuilder::new(root)
        .hidden(true)
        .parents(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .follow_links(false)
        .build()
        .flatten()
        .filter(|e| {
            e.depth() > 1
                && e.file_name() == INSTRUCTIONS_FILE
                && e.file_type().is_some_and(|t| t.is_file())
        })
        .map(ignore::DirEntry::into_path)
        .collect();
    out.sort();
    out
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn truncate_at_char_boundary(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        dir
    }

    #[test]
    fn nothing_in_scope_leaves_prompt_untouched() {
        let dir = repo();
        write(&dir.path().join("CLAUDE.md"), "claude");
        assert!(!is_active(dir.path(), &[]));
        assert_eq!(
            append_with_global("sys".into(), dir.path(), &[], None),
            "sys"
        );
    }

    #[test]
    fn root_file_is_inlined() {
        let dir = repo();
        write(&dir.path().join(INSTRUCTIONS_FILE), "Use tabs.");
        let out = append_with_global("sys".into(), dir.path(), &[], None);
        assert!(out.starts_with("sys\n\n<project_instructions"));
        assert!(out.contains("Use tabs."));
        assert!(out.ends_with("</project_instructions>"));
    }

    #[test]
    fn ancestors_stop_at_git_root_and_order_outermost_first() {
        let outer = tempfile::tempdir().unwrap();
        write(&outer.path().join(INSTRUCTIONS_FILE), "OUTSIDE");
        let repo = outer.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        write(&repo.join(INSTRUCTIONS_FILE), "REPO");
        let pkg = repo.join("pkg");
        write(&pkg.join(INSTRUCTIONS_FILE), "PKG");

        let found = ProjectInstructions::discover_with_global(&pkg, &[], None).unwrap();
        let bodies: Vec<&str> = found.inline.iter().map(|(_, b)| b.as_str()).collect();
        assert_eq!(bodies, ["REPO", "PKG"]);
    }

    #[test]
    fn workspace_file_directory_is_included() {
        let ws = tempfile::tempdir().unwrap();
        write(&ws.path().join("team.gaviero-workspace"), "{}");
        write(&ws.path().join(INSTRUCTIONS_FILE), "WORKSPACE");
        let member = ws.path().join("app");
        std::fs::create_dir_all(member.join(".git")).unwrap();

        let found = ProjectInstructions::discover_with_global(&member, &[], None).unwrap();
        assert_eq!(found.inline.len(), 1);
        assert_eq!(found.inline[0].1, "WORKSPACE");
    }

    #[test]
    fn no_git_root_only_checks_the_root_itself() {
        let outer = tempfile::tempdir().unwrap();
        write(&outer.path().join(INSTRUCTIONS_FILE), "OUTSIDE");
        let root = outer.path().join("plain");
        std::fs::create_dir_all(&root).unwrap();
        assert!(ProjectInstructions::discover_with_global(&root, &[], None).is_none());
    }

    #[test]
    fn nested_files_are_referenced_not_inlined_and_gitignore_is_honoured() {
        let dir = repo();
        write(&dir.path().join("crates/a").join(INSTRUCTIONS_FILE), "A");
        write(&dir.path().join("target/x").join(INSTRUCTIONS_FILE), "IGNORED");
        write(&dir.path().join(".gitignore"), "target/\n");

        let found = ProjectInstructions::discover_with_global(dir.path(), &[], None).unwrap();
        assert!(found.inline.is_empty());
        assert!(!found.replaces_native);
        assert_eq!(found.nested, [dir.path().join("crates/a").join(INSTRUCTIONS_FILE)]);
        // A nested file alone is folder-scoped and leaves native files on.
        assert!(!is_active(dir.path(), &[]));
        let rendered = found.render();
        assert!(rendered.contains("Folder-scoped instruction files"));
        assert!(!rendered.contains("\nA\n"));
    }

    #[test]
    fn additional_roots_are_scanned_and_deduplicated() {
        let a = repo();
        let b = repo();
        write(&b.path().join(INSTRUCTIONS_FILE), "B");
        let found = ProjectInstructions::discover_with_global(
            a.path(),
            &[b.path().to_path_buf(), b.path().to_path_buf()],
            None,
        )
        .unwrap();
        assert_eq!(found.inline.len(), 1);
        assert!(found.replaces_native);
    }

    #[test]
    fn oversized_body_is_truncated_with_pointer() {
        let dir = repo();
        write(&dir.path().join(INSTRUCTIONS_FILE), &"é".repeat(MAX_INLINE_BYTES));
        let rendered = ProjectInstructions::discover_with_global(dir.path(), &[], None)
            .unwrap()
            .render();
        assert!(rendered.contains("[truncated — read"));
        assert!(rendered.len() < MAX_INLINE_BYTES + 2048);
    }

    #[test]
    fn project_file_switches_native_discovery_off() {
        let dir = repo();
        write(&dir.path().join(INSTRUCTIONS_FILE), "x");
        let found = ProjectInstructions::discover_with_global(dir.path(), &[], None).unwrap();
        assert!(found.replaces_native);
        assert!(is_active(dir.path(), &[]));
    }

    #[test]
    fn user_wide_file_is_a_baseline_and_does_not_switch_native_off() {
        let dir = repo();
        write(&dir.path().join("CLAUDE.md"), "claude");
        let global = tempfile::tempdir().unwrap();
        let global_file = global.path().join(INSTRUCTIONS_FILE);
        write(&global_file, "GLOBAL");

        let found =
            ProjectInstructions::discover_with_global(dir.path(), &[], Some(&global_file))
                .unwrap();
        assert_eq!(found.inline.len(), 1);
        assert_eq!(found.inline[0].1, "GLOBAL");
        assert!(!found.replaces_native);
        // A user-wide file alone never hides the project's CLAUDE.md.
        assert!(!is_active(dir.path(), &[]));
    }

    #[test]
    fn user_wide_file_is_read_before_the_repository() {
        let dir = repo();
        write(&dir.path().join(INSTRUCTIONS_FILE), "REPO");
        let global = tempfile::tempdir().unwrap();
        let global_file = global.path().join(INSTRUCTIONS_FILE);
        write(&global_file, "GLOBAL");

        let found =
            ProjectInstructions::discover_with_global(dir.path(), &[], Some(&global_file))
                .unwrap();
        let bodies: Vec<&str> = found.inline.iter().map(|(_, b)| b.as_str()).collect();
        assert_eq!(bodies, ["GLOBAL", "REPO"]);
        assert!(found.replaces_native);
    }

    #[test]
    fn claude_settings_arg_lists_excludes() {
        let v: serde_json::Value = serde_json::from_str(&claude_settings_arg()).unwrap();
        let list = v["claudeMdExcludes"].as_array().unwrap();
        assert!(list.iter().any(|p| p == "**/CLAUDE.md"));
        assert!(list.iter().any(|p| p == "**/AGENTS.md"));
    }

    #[test]
    fn cursorignore_block_round_trips_and_preserves_user_lines() {
        let added = cursorignore_with_block(Some("secrets/\n"), true).unwrap();
        assert!(added.starts_with("secrets/\n\n# >>> gaviero"));
        assert!(added.contains("\nAGENTS.md\n"));
        // Idempotent.
        assert_eq!(cursorignore_with_block(Some(&added), true).unwrap(), added);
        // Removal restores the user's file.
        assert_eq!(cursorignore_with_block(Some(&added), false).unwrap(), "secrets/\n");
    }

    #[test]
    fn cursorignore_created_and_removed_with_gaviero_md() {
        let dir = repo();
        assert_eq!(sync_cursorignore(dir.path()).unwrap(), None);
        write(&dir.path().join(INSTRUCTIONS_FILE), "x");
        let path = sync_cursorignore(dir.path()).unwrap().unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("CLAUDE.md"));
        assert_eq!(sync_cursorignore(dir.path()).unwrap(), None);
        std::fs::remove_file(dir.path().join(INSTRUCTIONS_FILE)).unwrap();
        assert!(sync_cursorignore(dir.path()).unwrap().is_some());
        assert!(!path.exists());
    }

    #[test]
    fn cursorignore_roots_share_one_decision() {
        let a = repo();
        let b = repo();
        let roots = [a.path().to_path_buf(), b.path().to_path_buf()];
        assert!(sync_cursorignore_roots(&roots).is_empty());

        // A GAVIERO.md in a sibling root switches Cursor off in both.
        write(&b.path().join(INSTRUCTIONS_FILE), "B");
        assert_eq!(sync_cursorignore_roots(&roots).len(), 2);
        assert!(a.path().join(CURSORIGNORE_FILE).exists());
        assert!(b.path().join(CURSORIGNORE_FILE).exists());

        // Removing it takes both blocks out again.
        std::fs::remove_file(b.path().join(INSTRUCTIONS_FILE)).unwrap();
        assert_eq!(sync_cursorignore_roots(&roots).len(), 2);
        assert!(!a.path().join(CURSORIGNORE_FILE).exists());
        assert!(!b.path().join(CURSORIGNORE_FILE).exists());
    }
}
