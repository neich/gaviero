//! Shared discovery board for inter-agent communication.
//!
//! Parallel agents can post tagged discoveries (API patterns, configuration
//! requirements, architectural constraints) that are injected into subsequent
//! agents' prompts. Filtering is tag-based: entries whose tags overlap with
//! an agent's owned paths are included.
//!
//! This is a lightweight alternative to full inter-agent memory sharing.
//! See Phase 6 of the implementation plan.

use tokio::sync::RwLock;

/// A single discovery posted by an agent.
#[derive(Debug, Clone)]
pub struct SharedEntry {
    pub from_agent: String,
    pub content: String,
    /// Path-like tags used for filtering (e.g. `"src/auth.rs"`).
    pub tags: Vec<String>,
}

/// In-memory board shared across all agents in a swarm run.
pub struct SharedBoard {
    entries: RwLock<Vec<SharedEntry>>,
}

impl SharedBoard {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
        }
    }

    /// Post a discovery to the board.
    pub async fn post(&self, entry: SharedEntry) {
        self.entries.write().await.push(entry);
    }

    /// Format all entries relevant to the given owned paths as a prompt section.
    ///
    /// Returns an empty string if there are no relevant entries.
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
