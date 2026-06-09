//! Tool registry + executors for the in-process agent loop
//! (DeepSeek plan Units 6–7).
//!
//! Tools mirror Claude Code's schema (names + argument keys) so the chat
//! "Using X…" summaries and the system prompt reuse with zero new code. PR-3
//! ships read-only fs tools; PR-4: `Write`/`Edit`/`MultiEdit`; PR-5: `Bash` +
//! [`crate::agent_session::tool_agent::policy::ToolPolicy`]. MCP graph tools
//! plug in once `MemoryStores`/`GraphStore` are threaded into the session.

pub mod bash;
pub mod glob;
pub mod grep;
pub mod read;
pub mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::Mutex;

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::agent_session::tool_agent::policy::ToolPolicy;
use crate::agent_session::tool_agent::snapshot::TurnSnapshot;
use crate::observer::AcpObserver;
use crate::scope_enforcer::ScopeEnforcer;
use crate::types::FileScope;

/// Result of running a tool. `content` is fed back to the model as the `tool`
/// message; `is_error` is advisory (logging / future telemetry).
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            content: format!("Error: {}", msg.into()),
            is_error: true,
        }
    }
}

/// Execution context shared by all tools for one turn.
pub struct ToolCtx {
    pub workspace_root: PathBuf,
    pub additional_roots: Vec<PathBuf>,
    /// Active file scope. Default (empty) = no restriction (chat). Swarm passes
    /// the work unit's `owned_paths` in Phase 6.
    pub scope: FileScope,
    /// Per-turn snapshot for Option-B writes. `None` for read-only tool sets.
    pub snapshot: Option<Arc<Mutex<TurnSnapshot>>>,
    /// Bash permission policy (allowlist / denylist / timeout).
    pub policy: ToolPolicy,
    /// Per-turn `/autoapprove` — bypasses Bash permission prompts (not denylist).
    pub auto_approve: bool,
    /// Observer for Bash permission prompts. Required for the `Bash` tool.
    pub observer: Option<Arc<dyn AcpObserver>>,
}

impl ToolCtx {
    /// Resolve + confine a path argument to the workspace roots.
    pub fn confine(&self, arg: &str) -> Result<PathBuf> {
        confine(arg, &self.workspace_root, &self.additional_roots)
    }

    /// Sensitive-file read check (reuses [`ScopeEnforcer::check_read`]).
    pub fn check_read(&self, path: &Path) -> Result<()> {
        ScopeEnforcer::new(self.scope.clone())
            .check_read(path)
            .map_err(|v| anyhow!(v.to_string()))
    }

    /// Write-scope check (reuses [`ScopeEnforcer::check_write`]).
    pub fn check_write(&self, path: &Path) -> Result<()> {
        ScopeEnforcer::new(self.scope.clone())
            .check_write(path)
            .map_err(|v| anyhow!(v.to_string()))
    }
}

/// A model-callable tool.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    /// OpenAI function-tool schema: `{ "type": "function", "function": { … } }`.
    fn schema(&self) -> Value;
    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome;
}

/// Holds the active tool set and emits the `tools` array sent to the API.
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<Box<dyn Tool>>) -> Self {
        Self { tools }
    }

    /// The default read-only filesystem tool set (PR-3).
    pub fn read_only_fs() -> Self {
        Self::new(vec![
            Box::new(read::ReadTool),
            Box::new(grep::GrepTool),
            Box::new(glob::GlobTool),
        ])
    }

    /// Read-only tools plus `Write`/`Edit`/`MultiEdit` (PR-4).
    pub fn with_writes() -> Self {
        Self::new(vec![
            Box::new(read::ReadTool),
            Box::new(grep::GrepTool),
            Box::new(glob::GlobTool),
            Box::new(write::WriteTool),
            Box::new(write::EditTool),
            Box::new(write::MultiEditTool),
        ])
    }

    /// Full chat tool set: filesystem read/write + `Bash` (PR-5).
    pub fn full_chat() -> Self {
        Self::new(vec![
            Box::new(read::ReadTool),
            Box::new(grep::GrepTool),
            Box::new(glob::GlobTool),
            Box::new(write::WriteTool),
            Box::new(write::EditTool),
            Box::new(write::MultiEditTool),
            Box::new(bash::BashTool),
        ])
    }

    /// Build a registry from an allow-list of tool names (swarm `allowed_tools`).
    pub fn from_names(names: &[String]) -> Self {
        let mut tools: Vec<Box<dyn Tool>> = Vec::new();
        for name in names {
            match name.as_str() {
                "Read" => tools.push(Box::new(read::ReadTool)),
                "Glob" => tools.push(Box::new(glob::GlobTool)),
                "Grep" => tools.push(Box::new(grep::GrepTool)),
                "Write" => tools.push(Box::new(write::WriteTool)),
                "Edit" => tools.push(Box::new(write::EditTool)),
                "MultiEdit" => tools.push(Box::new(write::MultiEditTool)),
                "Bash" => tools.push(Box::new(bash::BashTool)),
                other => tracing::warn!("unknown swarm tool name '{other}', skipping"),
            }
        }
        Self::new(tools)
    }

    pub fn schemas(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.schema()).collect()
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|b| b.as_ref())
    }
}

/// Lexically normalize + confine `arg` to the given roots. Rejects paths that
/// escape every root (absolute paths outside, `..` traversal). Filesystem is
/// not touched, so this works for not-yet-existing paths (Glob/Write).
pub fn confine(arg: &str, workspace_root: &Path, additional_roots: &[PathBuf]) -> Result<PathBuf> {
    let raw = Path::new(arg);
    let abs = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace_root.join(raw)
    };
    let norm = normalize_lexically(&abs);

    let mut roots = vec![normalize_lexically(workspace_root)];
    roots.extend(additional_roots.iter().map(|r| normalize_lexically(r)));

    if roots.iter().any(|r| norm.starts_with(r)) {
        Ok(norm)
    } else {
        Err(anyhow!("path '{}' escapes the workspace", arg))
    }
}

/// Resolve `.` and `..` components without touching the filesystem.
fn normalize_lexically(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Directories the read-only tools never descend into.
pub(crate) fn is_ignored_dir(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir()
        && matches!(
            entry.file_name().to_str(),
            Some(".git" | "target" | "node_modules" | ".gaviero")
        )
}

/// Compile a glob (`*`, `**`, `?`) into an anchored regex over a forward-slash
/// path. `*` does not cross `/`; `**` does. Offline stand-in for the `globset`
/// crate (not in the lockfile); gitignore-aware matching via `ignore` is a
/// follow-up.
pub(crate) fn glob_to_regex(glob: &str) -> std::result::Result<regex::Regex, String> {
    let chars: Vec<char> = glob.chars().collect();
    let mut re = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    re.push_str(".*");
                    i += 1;
                    // Swallow a trailing slash so `**/x` also matches `x`.
                    if i + 1 < chars.len() && chars[i + 1] == '/' {
                        i += 1;
                    }
                } else {
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push_str("[^/]"),
            c @ ('.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '[' | ']' | '\\') => {
                re.push('\\');
                re.push(c);
            }
            c => re.push(c),
        }
        i += 1;
    }
    re.push('$');
    regex::Regex::new(&re).map_err(|e| format!("invalid glob '{glob}': {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confine_allows_within_root() {
        let root = Path::new("/ws");
        let p = confine("src/main.rs", root, &[]).unwrap();
        assert_eq!(p, Path::new("/ws/src/main.rs"));
    }

    #[test]
    fn confine_rejects_parent_escape() {
        let root = Path::new("/ws");
        assert!(confine("../etc/passwd", root, &[]).is_err());
        assert!(confine("/etc/passwd", root, &[]).is_err());
    }

    #[test]
    fn confine_allows_additional_root() {
        let root = Path::new("/ws");
        let extra = vec![PathBuf::from("/other")];
        let p = confine("/other/lib.rs", root, &extra).unwrap();
        assert_eq!(p, Path::new("/other/lib.rs"));
    }

    #[test]
    fn glob_double_star_crosses_slashes() {
        let re = glob_to_regex("**/*.rs").unwrap();
        assert!(re.is_match("src/a.rs"));
        assert!(re.is_match("a.rs"));
        assert!(!re.is_match("a.txt"));
    }

    #[test]
    fn glob_single_star_does_not_cross_slash() {
        let re = glob_to_regex("*.rs").unwrap();
        assert!(re.is_match("a.rs"));
        assert!(!re.is_match("src/a.rs"));
    }
}
