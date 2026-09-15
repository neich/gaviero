//! Planner-side types (M1 of PROVIDER_PLAN_V9).
//!
//! This module owns the type vocabulary the [`ContextPlanner`] consumes and
//! produces. Transport-side types (`Turn`, `AgentSession`) live in
//! `agent_session/` and are introduced in M5 — see V9 §0 rule 12 and §4 type
//! ownership table. Do not define those here.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// V9 §4 candidate types are owned by the modules that produce them
// (`MemoryStore`, `RepoMap`, `GraphStore`) to avoid module cycles back
// into `context_planner/`. Re-export here so consumers see V9's spec
// home — `crate::context_planner::types::{MemoryCandidate, ..}`.
pub use crate::memory::store::MemoryCandidate;
pub use crate::repo_map::store::ImpactSummary;
pub use crate::repo_map::{GraphCandidate, GraphConfidence};

/// How a provider preserves model-side state between turns.
///
/// Maps to V9 §5 provider table:
/// * `NativeResume` — Claude `--resume`; opaque server-side state survives subprocess exit.
/// * `ProcessBound` — Codex `app-server`; alive while the subprocess lives.
/// * `StatelessReplay` — Ollama, `codex exec`; client must replay history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContinuityMode {
    NativeResume,
    ProcessBound,
    StatelessReplay,
}

/// Typed provider-specific continuity state.
///
/// Variants carry provider identity, so persisted state is self-describing.
/// Adding a provider is a one-variant addition with no migration cost to
/// existing records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum ContinuityHandle {
    ClaudeSessionId(String),
    CodexThreadId(String),
    /// Cursor CLI session id captured from the `system.init` event. The
    /// chat path passes it back via `--resume <id>` on subsequent turns
    /// once the Cursor session is promoted to `NativeResume` in a later
    /// milestone; today (phase 1) the field round-trips through persisted
    /// `StoredConversation` records but is not consumed by the session.
    CursorThreadId(String),
    /// ACP `session/new` id for `dsh:` (and future ACP-native agents).
    AcpSessionId(String),
    // Future providers: add a variant here.
}

/// Bootstrap strategy tier for a provider (PUSH→PULL plan, Phase 0).
///
/// Selects how much repository context the pre-prompt assembler pushes on the
/// first turn:
/// * `Strong` — tool-capable, large-context providers. Later phases inject a
///   thin orientation anchor and let the model *pull* specifics through the
///   read-only MCP tools.
/// * `SmallLocal` — providers without reliable tool use or with a small
///   context window. They keep the full push until per-tier evidence proves a
///   thinner bootstrap is non-inferior.
///
/// The per-arm value in [`build_provider_profile`] is written as an explicit
/// literal so adding a provider forces a conscious choice, but the literal
/// must equal [`BootstrapTier::derive`] for that provider's capabilities —
/// `build_provider_profile_sets_tier` pins every arm to the rule so a literal
/// cannot silently diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapTier {
    Strong,
    SmallLocal,
}

impl BootstrapTier {
    /// Canonical derivation rule (PUSH→PULL plan, Phase 0): a provider is
    /// `SmallLocal` iff it lacks tool use or its context window is under 32k
    /// tokens; otherwise `Strong`. This is the single source of truth the
    /// per-arm literals in [`build_provider_profile`] are checked against, and
    /// the seam a later phase's `resolve_bootstrap_tier` builds on.
    pub fn derive(supports_tool_use: bool, max_context_tokens: Option<usize>) -> Self {
        if !supports_tool_use || max_context_tokens.is_some_and(|c| c < 32_000) {
            BootstrapTier::SmallLocal
        } else {
            BootstrapTier::Strong
        }
    }
}

/// Provider capability profile.
///
/// **Construct only via [`build_provider_profile`].** Inline construction is
/// forbidden by V9 §2 ("single factory in `crates/gaviero-core/src/context_planner/types.rs`")
/// and §9 non-goals. Adding a capability field becomes a compile error at
/// every unhandled site, which is the whole point of routing through one
/// constructor.
/// MCP transport a provider accepts injected servers over.
///
/// Recorded per provider because injection is not uniform: Claude/Codex/Cursor
/// are handed a config file and spawn stdio servers themselves, dsh accepts
/// only HTTP endpoints, and the in-process loop links a live server rather than
/// spawning anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransport {
    /// Provider spawns stdio MCP servers itself from a config file written to
    /// disk (Claude `.mcp.json`, Codex `[mcp_servers.*]`, Cursor
    /// `.cursor/mcp.json`).
    ConfigFileStdio,
    /// Provider accepts HTTP/SSE MCP endpoints only (dsh 0.1.5-rc.1 advertises
    /// `mcpCapabilities.http`). A stdio entry would be rejected on `session/new`.
    HttpOnly,
    /// In-process harness — servers are linked as a live value, not spawned
    /// over any transport.
    InProcess,
}

/// Per-provider MCP capability gates — the `context7_allowed` /
/// `extra_servers_allowed` axes of the capability table, plus the transport
/// that says whether an MCP-server entry is even meaningful for this provider.
///
/// Declared **once** by [`Provider::mcp_capabilities`], which is the single
/// source for both `build_provider_profile`'s arms and every provider-specific
/// injection decision (dsh's `session/new`; the config synthesizer's vendors).
/// Splitting the fact across an arm literal and a consumer is the exact drift
/// `tool_surface.rs`'s stale header demonstrated (§0 of the parity plan).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpCapabilities {
    /// Whether context7 is injected for this provider.
    pub context7: bool,
    /// Whether workspace `extraServers` are injected for this provider.
    pub extra_servers: bool,
    /// How this provider consumes MCP servers, if at all. The two axes above
    /// say *what* a provider may use; this says *how* it would receive it, and
    /// is what stops an MCP-server entry being emitted for a provider that has
    /// no MCP transport (see [`Self::uses_mcp_servers`]).
    pub transport: McpTransport,
}

impl McpCapabilities {
    /// Permissive default for a config-synthesizer vendor the table does not
    /// name. A new vendor is added to the table deliberately; until then it is
    /// not silently muted.
    ///
    /// Transport is [`McpTransport::ConfigFileStdio`] because every caller of
    /// this constructor is a config-file emitter. Use [`Self::permissive_over`]
    /// where the provider's real transport is known.
    pub const fn permissive() -> Self {
        Self::permissive_over(McpTransport::ConfigFileStdio)
    }

    /// Both axes allowed over a known transport.
    pub const fn permissive_over(transport: McpTransport) -> Self {
        Self {
            context7: true,
            extra_servers: true,
            transport,
        }
    }

    /// Whether this provider consumes MCP servers over any transport at all.
    ///
    /// `false` for the in-process harness, which *links* servers as a live
    /// value rather than spawning or connecting to them — so a `mcpServers`
    /// entry is meaningless for it no matter what the two axes say. This is the
    /// distinction that keeps `context7: true` (Phase 2d: the in-process loop
    /// reaches context7 as a *native* tool) from also registering a context7
    /// MCP server for a provider that would never read one.
    pub const fn uses_mcp_servers(self) -> bool {
        !matches!(self.transport, McpTransport::InProcess)
    }

    /// Capability row for a config-synthesizer vendor key (`"claude"`,
    /// `"codex"`, `"cursor"`) — the reverse of [`Provider::synth_vendor`].
    ///
    /// Resolved by searching [`Provider::ALL`] rather than matching on the
    /// string, so this lookup cannot drift from the forward one: renaming a
    /// vendor key re-points both directions at once.
    ///
    /// An unrecognised vendor gets [`Self::permissive`]. A vendor the table
    /// does not name is one gaviero has not reasoned about, and silently
    /// muting it would be the worse failure of the two.
    pub fn for_synth_vendor(vendor: &str) -> Self {
        Provider::ALL
            .iter()
            .find(|p| p.synth_vendor() == Some(vendor))
            .map(|p| p.mcp_capabilities())
            .unwrap_or_else(Self::permissive)
    }
}

impl Provider {
    /// Every provider arm.
    ///
    /// Exists so exhaustive iteration cannot silently miss an arm when one is
    /// added — the reverse lookup in [`McpCapabilities::for_synth_vendor`] and
    /// the table's own tests both walk this instead of a hand-copied list.
    pub const ALL: [Provider; 7] = [
        Provider::Claude,
        Provider::Codex,
        Provider::CodexAppServer,
        Provider::Cursor,
        Provider::Ollama,
        Provider::Deepseek,
        Provider::Dsh,
    ];

    /// The single source for this provider's MCP capability gates.
    ///
    /// **Decision (provider-parity #2): context7 is available to every
    /// provider.** The in-process loop (`ollama:`/`deepseek:`) still reads
    /// `false` because its injection path is unwired (Phase 2d), not because
    /// the provider is excluded.
    pub const fn mcp_capabilities(self) -> McpCapabilities {
        match self {
            Provider::Claude
            | Provider::Codex
            | Provider::CodexAppServer
            | Provider::Cursor => {
                McpCapabilities::permissive_over(McpTransport::ConfigFileStdio)
            }
            Provider::Dsh => McpCapabilities::permissive_over(McpTransport::HttpOnly),
            // Phase 2d: context7 reaches the in-process loop as a *native* tool
            // over context7's REST API (`tools/context7.rs`), not as an MCP
            // server — so `context7` is allowed. `extra_servers` stays `false`
            // and is not a gap: reaching a *foreign* MCP server would need the
            // in-process MCP client §2.7-C left unbuilt, and no native adapter
            // exists for arbitrary servers the way it does for context7.
            Provider::Ollama | Provider::Deepseek => McpCapabilities {
                context7: true,
                extra_servers: false,
                transport: McpTransport::InProcess,
            },
        }
    }

    /// Config-synthesizer vendor key for this provider, or `None` when it
    /// reads no generated MCP config file (the in-process loop links a live
    /// server; dsh receives servers on `session/new` instead).
    ///
    /// `codex exec` and `codex app-server` share one key because both load the
    /// same `<worktree>/.codex/config.toml`.
    pub const fn synth_vendor(self) -> Option<&'static str> {
        match self {
            Provider::Claude => Some("claude"),
            Provider::Codex | Provider::CodexAppServer => Some("codex"),
            Provider::Cursor => Some("cursor"),
            Provider::Ollama | Provider::Deepseek | Provider::Dsh => None,
        }
    }
}

/// How (and whether) `agent.availableTools` is enforced for a provider.
///
/// This is the axis that today has three implementations and one absence; see
/// `plans/provider-parity` §2.4. Recording it per provider is what makes a
/// missing enforcement (`Unenforced`) a visible declaration instead of an
/// accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEnforcement {
    /// Build-time, via argv (`claude --tools` / `--allowedTools`).
    Argv,
    /// Build-time, via deny rules baked into a generated config file
    /// (Cursor `.cursor/cli.json`).
    GeneratedConfig,
    /// Runtime — the host decides per call through `AgentToolSurface`
    /// (Codex app-server).
    RuntimeHost,
    /// Registry membership: a tool is reachable only if present in the set the
    /// loop can dispatch (in-process `deepseek:` / `ollama:`).
    RegistryMembership,
    /// Structurally unenforced — the protocol or session carries no tool list
    /// and no permission policy, so gaviero cannot restrict it (dsh
    /// `session/new`; `codex exec`).
    Unenforced,
}

impl ToolEnforcement {
    /// The sentence a UI should show when this provider's enforcement is
    /// absent, or `None` when there is nothing to disclose.
    ///
    /// Decision 1 (`plans/provider-parity` §0.1) is that dsh enforcement is
    /// **declared** absent rather than attempted: gaviero does not synthesize a
    /// tool list the protocol has no field for. Declaring it in the table is
    /// only half of that — the user has to be told, or "declared" means
    /// "recorded where they cannot see it". This is the other half.
    ///
    /// Deliberately `None` for every enforced variant: a notice that fires for
    /// all providers is noise, and noise is how a real warning gets ignored.
    pub fn ui_disclosure(&self, provider: &str) -> Option<String> {
        match self {
            Self::Unenforced => Some(format!(
                "{provider} runs unenforced: its protocol carries no tool list and no permission \
                 policy, so gaviero cannot restrict which tools or commands it runs. \
                 `agent.availableTools` and `agent.permissions` do not apply to this agent."
            )),
            Self::Argv | Self::GeneratedConfig | Self::RuntimeHost | Self::RegistryMembership => {
                None
            }
        }
    }
}

/// Shape of the mid-turn question a provider can ask the user.
///
/// This collapses the two axes an earlier draft named separately
/// (`can_prompt_user` + `prompt_kind`). A `bool` beside a kind invites the two
/// to disagree, which is the exact drift this table exists to prevent —
/// [`PromptKind::None`] *is* "cannot prompt".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// No interactive channel exists. Headless Cursor (`-p` / `--print`) and
    /// `codex exec` are both non-interactive by construction, so no prompt can
    /// be synthesised.
    None,
    /// Yes/no approval only (Codex command approval; dsh
    /// `session/request_permission`; the in-process loop's Bash gate).
    YesNo,
    /// Yes/no **and** multi-choice options (Claude `AskUserQuestion`).
    MultiChoice,
}

impl PromptKind {
    /// Whether this provider can surface a multi-choice question to the user.
    ///
    /// The in-process loop registers its `AskUserQuestion` tool off this
    /// predicate, so the table — not the registry — decides which providers
    /// hold an ask tool. `prompt_kind` already collapses "can prompt" and "what
    /// shape", so a second `bool` beside it would just be a way for the two to
    /// disagree.
    pub const fn has_multi_choice(self) -> bool {
        matches!(self, Self::MultiChoice)
    }
}

/// Capability record for a provider, built solely by
/// [`build_provider_profile`].
///
/// **This struct is the single declared source of truth for what gaviero
/// contributes to each provider** — see `plans/provider-parity`. Fields after
/// `bootstrap_tier` describe the parity axes; a field added here becomes a
/// compile error in every arm of the factory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderProfile {
    pub provider: String,
    pub model: String,
    pub continuity_mode: ContinuityMode,
    pub supports_tool_use: bool,
    pub supports_native_resume: bool,
    pub max_context_tokens: Option<usize>,
    /// PUSH→PULL bootstrap tier. Filled explicitly in every
    /// [`build_provider_profile`] arm; must match [`BootstrapTier::derive`].
    pub bootstrap_tier: BootstrapTier,
    /// Transport this provider accepts injected MCP servers over.
    pub mcp_transport: McpTransport,
    /// Whether context7 is injected for this provider.
    ///
    /// **Decision (provider-parity #2): context7 is available to every
    /// provider.** Values below still read `false` for Cursor/dsh/in-process
    /// only because those paths are not yet wired (Phase 2); the intended
    /// steady state is `true` throughout, and this field exists so a future
    /// exclusion is a visible edit rather than a silent omission.
    pub context7_allowed: bool,
    /// Whether workspace `extraServers` are injected for this provider.
    pub extra_servers_allowed: bool,
    /// How `agent.availableTools` is enforced (see [`ToolEnforcement`]).
    pub tool_enforcement: ToolEnforcement,
    /// Strictest mid-turn question this provider can ask (see [`PromptKind`]).
    pub prompt_kind: PromptKind,
}

impl ProviderProfile {
    /// The MCP capability gates as a compact value, for consumers that need
    /// only the injection decision (dsh's `session/new`, the config
    /// synthesizer). Read straight off the profile so the profile remains the
    /// single carrier of the table.
    pub const fn mcp_capabilities(&self) -> McpCapabilities {
        McpCapabilities {
            context7: self.context7_allowed,
            extra_servers: self.extra_servers_allowed,
            transport: self.mcp_transport,
        }
    }
}

/// Parsed `<prefix>:<model>` model spec.
///
/// Constructed from the user-facing `model: String` (e.g. `"claude:sonnet"`).
/// Parsing mirrors `swarm/backend/shared.rs::backend_config_for_model` —
/// keep them in sync (M10 will collapse the duplication).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Raw provider prefix (`"claude"`, `"codex"`,
    /// `"ollama"`, `"local"`, or `""` if no prefix was supplied — defaults to Claude).
    pub provider_prefix: String,
    /// Model name without the prefix.
    pub model: String,
    /// Raw user-supplied spec, kept for diagnostics.
    pub raw: String,
}

impl ModelSpec {
    /// Parse a `<prefix>:<model>` spec. Bare `"<model>"` is treated as Claude.
    pub fn parse(raw: &str) -> Self {
        let trimmed = raw.trim();
        // M8: `codex-app-server` must appear before `codex` so the longer
        // prefix wins (string-prefix comparison would not disambiguate them
        // on the suffix, but "codex-app-server:" does not start with "codex:"
        // anyway — the colon makes them distinct).
        for prefix in [
            "ollama",
            "local",
            "codex-app-server",
            "codex",
            "cursor",
            "deepseek",
            "dsh",
            "claude",
        ] {
            let with_colon = format!("{}:", prefix);
            if let Some(model) = trimmed.strip_prefix(&with_colon) {
                return Self {
                    provider_prefix: prefix.to_string(),
                    model: model.trim().to_string(),
                    raw: raw.to_string(),
                };
            }
        }
        // Bare model defaults to Claude (matches backend_config_for_model).
        Self {
            provider_prefix: String::new(),
            model: trimmed.to_string(),
            raw: raw.to_string(),
        }
    }

    /// Logical provider this spec maps to.
    pub fn provider(&self) -> Provider {
        match self.provider_prefix.as_str() {
            "ollama" | "local" => Provider::Ollama,
            "codex-app-server" => Provider::CodexAppServer,
            "codex" => Provider::Codex,
            "cursor" => Provider::Cursor,
            "deepseek" => Provider::Deepseek,
            "dsh" => Provider::Dsh,
            // Bare or claude-prefixed → Claude.
            _ => Provider::Claude,
        }
    }
}

/// Logical provider category. Distinct from `provider_prefix`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    /// `codex exec` — `StatelessReplay` fallback.
    Codex,
    /// `codex app-server` — `ProcessBound` continuity (M8).
    CodexAppServer,
    /// Cursor CLI — `StatelessReplay` in phase 1 (a later milestone
    /// promotes it to `NativeResume` via `--resume <chat-id>`).
    Cursor,
    Ollama,
    /// DeepSeek V4 Pro — in-process API tool-agent (`StatelessReplay`, native
    /// function-calling). See docs/plans/deepseek_v4_pro_provider.md.
    Deepseek,
    /// DeepSeek via `dsh --profile acp` over the Agent Client Protocol
    /// (`ProcessBound`).
    Dsh,
}

/// Runtime config the factory needs.
///
/// Kept minimal in M1; M9 may add Ollama-specific knobs and M8 may add
/// Codex `app-server` toggles.
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    pub ollama_base_url: Option<String>,
}

/// Single factory for [`ProviderProfile`] construction (V9 §2 locked decision).
///
/// Adding a capability field to `ProviderProfile` becomes a compile error in
/// this function until every provider arm fills it — which is exactly the
/// guarantee V9 §2 cites as the rationale.
pub fn build_provider_profile(spec: &ModelSpec, _runtime: &RuntimeConfig) -> ProviderProfile {
    // The MCP capability gates are declared once on `Provider` and copied here,
    // so an arm cannot drift from what the consumers enforce (dsh's
    // `session/new`; `Provider::synth_vendor`'s config files). See
    // `Provider::mcp_capabilities`.
    let caps = spec.provider().mcp_capabilities();
    match spec.provider() {
        Provider::Claude => ProviderProfile {
            provider: "claude".to_string(),
            model: spec.model.clone(),
            continuity_mode: ContinuityMode::NativeResume,
            supports_tool_use: true,
            supports_native_resume: true,
            // Sonnet/Haiku/Opus all share 200k context (May 2025). M6 may
            // refine per model; M0 Finding C means we don't yet read this
            // value back from any backend.
            max_context_tokens: Some(200_000),
            // tool_use + 200k context ⇒ Strong (matches BootstrapTier::derive).
            bootstrap_tier: BootstrapTier::Strong,
            mcp_transport: caps.transport,
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            // `claude --tools` argv (`acp/session.rs`).
            tool_enforcement: ToolEnforcement::Argv,
            // `AskUserQuestion` + y/n.
            prompt_kind: PromptKind::MultiChoice,
        },
        Provider::CodexAppServer => ProviderProfile {
            provider: "codex".to_string(),
            model: spec.model.clone(),
            // M8: `codex-app-server:` prefix → ProcessBound (V9 §5 table).
            // The subprocess stays alive across turns; thread ID round-trips
            // via ContinuityHandle::CodexThreadId.
            continuity_mode: ContinuityMode::ProcessBound,
            supports_tool_use: true,
            supports_native_resume: true,
            max_context_tokens: None,
            // tool_use + unknown (None) context ⇒ Strong (derive treats an
            // unknown window as not-small).
            bootstrap_tier: BootstrapTier::Strong,
            mcp_transport: caps.transport,
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            // Runtime decisions via `AgentToolSurface` (`tool_surface.rs`).
            tool_enforcement: ToolEnforcement::RuntimeHost,
            // y/n only — `item/commandExecution/requestApproval`.
            prompt_kind: PromptKind::YesNo,
        },
        Provider::Codex => ProviderProfile {
            provider: "codex".to_string(),
            model: spec.model.clone(),
            // `codex exec` stays StatelessReplay (V9 §5). M8 adds the
            // `codex-app-server:` prefix for ProcessBound; the plain `codex:`
            // prefix retains this arm unchanged.
            continuity_mode: ContinuityMode::StatelessReplay,
            supports_tool_use: true,
            supports_native_resume: false,
            // M0 Finding I: Codex backend doesn't surface usage today;
            // value is informational until M8/M9 wire budgeting.
            max_context_tokens: None,
            // tool_use + unknown (None) context ⇒ Strong.
            bootstrap_tier: BootstrapTier::Strong,
            mcp_transport: caps.transport,
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            // No gaviero-side enforcement wired; `codex exec` carries no tool
            // list to this session (see `codex_exec.rs`). UNVERIFIED against a
            // live codex — Phase 7's parity test must confirm.
            tool_enforcement: ToolEnforcement::Unenforced,
            // `codex exec` is non-interactive by construction
            // (`codex_exec.rs` module doc).
            prompt_kind: PromptKind::None,
        },
        Provider::Cursor => ProviderProfile {
            provider: "cursor".to_string(),
            model: spec.model.clone(),
            // `agent --resume <chat-id>` carries the prior thread's
            // server-side state across turns, so the planner can omit
            // replay history and rely on Cursor's continuity. The
            // session updates the handle from the `system.init` event
            // every turn so an expired thread cleanly falls back to a
            // fresh chat id.
            continuity_mode: ContinuityMode::NativeResume,
            supports_tool_use: true,
            supports_native_resume: true,
            // Cursor's hosted models vary by account; 200k is a safe upper
            // bound that matches Claude / Codex.
            max_context_tokens: Some(200_000),
            // tool_use + 200k context ⇒ Strong.
            bootstrap_tier: BootstrapTier::Strong,
            mcp_transport: caps.transport,
            // Delivered (Phase 2, decision #2): Cursor's context7 exclusion is
            // gone. The registration is a `url` entry, so it is not the stdio
            // shape `validate_synthesized_cursor_remote_mcp` rejects; the
            // preflight now keys on transport rather than on the server name.
            // A *stdio* context7 is still withheld from Cursor when a remote
            // URL extra exists — that is the conditional half of decision #2.
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            // Deny rules baked into the generated `.cursor/mcp.json` /
            // `cli.json` (`config_synth.rs`).
            tool_enforcement: ToolEnforcement::GeneratedConfig,
            // Headless `-p`/`--print`: no interactive channel exists.
            prompt_kind: PromptKind::None,
        },
        Provider::Ollama => ProviderProfile {
            provider: "ollama".to_string(),
            model: spec.model.clone(),
            continuity_mode: ContinuityMode::StatelessReplay,
            // Tool-use depends on the local model. Default false matches the
            // conservative path; models that support tool use can override
            // via config in a later milestone.
            supports_tool_use: false,
            supports_native_resume: false,
            // M9: set a conservative default context window for Ollama models.
            // 8 192 tokens matches llama3.1 7B and many other popular models.
            // The token-pressure compaction trigger in `OllamaSession` uses
            // this value to bound replay history size. A future milestone may
            // query the Ollama `/api/show` endpoint for per-model context size.
            max_context_tokens: Some(8_192),
            // No reliable tool use (and an 8k window) ⇒ SmallLocal: keep the
            // full push until per-tier evidence proves a thin bootstrap holds.
            bootstrap_tier: BootstrapTier::SmallLocal,
            // In-process loop: servers are linked, not spawned.
            mcp_transport: caps.transport,
            // Phase 2 wires context7 for the in-process loop.
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            // Tool reachability = registry membership (`tools/mod.rs`).
            tool_enforcement: ToolEnforcement::RegistryMembership,
            // Delivered (Phase 4): the in-process loop now holds
            // `AskUserQuestion` (`tools/ask.rs`), riding the same
            // `on_permission_request` channel as the Bash gate.
            prompt_kind: PromptKind::MultiChoice,
        },
        Provider::Deepseek => ProviderProfile {
            provider: "deepseek".to_string(),
            model: spec.model.clone(),
            // Raw DeepSeek HTTP API: the harness replays history each turn
            // (no server-side thread), and DeepSeek V4 Pro exposes native
            // function-calling, so the in-process loop drives tool use.
            continuity_mode: ContinuityMode::StatelessReplay,
            supports_tool_use: true,
            supports_native_resume: false,
            max_context_tokens: Some(128_000),
            // tool_use + 128k context ⇒ Strong.
            bootstrap_tier: BootstrapTier::Strong,
            mcp_transport: caps.transport,
            // Phase 2 wires context7 for the in-process loop.
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            tool_enforcement: ToolEnforcement::RegistryMembership,
            // Delivered (Phase 4): `tools/ask.rs`, shared with the Ollama arm.
            prompt_kind: PromptKind::MultiChoice,
        },
        Provider::Dsh => ProviderProfile {
            provider: "dsh".to_string(),
            model: spec.model.clone(),
            continuity_mode: ContinuityMode::ProcessBound,
            supports_tool_use: true,
            supports_native_resume: true,
            max_context_tokens: Some(128_000),
            bootstrap_tier: BootstrapTier::Strong,
            // dsh 0.1.5-rc.1 advertises `mcpCapabilities.http` only
            // (`dsh.rs`). Delivered (Phase 2, decisions #2/E): `session/new`
            // now carries context7 and URL-form `extraServers`, each behind
            // the same `mcp.permissions` registration gate the file-based
            // providers apply. A stdio entry — a `command` extra, or context7
            // in stdio-fallback mode — cannot be hosted and is skipped.
            mcp_transport: caps.transport,
            context7_allowed: caps.context7,
            extra_servers_allowed: caps.extra_servers,
            // `session/new` carries no tool list and no permission policy —
            // structurally unenforced (decision #1: declared, not attempted).
            tool_enforcement: ToolEnforcement::Unenforced,
            // y/n via `session/request_permission`.
            prompt_kind: PromptKind::YesNo,
        },
    }
}

/// PUSH→PULL Phase 4: resolve the effective bootstrap tier, letting an explicit
/// `agent.bootstrapTier` workspace setting override the capability-derived tier
/// (`profile.bootstrap_tier`).
///
/// The explicit setting wins, so a known-good local tool-calling model can be
/// forced onto the thin-anchor (`strong`) path, or a flaky one pinned to the
/// full push (`smalllocal`). An empty or unrecognized setting falls back to the
/// derived tier — keeping [`BootstrapTier::derive`] the default source of truth.
pub fn resolve_bootstrap_tier(profile: &ProviderProfile, setting: Option<&str>) -> BootstrapTier {
    match setting.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("strong") => BootstrapTier::Strong,
        Some("smalllocal" | "small_local" | "small-local" | "local" | "weak") => {
            BootstrapTier::SmallLocal
        }
        // None, "", or anything unrecognized → the capability-derived tier.
        _ => profile.bootstrap_tier,
    }
}

/// Planner input. Replaces the ad-hoc tuple-of-strings each call site assembled.
///
/// **Note (V9 §0 rule 4, §10 stop 11):** there is intentionally no
/// `conversation_history` field here. Replay data lives in
/// `SessionLedger::replay_history` — the single source of truth.
/// Adding such a field to this struct is forbidden.
pub struct PlannerInput<'a> {
    pub user_message: &'a str,

    /// Files the user explicitly named (e.g. `@file` mentions in chat).
    pub explicit_refs: &'a [PathBuf],

    /// Seeds for graph ranking: explicit refs ∪ active buffer path (chat) or
    /// `WorkUnit::scope::owned_paths` (swarm).
    pub seed_paths: &'a [PathBuf],

    pub provider_profile: &'a ProviderProfile,

    /// Memory namespaces to read from (chat: workspace settings; swarm:
    /// per-pipeline `read_namespaces`).
    pub read_namespaces: &'a [String],

    /// Graph context budget in tokens. 0 disables graph injection (matches
    /// the existing `agent_settings.graph_budget_tokens` semantics).
    ///
    /// M1 plumbing field; M3 may move this into a structured policy struct.
    pub graph_budget_tokens: usize,

    /// Memory query override (swarm uses `WorkUnit::memory_read_query` here;
    /// chat reuses `user_message`).
    pub memory_query_override: Option<&'a str>,

    /// Memory result limit (swarm uses `WorkUnit::memory_read_limit`; chat
    /// uses 5 to match today's hardcode).
    pub memory_limit: usize,

    /// Pre-computed file_refs (path, contents) — chat assembles these from
    /// `@file` parsing and disk reads. M1 keeps the existing assembly site;
    /// M3 may move it into the planner.
    pub file_ref_blobs: &'a [(String, String)],

    /// Pre-computed graph impact text (chat-side spawn_blocking call). M1
    /// keeps the current spawn_blocking site in `app/session.rs`; M2/M3
    /// migrate it into the planner. None = no impact text injected today
    /// (swarm's `impact_text` parameter passes through here).
    pub pre_fetched_impact_text: Option<&'a str>,

    /// Full pre-rendered graph context block (chat path: result of
    /// `build_graph_context` which already concatenates repo outline +
    /// impact text inside a spawn_blocking task). When `Some(_)`, the
    /// planner skips its own `RepoMap::rank_for_agent` query and uses this
    /// verbatim as the single graph selection. M2 removes this carrier
    /// when the chat path stops assembling graph context itself.
    pub pre_fetched_graph_context: Option<&'a str>,

    /// Full pre-rendered memory context block (chat path: result of
    /// `MemoryStore::search_context`). When `Some(_)`, the planner skips
    /// its own memory query and uses this verbatim as the single memory
    /// selection. M2 / M3 removes this carrier.
    pub pre_fetched_memory_context: Option<&'a str>,

    /// Workspace-wide opt-in: additional folder paths to query memory
    /// against, on top of the planner's own `workspace_root`. When non-
    /// empty, `collect_memory` retrieves once per folder (each yielding
    /// folder + workspace + global candidates) and dedupes by
    /// content_hash. Empty in single-folder mode and on focused-folder
    /// chat dispatch (today's default). The TUI sets this when the user
    /// arms `/workspace`, listing every other workspace folder.
    pub extra_folder_paths: &'a [&'a std::path::Path],

    /// Workspace-wide opt-in: additional repo maps to rank graph
    /// candidates against, on top of `ContextPlanner.repo_map`. The
    /// planner ranks each map separately and merges results by
    /// `rank_score` under the existing `graph_budget_tokens`. Empty in
    /// single-folder mode. Length matches `extra_folder_paths` in chat
    /// usage but the planner does not require alignment.
    pub extra_repo_maps: &'a [&'a crate::repo_map::RepoMap],

    /// Shallow directory map settings (`agent.topology.*`).
    pub topology_config: crate::repo_map::TopologyConfig,

    /// Pre-built topology body (chat/swarm prefetch). When `Some`, the
    /// planner skips `build_folder_topology` for the primary root.
    pub pre_fetched_topology: Option<&'a str>,

    /// Additional topology blocks for multi-root `/workspace` mode.
    /// Each entry is `(folder_label, body)` without XML tags.
    pub extra_topology_blocks: &'a [(&'a str, &'a str)],

    /// Turn-scoped skills resolved from `$skill` invocations in chat.
    /// Passed every turn (outside the bootstrap gate).
    pub resolved_skills: &'a [crate::skills::ResolvedSkill],

    /// Resolved per-layer bootstrap switches for this planner pass.
    /// Chat callers use [`resolve_chat_bootstrap_arms`]; swarm passes
    /// [`BootstrapArms::swarm_first_turn`].
    pub bootstrap_arms: super::bootstrap::BootstrapArms,
}

/// Memory selection record.
///
/// M1 carries the legacy concatenated string in `content`. M3 widens this
/// struct with `id`, `score`, `trust`, `source_hash`, `updated_at` per V9 §4
/// `MemoryCandidate` and populates one entry per memory hit. Keep optional
/// fields here so M3 only adds population logic, not struct-shape churn.
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySelection {
    /// Canonical memory id. M1: `None` (legacy string). M3: real id.
    pub id: Option<i64>,
    /// Memory namespace (e.g. `"workspace"`, `"repo"`). M1: `None`.
    pub namespace: Option<String>,
    /// Scope label for tracing. M1: `None`.
    pub scope_label: Option<String>,
    /// Selection score. M1: `None`.
    pub score: Option<f32>,
    /// Trust marker. M1: `None`.
    pub trust: Option<String>,
    /// Memory body. M1: full pre-formatted memory block.
    pub content: String,
    /// Source hash for invalidation. M1: `None`.
    pub source_hash: Option<String>,
    /// Updated-at timestamp. M1: `None`.
    pub updated_at: Option<String>,
}

/// What kind of graph attachment the planner chose.
///
/// Mirrors V9 §4 `GraphDecision`. Repeated here as a `*Kind` enum for use on
/// `GraphSelection`; the ledger still uses `GraphDecision` directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphSelectionKind {
    PathOnly,
    SignatureOnly,
    OutlineOnly,
    FullContent,
    /// Shallow directory map (`<repo_topology>`).
    Topology,
}

/// Graph selection record.
///
/// M1 carries the legacy concatenated outline in `content` with `path = None`
/// and `kind = OutlineOnly`. M3 widens to one entry per ranked file with real
/// `confidence`, `symbols`, `content_digest` per V9 §4 `GraphCandidate`.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphSelection {
    /// File the selection refers to. M1: `None` (legacy combined outline).
    pub path: Option<PathBuf>,
    pub kind: GraphSelectionKind,
    /// Token estimate from `RepoMap::rank_for_agent::ContextPlan::token_estimate`.
    pub token_estimate: usize,
    /// Pre-rendered content. M1: full outline / impact text. M3: per-file
    /// snippet or signature.
    pub content: String,
    /// Per-file rank. M1: `None`. M3: from PageRank.
    pub rank_score: Option<f64>,
    /// Confidence band. M1: `None`. M3: from M3's confidence model.
    pub confidence: Option<GraphConfidence>,
    /// Symbol summaries. M1: empty. M3: parsed signatures.
    pub symbols: Vec<Symbol>,
    /// Content digest for invalidation. M1: `None`.
    pub content_digest: Option<String>,
}

/// Placeholder for V9 §4 `Symbol`. M3 fills.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
}

/// File attachment record. M1 carries `(path, contents)` to mirror the
/// existing chat `file_refs: Vec<(String, String)>` shape.
#[derive(Debug, Clone, PartialEq)]
pub struct FileAttachment {
    pub path: PathBuf,
    /// Pre-fetched contents. M1: always `Some(_)` for chat (current behavior).
    /// M3 may make this lazy.
    pub content: Option<String>,
}

/// Turn-scoped skill injection record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSelection {
    pub name: String,
    pub scope_level: i32,
    pub rendered_body: String,
}

/// Replay payload for `StatelessReplay` providers.
///
/// Mirrors `SessionLedger::replay_history` so the rendering adapter can
/// transform it into the legacy `Vec<(String, String)>` `conversation_history`
/// shape backends expect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReplayPayload {
    pub entries: Vec<(crate::context_planner::ledger::Role, String)>,
}

/// Planner output.
///
/// **All fields are structured.** No prompt strings beyond the verbatim
/// content of memory/graph entries (which the renderer turns into a final
/// prompt). V9 §0 rule 5 forbids prompt formatting inside the planner.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlannerSelections {
    pub memory_selections: Vec<MemorySelection>,
    pub graph_selections: Vec<GraphSelection>,
    pub file_refs: Vec<FileAttachment>,
    pub skill_selections: Vec<SkillSelection>,
    /// `Some(_)` only for `StatelessReplay` providers. `None` for
    /// `NativeResume` / `ProcessBound` — they hold history server-side.
    pub replay_history: Option<ReplayPayload>,
    pub metadata: PlannerMetadata,
}

/// Tracing-only metadata. Not consumed by transports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlannerMetadata {
    pub memory_count: usize,
    pub graph_token_estimate: usize,
    pub graph_budget: usize,
    pub is_first_turn: bool,
    pub continuity_mode: Option<ContinuityMode>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_spec_parses_known_prefixes() {
        let cases = [
            ("claude:sonnet", "claude", "sonnet", Provider::Claude),
            ("claude:opus", "claude", "opus", Provider::Claude),
            ("sonnet", "", "sonnet", Provider::Claude),
            ("codex:gpt-5.5", "codex", "gpt-5.5", Provider::Codex),
            ("cursor:auto", "cursor", "auto", Provider::Cursor),
            (
                "cursor:claude-4.6-opus-high-thinking",
                "cursor",
                "claude-4.6-opus-high-thinking",
                Provider::Cursor,
            ),
            (
                "ollama:qwen2.5-coder:7b",
                "ollama",
                "qwen2.5-coder:7b",
                Provider::Ollama,
            ),
            ("local:llama3.1", "local", "llama3.1", Provider::Ollama),
            (
                "deepseek:deepseek-v4-pro",
                "deepseek",
                "deepseek-v4-pro",
                Provider::Deepseek,
            ),
            (
                "dsh:deepseek-v4-flash",
                "dsh",
                "deepseek-v4-flash",
                Provider::Dsh,
            ),
        ];
        for (raw, prefix, model, provider) in cases {
            let spec = ModelSpec::parse(raw);
            assert_eq!(spec.provider_prefix, prefix, "prefix mismatch for {}", raw);
            assert_eq!(spec.model, model, "model mismatch for {}", raw);
            assert_eq!(spec.provider(), provider, "provider mismatch for {}", raw);
        }
    }

    #[test]
    fn cursor_profile_is_native_resume_with_tool_use() {
        let runtime = RuntimeConfig::default();
        let profile = build_provider_profile(&ModelSpec::parse("cursor:auto"), &runtime);
        assert_eq!(profile.provider, "cursor");
        assert_eq!(profile.continuity_mode, ContinuityMode::NativeResume);
        assert!(profile.supports_tool_use);
        assert!(profile.supports_native_resume);
        assert_eq!(profile.max_context_tokens, Some(200_000));
    }

    #[test]
    fn cursor_thread_id_continuity_handle_round_trips() {
        let h = ContinuityHandle::CursorThreadId("abc-thread".to_string());
        let json = serde_json::to_string(&h).unwrap();
        assert!(
            json.contains("CursorThreadId"),
            "variant tag must be explicit so persisted state remains self-describing"
        );
        let back: ContinuityHandle = serde_json::from_str(&json).unwrap();
        assert_eq!(h, back);
    }

    #[test]
    fn factory_maps_providers_to_continuity_modes() {
        // Pins V9 §5 provider mapping table.
        let runtime = RuntimeConfig::default();

        let claude = build_provider_profile(&ModelSpec::parse("claude:sonnet"), &runtime);
        assert_eq!(claude.continuity_mode, ContinuityMode::NativeResume);
        assert!(claude.supports_native_resume);
        assert!(claude.supports_tool_use);
        assert_eq!(claude.max_context_tokens, Some(200_000));

        let codex = build_provider_profile(&ModelSpec::parse("codex:gpt-5.5"), &runtime);
        assert_eq!(codex.continuity_mode, ContinuityMode::StatelessReplay);
        assert!(!codex.supports_native_resume);

        // M8: codex-app-server: → ProcessBound (V9 §5 table).
        let codex_as =
            build_provider_profile(&ModelSpec::parse("codex-app-server:gpt-5.5"), &runtime);
        assert_eq!(codex_as.continuity_mode, ContinuityMode::ProcessBound);
        assert!(codex_as.supports_native_resume);
        assert_eq!(codex_as.provider, "codex");

        let ollama = build_provider_profile(&ModelSpec::parse("ollama:llama3.1"), &runtime);
        assert_eq!(ollama.continuity_mode, ContinuityMode::StatelessReplay);
        assert!(!ollama.supports_native_resume);

        let deepseek =
            build_provider_profile(&ModelSpec::parse("deepseek:deepseek-v4-pro"), &runtime);
        assert_eq!(deepseek.provider, "deepseek");
        assert_eq!(deepseek.continuity_mode, ContinuityMode::StatelessReplay);
        assert!(deepseek.supports_tool_use);
        assert!(!deepseek.supports_native_resume);
        assert_eq!(deepseek.max_context_tokens, Some(128_000));

        let dsh = build_provider_profile(&ModelSpec::parse("dsh:deepseek-v4-flash"), &runtime);
        assert_eq!(dsh.provider, "dsh");
        assert_eq!(dsh.continuity_mode, ContinuityMode::ProcessBound);
        assert!(dsh.supports_tool_use);
        assert!(dsh.supports_native_resume);
        assert_eq!(dsh.max_context_tokens, Some(128_000));

        let bare = build_provider_profile(&ModelSpec::parse("haiku"), &runtime);
        assert_eq!(bare.continuity_mode, ContinuityMode::NativeResume);
        assert_eq!(bare.provider, "claude");
    }

    #[test]
    fn build_provider_profile_sets_tier() {
        // PUSH→PULL Phase 0 gate: Ollama is the only SmallLocal provider
        // (no reliable tool use); every other arm is Strong.
        let runtime = RuntimeConfig::default();

        let ollama = build_provider_profile(&ModelSpec::parse("ollama:llama3.1"), &runtime);
        assert_eq!(ollama.bootstrap_tier, BootstrapTier::SmallLocal);
        let local = build_provider_profile(&ModelSpec::parse("local:llama3.1"), &runtime);
        assert_eq!(local.bootstrap_tier, BootstrapTier::SmallLocal);

        for spec in [
            "claude:sonnet",
            "codex-app-server:gpt-5.5",
            "codex:gpt-5.5",
            "cursor:auto",
            "deepseek:deepseek-v4-pro",
            "dsh:deepseek-v4-flash",
            "haiku", // bare → claude
        ] {
            let p = build_provider_profile(&ModelSpec::parse(spec), &runtime);
            assert_eq!(
                p.bootstrap_tier,
                BootstrapTier::Strong,
                "spec {spec} should resolve to Strong"
            );
        }
    }

    #[test]
    fn build_provider_profile_tier_matches_derivation_rule() {
        // Pin every arm's explicit literal to BootstrapTier::derive so the two
        // can never silently diverge as providers are added or capabilities
        // change.
        let runtime = RuntimeConfig::default();
        for spec in [
            "claude:sonnet",
            "codex-app-server:gpt-5.5",
            "codex:gpt-5.5",
            "cursor:auto",
            "ollama:llama3.1",
            "deepseek:deepseek-v4-pro",
            "dsh:deepseek-v4-flash",
        ] {
            let p = build_provider_profile(&ModelSpec::parse(spec), &runtime);
            assert_eq!(
                p.bootstrap_tier,
                BootstrapTier::derive(p.supports_tool_use, p.max_context_tokens),
                "arm literal for {spec} diverged from BootstrapTier::derive"
            );
        }
    }

    #[test]
    fn resolve_bootstrap_tier_override_wins_over_derived() {
        let runtime = RuntimeConfig::default();
        let ollama = build_provider_profile(&ModelSpec::parse("ollama:llama3.1"), &runtime);
        let claude = build_provider_profile(&ModelSpec::parse("claude:sonnet"), &runtime);

        // No / empty / unrecognized setting → the derived tier.
        assert_eq!(
            resolve_bootstrap_tier(&ollama, None),
            BootstrapTier::SmallLocal
        );
        assert_eq!(
            resolve_bootstrap_tier(&ollama, Some("")),
            BootstrapTier::SmallLocal
        );
        assert_eq!(
            resolve_bootstrap_tier(&ollama, Some("nonsense")),
            BootstrapTier::SmallLocal
        );
        assert_eq!(resolve_bootstrap_tier(&claude, None), BootstrapTier::Strong);

        // Explicit override wins, case/whitespace-insensitive.
        assert_eq!(
            resolve_bootstrap_tier(&ollama, Some("strong")),
            BootstrapTier::Strong
        );
        assert_eq!(
            resolve_bootstrap_tier(&ollama, Some("  Strong ")),
            BootstrapTier::Strong
        );
        assert_eq!(
            resolve_bootstrap_tier(&claude, Some("small-local")),
            BootstrapTier::SmallLocal
        );
        assert_eq!(
            resolve_bootstrap_tier(&claude, Some("SmallLocal")),
            BootstrapTier::SmallLocal
        );
    }

    #[test]
    fn continuity_handle_round_trips_with_variant_tag() {
        let h = ContinuityHandle::ClaudeSessionId("abc-123".to_string());
        let json = serde_json::to_string(&h).unwrap();
        // Variant tag is explicit so persisted state remains self-describing
        // when M4 lands (V9 §4 ContinuityHandle doc-comment).
        assert!(json.contains("ClaudeSessionId"));
        let back: ContinuityHandle = serde_json::from_str(&json).unwrap();
        assert_eq!(h, back);
    }

    // ── Provider capability table (provider-parity Phase 1) ───────────────────
    //
    // These pin the declared matrix so it cannot rot the way `tool_surface.rs`'s
    // header did. They assert *today's verified* values; the `Phase N` comments
    // in `build_provider_profile` name the intended changes, and each such
    // assertion carries the phase that will flip it.

    fn profile(spec: &str) -> ProviderProfile {
        build_provider_profile(&ModelSpec::parse(spec), &RuntimeConfig::default())
    }

    #[test]
    fn claude_is_argv_enforced_with_multi_choice_prompting() {
        let p = profile("claude:sonnet");
        assert_eq!(p.mcp_transport, McpTransport::ConfigFileStdio);
        assert!(p.context7_allowed);
        assert!(p.extra_servers_allowed);
        assert_eq!(p.tool_enforcement, ToolEnforcement::Argv);
        assert_eq!(p.prompt_kind, PromptKind::MultiChoice);
    }

    #[test]
    fn codex_app_server_enforces_at_runtime_and_prompts_yesno() {
        let p = profile("codex-app-server:gpt-5");
        assert_eq!(p.mcp_transport, McpTransport::ConfigFileStdio);
        assert_eq!(p.tool_enforcement, ToolEnforcement::RuntimeHost);
        assert_eq!(p.prompt_kind, PromptKind::YesNo);
    }

    /// `codex exec` must NOT be conflated with `codex app-server`: it is
    /// non-interactive and unenforced from gaviero's side.
    #[test]
    fn codex_exec_is_unenforced_and_cannot_prompt() {
        let p = profile("codex:gpt-5");
        assert_eq!(p.tool_enforcement, ToolEnforcement::Unenforced);
        assert_eq!(p.prompt_kind, PromptKind::None);
    }

    #[test]
    fn cursor_is_build_time_enforced_with_no_prompt_channel() {
        let p = profile("cursor:gpt-5");
        assert_eq!(p.tool_enforcement, ToolEnforcement::GeneratedConfig);
        assert_eq!(p.prompt_kind, PromptKind::None);
        // Delivered (Phase 2, decision #2): Cursor is no longer excluded from
        // context7. The registration is a `url` entry, so it is not the stdio
        // shape `validate_synthesized_cursor_remote_mcp` rejects.
        assert!(p.context7_allowed);
    }

    #[test]
    fn dsh_is_http_only_and_structurally_unenforced() {
        let p = profile("dsh:deepseek-chat");
        assert_eq!(p.mcp_transport, McpTransport::HttpOnly);
        // Decision #1: declared unenforced, not attempted.
        assert_eq!(p.tool_enforcement, ToolEnforcement::Unenforced);
        assert_eq!(p.prompt_kind, PromptKind::YesNo);
        // Delivered (Phase 2, decisions #2/E): `session/new` now carries
        // context7 and URL-form `extraServers`, each behind the same
        // `mcp.permissions` registration gate the file-based providers apply.
        assert!(p.context7_allowed);
        assert!(p.extra_servers_allowed);
    }

    /// §D wiring: the arms copy `Provider::mcp_capabilities`, so a consumer
    /// that reads the table cannot disagree with the profile it is handed.
    #[test]
    fn profile_gates_match_the_provider_table() {
        for (spec, provider) in [
            ("claude:sonnet", Provider::Claude),
            ("codex:gpt-5", Provider::Codex),
            ("codex-app-server:gpt-5", Provider::CodexAppServer),
            ("cursor:gpt-5", Provider::Cursor),
            ("ollama:llama3.1", Provider::Ollama),
            ("deepseek:deepseek-chat", Provider::Deepseek),
            ("dsh:deepseek-chat", Provider::Dsh),
        ] {
            let p = profile(spec);
            let caps = provider.mcp_capabilities();
            assert_eq!(p.context7_allowed, caps.context7, "{spec}");
            assert_eq!(p.extra_servers_allowed, caps.extra_servers, "{spec}");
            assert_eq!(p.mcp_capabilities(), caps, "{spec}");
        }
    }

    /// The synthesizer keys off `synth_vendor`. Providers that read no
    /// generated MCP config file — the in-process loop links a live server, dsh
    /// receives servers on `session/new` — must say so rather than being
    /// assumed to have a vendor key.
    #[test]
    fn only_config_file_providers_name_a_synth_vendor() {
        assert_eq!(Provider::Claude.synth_vendor(), Some("claude"));
        assert_eq!(Provider::Codex.synth_vendor(), Some("codex"));
        // One key: both load `<worktree>/.codex/config.toml`.
        assert_eq!(Provider::CodexAppServer.synth_vendor(), Some("codex"));
        assert_eq!(Provider::Cursor.synth_vendor(), Some("cursor"));
        for p in [Provider::Ollama, Provider::Deepseek, Provider::Dsh] {
            assert_eq!(p.synth_vendor(), None, "{p:?}");
        }
    }

    /// §2.7-D: the synthesizer resolves a vendor key back to a table row. The
    /// reverse lookup must agree with the forward one for **every** provider, so
    /// renaming a key re-points both directions at once and a new arm cannot be
    /// added to one side only.
    #[test]
    fn synth_vendor_reverse_lookup_matches_the_table() {
        for p in Provider::ALL {
            // Only providers with a vendor key are reachable from the
            // synthesizer; the rest declare `None` by design.
            let Some(vendor) = p.synth_vendor() else {
                continue;
            };
            assert_eq!(
                McpCapabilities::for_synth_vendor(vendor),
                p.mcp_capabilities(),
                "{p:?} ({vendor}) reverse lookup disagreed with the table"
            );
        }
    }

    /// A vendor the table does not name is permissive, never muted: the
    /// compatibility direction that cannot silently starve a config file.
    #[test]
    fn unknown_synth_vendor_is_permissive() {
        assert_eq!(
            McpCapabilities::for_synth_vendor("some-future-vendor"),
            McpCapabilities::permissive()
        );
        // The in-process providers have no vendor key, so their natural names
        // are unrecognised — they must not resolve to their own restrictive row.
        for p in [Provider::Ollama, Provider::Deepseek, Provider::Dsh] {
            assert_eq!(
                McpCapabilities::for_synth_vendor(&format!("{p:?}").to_lowercase()),
                McpCapabilities::permissive(),
                "{p:?} resolved to a restrictive row despite having no vendor key"
            );
        }
    }

    #[test]
    fn in_process_loop_is_registry_enforced_over_a_linked_server() {
        for spec in ["deepseek:deepseek-chat", "ollama:llama3.1"] {
            let p = profile(spec);
            assert_eq!(p.mcp_transport, McpTransport::InProcess, "{spec}");
            assert_eq!(
                p.tool_enforcement,
                ToolEnforcement::RegistryMembership,
                "{spec}"
            );
            // Delivered (Phase 4): the in-process loop holds `AskUserQuestion`
            // (`tools/ask.rs`), so its prompt channel is multi-choice.
            assert_eq!(p.prompt_kind, PromptKind::MultiChoice, "{spec}");
            assert!(p.prompt_kind.has_multi_choice(), "{spec}");
        }
    }

    /// The whole point of one table: the declared prompt channel matches what
    /// each provider actually offers. Cursor and `codex exec` are the two with
    /// no channel at all; Codex-app-server and dsh have y/n; Claude and the
    /// in-process loop reach multi-choice. If this fails, either a real
    /// capability changed (update the table deliberately) or the table drifted
    /// from the code.
    #[test]
    fn prompt_channels_are_declared_not_assumed() {
        let no_channel = ["cursor:x", "codex:x"].map(|s| profile(s).prompt_kind);
        assert!(
            no_channel.iter().all(|k| *k == PromptKind::None),
            "expected no prompt channel: {no_channel:?}"
        );
        assert!(no_channel.iter().all(|k| !k.has_multi_choice()));

        let yes_no = ["codex-app-server:gpt-5", "dsh:x"].map(|s| profile(s).prompt_kind);
        assert!(
            yes_no.iter().all(|k| *k == PromptKind::YesNo),
            "expected y/n channels: {yes_no:?}"
        );
        assert!(yes_no.iter().all(|k| !k.has_multi_choice()));

        // Two providers reach multi-choice: Claude natively, and the in-process
        // loop via its own ask tool (Phase 4).
        let multi = ["claude:sonnet", "deepseek:x", "ollama:x"].map(|s| profile(s).prompt_kind);
        assert!(
            multi.iter().all(|k| *k == PromptKind::MultiChoice),
            "expected multi-choice channels: {multi:?}"
        );
        assert!(multi.iter().all(|k| k.has_multi_choice()));
    }

    /// `has_multi_choice` is the registration predicate for the in-process ask
    /// tool, so it must agree with the enum in both directions — a provider that
    /// gains the tool without the row (or the reverse) is the drift this table
    /// exists to prevent.
    #[test]
    fn has_multi_choice_agrees_with_the_kind_it_is_derived_from() {
        for kind in [
            PromptKind::None,
            PromptKind::YesNo,
            PromptKind::MultiChoice,
        ] {
            assert_eq!(kind.has_multi_choice(), kind == PromptKind::MultiChoice);
        }
    }

    /// Phase 6: an unenforced provider discloses; every enforced one stays
    /// silent, and the disclosed set is exactly the two structurally
    /// unconfined backends (`codex exec` + dsh).
    #[test]
    fn only_structurally_unenforced_providers_disclose() {
        let mut disclosed: Vec<String> = Vec::new();
        for spec in [
            "claude:sonnet",
            "codex:x",
            "codex-app-server:gpt-5",
            "cursor:x",
            "dsh:x",
            "deepseek:x",
            "ollama:x",
        ] {
            let profile = build_provider_profile(
                &ModelSpec::parse(spec),
                &RuntimeConfig::default(),
            );
            if let Some(msg) = profile
                .tool_enforcement
                .ui_disclosure(&profile.provider)
            {
                assert!(
                    msg.contains(&profile.provider),
                    "disclosure must name the provider: {msg}"
                );
                assert!(msg.contains("unenforced"), "{msg}");
                disclosed.push(profile.provider.clone());
            }
        }
        assert_eq!(disclosed, vec!["codex".to_string(), "dsh".to_string()]);

        // A warning that fires for everyone is noise: the enforced variants
        // must return `None` for the same provider name.
        for enforced in [
            ToolEnforcement::Argv,
            ToolEnforcement::GeneratedConfig,
            ToolEnforcement::RuntimeHost,
            ToolEnforcement::RegistryMembership,
        ] {
            assert!(
                enforced.ui_disclosure("dsh").is_none(),
                "{enforced:?} must not disclose"
            );
        }
    }
}
