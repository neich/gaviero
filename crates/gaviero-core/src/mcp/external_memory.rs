//! External MCP memory-server detection + migration (Tier A / A5).
//!
//! Scans the user's and workspace's MCP config files for entries that
//! name external memory servers Gaviero now supersedes. Known
//! offenders (plan §A5):
//!
//! * `@modelcontextprotocol/server-memory`
//! * `mem0-mcp`
//! * `memory-bank-mcp`
//!
//! On detection, the TUI posts a banner offering migration. The import
//! path (below) reads the server's JSONL dump and inserts each entry
//! as `MemorySource::McpImport` at `Workspace` scope with `trust_score
//! = 0.5` — Phase 1 A3's schema is the target.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::memory::writer::WriterHandle;
use crate::memory::{MemorySource, WriteMeta, WriteScope, WriterMessage};

/// Detected external memory server entry. `source_tag` is a stable
/// non-PII label used for UI messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalMemoryServer {
    pub name: String,
    pub source_tag: &'static str,
    pub config_path: PathBuf,
}

/// Candidate config-file locations for MCP detection. Checked in
/// order; all present files are scanned. Caller passes the workspace
/// root; we also probe the home directory.
pub fn candidate_config_paths(workspace: &Path) -> Vec<PathBuf> {
    let mut paths = vec![
        workspace.join(".mcp.json"),
        workspace.join(".codex/config.toml"),
    ];
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".claude/claude.json"));
        paths.push(home.join(".claude.json"));
        paths.push(home.join(".codex/config.toml"));
    }
    paths
}

/// Scan the given config paths for known external memory servers.
/// Returns all hits; empty vec means the user is already clean.
pub fn detect_external_memory_servers(paths: &[PathBuf]) -> Vec<ExternalMemoryServer> {
    let mut hits = Vec::new();
    for path in paths {
        if !path.exists() {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(path) else {
            continue;
        };
        for (needle, tag) in KNOWN_SERVERS {
            if body.contains(needle) {
                let name = infer_server_name(&body, needle);
                hits.push(ExternalMemoryServer {
                    name,
                    source_tag: tag,
                    config_path: path.clone(),
                });
            }
        }
    }
    hits
}

/// Best-effort disable for known external memory MCP servers.
///
/// The config remains syntactically valid: known command/package strings are
/// replaced with inert placeholders, and a sibling `.gaviero.bak` backup is
/// written before the first edit.
pub fn disable_external_memory_servers(paths: &[PathBuf]) -> Result<Vec<ExternalMemoryServer>> {
    let hits = detect_external_memory_servers(paths);
    let mut touched: Vec<PathBuf> = Vec::new();

    for path in hits.iter().map(|h| h.config_path.clone()) {
        if touched.iter().any(|p| p == &path) {
            continue;
        }
        touched.push(path.clone());
        let body = match std::fs::read_to_string(&path) {
            Ok(body) => body,
            Err(e) => {
                tracing::warn!(
                    target: "mcp_external",
                    path = %path.display(),
                    error = %e,
                    "unable to read config for disable"
                );
                continue;
            }
        };

        let mut updated = body.clone();
        for (needle, tag) in KNOWN_SERVERS {
            if updated.contains(needle) {
                updated = updated.replace(needle, &format!("gaviero-disabled-{tag}"));
            }
        }

        if updated == body {
            continue;
        }

        let backup_path = backup_path_for(&path);
        if !backup_path.exists() {
            std::fs::write(&backup_path, &body)
                .with_context(|| format!("writing backup {}", backup_path.display()))?;
        }
        std::fs::write(&path, updated)
            .with_context(|| format!("disabling external MCP servers in {}", path.display()))?;
    }

    Ok(hits)
}

const KNOWN_SERVERS: &[(&str, &str)] = &[
    ("@modelcontextprotocol/server-memory", "server-memory"),
    ("mem0-mcp", "mem0"),
    ("memory-bank-mcp", "memory-bank"),
];

fn backup_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "mcp-config".to_string());
    path.with_file_name(format!("{file_name}.gaviero.bak"))
}

/// Best-effort server-block name lookup. Matches `"<name>": { … "foo" }`
/// in both JSON (.mcp.json / claude.json) and TOML
/// (`[mcp_servers.<name>]`). Falls back to the raw needle so the UI
/// always has something to show.
fn infer_server_name(body: &str, needle: &str) -> String {
    let Some(idx) = body.find(needle) else {
        return needle.to_string();
    };
    // Walk back to the previous header-like token.
    let head = &body[..idx];
    if let Some(name) = head.rsplit("mcp_servers.").next().and_then(|s| {
        let end = s
            .find(']')
            .unwrap_or(s.len())
            .min(s.find('\n').unwrap_or(s.len()));
        let cand = &s[..end];
        let trimmed = cand.trim().trim_matches(|c: char| c == '"' || c == '.');
        if trimmed.is_empty() || trimmed.contains('[') {
            None
        } else {
            Some(trimmed.to_string())
        }
    }) {
        return name;
    }
    if let Some(name) = head.rsplit('"').nth(1).map(|s| s.to_string()) {
        if !name.is_empty() && !name.contains('/') {
            return name;
        }
    }
    needle.to_string()
}

/// Import entries from `@modelcontextprotocol/server-memory`'s
/// `memory.jsonl` dump. Each JSONL line is a `{ name, contents, ... }`
/// record; we lossy-map each into a Workspace-scope memory via the
/// writer task.
///
/// Returns (inserted_count, skipped_count). Non-fatal on parse errors
/// — prints a warning per line.
pub async fn import_server_memory_jsonl(
    jsonl_path: &Path,
    writer: &WriterHandle,
) -> Result<(usize, usize)> {
    let body = std::fs::read_to_string(jsonl_path)
        .with_context(|| format!("reading {}", jsonl_path.display()))?;
    let mut inserted = 0usize;
    let mut skipped = 0usize;
    for (lineno, line) in body.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    target: "mcp_import",
                    line = lineno + 1,
                    error = %e,
                    "skipping malformed jsonl record"
                );
                skipped += 1;
                continue;
            }
        };
        // Entry shape varies: server-memory uses `{ type: "entity",
        // name, entityType, observations: [...] }` — we flatten
        // observations into `"<name>: <obs>"` lines.
        let text = match entity_to_text(&value) {
            Some(t) => t,
            None => {
                skipped += 1;
                continue;
            }
        };
        let meta = WriteMeta::for_source(MemorySource::McpImport).with_importance(0.5);
        let _ = writer.enqueue(WriterMessage::SwarmConsolidate {
            scope: WriteScope::Workspace,
            content: text,
            meta,
            ack: None,
        });
        inserted += 1;
    }
    Ok((inserted, skipped))
}

fn entity_to_text(value: &serde_json::Value) -> Option<String> {
    let name = value.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let ty = value
        .get("entityType")
        .or_else(|| value.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let obs: Vec<&str> = value
        .get("observations")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect())
        .unwrap_or_default();
    if name.is_empty() && obs.is_empty() {
        return None;
    }
    let body = if obs.is_empty() {
        value
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        obs.join(" · ")
    };
    if body.trim().is_empty() {
        return None;
    }
    Some(format!("[{ty}] {name}: {body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detect_finds_server_memory_in_claude_config() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("claude.json");
        std::fs::write(
            &cfg,
            r#"{"mcpServers":{"memory":{"command":"npx","args":["-y","@modelcontextprotocol/server-memory"]}}}"#,
        )
        .unwrap();
        let hits = detect_external_memory_servers(&[cfg.clone()]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_tag, "server-memory");
    }

    #[test]
    fn detect_is_quiet_on_clean_config() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join("clean.json");
        std::fs::write(&cfg, r#"{"mcpServers":{}}"#).unwrap();
        assert!(detect_external_memory_servers(&[cfg]).is_empty());
    }

    #[test]
    fn disable_rewrites_known_server_and_writes_backup() {
        let dir = tempdir().unwrap();
        let cfg = dir.path().join(".mcp.json");
        std::fs::write(
            &cfg,
            r#"{"mcpServers":{"memory":{"command":"npx","args":["-y","@modelcontextprotocol/server-memory"]}}}"#,
        )
        .unwrap();

        let hits = disable_external_memory_servers(&[cfg.clone()]).unwrap();
        assert_eq!(hits.len(), 1);
        let updated = std::fs::read_to_string(&cfg).unwrap();
        assert!(!updated.contains("@modelcontextprotocol/server-memory"));
        assert!(updated.contains("gaviero-disabled-server-memory"));
        assert!(backup_path_for(&cfg).exists());
    }

    #[test]
    fn entity_to_text_flattens_observations() {
        let v = serde_json::json!({
            "name": "ripgrep",
            "entityType": "tool",
            "observations": ["use -S for smart case", "exclude node_modules"]
        });
        let t = entity_to_text(&v).unwrap();
        assert!(t.contains("ripgrep"));
        assert!(t.contains("smart case"));
    }
}
