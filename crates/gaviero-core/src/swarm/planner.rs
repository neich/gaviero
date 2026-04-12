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
    let tools = &["Read", "Glob", "Grep"];
    let mut session = AcpSession::spawn(
        model,
        workspace_root,
        user_prompt,
        system_prompt,
        tools,
        tools,
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

/// Extract a JSON string from a response that may contain markdown fences.
///
/// Looks for JSON arrays (starting with `[`) or objects (starting with `{`)
/// either directly or inside markdown code fences.
pub(crate) fn extract_json(response: &str) -> Result<String> {
    let trimmed = response.trim();

    // Try parsing directly (array or object)
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return Ok(repair_truncated_json(trimmed));
    }

    // Look for ```json ... ``` block
    if let Some(start) = trimmed.find("```json") {
        let after = &trimmed[start + 7..];
        if let Some(end) = after.find("```") {
            return Ok(repair_truncated_json(after[..end].trim()));
        }
        // No closing ``` — response was truncated. Use everything after ```json
        let inner = after.trim();
        if inner.starts_with('[') || inner.starts_with('{') {
            return Ok(repair_truncated_json(inner));
        }
    }

    // Look for ``` ... ``` block
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        if let Some(end) = after.find("```") {
            let inner = after[..end].trim();
            if inner.starts_with('[') || inner.starts_with('{') {
                return Ok(repair_truncated_json(inner));
            }
        }
        // No closing ``` — truncated
        let inner = after.trim();
        if inner.starts_with('[') || inner.starts_with('{') {
            return Ok(repair_truncated_json(inner));
        }
    }

    // Last resort: find the first { or [ in the response
    if let Some(pos) = trimmed.find('{') {
        return Ok(repair_truncated_json(&trimmed[pos..]));
    }
    if let Some(pos) = trimmed.find('[') {
        return Ok(repair_truncated_json(&trimmed[pos..]));
    }

    anyhow::bail!("could not extract JSON from response")
}

/// Attempt to repair truncated JSON by closing unclosed brackets and braces.
///
/// LLM responses frequently get cut off mid-JSON when the output token limit
/// is reached. This function iteratively tries to make the JSON parseable by:
/// 1. Closing unclosed strings
/// 2. Removing trailing incomplete key-value pairs
/// 3. Closing unclosed brackets/braces
fn repair_truncated_json(json: &str) -> String {
    // If it already parses, return as-is
    if serde_json::from_str::<serde_json::Value>(json).is_ok() {
        return json.to_string();
    }

    // Strategy: scan to find open/close state, then fix up the tail
    let mut in_string = false;
    let mut escape_next = false;
    let mut stack: Vec<char> = Vec::new();
    let bytes = json.as_bytes();

    for &b in bytes {
        let ch = b as char;
        if escape_next { escape_next = false; continue; }
        if ch == '\\' && in_string { escape_next = true; continue; }
        if ch == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match ch {
            '{' => stack.push('{'),
            '[' => stack.push('['),
            '}' => { stack.pop(); }
            ']' => { stack.pop(); }
            _ => {}
        }
    }

    // If no unclosed delimiters, it's complete (maybe a value issue)
    if stack.is_empty() && !in_string {
        return json.to_string();
    }

    let mut result = json.to_string();

    // If we ended inside a string, close it
    if in_string {
        result.push('"');
    }

    // Iteratively trim the tail and try to close delimiters until it parses
    // This handles cases like: {"key": "trunc  →  {"key": "trunc"}
    // and: {"a": [{"id": "x", "val": "tru  →  {"a": [{"id": "x", "val": "tru"}]}
    for _ in 0..5 {
        let candidate = close_json(&result);
        if serde_json::from_str::<serde_json::Value>(&candidate).is_ok() {
            tracing::info!("Repaired truncated JSON ({} → {} bytes)", json.len(), candidate.len());
            return candidate;
        }

        // Trim back: remove from the last comma or colon to try a smaller valid subset
        let trimmed = result.trim_end();
        if let Some(pos) = trimmed.rfind(|c: char| c == ',' || c == ':') {
            result = trimmed[..pos].to_string();
        } else {
            break;
        }
    }

    // Final attempt: just close everything on the current state
    let candidate = close_json(&result);
    if serde_json::from_str::<serde_json::Value>(&candidate).is_ok() {
        return candidate;
    }

    // Give up — return original (will fail in caller with clear error)
    tracing::warn!("Could not repair truncated JSON ({} bytes)", json.len());
    json.to_string()
}

/// Close all unclosed strings, brackets, and braces in a JSON fragment.
fn close_json(json: &str) -> String {
    let mut result = json.to_string();

    // Scan for state
    let mut in_string = false;
    let mut escape_next = false;
    let mut stack: Vec<char> = Vec::new();

    for ch in result.chars() {
        if escape_next { escape_next = false; continue; }
        if ch == '\\' && in_string { escape_next = true; continue; }
        if ch == '"' { in_string = !in_string; continue; }
        if in_string { continue; }
        match ch {
            '{' => stack.push('{'),
            '[' => stack.push('['),
            '}' => { stack.pop(); }
            ']' => { stack.pop(); }
            _ => {}
        }
    }

    if in_string {
        result.push('"');
    }

    // Remove trailing comma after closing string
    let trimmed = result.trim_end();
    if trimmed.ends_with(',') {
        result = trimmed[..trimmed.len() - 1].to_string();
    }

    // Close in reverse order
    for &opener in stack.iter().rev() {
        match opener {
            '{' => result.push('}'),
            '[' => result.push(']'),
            _ => {}
        }
    }

    result
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

    // ── JSON repair tests ───────────────────────────────────────

    #[test]
    fn test_repair_valid_json_unchanged() {
        let input = r#"{"a": 1, "b": [2, 3]}"#;
        assert_eq!(repair_truncated_json(input), input);
    }

    #[test]
    fn test_repair_missing_closing_brace() {
        let input = r#"{"a": 1, "b": 2"#;
        let repaired = repair_truncated_json(input);
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok());
    }

    #[test]
    fn test_repair_truncated_array() {
        let input = r#"{"units": [{"id": "a"}, {"id": "b""#;
        let repaired = repair_truncated_json(input);
        let v: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        assert!(v.get("units").unwrap().as_array().unwrap().len() >= 1);
    }

    #[test]
    fn test_repair_truncated_string() {
        let input = r#"{"plan": "Some long desc"#;
        let repaired = repair_truncated_json(input);
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok());
    }

    #[test]
    fn test_repair_trailing_comma() {
        let input = r#"{"a": 1,"#;
        let repaired = repair_truncated_json(input);
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok());
    }

    #[test]
    fn test_repair_nested_truncation() {
        let input = r#"{"units": [{"id": "a", "scope": {"owned_paths": ["src/"#;
        let repaired = repair_truncated_json(input);
        assert!(serde_json::from_str::<serde_json::Value>(&repaired).is_ok());
    }

    #[test]
    fn test_extract_json_truncated_fenced() {
        // Truncated ```json block without closing ```
        let input = "Here's the plan:\n```json\n{\"id\": \"a\", \"data\": [1, 2";
        let result = extract_json(input).unwrap();
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_ok());
    }

    #[test]
    fn test_repair_realistic_truncation() {
        // Simulates a coordinator response cut mid-unit
        let input = r#"{
            "plan_summary": "Migrate to Rust",
            "units": [
                {"id": "U0", "description": "Setup", "tier": "mechanical"},
                {"id": "U1", "description": "Models", "tier": "reasoning", "depends_on": ["U0"]},
                {"id": "U2", "description": "Routes", "coordinator_instructions": "Implement the API rou"#;
        let repaired = repair_truncated_json(input);
        let v: serde_json::Value = serde_json::from_str(&repaired).unwrap();
        let units = v.get("units").unwrap().as_array().unwrap();
        // Should have at least the first 2 complete units
        assert!(units.len() >= 2);
    }
}
