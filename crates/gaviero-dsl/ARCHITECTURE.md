# gaviero-dsl - Architecture

`gaviero-dsl` is a compiler crate. It does not execute agents. Its job is to
turn `.gaviero` source into a validated `CompiledPlan` for `gaviero-core`.

## Compilation pipeline

```text
source text
  -> lexer.rs      tokens + spans
  -> parser.rs     AST
  -> compiler.rs   semantic analysis + plan construction
  -> CompiledPlan
```

Errors are reported with source spans through `miette`.

## Module map

```text
gaviero-dsl/src/
├── lib.rs        public compile entry point
├── lexer.rs      logos tokenization
├── parser.rs     chumsky parser
├── ast.rs        syntax tree types
├── compiler.rs   semantic analysis and plan construction
└── error.rs      diagnostic wrappers
```

## Public API

```rust
pub fn compile(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>
```

- `workflow`: optional workflow selector when a file defines more than one
- `runtime_prompt`: optional `{{PROMPT}}` substitution input

## AST responsibilities

The AST is intentionally close to source structure:

- `ClientDecl`: `tier`, `model`, `privacy`
- `AgentDecl`: `description`, `client`, `scope`, `depends_on`, `prompt`,
  `max_retries`, `memory`, `context`
- `WorkflowDecl`: `steps`, `max_parallel`, `memory`, `strategy`,
  `test_first`, `max_retries`, `attempts`, `escalate_after`, `verify`
- `LoopBlock`: loop agent list, iteration cap, and `until` condition

This layer does not resolve names or enforce semantic rules. That belongs in
`compiler.rs`.

## Compiler responsibilities

`compiler.rs` performs the semantic work:

1. Index declarations and detect duplicate names
2. Select the workflow to compile
3. Resolve `client` references for each agent
4. Build `WorkUnit` values from agent declarations
5. Validate `depends_on` references and detect cycles
6. Build workflow-level iteration and verification config
7. Extract explicit loop config and attach it to the plan graph

Important boundary: `model` is treated as an opaque string here. Provider
resolution happens later in `gaviero-core`.

## Grammar notes

- Lists are whitespace-separated inside `[...]`
- Strings can be regular quoted strings or raw `#"... "#` blocks
- Deprecated tier aliases such as `coordinator`, `reasoning`, `execution`, and
  `mechanical` still parse and are normalized to runtime tiers
- `best_of_N` is parsed from an identifier-shaped strategy literal such as
  `best_of_3`

## Output shape

The compiler produces a `CompiledPlan` containing:

- a DAG of plan nodes / work units
- workflow-level `IterationConfig`
- workflow-level `VerificationConfig`
- zero or more explicit `LoopConfig` entries

That output is consumed directly by `gaviero-core::swarm::pipeline`.

## Design intent

- Keep syntax concerns separate from runtime behavior
- Preserve source locations all the way through diagnostics
- Compile provider-neutral model strings instead of embedding backend logic
- Make coordinated plans editable text, not opaque JSON
