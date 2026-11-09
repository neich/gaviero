//! Preparing a DSL script run: what every front end must do before
//! compiling, so a script behaves the same from `gaviero-cli` and the TUI.
//!
//! Anchors `execution document` workflows to their `PLAN_FILE`, defaults and
//! re-anchors `OUT_DIR`, brings files named by vars within reach of agent
//! worktrees, and derives the remote MCP servers scripts declare through
//! vars. Progress is returned as notes rather than printed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::plan::ExecutionMode;

/// Where a script runs and the vars it runs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRunAnchor {
    pub root: PathBuf,
    pub vars: Vec<(String, String)>,
    /// One line per decision worth showing the user.
    pub notes: Vec<String>,
}

fn var<'a>(vars: &'a [(String, String)], key: &str) -> Option<&'a str> {
    vars.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn set_var(vars: &mut Vec<(String, String)>, key: &str, value: String) {
    if let Some((_, v)) = vars.iter_mut().find(|(k, _)| k == key) {
        *v = value;
    } else {
        vars.push((key.to_string(), value));
    }
}

/// Decide the run root and final vars.
///
/// `repo` is the root the caller asked for and `repo_is_default` says it was
/// not chosen explicitly, which is what lets a `PLAN_FILE` move the root of a
/// document workflow. `workspace` is an explicit document root.
pub fn anchor_script_run(
    cwd: &Path,
    repo: &Path,
    workspace: Option<&Path>,
    repo_is_default: bool,
    execution_mode: ExecutionMode,
    mut vars: Vec<(String, String)>,
) -> Result<ScriptRunAnchor> {
    let mut notes = Vec::new();
    let mut repo_path = workspace.unwrap_or(repo).to_path_buf();

    if execution_mode == ExecutionMode::Document {
        if let Some(ws) = workspace {
            repo_path = ws.to_path_buf();
            notes.push(format!(
                "[execution] document workspace: {}",
                repo_path.display()
            ));
        } else if let Some(plan_value) = var(&vars, "PLAN_FILE").map(str::to_string) {
            let Some(plan_host) = resolve_host_path_early(cwd, &repo_path, &plan_value) else {
                bail!("--var PLAN_FILE={plan_value}: file not found (pass an existing path)");
            };
            let plan_dir = plan_host
                .parent()
                .ok_or_else(|| anyhow::anyhow!("--var PLAN_FILE has no parent directory"))?;
            let plan_dir = std::fs::canonicalize(plan_dir)
                .with_context(|| format!("resolving plan directory {}", plan_dir.display()))?;
            if repo_is_default {
                repo_path = plan_dir.clone();
                let plan_rel = path_spec_for_worktree(&plan_dir, &plan_host);
                set_var(&mut vars, "PLAN_FILE", plan_rel);
                notes.push(format!(
                    "[execution] document workspace: {} (PLAN_FILE={})",
                    plan_dir.display(),
                    var(&vars, "PLAN_FILE").unwrap_or("?")
                ));
                // After the anchor line, so the rewrite reads as a consequence
                // of it rather than as an unexplained edit to the caller's var.
                if let Some(note) = reanchor_out_dir_to_workspace(&mut vars, cwd, &plan_dir)? {
                    notes.push(note);
                }
            }
            apply_out_dir_default_from_plan(&mut vars, &repo_path, &plan_host);
        } else if repo_is_default {
            // Document workflows that only need a shared write root (e.g.
            // plan_refinement with OUT_DIR under a gitignored `plans/`) can
            // run against the current checkout without PLAN_FILE.
            repo_path = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
            notes.push(format!(
                "[execution] document workspace: {} (cwd; pass --workspace to override)",
                repo_path.display()
            ));
        } else {
            bail!("execution document requires --workspace <dir> or --var PLAN_FILE=<path>");
        }
    } else if let Some(plan_value) = var(&vars, "PLAN_FILE").map(str::to_string)
        && let Some(plan_host) = resolve_host_path_early(cwd, &repo_path, &plan_value)
    {
        let repo_canon = std::fs::canonicalize(&repo_path).unwrap_or_else(|_| repo_path.clone());
        apply_out_dir_default_from_plan(&mut vars, &repo_canon, &plan_host);
    }

    if execution_mode == ExecutionMode::Document
        && let Some(out) = var(&vars, "OUT_DIR")
    {
        notes.push(format!(
            "[execution] OUT_DIR={out} (versioned artefacts; not for commit)"
        ));
    }

    Ok(ScriptRunAnchor {
        root: repo_path,
        vars,
        notes,
    })
}

/// Resolve a path from a var before the workspace root is finalised.
fn resolve_host_path_early(cwd: &Path, repo: &Path, value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        return None;
    }
    let host = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        let under_repo = repo.join(trimmed);
        if under_repo.is_file() {
            under_repo
        } else {
            cwd.join(trimmed)
        }
    };
    host.is_file().then_some(host)
}

/// Re-anchor an explicitly supplied `OUT_DIR` after `PLAN_FILE` moved the
/// document workspace root. Returns a note when the var was rewritten.
///
/// `--var OUT_DIR=plans/out` written for a repo-root invocation stays
/// repo-root-relative once the workspace anchors to the plan's own folder, so
/// it resolves one level too deep (`<plan_dir>/plans/out`). Agents that write
/// through their own tools probe the filesystem and silently self-correct;
/// agents that write through the in-band `<file>` channel do not — the host
/// joins their path onto the workspace root and the artefact lands in a
/// directory nobody reads. Rewrite the var when the value clearly named a
/// path inside the new workspace, and reject one that names a path outside it,
/// which scope enforcement cannot honour.
pub fn reanchor_out_dir_to_workspace(
    vars: &mut Vec<(String, String)>,
    cwd: &Path,
    workspace: &Path,
) -> Result<Option<String>> {
    let Some(out) = var(vars, "OUT_DIR").map(str::to_string) else {
        return Ok(None);
    };
    let trimmed = out.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let as_given = Path::new(trimmed);
    let cwd_canon = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let from_cwd = if as_given.is_absolute() {
        as_given.to_path_buf()
    } else {
        cwd_canon.join(as_given)
    };

    // Interpreting the value against the new root already lands where the
    // caller meant it to — the common case of running from inside the plan's
    // own directory.
    if !as_given.is_absolute() && workspace.join(as_given) == from_cwd {
        return Ok(None);
    }

    match from_cwd.strip_prefix(workspace) {
        Ok(_) => {
            let rel = path_spec_for_worktree(workspace, &from_cwd);
            set_var(vars, "OUT_DIR", rel.clone());
            Ok(Some(format!(
                "[execution] OUT_DIR re-anchored to workspace: {out} -> {rel}"
            )))
        }
        // A value like `research` from an unrelated cwd was always meant to be
        // workspace-relative; leave it alone.
        Err(_) if !as_given.is_absolute() => Ok(None),
        Err(_) => bail!(
            "--var OUT_DIR={out} points outside the document workspace {} \
             (anchored to the PLAN_FILE directory). Agent scopes are enforced \
             on workspace-relative paths, so artefacts written there would \
             escape the scope check. Pass a path inside the workspace, or use \
             --workspace <dir> to anchor the run somewhere that contains both.",
            workspace.display()
        ),
    }
}

fn apply_out_dir_default_from_plan(
    vars: &mut Vec<(String, String)>,
    repo: &Path,
    plan_host: &Path,
) {
    if vars.iter().any(|(k, _)| k == "OUT_DIR") {
        return;
    }
    let Some(plan_dir) = plan_host.parent() else {
        return;
    };
    let out = path_spec_for_worktree(repo, plan_dir);
    set_var(vars, "OUT_DIR", out);
}

/// Resolve a var value to an existing file under or outside the run root.
fn resolve_var_file_path(repo: &Path, value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.ends_with('/') {
        return None;
    }
    let host = if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        repo.join(trimmed)
    };
    host.is_file().then_some(host)
}

fn path_spec_for_worktree(repo: &Path, host: &Path) -> String {
    let normalize_rel = |rel: &Path| {
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() { ".".to_string() } else { s }
    };
    if let Ok(rel) = host.strip_prefix(repo) {
        return normalize_rel(rel);
    }
    let ws = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let host_canon = std::fs::canonicalize(host).unwrap_or_else(|_| host.to_path_buf());
    if let Ok(rel) = host_canon.strip_prefix(&ws) {
        normalize_rel(rel)
    } else {
        host.to_string_lossy().to_string()
    }
}

/// Files named by vars or a prompt file, which may need copying into
/// worktrees when they are missing from `HEAD`.
pub fn worktree_context_paths(
    repo: &Path,
    cwd: &Path,
    override_vars: &[(String, String)],
    prompt_file: Option<&Path>,
) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    if let Some(p) = prompt_file {
        let host = if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        };
        if host.is_file() {
            let spec = path_spec_for_worktree(repo, &host);
            if seen.insert(spec.clone()) {
                paths.push(spec);
            }
        }
    }

    for (_key, value) in override_vars {
        if let Some(host) = resolve_var_file_path(repo, value) {
            let spec = path_spec_for_worktree(repo, &host);
            if seen.insert(spec.clone()) {
                paths.push(spec);
            }
        }
    }

    paths
}

/// When a var points at a file outside the run root, copy it under
/// `.gaviero/injected/` and rewrite the var so agents and worktrees can read
/// it. Returns one note per copied file.
pub fn materialize_external_vars(
    repo: &Path,
    vars: &mut [(String, String)],
) -> Result<Vec<String>> {
    let injected_dir = repo.join(".gaviero/injected");
    let ws = std::fs::canonicalize(repo).unwrap_or_else(|_| repo.to_path_buf());
    let mut notes = Vec::new();

    for (key, value) in vars.iter_mut() {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(host) = resolve_var_file_path(repo, trimmed) else {
            continue;
        };
        let host_canon = std::fs::canonicalize(&host).unwrap_or(host.clone());
        if host_canon.starts_with(&ws) {
            continue;
        }
        std::fs::create_dir_all(&injected_dir)
            .with_context(|| format!("creating {}", injected_dir.display()))?;
        let dest = injected_dir.join(
            host.file_name()
                .ok_or_else(|| anyhow::anyhow!("path has no filename: {}", host.display()))?,
        );
        std::fs::copy(&host, &dest)
            .with_context(|| format!("copying external file into {}", dest.display()))?;
        let rel = dest
            .strip_prefix(repo)
            .with_context(|| "injected file must stay under the run root")?
            .to_string_lossy()
            .replace('\\', "/");
        notes.push(format!(
            "[vars] {key}: file outside the run root copied to {rel} (was {})",
            host.display()
        ));
        *value = rel;
    }
    Ok(notes)
}

/// Normalize remote MCP URLs (some hosts 308-redirect unless the path ends with `/`).
pub fn normalize_remote_mcp_url(url: &str) -> String {
    let url = url.trim();
    if url.starts_with("https://mcp.wuilder.com/") && !url.ends_with('/') {
        format!("{url}/")
    } else {
        url.to_string()
    }
}

/// `CONSENSUS_MCP_URL` / `CONSENSUS_SERVER` script vars as `(name, url)` MCP servers.
pub fn mcp_urls_from_script_vars(vars: &[(String, String)]) -> Vec<(String, String)> {
    let Some(url) = var(vars, "CONSENSUS_MCP_URL").filter(|v| !v.trim().is_empty()) else {
        return Vec::new();
    };
    let name = var(vars, "CONSENSUS_SERVER")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "consensus".to_string());
    vec![(name, normalize_remote_mcp_url(url))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn repo_workflow_stays_on_the_requested_root() {
        let dir = tempfile::tempdir().unwrap();
        let anchor = anchor_script_run(
            dir.path(),
            dir.path(),
            None,
            true,
            ExecutionMode::Repo,
            vars(&[("X", "1")]),
        )
        .unwrap();
        assert_eq!(anchor.root, dir.path());
        assert_eq!(anchor.vars, vars(&[("X", "1")]));
    }

    #[test]
    fn document_workflow_anchors_to_the_plan_directory() {
        let dir = tempfile::tempdir().unwrap();
        let plans = dir.path().join("plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(plans.join("draft.md"), "x").unwrap();

        let anchor = anchor_script_run(
            dir.path(),
            dir.path(),
            None,
            true,
            ExecutionMode::Document,
            vars(&[("PLAN_FILE", "plans/draft.md")]),
        )
        .unwrap();

        assert_eq!(anchor.root, plans.canonicalize().unwrap());
        assert_eq!(var(&anchor.vars, "PLAN_FILE"), Some("draft.md"));
        assert_eq!(var(&anchor.vars, "OUT_DIR"), Some("."));
        assert!(!anchor.notes.is_empty());
    }

    #[test]
    fn missing_plan_file_is_an_error_for_a_document_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let err = anchor_script_run(
            dir.path(),
            dir.path(),
            None,
            true,
            ExecutionMode::Document,
            vars(&[("PLAN_FILE", "nope.md")]),
        )
        .unwrap_err();
        assert!(err.to_string().contains("file not found"));
    }

    #[test]
    fn external_var_file_is_copied_under_the_run_root() {
        let repo = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("input.md");
        std::fs::write(&file, "x").unwrap();

        let mut v = vars(&[("INPUT", &file.to_string_lossy())]);
        let notes = materialize_external_vars(repo.path(), &mut v).unwrap();

        assert_eq!(v[0].1, ".gaviero/injected/input.md");
        assert!(repo.path().join(".gaviero/injected/input.md").is_file());
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn consensus_vars_become_an_mcp_server() {
        assert_eq!(
            mcp_urls_from_script_vars(&vars(&[("CONSENSUS_MCP_URL", "https://x/y")])),
            vec![("consensus".to_string(), "https://x/y".to_string())]
        );
        assert!(mcp_urls_from_script_vars(&vars(&[("OTHER", "1")])).is_empty());
    }
}
