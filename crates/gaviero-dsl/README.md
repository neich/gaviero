# gaviero-dsl

## Overview

Compiler for declarative `.gaviero` workflow scripts. Define multi-agent tasks with scopes, dependencies, verification, and iteration strategies; the compiler transforms DSL source into a `CompiledPlan` DAG consumed by the swarm runtime in [`gaviero-core`](../gaviero-core/README.md).

Pipeline: `include resolver → logos lexer → chumsky parser → AST → semantic analysis → CompiledPlan`. This crate does **not** execute anything.

## Installation

```bash
cargo build -p gaviero-dsl
cargo test  -p gaviero-dsl
cargo clippy -p gaviero-dsl
```

## Usage

### From the CLI

```bash
gaviero-cli --script path/to/workflow.gaviero
gaviero-cli --script workflows/refactor.gaviero --var PLANS=output --tier judge=low
gaviero-cli --script examples/update_docs.gaviero --profile codex
```

### From the TUI

`Alt+w` opens the SWARM panel. Pick a script, fill in its inputs (prompt, `param`s, `vars`, folder, workflow), and follow the run agent by agent.

Named scripts live in `.gaviero/dsl/<name>/<name>.gaviero` (the main file is named after its folder), in the same scopes as skills: a workspace folder, the workspace layer, or the global `.gaviero`. A bare name resolves to the nearest definition. A leading `// description: …` comment is shown in the list. Files placed beside `<name>.gaviero` can be pulled in with `include`.

### Coordinated planning

```bash
gaviero-cli --coordinated --task "refactor the auth module" --output tmp/plan.gaviero
gaviero-cli --script tmp/plan.gaviero
```

## Examples

### Two-agent workflow

```gaviero
agent design {
    description "Plan the refactor"
    tier high
    scope { read_only ["src/" "tests/" "docs/"] }
    prompt "Inspect the implementation and produce a concrete refactor plan."
}

agent implement {
    description "Apply the refactor"
    tier medium
    depends_on [design]
    scope { owned ["src/" "tests/"] read_only ["docs/"] impact_scope true }
    context { callers_of ["src/auth/session.rs"]  tests_for ["src/auth/"]  depth 2 }
    prompt "Implement the approved plan."
    max_retries 4
}

workflow refactor_auth {
    steps [design implement]
    strategy refine
    verify { compile true  clippy true  impact_tests true }
}
```

### Shipped examples

Twelve workflow scripts and four model profiles live in `examples/`:

| File | Demonstrates |
|---|---|
| `plan_refinement.gaviero` | Multi-provider plan refinement with judge loop (roster items on provider tiers) |
| `generic_consensus.gaviero` | N-reviewer consensus via roster expansion |
| `phased_plan.gaviero` | Dynamic phase extraction |
| `codebase_review.gaviero` | Per-module review loop (findings only) |
| `update_docs.gaviero` | Parallel doc rewrite; swap models with `--profile` |
| `security_audit_memory.gaviero` | `memory {}` overrides, `read_ns`, staleness |
| `tiered_review.gaviero` | The three ways a tier is chosen: agent `tier`, workflow `tiers { }`, item `tier` |
| `mcp_smoke_test.gaviero` | One probe per provider on provider-pinned tiers |
| `pattern_map_reduce.gaviero` | `pattern map_reduce`; spawn-manifest workers name a `tier` |
| `scientific_research.gaviero` | Literature-research consensus panel |
| `for_loops.gaviero` | Explicit `for … parallel` / `for … sequential` over a list param |
| `deep-explore.gaviero` | Autonomous literature explorer on the `academic` MCP server, in two workflows: `deep-explore` (scout → loop: curator → runtime `for … in json` investigators → cross-family verifier, ended by the `pass_gate.py` script) and `deep-explore-report` (synthesize until `report_gate.py` passes); helper scripts in `examples/deep-explore/`; `budget`, `produces json`, `until command`; tiers from the profile |

**Model profiles** (`examples/profiles/`, see [below](#model-profiles)): `default.gaviero` (generic `low`…`top` tiers plus `<level>-claude` / `-codex` / `-cursor` and `high-deepseek`; copy it to `.gaviero/dsl/profiles/default.gaviero`), `claude.gaviero`, `codex.gaviero`, `mixed.gaviero`. The examples' rosters use the provider-pinned tiers, so they need a profile that defines them, such as `default.gaviero`.

```bash
gaviero-cli --script crates/gaviero-dsl/examples/plan_refinement.gaviero \
    --prompt "Add workspace settings cascade"
```

### Model strings

Model strings appear only in model profiles (`client { model "…" }`). Canonical form is `provider:model`; bare names are rejected at dispatch.

| Provider | Examples |
|---|---|
| Claude | `claude:fable`, `claude:sonnet`, `claude:opus`, `claude:claude-fable-5-2`, `claude:claude-opus-5-5` |
| Codex | `codex:gpt-6.1-sol`, `codex:gpt-6-astra`, `codex:gpt-6-sol`, `codex:gpt-6-luna`, `codex:gpt-5.6-sol`, `codex:gpt-6.1-terra`, `codex:gpt-5.6-terra`, `codex:gpt-5.6-luna`, `codex:gpt-5.5` |
| Cursor | `cursor:composer-2.5`, `cursor:grok-4.7-high` (ids embed effort — see `agent --list-models`) |
| Ollama / local | `ollama:qwen2.5-coder:7b`, `local:model-name` |
| DeepSeek | `deepseek:deepseek-flash`, `deepseek:deepseek-v4-pro` (legacy: `deepseek:deepseek-v4-flash`) |
| DeepSeek (ACP) | `dsh:deepseek-flash`, `dsh:deepseek-v4-pro` (legacy: `dsh:deepseek-v4-flash`) |
| Antigravity (ACP) | `agy:gemini-4-neon-high`, `agy:gemini-3.8-flash-low`, … (effort is part of the id) |

## Configuration

The DSL has no runtime config — behaviour is declared in the script and adjusted at compile time:

| Override | CLI flag | Precedence |
|---|---|---|
| Vars | `--var KEY=VALUE` | agent `vars {}` > CLI > script `vars {}` |
| What each tier means | `--profile NAME\|PATH` | chosen profile, else the profile named `default` |
| An agent's tier | `--tier AGENT=TIER` | CLI > workflow `tiers { }` > the agent's own `tier`; a list item's `tier` moves its clone |
| A tier's model, this run | `--tier-client TIER=CLIENT` | Points a tier at another client **of the same profile** (adds the tier if the profile lacks it); the profile file is unchanged |
| A tier's effort, this run | `--tier-effort TIER=EFFORT` | Runs that tier's client at another effort; other tiers on the same client are untouched |
| Workflow list params | `--param NAME=VALUE` | Required params without defaults must be supplied |

Provider resolution, memory namespaces, and verification execution are configured via `.gaviero/settings.json` on the `gaviero-core` side. See [gaviero-cli](../gaviero-cli/README.md#configuration).

### Typed inputs

A top-level var is a string unless it says otherwise. `file` marks the path of an existing input file, and `dir` a folder:

```gaviero
vars {
    OUT_DIR "plans/out"             // a string, as always
    BRIEF   file "docs/brief.md"    // a file, with a default
    PLAN    file                    // a file the caller must supply
    SOURCES dir "src"               // a folder
}
```

A run is refused before it starts if a `file` var names nothing on disk. Use a plain string or `dir` for a path the run will create. A `file` or `dir` with no default and no `--var` fails compilation. Agent-level `vars` take no type.

A `file` outside the run root is copied in, whatever it holds (a PDF works). In a repo run an uncommitted `file` is also copied into agent worktrees, but only when it is UTF-8 text: any other file is skipped with a warning and the run goes on, so an agent in a worktree does not get it. A document run has no worktrees and skips this step. `dir` has one effect only: the SWARM form completes the field with folders. At run time it is a string, so a folder need not exist and is never copied.

In the TUI's SWARM form `file` and `dir` vars complete as you type: `file` from the same list as `@` in chat, `dir` from the workspace's folders. The popup shows eight rows and scrolls through every match (Up/Down, PageUp/PageDown, wheel). List `param` items complete the tier after `id=` from the profile selected on the form.

### Model profiles

A script chooses models in one way only: by tier. Tier names are free-form; `low`, `medium`, `high` and `top` are the convention.

- **Every agent declares its default tier** with `tier <name>`. An agent without one is a compile error.
- **A workflow may move agents** to other tiers in its `tiers { <agent> <tier> … }` block; agents it does not name keep their own.
- **A run may move agents** with `--tier AGENT=TIER`, which overrides the block.
- **A run may rebind a tier** with `--tier-client TIER=CLIENT` to another client the profile declares, and change its effort with `--tier-effort TIER=EFFORT` (the SWARM form's Tiers rows do both). Models are still declared only in the profile.
- **A list item's `tier`** (`{ id "a" tier top }`, `--param panel=a=top`) moves that item's clones; items without one keep their body agent's tier.

```gaviero
agent gather { tier low prompt "…" }
agent judge  { tier low prompt "…" }
agent review { tier high template true prompt "…" }

workflow review {
    tiers { judge medium }                        // this workflow's judge runs on medium
    param panel [ { id "a" } { id "b" tier top } ] // b's clone runs on top
    steps [ gather loop { reviewers panel template_refine review until agent judge } ]
}
```

A script never declares `client` blocks or `tier <name> <client>` lines, not even in an included file — the compiler rejects them. There are no model-valued params, and list items take no `model` or `effort`.

A profile says what each tier means. It holds only `client` and `tier` declarations (`include` works, for a shared client pool), and tiers may mix providers:

```gaviero
// .gaviero/dsl/profiles/mixed.gaviero
client terra { model "codex:gpt-6.1-terra" effort medium }
client opus  { model "claude:opus"         effort high }

tier medium terra
tier high   opus
```

Profiles are looked up by file name in each workspace folder's `.gaviero/dsl/profiles/`, then the workspace layer, then `$GAVIERO_ROOT/.gaviero/dsl/profiles/` — nearest wins. (`profiles` is reserved there, so no script can be called that.) Choose one per run without touching the script:

```bash
gaviero-cli --script review.gaviero --profile mixed
gaviero-cli --script review.gaviero --profile ./one-off.gaviero
gaviero-cli --script review.gaviero --profile mixed --tier judge=high
```

The TUI's SWARM form offers the same choice in a **Profile** selector.

| Situation | Result |
|---|---|
| Profile chosen | It defines every tier of the run |
| No choice, a profile named `default` exists | It defines every tier of the run |
| No profile at all | Compile error naming `--profile` / the `default` profile |
| An agent's tier the profile does not define | Compile error listing the tiers it does define |
| A client in the profile is marked `default` | Compile error — every agent declares a tier |

`effort` names are checked at compile time, so a misspelling is an error listing the valid names. A model + effort pair the model cannot run stops the run before any agent starts; `auto` and `off` suit every model. Each profile client sets both together, so changing profile keeps the pair valid. Cursor takes no `effort`: pick a model id that has the level in it (`cursor:grok-4.7-high`).

A tier fixes model, effort, privacy and `extra` together, so nothing in a script can pair a tier with a different effort.

## API

### Compile entry points

```rust
use gaviero_dsl::{
    compile, compile_with_vars, compile_file_with_profile,
    workflow_execution_mode, CompiledPlan, ExecutionMode,
};

// Inline (no `include` statements); `profile` says what each tier means
let plan = compile(source, filename, Some(&profile), Some("workflow_name"), Some("runtime prompt"))?;

// Inline + CLI overrides (`--var`, `--tier AGENT=TIER`, `--param`)
let plan = compile_with_vars(
    source, filename, Some(&profile), Some("workflow_name"), Some("runtime prompt"),
    &override_vars, &agent_tiers, &override_params,
)?;

// From disk — resolves `include` transitively
let plan = compile_file_with_profile(path, Some(&profile), Some("workflow"), Some("prompt"), &[], &[], &[])?;

let mode: ExecutionMode = workflow_execution_mode(path, Some("my-workflow"))?;
```

`profile` is a [`SelectedProfile`](../gaviero-core/src/script_profiles.rs), usually picked with `ProfileCatalog::select`. All compile functions return `Result<CompiledPlan, miette::Report>` with source diagnostics on failure.

`CompiledPlan` is re-exported from `gaviero_core::swarm::plan`. Public modules: `ast`, `compiler`, `reviewers`, `workflow_params`, `error`, `lexer`, `parser`, `resolver`, `profile`, `patterns`, `inspect`.

### Language surface (summary)

- **Client** (model profiles only) — model + effort + privacy + optional `extra {}` provider pairs, bound to a tier with `tier <name> <client>`
- **Agent** — `tier` (required), scope, prompt, dependencies, `produces []`, `context {}`, `memory {}`, `tools []`
- **Workflow** — steps, `tiers {}`, `verify {}`, iteration strategy, explicit `loop {}` blocks, `for … parallel|sequential` over list params
- **Include** — splice shared declarations; cycles rejected; requires `compile_file`
- **Scope** — `owned`, `read_only`, `impact_scope` with glob patterns
- **Produces** — exact artefact paths the agent must have written when its turn
  ends; enforced by the runtime, not by the prompt

### Explicit loops (`for`)

`for <var> in <list-param> parallel|sequential { <agent>+ [peer_template "…"] }`
iterates a list param and clones each body agent once per item. Example:
[`examples/for_loops.gaviero`](examples/for_loops.gaviero).

```
workflow w {
    execution_mode document          // required by `for … sequential`
    param topics [
        { id "a" focus "storage" }
        { id "b" focus "network" tier high }
    ]
    steps [
        for t in topics parallel {
            analyst
            peer_template #"- {{OUT_DIR}}/{{peer}}-notes.md"#
        }
        for t in topics sequential { editor }
        summarizer                   // depends_on [analyst] → every analyst clone
        loop {
            for t in topics parallel { reviewer }   // only `parallel` inside a loop
            until agent judge
        }
    ]
}
```

- **List params** are the roster param kind with free fields: entries are
  `{ id "…" [tier <name>] [<ident> "<string>"]* }`. `id` is required,
  unique, and matches `[A-Za-z0-9][A-Za-z0-9_.-]*`. Free-field names must not be
  DSL keywords; `index`, `count`, `peers`, `peer_count`, `prev` and `peer_block`
  are reserved.
- **Clones** are named `<item-id>-<agent>` (`a-analyst`, `b-analyst`, …). With
  `tier` the clone runs on that tier; without it the clone keeps the body
  agent's tier (after the workflow's `tiers { }` and `--tier`).
  Body agents run only through their clones: listing one as a plain step, or in
  two `for` blocks, is an error. They need no `template true`.
- **Item vars** inside a clone: `{{t.id}}`, `{{t.<field>}}`, `{{t.tier}}`
  when set, `{{t.index}}` (1-based), `{{t.count}}`, `{{t.peers}}`
  (other ids, comma-joined), `{{t.peer_count}}`, `{{t.prev}}` (sequential only;
  empty for the first item), and `{{t.peer_block}}` — the `peer_template`
  rendered once per peer with `{{peer}}` = the peer id, joined by newlines
  (`(none)` without peers). Item vars are substituted before agent, CLI and
  script vars, so `{{OUT_DIR}}` inside a `peer_template` resolves. An unknown
  `{{t.x}}` is a compile error.
- **Files:** every clone's `scope.owned` and `produces` paths must contain
  `{{t.id}}` — clones never share files, in either mode.
- **`parallel`**: clones are independent. **`sequential`**: every clone of item
  k depends on every clone of item k−1, and the workflow must declare
  `execution_mode document` (item k reads item k−1's files; repo-mode worktrees
  would hide them until the run ends).
- **Fan-in:** `depends_on [analyst]` on any agent outside the body means "after
  every analyst clone". Inside the body, `depends_on` a sibling body agent means
  that item's own clone.
- **Inside a `loop`** only `for … parallel` is allowed; its clones become the
  loop's agents. When the step directly before the loop is a top-level `for`
  over the same list, its clones are the loop's **baseline**
  (`LoopConfig::baseline_units`): they may own the same paths as the loop
  agents, and a resumed run skips them.
- **CLI:** `--param 'topics=[{id "a" focus "x"} {id "b"}]'` (a DSL list
  literal) or the short form `--param topics=a,b` / `id=tier,…`.
- **Inside a `loop`, `depends_on` between loop agents orders them within
  every pass**: `curator { depends_on [investigator] }` runs after that pass's
  investigators, on pass 1 and on every later pass.
- **Not supported:** `for … sequential` inside a `loop`, `loop` inside `for`,
  `for` inside `for`, more than one `loop` per workflow. For lists only known
  at run time see `for … in json` below.

#### Runtime lists (`for … in json`)

```
for q in json "{{OUT_DIR}}/assignments-v{{ITER}}.json" parallel max 6 { investigator }
```

`for <var> in json "<path>" parallel [max <N>] { <agent> }` reads its list
from a JSON file when the step runs instead of from a list param. Example:
[`examples/deep-explore.gaviero`](examples/deep-explore.gaviero).

- **Where:** a top-level step, or inside a loop that is not
  `branch_chain stacked`. The workflow must declare `execution_mode document`.
  Inside a loop the file is read again on every pass, with that pass's
  `{{ITER}}` / `{{PREV_ITER}}` in the path; compile-time vars (`{{OUT_DIR}}`)
  resolve when the script compiles. The path is relative to the workspace root.
- **The file** is a JSON array of flat objects. `id` is required, a string
  matching `[A-Za-z0-9][A-Za-z0-9_.-]*`, unique. `tier` (a tier of the run's
  profile) is optional; `model` and `effort` keys are rejected. Every other key is
  a field: strings as written, numbers and bools as text, arrays and objects
  as compact JSON. Values are not var-substituted.
- **Item vars:** `{{q.id}}`, `{{q.<field>}}`, `{{q.tier}}`,
  `{{q.index}}` (1-based), `{{q.count}}`, `{{q.peers}}`, `{{q.peer_count}}`.
  A `{{q.x}}` that some item has no value for stops the run before any clone
  starts.
- **Body:** exactly one agent. It must declare `produces`, and its `owned` and
  `produces` paths must contain `{{q.id}}`. It may not also be a plain step or
  the body of another `for`.
- **`max`:** defaults to 16. A list longer than `max` stops the run with an
  error. An empty list runs no clone, and dependents still run.
- **Clones** are named `<agent>@<item-id>` and run in parallel under the
  workflow's `max_parallel`.
- **Fan-in is strict:** `depends_on [investigator]` waits for every clone of
  the step; if any clone fails, the step fails and its dependents are skipped.
- **Resume:** at top level every clone is checkpointed and `--resume` re-runs
  only the unfinished ones. Inside a loop a crashed pass re-runs whole
  (passes resume from their artefacts, as for every loop).
- **Not supported:** `sequential`, more than one body agent, `peer_template`,
  `execution_mode repo`.

`pattern map_reduce { discover … reduce … max_spawn N }` runs on the same
machinery: it requires `execution_mode document`; the workers listed in the
discover agent's `spawn_manifest.json` run in parallel and are checkpointed
one by one; a missing manifest or one over `max_spawn` stops the run; and a
failed worker skips `reduce`.

The `loop { reviewers … template_init … template_refine … }` roster sugar
remains and keeps its `<id>-init` / `<id>-refine` names and `REVIEWER_ID` /
`PEER_*` vars. Prefer `for` when the list carries more than a model per item,
when items must run in order, or when the peer-file layout is not the
conclusion/summary pair the roster's `PEER_READ_BLOCK` assumes.

### Output contracts (`produces`)

```
agent reviewer-refine {
    scope { owned ["{{OUT_DIR}}/{{REVIEWER_ID}}-summary-v*.md"] }
    produces ["{{OUT_DIR}}/{{REVIEWER_ID}}-summary-v{{ITER}}.md"]
}
```

Unlike `owned` these are literal paths, not globs. Compile-time vars are
substituted as usual; `{{ITER}}` / `{{PREV_ITER}}` survive to the runtime and
are substituted per loop pass, so one declaration covers every iteration.

Two checks enforce it, neither of which costs a model call:

- **Per agent** — an agent that ends its turn with a declared path missing or
  empty gets one corrective retry, then a `Failed` manifest. Without this,
  a model that narrates a file it never wrote, a proposal dropped by scope,
  and a genuine success are all indistinguishable `Completed` manifests.
- **Per loop iteration** — a loop refuses to invoke its judge unless every body
  agent delivered that pass, and aborts the run naming the agent and the missing
  paths. A judge scoring a panel that silently lost a member returns a
  meaningless verdict and burns the iteration budget. The check runs immediately
  before the judge is dispatched, so under `until … and …` a cheaper condition
  that fails first leaves it untouched.

Agents that declare no `produces` fall back to a weaker check: the loop
verifies that *something* the agent owns changed during the pass.

`produces json ["…"]` declares outputs that must also **parse as JSON**: the
paths join `produces`, and a file that exists but does not parse counts as
missing — same corrective retry, same per-pass delivery gate. Use it for files
another step reads, such as the list of a `for … in json`. Schemas stay in a
`until command "…"` check.

### Budgets (`budget`)

```
workflow w {
    budget { wall "8h" usd 40 }
}
```

- `wall` is `<digits>[s|m|h]` (no suffix = seconds). With
  `gaviero-cli --run-timeout`, the smaller of the two applies; the TUI, which
  passes no run timeout, gets the script's `wall`.
- `usd` caps the cost providers report. Only `claude:` and `deepseek:` report
  cost; `codex:`, `cursor:`, `ollama:` and `dsh:` report none, and a run with
  a `usd` budget names the units whose spend it cannot see when it starts.
  Spend includes every retry and every `best_of_N` attempt, and judges.
- Both are checked before every dispatch — each agent, clone, worker, loop
  pass and judge. A spent budget stops the run like `--run-timeout` does (no
  error; the phase reads `run budget exhausted: …`). Spend is known only when a
  dispatch returns, so agents already running in parallel can carry the total
  past `usd`.

### Composed exit conditions (`until … and …`)

```
loop {
    agents [impl]
    until { compile true } and command "cargo test --quiet" and agent reviewer
}
```

A loop exits when **every** condition passes. Conditions are evaluated
cheapest-first — `verify` block, then `command`, then the judge agent — regardless
of the order they are written in, and evaluation stops at the first one that does
not pass. A judge is an LLM call, so it is only consulted once the deterministic
conditions already agree; a failing `cargo test` costs nothing but the test run.

At most one `agent` may appear: a loop produces a single verdict, so a second
judge has no defined precedence and is a compile error. `and` is a soft keyword —
a script that already uses `and` as a name keeps working.

A pass failed by a deterministic condition issues no judge verdict, so it leaves
the `irreconcilable_after` disagreement counter untouched, and the delivery gate
does not run — there is no panel to protect when no judge was dispatched.

### Termination (`timeout`, `irreconcilable_after`)

```
agent reviewer { timeout 3600 }              # per-dispatch budget, seconds
workflow w { steps [ loop { irreconcilable_after 2 } ] }
```

`timeout` bounds one dispatch of an agent — every retry included — and defaults
to 3600s. It is what makes a run finite: provider sessions only give up when
their subprocess *exits*, so a wedged-but-alive CLI otherwise hangs the workflow
with no upper bound. `timeout 0` opts out. A blown budget becomes a `Failed`
manifest, not an error, so the delivery gate and loop verdict handle it normally.
`gaviero-cli --run-timeout <secs>` adds an outer cap on the whole run.

`irreconcilable_after` is the failure-side mirror of `stability`: N consecutive
PASS verdicts mean the agreement is real, and N consecutive *identical* blocking
disagreements mean the deadlock is structural. Either that counter or an
explicit `"verdict": "irreconcilable"` from the judge stops the loop and writes
`consensus-irreconcilable.md` — one section per reviewer, from the judge's
`blockers` array. Repeat detection fingerprints the blockers' agent/criterion
pairing rather than the prose, so a judge rephrasing itself does not reset the
count. `0` disables it.
- **Params** — list `param`s on workflows: `id=tier,…`, bare ids `a,b`, or a DSL list literal `[{id "a" …}]`

Full language reference: see the prior detailed blocks in [ARCHITECTURE.md](ARCHITECTURE.md) or browse `examples/`.

## See Also

- [gaviero-core](../gaviero-core/README.md) — execution runtime
- [gaviero-cli](../gaviero-cli/README.md) — `--script`, `--var`, `--param`, `--tier`, `--profile`
- [Root ARCHITECTURE.md](../../ARCHITECTURE.md) — compilation pipeline

## License

Apache-2.0 — see the workspace [LICENSE](../../LICENSE).
