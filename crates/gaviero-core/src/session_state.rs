//! Workspace session state — persists open tabs, pane layout, tree state, etc.
//!
//! State is stored in the platform data directory:
//!   Linux:   ~/.local/share/gaviero/workspaces/<key>/state.json
//!   macOS:   ~/Library/Application Support/gaviero/workspaces/<key>/state.json
//!   Windows: %APPDATA%/gaviero/workspaces/<key>/state.json
//!
//! The <key> is derived from the canonical workspace path to avoid collisions.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Persisted state for one editing session.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionState {
    /// Open tabs in order, with cursor/scroll positions.
    #[serde(default)]
    pub tabs: Vec<TabState>,

    /// Index of the active tab (0-based).
    #[serde(default)]
    pub active_tab: usize,

    /// Panel visibility.
    #[serde(default)]
    pub panels: PanelState,

    /// Expanded directories in the file tree (stored as relative paths from workspace root).
    #[serde(default)]
    pub tree_expanded: Vec<String>,

    /// Selected index in the file tree.
    #[serde(default)]
    pub tree_selected: usize,

    /// Active layout preset index (None = default widths).
    #[serde(default)]
    pub active_preset: Option<usize>,

    /// Terminal panel height as a percentage of the main area (10–80).
    #[serde(default)]
    pub terminal_split_percent: Option<u16>,

    /// Terminal tab state (tab metadata for session restore).
    #[serde(default)]
    pub terminal_session: Option<crate::terminal::session::TerminalSessionState>,
}

/// State for a single open tab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    /// Absolute path of the open file.
    pub path: String,

    /// Cursor line (0-indexed).
    #[serde(default)]
    pub cursor_line: usize,

    /// Cursor column (0-indexed).
    #[serde(default)]
    pub cursor_col: usize,

    /// Top visible line (scroll position).
    #[serde(default)]
    pub scroll_top: usize,
}

/// Panel visibility state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelState {
    #[serde(default = "default_true")]
    pub file_tree: bool,

    #[serde(default)]
    pub side_panel: bool,

    #[serde(default)]
    pub terminal: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            file_tree: true,
            side_panel: false,
            terminal: false,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Compute the state directory for a given workspace path.
/// Uses the canonical path, hashed to avoid filesystem-unfriendly characters.
pub fn state_dir_for(workspace_key: &Path) -> Option<PathBuf> {
    let data_dir = dirs::data_dir()?;
    let key = path_to_key(workspace_key);
    Some(data_dir.join("gaviero").join("workspaces").join(key))
}

/// Load session state for a workspace. Returns `Default` if no state file exists.
pub fn load_session(workspace_key: &Path) -> SessionState {
    let Some(dir) = state_dir_for(workspace_key) else {
        return SessionState::default();
    };
    let state_path = dir.join("state.json");
    match std::fs::read_to_string(&state_path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => state,
            Err(e) => {
                tracing::warn!("Corrupt session state at {}, using defaults: {}", state_path.display(), e);
                SessionState::default()
            }
        },
        Err(_) => SessionState::default(),
    }
}

/// Save session state for a workspace.
pub fn save_session(workspace_key: &Path, state: &SessionState) -> Result<()> {
    let dir = state_dir_for(workspace_key)
        .context("could not determine data directory for session state")?;
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating state dir {}", dir.display()))?;

    let state_path = dir.join("state.json");
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(&state_path, content)
        .with_context(|| format!("writing state to {}", state_path.display()))?;
    Ok(())
}

/// Derive a filesystem-safe key from a path.
/// Uses a simple hash (not crypto-grade, just collision-resistant enough).
fn path_to_key(path: &Path) -> String {
    // Canonicalize if possible, otherwise use as-is
    let canonical = std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf());
    let s = canonical.to_string_lossy();

    // Simple FNV-1a 64-bit hash — fast, no extra deps
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

// ── Conversation persistence ────────────────────────────────────

/// A single chat message stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub role: String,    // "user", "assistant", "system"
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<String>,
    /// Unix timestamp (seconds since epoch).
    #[serde(default)]
    pub timestamp: u64,
}

/// A full conversation stored on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredConversation {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub messages: Vec<StoredMessage>,
    /// Unix timestamp of creation.
    pub created: u64,
    /// Unix timestamp of last activity.
    pub updated: u64,
    /// Per-conversation model override (None = use global default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Per-conversation effort level override (None = use global default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort_override: Option<String>,
}

/// Index of all conversations for a workspace (lightweight, no messages).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConversationIndex {
    pub conversations: Vec<ConversationSummary>,
    /// ID of the active conversation.
    #[serde(default)]
    pub active_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub updated: u64,
    pub message_count: usize,
}

fn conversations_dir(workspace_key: &Path) -> Option<PathBuf> {
    let dir = state_dir_for(workspace_key)?;
    Some(dir.join("conversations"))
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Generate a short unique ID for a conversation.
///
/// Uses an atomic counter to ensure uniqueness even when called
/// multiple times within the same second.
pub fn new_conversation_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);

    let ts = now_unix();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:08x}{:04x}", ts, seq & 0xFFFF)
}

/// Load the conversation index for a workspace.
pub fn load_conversation_index(workspace_key: &Path) -> ConversationIndex {
    let Some(dir) = conversations_dir(workspace_key) else {
        return ConversationIndex::default();
    };
    let index_path = dir.join("index.json");
    match std::fs::read_to_string(&index_path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(index) => index,
            Err(e) => {
                tracing::warn!("Corrupt conversation index at {}, using defaults: {}", index_path.display(), e);
                ConversationIndex::default()
            }
        },
        Err(_) => ConversationIndex::default(),
    }
}

/// Save the conversation index.
pub fn save_conversation_index(workspace_key: &Path, index: &ConversationIndex) -> Result<()> {
    let dir = conversations_dir(workspace_key)
        .context("could not determine conversations directory")?;
    std::fs::create_dir_all(&dir)?;
    let content = serde_json::to_string_pretty(index)?;
    std::fs::write(dir.join("index.json"), content)?;
    Ok(())
}

/// Load a single conversation by ID.
pub fn load_conversation(workspace_key: &Path, conv_id: &str) -> Option<StoredConversation> {
    let dir = conversations_dir(workspace_key)?;
    let path = dir.join(format!("{}.json", conv_id));
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save a single conversation.
pub fn save_conversation(workspace_key: &Path, conv: &StoredConversation) -> Result<()> {
    let dir = conversations_dir(workspace_key)
        .context("could not determine conversations directory")?;
    std::fs::create_dir_all(&dir)?;
    let content = serde_json::to_string_pretty(conv)?;
    std::fs::write(dir.join(format!("{}.json", conv.id)), content)?;
    Ok(())
}

/// Delete a conversation by ID.
pub fn delete_conversation(workspace_key: &Path, conv_id: &str) -> Result<()> {
    let dir = conversations_dir(workspace_key)
        .context("could not determine conversations directory")?;
    let path = dir.join(format!("{}.json", conv_id));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_to_key_deterministic() {
        let k1 = path_to_key(Path::new("/home/user/project"));
        let k2 = path_to_key(Path::new("/home/user/project"));
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_path_to_key_different_paths() {
        let k1 = path_to_key(Path::new("/home/user/project-a"));
        let k2 = path_to_key(Path::new("/home/user/project-b"));
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_roundtrip_state() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path();

        let state = SessionState {
            tabs: vec![TabState {
                path: "/tmp/file.rs".to_string(),
                cursor_line: 10,
                cursor_col: 5,
                scroll_top: 3,
            }],
            active_tab: 0,
            panels: PanelState {
                file_tree: true,
                side_panel: false,
                terminal: true,
            },
            tree_expanded: vec!["src".to_string(), "src/editor".to_string()],
            tree_selected: 3,
            active_preset: Some(2),
            terminal_split_percent: Some(30),
            terminal_session: None,
        };

        save_session(key, &state).unwrap();
        let loaded = load_session(key);
        assert_eq!(loaded.tabs.len(), 1);
        assert_eq!(loaded.tabs[0].path, "/tmp/file.rs");
        assert_eq!(loaded.tabs[0].cursor_line, 10);
        assert_eq!(loaded.active_tab, 0);
        assert!(loaded.panels.terminal);
        assert_eq!(loaded.tree_expanded.len(), 2);
        assert_eq!(loaded.tree_selected, 3);
    }

    #[test]
    fn test_load_missing_returns_default() {
        let state = load_session(Path::new("/nonexistent/path/that/does/not/exist"));
        assert!(state.tabs.is_empty());
        assert_eq!(state.active_tab, 0);
    }

    #[test]
    fn test_deserialize_partial_json() {
        // Ensure forward-compatibility: missing fields get defaults
        let json = r#"{ "tabs": [], "active_tab": 2 }"#;
        let state: SessionState = serde_json::from_str(json).unwrap();
        assert_eq!(state.active_tab, 2);
        assert!(state.panels.file_tree); // default true
        assert!(!state.panels.terminal); // default false
    }
}
