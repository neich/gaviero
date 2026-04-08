# gaviero-dsl — Architecture

A compiler for the `.gaviero` domain-specific language. Transforms declarative workflow scripts into a `CompiledPlan` — an immutable petgraph DAG of `WorkUnit` nodes — ready for execution by `gaviero-core`'s swarm pipeline.

---

## Compilation pipeline

```
source text (.gaviero)
       │
       ▼ logos tokeniser (zero-copy, derive macros)
Vec<(Token, Span)>  +  lex errors
       │
       ▼ chumsky parser (combinators, error recovery)
Script { items: Vec<Item> }  +  parse errors
       │
       ▼ compiler (5-phase semantic analysis)
CompiledPlan { graph: DiGraph<PlanNode, DependencyEdge>, iteration_config, loop_configs, … }
       │
       errors reported via miette (annotated source spans)
```

All errors are collected before reporting (multiple errors per pass). The parser can produce a partial AST alongside errors for improved diagnostics.

---

## Module map

```
gaviero-dsl/src/
├── lib.rs           pub fn compile(source, filename, workflow, runtime_prompt) → Result<CompiledPlan>
├── lexer.rs         Token enum (logos derive), lex() function
├── ast.rs           Script, Item, ClientDecl, AgentDecl, WorkflowDecl, ScopeBlock, MemoryBlock,
│                    VerifyBlock, StepItem, LoopBlock, UntilCondition, TierLit, PrivacyLit, StrategyLit
├── parser.rs        parse() — chumsky combinators; grammar defined inline as functions
├── compiler.rs      compile_ast() — 7-phase analysis; build_iteration_config(), build_verification_config(),
│                    extract_loop_configs()
└── error.rs         DslError (Lex/Parse/Compile variants), DslErrors (miette wrapper)
```

---

## Token inventory (`lexer.rs`)

### Declaration keywords
`client`, `agent`, `workflow`

### Field keywords
`tier`, `model`, `privacy`, `scope`, `owned`, `read_only`, `depends_on`, `prompt`, `description`, `max_retries`, `steps`, `max_parallel`, `memory`, `read_ns`, `write_ns`, `importance`, `staleness_sources`, `read_query`, `read_limit`, `write_content`, `strategy`, `test_first`, `attempts`, `escalate_after`, `verify`, `compile`, `clippy`, `test`

### Loop keywords
`loop`, `until`, `agents`, `max_iterations`, `command`

### Tier value tokens
`coordinator`, `reasoning`, `execution`, `mechanical` (deprecated — compile to `Cheap`/`Expensive`)  
`cheap`, `expensive` (canonical)

### Privacy tokens
`public`, `local_only`

### Strategy tokens
`single_pass`, `refine`  
`BestOfN` — lexed as `Ident("best_of_3")`; the parser regex-matches `^best_of_(\d+)$` and extracts `n`.

### Literals
- `Str(String)` — `"double quoted"` (backslash escapes processed)
- `RawStr(String)` — `#"raw multiline"#` (no escape processing, leading/trailing whitespace trimmed)
- `Int(u64)` — non-negative integers
- `Float(f32)` — decimal literals (`0.9`, `1.0`)
- `Ident(String)` — identifiers: `[a-zA-Z][a-zA-Z0-9_-]*`

### Whitespace + comments
Both silently skipped. Line comments only (`// …`). No block comments.

**Ordering invariant:** `test_first` must be lexed before `test` to prevent logos from matching the shorter prefix.

---

## AST node types (`ast.rs`)

```
Script
└── items: Vec<Item>
    ├── ClientDecl { name, tier?, model?, privacy? }
    ├── AgentDecl  { name, description?, client?, scope?, depends_on?, prompt?,
    │                max_retries?, memory? }
    │   └── ScopeBlock  { owned: Vec<String>, read_only: Vec<String> }
    │   └── MemoryBlock { read_ns, write_ns?, importance?, staleness_sources,
    │                      read_query?, read_limit?, write_content? }
    └── WorkflowDecl { name, steps?, max_parallel?, memory?,
                       strategy?, test_first?, max_retries?, attempts?,
                       escalate_after?, verify? }
        └── steps: Vec<StepItem>
            ├── StepItem::Agent(name)
            └── StepItem::Loop(LoopBlock)
                ├── agents: Vec<name>
                ├── max_iterations: u32
                └── until: UntilCondition
                    ├── Verify(VerifyBlock)
                    ├── Agent(name)
                    └── Command(string)
        └── VerifyBlock { compile: bool, clippy: bool, test: bool }

TierLit    = Cheap | Expensive | Coordinator* | Reasoning* | Execution* | Mechanical*
             (* deprecated aliases)
PrivacyLit = Public | LocalOnly
StrategyLit = SinglePass | Refine | BestOfN(u32)
```

All nodes carry a `Span` for error reporting. Field-level spans enable precise diagnostic highlighting.

---

## Compiler: 7 phases (`compiler.rs`)

### Phase 1 — Index declarations
Build `HashMap<name, ClientDecl>`, `HashMap<name, AgentDecl>`, `HashMap<name, WorkflowDecl>`. Collect duplicate-name errors without halting.

### Phase 2 — Determine execution order
Select which agents to compile and in what order:

| Condition | Behaviour |
|---|---|
| `workflow` arg provided | Extract `steps` list from named workflow |
| Exactly one workflow | Use it implicitly |
| No workflows | All agents in declaration order |
| Multiple workflows, no selector | Compile error listing available names |

### Phase 3 — Compile each agent → WorkUnit
For each agent:
1. Resolve `client` reference → extract tier (mapped), model string, privacy level
2. Build `FileScope` from `scope` block (default: `owned = ["."]`)
3. Substitute `{{PROMPT}}` in description, prompt, `read_query`, and `write_content` fields
4. Merge memory namespaces (additive `read_ns`; agent `write_ns` overrides workflow)
5. Map explicit memory control fields (`read_query`, `read_limit`, `write_content`)
6. Emit complexity warning (stderr) when >1 independent agent

**Tier mapping:**
```
Cheap | Execution | Mechanical  →  ModelTier::Cheap
Expensive | Coordinator | Reasoning  →  ModelTier::Expensive
```

### Phase 4 — Validate dependency references
Every name in each agent's `depends_on` list must exist in the agent map. Collect all undefined-ref errors.

### Phase 5 — Build DAG + cycle detection
Insert `PlanNode` per `WorkUnit` into petgraph `DiGraph`. Add directed edges for `depends_on` relationships. DFS cycle detection: halt with the cycle path on first cycle found.

### Phase 6 — Build iteration + verification configs
(see below)

### Phase 7 — Extract loop configs
For each `StepItem::Loop` in the workflow's steps, build a `LoopConfig`:
```
LoopConfig {
    agent_ids:      Vec<String>,         // work unit IDs in loop
    until:          LoopUntilCondition,  // Verify | Agent | Command
    max_iterations: u32,
}
```
Attached to `CompiledPlan.loop_configs`. Used by the swarm pipeline to re-run loop agents until the exit condition is met.

### Iteration config building
```
IterationConfig {
    strategy:       wf.strategy     ?? Refine
    max_retries:    wf.max_retries  ?? 5
    max_attempts:   wf.attempts     ?? 1
    test_first:     wf.test_first   ?? false
    escalate_after: wf.escalate_after ?? 3
}
```

### Verification config building
```
VerificationConfig {
    compile: wf.verify.compile ?? false
    clippy:  wf.verify.clippy  ?? false
    test:    wf.verify.test    ?? false
}
```

---

## Parser properties (`parser.rs`)

- **First-occurrence semantics** — duplicate fields silently ignored; first wins.
- **No commas in lists** — syntax is `[item item …]`.
- **Keyword-as-ident** — `verify`, `compile`, `test`, `strategy`, `loop`, `until`, `agents`, `command` may be used as agent/workflow names where the grammar is unambiguous.
- **Recoverable** — chumsky collects multiple errors per pass; parser can emit partial AST.

---

## Error types (`error.rs`)

```rust
enum DslError {
    Lex     { src: NamedSource, span: SourceSpan },
    Parse   { src: NamedSource, span: SourceSpan, reason: String },
    Compile { src: NamedSource, span: SourceSpan, reason: String },
}
```

Common compile errors:
- `duplicate client name 'foo'`
- `undefined client 'bar'`
- `agent 'a' depends_on 'ghost' which is not defined`
- `dependency cycle detected: a -> b -> a`
- `multiple workflows defined (x, y); pass --workflow <name>`

---

## Public API surface (`lib.rs`)

```rust
pub fn compile(
    source:         &str,
    filename:       &str,
    workflow:       Option<&str>,        // None = auto-select
    runtime_prompt: Option<&str>,        // substituted for {{PROMPT}}
) -> Result<CompiledPlan, miette::Report>

// Also re-exported:
pub use gaviero_core::swarm::plan::CompiledPlan;
pub mod ast;
pub mod parser;
pub mod lexer;
pub mod compiler;
pub mod error;
```

---

## Key dependencies

| Crate | Role |
|---|---|
| `logos` | Zero-copy tokenisation via derive macros |
| `chumsky` | Parser combinators with error recovery |
| `miette` | Annotated source diagnostics with coloured spans |
| `thiserror` | Error type derive |
| `gaviero-core` | `WorkUnit`, `FileScope`, `ModelTier`, `CompiledPlan`, `IterationConfig`, `VerificationConfig`, `LoopConfig`, `LoopUntilCondition` |
