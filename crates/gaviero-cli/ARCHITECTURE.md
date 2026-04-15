# gaviero-cli — Architecture

Headless CLI runner. Thin wrapper around `gaviero-core` with argument parsing and observer wiring.

---

## 1. Design Philosophy

**Intentionally minimal.** All logic in `gaviero-core`.

- Parse CLI flags with `clap`
- Build one of 3 input plan shapes
- Wire up observers (AcpObserver, SwarmObserver)
- Delegate execution to core
- Print results to stdout (JSON/text)

**No state management, no caching, no business logic in this crate.**

---

## 2. File Structure

```
gaviero-cli/src/
└─ main.rs      ~500 LOC: Cli struct, observers, execution
```

---

## 3. Execution Flow

```
Parse CLI args (clap)
  │
  ├─ Load Workspace
  ├─ Resolve workspace root + settings
  └─ Initialize memory store (best-effort)
      │
      ├─ Choose input mode:
      │   ├─ --task <text>
      │   │   → Synthetic WorkUnit
      │   │   → Model resolved from --model or workspace default
      │   │
      │   ├─ --work-units <json>
      │   │   → Deserialize JSON array of WorkUnit objects
      │   │   → Use as-is
      │   │
      │   └─ --script <path>
      │       → gaviero_dsl::compile(source, ...)
      │       → Produces CompiledPlan
      │
      ├─ Apply CLI iteration overrides
      │   ├─ --max-retries N
      │   ├─ --attempts N
      │   ├─ --test-first
      │   └─ --escalate-after M
      │
      └─ Choose execution:
          ├─ IF --coordinated
          │   ├─ swarm::coordinator::plan_coordinated()
          │   │   → NLP task decomposition (Opus-powered)
          │   │   → Produces CompiledPlan
          │   ├─ Write to gaviero_plan.txt or stdout
          │   └─ Exit (no execution)
          │
          └─ ELSE
              ├─ swarm::pipeline::execute(plan, config, memory, observer, ...)
              │
              ├─ Print SwarmResult to stdout
              │
              └─ Exit with status: 0 (success) / 1 (failure)
```

---

## 4. Input Modes

### `--task <text>`

**Creates a synthetic WorkUnit:**

```rust
let unit = WorkUnit {
    id: "task".to_string(),
    description: text.to_string(),
    scope: FileScope {
        owned_paths: vec![".".to_string()],  // full repo
        read_only_paths: vec![],
        interface_contracts: Default::default(),
    },
    depends_on: vec![],
    coordinator_instructions: text.to_string(),
    model: resolved_model.clone(),  // from --model or workspace
    tier: ModelTier::Cheap,         // default
    privacy: PrivacyLevel::Public,  // default
    max_retries: 1,                 // default
    escalation_tier: None,
    // ... other fields default
};

let plan = CompiledPlan::from_work_units(vec![unit])?;
```

### `--work-units <json>`

**Deserialize JSON array:**

```json
[
  {
    "id": "parse_ast",
    "description": "Parse Python AST",
    "scope": {
      "owned_paths": ["src/parser/"],
      "read_only_paths": []
    },
    "depends_on": [],
    "model": "claude:opus",
    ...
  },
  ...
]
```

Parsed into `Vec<WorkUnit>`, then `CompiledPlan::from_work_units()`.

### `--script <path>`

**Compile `.gaviero` DSL file:**

```rust
let source = fs::read_to_string(path)?;
let plan = gaviero_dsl::compile(&source, path, workflow, runtime_prompt)?;
```

**This path supports provider-aware model strings in DSL:**
```
client sonnet { model claude:sonnet }
client local { model ollama:llama2 }
agent task1 { client sonnet }
```

---

## 5. Observers

### CliAcpObserver

Prints agent stream events to stderr:

```rust
pub struct CliAcpObserver;

impl AcpObserver for CliAcpObserver {
    fn on_stream_chunk(&self, chunk: &str) {
        eprintln!("[agent] {}", chunk);
    }
    
    fn on_tool_call_started(&self, tool_name: &str, tool_use_id: &str) {
        eprintln!("[tool] {} ({})", tool_name, tool_use_id);
    }
    
    fn on_message_complete(&self, stats: &MessageStats) {
        eprintln!("[done] {} tokens, ${:.4}", stats.output_tokens, stats.cost_usd);
    }
    
    // ... other callbacks log to stderr
}
```

### CliSwarmObserver

Prints swarm phases to stderr:

```rust
pub struct CliSwarmObserver;

impl SwarmObserver for CliSwarmObserver {
    fn on_phase_changed(&self, phase: &str) {
        eprintln!("[swarm] phase: {}", phase);
    }
    
    fn on_agent_state_changed(&self, unit_id: &str, status: AgentStatus) {
        eprintln!("[unit] {}: {:?}", unit_id, status);
    }
    
    fn on_tier_started(&self, tier: usize, units: &[WorkUnit]) {
        eprintln!("[tier] {} ({} units)", tier, units.len());
    }
    
    fn on_tier_dispatch(&self, tier: usize, unit_ids: &[String]) {
        eprintln!("[dispatch] tier {}: {:?}", tier, unit_ids);
    }
    
    fn on_completed(&self, result: &SwarmResult) {
        eprintln!("[complete] success: {}", result.success);
    }
    
    // ... other callbacks
}
```

**Key pattern:** All events to stderr, so stdout remains clean for structured output (JSON).

---

## 6. Model Resolution

### Model Spec Parsing

The CLI accepts `--model <spec>` in several formats:

```
sonnet                  → implicit "claude:sonnet"
claude:opus             → Claude API (Opus)
claude:sonnet           → Claude API (Sonnet)
claude:haiku            → Claude API (Haiku)
ollama:llama2           → Local Ollama + model name
ollama:neural-chat      → Local Ollama + model name
codex:http://localhost:8000  → OpenAI-compatible endpoint
local:http://localhost:8000  → Alias for codex
```

Validated via `gaviero_core::swarm::backend::shared::validate_model_spec()`.

### Tier/Provider Mapping

`TierRouter` resolves `(ModelTier, PrivacyLevel)` to concrete backend:

```
Cheap + Public      → Claude Haiku
Expensive + Public  → Claude Sonnet or Opus
LocalOnly           → Ollama (if available) or error
```

If `--model` specifies provider directly, it overrides tier mapping.

---

## 7. Iteration Overrides

CLI flags can override `CompiledPlan` iteration config:

```
--max-retries 5         IterationConfig::max_retries = 5
--attempts 3            IterationConfig::attempts = 3 (best_of_3)
--test-first            IterationConfig::test_first = true
--escalate-after 2      IterationConfig::escalate_after = 2
```

Applied **after** parsing input plan:

```rust
if let Some(max_retries) = args.max_retries {
    iteration_config.max_retries = max_retries;
}
if let Some(attempts) = args.attempts {
    iteration_config.strategy = Strategy::BestOfN(attempts);
}
// ... etc
```

---

## 8. Coordinated Mode

`--coordinated` flag triggers planning without execution:

```
gaviero-cli --task "Refactor parser module" --coordinated --model claude:opus
```

**Execution:**

```rust
let plan = swarm::coordinator::plan_coordinated(
    &task_description,
    &workspace,
    memory.as_ref(),
    coordinator_model,
)?;

// Write output
println!("{}", plan.to_dsl());  // Renders CompiledPlan as .gaviero

// Exit (no execution)
```

**Output:** `.gaviero` script suitable for review, modification, and `--script` input.

---

## 9. Output Modes

### Text (default)

Human-readable summary printed to stdout:

```
Swarm Result
============
Success: true
Units completed: 3/3
Execution time: 45.2s
Total cost: $0.32

Manifests:
- parse_config (completed, 2 attempts)
- build_schema (completed, 1 attempt)
- write_docs (completed, 1 attempt)

Merge: Success (3 branches merged, 0 conflicts)
```

### JSON (`--format json`)

Structured output:

```json
{
  "success": true,
  "manifests": [
    {
      "unit_id": "parse_config",
      "status": "completed",
      "attempts": 2,
      "cost_usd": 0.12
    },
    ...
  ],
  "merge_results": [...],
  "total_cost_usd": 0.32,
  "total_time_secs": 45.2
}
```

Suitable for CI integration, log aggregation, or tooling.

---

## 10. Resume Checkpoint

`--resume` flag loads execution state from checkpoint:

```
gaviero-cli --script plan.gaviero --resume
```

**Behavior:**

1. Load `.gaviero/state/{plan_hash}.json`
2. Find completed nodes in ExecutionState
3. Skip them; resume at next pending node
4. Continue execution
5. Update checkpoint after each node

**Checkpoint format:** JSON serialization of `ExecutionState`:

```json
{
  "plan_hash": "abc123def456",
  "nodes": [
    { "node_id": "parse_config", "status": "completed" },
    { "node_id": "build_schema", "status": "pending" }
  ]
}
```

---

## 11. Memory Integration

Memory store initialized best-effort:

```rust
let memory = match memory::init_workspace(&workspace).await {
    Ok(store) => Some(Arc::new(store)),
    Err(e) => {
        eprintln!("Warning: memory init failed: {}", e);
        None
    }
};
```

If memory unavailable, execution continues (non-fatal).

---

## 12. Exit Codes

| Code | Meaning |
|---|---|
| 0 | Success: all agents completed, all verification passed |
| 1 | Failure: agent failed, validation failed, or execution aborted |
| 2 | Argument error: invalid flags, missing required input |
| 3 | Setup error: workspace not found, memory init panic, etc. |

---

## 13. CLI Flags Reference

### Input

```
--task <text>           Single task (full repo scope)
--work-units <json>     JSON array of WorkUnit objects
--script <path>         .gaviero DSL file
--workflow <name>       Select workflow (if multiple in DSL file)
```

### Execution Control

```
--model <spec>          Model string (default: sonnet)
--auto-accept           Skip interactive review
--resume                Resume from checkpoint
--max-retries N         Override iteration max_retries
--attempts N            Best-of-N strategy
--test-first            TDD red phase
--escalate-after M      Escalate model after N attempts
```

### Memory

```
--namespace <name>      Write namespace (default: module)
--read-ns <name>        Override read namespace
--no-memory             Disable memory entirely
```

### Coordination

```
--coordinated           Generate .gaviero plan only (no execute)
--coordinator-model <spec>  Model for planning (default: opus)
```

### Output

```
--format <text|json>    Output format (default: text)
--output <path>         Write output to file (default: stdout)
```

### Config

```
--settings <path>       Override workspace settings file
--debug                 Enable debug logging (RUST_LOG=debug)
```

---

## 14. Dependencies

- **gaviero-core:** all runtime logic
- **gaviero-dsl:** DSL compilation
- **clap:** argument parsing (derive macros)
- **tokio:** async runtime
- **serde/serde_json:** JSON deserialization
- **miette:** error formatting

---

## 15. Error Messages

All errors printed to stderr with context:

```
error: invalid model spec
  expected: "model:variant" or plain name
  got: "badmodel::"
  
error: workspace not found
  searched: .gaviero-workspace, ~/.config/gaviero/settings.json
  
error: compilation failed
  [source miette diagnostic from dsl]
```

---

See [CLAUDE.md](CLAUDE.md) and [README.md](README.md) for flags examples and use cases.
