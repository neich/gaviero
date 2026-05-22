# gaviero-dsl — Architecture

Compiler for `.gaviero` workflow scripts. Source text → AST → [`CompiledPlan`](../gaviero-core/src/swarm/plan.rs) DAG consumed by [`gaviero_core::swarm::pipeline::execute`](../gaviero-core/src/swarm/pipeline.rs).

---

## 1. Topology

```
.gaviero source                          ┌── tier profile (--tiers-file)
        │                                ▼
        ▼               resolver  ◀── load_tier_overrides
   lexer (logos)              │
        │                     ▼
        ▼               { ast, sources }
   parser (chumsky)           │
        │                     ▼
        ▼              compiler::compile_ast_with_sources
       AST                    │
        │                     ▼
        └────────────────► CompiledPlan  ──►  gaviero_core::swarm
```

Single library crate; no async, no I/O outside `compile_file`'s `resolver`. Depends only on [`gaviero-core`](../gaviero-core), `logos`, `chumsky`, `miette`, `thiserror`.

---

## 2. Module Map

```
gaviero-dsl/src/
├─ lib.rs        Public entry points: compile, compile_with_vars,
│                compile_file, load_tier_overrides (re-export)
├─ lexer.rs      Logos tokenizer (shebang, strings, raw blocks, idents)
├─ ast.rs        Syntax tree types: Script, Item, AgentDecl, WorkflowDecl,
│                ClientDecl, PromptDecl, TierAlias, blocks. AUTHORITATIVE
│                for the DSL surface
├─ parser.rs     Chumsky parser combinators → Script
├─ compiler.rs   Semantic analysis: resolve vars/prompts/tiers, apply
│                substitutions, validate scopes, build CompiledPlan
├─ resolver.rs   `include "..."` resolution (transitive, cycle-detected,
│                idempotent dedup by canonical path); feeds compile_file
├─ tiers.rs      load_tier_overrides — parses a `.gaviero` profile that
│                contains only `tier <role> <client>` bindings, used by
│                gaviero-cli --tiers-file
└─ error.rs      DslError / DslErrors — miette diagnostics with spans
```

Tests in [`tests/compile.rs`](tests/compile.rs). Six example scripts in [`examples/`](examples) cover `include`, `prompt`, top-level `vars`, `tier` aliases, loops with `branch_chain`, judge controls, `extra` pass-through, glob-disjoint scopes, and `memory {}` blocks. [`examples/profiles/`](examples/profiles) ships two tier-overrides files (`doc-claude.gaviero`, `doc-codex.gaviero`) that drive `--tiers-file` for `update_docs.gaviero`.

---

## 3. Public API

```rust
// crates/gaviero-dsl/src/lib.rs
pub fn compile(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>;

pub fn compile_with_vars(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    override_tiers: &[(String, String)],
) -> Result<CompiledPlan, miette::Report>;

pub fn compile_file(
    entry_path: &std::path::Path,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
    override_vars: &[(String, String)],
    override_tiers: &[(String, String)],
) -> Result<CompiledPlan, miette::Report>;

pub use tiers::load_tier_overrides;     // for --tiers-file
pub use error::{DslError, DslErrors};
pub use gaviero_core::swarm::plan::CompiledPlan;
```

- `workflow`: select by name when the file declares multiple.
- `runtime_prompt`: substituted for every `{{PROMPT}}`; also acts as the full prompt for agents without a `prompt` field.
- `override_vars`: replace entries in the top-level `vars {}` (used by `gaviero-cli --var KEY=VALUE`). Agent-level `vars` still win; `AGENT` and `PROMPT` are reserved.
- `override_tiers`: replace `tier <alias> <client>` bindings (used by `gaviero-cli --tiers-file`). CLI profile beats script/includes. [`load_tier_overrides`](src/tiers.rs) parses profile files.
- `compile_file`: file-on-disk entry point — invokes [`resolver::resolve`](src/resolver.rs) before lexing.

Output is [`gaviero_core::swarm::plan::CompiledPlan`](../gaviero-core/src/swarm/plan.rs) — no further transformation needed before execution.

---

## 4. Compilation Pipeline

```
source / entry_path
  │
  ▼ (compile_file only)
  resolver::resolve
    → load entry + transitively follow `include "..."`
    → reject cycles, dedup by canonical path
    → merge top-level items in include order (later wins on name clash;
       compiler rejects with a span pointing at the duplicate)
  │
  ▼ lexer::lex
tokens
  │
  ▼ parser::parse
AST (Script { items: Vec<Item> })
  │
  ▼ workflow_params::expand_workflow_params_in_script
    (client params → `__param_*` clients; roster → per-reviewer clones)
  │
  ▼ compiler::compile_ast_with_vars  /  compile_ast_with_sources
    1. index Items (clients, agents, workflows, prompts, vars, tiers)
    2. duplicate-name checks (carry spans)
    3. select workflow (by name | single | error)
    4. merge vars (agent-level > CLI --var > script-level;
                   AGENT and PROMPT are reserved)
    5. resolve prompt refs    (PromptSource::Ref → PromptDecl.content)
    6. resolve tier aliases   (TierAlias → ClientDecl; CLI --tiers-file
                               override applied last)
    7. apply_vars over prompts, descriptions, scope paths, memory fields
    8. scope validation via path_pattern::patterns_overlap
    9. build WorkUnit / LoopConfig / IterationConfig / VerificationConfig
   10. assemble CompiledPlan (petgraph DAG)
  │
  ▼
CompiledPlan
```

Errors at any stage produce a `miette::Report` carrying `DslErrors(Vec<DslError>)`; all spans point into the originating file (multi-file via `compile_file` carries the per-include source through `NamedSource`).

---

## 5. AST Shape

Read [`ast.rs`](src/ast.rs) for exact field names — it is the source of truth for the DSL surface.

```rust
pub struct Script { pub items: Vec<Item> }

pub enum Item {
    Client(ClientDecl),
    Agent(AgentDecl),
    Workflow(WorkflowDecl),
    Prompt(PromptDecl),                  // top-level named prompt
    Vars(Vec<(String, String)>),         // script-level substitution map
    TierAlias(TierAlias),                // `tier <name> <client-ref>`
    Include(IncludeDecl),                // `include "..."`
}
```

### `ClientDecl`

```rust
pub struct ClientDecl {
    pub name: String,
    pub tier: Option<(TierLit, Span)>,
    pub model: Option<(String, Span)>,
    pub effort: Option<(String, Span)>,      // off/auto/low/medium/high/xhigh/max
    pub extra: Vec<ExtraPair>,               // provider pass-through
    pub privacy: Option<(PrivacyLit, Span)>,
    pub is_default: bool,                    // `default` keyword
    pub span: Span,
}
```

`extra { "key" "value" }` pairs are forwarded verbatim to the backend (Codex forwards as `-c k=v` to `codex exec`; Claude consumes the whitelist in [`swarm/backend/claude_code.rs`](../gaviero-core/src/swarm/backend/claude_code.rs) and logs the rest at `tracing::debug`; Cursor passes recognized keys to `agent` argv).

### `AgentDecl`

```rust
pub struct AgentDecl {
    pub name: String,
    pub description: Option<(String, Span)>,
    pub client: Option<(String, Span)>,         // mutually exclusive with tier_ref
    pub tier_ref: Option<(String, Span)>,       // reference to a TierAlias
    pub scope: Option<ScopeBlock>,
    pub depends_on: Option<(Vec<(String, Span)>, Span)>,
    pub prompt: Option<(PromptSource, Span)>,   // Inline(text) or Ref(name)
    pub max_retries: Option<(u8, Span)>,
    pub memory: Option<MemoryBlock>,
    pub context: Option<ContextBlock>,
    pub vars: Vec<(String, String)>,            // per-agent substitutions
    pub span: Span,
}
```

### `MemoryBlock`

Field names match compiler output: `read_ns`, `write_ns`, `importance`, `read_query`, `read_limit`, `write_content`, `staleness_sources`. `write_content` supports `{{SUMMARY}}`, `{{FILES}}`, `{{AGENT}}`, `{{DESCRIPTION}}`.

### `ContextBlock`

`callers_of`, `tests_for`, `depth`. `impact_scope` lives on `ScopeBlock`.

### `PromptDecl`, `TierAlias`, `PromptSource`, `IncludeDecl`

```rust
pub struct PromptDecl   { pub name: String, pub content: String, ... }
pub struct TierAlias    { pub name: String, pub client_ref: String, ... }
pub enum PromptSource   { Inline(String), Ref(String, Span) }
pub struct IncludeDecl  { pub path: String, pub span: Span }
```

`{{AGENT}}` inside a referenced `prompt` expands to the caller's agent name — one shared prompt yields per-agent output paths.

### `WorkflowDecl` + `LoopBlock`

Workflows hold `steps: Vec<StepItem>`, `max_parallel`, `strategy`, `verify`, `memory`, and any number of `LoopBlock`s. Loops carry `agents`, `max_iterations`, `iter_start`, `stability`, `judge_timeout`, `strict_judge`, `branch_chain`, and an `UntilCondition` (`Verify(VerifyBlock) | Agent(name) | Command(string)`).

---

## 6. Variable + Prompt Substitution

Precedence (high → low) for a `{{KEY}}` token:
1. Reserved runtime values: `{{PROMPT}}`, `{{AGENT}}`, planner-injected (`{{ITER}}`, `{{PREV_ITER}}`, `{{ITER_EVIDENCE}}`).
2. Agent-level `vars { KEY "v" }`.
3. CLI `--var KEY=VALUE` (`override_vars`).
4. Script-level `vars {}`.

[`compiler::apply_vars`](src/compiler.rs) runs over:
- agent `prompt` bodies (inline or resolved `PromptDecl`),
- `description`,
- `scope { owned [...] read_only [...] impact_scope [...] }` path lists,
- `memory { staleness_sources read_query write_content }`.

Substitution is single-pass — `{{FOO}}` expands once; nested refs inside expanded values do not.

---

## 7. Include Resolution ([`resolver.rs`](src/resolver.rs))

`compile_file(entry, …)` calls [`resolver::resolve`](src/resolver.rs):

```
resolve(entry)
  open entry, lex/parse → Script
  for each Include item:
      compute candidate = canonicalize(entry.dir / include.path)
      if already in visited (by canonical path):    skip (idempotent dedup)
      if in active stack:                            cycle error (span)
      recurse → returns Script
  merge items in include order; later includes' items append to the
  parent's item list (compiler later flags duplicate names with spans)
  return (merged_script, source_map)
```

Inline `compile` / `compile_with_vars` rejects `include` statements at compile time with a diagnostic pointing the caller at `compile_file`.

---

## 8. Tier Overrides ([`tiers.rs`](src/tiers.rs))

`gaviero-cli --tiers-file <profile.gaviero>` loads a `.gaviero` file that contains **only** `tier <role> <client>` bindings. `load_tier_overrides(path)` returns `Vec<(String, String)>` — passed straight into `compile_file` as `override_tiers`. The compiler treats overrides as last-writer-wins against `tier` lines from the script and any transitive `include`s.

---

## 9. Scope Validation

`compiler.rs` delegates to [`gaviero_core::path_pattern::patterns_overlap`](../gaviero-core/src/path_pattern.rs) for pairwise checks across every `owned` path. Glob-disjoint siblings are accepted: `plans/claude-*.md` does not overlap `plans/codex-*.md`. Concrete prefix / substring / subdirectory cases remain flagged (`src/` overlaps `src/foo.rs`). Overlap within the same `loop { agents [...] }` group is allowed (intra-loop scope sharing is intentional).

The same matcher backs [`scope_enforcer::is_scope_allowed`](../gaviero-core/src/scope_enforcer.rs) at runtime, so compile-time and runtime agree.

---

## 10. Error Handling

All errors carry `SourceSpan`s and route through `miette`:

```rust
pub enum DslError {
    Lex      { src, span },
    Parse    { message, expected, span, … },
    Compile  { message, span, context, … },
    Resolve  { message, span, path, … },     // from resolver.rs
}

pub struct DslErrors(pub Vec<DslError>);     // miette::Report wrapper
```

Representative diagnostics: unknown client / tier / prompt reference, duplicate name, circular `depends_on`, scope overlap, reserved var shadow (`PROMPT`/`AGENT`), missing workflow in multi-workflow files, unknown `{{KEY}}` substitution, `include` cycle, `include` in inline-compile entry.

---

## 11. Name Resolution

All names resolved at compile time:
- agent → `ClientDecl` (direct) or `TierAlias` → `ClientDecl` (indirect),
- workflow `steps` and `depends_on` targets must exist as agents,
- `prompt <name>` must reference a `PromptDecl`,
- no cycles in `depends_on` (DFS).

No runtime name lookup — `CompiledPlan` carries fully resolved `WorkUnit`s.

---

## 12. Output — `CompiledPlan`

```rust
pub struct CompiledPlan {
    pub graph: DiGraph<PlanNode, DependencyEdge>,
    pub max_parallel: Option<usize>,
    pub source_file: Option<PathBuf>,
    pub iteration_config: IterationConfig,
    pub verification_config: VerificationConfig,
    pub loop_configs: Vec<LoopConfig>,
}
```

Key methods: `work_units_ordered` (Kahn topo-sort), `from_work_units`, `hash` (stable checkpoint id).

---

## 13. Grammar Syntax (Cheat Sheet)

### Literals

```
"double-quoted"   #"raw block"#           // strings
[a b c]                                     // lists
0..255                                      // numbers (tiers, retries)
true / false                                // booleans
src/foo.rs   *.rs   **/*.py                 // path globs
```

### Top-level

`client`, `agent`, `workflow`, `prompt`, `vars`, `tier <alias> <client>`, `include "..."`.

### Client

`tier`, `model`, `effort`, `privacy`, `extra { "k" "v" }`, `default`.

### Agent

`description`, `client` | `tier`, `scope { owned read_only impact_scope }`, `memory { read_ns write_ns importance staleness_sources read_query read_limit write_content }`, `context { callers_of tests_for depth }`, `depends_on [...]`, `prompt "…"` | `prompt <name>`, `max_retries`, `vars { KEY "v" }`.

### Workflow

`steps [...]` (agents or `loop { ... }` blocks), `max_parallel`, `strategy` (`single_pass | refine | best_of_N`), `verify`, `memory`, `max_retries`, `attempts`, `test_first`, `escalate_after`.

### Loop

`agents`, `max_iterations`, `iter_start`, `stability`, `judge_timeout`, `strict_judge`, `branch_chain` (`none` | `stacked`), `until { compile … }` | `until agent <name>` | `until command <string>`.

---

## 14. Concurrency & Safety

Compilation is single-threaded, synchronous, no locks. All errors return `Result`; no panics in the public API. Source spans are preserved end-to-end so editor tooling can highlight offending tokens.

---

## 15. Dependencies

- [`gaviero-core`](../gaviero-core) — `CompiledPlan`, `WorkUnit`, `FileScope`, `path_pattern`, shared types.
- `logos` — lexer.
- `chumsky` — parser combinators.
- `miette` + `thiserror` — diagnostics.

---

See [CLAUDE.md](CLAUDE.md) for conventions and build commands. Examples in [`examples/`](examples) (`clients.gaviero`, `plan_refinement.gaviero`, `phased_plan.gaviero`, `codebase_review.gaviero`, `update_docs.gaviero`, `security_audit_memory.gaviero`) demonstrate every advanced feature.
