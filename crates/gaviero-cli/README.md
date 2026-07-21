# gaviero-cli

Headless CLI runner for AI agent tasks. Execute single tasks, run DSL workflows, or generate coordinated plans from the command line or CI pipelines. All logic delegates to `gaviero-core`.

## Overview

`gaviero-cli` is a thin, non-interactive wrapper around `gaviero-core` + `gaviero-dsl`. It offers four ways to define work:

1. **Single task** — a description with auto-generated full-repo scope.
2. **Workflow script** — a compiled `.gaviero` DSL file with agents and verification.
3. **Work units JSON** — explicit structured task definitions.
4. **Coordinated planning** — auto-decompose a task into a reviewable `.gaviero` plan.

Results go to stdout; progress and observer telemetry go to stderr. There is no interactive review — writes still pass through the Write Gate in `AutoAccept` mode when `--auto-accept` is set.

The binary has no public library API; the authoritative flag list is the `Cli` struct in `src/main.rs`.

## Installation

```bash
cargo build  -p gaviero-cli
cargo test   -p gaviero-cli
cargo clippy -p gaviero-cli
```

Binary name: `gaviero-cli`.

## Usage

### Single task

```bash
gaviero-cli --task "Fix compilation errors in the auth module"
gaviero-cli --repo ~/my-project --task "Refactor database layer"
```

### DSL workflow

```bash
gaviero-cli --script workflows/review_and_fix.gaviero
gaviero-cli --repo ~/my-project --script ./ci/refactor.gaviero
```

Model strings in the `.gaviero` file are respected; CLI `--model` sets the default when the file doesn't specify one.

### Work units (JSON)

```bash
gaviero-cli --work-units '[
  {"id": "design",    "description": "Plan the refactor",
   "scope": {"owned_paths": ["src/"], "read_only_paths": ["docs/"]}},
  {"id": "implement", "description": "Apply the plan", "depends_on": ["design"],
   "scope": {"owned_paths": ["src/"]}}
]'
```

### Coordinated planning

Generate a `.gaviero` plan without executing, then review and run it:

```bash
gaviero-cli --coordinated --task "Refactor auth module" --output tmp/plan.gaviero
gaviero-cli --script tmp/plan.gaviero
```

## Model Routing

Every spec requires a `provider:model` prefix — bare names are rejected.

| Provider | Examples |
|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus`, `claude:haiku`, `claude:opusplan`, `claude:sonnet[1m]` |
| Codex | `codex:gpt-5.5`, `codex:gpt-5.4` |
| Cursor | `cursor:claude-4-sonnet` |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:qwen2.5-coder:14b` |
| DeepSeek (API) | `deepseek:deepseek-v4-pro`, `deepseek:deepseek-v4-flash` |

**Execution model precedence:** `--model` → workspace `agent.model` → `claude:sonnet`.
**Coordinator model precedence:** `--coordinator-model` → `--model` → workspace `agent.coordinator.model` → `claude:sonnet`.
**Ollama URL precedence:** `--ollama-base-url` → workspace `agent.ollamaBaseUrl` → `http://localhost:11434`.

```bash
gaviero-cli --coordinated --task "..." \
  --model ollama:qwen2.5-coder:7b \
  --coordinator-model claude:sonnet
```

## Flag Reference

The `Cli` struct in `src/main.rs` is authoritative; this reference trails it. Flags are grouped by function.

### Core execution

| Flag | Argument | Purpose |
|---|---|---|
| `--repo` | `<path>` | Git repository root for `execution repo` workflows (default: `.`) |
| `--workspace` | `<path>` | Data directory for `execution document` workflows (no git lifecycle) |
| `--task` | `<text>` | Single-task mode — auto-scoped, full repo |
| `--work-units` | `<json>` | Explicit `WorkUnit` array |
| `--script` | `<path>` | `.gaviero` DSL workflow file |
| `--workflow` | `<name>` | Pick a workflow when the script defines several (requires `--script`) |
| `--prompt` | `<text>` | Inline text that replaces every `{{PROMPT}}` (requires `--script`; conflicts with `--prompt-file`) |
| `--prompt-file` | `<path>` | File whose contents replace every `{{PROMPT}}` (requires `--script`) |
| `--var` | `KEY=VALUE` | Override a `vars {}` entry (repeatable, requires `--script`) |
| `--param` | `NAME=VALUE` | Supply a workflow `param` — roster `id=provider:model[@effort],…` or client `provider:model[@effort]` (repeatable, requires `--script`) |
| `--tiers-file` | `<path>` | Tier profile (`tier <role> <client>` lines only); overrides script tier bindings (requires `--script`) |
| `--coordinated` | — | Emit a reviewable plan and exit (requires `--task`) |
| `--output` | `<path>` | Save the generated plan (`--coordinated` only) |

### Model selection

| Flag | Argument | Purpose |
|---|---|---|
| `--model` | `<spec>` | Execution model (default: workspace `agent.model`, then `claude:sonnet`) |
| `--coordinator-model` | `<spec>` | Planner model for `--coordinated` |
| `--ollama-base-url` | `<url>` | Ollama server URL |

### Execution control

| Flag | Argument | Purpose |
|---|---|---|
| `--auto-accept` | — | Skip interactive review; apply through the Write Gate in `AutoAccept` mode |
| `--max-parallel` | `<n>` | Parallel-agent cap (default `1`) |
| `--max-retries` | `<n>` | Inner validation-feedback retries (default `5`) |
| `--attempts` | `<n>` | Independent attempts for the best-of-N strategy (default `1`) |
| `--test-first` | — | Generate failing tests before the edit loop (TDD) |
| `--no-iterate` | — | Single pass only (overrides `--max-retries`) |
| `--resume` | — | Resume from `.gaviero/state/<plan-hash>.json`, skipping completed nodes |

### Output

| Flag | Argument | Purpose |
|---|---|---|
| `--format` | `text\|json` | Output format on stdout |
| `--verbose` / `-v` | — | INFO logging to stderr; `-vv` for DEBUG |
| `--trace` | `<file>` | Write DEBUG-level JSON trace log |

### Memory

| Flag | Argument | Purpose |
|---|---|---|
| `--namespace` | `<ns>` | Memory write namespace |
| `--read-ns` | `<ns>` | Additional read namespaces (repeatable) |
| `--remember` | `<text>` | Store a memory and exit |
| `--remember-scope` | `<scope>` | Scope for `--remember`: `run\|module\|repo\|workspace\|global` (default `repo`) |

### Repo-map

| Flag | Argument | Purpose |
|---|---|---|
| `--graph` | — | Build/update the code knowledge graph, print stats, and exit |
| `--enrich` | — | With `--graph`, run rustdoc JSON symbol enrichment (needs a nightly toolchain) |
| `--enrich-no-embed` | — | With `--graph --enrich`, skip embedding vectors (signatures/docs only) |
| `--exclude` | `<pattern>` | Exclude folders from repo-map scanning (repeatable, comma-separated) |

### MCP (swarm / `--script` runs)

| Flag | Argument | Purpose |
|---|---|---|
| `--no-mcp` | — | Skip MCP config synthesis and the in-process Gaviero MCP server |
| `--mcp-url` | `name=url` | Extra remote MCP server merged into every agent worktree (repeatable) |
| `--mcp-stdio` | `name=cmd,args…` | Extra stdio MCP server (repeatable) |
| `--mcp-codex-trust` | `granted\|denied\|unknown` | Codex trust for synthesized `.codex/config.toml` (use `granted` in CI) |
| `--skip-mcp-preflight` | — | Skip shim/URL validation before agents run |
| `--mcp-stats` | — | Print per-tool MCP telemetry (counts, p50/p95 latency, error/empty rates) and exit |
| `--mcp-stats-path` | `<path>` | Override the NDJSON path read by `--mcp-stats` |

### Branch cleanup

| Flag | Argument | Purpose |
|---|---|---|
| `--cleanup-branches` | — | Preview stale `gaviero/*` git branches (dry-run) and exit |
| `--force` | — | With `--cleanup-branches`, actually delete the matched branches |

### Memory admin

| Flag | Argument | Purpose |
|---|---|---|
| `--manifest-last` | `<n>` | Print the N most recent retrieval manifests and exit |
| `--manifest-turn` | `<id>` | Print the manifest for a specific turn id and exit |
| `--deletions-last` | `<n>` | List the N most recent soft-deletions and exit |
| `--restore-id` | `<id>` | Restore a single soft-deleted memory by audit id |
| `--restore-since` | `<when>` | Restore all pending deletions newer than a duration (`7 days`, `2 hours`) |
| `--forget-query` | `<text>` | Bulk-forget by fuzzy content match (Records/Summaries only) |
| `--forget-scope` | `<path>` | Bulk-forget every memory at a canonical scope path |
| `--forget-type` | `<type>` | Bulk-forget every memory of a given type |
| `--forget-source` | `<source>` | Bulk-forget by write source (e.g. `llm_extracted`, `user_remember`) |
| `--forget-dry-run` | — | Preview a `--forget-*` match count without writing |
| `--forget-yes` | — | Confirm a `--forget-*` (otherwise defaults to dry-run) |
| `--forget-reason` | `<text>` | Reason text recorded on every `--forget-*` audit row |
| `--forget-history-id` | `<id>` | Redact a history row in place (one-way; needs `--redact-confirm REDACT`) |
| `--redact-confirm` | `REDACT` | Literal confirmation for `--forget-history-id` |
| `--redact-reason` | `<text>` | Required non-empty reason for `--forget-history-id` |
| `--accept-c1-migration` | — | Accept the C1 typed-stores schema migration on first post-upgrade run |

### Sleeptime & utilization

| Flag | Argument | Purpose |
|---|---|---|
| `--sleep` | — | Run the sleeptime consolidation pass and exit |
| `--sleep-dry-run` | — | Simulate the sleeptime pass without writing |
| `--utilization-scope` | `<n>` | Report utilization at a scope level (`0`=Global … `4`=Run) and exit |
| `--utilization-top` | `<n>` | Show the top/least N entries (default `20`) |
| `--utilization-asc` | — | Sort ascending (least-utilized first) |

### Eval

| Flag | Argument | Purpose |
|---|---|---|
| `--eval-fixture` | `<path>` | Run the retrieval smoke test against a JSONL fixture; prints recall@K and MRR |
| `--eval-tolerance` | `<f>` | Recall@5 regression tolerance (default `0.02`) |
| `--eval-report-out` | `<path>` | Write the report (defaults to `<fixture>.last.json`) |
| `--eval-update-baseline` | — | Lock the baseline to this run's results |
| `--eval-allow-missing-baseline` | — | Do not fail when the baseline file is absent |
| `--eval-rerank-ablation` | — | Run with/without the reranker; print recall/MRR deltas |
| `--eval-embedder-ablation` | — | `nomic` vs `gte-modernbert` on a seeded corpus (backs up + restores `memory.db`) |
| `--eval-scope-matrix` | — | Re-run the fixture across scope hints; print Recall/Precision/blast-leakage per scope |
| `--eval-scope-matrix-scopes` | `<list>` | Scope chain for `--eval-scope-matrix` (default `repo,module,run`) |
| `--eval-budget-sweep` | — | Sweep `maxItems` {3,5,8} × `graphBudgetTokens` {4k,8k,12k} |
| `--eval-anchor-ab` | — | Thin-anchor vs full-push gold-coverage A/B (offline) |
| `--eval-from-manifests` | `<n>` | Rescore the fixture against N persisted manifests (no embedder/LLM) |
| `--eval-bootstrap-from-manifests` | — | Emit a fixture from recent manifests |
| `--seed-corpus-from-paths` | — | Seed one Record memory per `gold_must` File entry in the fixture |
| `--seed-corpus-doc-chars` | `<n>` | Max leading-doc chars per seeded file (default `480`) |

## Configuration

Beyond flags, the CLI reads the same `.gaviero/settings.json` cascade as the editor (see [`gaviero-tui`](../gaviero-tui/README.md#configuration)). Two swarm-relevant keys:

`mcp.extraServers` merges remote MCP servers into every agent worktree (CLI `--mcp-url` overrides same-named entries):

```json
{
  "mcp.extraServers": [
    { "name": "semantic-scholar", "url": "https://YOUR-MCP-ENDPOINT" }
  ],
  "mcp.gavieroServer.codexTrust": "granted"
}
```

`mcp.permissions` is a single allow/deny list of `server:tool` glob patterns applied uniformly to Claude, Cursor, and Codex (swarm *and* interactive chat). Deny wins; an empty policy allows everything; a non-empty `allow` is an allowlist:

```json
{
  "mcp.permissions": {
    "allow": ["gaviero:*", "context7:*", "semantic-scholar:get_*"],
    "deny":  ["*:delete_*"]
  }
}
```

- **Server registration is the hard gate** — a disallowed server is never written into any provider config, so no provider can reach it (even Claude under `--dangerously-skip-permissions`).
- **Gaviero's own tools are enforced server-side** by the in-process MCP server.
- **Per-tool rules for third-party servers** are translated into Claude/Cursor permission rules on a best-effort basis; Codex has only the server-registration gate.

## Output

- **stdout** — results, plan files, structured data (JSON mode).
- **stderr** — progress, observer events, logs.

This split lets pipelines capture results without losing telemetry:

```bash
gaviero-cli --task "..." --format json > results.json 2> progress.log
```

Exit codes: `0` on success, non-zero on compile/validation/runtime error (structured failures still print a `miette` report to stderr).

## Examples

```bash
# Simple task, auto-accept
gaviero-cli --repo ~/my-project \
  --task "Add comprehensive error handling to API routes" --auto-accept

# Multi-agent workflow, test-first
gaviero-cli --script workflows/feature-branch.gaviero --test-first --max-retries 3

# Cost-sensitive work on local Ollama
gaviero-cli --task "Update docstrings" \
  --model ollama:qwen2.5-coder:7b --ollama-base-url http://localhost:11434

# Best-of-3, JSON results
gaviero-cli --task "Refactor database schema" --attempts 3 --format json > results.json

# Document workflow with a remote MCP server
gaviero-cli --script examples/scientific_plan_refinement.gaviero \
  --workflow scientific-plan-refinement \
  --prompt "Sparse attention study" \
  --var PLAN_FILE=/path/to/draft-research-plan.md \
  --mcp-url semantic-scholar=https://YOUR-MCP-ENDPOINT \
  --mcp-codex-trust granted

# Store a memory from the CLI
gaviero-cli --remember "The auth module uses bcrypt for password hashing" --remember-scope repo

# Clean up stale swarm branches
gaviero-cli --repo ~/my-project --cleanup-branches --force
```

Document workflows declare `execution_mode document` in the DSL; with `PLAN_FILE` set the CLI anchors the workspace to the plan's directory and defaults `OUT_DIR` there. Repo workflows use `execution_mode repo` (default) with `--repo` for git worktrees + merge.

## See Also

- [Root README](../../README.md) — full feature overview
- [`crates/gaviero-core/README.md`](../gaviero-core/README.md) — execution engine
- [`crates/gaviero-dsl/README.md`](../gaviero-dsl/README.md) — workflow language

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
