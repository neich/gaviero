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
/// Matches on `ContinuityMode` rather than provider string so adding a
/// provider with an existing continuity behavior doesn't require a
/// registry change — the planner only cares about the mode anyway.
///
/// M5: introduced; all providers resolved to `LegacyAgentSession`.
/// M6: `NativeResume` (Claude) now returns `ClaudeSession`.
/// M8: `ProcessBound` (Codex app-server) will return `CodexSession`.
/// M9: remaining `StatelessReplay` (Ollama) will return `OllamaSession`.
pub fn create_session(args: SessionConstruction) -> Box<dyn AgentSession> {
    match args.profile.continuity_mode {
        ContinuityMode::NativeResume => {
            // M6: Claude — per-provider session owns the subprocess lifecycle.
            Box::new(ClaudeSession::new(args))
        }
        ContinuityMode::ProcessBound | ContinuityMode::StatelessReplay => {
            // M8/M9: Codex and Ollama still use the legacy shim until their
            // per-provider sessions land.
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
