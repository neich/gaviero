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
    // Future providers: add a variant here.
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
}

/// Parsed `<prefix>:<model>` model spec.
///
/// Constructed from the user-facing `model: String` (e.g. `"claude-code:sonnet"`).
/// Parsing mirrors `swarm/backend/shared.rs::backend_config_for_model` —
/// keep them in sync (M10 will collapse the duplication).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Raw provider prefix (`"claude"`, `"claude-code"`, `"codex"`, `"codex-cli"`,
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
        for prefix in ["ollama", "local", "codex-app-server", "codex-cli", "codex", "claude-code", "claude"] {
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
            "codex" | "codex-cli" => Provider::Codex,
            // Bare or claude-prefixed → Claude.
            _ => Provider::Claude,
        }
    }
}

/// Logical provider category. Distinct from `provider_prefix` because
/// multiple prefixes (`"claude"` / `"claude-code"`) map to the same provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Claude,
    /// `codex exec` — `StatelessReplay` fallback.
    Codex,
    /// `codex app-server` — `ProcessBound` continuity (M8).
    CodexAppServer,
    Ollama,
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
        },
        Provider::Codex => ProviderProfile {
            provider: "codex".to_string(),
            model: spec.model.clone(),
            // `codex exec` stays StatelessReplay (V9 §5). M8 adds the
            // `codex-app-server:` prefix for ProcessBound; the plain `codex:`
            // and `codex-cli:` prefixes retain this arm unchanged.
            continuity_mode: ContinuityMode::StatelessReplay,
            supports_tool_use: true,
            supports_native_resume: false,
            // M0 Finding I: Codex backend doesn't surface usage today;
            // value is informational until M8/M9 wire budgeting.
            max_context_tokens: None,
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
        },
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
            ("claude-code:sonnet", "claude-code", "sonnet", Provider::Claude),
            ("claude:opus", "claude", "opus", Provider::Claude),
            ("sonnet", "", "sonnet", Provider::Claude),
            ("codex:gpt-5-codex", "codex", "gpt-5-codex", Provider::Codex),
            ("codex-cli:o4-mini", "codex-cli", "o4-mini", Provider::Codex),
            ("ollama:qwen2.5-coder:7b", "ollama", "qwen2.5-coder:7b", Provider::Ollama),
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
    fn factory_maps_providers_to_continuity_modes() {
        // Pins V9 §5 provider mapping table.
        let runtime = RuntimeConfig::default();

        let claude = build_provider_profile(&ModelSpec::parse("claude-code:sonnet"), &runtime);
        assert_eq!(claude.continuity_mode, ContinuityMode::NativeResume);
        assert!(claude.supports_native_resume);
        assert!(claude.supports_tool_use);
        assert_eq!(claude.max_context_tokens, Some(200_000));

        let codex = build_provider_profile(&ModelSpec::parse("codex:gpt-5-codex"), &runtime);
        assert_eq!(codex.continuity_mode, ContinuityMode::StatelessReplay);
        assert!(!codex.supports_native_resume);

        // M8: codex-app-server: → ProcessBound (V9 §5 table).
        let codex_as = build_provider_profile(
            &ModelSpec::parse("codex-app-server:gpt-5-codex"),
            &runtime,
        );
        assert_eq!(codex_as.continuity_mode, ContinuityMode::ProcessBound);
        assert!(codex_as.supports_native_resume);
        assert_eq!(codex_as.provider, "codex");

        let ollama = build_provider_profile(&ModelSpec::parse("ollama:llama3.1"), &runtime);
        assert_eq!(ollama.continuity_mode, ContinuityMode::StatelessReplay);
        assert!(!ollama.supports_native_resume);

        let bare = build_provider_profile(&ModelSpec::parse("haiku"), &runtime);
        assert_eq!(bare.continuity_mode, ContinuityMode::NativeResume);
        assert_eq!(bare.provider, "claude");
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
