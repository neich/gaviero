//! Transport-session registry (V9 §11 M5).
//!
//! Small façade that picks an [`AgentSession`] implementation for a given
//! [`ProviderProfile`]. In M5 all providers resolve to [`LegacyAgentSession`];
//! later milestones swap entries per provider:
//!
//! * M6 — Claude returns `ClaudeSession`.
//! * M8 — Codex `app-server` returns `CodexSession`.
//! * M9 — Ollama returns `OllamaSession`.
//!
//! Keeping this in its own module lets the chat/swarm callers construct a
//! session by profile without knowing the transport implementation —
//! exactly what V9 §3 architectural principle 9 ("planner-side and
//! transport-side types in separate modules with a named conversion
//! boundary") prescribes.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::acp::session::AgentOptions;
use crate::context_planner::ProviderProfile;
use crate::observer::AcpObserver;
use crate::write_gate::WriteGatePipeline;

use crate::context_planner::ContinuityMode;

use super::{AgentSession, LegacyAgentSession};
use super::claude::ClaudeSession;
use super::codex_app_server::CodexAppServerSession;
use super::codex_exec::CodexExecSession;

/// Inputs the shim needs. Named struct (not positional args) so adding a
/// per-session field is additive; new providers drop the ones they don't
/// consume.
pub struct SessionConstruction {
    pub write_gate: Arc<Mutex<WriteGatePipeline>>,
    pub observer: Box<dyn AcpObserver>,
    pub model: String,
    pub ollama_base_url: Option<String>,
    pub workspace_root: PathBuf,
    pub agent_id: String,
    pub options: AgentOptions,
    pub profile: ProviderProfile,
}

/// Pick a transport session for the given profile.
///
/// Matches on `ContinuityMode` + `provider` so each provider gets an
/// independent session type with a named deletion target for M10 cleanup.
///
/// M5: introduced; all providers resolved to `LegacyAgentSession`.
/// M6: `NativeResume` (Claude) returns `ClaudeSession`.
/// M8: `ProcessBound` (Codex app-server) returns `CodexAppServerSession`;
///     `StatelessReplay` Codex exec returns `CodexExecSession`.
/// M9: remaining `StatelessReplay` (Ollama) will return `OllamaSession`.
pub fn create_session(args: SessionConstruction) -> Box<dyn AgentSession> {
    match args.profile.continuity_mode {
        ContinuityMode::NativeResume => {
            // M6: Claude — per-provider session owns the subprocess lifecycle.
            Box::new(ClaudeSession::new(args))
        }
        ContinuityMode::ProcessBound => {
            // M8: Codex app-server — keeps the subprocess alive across turns.
            // Only Codex uses ProcessBound in M8; future providers (if any)
            // would add arms here before M10.
            Box::new(CodexAppServerSession::new(args))
        }
        ContinuityMode::StatelessReplay => {
            if args.profile.provider == "codex" {
                // M8: `codex exec` — named type distinct from the generic shim.
                Box::new(CodexExecSession::new(args))
            } else {
                // M9: Ollama and other StatelessReplay providers still use the
                // legacy shim until OllamaSession lands.
                Box::new(LegacyAgentSession::new(
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
    }
}
