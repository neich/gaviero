//! Context planner (M1 of PROVIDER_PLAN_V9).
//!
//! The planner is the single owner of bootstrap / delta / replay policy.
//! It consumes a [`PlannerInput`] and returns structured [`PlannerSelections`].
//! Final prompt formatting lives at the provider edge — V9 §0 rule 5.
//!
//! M1 scope: byte-identical behavior to today via thin adapters in
//! `swarm/backend/shared.rs` and `gaviero-tui/src/app/session.rs`.
//! The planner does the same memory/graph queries today's call sites do
//! and wraps the legacy concatenated strings in single-entry `MemorySelection`
//! / `GraphSelection` records. M3 will widen to one entry per ranked item.

pub mod ledger;
pub mod types;

pub use ledger::{
    CompactionRecord, ContentDigest, GraphDecision, PlannerFingerprint, Role, SessionLedger,
};
pub use types::{
    build_provider_profile, ContinuityHandle, ContinuityMode, FileAttachment, GraphConfidence,
    GraphSelection, GraphSelectionKind, MemorySelection, ModelSpec, PlannerInput, PlannerMetadata,
    PlannerSelections, Provider, ProviderProfile, ReplayPayload, RuntimeConfig, Symbol,
};

use std::path::Path;

use anyhow::Result;

use crate::memory::MemoryStore;
use crate::repo_map::RepoMap;

/// Owns bootstrap / delta / replay policy.
///
/// V9 §4 type. M1 only consults `memory` and `repo_map`; M3 adds structured
/// candidate APIs and `graph_store` (impact queries) wiring.
pub struct ContextPlanner<'a> {
    pub memory: Option<&'a MemoryStore>,
    pub repo_map: Option<&'a RepoMap>,
    pub ledger: &'a mut SessionLedger,
    pub workspace_root: &'a Path,
}

impl<'a> ContextPlanner<'a> {
    /// Run a single planning pass.
    ///
    /// **M1 behavior is intentionally a 1:1 translation of today's chat and
    /// swarm assembly logic** — see V9 §11 M1 acceptance ("M0 metrics
    /// unchanged"). Adapters in `shared.rs` (swarm) and `app/session.rs`
    /// (chat) render the resulting selections back into the same prompt
    /// strings the existing backends consume.
    pub async fn plan(&mut self, input: &PlannerInput<'_>) -> Result<PlannerSelections> {
        let is_first_turn = self.ledger.is_first_turn();
        let mut selections = PlannerSelections {
            memory_selections: Vec::new(),
            graph_selections: Vec::new(),
            file_refs: Vec::new(),
            replay_history: None,
            metadata: PlannerMetadata {
                memory_count: 0,
                graph_token_estimate: 0,
                graph_budget: input.graph_budget_tokens,
                is_first_turn,
                continuity_mode: Some(self.ledger.continuity_mode),
            },
        };

        // Bootstrap-only injection. Today's chat path skips memory + graph
        // on follow-up turns (Claude --resume holds context server-side);
        // today's swarm path always treats every attempt as a fresh first
        // turn (no persistence). One-shot ledger semantics for swarm match
        // because each attempt builds a fresh ledger.
        if is_first_turn {
            self.collect_memory(input, &mut selections).await;
            self.collect_graph(input, &mut selections);
            self.collect_pre_fetched_impact(input, &mut selections);
        }

        // file_refs always pass through — chat parses @file mentions on
        // every turn today.
        for (path, content) in input.file_ref_blobs {
            selections.file_refs.push(FileAttachment {
                path: std::path::PathBuf::from(path),
                content: Some(content.clone()),
            });
        }

        // Replay history for StatelessReplay providers. Today's chat path
        // sends history only on first turn (skip-bootstrap behavior); follow-
        // up turns send empty history regardless of continuity mode. M9 will
        // change this for Ollama (real client-side replay); preserve current
        // behavior here for byte-identity.
        if is_first_turn {
            selections.replay_history = match input.provider_profile.continuity_mode {
                ContinuityMode::StatelessReplay => Some(ReplayPayload {
                    entries: self.ledger.replay_history.clone(),
                }),
                _ => None,
            };
        }

        Ok(selections)
    }

    async fn collect_memory(
        &self,
        input: &PlannerInput<'_>,
        out: &mut PlannerSelections,
    ) {
        // Chat path passes pre-fetched memory context (computed via
        // search_context inside the spawn task). M1 wraps it verbatim.
        if let Some(text) = input.pre_fetched_memory_context {
            if !text.is_empty() {
                out.memory_selections.push(MemorySelection {
                    id: None,
                    namespace: None,
                    scope_label: None,
                    score: None,
                    trust: None,
                    content: text.to_string(),
                    source_hash: None,
                    updated_at: None,
                });
                out.metadata.memory_count = 1;
            }
            return;
        }
        // Swarm path: planner queries the memory store directly.
        let Some(mem) = self.memory else { return };
        if input.read_namespaces.is_empty() {
            return;
        }
        let query = input.memory_query_override.unwrap_or(input.user_message);
        let ctx = mem.search_context(input.read_namespaces, query, input.memory_limit).await;
        if ctx.is_empty() {
            return;
        }
        out.memory_selections.push(MemorySelection {
            id: None,
            namespace: None,
            scope_label: None,
            score: None,
            trust: None,
            content: ctx,
            source_hash: None,
            updated_at: None,
        });
        out.metadata.memory_count = 1;
    }

    fn collect_graph(&self, input: &PlannerInput<'_>, out: &mut PlannerSelections) {
        // Chat path passes the entire graph block pre-rendered. Use as-is.
        if let Some(text) = input.pre_fetched_graph_context {
            if !text.is_empty() {
                out.graph_selections.push(GraphSelection {
                    path: None,
                    kind: GraphSelectionKind::OutlineOnly,
                    token_estimate: 0,
                    content: text.to_string(),
                    rank_score: None,
                    confidence: None,
                    symbols: Vec::new(),
                    content_digest: None,
                });
            }
            return;
        }
        // Swarm path: planner queries RepoMap directly.
        let Some(rm) = self.repo_map else { return };
        if input.seed_paths.is_empty() || input.graph_budget_tokens == 0 {
            return;
        }
        let seeds: Vec<String> = input
            .seed_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let plan = rm.rank_for_agent(&seeds, input.graph_budget_tokens);
        if plan.repo_outline.is_empty() {
            return;
        }
        out.metadata.graph_token_estimate = plan.token_estimate;
        out.graph_selections.push(GraphSelection {
            path: None,
            kind: GraphSelectionKind::OutlineOnly,
            token_estimate: plan.token_estimate,
            content: plan.repo_outline,
            rank_score: None,
            confidence: None,
            symbols: Vec::new(),
            content_digest: None,
        });
    }

    fn collect_pre_fetched_impact(
        &self,
        input: &PlannerInput<'_>,
        out: &mut PlannerSelections,
    ) {
        let Some(text) = input.pre_fetched_impact_text else { return };
        if text.is_empty() {
            return;
        }
        out.graph_selections.push(GraphSelection {
            path: None,
            kind: GraphSelectionKind::OutlineOnly,
            token_estimate: 0,
            content: text.to_string(),
            rank_score: None,
            confidence: None,
            symbols: Vec::new(),
            content_digest: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_profile() -> ProviderProfile {
        build_provider_profile(
            &ModelSpec::parse("claude-code:sonnet"),
            &RuntimeConfig::default(),
        )
    }

    #[tokio::test]
    async fn plan_with_no_inputs_is_empty() {
        let profile = fixture_profile();
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        let workspace = std::path::PathBuf::from("/tmp");
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let input = PlannerInput {
            user_message: "hello",
            explicit_refs: &[],
            seed_paths: &[],
            provider_profile: &profile,
            read_namespaces: &[],
            graph_budget_tokens: 0,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            pre_fetched_impact_text: None,
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
        };
        let sel = planner.plan(&input).await.unwrap();
        assert!(sel.memory_selections.is_empty());
        assert!(sel.graph_selections.is_empty());
        assert!(sel.file_refs.is_empty());
        // Claude is NativeResume → no replay payload even on first turn.
        assert!(sel.replay_history.is_none());
        assert!(sel.metadata.is_first_turn);
    }

    #[tokio::test]
    async fn stateless_replay_first_turn_emits_empty_payload() {
        let profile = build_provider_profile(
            &ModelSpec::parse("ollama:llama3.1"),
            &RuntimeConfig::default(),
        );
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        let workspace = std::path::PathBuf::from("/tmp");
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let input = PlannerInput {
            user_message: "hi",
            explicit_refs: &[],
            seed_paths: &[],
            provider_profile: &profile,
            read_namespaces: &[],
            graph_budget_tokens: 0,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            pre_fetched_impact_text: None,
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
        };
        let sel = planner.plan(&input).await.unwrap();
        // StatelessReplay emits Some(_) on first turn even when empty —
        // the adapter then maps it to Vec::new() for legacy backends.
        assert!(sel.replay_history.is_some());
        assert!(sel.replay_history.unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn follow_up_turn_skips_bootstrap() {
        let profile = fixture_profile();
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        ledger.record_turn_dispatched(); // simulate completed turn 1
        let workspace = std::path::PathBuf::from("/tmp");
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let input = PlannerInput {
            user_message: "hi again",
            explicit_refs: &[],
            seed_paths: &[],
            provider_profile: &profile,
            read_namespaces: &[],
            graph_budget_tokens: 0,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            pre_fetched_impact_text: Some("ignored on follow-up"),
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
        };
        let sel = planner.plan(&input).await.unwrap();
        assert!(sel.memory_selections.is_empty());
        assert!(sel.graph_selections.is_empty(), "graph must skip on follow-up turn");
        assert!(!sel.metadata.is_first_turn);
    }

    #[tokio::test]
    async fn pre_fetched_impact_is_added_when_first_turn() {
        let profile = fixture_profile();
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        let workspace = std::path::PathBuf::from("/tmp");
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let input = PlannerInput {
            user_message: "hi",
            explicit_refs: &[],
            seed_paths: &[],
            provider_profile: &profile,
            read_namespaces: &[],
            graph_budget_tokens: 0,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            pre_fetched_impact_text: Some("[Impact analysis] foo.rs touches bar.rs"),
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
        };
        let sel = planner.plan(&input).await.unwrap();
        assert_eq!(sel.graph_selections.len(), 1);
        assert!(sel.graph_selections[0].content.contains("Impact"));
    }
}
