//! Shared artifact blackboard for inter-agent result sharing.
//!
//! Extends the original discovery board: agents publish typed artifacts
//! (conclusions, critiques, spawn manifests, discoveries) to a run-scoped
//! store under `.gaviero/runs/<run_id>/artifacts/`. Peer lists are injected
//! into later prompts so refine/consensus loops need not hand-author
//! `{{PEER_READ_BLOCK}}` conventions.
//!
//! `SharedBoard` remains the public type name; `ArtifactBlackboard` is an alias.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Compatibility alias — preferred name in new code.
pub type ArtifactBlackboard = SharedBoard;

/// Kind of artifact on the blackboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Conclusion,
    Critique,
    Discovery,
    SpawnManifest,
    Other(String),
}

impl ArtifactKind {
    pub fn dir_name(&self) -> &str {
        match self {
            Self::Conclusion => "conclusion",
            Self::Critique => "critique",
            Self::Discovery => "discovery",
            Self::SpawnManifest => "spawn_manifest",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn parse_dir(name: &str) -> Self {
        match name {
            "conclusion" => Self::Conclusion,
            "critique" => Self::Critique,
            "discovery" => Self::Discovery,
            "spawn_manifest" => Self::SpawnManifest,
            other => Self::Other(other.to_string()),
        }
    }
}

/// A typed artifact registered on the board (and optionally persisted to disk).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    pub from_agent: String,
    pub key: String,
    /// Path relative to the workspace root (or absolute under run artifacts).
    pub relative_path: String,
    pub tags: Vec<String>,
    pub iter: Option<u32>,
}

/// Legacy discovery entry (in-memory only content). Still used by
/// `[discovery: tag] content` parsing from agent output.
#[derive(Debug, Clone)]
pub struct SharedEntry {
    pub from_agent: String,
    pub content: String,
    /// Path-like tags used for filtering (e.g. `"src/auth.rs"`).
    pub tags: Vec<String>,
}

/// In-memory + filesystem board shared across all agents in a swarm run.
pub struct SharedBoard {
    entries: RwLock<Vec<SharedEntry>>,
    artifacts: RwLock<Vec<Artifact>>,
    run_root: RwLock<Option<PathBuf>>,
    run_id: RwLock<Option<String>>,
}

impl SharedBoard {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            artifacts: RwLock::new(Vec::new()),
            run_root: RwLock::new(None),
            run_id: RwLock::new(None),
        }
    }

    /// Bind this board to a workspace run directory.
    ///
    /// Artifacts persist under `<workspace>/.gaviero/runs/<run_id>/artifacts/`.
    pub async fn bind_run(&self, workspace_root: &Path, run_id: &str) {
        let root = workspace_root
            .join(".gaviero")
            .join("runs")
            .join(run_id)
            .join("artifacts");
        let _ = std::fs::create_dir_all(&root);
        *self.run_root.write().await = Some(root);
        *self.run_id.write().await = Some(run_id.to_string());
    }

    pub async fn run_id(&self) -> Option<String> {
        self.run_id.read().await.clone()
    }

    /// Absolute artifacts root for this run, if bound.
    pub async fn artifacts_root(&self) -> Option<PathBuf> {
        self.run_root.read().await.clone()
    }

    /// Post a legacy discovery entry.
    pub async fn post(&self, entry: SharedEntry) {
        self.entries.write().await.push(entry);
    }

    /// Publish a typed artifact. When the run is bound, copies/writes under
    /// `artifacts/<kind>/<key>` (sanitized). Returns the registered artifact.
    pub async fn publish(
        &self,
        kind: ArtifactKind,
        from_agent: &str,
        key: &str,
        source_path: Option<&Path>,
        content: Option<&str>,
        tags: Vec<String>,
        iter: Option<u32>,
    ) -> std::io::Result<Artifact> {
        let safe_key: String = key
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let kind_dir = kind.dir_name().to_string();
        let file_name = if safe_key.contains('.') {
            safe_key.clone()
        } else {
            format!("{safe_key}.md")
        };

        let relative_path = format!("artifacts/{kind_dir}/{file_name}");
        if let Some(root) = self.run_root.read().await.as_ref() {
            let dest_dir = root.join(&kind_dir);
            std::fs::create_dir_all(&dest_dir)?;
            let dest = dest_dir.join(&file_name);
            if let Some(src) = source_path {
                if src.exists() {
                    let _ = std::fs::copy(src, &dest);
                } else if let Some(body) = content {
                    std::fs::write(&dest, body)?;
                }
            } else if let Some(body) = content {
                std::fs::write(&dest, body)?;
            }
        }

        let id = format!("{from_agent}:{kind_dir}:{safe_key}");
        let artifact = Artifact {
            id: id.clone(),
            kind,
            from_agent: from_agent.to_string(),
            key: safe_key,
            relative_path,
            tags,
            iter,
        };
        self.artifacts.write().await.push(artifact.clone());
        Ok(artifact)
    }

    pub async fn list(&self, kind: &ArtifactKind) -> Vec<Artifact> {
        self.artifacts
            .read()
            .await
            .iter()
            .filter(|a| &a.kind == kind)
            .cloned()
            .collect()
    }

    pub async fn list_peers(&self, exclude_agent: &str) -> Vec<Artifact> {
        self.artifacts
            .read()
            .await
            .iter()
            .filter(|a| a.from_agent != exclude_agent)
            .cloned()
            .collect()
    }

    /// Markdown peer block for prompt injection (paths relative to run artifacts).
    pub async fn format_peer_block(
        &self,
        exclude_agent: &str,
        kind: Option<&ArtifactKind>,
    ) -> String {
        let arts = self.artifacts.read().await;
        let relevant: Vec<&Artifact> = arts
            .iter()
            .filter(|a| a.from_agent != exclude_agent)
            .filter(|a| kind.is_none_or(|k| &a.kind == k))
            .collect();
        if relevant.is_empty() {
            return String::new();
        }
        let mut out = String::from("## Peer artifacts (blackboard):\n");
        for a in relevant {
            out.push_str(&format!(
                "- [{}] {} — `{}` (from {})\n",
                a.kind.dir_name(),
                a.key,
                a.relative_path,
                a.from_agent
            ));
        }
        out.push_str(
            "\nRead these peer files before refining your own conclusion. Compare and improve.\n",
        );
        out
    }

    pub fn path_for(run_root: &Path, artifact: &Artifact) -> PathBuf {
        // relative_path is `artifacts/<kind>/<key>`; run_root already ends at `artifacts/`
        let rest = artifact
            .relative_path
            .strip_prefix("artifacts/")
            .unwrap_or(&artifact.relative_path);
        run_root.join(rest)
    }

    /// Format legacy discovery entries relevant to the given owned paths.
    pub async fn format_for_prompt(&self, owned_paths: &[String]) -> String {
        let entries = self.entries.read().await;
        let relevant: Vec<&SharedEntry> = entries
            .iter()
            .filter(|e| {
                e.tags.iter().any(|tag| {
                    owned_paths
                        .iter()
                        .any(|p| p.contains(tag.as_str()) || tag.contains(p.as_str()))
                })
            })
            .collect();

        if relevant.is_empty() {
            return String::new();
        }

        let mut out = String::from("## Discoveries from other agents:\n");
        for entry in relevant {
            out.push_str(&format!(
                "- (from {}) {}\n",
                entry.from_agent, entry.content
            ));
        }
        out
    }

    /// Auto-register `*-conclusion-v*.md` files under `search_root` written by `agent_id`.
    pub async fn auto_register_conclusions(
        &self,
        agent_id: &str,
        search_root: &Path,
        iter: Option<u32>,
    ) {
        let walker = walkdir::WalkDir::new(search_root).max_depth(4);
        for entry in walker.into_iter().flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !(name.contains("-conclusion-v") && name.ends_with(".md")) {
                continue;
            }
            let key = name.trim_end_matches(".md").to_string();
            let _ = self
                .publish(
                    ArtifactKind::Conclusion,
                    agent_id,
                    &key,
                    Some(path),
                    None,
                    vec![agent_id.to_string()],
                    iter,
                )
                .await;
        }
    }

    /// Load a spawn manifest artifact for `from_agent`, if present.
    pub async fn spawn_manifest_path(&self, from_agent: &str) -> Option<PathBuf> {
        let root = self.run_root.read().await.clone()?;
        let arts = self.artifacts.read().await;
        arts.iter()
            .rev()
            .find(|a| {
                a.from_agent == from_agent && matches!(a.kind, ArtifactKind::SpawnManifest)
            })
            .map(|a| Self::path_for(&root, a))
            .or_else(|| {
                // Convention: artifacts/spawn_manifest/from-<unit>.json
                let p = root
                    .join("spawn_manifest")
                    .join(format!("from-{from_agent}.json"));
                if p.exists() { Some(p) } else { None }
            })
    }
}

impl Default for SharedBoard {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse `[discovery: <tag>] <content>` patterns from agent output text.
pub fn parse_discoveries(from_agent: &str, text: &str) -> Vec<SharedEntry> {
    let mut entries = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("[discovery:") {
            if let Some((tag_part, content)) = rest.split_once(']') {
                let tag = tag_part.trim().to_string();
                let content = content.trim().to_string();
                if !content.is_empty() {
                    entries.push(SharedEntry {
                        from_agent: from_agent.to_string(),
                        content,
                        tags: vec![tag],
                    });
                }
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn publish_list_and_peer_block() {
        let dir = tempdir().unwrap();
        let board = SharedBoard::new();
        board.bind_run(dir.path(), "run-test").await;

        board
            .publish(
                ArtifactKind::Conclusion,
                "alice",
                "alice-conclusion-v1",
                None,
                Some("# Alice\n"),
                vec![],
                Some(1),
            )
            .await
            .unwrap();
        board
            .publish(
                ArtifactKind::Conclusion,
                "bob",
                "bob-conclusion-v1",
                None,
                Some("# Bob\n"),
                vec![],
                Some(1),
            )
            .await
            .unwrap();

        let conclusions = board.list(&ArtifactKind::Conclusion).await;
        assert_eq!(conclusions.len(), 2);

        let peers = board.list_peers("alice").await;
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].from_agent, "bob");

        let block = board
            .format_peer_block("alice", Some(&ArtifactKind::Conclusion))
            .await;
        assert!(block.contains("bob-conclusion-v1"));
        assert!(!block.contains("alice-conclusion-v1"));
    }

    #[tokio::test]
    async fn auto_register_conclusion_files() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("claude-conclusion-v1.md"), "hi").unwrap();

        let board = SharedBoard::new();
        board.bind_run(dir.path(), "run2").await;
        board
            .auto_register_conclusions("claude", &out, Some(1))
            .await;
        let list = board.list(&ArtifactKind::Conclusion).await;
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].from_agent, "claude");
    }
}
