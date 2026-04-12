//! Provider-agnostic AI backend abstraction.
//!
//! All backends implement the [`AgentBackend`] trait and produce a normalized
//! [`UnifiedStreamEvent`] stream. This replaces the old dual-dispatch pattern
//! (separate code paths for Claude Code subprocess and Ollama HTTP).

pub mod claude_code;
pub mod executor;
pub mod mock;
pub mod ollama;
pub mod runner;
pub mod shared;

use std::path::PathBuf;
use std::pin::Pin;

use anyhow::Result;
use futures::Stream;
use serde::{Deserialize, Serialize};

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

    /// Human-readable backend name (e.g. "claude-code:sonnet", "ollama:qwen2.5-coder:7b").
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
    /// Agent started a tool call.
    ToolCallStart { id: String, name: String },
    /// Incremental JSON arguments for a tool call.
    ToolCallDelta { id: String, args_chunk: String },
    /// Tool call arguments are complete.
    ToolCallEnd { id: String },
    /// A complete `<file path="...">content</file>` block was detected.
    FileBlock { path: PathBuf, content: String },
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
        }
    }
}

// ── Completion Request ──────────────────────────────────────────────────────

/// Provider-agnostic completion request.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    /// The user/task prompt.
    pub prompt: String,
    /// Optional system prompt (prepended or sent as system message).
    pub system_prompt: Option<String>,
    /// Workspace root directory (used by subprocess backends for --add-dir).
    pub workspace_root: PathBuf,
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
    /// Optional max output tokens.
    pub max_tokens: Option<u32>,
    /// Whether the backend should auto-approve provider permission prompts.
    pub auto_approve: bool,
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
    Ollama {
        model: String,
        base_url: Option<String>,
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
        BackendConfig::Ollama { model, base_url } => {
            let url = base_url
                .as_deref()
                .unwrap_or("http://localhost:11434");
            Ok(Box::new(ollama::OllamaStreamBackend::new(url, model)))
        }
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
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            max_tokens: None,
            auto_approve: true,
        };

        let mut stream = backend.stream_completion(req).await.unwrap();
        let mut collected = Vec::new();
        while let Some(event) = stream.next().await {
            collected.push(event.unwrap());
        }

        assert_eq!(collected.len(), 4);
        assert_eq!(collected[0], UnifiedStreamEvent::TextDelta("Hello ".into()));
        assert_eq!(collected[1], UnifiedStreamEvent::TextDelta("world".into()));
        assert!(matches!(&collected[2], UnifiedStreamEvent::FileBlock { path, .. } if path == &PathBuf::from("src/main.rs")));
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
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            max_tokens: None,
            auto_approve: true,
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

        let full = Capabilities {
            tool_use: true,
            streaming: true,
            vision: true,
            extended_thinking: true,
            max_context_tokens: 200_000,
            supports_system_prompt: true,
            supports_file_blocks: true,
        };
        assert!(full.tool_use);
        assert_eq!(full.max_context_tokens, 200_000);
    }

    // Test 4: BackendConfig serde round-trip
    #[test]
    fn test_backend_config_serde_roundtrip() {
        // ClaudeCode variant
        let cc = BackendConfig::ClaudeCode { model: Some("sonnet".into()) };
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
        let cc = BackendConfig::ClaudeCode { model: Some("sonnet".into()) };
        let backend = create_backend(&cc).unwrap();
        assert!(backend.name().contains("claude"));

        // Ollama backend
        let ol = BackendConfig::Ollama {
            model: "qwen".into(),
            base_url: None,
        };
        let ol_backend = create_backend(&ol).unwrap();
        assert!(ol_backend.name().contains("ollama"));

        // Custom not yet implemented — returns error
        let custom = BackendConfig::Custom {
            command: "foo".into(),
            args: vec![],
        };
        assert!(create_backend(&custom).is_err());
    }
}
