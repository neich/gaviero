#![allow(deprecated)]
//! Coordinator: provider-aware task decomposition with tier annotations.
//!
//! The coordinator replaces the existing planner for coordinated swarm runs.
//! It produces a `TaskDAG` with tier annotations, dependency edges, and a
//! verification strategy selection.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};

use super::backend::{CompletionRequest, executor, shared};
use super::models::{AgentBackend, WorkUnit};
use super::planner::extract_json;
use super::validation;
use super::verify::VerificationStrategy;
use crate::memory::store::{MemoryStore, PrivacyFilter};
use crate::types::{FileScope, ModelTier, PrivacyLevel};

/// Configuration for the coordinator.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    pub model: String,
    pub ollama_base_url: Option<String>,
    pub max_context_tokens: u32,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            model: "opus".into(),
            ollama_base_url: None,
            max_context_tokens: 80000,
        }
    }
}

/// Coordinator-produced task DAG with tier annotations.
///
/// Deserialization is lenient: `dependency_graph` and `verification_strategy`
/// have defaults because LLMs produce varying JSON shapes for these fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDAG {
    #[serde(default)]
    pub plan_summary: String,
    pub units: Vec<WorkUnit>,
    #[serde(default, deserialize_with = "deserialize_dep_graph")]
    pub dependency_graph: Vec<(String, String)>,
    #[serde(default)]
    pub verification_strategy: VerificationStrategy,
    #[serde(default)]
    pub continued_from: Option<String>,
}

/// Deserialize dependency_graph from various LLM shapes:
/// - `[["a","b"], ["c","d"]]` — tuples as arrays
/// - `[{"from":"a","to":"b"}]` — objects
/// - missing/null — empty vec
fn deserialize_dep_graph<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<(String, String)>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Edge {
        Tuple(String, String),
        Object { from: String, to: String },
    }

    let edges: Vec<Edge> = match Vec::deserialize(deserializer) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };
    Ok(edges
        .into_iter()
        .map(|e| match e {
            Edge::Tuple(a, b) => (a, b),
            Edge::Object { from, to } => (from, to),
        })
        .collect())
}

/// Context from a prior run, enabling continuity.
#[derive(Debug, Clone)]
pub struct ContinuityContext {
    pub prior_run_id: String,
    pub completed_units: Vec<String>,
    pub failed_units: Vec<FailedUnit>,
    pub prior_plan_summary: String,
    pub prior_dependency_graph: Vec<(String, String)>,
}

/// A unit that failed in a prior run.
#[derive(Debug, Clone)]
pub struct FailedUnit {
    pub id: String,
    pub failure_reason: String,
    pub tier_at_failure: ModelTier,
}

/// The coordinator: plans tier-annotated task DAGs via a single model call.
pub struct Coordinator {
    memory: Option<Arc<MemoryStore>>,
    config: CoordinatorConfig,
}

impl Coordinator {
    pub fn new(memory: Option<Arc<MemoryStore>>, config: CoordinatorConfig) -> Self {
        Self { memory, config }
    }

    /// Produce a TaskDAG from a user prompt + repo context + memory.
    ///
    /// This is a single Opus call per swarm run. If `observer` is provided,
    /// streaming events (text chunks, tool calls) are forwarded to it so the
    /// TUI can show coordinator progress.
    pub async fn plan(
        &self,
        prompt: &str,
        workspace_root: &Path,
        file_list: &[String],
        read_namespaces: &[String],
        observer: Option<Box<dyn crate::observer::AcpObserver>>,
    ) -> Result<TaskDAG> {
        // Memory enrichment (privacy-filtered for API)
        let memory_context = if let Some(ref mem) = self.memory {
            mem.search_context_filtered(
                read_namespaces,
                prompt,
                10,
                PrivacyFilter::ExcludeLocalOnly,
            )
            .await
        } else {
            String::new()
        };

        let system_prompt = build_coordinator_prompt(file_list, &memory_context);
        let user_prompt = format!(
            "Decompose this task into a tier-annotated TaskDAG:\n\n{}\n\n\
             Respond with a JSON object matching this schema:\n\
             {{\n\
               \"plan_summary\": \"string\",\n\
               \"units\": [{{ WorkUnit with tier, privacy, coordinator_instructions, estimated_tokens }}],\n\
               \"verification_strategy\": {{ \"type\": \"combined\", \"review_tiers\": [...], \"test_command\": \"...\" }}\n\
             }}",
            prompt
        );

        let response = run_coordinator_request(
            &self.config.model,
            self.config.ollama_base_url.as_deref(),
            workspace_root,
            &system_prompt,
            &user_prompt,
            observer.as_deref(),
            "Building plan...",
        )
        .await?;

        // Parse the JSON response leniently (LLMs produce varying shapes)
        let json_str =
            extract_json(&response).context("extracting JSON from coordinator response")?;
        let mut dag = parse_task_dag_lenient(&json_str)?;

        // Auto-resolve scope overlaps before validation
        let scope_fixes = resolve_scope_overlaps(&mut dag.units);
        if !scope_fixes.is_empty() {
            tracing::info!("Auto-resolved {} scope overlaps", scope_fixes.len());
            if let Some(obs) = observer.as_ref() {
                let mut msg = format!("\nAuto-resolved {} scope overlaps:\n", scope_fixes.len());
                for fix in &scope_fixes {
                    msg.push_str(&format!("  • {}\n", fix));
                }
                obs.on_stream_chunk(&msg);
            }
        }

        // Send human-readable plan summary to observer
        if let Some(ref obs) = observer {
            let mut summary = format!("Plan: {}\n", dag.plan_summary);
            summary.push_str(&format!("{} tasks:\n", dag.units.len()));
            for unit in &dag.units {
                let tier_label = match unit.tier {
                    ModelTier::Cheap => "C",
                    ModelTier::Expensive => "E",
                };
                let deps = if unit.depends_on.is_empty() {
                    String::new()
                } else {
                    format!(" (after: {})", unit.depends_on.join(", "))
                };
                summary.push_str(&format!(
                    "  [{}] {} — {}{}\n",
                    tier_label, unit.id, unit.description, deps
                ));
            }
            obs.on_stream_chunk(&summary);
        }

        // Validate the DAG
        self.validate_dag(&dag)?;

        Ok(dag)
    }

    /// Produce a `.gaviero` DSL file from a user prompt + repo context + memory.
    ///
    /// This replaces `plan()` as the primary coordinator entry point. Instead of
    /// emitting fragile JSON, Opus produces a human-readable, compiler-validated
    /// `.gaviero` DSL file. The caller is responsible for writing this to disk,
    /// presenting it for user review, and compiling it with `gaviero_dsl::compile()`.
    ///
    /// Returning `Result<String>` (raw DSL text) instead of a parsed struct
    /// eliminates all lenient JSON parsing and associated silent failure modes.
    pub async fn plan_as_dsl(
        &self,
        prompt: &str,
        workspace_root: &Path,
        file_list: &[String],
        read_namespaces: &[String],
        observer: Option<Box<dyn crate::observer::AcpObserver>>,
    ) -> Result<String> {
        let obs = observer.as_deref();

        if let Some(o) = obs {
            o.on_streaming_status("Searching memory context...");
        }
        let memory_context = if let Some(ref mem) = self.memory {
            mem.search_context_filtered(
                read_namespaces,
                prompt,
                10,
                PrivacyFilter::ExcludeLocalOnly,
            )
            .await
        } else {
            String::new()
        };

        let system_prompt = build_coordinator_dsl_prompt(file_list, &memory_context);
        let user_prompt = format!(
            "Decompose this task into a `.gaviero` DSL workflow:\n\n{}",
            prompt
        );

        let total_kb = (user_prompt.len() + system_prompt.len()) / 1024;
        if let Some(o) = obs {
            o.on_streaming_status(&format!(
                "Waiting for model response... (~{}KB prompt, this may take a while)",
                total_kb
            ));
        }
        let response = run_coordinator_request(
            &self.config.model,
            self.config.ollama_base_url.as_deref(),
            workspace_root,
            &system_prompt,
            &user_prompt,
            obs,
            "Building DSL plan...",
        )
        .await?;

        // Strip markdown code fences if Opus wrapped the DSL in ```gaviero ... ```
        let dsl = strip_code_fence(&response);
        if dsl.trim().is_empty() {
            anyhow::bail!("coordinator returned empty DSL response");
        }
        Ok(dsl.to_string())
    }

    /// Validate the coordinator's output beyond JSON parsing.
    pub fn validate_dag(&self, dag: &TaskDAG) -> Result<()> {
        let units = &dag.units;

        // Check scope overlaps (reuse existing validation).
        // TaskDAG has no loop groupings — coordinator-produced DAGs are flat.
        let scope_errors = validation::validate_scopes(units, &[]);
        if !scope_errors.is_empty() {
            anyhow::bail!(
                "coordinator DAG has scope overlaps: {}",
                scope_errors
                    .iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }

        // Check dependency cycles
        validation::dependency_tiers(units)
            .map_err(|e| anyhow::anyhow!("coordinator DAG has dependency cycle: {}", e))?;

        // Check all depends_on references point to valid unit IDs
        let valid_ids: std::collections::HashSet<&str> =
            units.iter().map(|u| u.id.as_str()).collect();
        for unit in units {
            for dep in &unit.depends_on {
                if !valid_ids.contains(dep.as_str()) {
                    anyhow::bail!(
                        "unit '{}' depends on '{}' which is not in the DAG",
                        unit.id,
                        dep
                    );
                }
            }
        }

        Ok(())
    }

    /// Detect cross-run continuity from memory.
    ///
    /// Two matching strategies:
    /// 1. Explicit: if the prompt contains "run:{run_id}", look up that run directly
    /// 2. Semantic: search memory for agent results matching the prompt (score > 0.8)
    pub async fn detect_continuity(
        &self,
        prompt: &str,
        namespaces: &[String],
    ) -> Option<ContinuityContext> {
        let mem = self.memory.as_ref()?;

        // Strategy 1: explicit run reference
        if let Some(run_id) = extract_run_id(prompt) {
            return self.load_continuity_for_run(mem, namespaces, &run_id).await;
        }

        // Strategy 2: semantic search for matching prior runs
        let results = mem.search_multi(namespaces, prompt, 20).await.ok()?;

        // Filter to agent result entries with high similarity
        let agent_results: Vec<_> = results
            .iter()
            .filter(|r| r.entry.key.starts_with("agents:") && r.score > 0.8)
            .collect();

        if agent_results.is_empty() {
            return None;
        }

        // Group by run_id and pick the best-matching run
        let mut run_scores: std::collections::HashMap<String, (f32, usize)> =
            std::collections::HashMap::new();
        for r in &agent_results {
            if let Some(run_id) = extract_run_id_from_key(&r.entry.key) {
                let entry = run_scores.entry(run_id).or_insert((0.0, 0));
                entry.0 += r.score;
                entry.1 += 1;
            }
        }

        let best_run = run_scores.into_iter().max_by(|a, b| {
            a.1.0
                .partial_cmp(&b.1.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;

        self.load_continuity_for_run(mem, namespaces, &best_run.0)
            .await
    }

    /// Load continuity context for a specific run ID from memory.
    async fn load_continuity_for_run(
        &self,
        mem: &MemoryStore,
        namespaces: &[String],
        run_id: &str,
    ) -> Option<ContinuityContext> {
        // Search for all entries from this run
        let query = format!("agents:{}", run_id);
        let results = mem.search_multi(namespaces, &query, 50).await.ok()?;

        let agent_entries: Vec<_> = results
            .iter()
            .filter(|r| r.entry.key.starts_with(&format!("agents:{}:", run_id)))
            .collect();

        if agent_entries.is_empty() {
            return None;
        }

        // Partition into succeeded (content contains "Completed") and failed
        let mut completed_units = Vec::new();
        let mut failed_units = Vec::new();

        for entry in &agent_entries {
            let unit_id = entry
                .entry
                .key
                .strip_prefix(&format!("agents:{}:", run_id))
                .unwrap_or("")
                .to_string();

            if entry.entry.content.contains("Completed") || !entry.entry.content.contains("Failed")
            {
                completed_units.push(unit_id);
            } else {
                failed_units.push(FailedUnit {
                    id: unit_id,
                    failure_reason: entry.entry.content.clone(),
                    tier_at_failure: ModelTier::Cheap, // Approximate
                });
            }
        }

        // Load plan summary from verification entry if available
        let verification_key = format!("verification:{}", run_id);
        let plan_summary = mem
            .get(
                namespaces.first().map(|s| s.as_str()).unwrap_or("default"),
                &verification_key,
            )
            .await
            .ok()
            .flatten()
            .map(|e| e.content)
            .unwrap_or_default();

        Some(ContinuityContext {
            prior_run_id: run_id.to_string(),
            completed_units,
            failed_units,
            prior_plan_summary: plan_summary,
            prior_dependency_graph: Vec::new(),
        })
    }
}

async fn run_coordinator_request(
    model: &str,
    ollama_base_url: Option<&str>,
    workspace_root: &Path,
    system_prompt: &str,
    user_prompt: &str,
    observer: Option<&dyn crate::observer::AcpObserver>,
    building_status: &str,
) -> Result<String> {
    if let Some(obs) = observer {
        obs.on_streaming_status(building_status);
    }

    let backend = shared::create_backend_for_model(model, ollama_base_url)?;
    let response = executor::complete_to_text(
        &*backend,
        CompletionRequest {
            prompt: user_prompt.to_string(),
            system_prompt: Some(system_prompt.to_string()),
            workspace_root: workspace_root.to_path_buf(),
            allowed_tools: vec![],
            file_attachments: vec![],
            conversation_history: vec![],
            file_refs: vec![],
            effort: None,
            extra: Vec::new(),
            max_tokens: None,
            auto_approve: true,
        },
        observer,
    )
    .await?;

    Ok(response.text)
}

/// Extract a human-readable detail string from tool input JSON.
#[allow(dead_code)]
pub(crate) fn extract_tool_detail(tool_name: &str, input_json: &str) -> String {
    // Try full JSON parse first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(input_json) {
        let arg = match tool_name {
            "Read" | "read" => v
                .get("file_path")
                .or_else(|| v.get("path"))
                .and_then(|v| v.as_str()),
            "Glob" | "glob" => v.get("pattern").and_then(|v| v.as_str()),
            "Grep" | "grep" => v.get("pattern").and_then(|v| v.as_str()),
            "Write" | "write" | "Edit" | "edit" => v
                .get("file_path")
                .or_else(|| v.get("path"))
                .and_then(|v| v.as_str()),
            "Bash" | "bash" => v
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| if s.len() > 60 { &s[..60] } else { s }),
            _ => None,
        };
        if let Some(arg) = arg {
            return format!("[{}] {}", tool_name, arg);
        }
    }

    // Fallback: extract first string value from partial JSON
    // Look for "file_path":"...", "pattern":"...", etc.
    for key in &["file_path", "path", "pattern", "command"] {
        let needle = format!("\"{}\":\"", key);
        if let Some(pos) = input_json.find(&needle) {
            let start = pos + needle.len();
            let rest = &input_json[start..];
            let end = rest.find('"').unwrap_or(rest.len().min(80));
            let val = &rest[..end];
            if !val.is_empty() {
                return format!("[{}] {}", tool_name, val);
            }
        }
    }

    format!("[{}]", tool_name)
}

fn extract_run_id(prompt: &str) -> Option<String> {
    for word in prompt.split_whitespace() {
        if let Some(id) = word.strip_prefix("run:") {
            if !id.is_empty() {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Extract the run ID from a key like "agents:run123:unit-a".
fn extract_run_id_from_key(key: &str) -> Option<String> {
    let rest = key.strip_prefix("agents:")?;
    let colon = rest.find(':')?;
    Some(rest[..colon].to_string())
}

// ── Lenient JSON parsing for LLM output ─────────────────────────

/// Parse a TaskDAG from LLM-produced JSON, tolerating varying field names,
/// missing fields, extra fields, and non-standard enum representations.
///
/// This does NOT use serde derive on the top-level structure — instead it
/// parses into `serde_json::Value` and extracts fields manually with fallbacks.
fn parse_task_dag_lenient(json_str: &str) -> Result<TaskDAG> {
    let v: serde_json::Value = serde_json::from_str(json_str)
        .with_context(|| format!("invalid JSON: {}", &json_str[..json_str.len().min(200)]))?;

    let obj = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("expected JSON object, got {}", v_type(&v)))?;

    // plan_summary — string, optional
    let plan_summary = obj
        .get("plan_summary")
        .or_else(|| obj.get("summary"))
        .or_else(|| obj.get("plan"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // units — array of work unit objects (required)
    let units_val = obj
        .get("units")
        .or_else(|| obj.get("tasks"))
        .or_else(|| obj.get("work_units"))
        .ok_or_else(|| anyhow::anyhow!("missing 'units' array in TaskDAG"))?;
    let units_arr = units_val
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("'units' must be an array, got {}", v_type(units_val)))?;

    let mut units = Vec::with_capacity(units_arr.len());
    for (i, unit_val) in units_arr.iter().enumerate() {
        match parse_work_unit_lenient(unit_val) {
            Ok(unit) => units.push(unit),
            Err(e) => {
                tracing::warn!("Skipping unit {}: {}", i, e);
            }
        }
    }

    if units.is_empty() {
        anyhow::bail!(
            "no valid work units in TaskDAG (parsed {} entries)",
            units_arr.len()
        );
    }

    // verification_strategy — optional, default to Combined
    let verification_strategy = obj
        .get("verification_strategy")
        .or_else(|| obj.get("verification"))
        .map(parse_verification_strategy)
        .unwrap_or_default();

    // dependency_graph — optional, extract from units.depends_on if not present
    let dependency_graph = obj
        .get("dependency_graph")
        .and_then(|v| parse_dep_graph(v))
        .unwrap_or_default();

    Ok(TaskDAG {
        plan_summary,
        units,
        dependency_graph,
        verification_strategy,
        continued_from: None,
    })
}

/// Parse a single WorkUnit from a JSON value, leniently.
fn parse_work_unit_lenient(v: &serde_json::Value) -> Result<WorkUnit> {
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("work unit must be an object"))?;

    let id = get_str(obj, &["id", "name", "unit_id"])
        .ok_or_else(|| anyhow::anyhow!("work unit missing 'id'"))?;

    let description =
        get_str(obj, &["description", "task", "title", "summary"]).unwrap_or_default();

    let coordinator_instructions = get_str(
        obj,
        &[
            "coordinator_instructions",
            "instructions",
            "prompt",
            "details",
        ],
    )
    .unwrap_or_default();

    // scope — object with owned_paths, or array of strings, or single string
    let scope = obj.get("scope").map(parse_scope).unwrap_or_default();

    // depends_on — array of strings
    let depends_on = obj
        .get("depends_on")
        .or_else(|| obj.get("dependencies"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // tier — string like "reasoning", "execution", "mechanical"
    let tier = obj
        .get("tier")
        .and_then(|v| v.as_str())
        .map(parse_model_tier)
        .unwrap_or(ModelTier::Cheap);

    // privacy — string like "public", "local_only"
    let privacy = obj
        .get("privacy")
        .and_then(|v| v.as_str())
        .map(parse_privacy_level)
        .unwrap_or(PrivacyLevel::Public);

    let estimated_tokens = obj
        .get("estimated_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let max_retries = obj.get("max_retries").and_then(|v| v.as_u64()).unwrap_or(1) as u8;

    let escalation_tier = obj
        .get("escalation_tier")
        .and_then(|v| v.as_str())
        .map(parse_model_tier);

    let model = obj.get("model").and_then(|v| v.as_str()).map(String::from);

    Ok(WorkUnit {
        id,
        description,
        scope,
        depends_on,
        backend: AgentBackend::default(),
        model,
        effort: None,
        extra: Vec::new(),
        tier,
        privacy,
        coordinator_instructions,
        estimated_tokens,
        max_retries,
        escalation_tier,
        read_namespaces: None,
        write_namespace: None,
        memory_importance: None,
        staleness_sources: Vec::new(),
        memory_read_query: None,
        memory_read_limit: None,
        memory_write_content: None,
        impact_scope: false,
        context_callers_of: vec![],
        context_tests_for: vec![],
        context_depth: 2,
        extra_allowed_tools: Vec::new(),
    })
}

/// Parse FileScope from various LLM shapes.
fn parse_scope(v: &serde_json::Value) -> FileScope {
    match v {
        // Object: { "owned_paths": [...], "read_only_paths": [...] }
        serde_json::Value::Object(obj) => {
            let owned = obj
                .get("owned_paths")
                .or_else(|| obj.get("write"))
                .or_else(|| obj.get("files"))
                .and_then(parse_string_array)
                .unwrap_or_default();
            let read_only = obj
                .get("read_only_paths")
                .or_else(|| obj.get("read_only"))
                .or_else(|| obj.get("read"))
                .and_then(parse_string_array)
                .unwrap_or_default();
            FileScope {
                owned_paths: owned,
                read_only_paths: read_only,
                interface_contracts: std::collections::HashMap::new(),
            }
        }
        // Array: ["src/auth/", "src/types.rs"] → all as owned
        serde_json::Value::Array(arr) => {
            let owned = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            FileScope {
                owned_paths: owned,
                read_only_paths: Vec::new(),
                interface_contracts: std::collections::HashMap::new(),
            }
        }
        // String: "src/auth/" → single owned path
        serde_json::Value::String(s) => FileScope {
            owned_paths: vec![s.clone()],
            read_only_paths: Vec::new(),
            interface_contracts: std::collections::HashMap::new(),
        },
        _ => FileScope::default(),
    }
}

/// Parse VerificationStrategy from various LLM shapes.
fn parse_verification_strategy(v: &serde_json::Value) -> VerificationStrategy {
    let obj = match v.as_object() {
        Some(o) => o,
        None => return VerificationStrategy::default(),
    };

    // Check "type" field for variant
    let variant = obj
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("combined")
        .to_lowercase();

    match variant.as_str() {
        "structural_only" | "structural" => VerificationStrategy::StructuralOnly,
        "test_suite" | "test" | "tests" => {
            let command = obj
                .get("test_command")
                .or_else(|| obj.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("cargo test")
                .to_string();
            VerificationStrategy::TestSuite {
                command,
                targeted: obj
                    .get("targeted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            }
        }
        _ => {
            // "combined" or any unknown → Combined with defaults
            let review_tiers = obj
                .get("review_tiers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(parse_model_tier))
                        .collect()
                })
                .unwrap_or_else(|| vec![ModelTier::Cheap]);
            let test_command = obj
                .get("test_command")
                .or_else(|| obj.get("command"))
                .and_then(|v| v.as_str())
                .map(String::from);
            VerificationStrategy::Combined {
                review_tiers,
                test_command,
            }
        }
    }
}

fn parse_dep_graph(v: &serde_json::Value) -> Option<Vec<(String, String)>> {
    let arr = v.as_array()?;
    let mut edges = Vec::new();
    for item in arr {
        if let Some(pair) = item.as_array() {
            if pair.len() >= 2 {
                if let (Some(a), Some(b)) = (pair[0].as_str(), pair[1].as_str()) {
                    edges.push((a.to_string(), b.to_string()));
                }
            }
        } else if let Some(obj) = item.as_object() {
            if let (Some(from), Some(to)) = (
                obj.get("from").and_then(|v| v.as_str()),
                obj.get("to").and_then(|v| v.as_str()),
            ) {
                edges.push((from.to_string(), to.to_string()));
            }
        }
    }
    Some(edges)
}

// ── Helpers ─────────────────────────────────────────────────────

fn get_str(obj: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(v) = obj.get(*key) {
            if let Some(s) = v.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn parse_string_array(v: &serde_json::Value) -> Option<Vec<String>> {
    v.as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect()
    })
}

fn parse_model_tier(s: &str) -> ModelTier {
    match s.to_lowercase().as_str() {
        "expensive" | "coordinator" | "coord" | "c" | "reasoning" | "reason" | "r" => {
            ModelTier::Expensive
        }
        "cheap" | "execution" | "exec" | "e" | "mechanical" | "mech" | "m" => ModelTier::Cheap,
        _ => ModelTier::Cheap,
    }
}

fn parse_privacy_level(s: &str) -> PrivacyLevel {
    match s.to_lowercase().as_str() {
        "local_only" | "local" | "private" => PrivacyLevel::LocalOnly,
        _ => PrivacyLevel::Public,
    }
}

fn v_type(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Auto-resolve scope overlaps by moving conflicting paths to read_only.
///
/// First-come-first-served: the first unit (by array order) that claims a file
/// keeps it in owned_paths. Later units get it moved to read_only_paths.
/// Also handles directory prefix overlaps (e.g., `src/` vs `src/main.rs`).
///
/// Returns a list of human-readable fix descriptions.
fn resolve_scope_overlaps(units: &mut [WorkUnit]) -> Vec<String> {
    use std::collections::HashMap;

    // file_owner: normalized path → index of the unit that owns it
    let mut file_owner: HashMap<String, usize> = HashMap::new();
    let mut fixes: Vec<String> = Vec::new();

    for i in 0..units.len() {
        let mut to_move = Vec::new(); // indices into owned_paths to move to read_only

        for (j, path) in units[i].scope.owned_paths.iter().enumerate() {
            let normalized = crate::types::normalize_path(path);

            // Check exact match
            if let Some(&owner_idx) = file_owner.get(&normalized) {
                fixes.push(format!(
                    "{}: '{}' → read_only (owned by {})",
                    units[i].id, normalized, units[owner_idx].id,
                ));
                to_move.push(j);
                continue;
            }

            // Check directory prefix overlaps against all existing owners
            let mut conflict = false;
            for (owned_path, &owner_idx) in &file_owner {
                if paths_overlap_normalized(&normalized, owned_path) {
                    fixes.push(format!(
                        "{}: '{}' → read_only (overlaps '{}' owned by {})",
                        units[i].id, normalized, owned_path, units[owner_idx].id,
                    ));
                    to_move.push(j);
                    conflict = true;
                    break;
                }
            }

            if !conflict {
                file_owner.insert(normalized, i);
            }
        }

        // Move conflicting paths: remove from owned, add to read_only
        // Process in reverse to preserve indices
        for &j in to_move.iter().rev() {
            let path = units[i].scope.owned_paths.remove(j);
            if !units[i].scope.read_only_paths.contains(&path) {
                units[i].scope.read_only_paths.push(path);
            }
        }
    }

    fixes
}

/// Check if two normalized paths overlap (same as validation.rs logic).
fn paths_overlap_normalized(a: &str, b: &str) -> bool {
    crate::path_pattern::patterns_overlap(a, b)
}

/// Build the system prompt for DSL-output coordination.
///
/// Instead of JSON, the coordinator produces a `.gaviero` file that:
/// 1. Can be inspected and edited by the user before execution
/// 2. Is validated by the DSL compiler (miette errors on bad syntax)
/// 3. Explicitly shows which files will be created vs. which exist
fn build_coordinator_dsl_prompt(file_list: &[String], memory_context: &str) -> String {
    let mut prompt = String::from(
        "You are a code architect decomposing a development task into a `.gaviero` DSL workflow.\n\n\
         OUTPUT FORMAT — you MUST respond with ONLY a valid `.gaviero` file, no prose:\n\n\
         ```\n\
         // Optional comment\n\
         client <name> { tier <tier>  model \"<model-id>\" }\n\
         // tier is one of: coordinator | reasoning | execution | mechanical\n\n\
         agent <id> {\n\
             description \"<one-line summary>\"\n\
             client <client-name>\n\
             scope {\n\
                 owned    [\"path/\" \"file.rs\"]   // files this agent may write\n\
                 read_only [\"other/\"]             // files this agent may only read\n\
             }\n\
             depends_on [<other-agent-id> ...]    // omit if no dependencies\n\
             prompt #\"\n\
                 <self-contained task specification>\n\
             \"#\n\
             max_retries <n>   // optional, default 1\n\
         }\n\n\
         workflow main {\n\
             steps [<agent-ids in any order — DAG handles ordering>]\n\
             max_parallel <n>   // optional\n\
         }\n\
         ```\n\n\
         SCOPE RULES:\n\
         - owned_paths MUST be disjoint across all agents — each file in at most ONE agent's owned list\n\
         - If a file does not yet exist in the workspace, add a comment `// (will be created)` after its path\n\
         - read_only paths may overlap freely\n\n\
         PARALLELISM:\n\
         - Maximize agents that can run simultaneously (same tier with no depends_on)\n\
         - Only add depends_on when an agent TRULY needs another's output\n\
         - Prefer wide shallow DAGs (2-3 tiers) over deep linear chains\n\n\
         TIER ASSIGNMENT:\n\
         - reasoning: multi-file semantic changes, interface redesigns, complex logic\n\
         - execution: single-file focused changes, test writing, error handling\n\
         - mechanical: renames, import updates, call-site propagation, formatting\n\n\
         CLIENT NAMES to use:\n\
         - `reasoning` for reasoning-tier agents\n\
         - `execution` for execution-tier agents\n\
         - `mechanical` for mechanical-tier agents\n\
         - Prefer omitting explicit `model` fields so runtime routing can choose the active provider.\n\
           Only set a concrete model when the task truly requires a provider-specific override.\n\n\
         PROMPT CONTENT:\n\
         - Each agent's prompt must be SELF-CONTAINED — include all context needed\n\
         - Agents run in isolated git worktrees; they cannot access tmp/ or gitignored files\n\
         - If task requires content from an inlined [File: path]...[End of file: path] block,\n\
           copy the COMPLETE relevant sections verbatim into the prompt block\n\n\
         Respond with ONLY the `.gaviero` file. No explanation, no prose.\n\n",
    );

    if !file_list.is_empty() {
        prompt.push_str("Workspace files:\n");
        for f in file_list.iter().take(200) {
            prompt.push_str(&format!("  {}\n", f));
        }
        prompt.push('\n');
    }

    if !memory_context.is_empty() {
        prompt.push_str("MEMORY CONTEXT:\n");
        prompt.push_str(memory_context);
        prompt.push('\n');
    }

    prompt
}

/// Strip markdown code fences if the model wrapped the DSL in ```gaviero ... ``` or ``` ... ```.
///
/// The model may emit prose before the opening fence (e.g. "Here's the plan:\n```gaviero\n…").
/// We find the first fence anywhere in the text, not just at the very start.
fn strip_code_fence(text: &str) -> &str {
    let trimmed = text.trim();

    // Find the first opening fence (prefer the language-tagged one)
    let fence_pos = trimmed.find("```gaviero").or_else(|| trimmed.find("```"));

    if let Some(pos) = fence_pos {
        let after_fence = &trimmed[pos..];
        // Skip past the opening fence line to the first newline
        let inner_start = after_fence
            .find('\n')
            .map(|p| p + 1)
            .unwrap_or(after_fence.len());
        let inner = &after_fence[inner_start..];
        // Remove trailing ```
        if let Some(end) = inner.rfind("```") {
            return inner[..end].trim_end();
        }
        return inner.trim_end();
    }
    trimmed
}

fn build_coordinator_prompt(file_list: &[String], memory_context: &str) -> String {
    let mut prompt = String::from(
        "You are a code architect decomposing a development task into PARALLELIZABLE \
         subtasks for a multi-agent swarm.\n\n\
         PARALLELISM IS CRITICAL:\n\
         - Maximize independent units that can run simultaneously\n\
         - Minimize dependency chains — prefer WIDE SHALLOW trees over deep linear chains\n\
         - Only add depends_on when a unit TRULY needs the output of another\n\
         - Units that touch different files/modules can usually run in parallel\n\
         - Aim for at most 3-4 dependency tiers, even for large tasks\n\
         - Bad: A→B→C→D→E (serial, 5 tiers). Good: A,B,C in parallel → D,E in parallel (2 tiers)\n\n\
         TASK SIZING:\n\
         - Each unit should be a FOCUSED task completable in 1-3 minutes\n\
         - Prefer many small units over few large ones\n\
         - Each unit should touch at most 3-5 files\n\
         - The coordinator_instructions should be precise and self-contained\n\n\
         For each subtask, assign:\n\n\
         - id: short identifier (e.g., \"auth-models\", \"api-routes\")\n\
         - description: one-line summary\n\
         - tier: \"reasoning\" | \"execution\" | \"mechanical\"\n\
           - reasoning: multi-file semantic changes, interface redesigns, complex logic\n\
           - execution: single-file focused changes, test writing, error handling\n\
           - mechanical: renames, import updates, call-site propagation, formatting\n\n\
         - depends_on: IDs of subtasks that MUST complete first (minimize these!)\n\
         - scope: {{ owned_paths: [...], read_only_paths: [...] }}\n\
           CRITICAL: owned_paths must be DISJOINT across all units.\n\
           Each file may appear in at most ONE unit's owned_paths.\n\
           If multiple units need the same file, assign it to one unit and\n\
           use read_only_paths for the others.\n\
         - coordinator_instructions: Self-contained task specification for the subagent.\n\
           For simple mechanical tasks: 1-2 sentences is fine.\n\
           For complex implementation tasks: include ALL context the agent needs — full excerpts,\n\
           data structures, interfaces, expected outputs, constraints — as many paragraphs as needed.\n\
           Subagents run in isolated git worktrees and can only access git-tracked files with their\n\
           Read/Glob/Grep tools. They CANNOT access gitignored or tmp/ files.\n\
           If the task requires content from an inlined [File: path]...[End of file: path] block,\n\
           copy the COMPLETE relevant sections verbatim into coordinator_instructions.\n\
           Do NOT summarize or truncate — agents that lack context will produce empty results.\n\
         - estimated_tokens: approximate context needed\n\n\
         INLINED FILES: [File: path]...[End of file: path] blocks are available to YOU (coordinator)\n\
         but NOT to subagents. NEVER tell a subagent to read such a file by path.\n\
         Always embed the full relevant content directly in coordinator_instructions.\n\n\
         Select a verification_strategy:\n\
         - \"combined\" (DEFAULT): structural + diff review + tests\n\
         - \"structural_only\": when ALL subtasks are mechanical\n\
         - \"test_suite\": when a test suite exists and structural + tests suffice\n\n\
         Output a JSON object with: plan_summary, units, verification_strategy.\n\n",
    );

    if !file_list.is_empty() {
        prompt.push_str("Workspace files:\n");
        for f in file_list.iter().take(200) {
            prompt.push_str(&format!("  {}\n", f));
        }
        prompt.push('\n');
    }

    if !memory_context.is_empty() {
        prompt.push_str("MEMORY CONTEXT:\n");
        prompt.push_str(memory_context);
        prompt.push('\n');
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FileScope, PrivacyLevel};
    use std::collections::HashMap;

    fn make_unit(id: &str, deps: &[&str]) -> WorkUnit {
        WorkUnit {
            id: id.into(),
            description: format!("Task {}", id),
            scope: FileScope {
                owned_paths: vec![format!("src/{}/", id)],
                read_only_paths: vec![],
                interface_contracts: HashMap::new(),
            },
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            backend: Default::default(),
            model: None,
            effort: None,
            extra: Vec::new(),
            tier: ModelTier::Cheap,
            privacy: PrivacyLevel::Public,
            coordinator_instructions: String::new(),
            estimated_tokens: 4000,
            max_retries: 1,
            escalation_tier: Some(ModelTier::Expensive),
            read_namespaces: None,
            write_namespace: None,
            memory_importance: None,
            staleness_sources: vec![],
            memory_read_query: None,
            memory_read_limit: None,
            memory_write_content: None,
            impact_scope: false,
            context_callers_of: vec![],
            context_tests_for: vec![],
            context_depth: 2,
            extra_allowed_tools: vec![],
        }
    }

    #[test]
    fn test_validate_dag_valid() {
        let dag = TaskDAG {
            plan_summary: "Test plan".into(),
            units: vec![make_unit("a", &[]), make_unit("b", &["a"])],
            dependency_graph: vec![("a".into(), "b".into())],
            verification_strategy: VerificationStrategy::StructuralOnly,
            continued_from: None,
        };
        let coord = Coordinator::new(None, CoordinatorConfig::default());
        assert!(coord.validate_dag(&dag).is_ok());
    }

    #[test]
    fn test_validate_dag_invalid_dep_reference() {
        let dag = TaskDAG {
            plan_summary: "Test".into(),
            units: vec![make_unit("a", &["nonexistent"])],
            dependency_graph: vec![],
            verification_strategy: VerificationStrategy::StructuralOnly,
            continued_from: None,
        };
        let coord = Coordinator::new(None, CoordinatorConfig::default());
        let err = coord.validate_dag(&dag).unwrap_err();
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn test_validate_dag_scope_overlap() {
        let mut u1 = make_unit("a", &[]);
        let mut u2 = make_unit("b", &[]);
        // Give them overlapping paths
        u1.scope.owned_paths = vec!["src/shared.rs".into()];
        u2.scope.owned_paths = vec!["src/shared.rs".into()];

        let dag = TaskDAG {
            plan_summary: "overlap test".into(),
            units: vec![u1, u2],
            dependency_graph: vec![],
            verification_strategy: VerificationStrategy::StructuralOnly,
            continued_from: None,
        };
        let coord = Coordinator::new(None, CoordinatorConfig::default());
        assert!(coord.validate_dag(&dag).is_err());
    }

    #[test]
    fn test_validate_dag_cycle() {
        let dag = TaskDAG {
            plan_summary: "cycle test".into(),
            units: vec![make_unit("a", &["b"]), make_unit("b", &["a"])],
            dependency_graph: vec![],
            verification_strategy: VerificationStrategy::StructuralOnly,
            continued_from: None,
        };
        let coord = Coordinator::new(None, CoordinatorConfig::default());
        assert!(coord.validate_dag(&dag).is_err());
    }

    #[test]
    fn test_extract_run_id_from_prompt() {
        assert_eq!(extract_run_id("retry run:abc123"), Some("abc123".into()));
        assert_eq!(extract_run_id("/retry run:xyz"), Some("xyz".into()));
        assert_eq!(extract_run_id("fix the bug"), None);
        assert_eq!(extract_run_id("run:"), None);
    }

    #[test]
    fn test_extract_run_id_from_key() {
        assert_eq!(
            extract_run_id_from_key("agents:run123:unit-a"),
            Some("run123".into())
        );
        assert_eq!(
            extract_run_id_from_key("agents:17111:design"),
            Some("17111".into())
        );
        assert_eq!(extract_run_id_from_key("user:note"), None);
        assert_eq!(extract_run_id_from_key("agents:"), None);
    }

    #[tokio::test]
    async fn test_detect_continuity_no_memory() {
        let coord = Coordinator::new(None, CoordinatorConfig::default());
        let result = coord.detect_continuity("fix bug", &["ns".into()]).await;
        assert!(result.is_none());
    }

    #[test]
    fn test_task_dag_serde_roundtrip() {
        let dag = TaskDAG {
            plan_summary: "Refactor auth".into(),
            units: vec![make_unit("a", &[])],
            dependency_graph: vec![],
            verification_strategy: VerificationStrategy::Combined {
                review_tiers: vec![ModelTier::Cheap],
                test_command: Some("cargo test".into()),
            },
            continued_from: None,
        };
        let json = serde_json::to_string(&dag).unwrap();
        let back: TaskDAG = serde_json::from_str(&json).unwrap();
        assert_eq!(back.plan_summary, "Refactor auth");
        assert_eq!(back.units.len(), 1);
    }

    // ── Lenient parser tests ────────────────────────────────────

    #[test]
    fn test_lenient_minimal_json() {
        // Minimal JSON that an LLM might produce
        let json = r#"{
            "plan_summary": "Refactor auth",
            "units": [
                { "id": "auth", "tier": "reasoning" },
                { "id": "tests", "tier": "execution", "depends_on": ["auth"] }
            ]
        }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(dag.plan_summary, "Refactor auth");
        assert_eq!(dag.units.len(), 2);
        assert_eq!(dag.units[0].id, "auth");
        assert_eq!(dag.units[0].tier, ModelTier::Expensive); // reasoning → Expensive
        assert_eq!(dag.units[1].depends_on, vec!["auth"]);
    }

    #[test]
    fn test_lenient_with_description_variants() {
        let json = r#"{
            "plan_summary": "Test",
            "units": [
                { "id": "a", "task": "Do something" },
                { "id": "b", "title": "Another thing" },
                { "id": "c", "description": "Third thing" }
            ]
        }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(dag.units[0].description, "Do something");
        assert_eq!(dag.units[1].description, "Another thing");
        assert_eq!(dag.units[2].description, "Third thing");
    }

    #[test]
    fn test_lenient_scope_variants() {
        // scope as object
        let json = r#"{
            "units": [{ "id": "a", "scope": { "owned_paths": ["src/auth/"], "read_only_paths": ["src/types.rs"] } }]
        }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(dag.units[0].scope.owned_paths, vec!["src/auth/"]);
        assert_eq!(dag.units[0].scope.read_only_paths, vec!["src/types.rs"]);

        // scope as array
        let json = r#"{ "units": [{ "id": "b", "scope": ["src/auth.rs", "src/lib.rs"] }] }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(
            dag.units[0].scope.owned_paths,
            vec!["src/auth.rs", "src/lib.rs"]
        );

        // scope as string
        let json = r#"{ "units": [{ "id": "c", "scope": "src/" }] }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(dag.units[0].scope.owned_paths, vec!["src/"]);

        // scope missing
        let json = r#"{ "units": [{ "id": "d" }] }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert!(dag.units[0].scope.owned_paths.is_empty());
    }

    #[test]
    fn test_lenient_verification_strategy() {
        let json = r#"{
            "units": [{ "id": "a" }],
            "verification_strategy": {
                "type": "combined",
                "review_tiers": ["reasoning", "execution"],
                "test_command": "cargo test && npm run build",
                "notes": "Extra field that should be ignored"
            }
        }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        match &dag.verification_strategy {
            VerificationStrategy::Combined {
                review_tiers,
                test_command,
            } => {
                assert_eq!(review_tiers.len(), 2);
                assert_eq!(test_command.as_deref(), Some("cargo test && npm run build"));
            }
            _ => panic!("expected Combined"),
        }
    }

    #[test]
    fn test_lenient_missing_verification() {
        let json = r#"{ "units": [{ "id": "a" }] }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert!(matches!(
            dag.verification_strategy,
            VerificationStrategy::Combined { .. }
        ));
    }

    #[test]
    fn test_lenient_alt_field_names() {
        let json = r#"{
            "summary": "My plan",
            "tasks": [
                { "name": "P0", "instructions": "Do X", "dependencies": ["P1"] }
            ]
        }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(dag.plan_summary, "My plan");
        assert_eq!(dag.units[0].id, "P0");
        assert_eq!(dag.units[0].coordinator_instructions, "Do X");
        assert_eq!(dag.units[0].depends_on, vec!["P1"]);
    }

    #[test]
    fn test_lenient_tier_parsing() {
        assert_eq!(parse_model_tier("reasoning"), ModelTier::Expensive);
        assert_eq!(parse_model_tier("Reasoning"), ModelTier::Expensive);
        assert_eq!(parse_model_tier("EXECUTION"), ModelTier::Cheap);
        assert_eq!(parse_model_tier("mechanical"), ModelTier::Cheap);
        assert_eq!(parse_model_tier("mech"), ModelTier::Cheap);
        assert_eq!(parse_model_tier("unknown"), ModelTier::Cheap); // fallback
    }

    #[test]
    fn test_lenient_skips_bad_units() {
        let json = r#"{
            "units": [
                { "id": "good" },
                "not an object",
                42,
                { "id": "also_good", "tier": "reasoning" }
            ]
        }"#;
        let dag = parse_task_dag_lenient(json).unwrap();
        assert_eq!(dag.units.len(), 2);
        assert_eq!(dag.units[0].id, "good");
        assert_eq!(dag.units[1].id, "also_good");
    }

    // ── Scope overlap resolution tests ──────────────────────────

    #[test]
    fn test_resolve_no_overlaps() {
        let mut units = vec![make_unit("a", &[]), make_unit("b", &[])];
        units[0].scope.owned_paths = vec!["src/a.rs".into()];
        units[1].scope.owned_paths = vec!["src/b.rs".into()];

        let fixes = resolve_scope_overlaps(&mut units);
        assert!(fixes.is_empty());
        assert_eq!(units[0].scope.owned_paths, vec!["src/a.rs"]);
        assert_eq!(units[1].scope.owned_paths, vec!["src/b.rs"]);
    }

    #[test]
    fn test_resolve_exact_file_conflict() {
        let mut units = vec![make_unit("backend", &[]), make_unit("database", &[])];
        units[0].scope.owned_paths = vec!["src/main.rs".into(), "src/config.rs".into()];
        units[1].scope.owned_paths = vec!["src/main.rs".into(), "src/db.rs".into()];

        let fixes = resolve_scope_overlaps(&mut units);
        assert_eq!(fixes.len(), 1);
        assert!(fixes[0].contains("src/main.rs"));
        assert!(fixes[0].contains("database"));
        assert!(fixes[0].contains("owned by backend"));

        // backend keeps src/main.rs
        assert!(
            units[0]
                .scope
                .owned_paths
                .contains(&"src/main.rs".to_string())
        );
        // database has it moved to read_only
        assert!(
            !units[1]
                .scope
                .owned_paths
                .contains(&"src/main.rs".to_string())
        );
        assert!(
            units[1]
                .scope
                .read_only_paths
                .contains(&"src/main.rs".to_string())
        );
        // database keeps src/db.rs
        assert!(
            units[1]
                .scope
                .owned_paths
                .contains(&"src/db.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_directory_prefix_overlap() {
        let mut units = vec![make_unit("full", &[]), make_unit("partial", &[])];
        units[0].scope.owned_paths = vec!["src/".into()];
        units[1].scope.owned_paths = vec!["src/main.rs".into()];

        let fixes = resolve_scope_overlaps(&mut units);
        assert_eq!(fixes.len(), 1);
        // "full" owns src/ so "partial" can't own src/main.rs
        assert!(
            !units[1]
                .scope
                .owned_paths
                .contains(&"src/main.rs".to_string())
        );
        assert!(
            units[1]
                .scope
                .read_only_paths
                .contains(&"src/main.rs".to_string())
        );
    }

    #[test]
    fn test_resolve_multiple_conflicts() {
        let mut units = vec![
            make_unit("a", &[]),
            make_unit("b", &[]),
            make_unit("c", &[]),
        ];
        units[0].scope.owned_paths = vec!["shared.rs".into()];
        units[1].scope.owned_paths = vec!["shared.rs".into()];
        units[2].scope.owned_paths = vec!["shared.rs".into()];

        let fixes = resolve_scope_overlaps(&mut units);
        assert_eq!(fixes.len(), 2); // b and c both conflict with a
        assert!(
            units[0]
                .scope
                .owned_paths
                .contains(&"shared.rs".to_string())
        );
        assert!(
            units[1]
                .scope
                .read_only_paths
                .contains(&"shared.rs".to_string())
        );
        assert!(
            units[2]
                .scope
                .read_only_paths
                .contains(&"shared.rs".to_string())
        );
    }
}
