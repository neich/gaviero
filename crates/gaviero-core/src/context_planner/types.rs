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
    match setting
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
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
        assert_eq!(resolve_bootstrap_tier(&ollama, None), BootstrapTier::SmallLocal);
        assert_eq!(resolve_bootstrap_tier(&ollama, Some("")), BootstrapTier::SmallLocal);
        assert_eq!(resolve_bootstrap_tier(&ollama, Some("nonsense")), BootstrapTier::SmallLocal);
        assert_eq!(resolve_bootstrap_tier(&claude, None), BootstrapTier::Strong);

        // Explicit override wins, case/whitespace-insensitive.
        assert_eq!(resolve_bootstrap_tier(&ollama, Some("strong")), BootstrapTier::Strong);
        assert_eq!(resolve_bootstrap_tier(&ollama, Some("  Strong ")), BootstrapTier::Strong);
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
}
