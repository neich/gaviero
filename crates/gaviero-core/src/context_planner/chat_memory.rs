//! Tier S / S1 + S4: provider-agnostic chat memory injection.
//!
//! This module encapsulates the per-turn chat retrieval + manifest
//! persistence logic that previously lived inlined in the TUI side
//! panel. Lifting it out of `gaviero-tui` lets every chat-style call
//! site — TUI, `gaviero-cli`, future headless callers — reach the
//! foundation work that Tier S put in place.
//!
//! The helper is deliberately *not* an observer or a WriterMessage
//! producer beyond the manifest enqueue: callers fire whatever their
//! native event channel is from the returned [`ChatMemoryOutcome`].
//! That keeps this module free of TUI / CLI specific types.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value as JsonValue;

use crate::memory::{
    ChatInjection, ChatInjectionConfig, MemoryScope, MemoryStores, RerankConfig, Reranker,
    RetrievalConfig, WriterHandle, WriterMessage,
    retrieval::retrieve_for_chat_with_reranker,
};
use crate::observer::ChatInjectionSummary;

use super::types::{MemorySelection, PlannerSelections};

/// Inputs for a single chat-turn injection. All fields are borrowed —
/// the caller owns lifetimes so the helper can run inside a spawned
/// task without dictating ownership.
pub struct ChatMemoryRequest<'a> {
    /// Multi-DB registry. Required for retrieval.
    pub stores: &'a Arc<MemoryStores>,
    /// Writer task handle. `None` skips the manifest enqueue
    /// (callers without a writer still get retrieval + the rendered
    /// `<project_memory>` block).
    pub writer: Option<&'a WriterHandle>,
    pub workspace_root: &'a Path,
    /// Optional folder root if a buffer is focused — propagated into
    /// the retrieval scope so module-scope memories can rank.
    pub folder_root: Option<&'a Path>,
    /// User message that drives retrieval.
    pub user_prompt: &'a str,
    /// Identifiers used by the manifest persistence path. Both should
    /// be stable for the turn (the caller generates them when it
    /// initiates the turn).
    pub turn_id: &'a str,
    pub session_id: &'a str,
    pub injection_config: &'a ChatInjectionConfig,
    pub retrieval_config: &'a RetrievalConfig,
    pub reranker: Option<&'a dyn Reranker>,
    pub rerank_config: Option<&'a RerankConfig>,
    /// `memory.manifests.enabled` — when false the helper skips
    /// `WriterMessage::InjectionManifest` even if a writer is supplied.
    pub manifests_enabled: bool,
    /// `memory.manifests.captureCandidatePool` — when false the
    /// manifest carries selected ids only (smaller, no candidate pool).
    pub capture_candidate_pool: bool,
    /// Embedder + reranker names recorded on the manifest for
    /// retrospective re-evaluation across model swaps.
    pub embedder_name: &'a str,
    pub reranker_name: Option<&'a str>,
}

/// Result of [`perform_injection`]. Carries both the retrieval
/// outcome (so callers can splice it into `PlannerSelections`) and a
/// lightweight `ChatInjectionSummary` for observer / UI fan-out.
pub struct ChatMemoryOutcome {
    pub injection: Option<ChatInjection>,
    pub summary: ChatInjectionSummary,
}

/// Run S1 retrieval and persist the S4 manifest.
///
/// Behaviour:
/// 1. If `injection_config.enabled` is false, returns an empty
///    outcome — no retrieval, no manifest.
/// 2. Otherwise runs `retrieve_for_chat_with_reranker` over the
///    workspace + folder context.
/// 3. If retrieval surfaces any items and `manifests_enabled` and a
///    writer is supplied, enqueues `WriterMessage::InjectionManifest`
///    fire-and-forget. Manifest persistence failure never fails the
///    turn (Tier S acceptance criterion #8).
///
/// On retrieval error this returns an empty outcome and logs at
/// `warn!` — the turn proceeds without memory rather than aborting.
pub async fn perform_injection(req: ChatMemoryRequest<'_>) -> ChatMemoryOutcome {
    if !req.injection_config.enabled {
        return ChatMemoryOutcome {
            injection: None,
            summary: ChatInjectionSummary {
                items_injected: 0,
                pool_size: 0,
                tokens_used: 0,
                token_budget: req.injection_config.token_budget,
            },
        };
    }

    let scope = MemoryScope::from_context(req.workspace_root, req.folder_root, None, None);
    let injection = match retrieve_for_chat_with_reranker(
        req.stores,
        &scope,
        req.user_prompt,
        req.injection_config,
        req.retrieval_config,
        req.reranker,
        req.rerank_config,
    )
    .await
    {
        Ok(inj) => inj,
        Err(e) => {
            tracing::warn!(target: "memory_chat_injection", error = %e, "retrieve_for_chat failed");
            None
        }
    };

    let summary = ChatInjectionSummary {
        items_injected: injection.as_ref().map(|i| i.items.len()).unwrap_or(0),
        pool_size: injection.as_ref().map(|i| i.pool.len()).unwrap_or(0),
        tokens_used: injection.as_ref().map(|i| i.tokens_used).unwrap_or(0),
        token_budget: injection
            .as_ref()
            .map(|i| i.token_budget)
            .unwrap_or(req.injection_config.token_budget),
    };

    if let Some(ref inj) = injection {
        if req.manifests_enabled
            && let Some(writer) = req.writer
        {
            let payload = build_manifest_payload(
                req.user_prompt,
                inj,
                req.capture_candidate_pool,
                req.embedder_name,
                req.reranker_name,
            );
            if let Err(e) = writer.enqueue(WriterMessage::InjectionManifest {
                turn_id: req.turn_id.to_string(),
                session_id: req.session_id.to_string(),
                payload,
            }) {
                tracing::warn!(
                    target: "memory_chat_injection",
                    error = %e,
                    turn_id = req.turn_id,
                    "manifest enqueue failed (writer task terminated?)"
                );
            }
        }
    }

    ChatMemoryOutcome { injection, summary }
}

/// Tier S / S3 + Tier B / B6: post-turn writer enqueueing.
///
/// Fires `WriterMessage::TurnComplete` (extractor — Tier S / S3) and
/// `WriterMessage::TelemetryClassify` (retrieval-use — Tier B / B6),
/// each gated by its own `*_enabled` flag. Both are fire-and-forget;
/// the user is never blocked. Failures log at `warn!` only — losing a
/// post-turn enqueue is recoverable, dropping the user response is not.
///
/// Callers handle their own conversation-state bookkeeping (turn-id
/// generation, module-path resolution); this helper just enqueues.
pub struct PostTurnRequest<'a> {
    pub writer: &'a WriterHandle,
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub repo_id: &'a str,
    pub module_path: Option<String>,
    pub run_id: &'a str,
    pub transcript: String,
    pub annotations: Option<JsonValue>,
    /// Plain assistant response (post-strip), used by B6 telemetry to
    /// classify which injected memories were "used" vs "unused" via
    /// embedding cosine to the response.
    pub response_text: String,
    /// `memory.extractor.enabled` resolved for the turn's scope.
    pub extractor_enabled: bool,
    /// `memory.telemetry.enabled` resolved for the turn's scope.
    pub telemetry_enabled: bool,
}

/// Enqueue post-turn writer messages per [`PostTurnRequest`].
///
/// Returns `()` regardless of enqueue outcome — both messages are
/// intentionally fire-and-forget. Errors from a terminated writer
/// task are logged but never propagated.
pub fn enqueue_post_turn(req: PostTurnRequest<'_>) {
    if req.extractor_enabled
        && let Err(e) = req.writer.turn_complete(
            req.session_id,
            req.turn_id,
            req.repo_id,
            req.module_path,
            req.run_id,
            req.transcript,
            req.annotations,
        )
    {
        tracing::warn!(
            target: "memory_post_turn",
            error = %e,
            turn_id = req.turn_id,
            "TurnComplete enqueue failed (writer task terminated?)"
        );
    }

    if req.telemetry_enabled
        && let Err(e) =
            req.writer
                .telemetry_classify(req.turn_id, req.session_id, req.response_text)
    {
        tracing::warn!(
            target: "memory_post_turn",
            error = %e,
            turn_id = req.turn_id,
            "TelemetryClassify enqueue failed (writer task terminated?)"
        );
    }
}

/// Splice an injection into `PlannerSelections::memory_selections` as
/// a single pre-rendered block, matching the contract the provider
/// renderers expect (entries with `id.is_none() && namespace.is_none()`
/// are emitted as `content` verbatim).
///
/// No-op when the injection is `None` or carries an empty block.
pub fn splice_into_selections(
    injection: Option<ChatInjection>,
    selections: &mut PlannerSelections,
) {
    if let Some(inj) = injection
        && !inj.items.is_empty()
        && !inj.block.is_empty()
    {
        selections.memory_selections.push(MemorySelection {
            id: None,
            namespace: None,
            scope_label: None,
            score: None,
            trust: None,
            content: inj.block,
            source_hash: None,
            updated_at: None,
        });
    }
}

/// Build the JSON payload persisted to the `injection_manifests`
/// table. Schema versioning: `v1-composite` for composite-only,
/// `v2-rerank-blend` when any candidate carries a reranker score.
/// Both shapes are additive so a v1 reader can still parse v2.
fn build_manifest_payload(
    query_text: &str,
    inj: &ChatInjection,
    include_pool: bool,
    embedder_name: &str,
    reranker_name: Option<&str>,
) -> JsonValue {
    let selected_ids: Vec<i64> = inj.items.iter().map(|m| m.id).collect();
    let any_reranked = inj.pool.iter().any(|c| c.rerank_score.is_some());
    let schema_version = if any_reranked { 2 } else { 1 };
    let scoring_formula_version = if any_reranked {
        "v2-rerank-blend"
    } else {
        "v1-composite"
    };

    let mut payload = serde_json::json!({
        "schema_version": schema_version,
        "scoring_formula_version": scoring_formula_version,
        "embedder_name": embedder_name,
        "query_text": query_text,
        "selected_ids": selected_ids,
        "token_budget_used": inj.tokens_used,
        "token_budget_limit": inj.token_budget,
    });

    if let Some(name) = reranker_name {
        payload["reranker_name"] = JsonValue::String(name.to_string());
    }

    // Stable scope distribution summary keyed by scope label so the
    // panel and CLI can render cross-scope balance per turn without
    // re-scanning the pool. BTreeMap is intentional — diffs across
    // replays compare identically.
    {
        use std::collections::BTreeMap;
        let mut by_scope: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for c in &inj.pool {
            let entry = by_scope.entry(c.scope_label.clone()).or_default();
            entry.0 += 1;
            if c.selected {
                entry.1 += 1;
            }
        }
        let dist: Vec<JsonValue> = by_scope
            .into_iter()
            .map(|(label, (in_pool, selected))| {
                serde_json::json!({
                    "scope_label": label,
                    "count_in_pool": in_pool,
                    "count_selected": selected,
                })
            })
            .collect();
        payload["scope_distribution"] = JsonValue::Array(dist);
    }

    if include_pool {
        let pool: Vec<JsonValue> = inj
            .pool
            .iter()
            .map(|c| {
                serde_json::json!({
                    "memory_id": c.memory_id,
                    "scope_label": c.scope_label,
                    "namespace": c.namespace,
                    "raw_similarity": c.raw_similarity,
                    "composite_score": c.composite_score,
                    "selected": c.selected,
                    "exclusion_reason": c.exclusion_reason,
                    "rerank_score": c.rerank_score,
                    "blended_score": c.blended_score,
                })
            })
            .collect();
        payload["candidate_pool"] = JsonValue::Array(pool);
    }

    payload
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryServices, ScopeMix};

    fn null_injection_config(enabled: bool) -> ChatInjectionConfig {
        ChatInjectionConfig {
            enabled,
            scopes: ScopeMix {
                workspace: true,
                repo: true,
                module: true,
                global: false,
            },
            max_items: 5,
            token_budget: 1000,
            min_similarity: 0.0,
        }
    }

    fn null_retrieval_config() -> RetrievalConfig {
        RetrievalConfig::default()
    }

    #[tokio::test]
    async fn disabled_config_returns_empty_outcome_without_touching_writer() {
        let services = MemoryServices::for_tests_in_memory().unwrap();
        let cfg = null_injection_config(false);
        let rcfg = null_retrieval_config();
        let outcome = perform_injection(ChatMemoryRequest {
            stores: &services.stores,
            writer: Some(&services.writer),
            workspace_root: std::path::Path::new("/tmp/wsx"),
            folder_root: None,
            user_prompt: "hello",
            turn_id: "t-1",
            session_id: "s-1",
            injection_config: &cfg,
            retrieval_config: &rcfg,
            reranker: None,
            rerank_config: None,
            manifests_enabled: true,
            capture_candidate_pool: true,
            embedder_name: "null",
            reranker_name: None,
        })
        .await;
        assert!(outcome.injection.is_none());
        assert_eq!(outcome.summary.items_injected, 0);
        assert_eq!(outcome.summary.token_budget, 1000);
    }

    #[tokio::test]
    async fn post_turn_disabled_flags_skip_enqueue() {
        // Both flags off → no writer messages, no panics, no errors.
        let services = MemoryServices::for_tests_in_memory().unwrap();
        let depth_before = services.writer.queue_depth();
        enqueue_post_turn(PostTurnRequest {
            writer: &services.writer,
            session_id: "s-1",
            turn_id: "t-1",
            repo_id: "abc",
            module_path: None,
            run_id: "r-1",
            transcript: "user: hi\nassistant: hi back".into(),
            annotations: None,
            response_text: "hi back".into(),
            extractor_enabled: false,
            telemetry_enabled: false,
        });
        // The writer task is async; just confirm we didn't enqueue
        // anything on the synchronous side.
        assert_eq!(services.writer.queue_depth(), depth_before);
    }

    #[tokio::test]
    async fn empty_store_yields_empty_injection() {
        let services = MemoryServices::for_tests_in_memory().unwrap();
        let cfg = null_injection_config(true);
        let rcfg = null_retrieval_config();
        let outcome = perform_injection(ChatMemoryRequest {
            stores: &services.stores,
            writer: Some(&services.writer),
            workspace_root: std::path::Path::new("/tmp/wsx"),
            folder_root: None,
            user_prompt: "any prompt",
            turn_id: "t-1",
            session_id: "s-1",
            injection_config: &cfg,
            retrieval_config: &rcfg,
            reranker: None,
            rerank_config: None,
            manifests_enabled: true,
            capture_candidate_pool: true,
            embedder_name: "null",
            reranker_name: None,
        })
        .await;
        // No writes → retrieval surfaces nothing → injection is None.
        assert_eq!(outcome.summary.items_injected, 0);
    }
}
