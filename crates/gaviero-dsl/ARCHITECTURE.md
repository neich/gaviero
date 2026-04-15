# gaviero-dsl — Architecture

Compiler for `.gaviero` DSL. Transforms source text → CompiledPlan DAG consumed by `gaviero-core::swarm::pipeline`.

---

## 1. Module Map

```
gaviero-dsl/src/
├─ lib.rs                Entry point: pub fn compile(...)
├─ lexer.rs              Logos tokenizer
├─ ast.rs                Syntax tree types
├─ parser.rs             Chumsky parser combinators
├─ compiler.rs           Semantic analysis (7-phase)
└─ error.rs              Miette diagnostic errors
```

---

## 2. Compilation Pipeline

```
              ┌─────────────────────────────────────────┐
              │         Source Text (.gaviero)          │
              └──────────────┬──────────────────────────┘
                             │
                    ┌────────▼────────┐
                    │    Lexer        │
                    │  (logos)        │
                    │ → Token[]       │
                    └────────┬────────┘
                             │
                    ┌────────▼────────┐
                    │    Parser       │
                    │  (chumsky)      │
                    │ → AST           │
                    └────────┬────────┘
                             │
                    ┌────────▼────────────────────┐
                    │  Compiler (7-phase)         │
                    │                             │
                    │ Phase 1: Lex into tokens    │
                    │ Phase 2: Parse to AST       │
                    │ Phase 3: Index declarations │
                    │ Phase 4: Detect duplicates  │
                    │ Phase 5: Select workflow    │
                    │ Phase 6: Validate scopes    │
                    │ Phase 7: Build CompiledPlan │
                    │ → errors with spans         │
                    └────────┬────────────────────┘
                             │
                    ┌────────▼────────────┐
                    │   CompiledPlan      │
                    │  (petgraph DAG)     │
                    │  (ready for exec)   │
                    └─────────────────────┘
```

---

## 3. Public API

```rust
pub fn compile(
    source: &str,
    filename: &str,
    workflow: Option<&str>,
    runtime_prompt: Option<&str>,
) -> Result<CompiledPlan, miette::Report>
```

**Inputs:**
- `source`: `.gaviero` source code
- `filename`: for span/error reporting
- `workflow`: optional selector if file has multiple `workflow` blocks
- `runtime_prompt`: optional `{{PROMPT}}` substitution (from user/CLI)

**Output:**
- `Ok(CompiledPlan)`: DAG ready for swarm execution
- `Err(miette::Report)`: diagnostic with source spans

---

## 4. AST Responsibilities

Syntax tree stays close to source structure. No name resolution, no semantic validation.

### Top-level Types (ast.rs)

```rust
pub struct Script {
    pub items: Vec<Item>,
}

pub enum Item {
    ClientDecl(ClientDecl),
    AgentDecl(AgentDecl),
    WorkflowDecl(WorkflowDecl),
}

pub struct ClientDecl {
    pub name: String,
    pub tier: Option<ModelTier>,
    pub model: String,
    pub privacy: Option<PrivacyLevel>,
}

pub struct AgentDecl {
    pub name: String,
    pub description: String,
    pub client: String,
    pub scope: Option<ScopeBlock>,
    pub memory: Option<MemoryBlock>,
    pub context: Option<ContextBlock>,
    pub verify: Option<VerifyBlock>,
    pub depends_on: Vec<String>,
    pub prompt: String,
    pub max_retries: Option<u8>,
}

pub struct WorkflowDecl {
    pub name: String,
    pub steps: Vec<String>,  // Agent names in order
    pub max_parallel: Option<usize>,
    pub strategy: Option<StrategyLit>,
    pub test_first: bool,
    pub max_retries: Option<u8>,
    pub attempts: Option<u8>,
    pub escalate_after: Option<u8>,
    pub memory: Option<MemoryBlock>,
    pub verify: Option<VerifyBlock>,
    pub loops: Vec<LoopBlock>,
}
```

### Block Types

**ScopeBlock:**
```rust
pub struct ScopeBlock {
    pub owned: Vec<String>,
    pub read_only: Vec<String>,
    pub interface_contracts: HashMap<String, String>,
}
```

**MemoryBlock:**
```rust
pub struct MemoryBlock {
    pub read_namespaces: Vec<String>,
    pub write_namespace: Option<String>,
    pub importance: Option<f32>,
    pub read_query: Option<String>,
    pub read_limit: Option<usize>,
}
```

**ContextBlock:**
```rust
pub struct ContextBlock {
    pub callers_of: Vec<String>,
    pub tests_for: Vec<String>,
    pub depth: Option<u32>,
    pub impact_scope: bool,
}
```

**VerifyBlock:**
```rust
pub struct VerifyBlock {
    pub verify_compilation: bool,
    pub verify_tests: bool,
    pub verify_diffs: bool,
    pub impact_tests: Vec<String>,
}
```

**LoopBlock:**
```rust
pub struct LoopBlock {
    pub agents: Vec<String>,
    pub until: UntilCondition,
    pub max_iterations: u32,
}

pub enum UntilCondition {
    Verify,
    Agent { judgment: String },
    Command { exit_code: i32 },
}
```

---

## 5. Compiler Responsibilities

`compiler.rs` performs semantic work:

### Phase-by-phase

**Phase 1: Lex**
```
lexer::lex(source) → Token[]
```

**Phase 2: Parse**
```
parser::parse(tokens) → AST
```

**Phase 3: Index Declarations**
```
FOR each Item in AST:
  ├─ ClientDecl → Map<String, ClientDecl>
  ├─ AgentDecl → Map<String, AgentDecl>
  └─ WorkflowDecl → Map<String, WorkflowDecl>
  
Detect & error on duplicate names
```

**Phase 4: Select Workflow**
```
IF multiple workflows:
  SELECT by workflow param OR error "ambiguous"
ELSE:
  SELECT single workflow
  
IF no workflows: error "no workflow found"
```

**Phase 5: Validate Scopes**
```
FOR each agent in workflow:
  ├─ Resolve client reference
  ├─ Build FileScope from scope block
  └─ Check no owned_path overlaps with other agents
  
error on: scope violations, unknown client refs
```

**Phase 6: Resolve Dependencies**
```
FOR each depends_on edge:
  ├─ Check agent exists
  └─ Detect cycles via DFS
  
error on: unknown agent, circular dependency
```

**Phase 7: Build CompiledPlan**
```
FOR each agent in workflow order:
  ├─ Build WorkUnit from AgentDecl + ClientDecl
  ├─ Attach depends_on edges
  └─ Resolve context expansion (callers_of, tests_for)

Build DAG via petgraph::DiGraph<PlanNode, DependencyEdge>
Attach IterationConfig from workflow strategy/retries/attempts
Attach VerificationConfig from workflow verify block
Attach LoopConfig[] from workflow loops
Compute plan hash for checkpointing

Return CompiledPlan
```

---

## 6. Error Handling

All errors carry source spans for precise diagnostics via `miette`.

### Error Types (error.rs)

```rust
pub enum DslError {
    Lex {
        message: String,
        span: SourceSpan,
        label: String,
    },
    Parse {
        message: String,
        expected: Vec<String>,
        span: SourceSpan,
    },
    Compile {
        message: String,
        span: SourceSpan,
        context: Option<String>,
    },
}
```

**Wrapper:** `DslErrors` (plural) — `miette::Report` with proper formatting.

**Errors propagate with spans all the way to CLI/TUI:**

```
error: duplicate agent name "parse_config"
  ┌─ example.gaviero:15:1
  │
15 │ agent parse_config {
   │ ^^^^^^^^^^^^^^^^^^ previously defined here
   │
16 │ agent parse_config {  ← duplicate
   │ ^^^^^^^^^^^^^^^^^^ error
```

---

## 7. Name Resolution

Agent references (`depends_on`, workflow `steps`) resolved at compile time.

**Invariants:**
- All agent names in workflow must exist in same source
- All `depends_on` targets must exist
- No circular dependencies
- No overlapping FileScope

**No runtime name resolution.** All names resolved before CompiledPlan produced.

---

## 8. Model Strings

Models remain **opaque strings** in DSL. Provider-neutral.

```
"claude:opus"       (Claude API)
"claude:sonnet"     (Claude API)
"ollama:llama2"     (Ollama)
"codex:endpoint"    (OpenAI-compatible)
"sonnet"            (implicit "claude:sonnet")
```

**Resolution deferred to `gaviero-core::swarm::backend::shared`.**

---

## 9. Output: CompiledPlan Structure

```rust
pub struct CompiledPlan {
    pub graph: DiGraph<PlanNode, DependencyEdge>,
    pub max_parallel: Option<usize>,
    pub source_file: Option<PathBuf>,
    pub iteration_config: IterationConfig,
    pub verification_config: VerificationConfig,
    pub loop_configs: Vec<LoopConfig>,
}

pub struct PlanNode {
    pub work_unit: WorkUnit,
    pub status: NodeStatus,
}
```

**Key methods:**
- `work_units_ordered() -> Vec<WorkUnit>`: Kahn's topo-sort
- `hash() -> String`: stable checkpoint ID

---

## 10. Grammar Syntax Notes

### Literals

```
String:   "double quoted" or #"raw block"#
Identifier: [a-zA-Z_][a-zA-Z0-9_]*
List:     [item1 item2 item3]
Number:   0-255 (for tiers, retries)
Boolean:  true/false
Path:     src/foo.rs, *.rs, **/*.py
```

### Keywords

**Declaration:**
`client`, `agent`, `workflow`

**Scope:**
`scope`, `owned`, `read_only`, `interface_contract`

**Memory:**
`memory`, `read_from`, `write_to`, `importance`, `read_query`, `read_limit`

**Context:**
`context`, `callers_of`, `tests_for`, `depth`, `impact_scope`

**Verification:**
`verify`, `compilation`, `tests`, `diffs`, `impact_tests`

**Iteration:**
`strategy` (e.g., `best_of_3`), `max_retries`, `attempts`, `test_first`, `escalate_after`

**Execution:**
`depends_on`, `max_parallel`, `tier`, `privacy`

**Loops:**
`loop`, `until`, `max_iterations` (verify | agent | command)

### Deprecated Tier Names

Legacy names still parse but normalize to new tiers:

```
coordinator     → expensive tier
reasoning       → expensive tier
execution       → cheap tier
mechanical      → cheap tier
```

---

## 11. Integration with gaviero-core

CompiledPlan directly consumed by:

```
gaviero_core::swarm::pipeline::execute(plan, config, memory, observer)
```

No transformation needed. All plan data structures (`WorkUnit`, `PlanNode`, `CompiledPlan`) shared between `gaviero-dsl` and `gaviero-core`.

---

## 12. Concurrency & Safety

**Single-threaded compilation.** No async, no Mutex.

**Safety:**
- Source spans preserved throughout
- All errors include context
- No panics in public API (returns `Result`)

---

## 13. Dependencies

- **gaviero-core** — `CompiledPlan`, `WorkUnit`, `PlanNode`, `FileScope`, `ModelTier`, `PrivacyLevel`
- **logos** — lexer generation
- **chumsky** — parser combinators
- **miette** — error diagnostics with source spans
- **thiserror** — error type derivation
- **serde** — serialization (if needed)

---

## 14. Example Compilation

```gaviero
client opus {
  tier expensive
  model claude:opus
  privacy public
}

agent parse_config {
  client opus
  description "Extract configuration schema from source"
  
  scope {
    owned [src/ docs/]
    read_only [Cargo.toml]
  }
  
  memory {
    read_from [module]
    write_to module
    importance 0.8
  }
  
  context {
    tests_for [src/config.rs]
    depth 2
  }
  
  depends_on []
  
  prompt #"
    Extract the config struct definition and generate JSON schema.
  "#
}

workflow main {
  steps [parse_config]
  max_parallel 1
  strategy best_of_2
  verify {
    compilation true
  }
}
```

**Compilation result:**
- One `WorkUnit` for `parse_config`
- CompiledPlan with single-node DAG
- IterationConfig: `best_of_2`
- VerificationConfig: `verify_compilation=true`
- Ready for execution

---

See [CLAUDE.md](CLAUDE.md) for build, test, conventions.
