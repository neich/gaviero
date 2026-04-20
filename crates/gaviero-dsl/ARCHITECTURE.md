# gaviero-dsl — Architecture

Compiler for `.gaviero` scripts. Source text → AST → `CompiledPlan` DAG consumed by `gaviero_core::swarm::pipeline`.

---

## 1. Module Map

```
gaviero-dsl/src/
├─ lib.rs       Public entry points: compile, compile_with_vars
├─ lexer.rs     Logos tokenizer (shebang, strings, raw blocks, idents)
├─ ast.rs       Syntax tree types (Script, Item, Agent/Workflow/Client,
│               PromptDecl, TierAlias, Vars, blocks)
├─ parser.rs    Chumsky parser combinators → Script
├─ compiler.rs  Semantic analysis: resolve vars/prompts/tiers, apply
│               substitutions, validate scopes, build CompiledPlan
└─ error.rs     DslError / DslErrors — miette diagnostics with spans
```

---

## 2. Compilation Pipeline

```
source
  │
  ▼ lexer::lex
tokens
  │
  ▼ parser::parse
AST (Script { items: Vec<Item> })
  │
  ▼ compiler::compile_ast_with_vars
    1. index Items   (clients, agents, workflows, prompts, vars, tiers)
    2. duplicate-name checks (carrying spans)
    3. select workflow (by name if given, else single, else error)
    4. merge vars      (CLI overrides > script vars > agent vars; AGENT
                        and PROMPT are reserved)
    5. resolve prompt refs (PromptSource::Ref → PromptDecl.content)
    6. resolve tier aliases (TierAlias → ClientDecl)
    7. apply_vars over prompts, descriptions, scope paths, memory fields
    8. scope validation via path_pattern::patterns_overlap
    9. build WorkUnit / LoopConfig / IterationConfig / VerificationConfig
   10. assemble CompiledPlan (petgraph DAG)
  │
  ▼
CompiledPlan   (gaviero_core::swarm::plan::CompiledPlan)
```

---

## 3. Public API

```rust
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
) -> Result<CompiledPlan, miette::Report>;
```

- `workflow`: select by name when the file declares multiple.
- `runtime_prompt`: substitutes every `{{PROMPT}}`; also acts as full prompt for agents without a `prompt` field.
- `override_vars`: replace entries in the top-level `vars {}` (used by `gaviero-cli --var KEY=VALUE`). Agent-level `vars` still win; `AGENT` and `PROMPT` are reserved.

Output is `gaviero_core::swarm::plan::CompiledPlan` — no transformation needed before execution.

---

## 4. AST Shape

```rust
pub struct Script { pub items: Vec<Item> }

pub enum Item {
    Client(ClientDecl),
    Agent(AgentDecl),
    Workflow(WorkflowDecl),
    Prompt(PromptDecl),                  // top-level named prompt
    Vars(Vec<(String, String)>),         // script-level substitution map
    TierAlias(TierAlias),                // `tier <name> <client-ref>`
}
```

### `ClientDecl` (`ast.rs`)

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

`extra { "key" "value" }` pairs are forwarded verbatim to the backend (Codex forwards as `-c k=v` to `codex exec`; Claude consumes the whitelist in `backend/claude_code.rs` and logs the rest at `tracing::debug`).

### `AgentDecl`

```rust
pub struct AgentDecl {
    pub name: String,
    pub description: Option<(String, Span)>,
    pub client: Option<(String, Span)>,         // mutually exclusive with tier_ref
    pub tier_ref: Option<(String, Span)>,       // reference to TierAlias
    pub scope: Option<ScopeBlock>,
    pub depends_on: Option<(Vec<(String, Span)>, Span)>,
    pub prompt: Option<(PromptSource, Span)>,   // Inline(text) or Ref(name, span)
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

`callers_of`, `tests_for`, `depth`, plus `impact_scope` (moved to `ScopeBlock` in the scope form).

### `PromptDecl`, `TierAlias`, `PromptSource`

```rust
pub struct PromptDecl { pub name: String, pub content: String, ... }
pub struct TierAlias  { pub name: String, pub client_ref: String, ... }
pub enum PromptSource { Inline(String), Ref(String, Span) }
```

`{{AGENT}}` inside a referenced `prompt` expands to the caller's agent name, letting one shared prompt produce per-agent output paths.

### `WorkflowDecl` + `LoopBlock`

Workflows hold `steps: Vec<StepItem>`, `max_parallel`, `strategy`, `verify`, `memory`, and a `Vec<LoopBlock>`. Loops have `agents`, `max_iterations`, `iter_start`, `stability`, `judge_timeout`, `strict_judge`, and an `UntilCondition` (`Verify(VerifyBlock) | Agent(name) | Command(exit)`).

---

## 5. Variable + Prompt Substitution

Precedence (high → low) for a `{{KEY}}` token:
1. Reserved runtime values: `{{PROMPT}}`, `{{AGENT}}`, and planner-injected keys (`{{ITER}}`, `{{PREV_ITER}}`, `{{ITER_EVIDENCE}}`) at runtime.
2. Agent-level `vars { KEY "v" }`.
3. CLI `--var KEY=VALUE` (`override_vars`).
4. Script-level `vars {}`.

`apply_vars` runs over:
- agent `prompt` bodies (inline or resolved `PromptDecl`),
- `description`,
- `scope { owned [...] read_only [...] }` path lists,
- `memory { staleness_sources read_query write_content }`.

This lets agents declare `DOC_NAME` once and use it in both their prompt body and their scope paths.

---

## 6. Scope Validation

`compiler.rs` delegates to `gaviero_core::path_pattern::patterns_overlap` for pairwise checks across every `owned` path. Glob-disjoint siblings are accepted: `plans/claude-*.md` does not overlap `plans/codex-*.md`. Concrete prefix / substring / subdirectory cases remain flagged (e.g., `src/` overlaps `src/foo.rs`).

The same matcher backs `scope_enforcer::is_scope_allowed` at runtime, so compile-time and runtime agree.

---

## 7. Error Handling

All errors carry `SourceSpan`s and route through `miette`:

```rust
pub enum DslError {
    Lex   { src, span },
    Parse { message, expected, span, … },
    Compile { message, span, context, … },
}

pub struct DslErrors(pub Vec<DslError>);   // miette::Report wrapper
```

Representative diagnostics: unknown client / tier / prompt reference, duplicate name, circular `depends_on`, scope overlap, reserved var shadow (`PROMPT`/`AGENT`), missing workflow in multi-workflow files, unknown `{{KEY}}` substitution.

---

## 8. Name Resolution

All names resolved at compile time:
- agent → `ClientDecl` (direct) or `TierAlias` → `ClientDecl` (indirect),
- workflow `steps` and `depends_on` targets must exist as agents,
- `prompt <name>` must reference a `PromptDecl`,
- no cycles in `depends_on` (DFS).

No runtime name lookup — `CompiledPlan` carries fully resolved `WorkUnit`s.

---

## 9. Output — `CompiledPlan`

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

Key methods: `work_units_ordered` (Kahn topo-sort), `from_work_units`, `hash`.

---

## 10. Grammar Syntax

### Literals

```
"double-quoted"   #"raw block"#           // strings
[a b c]                                     // lists
0..255                                      // numbers (tiers, retries)
true / false                                // booleans
src/foo.rs   *.rs   **/*.py                 // path globs
```

### Top-level declarations

`client`, `agent`, `workflow`, `prompt`, `vars`, `tier <alias> <client>`.

### Client

`tier`, `model`, `effort`, `privacy`, `extra { "k" "v" }`, `default`.

### Agent

`description`, `client` | `tier`, `scope { owned read_only impact_scope }`, `memory { read_ns write_ns importance staleness_sources read_query read_limit write_content }`, `context { callers_of tests_for depth }`, `depends_on [...]`, `prompt "…"` | `prompt <name>`, `max_retries`, `vars { KEY "v" }`.

### Workflow

`steps [...]` (agents or `loop { ... }` blocks), `max_parallel`, `strategy` (`single_pass | refine | best_of_N`), `verify`, `memory`, `max_retries`, `attempts`, `test_first`, `escalate_after`.

### Loop

`agents`, `max_iterations`, `iter_start`, `stability`, `judge_timeout`, `strict_judge`, `until verify { ... }` | `until agent <name>` | `until command <string>`.

---

## 11. Integration

```
gaviero_core::swarm::pipeline::execute(plan, config, memory, observer, …)
```

consumes `CompiledPlan` directly. `WorkUnit`, `PlanNode`, `FileScope`, `ModelTier`, and `PrivacyLevel` live in `gaviero-core` and are shared — no intermediate representation.

---

## 12. Concurrency & Safety

Compilation is single-threaded, no async, no locks. All errors return `Result`; no panics in the public API. Source spans are preserved end-to-end.

---

## 13. Dependencies

- `gaviero-core` — `CompiledPlan`, `WorkUnit`, `FileScope`, `path_pattern`, types
- `logos` — lexer
- `chumsky` — parser combinators
- `miette` + `thiserror` — diagnostics

---

See [CLAUDE.md](CLAUDE.md) for build & conventions, and the examples in `examples/` (`plan_refinement.gaviero`, `update_docs.gaviero`, `phased_plan.gaviero`) for `prompt`, `vars`, `tier`-alias, loop, and `extra` usage.
