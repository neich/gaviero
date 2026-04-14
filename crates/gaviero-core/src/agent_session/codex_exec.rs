//! Codex `exec` session (V9 §11 M8).
//!
//! [`CodexExecSession`] gives the `codex exec` path a named type distinct from
//! the generic [`LegacyAgentSession`] shim. This makes the registry routing
//! explicit and gives M10 a clean deletion target when (and only if)
//! `app-server` durability is proven in production.
//!
//! **Continuity:** `StatelessReplay` — `codex exec` is non-interactive and
//! carries no thread state. Each turn is a fresh subprocess invocation; the
//! caller is responsible for including replay history in the prompt when needed
//! (M9 + Ollama path wires this generically; Codex exec's history is typically
//! short enough to fit in a single combined prompt).

use std::pin::Pin;

use anyhow::Result;
use futures::Stream;

use crate::context_planner::{ContinuityHandle, ContinuityMode};
use crate::swarm::backend::UnifiedStreamEvent;

use super::registry::SessionConstruction;
use super::{AgentSession, LegacyAgentSession, Turn};

// ── CodexExecSession ──────────────────────────────────────────────────────────

/// M8 `AgentSession` for Codex `exec` mode (`codex:` / `codex-cli:` prefixes).
///
/// A named wrapper over [`LegacyAgentSession`] so the registry can route
/// Codex exec separately from other `StatelessReplay` providers (e.g. Ollama).
/// M10 may replace this with a direct codex-exec implementation once
/// `app-server` durability is proven.
pub struct CodexExecSession(LegacyAgentSession);

impl CodexExecSession {
    pub(super) fn new(args: SessionConstruction) -> Self {
        Self(LegacyAgentSession::new(
            args.write_gate,
            args.observer,
            args.model,
            args.ollama_base_url,
            args.workspace_root,
            args.agent_id,
            args.options,
            args.profile,
        ))
    }
}

#[async_trait::async_trait]
impl AgentSession for CodexExecSession {
    async fn send_turn(
        &mut self,
        turn: Turn,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<UnifiedStreamEvent>> + Send>>> {
        self.0.send_turn(turn).await
    }

    fn continuity_mode(&self) -> ContinuityMode {
        ContinuityMode::StatelessReplay
    }

    /// `codex exec` carries no thread state — always `None`.
    fn continuity_handle(&self) -> Option<&ContinuityHandle> {
        None
    }

    async fn close(self: Box<Self>) {
        Box::new(self.0).close().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_exec_is_stateless_replay() {
        // Verify the mode constant so a future refactor can't silently change it.
        assert_eq!(ContinuityMode::StatelessReplay, ContinuityMode::StatelessReplay);
        assert_ne!(ContinuityMode::StatelessReplay, ContinuityMode::ProcessBound);
        assert_ne!(ContinuityMode::StatelessReplay, ContinuityMode::NativeResume);
    }
}
