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
use std::io::Write;
use std::path::{Path, PathBuf};

/// Write `content` to `path` atomically: fill a sibling `*.tmp` file, fsync
/// it, then rename over the target.
///
/// Every save on this path used to be a bare `std::fs::write` straight onto
/// the live file. Because saves only happen at exit, a crash *during* the one
/// save would leave a truncated conversation — losing sessions that had
/// previously been persisted just fine. Rename is atomic on both NTFS and
/// POSIX, so a reader sees either the old file or the new one, never a
/// half-written one.
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let dir = path
        .parent()
        .context("target path has no parent directory")?;
    let mut tmp_name = path
        .file_name()
        .context("target path has no file name")?
        .to_os_string();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);

    {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("creating temp file {}", tmp.display()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("writing temp file {}", tmp.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing temp file {}", tmp.display()))?;
    }

    // Windows can transiently refuse the replace while an indexer or AV
    // handle is open on the destination. Retry briefly rather than losing the
    // save; the bound is a few milliseconds, which is acceptable even on the
    // (synchronous) exit path.
    let mut last_err = None;
    for attempt in 0..3 {
        match std::fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
        }
    }

    let _ = std::fs::remove_file(&tmp);
    Err(last_err.expect("loop records an error before falling through")).with_context(|| {
        format!(
            "renaming {} onto {} (destination left untouched)",
            tmp.display(),
            path.display()
        )
    })
}

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

    /// Markdown preview layout this tab was left in: `"split"` or
    /// `"preview"`. Stored as an opaque string so core carries no UI type;
    /// absent or unrecognised restores as source-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_mode: Option<String>,
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
                tracing::warn!(
                    "Corrupt session state at {}, using defaults: {}",
                    state_path.display(),
                    e
                );
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
    write_atomic(&state_path, &content)
        .with_context(|| format!("writing state to {}", state_path.display()))?;
    Ok(())
}

/// Derive a filesystem-safe key from a path.
/// Uses a simple hash (not crypto-grade, just collision-resistant enough).
fn path_to_key(path: &Path) -> String {
    // Canonicalize if possible, otherwise use as-is
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
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
    pub role: String, // "user", "assistant", "system"
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
    /// V9 §11 M4: persisted planner ledger (continuity handle + fingerprint
    /// + turn count + replay history). Absent in records written before M4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_ledger: Option<crate::context_planner::ledger::PersistedLedger>,
    /// V9 §4 requires this alongside `session_ledger` for forward
    /// compatibility — a reader that understands only the handle (not the
    /// full ledger) can still resume. Redundant with `session_ledger.continuity_handle`
    /// but kept as an explicit top-level field per V9 §4 line 420.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuity_handle: Option<crate::context_planner::types::ContinuityHandle>,
    /// T1: latest server-reported token usage for this conversation.
    /// Persisted so the context-window indicator survives restart and
    /// reflects the actual session prefix size on resume rather than
    /// dropping back to the visible-panel char-count estimate. Absent in
    /// records written before T1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_token_usage: Option<StoredTokenUsage>,
}

/// Serializable mirror of [`crate::acp::protocol::TokenUsage`] for on-disk
/// persistence in [`StoredConversation`]. Mirrors the provider-reported
/// shape exactly so the TUI can rehydrate it without conversion logic.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredTokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

impl From<&crate::acp::protocol::TokenUsage> for StoredTokenUsage {
    fn from(u: &crate::acp::protocol::TokenUsage) -> Self {
        Self {
            input_tokens: u.input_tokens,
            cache_creation_input_tokens: u.cache_creation_input_tokens,
            cache_read_input_tokens: u.cache_read_input_tokens,
            output_tokens: u.output_tokens,
        }
    }
}

impl From<StoredTokenUsage> for crate::acp::protocol::TokenUsage {
    fn from(s: StoredTokenUsage) -> Self {
        Self {
            input_tokens: s.input_tokens,
            cache_creation_input_tokens: s.cache_creation_input_tokens,
            cache_read_input_tokens: s.cache_read_input_tokens,
            output_tokens: s.output_tokens,
        }
    }
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
                tracing::warn!(
                    "Corrupt conversation index at {}, using defaults: {}",
                    index_path.display(),
                    e
                );
                ConversationIndex::default()
            }
        },
        Err(_) => ConversationIndex::default(),
    }
}

/// Save the conversation index.
pub fn save_conversation_index(workspace_key: &Path, index: &ConversationIndex) -> Result<()> {
    let dir =
        conversations_dir(workspace_key).context("could not determine conversations directory")?;
    std::fs::create_dir_all(&dir)?;
    let content = serde_json::to_string_pretty(index)?;
    write_atomic(&dir.join("index.json"), &content)?;
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
    let dir =
        conversations_dir(workspace_key).context("could not determine conversations directory")?;
    std::fs::create_dir_all(&dir)?;
    let content = serde_json::to_string_pretty(conv)?;
    write_atomic(&dir.join(format!("{}.json", conv.id)), &content)?;
    Ok(())
}

/// Delete a conversation by ID.
pub fn delete_conversation(workspace_key: &Path, conv_id: &str) -> Result<()> {
    let dir =
        conversations_dir(workspace_key).context("could not determine conversations directory")?;
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
                preview_mode: Some("split".to_string()),
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
        assert_eq!(loaded.tabs[0].preview_mode.as_deref(), Some("split"));
        assert_eq!(loaded.active_tab, 0);
        assert!(loaded.panels.terminal);
        assert_eq!(loaded.tree_expanded.len(), 2);
        assert_eq!(loaded.tree_selected, 3);
    }

    #[test]
    fn write_atomic_replaces_existing_content_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("thing.json");
        std::fs::write(&path, "old contents that are much longer").unwrap();

        write_atomic(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(
            !dir.path().join("thing.json.tmp").exists(),
            "temp file must be renamed away, not left behind"
        );
    }

    #[test]
    fn write_atomic_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh.json");
        write_atomic(&path, "{}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    }

    #[test]
    fn save_conversation_leaves_a_readable_file_after_repeated_saves() {
        // Guards the truncation failure mode: the second save must never be
        // observable as a partially written first save.
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path();

        let mut conv = StoredConversation {
            id: "c1".into(),
            title: "First".into(),
            messages: vec![StoredMessage {
                role: "user".into(),
                content: "a".repeat(4096),
                tool_calls: Vec::new(),
                timestamp: 0,
            }],
            created: 1,
            updated: 2,
            model_override: None,
            effort_override: None,
            session_ledger: None,
            continuity_handle: None,
            last_token_usage: None,
        };
        save_conversation(key, &conv).unwrap();

        conv.title = "Second".into();
        conv.messages.clear();
        save_conversation(key, &conv).unwrap();

        let back = load_conversation(key, "c1").expect("conversation still parses");
        assert_eq!(back.title, "Second");
        assert!(back.messages.is_empty());

        let _ = std::fs::remove_dir_all(state_dir_for(key).unwrap());
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

    #[test]
    fn m4_stored_conversation_forward_compatible_without_ledger_fields() {
        // V9 §11 M4 forbidden shortcut: "No breaking schema change."
        // A record written pre-M4 (no session_ledger / continuity_handle)
        // must still deserialize. Both new fields have `#[serde(default)]`.
        let json = r#"{
            "id": "c1",
            "title": "Old",
            "messages": [],
            "created": 1000,
            "updated": 2000
        }"#;
        let stored: StoredConversation = serde_json::from_str(json).unwrap();
        assert_eq!(stored.id, "c1");
        assert!(stored.session_ledger.is_none());
        assert!(stored.continuity_handle.is_none());
    }

    #[test]
    fn stored_message_without_a_timestamp_defaults_to_zero() {
        // Conversations saved before message timestamps were persisted must
        // still load. `0` is the "unknown" sentinel the TUI renders as no
        // stamp at all, rather than as the epoch.
        let json = r#"{
            "id": "c1",
            "title": "Old",
            "messages": [{ "role": "user", "content": "hi" }],
            "created": 1000,
            "updated": 2000
        }"#;
        let stored: StoredConversation = serde_json::from_str(json).unwrap();
        assert_eq!(stored.messages[0].timestamp, 0);
        assert_eq!(stored.messages[0].content, "hi");
    }

    #[test]
    fn stored_message_timestamp_round_trips() {
        let stored = StoredConversation {
            id: "c1".into(),
            title: "T".into(),
            messages: vec![StoredMessage {
                role: "assistant".into(),
                content: "answer".into(),
                tool_calls: Vec::new(),
                timestamp: 1_757_000_042,
            }],
            created: 1000,
            updated: 2000,
            model_override: None,
            effort_override: None,
            session_ledger: None,
            continuity_handle: None,
            last_token_usage: None,
        };

        let json = serde_json::to_string(&stored).unwrap();
        let back: StoredConversation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.messages[0].timestamp, 1_757_000_042);
    }

    #[test]
    fn m4_stored_conversation_round_trips_with_ledger() {
        // Explicit variant tag on ContinuityHandle must survive the round-trip.
        use crate::context_planner::ledger::{PersistedLedger, PlannerFingerprint};
        use crate::context_planner::types::ContinuityHandle;

        let persisted = PersistedLedger {
            continuity_handle: Some(ContinuityHandle::ClaudeSessionId("abc".into())),
            fingerprint: PlannerFingerprint {
                provider: "claude".into(),
                model: "sonnet".into(),
                system_prompt_digest: String::new(),
                toolset_digest: String::new(),
                workspace_root_digest: String::new(),
                branch_name: None,
            },
            turn_count: 2,
            replay_history: Vec::new(),
            last_successful_resume_unix: Some(1_700_000_000),
        };
        let stored = StoredConversation {
            id: "c1".into(),
            title: "T".into(),
            messages: Vec::new(),
            created: 1000,
            updated: 2000,
            model_override: None,
            effort_override: None,
            session_ledger: Some(persisted.clone()),
            continuity_handle: Some(ContinuityHandle::ClaudeSessionId("abc".into())),
            last_token_usage: None,
        };

        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("ClaudeSessionId"));
        let back: StoredConversation = serde_json::from_str(&json).unwrap();
        let ledger = back.session_ledger.expect("ledger present");
        assert_eq!(ledger.turn_count, 2);
        match ledger.continuity_handle {
            Some(ContinuityHandle::ClaudeSessionId(id)) => assert_eq!(id, "abc"),
            _ => panic!("variant lost"),
        }
    }

    #[test]
    fn t1_stored_token_usage_round_trips() {
        // T1: persisted token usage must survive serde so the context
        // indicator reflects the resumed session immediately after a TUI
        // restart (not 0% / ~0% until the first post-restart turn).
        let stored = StoredConversation {
            id: "c1".into(),
            title: "T".into(),
            messages: Vec::new(),
            created: 1000,
            updated: 2000,
            model_override: None,
            effort_override: None,
            session_ledger: None,
            continuity_handle: None,
            last_token_usage: Some(StoredTokenUsage {
                input_tokens: 1500,
                cache_creation_input_tokens: 3000,
                cache_read_input_tokens: 42_000,
                output_tokens: 200,
            }),
        };
        let json = serde_json::to_string(&stored).unwrap();
        let back: StoredConversation = serde_json::from_str(&json).unwrap();
        let u = back.last_token_usage.expect("usage present");
        assert_eq!(u.input_tokens, 1500);
        assert_eq!(u.cache_creation_input_tokens, 3000);
        assert_eq!(u.cache_read_input_tokens, 42_000);
        assert_eq!(u.output_tokens, 200);
    }

    #[test]
    fn t1_stored_conversation_forward_compatible_without_token_usage() {
        // A record written pre-T1 (no `last_token_usage` field) must
        // deserialize cleanly with `None`. Same forward-compat pattern as
        // the M4 ledger field.
        let json = r#"{
            "id":"c","title":"T","messages":[],
            "created":1,"updated":2
        }"#;
        let stored: StoredConversation = serde_json::from_str(json).unwrap();
        assert!(stored.last_token_usage.is_none());
    }
}
