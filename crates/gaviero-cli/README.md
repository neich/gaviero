# gaviero-cli

Headless CLI runner for AI agent tasks. Execute single tasks, run DSL workflows, or generate coordinated plans from the command line or CI pipelines. All logic delegates to `gaviero-core`.

## Installation & Build

```bash
cargo build -p gaviero-cli
cargo test -p gaviero-cli
cargo clippy -p gaviero-cli
```

Binary name: `gaviero-cli`

## Overview

Gaviero CLI provides four ways to define work:

1. **Single task** — Simple task description with auto-generated scope
2. **Workflow script** — Compiled `.gaviero` DSL file with agents and verification
3. **Work units JSON** — Explicit structured task definitions
4. **Coordinated planning** — Auto-decompose a task into a reviewable `.gaviero` plan

Execution is non-interactive. Results and progress go to stdout/stderr.

## Usage: Single Task

Create one agent with full repository scope:

```bash
gaviero-cli --task "Fix compilation errors in the auth module"
gaviero-cli --repo ~/my-project --task "Refactor database layer"
```

## Usage: DSL Workflow

Execute a pre-written `.gaviero` workflow file:

```bash
gaviero-cli --script workflows/review_and_fix.gaviero
gaviero-cli --repo ~/my-project --script ./ci/refactor.gaviero
```

Model strings in the `.gaviero` file are respected. CLI `--model` sets the default if the file doesn't specify.

## Usage: Work Units (JSON)

Pass explicit work definitions as JSON:

```bash
gaviero-cli --work-units '[
  {
    "id": "design",
    "description": "Plan the refactor",
    "scope": {"owned_paths": ["src/"], "read_only_paths": ["docs/"]}
  },
  {
    "id": "implement",
    "description": "Apply the plan",
    "depends_on": ["design"],
    "scope": {"owned_paths": ["src/"]}
  }
]'
```

## Usage: Coordinated Planning

Generate a `.gaviero` plan without executing:

```bash
gaviero-cli --coordinated \
  --task "Split billing into planning, execution, and verification layers"
```

The generated plan is printed to stdout or saved via `--output`:

```bash
gaviero-cli --coordinated \
  --task "Refactor auth module" \
  --output tmp/plan.gaviero
```

Then review and execute:

```bash
gaviero-cli --script tmp/plan.gaviero
```

## Model Routing

The CLI supports provider-aware model specifications.

### Model spec formats

All specs require a provider prefix: `provider:model`.

- **Claude** — `claude:fable`, `claude:sonnet`, `claude:opus`, `claude:haiku`, `claude:opusplan`, `claude:sonnet[1m]`
- **Codex** — `codex:gpt-5.5`
- **Cursor** — `cursor:<model>` (e.g., `cursor:claude-4-sonnet`)
- **Ollama / local** — `ollama:qwen2.5-coder:7b`, `local:qwen2.5-coder:14b`

### Priority resolution

```
1. --model flag (if provided)
2. workspace agent.model setting
3. default: claude:sonnet
```

For coordinated planning, override the coordinator model:

```bash
gaviero-cli --coordinated \
  --task "..." \
  --model ollama:qwen2.5-coder:7b \
  --coordinator-model claude:sonnet
```

Ollama server URL precedence:

```
1. --ollama-base-url flag
2. workspace agent.ollamaBaseUrl
3. default: http://localhost:11434
```

## Flag Reference

The authoritative flag list is the `Cli` struct in `src/main.rs`. Flags are grouped below by function.

### Core execution

| Flag | Argument | Purpose |
|---|---|---|
| `--repo` | `<path>` | Workspace root (default: current directory) |
| `--task` | `<text>` | Single-task mode — auto-scoped, full repo |
| `--script` | `<path>` | `.gaviero` DSL workflow file |
| `--prompt-file` | `<path>` | File whose contents replace `{{PROMPT}}` in DSL script (requires `--script`) |
| `--var` | `KEY=VALUE` | Override a `vars {}` entry in a DSL script (repeatable, requires `--script`) |
| `--tiers-file` | `<path>` | Tier profile (only `tier <role> <client>` lines); overrides tier bindings from the script (requires `--script`) |
| `--work-units` | `<json>` | Explicit work unit definitions |
| `--coordinated` | — | Generate reviewable plan, don't execute |
| `--output` | `<path>` | Save generated plan to file (`--coordinated`) |
| `--workflow` | `<name>` | Pick a specific workflow when the script defines several |

### Model selection

| Flag | Argument | Purpose |
|---|---|---|
| `--model` | `<spec>` | Model: `claude:<m>`, `codex:<m>`, `cursor:<m>`, `ollama:<m>`, `local:<m>` (default: `claude:sonnet`) |
| `--coordinator-model` | `<spec>` | Planner model for `--coordinated` |
| `--ollama-base-url` | `<url>` | Ollama server URL |

### Execution control

| Flag | Argument | Purpose |
|---|---|---|
| `--auto-accept` | — | Skip interactive review, apply changes directly |
| `--max-parallel` | `<n>` | Override workflow parallelism |
| `--max-retries` | `<n>` | Retry limit for validation feedback |
| `--attempts` | `<n>` | Independent attempts (best-of-N strategy) |
| `--test-first` | — | Generate failing tests before code changes |
| `--no-iterate` | — | Single-pass execution, no retries |
| `--resume` | — | Resume from saved checkpoint |
| `--verbose` | — | Verbose progress output |

### Output

| Flag | Argument | Purpose |
|---|---|---|
| `--format` | `text\|json` | Output format |
| `--trace` | `<file>` | Write DEBUG-level JSON trace log |

### Memory

| Flag | Argument | Purpose |
|---|---|---|
| `--namespace` | `<ns>` | Memory write namespace |
| `--read-ns` | `<ns>` | Additional read namespaces (repeatable) |
| `--no-memory` | — | Disable memory subsystem for this run |
| `--remember` | `<text>` | Store a memory and exit |
| `--remember-scope` | `<scope>` | Scope for `--remember` (default: repo) |

### MCP (swarm / `--script` runs)

| Flag | Argument | Purpose |
|---|---|---|
| `--no-mcp` | — | Skip MCP config synthesis and the in-process Gaviero MCP server |
| `--mcp-url` | `name=url` | Extra remote MCP server merged into every agent worktree (repeatable) |
| `--mcp-stdio` | `name=cmd,args…` | Extra stdio MCP server (repeatable) |
| `--mcp-codex-trust` | `granted\|denied\|unknown` | Codex trust for synthesized `.codex/config.toml` (use `granted` in CI) |
| `--skip-mcp-preflight` | — | Skip shim/URL validation before agents run |

Workspace setting `mcp.extraServers` (JSON array) is merged the same way; CLI flags override entries with the same `name`. Example:

```bash
gaviero-cli --script examples/scientific_plan_refinement.gaviero \
  --workflow scientific-plan-refinement \
  --prompt "Sparse attention study" \
  --var PLAN_FILE=/path/to/draft-research-plan.md \
  --mcp-url semantic-scholar=https://YOUR-MCP-ENDPOINT \
  --mcp-codex-trust granted
```

Document workflows declare `execution_mode document` in the DSL. With `PLAN_FILE`, the CLI anchors the workspace to the plan's directory and defaults `OUT_DIR` there. Repo workflows use `execution_mode repo` (default) and `--repo` for git worktrees + merge.

Or in `<repo>/.gaviero/settings.json`:

```json
{
  "mcp.extraServers": [
    { "name": "semantic-scholar", "url": "https://YOUR-MCP-ENDPOINT" }
  ],
  "mcp.gavieroServer.codexTrust": "granted"
}
```

#### `mcp.permissions` — one permission policy for every provider

Define MCP permissions once at the Gaviero level instead of per provider.
`mcp.permissions` is a single allow/deny list of `server:tool` glob patterns,
applied uniformly to Claude, Cursor, and Codex (swarm *and* interactive chat):

```json
{
  "mcp.permissions": {
    "allow": ["gaviero:*", "context7:*", "semantic-scholar:get_*"],
    "deny":  ["*:delete_*"]
  }
}
```

Semantics (deny wins): an empty policy (the default) allows everything; a
non-empty `allow` is an allowlist. How it is enforced:

- **Server registration is the hard gate.** A server the policy disallows is
  never written into any provider's config (`.mcp.json`, `.cursor/mcp.json`,
  `.codex/config.toml`), so no provider can reach it — even Claude under
  `--dangerously-skip-permissions`.
- **Gaviero's own tools are enforced server-side** by the in-process MCP
  server, so a per-tool deny (e.g. `gaviero:blast_radius`) is authoritative
  regardless of provider.
- **Per-tool allow/deny for third-party servers** is translated into Claude
  (`.claude/settings.json`) and Cursor (`.cursor/cli.json`) permission rules
  (allow → auto-approve, deny → block) on a best-effort basis; Codex has no
  per-tool surface, so only the server-registration gate applies there.

### Memory admin

| Flag | Argument | Purpose |
|---|---|---|
| `--manifest-last` | `<n>` | Print the N most recent retrieval manifests and exit |
| `--manifest-turn` | `<id>` | Print the retrieval manifest for a specific turn id and exit |
| `--deletions-last` | `<n>` | Print the N most recently soft-deleted memories and exit |
| `--restore-id` | `<id>` | Restore a specific soft-deleted memory by id |
| `--restore-since` | `<date>` | Restore all memories soft-deleted since a given date |
| `--forget-history-id` | `<id>` | Redact a history row by id |
| `--redact-confirm` | — | Required confirmation flag for redaction |
| `--redact-reason` | `<text>` | Reason string stored in the audit table |
| `--accept-c1-migration` | — | Accept and run the C1 typed-stores migration |

### Sleeptime

| Flag | Argument | Purpose |
|---|---|---|
| `--sleep` | — | Run the sleeptime consolidation pass and exit |
| `--sleep-dry-run` | — | Simulate sleeptime pass without writing |

### Utilization reporting

| Flag | Argument | Purpose |
|---|---|---|
| `--utilization-scope` | `<scope>` | Scope to report memory utilization for |
| `--utilization-top` | `<n>` | Show the top N entries |
| `--utilization-asc` | — | Sort ascending instead of descending |

### Repo-map

| Flag | Argument | Purpose |
|---|---|---|
| `--graph` | — | Build/update code knowledge graph and exit |
| `--exclude` | `<pattern>` | Exclude folders from repo-map scanning (repeatable, comma-separated) |

### Branch cleanup

| Flag | Argument | Purpose |
|---|---|---|
| `--cleanup-branches` | — | Delete stale `gaviero/*` git branches and exit |
| `--force` | — | Skip confirmation when cleaning up branches |

### Eval

| Flag | Argument | Purpose |
|---|---|---|
| `--eval-fixture` | `<path>` | Run retrieval smoke test against a JSONL fixture; prints recall@K and MRR |
| `--eval-tolerance` | `<f>` | Recall@5 regression tolerance for `--eval-fixture` (default 0.02) |
| `--eval-report-out` | `<path>` | Write eval report to file (defaults to `<fixture>.last.json`) |
| `--eval-update-baseline` | — | Update the baseline file from this run's results |
| `--eval-rerank-ablation` | — | Run fixture twice with/without reranker; print recall/MRR + gold-set ndcg deltas |
| `--eval-embedder-ablation` | — | S1.1: `nomic` vs `gte-modernbert` on seeded corpus (backs up + restores `memory.db`) |
| `--eval-budget-sweep` | — | S1.3: sweep `maxItems` {3,5,8} and `graphBudgetTokens` {4k,8k,12k} |
| `--eval-from-manifests` | `<n>` | Rescore fixture against N persisted manifests (no embedder/LLM) |
| `--eval-allow-missing-baseline` | — | Do not fail when the baseline file does not exist |
| `--eval-bootstrap-from-manifests` | — | Bootstrap the baseline file from persisted manifests |

## Output

**Standard output** — Results, plan files, structured data (JSON mode)

**Standard error** — Progress, observer events, logs

This split allows shell pipelines to capture results without losing telemetry:

```bash
gaviero-cli --task "..." --format json > results.json 2> progress.log
```

## Examples

### Simple task, auto-accept

```bash
gaviero-cli --repo ~/my-project \
  --task "Add comprehensive error handling to API routes" \
  --auto-accept
```

### Multi-agent workflow with test-first

```bash
gaviero-cli \
  --script workflows/feature-branch.gaviero \
  --test-first \
  --max-retries 3
```

### Use local Ollama for cost-sensitive work

```bash
gaviero-cli \
  --task "Update docstrings" \
  --model ollama:qwen2.5-coder:7b \
  --ollama-base-url http://localhost:11434
```

### Best-of-3 execution

```bash
gaviero-cli \
  --task "Refactor database schema" \
  --attempts 3 \
  --format json > results.json
```

### Store a memory from the CLI

```bash
gaviero-cli --remember "The auth module uses bcrypt for password hashing" \
  --remember-scope repo
```

### Clean up stale swarm branches

```bash
gaviero-cli --repo ~/my-project --cleanup-branches --force
```

## See Also

- [Root README](../../README.md) — full feature overview
- [crates/gaviero-core/README.md](../gaviero-core/README.md) — execution engine
- [crates/gaviero-dsl/README.md](../gaviero-dsl/README.md) — workflow language
