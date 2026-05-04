# Test audit

Scope reviewed:
- All `tests/` directories under `crates/{gaviero-core,gaviero-cli,gaviero-dsl}`.
- All `#[cfg(test)] mod tests` blocks inside `crates/gaviero-core/src/**` and the other crates' `src/**`.
- Settled module surface per `reviews/latest/inventory.md` + `apply-1..6.md`.

What's already covered well: memory subsystem (multi-DB registry, writer task, retrieval, eval, sleeptime, telemetry — `multi_db_memory.rs`, `tier_b_integration.rs`, `headless_memory_services.rs`, plus heavy E2E in `memory_session_e2e.rs` / `memory_testbed_e2e.rs` gated `#[ignore]`); DSL compile (27 lib + 2 swarm-contract tests); per-module unit suites for `path_pattern`, `swarm::validation`, `swarm::backend::*`, `acp::protocol`, etc.

What's thin: the swarm orchestration and post-merge surface — almost nothing exercises `pipeline::execute`, the coordinator → DSL → execute path, branch/worktree lifecycle, the validation-gate pipeline, the repo-map graph builder, or the MCP server's actual JSON-RPC path. Integration tests for `agent_session::registry`, `WriteGatePipeline` proposal lifecycle through the runner, and the brand-new `cleanup_gaviero_branches` (PR #119) are missing entirely.

## Coverage gaps

### T1: `cleanup_gaviero_branches` has no integration test
Severity: critical
Module: `gaviero-core::swarm::pipeline` (`cleanup_gaviero_branches`, `BranchCleanupReport`)
Surface: `pipeline::cleanup_gaviero_branches(workspace_root, dry_run)` plus the underlying `git::list_local_branches_with_prefix` / `git::delete_branch` / `git::worktree_prune` helpers. Wired to the `gaviero-cli --cleanup-branches[/--force]` CLI surface.
Proposed test: Init a real git repo in a tempdir, commit once, create branches `gaviero/foo`, `gaviero/bar`, `feature/x`. Run with `dry_run=true` and assert `matched.len()==2`, `deleted.is_empty()`. Run with `dry_run=false` and assert both `gaviero/*` deleted while `feature/x` and `main` survive. Then check out `gaviero/baz` and run again — `skipped_current` must contain `gaviero/baz` and the branch must remain.
Why integration not unit: the function composes three git CLI/`git2` helpers and depends on the real on-disk branch state machinery; mocking would substitute exactly the behaviour under test.

### T2: `swarm::pipeline::execute` scope-validation gate is unverified end-to-end
Severity: critical
Module: `gaviero-core::swarm::pipeline`
Surface: `execute(plan, config, ...)` validation phase — `validation::validate_scopes` integrated into the orchestrator before any backend dispatch.
Proposed test: Build a `CompiledPlan` (via `CompiledPlan::from_work_units`) with two units sharing `owned_paths=["src/foo.rs"]` and no loop groups. Call `execute` with a noop `SwarmObserver` and `make_observer` returning a NoopAcpObserver. Assert the call returns `Err(_)` whose message contains `"scope validation"`. (No backend should be reachable — the gate must abort first.)
Why integration not unit: the orchestrator wires `validate_scopes`, plan extraction, observer notifications, and short-circuit error propagation; unit tests on `validate_scopes` alone don't prove this wiring survives refactors.

### T3: `WorktreeManager::provision` + `Drop` cleanup lifecycle is untested
Severity: critical
Module: `gaviero-core::git`
Surface: `WorktreeManager::new` → `provision(agent_id)` → `WorktreeHandle` → `Drop` (`teardown_all`).
Proposed test: Init a git repo with one commit. Construct `WorktreeManager`, call `provision("agent-x")`, assert the returned handle's path exists on disk and `gaviero/agent-x` shows up in `list_local_branches_with_prefix(_, "gaviero/")`. Drop the manager and assert the worktree directory has been removed (the branch may persist — that's expected).
Why integration not unit: real git CLI invocations and filesystem state changes are exactly the thing under test; the `Drop` semantics are visible only end-to-end.

### T4: `ExecutionState` save/load checkpoint round-trip is unverified
Severity: high
Module: `gaviero-core::swarm::execution_state`
Surface: `ExecutionState::new_from_plan` → `record_result` → `save(plan_hash)` → `load(plan_hash)`. Backs `gaviero-cli --resume`.
Proposed test: Build a tiny `CompiledPlan`, create an `ExecutionState`, mark one node Completed via `record_result(... AgentManifest{Completed})` and another HardFailure, change CWD to a tempdir, call `save(&plan_hash)`, then `load(&plan_hash)` and assert the recovered state has the same per-node statuses and cumulative `cost_estimate_usd`.
Why integration not unit: `save`/`load` use a hardcoded `.gaviero/state/` path relative to CWD; testing the JSON-on-disk round-trip with real serde + tempdir is the only way to catch breakage there.

### T5: Coordinator DSL output → `gaviero_dsl::compile` contract has no test
Severity: high
Module: `gaviero-core::swarm::coordinator` × `gaviero-dsl::compiler`
Surface: The `.gaviero` shape `build_coordinator_dsl_prompt` instructs the model to produce must compile cleanly through `gaviero_dsl::compile` and pass `swarm::validation::validate_scopes`. Today nothing pins the round-trip — a future prompt edit can silently produce DSL that the compiler rejects.
Proposed test: In `gaviero-dsl/tests/swarm_contract.rs` add a test whose source mirrors the coordinator's emit template (one `client` per tier, two agents with `depends_on`, one `workflow main { steps [...] }`). Compile via `gaviero_dsl::compile`, take `work_units_ordered`, run them through `swarm::validation::validate_scopes` and `swarm::validation::dependency_tiers`, assert no errors and ≥2 tiers.
Why integration not unit: the contract spans the DSL crate and `gaviero-core`'s validation crate boundaries; per-crate unit tests can only see one side of the seam.

**Bug surfaced while writing T5**: the coordinator's prompt template at `crates/gaviero-core/src/swarm/coordinator.rs:953-956` instructs models to use `reasoning` / `execution` / `mechanical` as **client names**, but those identifiers are reserved tier-value keywords by the DSL lexer (`crates/gaviero-dsl/src/lexer.rs:128-135`). A model that follows the prompt instructions produces DSL that fails to parse with `syntax error: found 'reasoning' expected something else`. The implementation captures this regression as `coordinator_prompt_uses_reserved_tier_words_as_client_names` so a future fix (either renaming the prompt's recommended client names or unreserving those tokens) flips the test from pass to fail and prompts removal of the guard.

### T6: `agent_session::registry::create_session` routing has no test
Severity: high
Module: `gaviero-core::agent_session::registry`
Surface: `create_session(SessionConstruction)` dispatches on `profile.continuity_mode` and `profile.provider` to four concrete session types (`ClaudeSession`, `CodexAppServerSession`, `CodexExecSession`, `OllamaSession`).
Proposed test: For each `(provider, continuity_mode)` pair from `build_provider_profile` — claude/`NativeResume`, codex-app-server/`ProcessBound`, codex/`StatelessReplay`, ollama/`StatelessReplay` — call `create_session` with a stub observer and a noop write-gate, then assert the returned `Box<dyn AgentSession>::continuity_mode()` matches. Drop without `send_turn` so no subprocess is spawned.
Why integration not unit: the routing branches across three modules' constructors, and `pub(super)` visibility means the mapping can only be exercised through `registry::create_session`.

### T7: `ValidationPipeline::fast_only` has no end-to-end test against real Rust
Severity: high
Module: `gaviero-core::validation_gate`
Surface: `ValidationPipeline::fast_only().run(files, workdir, true)` — the per-write tree-sitter syntax gate consumed by `swarm::backend::runner` after every agent turn.
Proposed test: tempdir with `src/ok.rs` (`fn main() {}`) → pipeline → returns `None` (all pass). Then `src/bad.rs` (`fn broken( {`) → pipeline → returns `Some(("tree-sitter", ValidationResult::Fail{message}))` whose `message` references the file path and a line number.
Why integration not unit: the gate trait, the structural verifier, and the tree-sitter language registry have to interlock on real on-disk content; the in-source unit tests cover each in isolation.

### T8: `repo_map::graph_builder::build_graph` has no integration coverage
Severity: high
Module: `gaviero-core::repo_map::graph_builder`
Surface: `build_graph(workspace, excludes) -> (GraphStore, BuildResult)` plus `GraphStore::impact_radius`. Backs the `gaviero-cli --graph` flag and the MCP `blast_radius` tool.
Proposed test: Tempdir with three small Rust files where one calls another. Run `build_graph`, assert `result.total_nodes > 0` and `result.total_edges > 0`. Run a second `build_graph` over the same dir and assert `result.files_changed == 0` (incremental no-op). Call `impact_radius(&["src/lib.rs"], 2)` and assert the returned `ImpactSummary.affected_files` contains the caller file.
Why integration not unit: the builder composes tree-sitter parsing, sha2 hashing, SQLite persistence, and the incremental diff; nothing else exercises the full pipeline.

### T9: `swarm::pipeline::revert_swarm` has no test
Severity: medium
Module: `gaviero-core::swarm::pipeline`
Surface: `revert_swarm(workspace_root, &SwarmResult)` — destructive `git reset --hard <pre_swarm_sha>` + branch deletion.
Proposed test: Init repo, commit A; create branch `gaviero/foo` at commit B (after another file write + commit). Build a synthetic `SwarmResult { manifests: [{branch:Some("gaviero/foo"), ..}], pre_swarm_sha: <A> }`. Call `revert_swarm`, assert HEAD == A, branch `gaviero/foo` deleted.
Why integration not unit: behaviour is destructive git state mutation; mocking removes the only thing under test.

### T10: MCP server end-to-end JSON-RPC `tools/call` is unverified
Severity: medium
Module: `gaviero-core::mcp::server`
Surface: `spawn_mcp_server` listener + the three read-only tools (`memory_search`, `blast_radius`, `node_doc`) over a Unix socket. Existing test only verifies socket bind + accept; nothing speaks the actual MCP protocol.
Proposed test: Spawn the server, connect via `rmcp` client (already a dep) over the Unix socket, call `tools/list` (expect 3), then `tools/call memory_search { query: "x" }` against an in-memory store seeded with one row, assert the response is a `MemorySearchOutput` with `results.len() >= 1`.
Why integration not unit: the JSON-RPC framing, the `rmcp::tool_router` macro dispatch, and the request/response serde shapes can only be exercised through a real client.

### T11: `swarm::merge::merge_branch` and `auto_resolve_conflicts` are uncovered
Severity: medium
Module: `gaviero-core::swarm::merge`
Surface: `merge_branch(repo, branch)` non-conflict path; `auto_resolve_conflicts` orchestration around `resolve_conflict` + `complete_merge`.
Proposed test (non-conflict path only — `auto_resolve_conflicts` needs a real LLM and is best left `#[ignore]`-gated): Init repo with main branch; create `gaviero/foo` branch with a non-conflicting file edit; checkout main; call `merge_branch(repo, "gaviero/foo")` and assert `MergeResult{success:true, conflicts:[]}`. Add a second test where the merge conflicts and assert `MergeResult{success:false, conflicts.len()>=1}`.
Why integration not unit: the function shells out to the git CLI; testing means running git.

### T12: `WriteGatePipeline` proposal-finalize lifecycle through the runner has no end-to-end test
Severity: medium
Module: `gaviero-core::write_gate` × `gaviero-core::swarm::backend::runner`
Surface: `Interactive` + `Deferred` modes — proposal created → reviewer accepts/rejects → `finalize` produces `AutoAcceptAction::Write/Delete` → caller writes to disk. The runner exercises only the AutoAccept path; Interactive flow only has unit tests on the gate itself.
Proposed test: Drive a `MockBackend` emitting one `FileBlock` through `swarm::backend::runner::run_backend` with `WriteMode::Interactive`. After `run_backend` returns, assert the write gate has one pending proposal with the correct path and content. Call `accept_all` + `finalize`, write the resulting `AutoAcceptAction` to disk, and assert the file content matches.
Why integration not unit: the AcpObserver/runner/gate triangle is what carries the proposal between the agent and the disk write; testing that triangle against a real `MockBackend` stream is the only way to pin it.

### T13: `swarm::context::build_context` has no coverage of memory + repo merger
Severity: medium
Module: `gaviero-core::swarm::context`
Surface: `build_context(memory, paths, ...)` — composes the legacy planner's pre-prompt context. Per-file helpers have unit tests but the merged output isn't exercised.
Proposed test: tempdir with two source files; in-memory `MemoryStores` with two seeded rows. Call `build_context`, assert the returned `RepoContext` has the file list and memory snippet rendered together. Single-pass smoke test.
Why integration not unit: the function joins two subsystems (file scan + memory query); per-helper unit tests don't prove the join.

### T14: `--coordinated` mode has no end-to-end test
Severity: medium
Module: `gaviero-cli` (`Coordinator::plan_as_dsl` → write to disk → `gaviero_dsl::compile` → `pipeline::execute`)
Surface: The `gaviero-cli --coordinated` workflow that produces a `.gaviero` plan, validates it, and exits without executing.
Proposed test: Reuse `mock_response.rs`-style plumbing if available, or stage a `gaviero-cli --coordinated --task '...' --plan-output <tempfile>` invocation under `#[ignore]` (the coordinator call itself is API-bound). For now, settle for a smaller test of the seam: feed a known-shape DSL (matching the coordinator's emit format) into `gaviero_dsl::compile` and assert it's a valid plan that `pipeline::execute` accepts up to the validation gate. Covered by T5 indirectly.
Why integration not unit: the seam crosses CLI ↔ core ↔ DSL ↔ disk; nothing else exercises it.

### T15: `WriteGatePipeline::insert_proposal` Deferred-mode duplicate-path suppression is unverified
Severity: low
Module: `gaviero-core::swarm::backend::runner` (`propose_write`)
Surface: The "drop later proposal for path — earlier proposal already pending review" logic at `runner.rs:486-502` (against both `proposal_for_path` and `pending_proposals`).
Proposed test: Drive two consecutive `FileBlock`s targeting the same path through `run_backend` in `WriteMode::Deferred`. Assert only one proposal lands in `pending_proposals` and the second is silently dropped.
Why integration not unit: the dedup path runs only when `pending_proposals` is consulted by the runner; testing through `WriteGatePipeline` directly bypasses it.

## Tests to retire or rewrite

### R1: `headless_memory_services::for_tests_in_memory_writer_is_alive` is tautological
Severity: low
Module: `gaviero-core/tests/headless_memory_services.rs`
Action: rewrite
Surface: The test asserts only `services.writer.is_alive()` and `queue_depth() == 0` immediately after `MemoryServices::for_tests_in_memory()`. Both are true by construction — the writer task is spawned and nothing has been sent yet.
Recommended replacement: enqueue one `user_remember_scoped` write, drain the queue, assert `is_alive()` still true and the row appears in `search_scoped`. That actually exercises the writer task survives at least one round-trip.
Why integration: the only way to prove the writer survives a write is to drive a write through it.

### R2: `c24_trigger_disable_invariant.rs` is a CI grep-check, not an integration test
Severity: low
Module: `gaviero-core/tests/c24_trigger_disable_invariant.rs`
Action: retire (move to a workspace lint or `xtask`)
Surface: 83-line test that walks every `.rs` file under `crates/` and counts text matches of `schema::drop_history_immutable_triggers(`. Useful as a guard, but it's not an integration test — it's a literal-string grep that lives in the test runner solely to fail CI.
Recommended action: keep the invariant guard but move it out of `cargo test` (e.g. a workspace `xtask::lint_invariants`) so the test suite stays focused on behaviour rather than file-content scans. If keeping in-tree is preferred, downgrade severity in this audit and leave it.

### R3: `swarm::backend::runner::tests::test_run_backend_scope_enforcement` over-relies on AutoAccept
Severity: low
Module: `gaviero-core/src/swarm/backend/runner.rs` (in-source `mod tests`)
Action: rewrite
Surface: The test sets `WriteMode::AutoAccept` and asserts `manifest.modified_files.len() == 0` when the FileBlock targets `tests/foo.rs` (out-of-scope for a `src/` unit). That's right but hides whether the rejection was due to the scope check or the path-not-existing check earlier in `propose_write`.
Recommended replacement: enable a tracing subscriber (or a small WriteGateObserver counter) and assert the path was rejected by `is_scope_allowed`, not silently dropped by some other branch. Or split into two tests — one for scope-rejected, one for path-already-pending.

### R4: `coordinator.rs` lenient-parser tests block iter7 cleanup; flag for re-evaluation when it lands
Severity: low
Module: `gaviero-core/src/swarm/coordinator.rs` (`mod tests`)
Action: retire (when iter7's coordinator-JSON-removal applies)
Surface: `test_lenient_minimal_json` … `test_resolve_multiple_conflicts` (~15 tests). Per `apply-3.md`, the entire JSON path (`parse_task_dag_lenient`, `resolve_scope_overlaps`, `parse_work_unit_lenient`, ...) is slated for deletion in favour of `plan_as_dsl`. The tests will compile-error the moment the module shrinks.
Recommended action: leave them in place until iter7's apply lands; then delete with the rest of the JSON path. No new tests should be added against this surface in the meantime.

## Implementation

Implemented every critical and high-severity gap (T1–T8) as new integration tests under the per-crate `tests/` directories. Tests use `tempfile::tempdir`, the workspace's `MockBackend` / `MockEmbedder` patterns, and real-git plus tree-sitter / SQLite end-to-end where the function under test demands it. No network, no model files, all green on a clean cargo cache.

Files added (file → tests):
- `crates/gaviero-core/tests/swarm_branch_cleanup.rs` — T1, 4 tests.
- `crates/gaviero-core/tests/swarm_pipeline_validation.rs` — T2, 2 tests.
- `crates/gaviero-core/tests/git_worktree_lifecycle.rs` — T3, 3 tests.
- `crates/gaviero-core/tests/swarm_execution_state_checkpoint.rs` — T4, 3 tests.
- `crates/gaviero-core/tests/agent_session_registry.rs` — T6, 6 tests.
- `crates/gaviero-core/tests/validation_pipeline_e2e.rs` — T7, 4 tests.
- `crates/gaviero-core/tests/repo_map_graph_builder.rs` — T8, 4 tests.
- `crates/gaviero-dsl/tests/swarm_contract.rs` — T5 + the prompt-template regression appended inline, 3 new tests.

Tests added: 29 passing, 0 failing.

Run with:
```bash
cargo test -p gaviero-core --test swarm_branch_cleanup \
  --test swarm_pipeline_validation --test git_worktree_lifecycle \
  --test swarm_execution_state_checkpoint --test agent_session_registry \
  --test validation_pipeline_e2e --test repo_map_graph_builder
cargo test -p gaviero-dsl --test swarm_contract
```

Full workspace run is green (`cargo test --workspace` — 752 + 141 + 114 + 27 lib/unit + per-test-file totals all `ok`).
