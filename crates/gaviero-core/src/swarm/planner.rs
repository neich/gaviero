//! Task planner: uses Claude to decompose a high-level task into WorkUnits.
//!
//! Spawns a Claude session with the task description, workspace file list,
//! and memory context. Parses the response as JSON `Vec<WorkUnit>`.
//! Validates scopes and dependencies, retrying on failure.

use std::path::Path;

use anyhow::{Context, Result};

use super::models::WorkUnit;
use super::validation;
use crate::acp::session::AcpSession;

/// Plan a task by asking Claude to decompose it into work units.
///
/// Returns a validated list of `WorkUnit`s with non-overlapping scopes
/// and a valid dependency graph.
pub async fn plan_task(
    task: &str,
    workspace_root: &Path,
    model: &str,
    file_list: &[String],
    memory_context: &str,
) -> Result<Vec<WorkUnit>> {
    let system_prompt = build_planner_prompt(file_list, memory_context);
    let user_prompt = format!(
        "Decompose this task into work units. \
         Each work unit should have non-overlapping file scopes.\n\n\
         Task: {}\n\n\
         Respond with a JSON array of WorkUnit objects. Each has:\n\
         - id: unique string\n\
         - description: what to do\n\
         - scope: {{ owned_paths: [...], read_only_paths: [...], interface_contracts: {{}} }}\n\
         - depends_on: [ids of work units that must finish first]\n\
         - model: optional model override\n\n\
         Respond ONLY with the JSON array, no other text.",
        task
    );

    const MAX_ATTEMPTS: usize = 2;
    for attempt in 0..MAX_ATTEMPTS {
        match try_plan(&system_prompt, &user_prompt, workspace_root, model).await {
            Ok(units) => return Ok(units),
            Err(e) => {
                if attempt == 0 {
                    tracing::warn!("Planning attempt 1 failed: {}. Retrying...", e);
                } else {
                    return Err(e).context("planning failed after 2 attempts");
                }
            }
        }
    }

    unreachable!()
}

async fn try_plan(
    system_prompt: &str,
    user_prompt: &str,
    workspace_root: &Path,
    model: &str,
) -> Result<Vec<WorkUnit>> {
    let options = crate::acp::session::AgentOptions::default();
    let mut session = AcpSession::spawn(
        model,
        workspace_root,
        user_prompt,
        system_prompt,
        &["Read", "Glob", "Grep"],
        &options,
        &[],  // no file attachments
    )?;

    // Collect the full response
    let mut response = String::new();
    loop {
        match session.next_event().await {
            Ok(Some(crate::acp::protocol::StreamEvent::ContentDelta(text))) => {
                response.push_str(&text);
            }
            Ok(Some(crate::acp::protocol::StreamEvent::ResultEvent { result_text, .. })) => {
                if response.is_empty() {
                    response = result_text;
                }
                break;
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("Stream error during planning: {}", e);
                break;
            }
            _ => {}
        }
    }
    let _ = session.wait().await;

    // Extract JSON from the response (may be wrapped in ```json ... ```)
    let json_str = extract_json(&response)?;
    let units: Vec<WorkUnit> = serde_json::from_str(&json_str)
        .with_context(|| format!("parsing work units JSON: {}", &json_str[..json_str.len().min(200)]))?;

    // Validate
    let scope_errors = validation::validate_scopes(&units);
    if !scope_errors.is_empty() {
        anyhow::bail!(
            "scope overlaps: {}",
            scope_errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")
        );
    }

    let _tiers = validation::dependency_tiers(&units)
        .map_err(|e| anyhow::anyhow!("dependency error: {}", e))?;

    Ok(units)
}

fn build_planner_prompt(file_list: &[String], memory_context: &str) -> String {
    let mut prompt = String::from(
        "You are a software architecture planner. \
         Your job is to decompose a development task into independent work units \
         that can be executed by separate AI agents in parallel.\n\n\
         Rules:\n\
         - Each work unit must have non-overlapping owned_paths (no two agents write to the same file/directory)\n\
         - Use depends_on to express ordering when one unit needs another's output\n\
         - Keep units as independent as possible to maximize parallelism\n\
         - Use directory-level scopes (ending with /) when an agent needs a whole subtree\n\n"
    );

    if !file_list.is_empty() {
        prompt.push_str("Workspace files:\n");
        for f in file_list.iter().take(200) {
            prompt.push_str(&format!("  {}\n", f));
        }
        prompt.push('\n');
    }

    if !memory_context.is_empty() {
        prompt.push_str("Relevant context from memory:\n");
        prompt.push_str(memory_context);
        prompt.push('\n');
    }

    prompt
}

/// Extract a JSON array from a response that may contain markdown fences.
fn extract_json(response: &str) -> Result<String> {
    let trimmed = response.trim();

    // Try parsing directly
    if trimmed.starts_with('[') {
        return Ok(trimmed.to_string());
    }

    // Look for ```json ... ``` block
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return Ok(after[..end].trim().to_string());
        }
    }

    // Look for ``` ... ``` block
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('[') {
                return Ok(inner.to_string());
            }
        }
    }

    anyhow::bail!("could not extract JSON array from response")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_direct() {
        let input = r#"[{"id": "a"}]"#;
        assert_eq!(extract_json(input).unwrap(), input);
    }

    #[test]
    fn test_extract_json_fenced() {
        let input = "Here's the plan:\n```json\n[{\"id\": \"a\"}]\n```\nDone.";
        assert_eq!(extract_json(input).unwrap(), "[{\"id\": \"a\"}]");
    }

    #[test]
    fn test_extract_json_no_json() {
        let input = "No JSON here.";
        assert!(extract_json(input).is_err());
    }
}
