# gaviero-cli

## Overview

Headless CLI runner for AI agent tasks. Execute single tasks, run DSL workflows, or generate coordinated plans from the command line or CI pipelines. All logic delegates to `gaviero-core` + `gaviero-dsl`. Results go to stdout; progress and observer telemetry go to stderr.

Four ways to define work:

1. **Single task** — description with auto-generated full-repo scope
2. **Workflow script** — compiled `.gaviero` DSL file
3. **Work units JSON** — explicit structured task definitions
4. **Coordinated planning** — auto-decompose into a reviewable `.gaviero` plan

## Installation

```bash
cargo build  -p gaviero-cli
cargo test   -p gaviero-cli
cargo clippy -p gaviero-cli
```

Binary name: `gaviero-cli`.

## Usage

```bash
# Single task
gaviero-cli --repo ~/my-project --task "Fix compilation errors" --auto-accept

# DSL workflow
gaviero-cli --script workflows/review_and_fix.gaviero --var PLANS=output

# Coordinated planning
gaviero-cli --coordinated --task "Refactor auth module" --output tmp/plan.gaviero
gaviero-cli --script tmp/plan.gaviero

# Work units
gaviero-cli --work-units '[{"id":"design","description":"Plan","scope":{"owned_paths":["src/"]}}]'
```

**Model precedence:** `--model` → workspace `agent.model` → `claude:sonnet`. Every spec requires `provider:model` — bare names are rejected.

| Provider | Examples |
|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus` |
| Codex | `codex:gpt-5.5`, `codex:gpt-5.4` |
| Cursor | `cursor:claude-4-sonnet` |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:qwen2.5-coder:14b` |
| DeepSeek | `deepseek:deepseek-v4-pro`, `deepseek:deepseek-v4-flash` |

## Examples

```bash
# Simple task with auto-accept
gaviero-cli --repo ~/my-project \
  --task "Add error handling to API routes" --auto-accept

# Multi-agent workflow, test-first
gaviero-cli --script workflows/feature.gaviero --test-first --max-retries 3

# Local Ollama
gaviero-cli --task "Update docstrings" \
  --model ollama:qwen2.5-coder:7b --ollama-base-url http://localhost:11434

# Document workflow with remote MCP (plan-anchored: workspace = the plan's folder)
gaviero-cli --script examples/scientific_research.gaviero \
  --workflow scientific-research-consensus \
  --prompt "Sparse attention study" \
  --var PLAN_FILE=/path/to/draft-research-plan.md \
  --mcp-url semantic-scholar=https://YOUR-MCP-ENDPOINT \
  --mcp-codex-trust granted

# Store a memory
gaviero-cli --remember "Auth uses bcrypt" --remember-scope repo

# Capture JSON results
gaviero-cli --task "Refactor schema" --attempts 3 --format json > results.json 2> progress.log
```

## Configuration

Reads the same `.gaviero/settings.json` cascade as the editor (see [gaviero-tui](../gaviero-tui/README.md#configuration)).

**MCP extra servers** — merged into every agent worktree (`--mcp-url` overrides same-named entries):

```json
{
  "mcp.extraServers": [
    { "name": "semantic-scholar", "url": "https://YOUR-MCP-ENDPOINT" }
  ],
  "mcp.gavieroServer.codexTrust": "granted"
}
```

**MCP permissions** — allow/deny list of `server:tool` glob patterns (deny wins):

```json
{
  "mcp.permissions": {
    "allow": ["gaviero:*", "context7:*"],
    "deny":  ["*:delete_*"]
  }
}
```

## API

No public library API. The `Cli` struct in `src/main.rs` is authoritative; this reference trails it.

### Core execution

| Flag | Argument | Purpose |
|---|---|---|
| `--repo` | `<path>` | Git repository root for `execution repo` workflows (default: `.`) |
| `--workspace` | `<path>` | Data directory for `execution document` workflows |
| `--task` | `<text>` | Single-task mode — auto-scoped, full repo |
| `--work-units` | `<json>` | Explicit `WorkUnit` array |
| `--script` | `<path>` | `.gaviero` DSL workflow file |
| `--workflow` | `<name>` | Pick a workflow when the script defines several (requires `--script`) |
| `--prompt` | `<text>` | Inline text replacing every `{{PROMPT}}` (requires `--script`) |
| `--prompt-file` | `<path>` | File contents replacing every `{{PROMPT}}` (requires `--script`) |
| `--var` | `KEY=VALUE` | Override a `vars {}` entry (repeatable, requires `--script`) |
| `--param` | `NAME=VALUE` | Supply a workflow `param` (repeatable, requires `--script`) |
| `--tiers-file` | `<path>` | Tier profile (`tier <role> <client>` lines only; requires `--script`) |
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
| `--auto-accept` | — | Skip interactive review; apply through Write Gate in `AutoAccept` mode |
| `--max-parallel` | `<n>` | Parallel-agent cap (default `1`) |
| `--max-retries` | `<n>` | Inner validation-feedback retries (default `5`) |
| `--attempts` | `<n>` | Independent attempts for best-of-N (default `1`) |
| `--test-first` | — | Generate failing tests before the edit loop (TDD) |
| `--no-iterate` | — | Single pass only (overrides `--max-retries`) |
| `--resume` | — | Resume from `.gaviero/state/<plan-hash>.json` |
| `--fresh` | — | Ignore artefacts in a `loop` block's `OUT_DIR`; restart every loop at its script `iter_start` |
| `--run-timeout` | `<secs>` | Wall-clock cap on the whole run (default `0` = no cap) |

### Bounding a run

A run is already finite without `--run-timeout`: every agent dispatch is bounded
by the DSL's `agent { timeout <secs> }` (default 3600), so the worst case for a
loop workflow is `(1 + max_iterations) × (agents × timeout + judge_timeout)`.
That bound is what stops a wedged provider subprocess from hanging the workflow —
provider sessions only give up when their process *exits*, not when it goes quiet.

`--run-timeout` is the outer cap for cost and latency. It is checked between
agents, so the run stops cleanly and keeps every artefact written so far;
re-running the same command resumes from them.

### Resuming a consensus loop

A `loop { }` block writes one versioned artefact set per reviewer per
iteration under `OUT_DIR` (`<id>-refine-plan-v3.md`, `<id>-conclusion-v3.md`,
…). Re-running the same command against that `OUT_DIR` **continues the panel
instead of overwriting it** — no flag required:

```bash
# First run: rounds v1..v3, then interrupted
gaviero-cli --script crates/gaviero-dsl/examples/plan_refinement.gaviero \
  --workflow feature-plan-refinement \
  --prompt-file brief.md --var OUT_DIR=plans/my-feature \
  --param roster=claude=claude:opus@max,codex=codex:gpt-5.5@high

# Same command again: detects v3, resumes at v4 reading the v3 plans
```

The runtime picks the newest iteration for which **every** reviewer produced
its full artefact set, and starts at the next one. A round only some
reviewers finished — or one with an empty file — is reported and re-run from
scratch, so no agent reads peer input its peers never saw. Baseline
(`<id>-init`) agents are skipped when the artefacts already cover them.
`--verbose` is not needed; the `[resume]` lines list every file reused and
discarded.

Two things to know:

- `max_iterations` is a budget for *this* run, counted from the resumed
  start — resuming at v4 with `max_iterations 5` runs v4–v8.
- The `stability` PASS streak is not carried across runs; it restarts at
  zero and the judge re-evaluates as usual.

Use a new `OUT_DIR` (or `--fresh`) when the problem statement or roster
changed — the existing artefacts answer a different question. This is
independent of `--resume`, which restores the node-level checkpoint.

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
| `--remember-scope` | `<scope>` | Scope for `--remember`: `run\|module\|repo\|workspace\|global` |

### Repo-map

| Flag | Argument | Purpose |
|---|---|---|
| `--graph` | — | Build/update code knowledge graph and exit |
| `--enrich` | — | With `--graph`, run rustdoc JSON symbol enrichment |
| `--enrich-no-embed` | — | With `--graph --enrich`, skip embedding vectors |
| `--exclude` | `<pattern>` | Exclude folders from repo-map scanning (repeatable) |

### MCP (swarm / `--script` runs)

| Flag | Argument | Purpose |
|---|---|---|
| `--no-mcp` | — | Skip MCP config synthesis and in-process Gaviero MCP server |
| `--mcp-url` | `name=url` | Extra remote MCP server (repeatable) |
| `--mcp-stdio` | `name=cmd,args…` | Extra stdio MCP server (repeatable) |
| `--mcp-codex-trust` | `granted\|denied\|unknown` | Codex trust for synthesized config |
| `--skip-mcp-preflight` | — | Skip shim/URL validation before agents run |
| `--mcp-stats` | — | Print per-tool MCP telemetry and exit |
| `--mcp-stats-path` | `<path>` | Override NDJSON path for `--mcp-stats` |

### Branch cleanup

| Flag | Argument | Purpose |
|---|---|---|
| `--cleanup-branches` | — | Preview stale `gaviero/*` branches (dry-run) and exit |
| `--force` | — | With `--cleanup-branches`, delete matched branches |

### Memory admin

| Flag | Argument | Purpose |
|---|---|---|
| `--manifest-last` | `<n>` | Print N most recent retrieval manifests and exit |
| `--manifest-turn` | `<id>` | Print manifest for a specific turn id |
| `--deletions-last` | `<n>` | List N most recent soft-deletions |
| `--restore-id` | `<id>` | Restore a soft-deleted memory by audit id |
| `--restore-since` | `<when>` | Restore deletions newer than duration (`7 days`) |
| `--forget-query` | `<text>` | Bulk-forget by fuzzy content match |
| `--forget-scope` | `<path>` | Bulk-forget at a canonical scope path |
| `--forget-type` | `<type>` | Bulk-forget by memory type |
| `--forget-source` | `<source>` | Bulk-forget by write source |
| `--forget-dry-run` / `--forget-yes` | — | Preview or confirm `--forget-*` |
| `--forget-reason` | `<text>` | Reason on `--forget-*` audit rows |
| `--forget-history-id` | `<id>` | Redact a history row (needs `--redact-confirm REDACT`) |
| `--redact-confirm` | `REDACT` | Confirmation for `--forget-history-id` |
| `--redact-reason` | `<text>` | Required reason for `--forget-history-id` |
| `--accept-c1-migration` | — | Accept C1 typed-stores schema migration |

### Sleeptime, utilization, eval

| Flag | Argument | Purpose |
|---|---|---|
| `--sleep` / `--sleep-dry-run` | — | Run or simulate sleeptime consolidation |
| `--utilization-scope` | `<n>` | Report utilization at scope level (`0`=Global … `4`=Run) |
| `--utilization-top` | `<n>` | Top/least N entries (default `20`) |
| `--utilization-asc` | — | Sort ascending (least-utilized first) |
| `--eval-fixture` | `<path>` | Retrieval smoke test; recall@K and MRR |
| `--eval-tolerance` | `<f>` | Recall@5 regression tolerance (default `0.02`) |
| `--eval-report-out` | `<path>` | Write report (default `<fixture>.last.json`) |
| `--eval-update-baseline` | — | Lock baseline to this run |
| `--eval-rerank-ablation` | — | With/without reranker recall/MRR deltas |
| `--eval-embedder-ablation` | — | `nomic` vs `gte-modernbert` ablation |
| `--eval-scope-matrix` | — | Re-run fixture across scope hints |
| `--eval-budget-sweep` | — | Sweep `maxItems` × `graphBudgetTokens` |
| `--eval-anchor-ab` | — | Thin-anchor vs full-push A/B |
| `--eval-from-manifests` | `<n>` | Rescore fixture against persisted manifests |
| `--seed-corpus-from-paths` | — | Seed Record memory per `gold_must` file |

**Output:** stdout = results; stderr = progress/logs. Exit `0` on success.

## See Also

- [Root README](../../README.md) — feature overview
- [gaviero-core](../gaviero-core/README.md) — execution engine
- [gaviero-dsl](../gaviero-dsl/README.md) — workflow language

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
