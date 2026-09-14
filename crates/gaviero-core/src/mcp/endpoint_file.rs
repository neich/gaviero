//! Per-workspace MCP endpoint descriptor (`<root>/.gaviero/mcp-endpoint.json`).
//!
//! Written when the in-process server starts so `gaviero-mcp-shim --resolve`
//! can walk up from a nested cwd (vendor subagent, git worktree) and find
//! the *current* workspace's pipe/socket without a user-scope URL.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::transport::McpEndpoint;

pub const DESCRIPTOR_VERSION: u32 = 1;
pub const DESCRIPTOR_FILENAME: &str = "mcp-endpoint.json";

/// On-disk endpoint descriptor. `http_*` stay `None` until the HTTP
/// listener (P2) fills them in.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpEndpointDescriptor {
    pub v: u32,
    pub workspace_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_token_path: Option<PathBuf>,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
}

impl McpEndpointDescriptor {
    pub fn path(root: &Path) -> PathBuf {
        root.join(".gaviero").join(DESCRIPTOR_FILENAME)
    }

    pub fn from_listener(root: &Path, endpoint: &McpEndpoint, pid: u32) -> Self {
        let (pipe, socket) = match endpoint {
            McpEndpoint::Pipe(name) => (Some(name.clone()), None),
            McpEndpoint::Unix(path) => (None, Some(path.clone())),
        };
        Self {
            v: DESCRIPTOR_VERSION,
            workspace_id: crate::workspace::identity::workspace_id_hex16(root),
            pipe,
            socket,
            http_url: None,
            http_token_path: None,
            pid,
            started_at: Utc::now(),
        }
    }
}

pub fn write_descriptor(root: &Path, desc: &McpEndpointDescriptor) -> Result<PathBuf> {
    let path = McpEndpointDescriptor::path(root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(desc).context("serialising mcp-endpoint.json")?;
    write_atomic(&path, &json)?;
    Ok(path)
}

pub fn read_descriptor(path: &Path) -> Result<McpEndpointDescriptor> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Walk `cwd` and its parents for `.gaviero/mcp-endpoint.json`.
pub fn find_descriptor_upwards(cwd: &Path) -> Option<PathBuf> {
    let mut cur = cwd.to_path_buf();
    for _ in 0..64 {
        let candidate = McpEndpointDescriptor::path(&cur);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !cur.pop() {
            break;
        }
    }
    None
}

pub fn remove_descriptor(root: &Path) {
    let _ = std::fs::remove_file(McpEndpointDescriptor::path(root));
}

fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path.parent().context("mcp-endpoint.json has no parent")?;
    let mut tmp_name = path
        .file_name()
        .context("mcp-endpoint.json has no file name")?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("creating {}", tmp.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path).with_context(|| {
        format!("renaming {} onto {}", tmp.display(), path.display())
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint_for(root: &Path) -> McpEndpoint {
        #[cfg(windows)]
        {
            let _ = root;
            McpEndpoint::Pipe(r"\\.\pipe\gaviero-testdesc".into())
        }
        #[cfg(not(windows))]
        {
            McpEndpoint::Unix(root.join(".gaviero").join("mcp.sock"))
        }
    }

    #[test]
    fn write_read_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let ep = endpoint_for(dir.path());
        let desc = McpEndpointDescriptor::from_listener(dir.path(), &ep, 4242);
        let path = write_descriptor(dir.path(), &desc).unwrap();
        assert_eq!(path, McpEndpointDescriptor::path(dir.path()));
        let loaded = read_descriptor(&path).unwrap();
        assert_eq!(loaded.v, 1);
        assert_eq!(loaded.pid, 4242);
        assert_eq!(loaded.workspace_id, desc.workspace_id);
        assert!(!path.with_file_name("mcp-endpoint.json.tmp").exists());
    }

    #[test]
    fn find_descriptor_upwards_from_nested_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let ep = endpoint_for(dir.path());
        let desc = McpEndpointDescriptor::from_listener(dir.path(), &ep, 1);
        write_descriptor(dir.path(), &desc).unwrap();
        let found = find_descriptor_upwards(&nested).unwrap();
        assert_eq!(found, McpEndpointDescriptor::path(dir.path()));
    }

    #[test]
    fn find_descriptor_upwards_missing_is_none() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(find_descriptor_upwards(dir.path()).is_none());
    }

    #[test]
    fn remove_descriptor_deletes_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let ep = endpoint_for(dir.path());
        let desc = McpEndpointDescriptor::from_listener(dir.path(), &ep, 1);
        write_descriptor(dir.path(), &desc).unwrap();
        remove_descriptor(dir.path());
        assert!(!McpEndpointDescriptor::path(dir.path()).exists());
    }
}
