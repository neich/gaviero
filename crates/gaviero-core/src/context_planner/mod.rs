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

pub mod bootstrap;
pub mod chat_memory;
pub mod compaction;
pub mod ledger;
pub mod types;

pub use bootstrap::{
    BootstrapArms, BootstrapBudgets, BootstrapEstimateContext, BootstrapEstimateHints,
    BootstrapMode, BootstrapOneShot, estimate_bootstrap_tokens, resolve_chat_bootstrap_arms,
};
pub use chat_memory::{
    ChatMemoryOutcome, ChatMemoryRequest, PostTurnRequest, enqueue_post_turn, perform_injection,
    splice_into_selections,
};
pub use compaction::{CompactionPolicy, compact_replay, should_compact};
pub use ledger::{
    CompactionRecord, ContentDigest, GraphDecision, PlannerFingerprint, Role, SessionLedger,
};
pub use types::{
    BootstrapTier, ContinuityHandle, ContinuityMode, FileAttachment, GraphConfidence,
    GraphSelection, GraphSelectionKind, MemorySelection, ModelSpec, PlannerInput, PlannerMetadata,
    PlannerSelections, Provider, ProviderProfile, ReplayPayload, RuntimeConfig, SkillSelection,
    Symbol, build_provider_profile, resolve_bootstrap_tier,
};

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::memory::MemoryStores;
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
    pub memory: Option<&'a Arc<MemoryStores>>,
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
            skill_selections: Vec::new(),
            replay_history: None,
            metadata: PlannerMetadata {
                memory_count: 0,
                graph_token_estimate: 0,
                graph_budget: input.graph_budget_tokens,
                is_first_turn,
                continuity_mode: Some(self.ledger.continuity_mode),
            },
        };

        // Bootstrap injection is gated by `bootstrap_arms` resolved by the
        // caller (chat: mode + slash commands; swarm: always all layers on
        // the work unit's fresh first turn). Follow-up turns skip unless
        // `explicit` (e.g. `/inject memory` for codex exec).
        let arms = input.bootstrap_arms;
        if arms.memory {
            self.collect_memory(input, &mut selections).await;
        }
        if arms.topology {
            self.collect_topology(input, &mut selections);
        }
        if arms.outline {
            self.collect_graph(input, &mut selections);
        }
        if arms.impact {
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

        for skill in input.resolved_skills {
            selections.skill_selections.push(SkillSelection {
                name: skill.name.clone(),
                scope_level: skill.scope_level,
                rendered_body: skill.rendered_body.clone(),
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
        // M3 + Tier B: planner pulls structured candidates through the
        // central `retrieve_ranked` engine — same path as chat injection,
        // MCP, and the memory panel — so scope/trust scoring and B2
        // rerank are applied uniformly.
        let Some(mem) = self.memory else { return };
        if input.read_namespaces.is_empty() {
            return;
        }
        let query = input.memory_query_override.unwrap_or(input.user_message);
        // The planner doesn't carry an active-file path; folder = None
        // restricts the registry walk to workspace + global. The TUI
        // panel can extend this with `Workspace::folder_for_path` once
        // it threads the active editor's file in.
        //
        // Workspace-wide opt-in (`PlannerInput::extra_folder_paths`):
        // when non-empty, fan out retrieval per folder (each scope
        // yields folder + workspace + global candidates), dedupe by
        // canonical id, and re-truncate to `memory_limit`. Workspace +
        // global rows duplicate naturally across passes; the dedup
        // collapses them. This lets `/workspace` in the chat panel ask
        // the planner for memory across every workspace folder without
        // changing `MemoryScope`'s single-repo shape.
        let cfg = crate::memory::RetrievalConfig::default();
        let candidates = if input.extra_folder_paths.is_empty() {
            let scope = crate::memory::MemoryScope::from_context(
                self.workspace_root,
                None,
                None,
                None,
            );
            match crate::memory::retrieve_ranked(mem, &scope, query, input.memory_limit, &cfg, None, None).await {
                Ok(out) => out
                    .items
                    .iter()
                    .map(crate::memory::store::MemoryCandidate::from_scored)
                    .collect::<Vec<_>>(),
                Err(e) => {
                    tracing::warn!("planner retrieve_ranked failed: {e}");
                    return;
                }
            }
        } else {
            // Build one scope per folder (planner's workspace_root +
            // each extra). Oversample per scope: limit*2 keeps the
            // dedup pool meaningful without ballooning total work.
            let per_scope_limit = input.memory_limit.saturating_mul(2).max(input.memory_limit);
            let mut roots: Vec<&std::path::Path> = Vec::with_capacity(input.extra_folder_paths.len() + 1);
            roots.push(self.workspace_root);
            for p in input.extra_folder_paths {
                if !roots.iter().any(|r| *r == *p) {
                    roots.push(*p);
                }
            }
            let mut merged: Vec<crate::memory::store::MemoryCandidate> = Vec::new();
            let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
            for root in roots {
                let scope = crate::memory::MemoryScope::from_context(root, Some(root), None, None);
                match crate::memory::retrieve_ranked(mem, &scope, query, per_scope_limit, &cfg, None, None).await {
                    Ok(out) => {
                        for item in &out.items {
                            let cand = crate::memory::store::MemoryCandidate::from_scored(item);
                            if seen_ids.insert(cand.id) {
                                merged.push(cand);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "memory",
                            root = %root.display(),
                            error = %e,
                            "planner retrieve_ranked (workspace-wide) failed for folder"
                        );
                    }
                }
            }
            merged.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            merged.truncate(input.memory_limit);
            merged
        };
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

    fn collect_topology(&self, input: &PlannerInput<'_>, out: &mut PlannerSelections) {
        if !input.topology_config.enabled {
            return;
        }

        let mut blocks: Vec<String> = Vec::new();

        if let Some(body) = input.pre_fetched_topology {
            if !body.is_empty() {
                blocks.push(body.to_string());
            }
        } else {
            match crate::repo_map::build_folder_topology(
                self.workspace_root,
                &[],
                &input.topology_config,
            ) {
                Ok(body) if !body.is_empty() => blocks.push(body),
                Ok(_) => {}
                Err(e) => tracing::warn!("topology build failed: {e}"),
            }
        }

        for (label, body) in input.extra_topology_blocks {
            if body.is_empty() {
                continue;
            }
            blocks.push(format!("--- {label} ---\n{body}"));
        }

        if blocks.is_empty() {
            return;
        }

        let content = blocks.join("\n\n");
        let tokens = content.len().div_ceil(4);
        out.graph_selections.push(GraphSelection {
            path: None,
            kind: GraphSelectionKind::Topology,
            token_estimate: tokens,
            content,
            rank_score: None,
            confidence: None,
            symbols: Vec::new(),
            content_digest: None,
        });
        out.metadata.graph_token_estimate = out
            .metadata
            .graph_token_estimate
            .saturating_add(tokens);
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
        //
        // Workspace-wide opt-in (`PlannerInput::extra_repo_maps`): rank
        // candidates against each map and merge by `rank_score`. The
        // token budget is shared across the merged set — first sort by
        // score, then greedily admit until the budget is consumed. This
        // lets `/workspace` surface graph context from every workspace
        // folder without forcing the TUI to flatten N maps into one.
        // No primary map → nothing to rank. Extras alone aren't useful
        // without a primary anchor in today's chat path; the TUI always
        // builds the focused/primary folder's map first.
        let Some(rm) = self.repo_map else { return };
        if input.seed_paths.is_empty() || input.graph_budget_tokens == 0 {
            return;
        }
        let seeds: Vec<String> = input
            .seed_paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let candidates = if input.extra_repo_maps.is_empty() {
            rm.rank_for_agent_structured(&seeds, input.graph_budget_tokens)
        } else {
            // Oversample per map (each gets its own budget pass), then
            // sort merged by rank_score and greedily admit under the
            // total budget. Per-map budget = total / (1 + extras),
            // floored at 1 to avoid empty oversamples on tiny budgets.
            let map_count = 1 + input.extra_repo_maps.len();
            let per_map_budget = input
                .graph_budget_tokens
                .saturating_div(map_count.max(1))
                .max(1);
            let mut merged: Vec<crate::repo_map::GraphCandidate> = Vec::new();
            let mut seen_paths: std::collections::HashSet<std::path::PathBuf> =
                std::collections::HashSet::new();
            for c in rm.rank_for_agent_structured(&seeds, per_map_budget) {
                if seen_paths.insert(c.path.clone()) {
                    merged.push(c);
                }
            }
            for extra in input.extra_repo_maps {
                for c in extra.rank_for_agent_structured(&seeds, per_map_budget) {
                    if seen_paths.insert(c.path.clone()) {
                        merged.push(c);
                    }
                }
            }
            merged.sort_by(|a, b| {
                b.rank_score
                    .partial_cmp(&a.rank_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            // Greedy budget admit.
            let mut admitted = Vec::with_capacity(merged.len());
            let mut used: usize = 0;
            for c in merged {
                let cost = c.token_estimate;
                if used.saturating_add(cost) > input.graph_budget_tokens {
                    continue;
                }
                used = used.saturating_add(cost);
                admitted.push(c);
            }
            admitted
        };
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
            &ModelSpec::parse("claude:sonnet"),
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig {
                enabled: false,
                ..crate::repo_map::TopologyConfig::default()
            },
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms::all(),
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig::default(),
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms::all(),
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig::default(),
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms::none(),
        };
        let sel = planner.plan(&input).await.unwrap();
        assert!(sel.memory_selections.is_empty());
        assert!(
            sel.graph_selections.is_empty(),
            "graph must skip on follow-up turn"
        );
        assert!(!sel.metadata.is_first_turn);
    }

    /// Workspace-wide opt-in: planner ranks against primary + extras and
    /// merges by `rank_score`. Pin the contract so a future refactor of
    /// `collect_graph` doesn't silently drop one half of the merge.
    #[tokio::test]
    async fn workspace_wide_extra_repo_maps_contribute_to_graph_selections() {
        use crate::repo_map::RepoMap;
        use std::fs;

        let primary_dir = tempfile::tempdir().unwrap();
        let extra_dir = tempfile::tempdir().unwrap();
        // Each folder gets one source file with a distinct symbol so we
        // can assert merged selections include paths from both.
        fs::write(
            primary_dir.path().join("primary.rs"),
            "pub fn primary_thing() {}\n",
        )
        .unwrap();
        fs::write(
            extra_dir.path().join("extra.rs"),
            "pub fn extra_thing() {}\n",
        )
        .unwrap();
        let primary_map = RepoMap::build(primary_dir.path(), &[]).unwrap();
        let extra_map = RepoMap::build(extra_dir.path(), &[]).unwrap();

        let profile = fixture_profile();
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        let workspace = primary_dir.path().to_path_buf();
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: Some(&primary_map),
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let extras: [&RepoMap; 1] = [&extra_map];
        let extra_paths: [&std::path::Path; 1] = [extra_dir.path()];
        let seeds = [
            std::path::PathBuf::from("primary.rs"),
            std::path::PathBuf::from("extra.rs"),
        ];
        let input = PlannerInput {
            user_message: "what does the workspace do?",
            explicit_refs: &[],
            seed_paths: &seeds,
            provider_profile: &profile,
            read_namespaces: &[],
            graph_budget_tokens: 16_000,
            memory_query_override: None,
            memory_limit: 5,
            file_ref_blobs: &[],
            pre_fetched_impact_text: None,
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
            extra_folder_paths: &extra_paths,
            extra_repo_maps: &extras,
            topology_config: crate::repo_map::TopologyConfig::default(),
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms::all(),
        };
        let sel = planner.plan(&input).await.unwrap();
        // Exact rank counts are an implementation detail of
        // rank_for_agent_structured; what we pin is "extras contribute
        // at least one selection" — i.e. paths from both folders show
        // up in the merged set when the budget is generous.
        let rendered: String = sel
            .graph_selections
            .iter()
            .map(|g| g.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            rendered.contains("primary.rs"),
            "expected primary folder file in graph selections, got: {rendered}"
        );
        assert!(
            rendered.contains("extra.rs"),
            "expected extra folder file in graph selections (workspace-wide merge), got: {rendered}"
        );
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
            pre_fetched_impact_text: Some("Imp: foo.rs touches bar.rs"),
            pre_fetched_graph_context: None,
            pre_fetched_memory_context: None,
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig {
                enabled: false,
                ..crate::repo_map::TopologyConfig::default()
            },
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms {
                impact: true,
                ..BootstrapArms::none()
            },
        };
        let sel = planner.plan(&input).await.unwrap();
        assert_eq!(sel.graph_selections.len(), 1);
        assert!(sel.graph_selections[0].content.contains("Imp:"));
    }

    #[tokio::test]
    async fn pre_fetched_topology_on_first_turn_only() {
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
        let topo = ". (workspace: tmp)\n  src/";
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig::default(),
            pre_fetched_topology: Some(topo),
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms::topology_only(),
        };
        let sel = planner.plan(&input).await.unwrap();
        assert_eq!(sel.graph_selections.len(), 1);
        assert_eq!(sel.graph_selections[0].kind, GraphSelectionKind::Topology);
        assert!(sel.graph_selections[0].content.contains("src/"));

        ledger.record_turn_dispatched();
        let mut planner2 = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let input2 = PlannerInput {
            user_message: "hi again",
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig::default(),
            pre_fetched_topology: Some(topo),
            extra_topology_blocks: &[],
            resolved_skills: &[],
            bootstrap_arms: BootstrapArms::none(),
        };
        let sel2 = planner2.plan(&input2).await.unwrap();
        assert!(sel2.graph_selections.is_empty());
    }

    #[tokio::test]
    async fn resolved_skills_pass_through_on_follow_up_turn() {
        let profile = fixture_profile();
        let fp = PlannerFingerprint::from_profile(&profile);
        let mut ledger = SessionLedger::new(&profile, fp);
        ledger.record_turn_dispatched();
        let workspace = std::path::PathBuf::from("/tmp");
        let mut planner = ContextPlanner {
            memory: None,
            repo_map: None,
            ledger: &mut ledger,
            workspace_root: &workspace,
        };
        let resolved = vec![crate::skills::ResolvedSkill {
            name: "lint".to_string(),
            scope_level: crate::memory::scope::SCOPE_REPO,
            rendered_body: "run clippy".to_string(),
        }];
        let input = PlannerInput {
            user_message: "follow up",
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
            extra_folder_paths: &[],
            extra_repo_maps: &[],
            topology_config: crate::repo_map::TopologyConfig::default(),
            pre_fetched_topology: None,
            extra_topology_blocks: &[],
            resolved_skills: &resolved,
            bootstrap_arms: BootstrapArms::none(),
        };
        let sel = planner.plan(&input).await.unwrap();
        assert!(sel.memory_selections.is_empty());
        assert!(sel.graph_selections.is_empty());
        assert_eq!(sel.skill_selections.len(), 1);
        assert_eq!(sel.skill_selections[0].name, "lint");
    }
}
