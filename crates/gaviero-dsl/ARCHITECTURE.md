# gaviero-dsl — Architecture

`gaviero-dsl` is a compiler for the `.gaviero` domain-specific language. It transforms declarative workflow scripts into a `CompiledPlan` — an immutable petgraph DAG of `WorkUnit` nodes — ready for execution by `gaviero-core`'s swarm pipeline. It is used by `gaviero-cli` (via `--script`) and `gaviero-tui` (via `/run`), and is also the output target of the coordinated planner: when Opus generates a plan via `plan_as_dsl()`, it emits a `.gaviero` file compiled by this crate.

---

## Module map

| Module | Purpose |
|---|---|
| `lib` | Public API: single `compile()` function; re-exports `CompiledPlan`, deprecated `CompiledScript` alias |
| `lexer` | Tokenise source text using `logos`; produce `Vec<(Token, Span)>` |
| `parser` | Parse token stream using `chumsky`; produce `Script` AST |
| `ast` | AST node types: `Script`, `ClientDecl`, `AgentDecl`, `WorkflowDecl`, `ScopeBlock`, `MemoryBlock` |
| `compiler` | 5-phase semantic analysis; emits `CompiledPlan` |
| `error` | `DslError` and `DslErrors` types; `miette`-powered source diagnostics |

---

## Compilation pipeline

```
Source text (&str)
      │
      ▼
┌─────────────┐
│   lexer     │  logos tokenisation
│  lex()      │──────────────────────→  Vec<(Token, SimpleSpan)>
└─────────────┘                         + Vec<SimpleSpan>  (lex errors)
      │
      ▼
┌─────────────┐
│   parser    │  chumsky combinator
│  parse()    │──────────────────────→  Script { items: Vec<Item> }
└─────────────┘                         + Vec<ParseError>
      │
      ▼
┌─────────────┐
│  compiler   │  5-phase semantic
│ compile()   │──────────────────────→  CompiledPlan {
└─────────────┘                             graph: DiGraph<PlanNode, DependencyEdge>,
                                            max_parallel: Option<usize>,
                                            source_file: Option<PathBuf>,
                                        }
```

Any errors at any stage are collected, wrapped in `DslErrors`, and surfaced as a `miette::Report` with source-span highlighting.

`CompiledPlan` is defined in `gaviero_core::swarm::plan` and re-exported here. `CompiledScript` (the old `{ work_units: Vec<WorkUnit>, max_parallel }` struct) is a `#[deprecated]` backward-compat alias for `CompiledPlan` — new code should use `CompiledPlan` directly.

---

## Stage 1 — Lexer (`lexer.rs`)

**Tool:** `logos` (byte-pattern tokenisation, zero-copy)

### Token catalogue

| Category | Tokens |
|---|---|
| Declaration keywords | `client`, `agent`, `workflow` |
| Field keywords | `tier`, `model`, `privacy`, `scope`, `owned`, `read_only`, `depends_on`, `prompt`, `description`, `max_retries`, `steps`, `max_parallel`, `memory`, `read_ns`, `write_ns`, `importance`, `staleness_sources` |
| Tier values | `coordinator`, `reasoning`, `execution`, `mechanical` |
| Privacy values | `public`, `local_only` |
| Punctuation | `{`, `}`, `[`, `]` |
| Literals | `Str(String)` — `"…"`, `RawStr(String)` — `#"…"#`, `Int(u64)`, `Float(f32)`, `Ident(String)` |

Whitespace and `// line comments` are silently skipped.

**Raw strings (`#"…"#`):** A custom `logos` callback scans forward for the closing sentinel `"#`. Backslashes inside raw strings are literal — no escape processing. Leading and trailing whitespace is trimmed during compilation (not lexing).

**Public function:**
```rust
pub fn lex(source: &str) -> (Vec<(Token, SimpleSpan)>, Vec<SimpleSpan>)
```
Returns tokens with spans and lex-error spans. Lex errors are non-fatal; the parser receives the token stream minus unrecognised characters.

---

## Stage 2 — Parser (`parser.rs`)

**Tool:** `chumsky` (combinator parsing over `logos` token stream)

### Internal field enums

The parser uses private field enums to collect block fields before constructing AST nodes:

| Enum | Fields |
|---|---|
| `ClientField` | `Tier`, `Model`, `Privacy` |
| `AgentField` | `Description`, `Client`, `Scope`, `DependsOn`, `Prompt`, `MaxRetries`, `Memory` |
| `ScopeField` | `Owned`, `ReadOnly` |
| `WorkflowField` | `Steps`, `MaxParallel`, `Memory` |
| `MemoryField` | `ReadNs`, `WriteNs`, `Importance`, `StalenessSources` |

**First-occurrence semantics:** Each field parser uses `.get_or_insert()` — the first occurrence of a field in a block wins. Later duplicates are silently ignored. This matches typical LLM-generated output where fields may appear out of order or be accidentally repeated.

**List syntax:** Both string lists (`["a" "b"]`) and identifier lists (`[foo bar]`) are space-separated inside `[…]` — no commas. More forgiving of LLM output than comma-separated syntax.

### Parser output

```rust
pub struct Script {
    pub items: Vec<Item>,
}

pub enum Item {
    Client(ClientDecl),
    Agent(AgentDecl),
    Workflow(WorkflowDecl),
}
```

---

## Stage 3 — AST (`ast.rs`)

All AST types carry optional fields (the parser never fails on missing fields; the compiler enforces required fields with semantic errors).

### `ClientDecl`

| Field | Type | Notes |
|---|---|---|
| `name` | `String` | Identifier; must be unique in file |
| `tier` | `Option<TierLit>` | Routing hint; defaults to `Execution` |
| `model` | `Option<String>` | Model string, e.g. `"claude-opus-4-6"` |
| `privacy` | `Option<PrivacyLit>` | `Public` or `LocalOnly`; defaults to `Public` |

### `AgentDecl`

| Field | Type | Notes |
|---|---|---|
| `name` | `String` | Identifier; unique in file; becomes `WorkUnit.id` |
| `description` | `Option<String>` | Displayed in dashboards; falls back to `name` |
| `client` | `Option<String>` | Reference to `ClientDecl` by name |
| `scope` | `Option<ScopeBlock>` | Defaults to `owned ["."]` if absent |
| `depends_on` | `Option<Vec<String>>` | Dependency edges; validated in compiler phase 4 |
| `prompt` | `Option<String>` | Becomes `WorkUnit.coordinator_instructions` |
| `max_retries` | `Option<u8>` | Defaults to `1` (single attempt) |
| `memory` | `Option<MemoryBlock>` | Merged with workflow-level memory block |

### `WorkflowDecl`

| Field | Type | Notes |
|---|---|---|
| `name` | `String` | Identifier |
| `steps` | `Option<Vec<String>>` | Ordered agent names (DAG ordering done by compiler) |
| `max_parallel` | `Option<usize>` | Overrides `--max-parallel` CLI flag |
| `memory` | `Option<MemoryBlock>` | Workflow-level defaults, merged with per-agent blocks |

### `ScopeBlock`

| Field | Type |
|---|---|
| `owned` | `Option<Vec<String>>` |
| `read_only` | `Option<Vec<String>>` |

Maps to `FileScope { owned_paths, read_only_paths, interface_contracts: {} }`.

### `MemoryBlock`

| Field | Scope | Notes |
|---|---|---|
| `read_ns` | agent + workflow | Additive: workflow list prepended to agent list |
| `write_ns` | agent + workflow | Agent overrides workflow; no fallback = no writes |
| `importance` | agent only | `0.0–1.0`; defaults to `0.5` |
| `staleness_sources` | agent only | Relative paths; hash-checked before each run |

### Value literals

```rust
enum TierLit  { Coordinator, Reasoning, Execution, Mechanical }
enum PrivacyLit { Public, LocalOnly }
```

---

## Stage 4 — Compiler (`compiler.rs`)

### 5 phases

**Phase 1 — Index declarations**
- Build `HashMap<name, ClientDecl>`, `HashMap<name, AgentDecl>`, `HashMap<name, WorkflowDecl>`
- Collect duplicate-name errors (do not halt; report all)

**Phase 2 — Determine execution order**

| Condition | Behaviour |
|---|---|
| `--workflow <name>` provided | Use named workflow's `steps` list |
| Exactly one workflow | Use it implicitly |
| No workflows | Run all agents in declaration order |
| Multiple workflows, no selector | Compile error listing available names |

Extracts `max_parallel` and workflow-level `memory` block.

**Phase 3 — Compile each agent to `WorkUnit`**

For each agent in execution order, `compile_agent()`:
1. Resolve `client` reference → extract `tier`, `model`, `privacy`
2. Build `FileScope` from `scope` block; default `owned_paths = ["."]` if absent
3. Substitute `{{PROMPT}}` placeholder in description/prompt with `runtime_prompt` if provided
4. Merge memory namespaces (see below)
5. Construct `WorkUnit` with all resolved fields

**Phase 4 — Validate dependency references**
- Every string in `depends_on` must be a defined agent name
- Report `undefined agent` error with source span for each violation

**Phase 5 — Build `CompiledPlan` (DAG)**
- Construct a petgraph `DiGraph<PlanNode, DependencyEdge>` via `CompiledPlan::from_work_units(work_units)`
  - Each `WorkUnit` becomes a `PlanNode` (index node in the graph)
  - Each `depends_on` edge becomes a `DependencyEdge::Data` directed edge from dependency → dependant
- DFS cycle detection: halt with cycle path on first cycle found
- Return `CompiledPlan { graph, max_parallel, source_file }`

`CompiledPlan::work_units_ordered()` (called by the swarm pipeline) runs Kahn's algorithm over the DiGraph for stable topological ordering.

### Memory merge rule

```
effective_read_ns  = dedup(workflow.read_ns ++ agent.read_ns)
effective_write_ns = agent.write_ns ?? workflow.write_ns ?? None
```

`importance` and `staleness_sources` are agent-only; no workflow defaults exist.

### `{{PROMPT}}` substitution

If `runtime_prompt` is supplied to `compile()`, any occurrence of the literal string `{{PROMPT}}` in an agent's `description` or `prompt` field is replaced with the runtime value. If an agent has no `prompt` field and `runtime_prompt` is `Some(_)`, the runtime prompt is used as the agent's full instruction set.

### WorkUnit field mapping

| WorkUnit field | Source |
|---|---|
| `id` | `AgentDecl.name` |
| `description` | `AgentDecl.description` or `name` |
| `scope` | `AgentDecl.scope` → `FileScope` |
| `depends_on` | `AgentDecl.depends_on` |
| `tier` | `ClientDecl.tier` → `ModelTier` |
| `privacy` | `ClientDecl.privacy` → `PrivacyLevel` |
| `model` | `ClientDecl.model` |
| `coordinator_instructions` | `AgentDecl.prompt` (after `{{PROMPT}}` substitution) |
| `max_retries` | `AgentDecl.max_retries` (default `1`) |
| `read_namespaces` | merged `effective_read_ns` (None if empty) |
| `write_namespace` | `effective_write_ns` |
| `memory_importance` | `MemoryBlock.importance` (None → store default `0.5`) |
| `staleness_sources` | `MemoryBlock.staleness_sources` |
| `estimated_tokens` | `0` (filled by swarm pipeline) |
| `escalation_tier` | `None` (swarm assigns on failure) |
| `backend` | `AgentBackend::default()` |

---

## Error reporting (`error.rs`)

**Tool:** `miette` — annotated source diagnostics with coloured terminal output

### Error types

| Variant | Fields | When |
|---|---|---|
| `Lex` | `src: NamedSource`, `span: SourceSpan` | Unexpected character in source |
| `Parse` | `src`, `span`, `reason: String` | Grammar rule did not match |
| `Compile` | `src`, `span`, `reason: String` | Semantic violation (duplicate name, undefined ref, cycle) |

### `DslErrors` container

```rust
#[derive(Error, Diagnostic)]
pub struct DslErrors {
    #[related]
    pub errors: Vec<DslError>,
}
// message: "{N} DSL error(s)"
```

`#[related]` causes `miette` to render all errors together with source snippets.

### Example output

```
× 2 DSL error(s)

  × undefined agent 'typo_agent'
   ╭─[workflow.gaviero:14:17]
14 │     depends_on [typo_agent]
   ·                ^^^^^^^^^^^
   ╰────

  × dependency cycle detected: a -> b -> a
   ╭─[workflow.gaviero:8:5]
 8 │     depends_on [b]
   ·     ^^^^^^^^^^^^^^
   ╰────
```

---

## Public API

```rust
/// Compile a .gaviero source file to a `CompiledPlan` DAG.
///
/// - `source`         — full file contents
/// - `filename`       — used in error messages (e.g. "my_workflow.gaviero")
/// - `workflow`       — optional workflow name selector (required when > 1 workflow present)
/// - `runtime_prompt` — replaces `{{PROMPT}}` in agent descriptions/prompts;
///                      used as the full prompt for agents with no `prompt` field
pub fn compile(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>

// Deprecated backward-compat alias — use CompiledPlan directly
#[deprecated]
pub type CompiledScript = CompiledPlan;
```

---

## Key dependencies

| Crate | Purpose |
|---|---|
| `logos` | Regex-free tokenisation via derive macros |
| `chumsky` | Parser combinator with error recovery |
| `miette` | Rich diagnostic error reporting with source spans |
| `gaviero-core` | `WorkUnit`, `FileScope`, `ModelTier`, `PrivacyLevel`, `CompiledPlan` |
| `serde` | `CompiledPlan` serialisation (for CLI JSON output) |

---

## Design decisions

1. **`logos` + `chumsky`.** `logos` produces tokens at near-zero allocation cost. `chumsky` gives composable error recovery — the parser reports multiple errors in a single pass rather than aborting on the first failure.

2. **Raw strings for prompts.** `#"…"#` avoids escape-sequence confusion in multiline agent instructions. LLM-generated DSL can contain quotes, backslashes, and code snippets without escaping.

3. **First-occurrence semantics.** Silently ignoring duplicate fields is more robust for LLM-generated DSL where the model may repeat a field with corrections. Human authors see the effect immediately since compilation is fast.

4. **Separate `TierLit` / `PrivacyLit` from core enums.** The AST uses its own value literals so the DSL crate's parser does not need to import core types. The compiler maps them to `ModelTier` / `PrivacyLevel` at the end of phase 3.

5. **Memory merge at compile time.** Merging workflow defaults into agent `read_ns` during compilation (not at runtime) means each `WorkUnit` is self-contained; the swarm pipeline never needs to look up workflow context.

6. **`{{PROMPT}}` substitution.** Enables a single generic script (e.g. `run_task.gaviero`) to accept an external task description without file mutation, making it suitable for piping from shell scripts.

7. **Multiple workflows → explicit selector.** Rather than picking an arbitrary workflow, the compiler fails with a helpful error listing all available names. This is intentional — silent default selection would surprise users who add a second workflow for testing.

8. **DAG output over flat `Vec<WorkUnit>`.** `CompiledPlan` wraps a petgraph `DiGraph` rather than a plain vector. This enables: (a) topological ordering at compile time via Kahn's algorithm; (b) tier computation as a pure graph traversal; (c) cycle detection during compilation rather than at runtime; (d) the `--resume` path to skip specific nodes by graph index. The flat `CompiledScript` type was the initial design; the DAG representation is the current standard.
