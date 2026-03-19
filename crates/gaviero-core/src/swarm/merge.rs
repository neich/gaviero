//! Merge resolver for agent branches.
//!
//! After agents complete their work in separate worktrees/branches,
//! the merge resolver integrates each branch into the main branch.
//! Conflicts are resolved by spawning a Claude session with the
//! conflict markers and interface contracts.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};

use super::models::{MergeConflict, MergeResult};
use crate::acp::protocol::StreamEvent;
use crate::acp::session::{AcpSession, AgentOptions};

/// Merge an agent branch into the current branch.
///
/// Returns a `MergeResult` describing success/failure and any conflicts.
pub fn merge_branch(repo_dir: &Path, branch: &str) -> Result<MergeResult> {
    let work_unit_id = branch
        .strip_prefix("gaviero/")
        .unwrap_or(branch)
        .to_string();

    let output = Command::new("git")
        .args(["merge", "--no-ff", branch, "-m"])
        .arg(format!("Merge {}", branch))
        .current_dir(repo_dir)
        .output()
        .with_context(|| format!("running git merge {}", branch))?;

    if output.status.success() {
        return Ok(MergeResult {
            work_unit_id,
            success: true,
            conflicts: Vec::new(),
        });
    }

    // Check for merge conflicts
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    if stderr.contains("CONFLICT") || stdout.contains("CONFLICT") {
        let conflicts = collect_conflict_files(repo_dir)?;
        Ok(MergeResult {
            work_unit_id,
            success: false,
            conflicts,
        })
    } else {
        // Non-conflict failure
        anyhow::bail!("git merge {} failed: {}", branch, stderr.trim());
    }
}

/// Collect the list of files with merge conflicts.
fn collect_conflict_files(repo_dir: &Path) -> Result<Vec<MergeConflict>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", "--diff-filter=U"])
        .current_dir(repo_dir)
        .output()
        .context("running git diff --name-only")?;

    let files = String::from_utf8_lossy(&output.stdout);
    let conflicts = files
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|f| MergeConflict {
            file: f.trim().into(),
            resolved: false,
            resolution_method: None,
        })
        .collect();

    Ok(conflicts)
}

/// Abort an in-progress merge.
pub fn abort_merge(repo_dir: &Path) -> Result<()> {
    Command::new("git")
        .args(["merge", "--abort"])
        .current_dir(repo_dir)
        .status()
        .context("running git merge --abort")?;
    Ok(())
}

/// Read the conflict markers from a file for Claude to resolve.
pub fn read_conflict_content(repo_dir: &Path, file: &Path) -> Result<String> {
    let abs_path = repo_dir.join(file);
    std::fs::read_to_string(&abs_path)
        .with_context(|| format!("reading conflict file {}", abs_path.display()))
}

/// Write the resolved content and stage the file.
pub fn resolve_conflict(repo_dir: &Path, file: &Path, resolved_content: &str) -> Result<()> {
    let abs_path = repo_dir.join(file);
    std::fs::write(&abs_path, resolved_content)
        .with_context(|| format!("writing resolved file {}", abs_path.display()))?;

    Command::new("git")
        .args(["add"])
        .arg(file)
        .current_dir(repo_dir)
        .status()
        .context("staging resolved file")?;
    Ok(())
}

/// Complete a merge after all conflicts have been resolved (staged).
fn complete_merge(repo_dir: &Path, branch: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["commit", "--no-edit"])
        .current_dir(repo_dir)
        .output()
        .with_context(|| format!("completing merge for {}", branch))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git commit after conflict resolution failed: {}", stderr.trim());
    }
    Ok(())
}

/// Auto-resolve all conflicts in a merge using Claude.
///
/// For each conflicted file, reads the conflict markers, asks Claude to
/// produce a clean resolved version, writes it back, and stages it.
/// After all files are resolved, completes the merge commit.
pub async fn auto_resolve_conflicts(
    repo_dir: &Path,
    branch: &str,
    conflicts: &[MergeConflict],
    model: &str,
) -> Result<Vec<MergeConflict>> {
    let mut resolved_conflicts = Vec::new();

    for conflict in conflicts {
        let content = read_conflict_content(repo_dir, &conflict.file)?;

        let prompt = format!(
            "The file `{}` has git merge conflicts. Below is the file with conflict markers.\n\
             Resolve ALL conflicts by producing the complete file with NO conflict markers.\n\
             Keep the best parts from both sides. Output ONLY the resolved file content, \n\
             nothing else — no markdown fences, no explanation.\n\n{}",
            conflict.file.display(),
            content,
        );

        match resolve_single_file(repo_dir, &conflict.file, &prompt, model).await {
            Ok(resolved_content) => {
                resolve_conflict(repo_dir, &conflict.file, &resolved_content)?;
                resolved_conflicts.push(MergeConflict {
                    file: conflict.file.clone(),
                    resolved: true,
                    resolution_method: Some("claude".to_string()),
                });
            }
            Err(e) => {
                tracing::warn!("failed to auto-resolve {}: {}", conflict.file.display(), e);
                resolved_conflicts.push(MergeConflict {
                    file: conflict.file.clone(),
                    resolved: false,
                    resolution_method: Some(format!("failed: {}", e)),
                });
            }
        }
    }

    let all_resolved = resolved_conflicts.iter().all(|c| c.resolved);
    if all_resolved {
        complete_merge(repo_dir, branch)?;
    }

    Ok(resolved_conflicts)
}

/// Resolve a single conflicted file by asking Claude.
async fn resolve_single_file(
    repo_dir: &Path,
    _file: &Path,
    prompt: &str,
    model: &str,
) -> Result<String> {
    let mut session = AcpSession::spawn(
        model,
        repo_dir,
        prompt,
        "You are a merge conflict resolver. Output only the resolved file content.",
        &[],  // no tools needed
        &AgentOptions::default(),
        &[],  // no file attachments
    )?;

    let mut result_text = String::new();

    loop {
        match session.next_event().await? {
            Some(StreamEvent::ContentDelta(text)) => {
                result_text.push_str(&text);
            }
            Some(StreamEvent::ResultEvent { result_text: text, .. }) => {
                if !text.is_empty() {
                    result_text = text;
                }
                break;
            }
            None => break,
            _ => {}
        }
    }

    if result_text.trim().is_empty() {
        anyhow::bail!("Claude returned empty resolution");
    }

    // Strip markdown fences if Claude wraps the output
    let cleaned = result_text.trim();
    let cleaned = if cleaned.starts_with("```") {
        let after_first_line = cleaned.find('\n').map(|i| i + 1).unwrap_or(0);
        let before_last_fence = cleaned.rfind("```").unwrap_or(cleaned.len());
        &cleaned[after_first_line..before_last_fence]
    } else {
        cleaned
    };

    Ok(cleaned.to_string())
}
