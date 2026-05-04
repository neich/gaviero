# Repository inventory

MODULE_COUNT: 6

## Module 1 — gaviero-mcp-shim
Path: crates/gaviero-mcp-shim
Purpose: Standalone binary (`gaviero-mcp-shim`) that bridges subprocess agents (Claude Code, Codex) to Gaviero's in-process MCP server. Performs bidirectional `tokio::io::copy` between its own stdio and `<workspace>/.gaviero/mcp.sock`, with reconnect/backoff up to a configurable `--connect-timeout-secs`. Pure transport: no MCP protocol parsing, no workspace dependencies — the smallest, most isolated artifact in the repo. Listed first because it has zero in-edges and zero workspace out-edges, so a replan here cannot cascade.
Languages: Rust
Internal deps: (none)
Key entry points: crates/gaviero-mcp-shim/src/main.rs

## Module 2 — tree-sitter-gaviero
Path: crates/tree-sitter-gaviero
Purpose: Tree-sitter grammar for the `.gaviero` workflow DSL. Hand-maintained in `grammar.js`; the generated `parser.c`, `grammar.json`, `node-types.json` are committed and a Rust binding exposes the `LANGUAGE` constant. Consumed exclusively by `gaviero-core`, which re-exports it (along with the upstream `tree-sitter` types) so downstream crates never link the grammar directly. Grammar's job is syntax only — semantic validation lives in `gaviero-dsl`. Single in-edge (core), no out-edges into the workspace, so replan blast radius is limited to core and below.
Languages: JavaScript (grammar source), C (generated parser), Rust (bindings)
Internal deps: (none)
Key entry points: crates/tree-sitter-gaviero/grammar.js, crates/tree-sitter-gaviero/src/lib.rs, crates/tree-sitter-gaviero/build.rs

## Module 3 — gaviero-core
Path: crates/gaviero-core
Purpose: The runtime hub. A library crate exposing 22 public modules that together implement every piece of Gaviero's business logic: 6-phase swarm orchestration (validate → execute → merge → verify → cleanup → consolidate) with tier routing and pluggable backends (`ClaudeCodeBackend`, `CodexBackend` dual-mode, `OllamaStreamBackend`, `MockBackend`); multi-DB scoped semantic memory (ONNX embeddings via `ort` + `sqlite-vec`, single-consumer writer task, RRF hybrid retrieval, three-cadence consolidation, soft-delete audit); in-process read-only MCP server (`memory_search`, `blast_radius`, `node_doc`) over a Unix socket; backend-neutral `AgentSession` transport (claude/codex_exec/codex_app_server/ollama/registry); `ContextPlanner` for bootstrap/delta/replay policy; the Write Gate (scope-checked, observer-driven proposal lifecycle); validation gates (tree-sitter + cargo check); 16-language tree-sitter registry, indent engine, diff engine; `git2`-based repo and worktree management; PTY terminal with OSC 133 parsing; and the `repo_map` code knowledge graph with personalized PageRank. No UI, no DSL deps. Highest fan-out in the workspace.
Languages: Rust
Internal deps: tree-sitter-gaviero
Key entry points: crates/gaviero-core/src/lib.rs, crates/gaviero-core/src/swarm/pipeline.rs, crates/gaviero-core/src/swarm/coordinator.rs, crates/gaviero-core/src/memory/mod.rs, crates/gaviero-core/src/memory/writer.rs, crates/gaviero-core/src/memory/store/mod.rs, crates/gaviero-core/src/mcp/server.rs, crates/gaviero-core/src/agent_session/registry.rs, crates/gaviero-core/src/context_planner/mod.rs, crates/gaviero-core/src/write_gate.rs, crates/gaviero-core/src/types.rs

## Module 4 — gaviero-dsl
Path: crates/gaviero-dsl
Purpose: Compiler for the `.gaviero` workflow scripting language. Pipeline is logos lexer → chumsky parser → AST → semantic compiler → `gaviero_core::CompiledPlan`. Validates scope overlap (delegated to `gaviero_core::path_pattern::paths_overlap` so glob-disjoint siblings are accepted) and dependency cycles at compile time; emits `miette::Report` diagnostics with source spans via `thiserror` domain errors. Public API is the two `compile` / `compile_with_vars` entry points, used by both binaries to load `.gaviero` workflows. `vars {}` substitution is single-pass; precedence is agent-level > CLI `--var` overrides > script-level.
Languages: Rust
Internal deps: gaviero-core
Key entry points: crates/gaviero-dsl/src/lib.rs, crates/gaviero-dsl/src/lexer.rs, crates/gaviero-dsl/src/parser.rs, crates/gaviero-dsl/src/ast.rs, crates/gaviero-dsl/src/compiler.rs, crates/gaviero-dsl/src/error.rs

## Module 5 — gaviero-cli
Path: crates/gaviero-cli
Purpose: Headless CLI runner (`gaviero-cli` binary) for automation, CI pipelines, and scripted agent workflows. Thin wrapper around core+dsl: a single ~2.1 KLOC `src/main.rs` file containing the clap-derived `Cli` struct (authoritative flag list), mode dispatch (`--task` / `--work-units` / `--script` / `--coordinated`), and stderr observers for swarm / write-gate / agent / memory events. Also surfaces memory tooling (`--graph`, `--manifest-last`, `--manifest-turn`, eval / sleeptime / forget commands). Uses `gaviero-dsl` to compile `.gaviero` scripts and `gaviero-core` for everything else.
Languages: Rust
Internal deps: gaviero-core, gaviero-dsl
Key entry points: crates/gaviero-cli/src/main.rs, crates/gaviero-cli/tests/remember_cli.rs

## Module 6 — gaviero-tui
Path: crates/gaviero-tui
Purpose: Terminal UI editor (`gaviero` binary). Full-screen ratatui+crossterm app providing file tree, multi-tab editor (Ropey buffer + tree-sitter highlighting + diff overlay), embedded PTY terminal (`portable-pty` + `vt100` + `tui-term`), git panel, agent chat, swarm dashboard, memory inspection panel, search panel, and the interactive Write Gate diff review overlay. All business logic delegates to `gaviero-core` via observer traits (`WriteGateObserver`, `AcpObserver`, `SwarmObserver`, `ManifestObserver`); the TUI implements those traits, but core never imports TUI types. Single `mpsc::unbounded_channel<Event>` drives a `draw → recv → handle → repeat` main loop; no background task mutates `App` directly. Uses `notify` for filesystem watch and `gaviero-dsl` for `/run script.gaviero`.
Languages: Rust
Internal deps: gaviero-core, gaviero-dsl
Key entry points: crates/gaviero-tui/src/main.rs, crates/gaviero-tui/src/app.rs, crates/gaviero-tui/src/app/, crates/gaviero-tui/src/editor/, crates/gaviero-tui/src/panels/, crates/gaviero-tui/src/keymap.rs

## Global assessment

- **Architecture.** Strict layered topology with no cycles. Two leaves (`gaviero-mcp-shim`, `tree-sitter-gaviero`) sit at the bottom; `gaviero-core` is the hub (1 in-edge from each top-app, 1 from `gaviero-dsl`, 1 out-edge to `tree-sitter-gaviero`); `gaviero-dsl` is a thin shoulder above core; the two binaries (`gaviero-cli`, `gaviero-tui`) sit at the top depending on both core and dsl. The shim deliberately has zero workspace deps — it speaks to core only over a Unix socket, which keeps its binary tiny and immune to core API churn. Dependency direction is unambiguously bottom-up; no module reaches "sideways" into a sibling. The `cargo` workspace declaration in the root `Cargo.toml` matches this layout exactly.
- **Boundaries.** Clean and consciously enforced. `gaviero-core::lib.rs` re-exports `tree-sitter` types so downstream crates never link the upstream grammar crate directly (called out as a hard rule in core's CLAUDE.md). Inter-crate coupling between TUI/CLI and core flows through six observer traits (`WriteGateObserver`, `AcpObserver`, `SwarmObserver`, `MemoryObserver`, `ManifestObserver`, `McpToolCallObserver`) — core never imports TUI/CLI types. The MCP server is read-only **by construction** (no `WriterHandle` on the server type), so a stray write tool can't be added accidentally. Scope/glob checks are centralized in `path_pattern` and reused by both runtime (`scope_enforcer`) and DSL compiler. `WorkUnit`, `CompiledPlan`, `FileScope`, and the backend types all derive `serde`, so the boundary surface serializes cleanly.
- **Cross-cutting concerns.** Consistent. Logging is uniform `tracing` macros (`debug!`/`info!`/`warn!`/`error!`); error handling uses `anyhow::Result` workspace-wide with `thiserror`+`miette` reserved for DSL diagnostics with source spans. A single shared tokio runtime, with a documented "no Mutex across await/parse/I/O" rule and `#![deny(clippy::await_holding_lock)]` in `memory/writer.rs`. Settings cascade (`.gaviero/settings.json` > `.gaviero-workspace` > `~/.config/gaviero/settings.json` > defaults) is owned by `workspace::Workspace` and consumed identically by both binaries. `git2` only — never shells out — is a hard rule.
- **Hot spots.**
  - **`gaviero-core` is doing most of the work** — 22 public modules covering swarm, memory, MCP, agent sessions (5 backend modules), context planner, write/validation gates, repo-map, terminal, indent, diff. Inside core the densest sub-trees are `swarm/` (pipeline drives the 6-phase orchestration; backends in `swarm/backend/` implement `AgentBackend`; `swarm/verify/` has its own combined/structural/diff_review/test_runner split) and `memory/` (`store/` already had to be split into 8 op-files: search/write/panel_ops/deletions_ops/compression_ops/sleeptime_ops/telemetry_ops/manifest, plus extractor / session_consolidator / sleeptime / sleeptime_scheduler / annotations / compression / eval / reranker / telemetry / consolidation_llm / model_manager / onnx_embedder).
  - **`acp/` is explicitly legacy** — the newer V9 transport flows through `agent_session::claude`, with `LegacyAgentSession` shimming around `AcpPipeline` for byte-identical migration. Likely candidate for shrink/removal.
  - **`gaviero-cli/src/main.rs` is a single ~2.1 KLOC file** (clap struct + dispatch + observers) — borderline for a multi-module split, but the team flags it as the authoritative flag list, so the shape is intentional.
  - **`Replanner::evaluate` is a Phase-3 stub** (always returns `ReplanDecision::Continue`); the swarm replan path is a known gap.
  - **The cascade retrieval mode is a kill-switch** behind `memory.retrieval.mode = "cascade"`; the live default is merged multi-scope hybrid (RRF, vector 0.7 + FTS 0.3). Documentation and code paths agree on this, but it's a place where stale assumptions could leak in.
  - **`gaviero-mcp-shim` and `tree-sitter-gaviero` are both stable, low-churn leaves** — good candidates for "harmless replan" first.

VERDICT: PROCEED
