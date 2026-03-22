use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

// ── FileScope ────────────────────────────────────────────────────

/// Defines which paths an agent is allowed to write to.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileScope {
    #[serde(default)]
    pub owned_paths: Vec<String>,
    #[serde(default)]
    pub read_only_paths: Vec<String>,
    /// Interface contracts: symbol → signature/description.
    /// Used by the merge resolver to detect breaking changes.
    #[serde(default)]
    pub interface_contracts: HashMap<String, String>,
}

impl FileScope {
    /// Check if a path is within this scope's owned paths.
    /// Directory entries (ending with `/`) use prefix matching;
    /// file entries use exact matching.
    /// Paths are normalized: leading `./` is stripped, whitespace is trimmed.
    pub fn is_owned(&self, path: &str) -> bool {
        let normalized = normalize_path(path);
        self.owned_paths.iter().any(|owned| {
            let owned = normalize_path(owned);
            if owned.ends_with('/') {
                normalized.starts_with(&owned) || normalized == owned.trim_end_matches('/')
            } else {
                normalized == owned
            }
        })
    }

    /// Render the scope as a markdown clause for inclusion in prompts.
    pub fn to_prompt_clause(&self) -> String {
        let mut out = String::new();
        if !self.owned_paths.is_empty() {
            out.push_str("**Owned paths** (read/write):\n");
            for p in &self.owned_paths {
                out.push_str(&format!("- `{}`\n", p));
            }
        }
        if !self.read_only_paths.is_empty() {
            out.push_str("**Read-only paths**:\n");
            for p in &self.read_only_paths {
                out.push_str(&format!("- `{}`\n", p));
            }
        }
        if !self.interface_contracts.is_empty() {
            out.push_str("**Interface contracts**:\n");
            for (symbol, signature) in &self.interface_contracts {
                out.push_str(&format!("- `{}`: {}\n", symbol, signature));
            }
        }
        out
    }
}

/// Normalize a file path for comparison: trim whitespace and strip leading `./`.
pub fn normalize_path(path: &str) -> String {
    let p = path.trim();
    let p = p.strip_prefix("./").unwrap_or(p);
    p.to_string()
}

// ── Diff types ───────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffHunk {
    pub original_range: (usize, usize), // (start_line, end_line) 0-indexed
    pub proposed_range: (usize, usize),
    pub original_text: String,
    pub proposed_text: String,
    pub hunk_type: HunkType,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HunkType {
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub kind: String,
    pub name: Option<String>,
    pub range: (usize, usize), // (start_line, end_line)
}

#[derive(Clone, Debug)]
pub struct StructuralHunk {
    pub diff_hunk: DiffHunk,
    pub enclosing_node: Option<NodeInfo>,
    pub description: String,
    pub status: HunkStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HunkStatus {
    Pending,
    Accepted,
    Rejected,
}

// ── WriteProposal ────────────────────────────────────────────────

/// A proposed set of changes to a single file.
#[derive(Clone, Debug)]
pub struct WriteProposal {
    pub id: u64,
    pub source: String, // agent or component that produced this proposal
    pub file_path: PathBuf,
    pub original_content: String,
    pub proposed_content: String,
    pub structural_hunks: Vec<StructuralHunk>,
    pub status: ProposalStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProposalStatus {
    Pending,
    PartiallyAccepted,
    Accepted,
    Rejected,
}

// ── SymbolKind ───────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Class,
    Struct,
    Enum,
    Interface,
    Method,
    Const,
    Trait,
    Module,
}

// ── Tier Routing ────────────────────────────────────────────────

/// Model tier for task routing in the coordinated swarm pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// Coordinator: planning, decomposition, verification strategy (Opus)
    Coordinator,
    /// Complex multi-file semantic reasoning (Sonnet)
    Reasoning,
    /// Focused single-file execution (Haiku)
    Execution,
    /// Mechanical rote changes — optional, local LLM
    Mechanical,
}

impl Default for ModelTier {
    fn default() -> Self {
        Self::Execution
    }
}

/// Privacy classification for routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyLevel {
    /// Can be sent to any API-based model
    Public,
    /// Must stay on local model only
    LocalOnly,
}

impl Default for PrivacyLevel {
    fn default() -> Self {
        Self::Public
    }
}

/// Coordinator-produced task with tier annotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierAnnotation {
    pub tier: ModelTier,
    pub privacy: PrivacyLevel,
    pub estimated_context_tokens: u32,
    pub rationale: String,
}

/// Metadata stored alongside memory entries for versioning and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryMetadata {
    pub privacy: PrivacyLevel,
    /// Schema version for stored entries (current: 1).
    pub format_version: u8,
    /// Origin of the entry (e.g., "swarm_pipeline", "remember_command").
    pub source: String,
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_scope_exact_match() {
        let scope = FileScope {
            owned_paths: vec!["src/main.rs".into()],
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        };
        assert!(scope.is_owned("src/main.rs"));
        assert!(scope.is_owned("./src/main.rs"));
        assert!(!scope.is_owned("src/lib.rs"));
    }

    #[test]
    fn test_file_scope_directory_match() {
        let scope = FileScope {
            owned_paths: vec!["src/editor/".into()],
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        };
        assert!(scope.is_owned("src/editor/buffer.rs"));
        assert!(scope.is_owned("./src/editor/view.rs"));
        assert!(scope.is_owned("src/editor")); // exact dir name without trailing /
        assert!(!scope.is_owned("src/panels/file_tree.rs"));
    }

    #[test]
    fn test_file_scope_normalization() {
        let scope = FileScope {
            owned_paths: vec!["./src/lib.rs".into()],
            read_only_paths: vec![],
            interface_contracts: HashMap::new(),
        };
        assert!(scope.is_owned("src/lib.rs"));
        assert!(scope.is_owned("  src/lib.rs  "));
    }

    #[test]
    fn test_model_tier_serde_roundtrip() {
        let tier = ModelTier::Mechanical;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"mechanical\"");
        let back: ModelTier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tier);
    }

    #[test]
    fn test_model_tier_as_hash_key() {
        let mut map = HashMap::new();
        map.insert(ModelTier::Reasoning, "sonnet");
        map.insert(ModelTier::Execution, "haiku");
        assert_eq!(map[&ModelTier::Reasoning], "sonnet");
    }

    #[test]
    fn test_privacy_level_serde_roundtrip() {
        let lvl = PrivacyLevel::LocalOnly;
        let json = serde_json::to_string(&lvl).unwrap();
        assert_eq!(json, "\"local_only\"");
        let back: PrivacyLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, lvl);
    }

    #[test]
    fn test_entry_metadata_serde() {
        let meta = EntryMetadata {
            privacy: PrivacyLevel::Public,
            format_version: 1,
            source: "swarm_pipeline".into(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: EntryMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(back.format_version, 1);
        assert_eq!(back.privacy, PrivacyLevel::Public);
    }

    #[test]
    fn test_to_prompt_clause() {
        let scope = FileScope {
            owned_paths: vec!["src/".into()],
            read_only_paths: vec!["Cargo.toml".into()],
            interface_contracts: HashMap::from([("api::Client".into(), "pub fn connect(&self) -> Result<()>".into())]),
        };
        let clause = scope.to_prompt_clause();
        assert!(clause.contains("Owned paths"));
        assert!(clause.contains("`src/`"));
        assert!(clause.contains("Read-only"));
        assert!(clause.contains("Interface contracts"));
    }
}
