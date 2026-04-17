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

pub mod compaction;
pub mod ledger;
pub mod types;

pub use compaction::{CompactionPolicy, compact_replay, should_compact};
pub use ledger::{
    CompactionRecord, ContentDigest, GraphDecision, PlannerFingerprint, Role, SessionLedger,
};
pub use types::{
    ContinuityHandle, ContinuityMode, FileAttachment, GraphConfidence, GraphSelection,
    GraphSelectionKind, MemorySelection, ModelSpec, PlannerInput, PlannerMetadata,
    PlannerSelections, Provider, ProviderProfile, ReplayPayload, RuntimeConfig, Symbol,
    build_provider_profile,
};

use std::path::Path;

use anyhow::Result;

use crate::memory::MemoryStore;
use crate::repo_map::RepoMap;

/// Map a `GraphDecision` to the planner-side `GraphSelectionKind`.
fn graph_decision_to_kind(d: GraphDecision) -> GraphSelectionKind {
    match d {
        GraphDecision::PathOnly => GraphSelectionKind::PathOnly,
        GraphDecision::SignatureOnly => GraphSelectionKind::SignatureOnly,
        GraphDecision::OutlineOnly => GraphSelectionKind::OutlineOnly,
        GraphDecision::FullAttach => GraphSelectionKind::FullContent,
    }
}

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

    async fn collect_memory(&mut self, input: &PlannerInput<'_>, out: &mut PlannerSelections) {
        // Chat path can still pass pre-fetched memory context (M1/M2 carrier).
        // M3 keeps the carrier as a fallback for callers not yet migrated;
        // M10 deletes it.
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
        // M3: planner queries structured `MemoryCandidate`s directly.
        let Some(mem) = self.memory else { return };
        if input.read_namespaces.is_empty() {
            return;
        }
        let query = input.memory_query_override.unwrap_or(input.user_message);
        let candidates = mem
            .search_candidates(input.read_namespaces, query, input.memory_limit)
            .await;
        if candidates.is_empty() {
            return;
        }
        // One MemorySelection per candidate (V9 §11 M3 acceptance:
        // "planner logs show memory IDs ... per selection").
        for c in &candidates {
            self.ledger.injected_memory_ids.insert(c.id);
            tracing::info!(
                target: "turn_metrics",
                memory_id = c.id,
                memory_score = c.score,
                memory_namespace = %c.namespace,
                "memory_candidate"
            );
            out.memory_selections.push(MemorySelection {
                id: Some(c.id),
                namespace: Some(c.namespace.clone()),
                scope_label: Some(c.scope_label.clone()),
                score: Some(c.score),
                trust: c.trust.clone(),
                content: c.content.clone(),
                source_hash: c.source_hash.clone(),
                updated_at: c.updated_at.clone(),
            });
        }
        out.metadata.memory_count = candidates.len();
    }

    fn collect_graph(&mut self, input: &PlannerInput<'_>, out: &mut PlannerSelections) {
        // Chat path can still pass a pre-rendered graph block (M1/M2 carrier).
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
        // M3: planner queries `Vec<GraphCandidate>` directly and records
        // per-file decisions in the ledger (V9 §11 M3 acceptance: "Ledger
        // distinguishes attached vs outline-only files").
        let Some(rm) = self.repo_map else { return };
        if input.seed_paths.is_empty() || input.graph_budget_tokens == 0 {
            return;
        }
        let seeds: Vec<String> = input
            .seed_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let candidates = rm.rank_for_agent_structured(&seeds, input.graph_budget_tokens);
        if candidates.is_empty() {
            return;
        }
        let mut total_tokens = 0usize;
        for c in &candidates {
            total_tokens = total_tokens.saturating_add(c.token_estimate);
            self.ledger
                .injected_graph_decisions
                .insert(c.path.clone(), c.decision);
            match c.decision {
                GraphDecision::FullAttach => {
                    self.ledger.injected_file_digests.insert(
                        c.path.clone(),
                        crate::context_planner::ledger::ContentDigest(
                            c.content_digest.clone().unwrap_or_default(),
                        ),
                    );
                }
                GraphDecision::OutlineOnly
                | GraphDecision::SignatureOnly
                | GraphDecision::PathOnly => {
                    self.ledger.injected_outline_paths.insert(c.path.clone());
                }
            }
            out.graph_selections.push(GraphSelection {
                path: Some(c.path.clone()),
                kind: graph_decision_to_kind(c.decision),
                token_estimate: c.token_estimate,
                content: c.rendered_line.clone(),
                rank_score: Some(c.rank_score),
                confidence: Some(c.confidence),
                symbols: c
                    .symbols
                    .iter()
                    .map(|s| crate::context_planner::types::Symbol {
                        name: s.name.clone(),
                        kind: s.kind.clone(),
                    })
                    .collect(),
                content_digest: c.content_digest.clone(),
            });
        }
        out.metadata.graph_token_estimate = total_tokens;
    }

    fn collect_pre_fetched_impact(&self, input: &PlannerInput<'_>, out: &mut PlannerSelections) {
        let Some(text) = input.pre_fetched_impact_text else {
            return;
        };
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
        assert!(
            sel.graph_selections.is_empty(),
            "graph must skip on follow-up turn"
        );
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
