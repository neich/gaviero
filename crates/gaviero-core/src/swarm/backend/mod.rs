//! Provider-agnostic AI backend abstraction.
//!
//! All backends implement the [`AgentBackend`] trait and produce a normalized
//! [`UnifiedStreamEvent`] stream. This replaces the old dual-dispatch pattern
//! (separate code paths for Claude Code subprocess and Ollama HTTP).

pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod deepseek;
pub mod dsh;
pub mod executor;
pub mod mock;
pub mod ollama;
pub mod runner;
pub mod shared;

use crate::types::FileScope;
use crate::write_gate::WriteGatePipeline;

use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Trait ────────────────────────────────────────────────────────────────────

/// A provider-agnostic AI completion backend.
///
/// Implementations convert provider-specific protocols (NDJSON subprocess,
/// HTTP SSE, etc.) into a unified [`UnifiedStreamEvent`] stream.
#[async_trait::async_trait]
pub trait AgentBackend: Send + Sync {
    /// Stream a completion. Returns a stream of normalized events.
    async fn stream_completion(
        &self,
        request: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>>;

    /// Runtime capability reporting.
    fn capabilities(&self) -> Capabilities;

    /// Human-readable backend name (e.g. "claude:sonnet", "ollama:qwen2.5-coder:7b").
    fn name(&self) -> &str;

    /// Health check. Returns `Ok(())` if the backend is reachable.
    async fn health_check(&self) -> Result<()>;
}

// ── Unified Stream Events ───────────────────────────────────────────────────

/// Normalized streaming event emitted by all backends.
#[derive(Debug, Clone, PartialEq)]
pub enum UnifiedStreamEvent {
    /// Incremental text from the model response.
    TextDelta(String),
    /// Incremental thinking/reasoning text.
    ThinkingDelta(String),
    /// Agent started a tool call. `args` carries the parsed arguments when the
    /// backend can supply them at start time (Cursor, Codex `commandExecution`);
    /// Claude populates them once `AssistantMessage::tool_uses` lands. Stays
    /// `Value::Null` when args were not available.
    ToolCallStart {
        id: String,
        name: String,
        args: Value,
    },
    /// Incremental JSON arguments for a tool call.
    ToolCallDelta { id: String, args_chunk: String },
    /// Tool call arguments are complete.
    ToolCallEnd { id: String },
    /// A complete `<file path="...">content</file>` block was detected.
    FileBlock { path: PathBuf, content: String },
    /// Paths written directly on disk by an in-process tool-agent (Option-B).
    /// The runner records these for validation without re-writing through the gate.
    PathsModified(Vec<PathBuf>),
    /// Token usage / cost information.
    Usage(TokenUsage),
    /// Non-fatal error during streaming.
    Error(String),
    /// Stream is complete.
    Done(StopReason),
}

/// Token usage and cost metadata.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
}

/// Why the stream ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    Error,
    Timeout,
}

// ── Capabilities ────────────────────────────────────────────────────────────

/// Which read-only gaviero MCP retrieval tools are live for a session
/// (PUSH→PULL Phase 1).
///
/// Drives the retrieval-protocol ("pull") stanza in
/// [`shared::default_editor_system_prompt`], which must name only tools that
/// are actually wired. The default is all-false → no stanza, so behavior is
/// unchanged until a backend opts in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetrievalToolset {
    /// `memory_search` / `blast_radius` / `node_doc` are wired. These are the
    /// always-on gaviero MCP tools (present for every subprocess provider the
    /// host wires the gaviero MCP server for: Claude, Codex, Cursor).
    pub graph_and_memory: bool,
    /// `symbol_search` / `symbol_doc` are wired. These require the enrichment
    /// sidecar (`repoMap.symbolEnrichment.enabled`, off by default), so this
    /// stays false unless enrichment is on.
    pub symbols: bool,
    /// Explicit MCP tool names that should appear in the retrieval stanza.
    /// Empty means "derive names from `graph_and_memory` / `symbols`".
    pub exposed: Vec<String>,
}

impl RetrievalToolset {
    /// Build the stanza flags from `mcp.gavieroServer.exposedTools`.
    pub fn from_exposed(tools: &[String]) -> Self {
        let has = |n: &str| tools.iter().any(|t| t == n);
        Self {
            graph_and_memory: has("memory_search")
                || has("blast_radius")
                || has("node_doc")
                || has("memory_get")
                || has("repo_outline"),
            symbols: has("symbol_search") || has("symbol_doc"),
            exposed: tools.to_vec(),
        }
    }

    fn names(&self, tool: &str) -> bool {
        if !self.exposed.is_empty() {
            return self.exposed.iter().any(|t| t == tool);
        }
        match tool {
            "node_doc" | "blast_radius" | "memory_search" => self.graph_and_memory,
            "symbol_search" | "symbol_doc" => self.symbols,
            _ => false,
        }
    }
}

impl Capabilities {
    /// Overlay `mcp.gavieroServer.exposedTools` onto this capability set.
    pub fn with_exposed_tools(mut self, tools: Option<&[String]>) -> Self {
        if let Some(t) = tools
            && (self.retrieval.graph_and_memory || self.retrieval.symbols || !self.retrieval.exposed.is_empty())
        {
            self.retrieval = RetrievalToolset::from_exposed(t);
        }
        self
    }
}

/// Runtime capability flags for a backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub tool_use: bool,
    pub streaming: bool,
    pub vision: bool,
    pub extended_thinking: bool,
    pub max_context_tokens: usize,
    pub supports_system_prompt: bool,
    /// Whether the backend can produce `<file>` blocks in its output.
    pub supports_file_blocks: bool,
    /// Which read-only retrieval tools are live (drives the pull stanza).
    pub retrieval: RetrievalToolset,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            tool_use: false,
            streaming: false,
            vision: false,
            extended_thinking: false,
            max_context_tokens: 0,
            supports_system_prompt: false,
            supports_file_blocks: false,
            retrieval: RetrievalToolset::default(),
        }
    }
}

// ── Completion Request ──────────────────────────────────────────────────────

/// Provider-agnostic completion request.
#[derive(Debug, Clone, Default)]
pub struct CompletionRequest {
    /// The user/task prompt.
    pub prompt: String,
    /// Optional system prompt (prepended or sent as system message).
    pub system_prompt: Option<String>,
    /// Workspace root directory (used by subprocess backends for --add-dir).
    pub workspace_root: PathBuf,
    /// Additional workspace folders the agent should be able to read/write,
    /// in workspace-mode multi-folder setups. Subprocess backends emit one
    /// `--add-dir` per entry on top of `workspace_root`. Empty for single-folder
    /// workspaces and for swarm sub-agents (which run in per-agent worktrees).
    pub additional_roots: Vec<PathBuf>,
    /// Tools the agent is allowed to use (e.g. ["Read", "Write", "Edit"]).
    pub allowed_tools: Vec<String>,
    /// Files to attach (images, documents) via CLI flags.
    pub file_attachments: Vec<PathBuf>,
    /// Conversation history as (role, content) pairs.
    pub conversation_history: Vec<(String, String)>,
    /// Referenced file contents as (path, content) pairs.
    pub file_refs: Vec<(String, String)>,
    /// Optional effort / reasoning level.
    pub effort: Option<String>,
    /// Provider-specific pass-through parameters from the DSL
    /// `client { extra { ... } }` block. Each backend consumes the keys it
    /// understands and logs the rest at `tracing::debug` level.
    pub extra: Vec<(String, String)>,
    /// Optional max output tokens.
    pub max_tokens: Option<u32>,
    /// Whether the backend should auto-approve provider permission prompts.
    pub auto_approve: bool,
    /// When true, sets `CLAUDE_QUIET=1` on Claude subprocess spawns so
    /// global Stop hooks skip machine-consumed turns.
    pub suppress_hooks: bool,
    /// Owned-path scope for in-process tool-agent backends (swarm work units).
    pub file_scope: FileScope,
    /// Shell policy (`agent.permissions.bash` + approved tools) resolved by
    /// the host from the workspace cascade. Consumed by in-process
    /// tool-agent backends; subprocess backends get the same lists through
    /// the synthesized provider configs. `None` → resolve from
    /// `workspace_root` (finds nothing inside a swarm worktree).
    pub tool_policy: Option<crate::agent_session::tool_agent::policy::ToolPolicy>,
    /// MCP tools advertised to the agent (`mcp.gavieroServer.exposedTools`).
    /// `None` keeps the backend's default retrieval stanza (full graph + memory).
    pub exposed_tools: Option<Vec<String>>,
    /// Write gate for backends that propose mid-stream (`dsh:` ACP fs writes).
    /// `None` for backends that only emit `FileBlock` / `PathsModified`.
    pub write_gate: Option<WriteGateHandle>,
}

/// Cloneable write-gate pointer that is `Debug` without dumping pipeline state.
#[derive(Clone)]
pub struct WriteGateHandle(pub Arc<tokio::sync::Mutex<WriteGatePipeline>>);

impl std::fmt::Debug for WriteGateHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WriteGateHandle")
    }
}

// ── Backend Config ──────────────────────────────────────────────────────────

/// Serializable backend configuration. A factory function maps this to
/// a `Box<dyn AgentBackend>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum BackendConfig {
    ClaudeCode {
        model: Option<String>,
    },
    Codex {
        model: Option<String>,
    },
    Cursor {
        model: Option<String>,
    },
    Ollama {
        model: String,
        base_url: Option<String>,
    },
    /// In-process API tool-agent harness (`deepseek:` today).
    Deepseek {
        model: String,
    },
    /// Subprocess `dsh-acp` over the Agent Client Protocol (`dsh:`).
    Dsh {
        model: String,
    },
    Custom {
        command: String,
        args: Vec<String>,
    },
}

/// Create a backend from a serializable config.
pub fn create_backend(config: &BackendConfig) -> Result<Box<dyn AgentBackend>> {
    match config {
        BackendConfig::ClaudeCode { model } => {
            let m = model.as_deref().unwrap_or("sonnet");
            Ok(Box::new(claude_code::ClaudeCodeBackend::new(m)))
        }
        BackendConfig::Codex { model } => {
            let m = model.as_deref().unwrap_or("gpt-5.5");
            Ok(Box::new(codex::CodexBackend::new(m)))
        }
        BackendConfig::Cursor { model } => {
            let m = model.as_deref().unwrap_or(cursor::DEFAULT_CURSOR_MODEL);
            Ok(Box::new(cursor::CursorBackend::new(m)))
        }
        BackendConfig::Ollama { model, base_url } => {
            let url = base_url.as_deref().unwrap_or("http://localhost:11434");
            Ok(Box::new(ollama::OllamaStreamBackend::new(url, model)))
        }
        BackendConfig::Deepseek { model } => Ok(Box::new(deepseek::DeepseekBackend::new(model))),
        BackendConfig::Dsh { model } => Ok(Box::new(dsh::DshBackend::new(model))),
        BackendConfig::Custom { command, args } => {
            anyhow::bail!(
                "Custom backend not yet implemented (command={}, args={:?})",
                command,
                args,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    // Test 1: MockBackend event sequence (trait contract)
    #[tokio::test]
    async fn test_mock_backend_event_sequence() {
        let events = vec![
            UnifiedStreamEvent::TextDelta("Hello ".into()),
            UnifiedStreamEvent::TextDelta("world".into()),
            UnifiedStreamEvent::FileBlock {
                path: PathBuf::from("src/main.rs"),
                content: "fn main() {}".into(),
            },
            UnifiedStreamEvent::Done(StopReason::EndTurn),
        ];
        let backend = mock::MockBackend::new("test-mock", events);

        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            additional_roots: vec![],
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
            suppress_hooks: true,
            file_scope: FileScope::default(),
            tool_policy: None,
        exposed_tools: None,
            write_gate: None,
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        let mut collected = Vec::new();
        while let Some(event) = stream.next().await {
            collected.push(event.unwrap());
        }

        assert_eq!(collected.len(), 4);
        assert_eq!(collected[0], UnifiedStreamEvent::TextDelta("Hello ".into()));
        assert_eq!(collected[1], UnifiedStreamEvent::TextDelta("world".into()));
        assert!(
            matches!(&collected[2], UnifiedStreamEvent::FileBlock { path, .. } if path == &PathBuf::from("src/main.rs"))
        );
        assert_eq!(collected[3], UnifiedStreamEvent::Done(StopReason::EndTurn));
    }

    // Test 2: Trait object dynamic dispatch (Box<dyn AgentBackend>)
    #[tokio::test]
    async fn test_trait_object_dynamic_dispatch() {
        let events = vec![
            UnifiedStreamEvent::TextDelta("hi".into()),
            UnifiedStreamEvent::Done(StopReason::EndTurn),
        ];
        let backend: Box<dyn AgentBackend> = Box::new(mock::MockBackend::new("boxed", events));

        assert_eq!(backend.name(), "boxed");
        assert!(backend.health_check().await.is_ok());

        let req = CompletionRequest {
            prompt: "test".into(),
            system_prompt: None,
            workspace_root: PathBuf::from("/tmp"),
            additional_roots: vec![],
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
            suppress_hooks: true,
            file_scope: FileScope::default(),
            tool_policy: None,
        exposed_tools: None,
            write_gate: None,
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first, UnifiedStreamEvent::TextDelta("hi".into()));
    }

    // Test 3: Capabilities construction (all-false default, all-true)
    #[test]
    fn test_capabilities_construction() {
        let default = Capabilities::default();
        assert!(!default.tool_use);
        assert!(!default.streaming);
        assert!(!default.vision);
        assert!(!default.extended_thinking);
        assert_eq!(default.max_context_tokens, 0);
        assert!(!default.supports_system_prompt);
        assert!(!default.supports_file_blocks);
        assert_eq!(default.retrieval, RetrievalToolset::default());
        assert!(!default.retrieval.graph_and_memory);

        let full = Capabilities {
            tool_use: true,
            streaming: true,
            vision: true,
            extended_thinking: true,
            max_context_tokens: 200_000,
            supports_system_prompt: true,
            supports_file_blocks: true,
            retrieval: RetrievalToolset {
                graph_and_memory: true,
                symbols: true,
                exposed: vec![],
            },
        };
        assert!(full.tool_use);
        assert_eq!(full.max_context_tokens, 200_000);
        assert!(full.retrieval.graph_and_memory);
    }

    // Test 4: BackendConfig serde round-trip
    #[test]
    fn test_backend_config_serde_roundtrip() {
        // ClaudeCode variant
        let cc = BackendConfig::ClaudeCode {
            model: Some("sonnet".into()),
        };
        let json = serde_json::to_string(&cc).unwrap();
        let parsed: BackendConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(cc, parsed);

        // Ollama variant
        let ol = BackendConfig::Ollama {
            model: "qwen2.5-coder:7b".into(),
            base_url: None,
        };
        let json = serde_json::to_string(&ol).unwrap();
        let parsed: BackendConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(ol, parsed);

        // Custom variant
        let custom = BackendConfig::Custom {
            command: "my-agent".into(),
            args: vec!["--fast".into()],
        };
        let json = serde_json::to_string(&custom).unwrap();
        let parsed: BackendConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(custom, parsed);

        // Unknown type errors
        let bad_json = r#"{"type":"Unknown","foo":"bar"}"#;
        assert!(serde_json::from_str::<BackendConfig>(bad_json).is_err());
    }

    // Test 5: Factory produces correct backends
    #[test]
    fn test_factory_produces_correct_backends() {
        let cc = BackendConfig::ClaudeCode {
            model: Some("sonnet".into()),
        };
        let backend = create_backend(&cc).unwrap();
        assert!(backend.name().contains("claude"));

        // Ollama backend
        let ol = BackendConfig::Ollama {
            model: "qwen".into(),
            base_url: None,
        };
        let ol_backend = create_backend(&ol).unwrap();
        assert!(ol_backend.name().contains("ollama"));

        let ds = BackendConfig::Deepseek {
            model: "deepseek-v4-pro".into(),
        };
        let ds_backend = create_backend(&ds).unwrap();
        assert!(ds_backend.name().contains("deepseek"));

        let dsh = BackendConfig::Dsh {
            model: "deepseek-v4-flash".into(),
        };
        let dsh_backend = create_backend(&dsh).unwrap();
        assert!(dsh_backend.name().contains("dsh"));
        assert!(!dsh_backend.capabilities().supports_file_blocks);

        // Custom not yet implemented — returns error
        let custom = BackendConfig::Custom {
            command: "foo".into(),
            args: vec![],
        };
        assert!(create_backend(&custom).is_err());
    }
}
