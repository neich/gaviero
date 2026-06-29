use anyhow::Result;

use crate::context_planner::PlannerSelections;
use crate::types::FileScope;

use super::{
    AgentBackend, BackendConfig, Capabilities, CompletionRequest, RetrievalToolset, create_backend,
};

const HISTORY_TRUNCATION_CHARS: usize = 2000;
const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
pub const SUPPORTED_PROVIDER_PREFIXES: &[&str] =
    &["claude", "codex", "cursor", "ollama", "local", "deepseek"];

/// DeepSeek HTTP API model ids (without the `deepseek:` provider prefix).
pub const DEEPSEEK_API_MODELS: &[&str] = &["deepseek-v4-pro", "deepseek-v4-flash"];

/// Canonical Claude model aliases the `/model` picker always offers, without
/// the `claude:` prefix. Independent of `claude --help` parsing so the picker
/// stays populated even when the CLI is absent or its help text drifts; CLI
/// discovery ([`crate::acp::session::discover_model_options`]) is merged on
/// top to surface full model names. Mirrors the documented `/model` aliases;
/// the `[1m]` long-context variants are valid specs (context-window sizing
/// strips the suffix before matching).
pub const CLAUDE_MODEL_ALIASES: &[&str] =
    &["fable", "sonnet", "opus", "haiku", "opusplan", "sonnet[1m]", "opus[1m]"];

pub fn build_enriched_prompt(
    prompt: &str,
    conversation_history: &[(String, String)],
    file_refs: &[(String, String)],
) -> String {
    // `prompt` at TOP keeps user question inside Claude Read 2000-line window
    // when blob is spilled to .gaviero/tmp/prompt-*.md on bootstrap turns.
    // Section boundaries use XML tags so the agent can distinguish injected
    // context from the user's actual request; the tag is the marker, the body
    // keeps the caveman U:/A:/@path scaffolding to stay token-cheap.
    let mut parts = Vec::new();
    parts.push(prompt.to_string());

    if !conversation_history.is_empty() {
        let mut body = String::new();
        for (role, content) in conversation_history {
            let truncated: String = content.chars().take(HISTORY_TRUNCATION_CHARS).collect();
            let ellipsis = if content.chars().count() > HISTORY_TRUNCATION_CHARS {
                "..."
            } else {
                ""
            };
            let sigil = role_sigil(role);
            body.push_str(&format!("{}: {}{}\n", sigil, truncated, ellipsis));
        }
        parts.push(format!("<prev_conv>\n{}</prev_conv>", body));
    }

    if !file_refs.is_empty() {
        let mut body = String::new();
        for (path, content) in file_refs {
            body.push_str(&format!("@{}\n{}\n/@{}\n", path, content, path));
        }
        parts.push(format!("<file_refs>\n{}</file_refs>", body));
    }

    parts.join("\n\n")
}

/// Caveman role sigil for transcript turns. `user` → `U`, `assistant` → `A`,
/// `system` → `S`. Falls back to the first uppercase letter for unknown roles
/// so future role names degrade gracefully.
fn role_sigil(role: &str) -> String {
    match role {
        "user" => "U".to_string(),
        "assistant" => "A".to_string(),
        "system" => "S".to_string(),
        other => other
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string()),
    }
}

pub fn default_editor_system_prompt(capabilities: &Capabilities) -> String {
    let tool_clause = if capabilities.tool_use {
        "Use the available tools to inspect the workspace before making changes when more context is needed."
    } else {
        "You do not have direct repo tools in this session, so rely on the provided prompt context and referenced files."
    };

    // The in-band file-block channel exists only for backends whose native
    // stream cannot carry tool calls (Codex, Ollama). Backends that emit
    // native tool-use events (Claude) edit files via Write/Edit/MultiEdit
    // and must NOT be instructed about the in-band marker — instructing
    // them causes the model to quote the marker back in prose, which the
    // parser cannot reliably distinguish from a real proposal.
    let file_clause = if capabilities.supports_file_blocks {
        "All code edits must be proposed as complete <file path=\"relative/path\">...</file> \
         blocks so the editor can review them before applying. Do not edit files directly, \
         and do not emit partial file fragments; include the complete final content for each \
         edited file.\n\n"
    } else if capabilities.tool_use {
        "When you need to change files, use the Write, Edit, or MultiEdit tools. \
         Do not paste file contents inline as a substitute for a tool call — only the \
         tool-call channel is reviewed by the editor.\n\n"
    } else {
        ""
    };

    // PUSH→PULL Phase 1 retrieval ("pull") stanza. Emitted only when the
    // read-only graph/memory tools are actually wired, so it never names a
    // tool the session cannot call. Session-stable (no per-turn data) so it
    // stays inside the prompt cache boundary alongside the annotations block.
    let retrieval_clause = retrieval_protocol_clause(&capabilities.retrieval);

    format!(
        "You are a coding assistant working inside the gaviero editor.\n\n{}\n{}{retrieval_clause}{ann}",
        tool_clause,
        file_clause,
        ann = TURN_ANNOTATIONS_CONVENTION,
    )
}

/// The retrieval-protocol ("pull") stanza for the system prompt.
///
/// Returns `""` when no retrieval tools are live (so the prompt is byte-for-byte
/// unchanged for backends that haven't opted in). The symbol-tool sentence is
/// appended only when `symbols` is live, so the stanza never points the model at
/// `symbol_search`/`symbol_doc` when the enrichment sidecar is absent.
fn retrieval_protocol_clause(retrieval: &RetrievalToolset) -> String {
    if !retrieval.graph_and_memory {
        return String::new();
    }
    let mut s = String::from(
        "You have read-only repository tools. The <repo_outline> you were given is a thin \
         index — file paths with top symbol names, not full code. Before answering questions \
         that need a definition or body, read it with node_doc(path); use blast_radius(path) \
         for callers, affected files, and missing tests; use memory_search for prior decisions. \
         Do not ask the user to paste code you can retrieve.",
    );
    if retrieval.symbols {
        s.push_str(
            " For a symbol whose file you don't know yet, search by name with \
             symbol_search(query) and expand it with symbol_doc(qualified_name).",
        );
    }
    s.push_str("\n\n");
    s
}

/// Teaches the LLM the `<turn_annotations>` sidecar convention.
///
/// **Cache discipline (Anthropic prompt caching, plan §A1 risks):** this
/// block is deliberately placed at the end of the system prompt so it
/// lives in the cached segment. It doesn't depend on per-turn context;
/// the cache boundary is correct today because every concatenation in
/// `default_editor_system_prompt` is stable across turns for a given
/// model.
pub const TURN_ANNOTATIONS_CONVENTION: &str = r#"MEMORY SIDECAR — always end your final response with a `<turn_annotations>...</turn_annotations>` JSON block. The editor strips this block before showing your reply to the user, so it is never visible; its sole purpose is to flag durable project facts for future retrieval.

Required shape:

<turn_annotations>
{
  "v": 1,
  "flags": [
    { "type": "decision", "importance": 0.8, "scope": "repo",
      "text": "…≤280 chars…", "refs": ["src/foo.rs:L42"] }
  ],
  "session_thread": "one-line summary of what the current turn is about",
  "open_questions": ["questions you did not resolve this turn"]
}
</turn_annotations>

Rules:
- `type` ∈ { decision, lesson, error, convention, preference, gotcha, invariant }
- `scope` ∈ { run, module, repo, workspace, global }
- `importance` ∈ [0.0, 1.0]; emit only ≥ 0.3. 0.9+ = architectural; 0.6–0.9 = module-level; 0.3–0.6 = local.
- 0–5 flags per turn. `{"flags": []}` is valid; **do not skip the block**.
- Do NOT flag: generic programming knowledge, restatements of the user's request, tentative plans, assistant intent. Only outcomes.
- Emit valid JSON — no code fences around the block, no trailing commentary."#;

pub fn backend_config_for_model(model_spec: &str, ollama_base_url: Option<&str>) -> BackendConfig {
    let trimmed = model_spec.trim();

    if let Some(model) = trimmed
        .strip_prefix("ollama:")
        .or_else(|| trimmed.strip_prefix("local:"))
    {
        return BackendConfig::Ollama {
            model: model.trim().to_string(),
            base_url: Some(
                ollama_base_url
                    .unwrap_or(DEFAULT_OLLAMA_BASE_URL)
                    .to_string(),
            ),
        };
    }

    if let Some(model) = trimmed.strip_prefix("codex:") {
        let m = model.trim();
        return BackendConfig::Codex {
            model: if m.is_empty() {
                None
            } else {
                Some(m.to_string())
            },
        };
    }

    if let Some(model) = trimmed.strip_prefix("cursor:") {
        let m = model.trim();
        return BackendConfig::Cursor {
            model: if m.is_empty() {
                None
            } else {
                Some(m.to_string())
            },
        };
    }

    if let Some(model) = trimmed.strip_prefix("deepseek:") {
        return BackendConfig::Deepseek {
            model: model.trim().to_string(),
        };
    }

    let claude_model = trimmed.strip_prefix("claude:").unwrap_or(trimmed);

    BackendConfig::ClaudeCode {
        model: if claude_model.is_empty() {
            None
        } else {
            Some(claude_model.to_string())
        },
    }
}

pub fn validate_model_spec(model_spec: &str) -> Result<()> {
    let trimmed = model_spec.trim();
    if trimmed.is_empty() {
        anyhow::bail!("model spec cannot be empty");
    }

    let Some((prefix, remainder)) = trimmed.split_once(':') else {
        anyhow::bail!(
            "model spec '{}' is missing a provider prefix; \
             use the canonical `provider:model` form \
             (e.g. `claude:opus`, `codex:gpt-5`, `ollama:qwen2.5-coder:7b`). \
             Supported prefixes: {}",
            trimmed,
            SUPPORTED_PROVIDER_PREFIXES.join(", ")
        );
    };

    // Reject doubly-prefixed specs like `claude:cursor:composer-2.5`: the model
    // name must not itself begin with a known provider prefix. (Ollama specs
    // such as `ollama:qwen2.5-coder:7b` are fine — `qwen2.5-coder` is not a
    // provider.)
    if model_id_has_nested_provider(remainder.trim()) {
        anyhow::bail!(
            "model spec '{}' has a nested provider prefix; \
             use a single `provider:model` pair \
             (e.g. `claude:opus`, not `claude:cursor:composer-2.5`)",
            trimmed
        );
    }

    match prefix {
        "ollama" | "local" | "claude" | "codex" | "cursor" => {
            if remainder.trim().is_empty() {
                anyhow::bail!("model spec '{}' is missing a model name", trimmed);
            }
        }
        "deepseek" => {
            let model = remainder.trim();
            if model.is_empty() {
                anyhow::bail!("model spec '{}' is missing a model name", trimmed);
            }
            if !DEEPSEEK_API_MODELS.contains(&model) {
                anyhow::bail!(
                    "unsupported DeepSeek model '{}'; supported models: {}",
                    model,
                    DEEPSEEK_API_MODELS.join(", ")
                );
            }
        }
        _ => {
            anyhow::bail!(
                "unknown model prefix '{}'; supported prefixes: {}",
                prefix,
                SUPPORTED_PROVIDER_PREFIXES.join(", ")
            );
        }
    }

    Ok(())
}

/// True when a model id (the part after `provider:`) itself begins with a
/// known `provider:` prefix — e.g. `cursor:composer-2.5`. Used to keep
/// doubly-prefixed specs such as `claude:cursor:composer-2.5` out of the
/// completion candidates and out of `validate_model_spec`.
fn model_id_has_nested_provider(model: &str) -> bool {
    model
        .split_once(':')
        .is_some_and(|(head, _)| SUPPORTED_PROVIDER_PREFIXES.contains(&head.trim()))
}

/// Tab-completion candidates for `/model <spec>`.
///
/// Two shapes, matching the canonical `provider:model` schema:
/// * `partial` contains `:` → complete model names for the typed provider.
///   Candidates are the provider's static ids (Claude / DeepSeek) merged with
///   any matching entries from `discovered` (CLI-discovered Claude / Cursor /
///   Ollama specs), filtered by the typed model fragment.
/// * `partial` has no `:` yet → offer provider prefixes only. Selecting one
///   re-triggers completion in the model-name branch. Listing prefixes (not
///   full discovered specs) keeps the picker aligned with the schema and
///   avoids burying providers below the truncation limit.
///
/// Candidates whose model id is itself a `provider:` prefix are dropped, so a
/// doubly-prefixed spec can never be suggested.
pub fn model_spec_completions(partial: &str, discovered: &[String]) -> Vec<String> {
    let partial = partial.trim();
    let partial_lower = partial.to_lowercase();

    // ── `provider:model` — complete model names for the typed provider ──
    if let Some((provider, model_part)) = partial.split_once(':') {
        let provider_lower = provider.to_lowercase();
        let model_part_lower = model_part.to_lowercase();

        // Static ids always offered for this provider, regardless of CLI
        // discovery (which can be empty or drift across CLI versions).
        let mut model_ids: Vec<String> = match provider_lower.as_str() {
            "claude" => CLAUDE_MODEL_ALIASES.iter().map(|s| s.to_string()).collect(),
            "deepseek" => DEEPSEEK_API_MODELS.iter().map(|s| s.to_string()).collect(),
            _ => Vec::new(),
        };

        // Merge model ids surfaced by CLI discovery for the same provider.
        let disc_prefix = format!("{provider_lower}:");
        for spec in discovered {
            if spec.to_lowercase().starts_with(&disc_prefix) {
                let model = &spec[disc_prefix.len()..];
                if !model.is_empty() {
                    model_ids.push(model.to_string());
                }
            }
        }

        let mut candidates: Vec<String> = model_ids
            .into_iter()
            .filter(|m| !model_id_has_nested_provider(m))
            .filter(|m| model_part.is_empty() || m.to_lowercase().starts_with(&model_part_lower))
            .map(|m| format!("{provider_lower}:{m}"))
            .collect();

        candidates.sort();
        candidates.dedup();
        candidates.truncate(10);
        return candidates;
    }

    // ── No `:` yet — offer provider prefixes only ──
    let mut out: Vec<String> = SUPPORTED_PROVIDER_PREFIXES
        .iter()
        .filter(|p| partial.is_empty() || p.starts_with(&partial_lower))
        .map(|p| format!("{p}:"))
        .collect();

    out.sort();
    out.dedup();
    out.truncate(10);
    out
}

pub fn create_backend_for_model(
    model_spec: &str,
    ollama_base_url: Option<&str>,
) -> Result<Box<dyn AgentBackend>> {
    validate_model_spec(model_spec)?;
    let config = backend_config_for_model(model_spec, ollama_base_url);
    create_backend(&config)
}

pub fn is_ollama_model(model_spec: &str) -> bool {
    model_spec.trim().starts_with("ollama:") || model_spec.trim().starts_with("local:")
}

pub fn is_codex_model(model_spec: &str) -> bool {
    let t = model_spec.trim();
    t.starts_with("codex:")
}

pub fn is_cursor_model(model_spec: &str) -> bool {
    model_spec.trim().starts_with("cursor:")
}

pub fn is_deepseek_model(model_spec: &str) -> bool {
    model_spec.trim().starts_with("deepseek:")
}

/// Render planner selections back into the legacy single-string prompt swarm
/// backends consume today.
///
/// **Byte-identical guarantee** (M1, preserved through M3) — the output of
/// this function for the selections produced by
/// [`crate::context_planner::ContextPlanner::plan`] must equal the output
/// of the legacy `runner::build_prompt` for the same inputs.
///
/// M3 distinguishes two selection shapes:
/// * **Structured** (`path.is_some()` / `id.is_some()`): one selection per
///   ranked file or memory hit; renderer combines them into the legacy
///   `## Repository context:\n  ...` and `[Memory context]:\n- ...` blocks.
/// * **Pre-rendered** (`path.is_none()` / `id.is_none()`): a single
///   selection whose `content` already contains the formatted block
///   (M1/M2 carrier from chat path). Renderer emits as-is.
///
/// Order matches `runner::build_prompt`: graph block, memory block, scope
/// clause, task text. Joined with `"\n\n"`.
pub fn render_swarm_prompt(
    selections: &PlannerSelections,
    scope: &FileScope,
    task_text: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(block) = render_graph_block(&selections.graph_selections) {
        parts.push(block);
    }
    if let Some(block) = render_memory_block(&selections.memory_selections) {
        parts.push(block);
    }
    if let Some(block) = render_skill_block(&selections.skill_selections) {
        parts.push(block);
    }

    let scope_clause = scope.to_prompt_clause();
    if !scope_clause.is_empty() {
        parts.push(format!("<file_scope>\n{}</file_scope>", scope_clause));
    }

    parts.push(format!("<user_message>\n{}\n</user_message>", task_text));

    parts.join("\n\n")
}

/// Format graph selections into the legacy `## Repository context:` block.
///
/// Public so the chat adapter (`gaviero_tui::app::session`) can reuse it
/// instead of duplicating the structured-vs-pre-rendered logic.
pub fn render_graph_block(
    graph_selections: &[crate::context_planner::GraphSelection],
) -> Option<String> {
    use crate::context_planner::GraphSelectionKind;

    let topology: Vec<&crate::context_planner::GraphSelection> = graph_selections
        .iter()
        .filter(|g| g.kind == GraphSelectionKind::Topology)
        .collect();
    let structured: Vec<&crate::context_planner::GraphSelection> = graph_selections
        .iter()
        .filter(|g| g.path.is_some())
        .collect();
    let pre_rendered: Vec<&crate::context_planner::GraphSelection> = graph_selections
        .iter()
        .filter(|g| {
            g.path.is_none()
                && g.kind != GraphSelectionKind::Topology
        })
        .collect();

    let mut chunks: Vec<String> = Vec::new();

    for g in topology {
        if !g.content.is_empty() {
            chunks.push(format!(
                "<repo_topology>\n{}\n</repo_topology>",
                g.content
            ));
        }
    }

    if !structured.is_empty() {
        let lines: Vec<String> = structured
            .iter()
            .filter(|g| !g.content.is_empty())
            .map(|g| g.content.clone())
            .collect();
        if !lines.is_empty() {
            chunks.push(format!("<repo_outline>\n{}\n</repo_outline>", lines.join("\n")));
        }
    }
    for g in pre_rendered {
        if !g.content.is_empty() {
            chunks.push(g.content.clone());
        }
    }

    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n\n"))
    }
}

/// Format memory selections into the legacy `[Memory context]:` block.
pub fn render_memory_block(
    memory_selections: &[crate::context_planner::MemorySelection],
) -> Option<String> {
    let structured: Vec<&crate::context_planner::MemorySelection> = memory_selections
        .iter()
        .filter(|m| m.id.is_some())
        .collect();
    let pre_rendered: Vec<&crate::context_planner::MemorySelection> = memory_selections
        .iter()
        .filter(|m| m.id.is_none())
        .collect();

    let mut chunks: Vec<String> = Vec::new();

    if !structured.is_empty() {
        let mut body = String::new();
        for m in structured {
            let ns = m.namespace.as_deref().unwrap_or("");
            let score = m.score.unwrap_or(0.0);
            body.push_str(&format!("{}|{}|s{:.2}\n", ns, m.content, score));
        }
        chunks.push(format!("<project_memory>\n{}</project_memory>", body));
    }
    for m in pre_rendered {
        if !m.content.is_empty() {
            chunks.push(m.content.clone());
        }
    }

    if chunks.is_empty() {
        None
    } else {
        Some(chunks.join("\n\n"))
    }
}

/// Format skill selections into `<skill name="…">` blocks.
pub fn render_skill_block(
    skill_selections: &[crate::context_planner::SkillSelection],
) -> Option<String> {
    if skill_selections.is_empty() {
        return None;
    }
    let blocks: Vec<String> = skill_selections
        .iter()
        .map(|s| {
            format!(
                "<skill name=\"{}\">\n{}\n</skill>",
                s.name, s.rendered_body
            )
        })
        .collect();
    Some(blocks.join("\n\n"))
}

pub fn request_prompt(request: &CompletionRequest) -> String {
    build_enriched_prompt(
        &request.prompt,
        &request.conversation_history,
        &request.file_refs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieval_stanza_gated_on_live_tools() {
        // No retrieval tools → no stanza, prompt unchanged.
        let mut caps = Capabilities {
            tool_use: true,
            supports_system_prompt: true,
            ..Capabilities::default()
        };
        let none = default_editor_system_prompt(&caps);
        assert!(!none.contains("read-only repository tools"));
        assert!(!none.contains("node_doc"));

        // graph_and_memory live → stanza names node_doc/blast_radius/memory_search
        // but NOT the symbol tools (enrichment off).
        caps.retrieval = RetrievalToolset {
            graph_and_memory: true,
            symbols: false,
        };
        let graph = default_editor_system_prompt(&caps);
        assert!(graph.contains("node_doc(path)"));
        assert!(graph.contains("blast_radius(path)"));
        assert!(graph.contains("memory_search"));
        assert!(!graph.contains("symbol_search"));

        // symbols live → the symbol-tool sentence is appended.
        caps.retrieval.symbols = true;
        let sym = default_editor_system_prompt(&caps);
        assert!(sym.contains("symbol_search(query)"));
        assert!(sym.contains("symbol_doc(qualified_name)"));

        // The annotations convention still terminates the prompt (cache tail).
        assert!(sym.trim_end().ends_with("no trailing commentary."));
    }

    #[test]
    fn render_skill_block_formats_xml() {
        use crate::context_planner::SkillSelection;
        let block = render_skill_block(&[SkillSelection {
            name: "lint".to_string(),
            scope_level: 2,
            rendered_body: "run clippy".to_string(),
        }])
        .expect("block");
        assert!(block.contains("<skill name=\"lint\">"));
        assert!(block.contains("run clippy"));
    }

    #[test]
    fn render_skill_block_empty_returns_none() {
        assert!(render_skill_block(&[]).is_none());
    }

    #[test]
    fn render_skill_block_byte_identical_across_swarm_and_chat_edges() {
        use crate::context_planner::{PlannerSelections, SkillSelection};
        use crate::types::FileScope;

        let selections = PlannerSelections {
            skill_selections: vec![SkillSelection {
                name: "migrate-component".to_string(),
                scope_level: 2,
                rendered_body: "Migrate the SearchBar component from React to Vue."
                    .to_string(),
            }],
            ..PlannerSelections::default()
        };

        let skill_xml = render_skill_block(&selections.skill_selections)
            .expect("skill block");
        let swarm = render_swarm_prompt(&selections, &FileScope::default(), "do it");
        let chat = format!(
            "do it\n\n{}",
            skill_xml
        );

        assert!(swarm.contains(&skill_xml));
        assert!(chat.contains(&skill_xml));
        assert_eq!(
            swarm.find(&skill_xml).map(|p| &swarm[p..p + skill_xml.len()]),
            chat.find(&skill_xml).map(|p| &chat[p..p + skill_xml.len()])
        );
    }

    #[test]
    fn test_build_enriched_prompt_includes_history_and_refs() {
        let prompt = build_enriched_prompt(
            "Implement it",
            &[("user".into(), "first question".into())],
            &[("src/lib.rs".into(), "fn demo() {}".into())],
        );

        // XML tags mark section boundaries; caveman body (U:/A: sigils, @path
        // fences) stays inside so the agent can distinguish injected context
        // from the user's actual request without paying for verbose markers.
        assert!(prompt.contains("<prev_conv>\nU: first question\n</prev_conv>"));
        assert!(prompt.contains("<file_refs>\n@src/lib.rs\nfn demo() {}\n/@src/lib.rs\n</file_refs>"));
        // Prompt at TOP, not appended after context.
        assert!(prompt.starts_with("Implement it"));
    }

    #[test]
    fn test_backend_config_for_model_parses_claude_prefix() {
        let config = backend_config_for_model("claude:sonnet", None);
        assert_eq!(
            config,
            BackendConfig::ClaudeCode {
                model: Some("sonnet".into())
            }
        );
    }

    #[test]
    fn test_backend_config_for_model_parses_ollama_prefix() {
        let config = backend_config_for_model("ollama:qwen2.5-coder:7b", Some("http://ollama"));
        assert_eq!(
            config,
            BackendConfig::Ollama {
                model: "qwen2.5-coder:7b".into(),
                base_url: Some("http://ollama".into())
            }
        );
    }

    #[test]
    fn test_validate_model_spec_accepts_supported_forms() {
        for spec in [
            "claude:sonnet",
            "claude:opus",
            "claude:fable",
            "claude:claude-fable-5",
            "claude:opusplan",
            "claude:sonnet[1m]",
            "ollama:qwen2.5-coder:7b",
            "local:qwen2.5-coder:14b",
            "codex:gpt-5.5",
            "cursor:auto",
            "cursor:gpt-5.2",
            "cursor:claude-4.6-opus-high-thinking",
            "deepseek:deepseek-v4-pro",
            "deepseek:deepseek-v4-flash",
        ] {
            validate_model_spec(spec).unwrap();
        }
    }

    #[test]
    fn test_backend_config_for_model_parses_cursor_prefix() {
        let config = backend_config_for_model("cursor:auto", None);
        assert_eq!(
            config,
            BackendConfig::Cursor {
                model: Some("auto".into())
            }
        );
    }

    #[test]
    fn test_is_cursor_model_recognises_prefix() {
        assert!(is_cursor_model("cursor:auto"));
        assert!(is_cursor_model("cursor:gpt-5.2"));
        assert!(!is_cursor_model("claude:sonnet"));
        assert!(!is_cursor_model("codex:gpt-5"));
        assert!(!is_cursor_model("auto"));
    }

    #[test]
    fn test_backend_config_for_model_parses_deepseek_prefix() {
        let config = backend_config_for_model("deepseek:deepseek-v4-pro", None);
        assert_eq!(
            config,
            BackendConfig::Deepseek {
                model: "deepseek-v4-pro".into()
            }
        );
    }

    #[test]
    fn test_backend_config_for_model_parses_codex_prefix() {
        let config = backend_config_for_model("codex:gpt-5.5", None);
        assert_eq!(
            config,
            BackendConfig::Codex {
                model: Some("gpt-5.5".into())
            }
        );
    }

    #[test]
    fn test_is_codex_model() {
        assert!(is_codex_model("codex:gpt-5"));
        assert!(!is_codex_model("claude:sonnet"));
        assert!(!is_codex_model("ollama:qwen"));
        assert!(!is_codex_model("sonnet"));
    }

    #[test]
    fn test_validate_model_spec_rejects_empty_and_unknown_prefixes() {
        assert!(validate_model_spec("").is_err());
        assert!(validate_model_spec("ollama:").is_err());
        let err = validate_model_spec("openai:gpt-4.1").unwrap_err();
        assert!(err.to_string().contains("unknown model prefix"));
    }

    #[test]
    fn test_validate_model_spec_rejects_bare_names_without_provider_prefix() {
        for spec in ["sonnet", "opus", "haiku", "opusplan", "gpt-5.5", "qwen2.5"] {
            let err = validate_model_spec(spec).unwrap_err();
            assert!(
                err.to_string().contains("provider prefix"),
                "expected `provider prefix` complaint for `{spec}`, got: {err}"
            );
        }
    }

    #[test]
    fn test_validate_model_spec_rejects_unknown_deepseek_models() {
        let err = validate_model_spec("deepseek:deepseek-v4").unwrap_err();
        assert!(err.to_string().contains("unsupported DeepSeek model"));
        validate_model_spec("deepseek:deepseek-v4-flash").unwrap();
    }

    #[test]
    fn test_model_spec_completions_provider_prefix() {
        let hits = model_spec_completions("dee", &[]);
        assert!(hits.iter().any(|h| h == "deepseek:"));
    }

    #[test]
    fn test_model_spec_completions_deepseek_models() {
        let hits = model_spec_completions("deepseek:deep", &[]);
        assert!(hits.contains(&"deepseek:deepseek-v4-pro".to_string()));
        assert!(hits.contains(&"deepseek:deepseek-v4-flash".to_string()));
    }

    #[test]
    fn test_model_spec_completions_claude_models_from_static_aliases() {
        // Regression: typing `claude:` must surface Claude models even with no
        // CLI discovery (empty `discovered`). The static alias list backs this.
        let hits = model_spec_completions("claude:", &[]);
        assert!(hits.contains(&"claude:fable".to_string()), "got {hits:?}");
        assert!(hits.contains(&"claude:sonnet".to_string()), "got {hits:?}");
        assert!(hits.contains(&"claude:opus".to_string()), "got {hits:?}");
    }

    #[test]
    fn test_model_spec_completions_claude_filters_by_fragment() {
        let hits = model_spec_completions("claude:op", &[]);
        assert!(hits.iter().all(|h| h.starts_with("claude:op")), "got {hits:?}");
        assert!(hits.contains(&"claude:opus".to_string()), "got {hits:?}");
        assert!(hits.contains(&"claude:opusplan".to_string()), "got {hits:?}");
    }

    #[test]
    fn test_model_spec_completions_merges_discovered_with_statics() {
        let discovered = vec!["claude:claude-fable-5".to_string()];
        let hits = model_spec_completions("claude:claude", &discovered);
        assert_eq!(hits, vec!["claude:claude-fable-5".to_string()]);
    }

    #[test]
    fn test_model_spec_completions_cursor_from_discovered() {
        // Cursor has no static list — its models come only from discovery.
        let discovered = vec!["cursor:composer-2.5".to_string(), "cursor:auto".to_string()];
        let hits = model_spec_completions("cursor:comp", &discovered);
        assert_eq!(hits, vec!["cursor:composer-2.5".to_string()]);
    }

    #[test]
    fn test_model_spec_completions_no_colon_lists_prefixes_only() {
        // Before a `:` is typed the picker offers provider prefixes, not full
        // discovered specs — keeps it aligned with `provider:model` and avoids
        // burying providers below the truncation limit.
        let discovered = vec!["cursor:composer-2.5".to_string()];
        let hits = model_spec_completions("", &discovered);
        assert!(hits.contains(&"cursor:".to_string()), "got {hits:?}");
        assert!(hits.contains(&"claude:".to_string()), "got {hits:?}");
        assert!(
            !hits.iter().any(|h| h == "cursor:composer-2.5"),
            "no-colon view must not list full discovered specs: {hits:?}"
        );
    }

    #[test]
    fn test_model_spec_completions_never_emits_nested_provider_prefix() {
        // Even if a malformed doubly-prefixed spec leaks into `discovered`, it
        // must never be offered as a completion.
        let discovered = vec!["claude:cursor:composer-2.5".to_string()];
        let hits = model_spec_completions("claude:", &discovered);
        assert!(
            hits.iter().all(|h| !h.contains("cursor:")),
            "nested provider prefix leaked into completions: {hits:?}"
        );
    }

    #[test]
    fn test_validate_model_spec_rejects_nested_provider_prefix() {
        let err = validate_model_spec("claude:cursor:composer-2.5").unwrap_err();
        assert!(
            err.to_string().contains("nested provider prefix"),
            "got: {err}"
        );
        // The colon-bearing Ollama form is NOT a nested prefix and stays valid.
        validate_model_spec("ollama:qwen2.5-coder:7b").unwrap();
        validate_model_spec("local:qwen2.5-coder:14b").unwrap();
    }

    // ── Tagged-prompt format tests ────────────────────────────────
    //
    // The renderer is no longer byte-identical to the legacy
    // `runner::build_prompt` — XML section tags were introduced to give the
    // agent unambiguous boundaries between injected context and the user's
    // request. These tests pin the new format directly.

    use crate::context_planner::{
        GraphSelection, GraphSelectionKind, MemorySelection, PlannerSelections,
    };
    use crate::types::FileScope;

    fn graph_outline_selection(content: &str, tokens: usize) -> GraphSelection {
        GraphSelection {
            path: None,
            kind: GraphSelectionKind::OutlineOnly,
            token_estimate: tokens,
            content: content.to_string(),
            rank_score: None,
            confidence: None,
            symbols: Vec::new(),
            content_digest: None,
        }
    }

    fn memory_selection(content: &str) -> MemorySelection {
        MemorySelection {
            id: None,
            namespace: None,
            scope_label: None,
            score: None,
            trust: None,
            content: content.to_string(),
            source_hash: None,
            updated_at: None,
        }
    }

    #[test]
    fn render_swarm_prompt_wraps_user_message_only_when_no_context() {
        let scope = FileScope::default();
        let task = "do the thing";
        let selections = PlannerSelections::default();

        let rendered = render_swarm_prompt(&selections, &scope, task);
        let expected = "<user_message>\ndo the thing\n</user_message>";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_swarm_prompt_wraps_graph_scope_and_user_message() {
        let scope = FileScope {
            owned_paths: vec!["src/lib.rs".into()],
            ..Default::default()
        };
        let task = "implement foo";
        let outline = "[Repo outline]\nfile1.rs\nfile2.rs";
        let mut selections = PlannerSelections::default();
        selections
            .graph_selections
            .push(graph_outline_selection(outline, 2000));

        let rendered = render_swarm_prompt(&selections, &scope, task);
        // Pre-rendered graph content passes through verbatim (no auto-wrap),
        // while scope and user message are wrapped in their tags.
        let expected = "[Repo outline]\nfile1.rs\nfile2.rs\n\n\
                        <file_scope>\n**Owned paths** (read/write):\n- `src/lib.rs`\n</file_scope>\n\n\
                        <user_message>\nimplement foo\n</user_message>";
        assert_eq!(rendered, expected);
    }

    fn structured_graph_selection(
        path: &str,
        decision: crate::repo_map::GraphDecision,
        line: &str,
        tokens: usize,
    ) -> GraphSelection {
        GraphSelection {
            path: Some(std::path::PathBuf::from(path)),
            kind: match decision {
                crate::repo_map::GraphDecision::PathOnly => GraphSelectionKind::PathOnly,
                crate::repo_map::GraphDecision::SignatureOnly => GraphSelectionKind::SignatureOnly,
                crate::repo_map::GraphDecision::OutlineOnly => GraphSelectionKind::OutlineOnly,
                crate::repo_map::GraphDecision::FullAttach => GraphSelectionKind::FullContent,
            },
            token_estimate: tokens,
            content: line.to_string(),
            rank_score: Some(0.5),
            confidence: Some(crate::repo_map::GraphConfidence::High),
            symbols: Vec::new(),
            content_digest: None,
        }
    }

    fn structured_memory_selection(
        id: i64,
        namespace: &str,
        body: &str,
        score: f32,
    ) -> MemorySelection {
        MemorySelection {
            id: Some(id),
            namespace: Some(namespace.to_string()),
            scope_label: Some(namespace.to_string()),
            score: Some(score),
            trust: None,
            content: body.to_string(),
            source_hash: None,
            updated_at: None,
        }
    }

    #[test]
    fn topology_renders_before_repo_outline() {
        let scope = FileScope::default();
        let mut sel = PlannerSelections::default();
        sel.graph_selections.push(GraphSelection {
            path: None,
            kind: GraphSelectionKind::Topology,
            token_estimate: 100,
            content: "  crates/\n    gaviero-core/".to_string(),
            rank_score: None,
            confidence: None,
            symbols: Vec::new(),
            content_digest: None,
        });
        sel.graph_selections.push(structured_graph_selection(
            "src/lib.rs",
            crate::repo_map::GraphDecision::OutlineOnly,
            "  OWN src/lib.rs",
            50,
        ));

        let rendered = render_swarm_prompt(&sel, &scope, "task");
        let topo_pos = rendered.find("<repo_topology>").unwrap();
        let outline_pos = rendered.find("<repo_outline>").unwrap();
        assert!(topo_pos < outline_pos);
    }

    #[test]
    fn structured_graph_renders_repo_outline_tag() {
        // Structured per-file selections collapse into a single
        // `<repo_outline>` block. Per-row sigils (`OWN`, `(s0.92)`) live in
        // `repo_map::rank_for_agent_structured`; here we pin the tag wrapper
        // + line concatenation.
        let scope = FileScope::default();
        let mut sel = PlannerSelections::default();
        sel.graph_selections.push(structured_graph_selection(
            "src/lib.rs",
            crate::repo_map::GraphDecision::FullAttach,
            "  OWN src/lib.rs",
            500,
        ));
        sel.graph_selections.push(structured_graph_selection(
            "src/util.rs",
            crate::repo_map::GraphDecision::SignatureOnly,
            "  src/util.rs (foo, bar)",
            20,
        ));

        let rendered = render_swarm_prompt(&sel, &scope, "the task");
        let expected = "<repo_outline>\n  OWN src/lib.rs\n  src/util.rs (foo, bar)\n</repo_outline>\n\n\
                        <user_message>\nthe task\n</user_message>";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn structured_memory_renders_project_memory_tag() {
        // Per-entry format inside the tag: `{ns}|{content}|s{score:.2}\n`.
        let scope = FileScope::default();
        let mut sel = PlannerSelections::default();
        sel.memory_selections.push(structured_memory_selection(
            10,
            "workspace",
            "remember to use git2",
            3.05,
        ));
        sel.memory_selections.push(structured_memory_selection(
            11,
            "workspace",
            "tests must hit real db",
            2.42,
        ));

        let rendered = render_swarm_prompt(&sel, &scope, "the task");
        let expected = "<project_memory>\n\
                        workspace|remember to use git2|s3.05\n\
                        workspace|tests must hit real db|s2.42\n\
                        </project_memory>\n\n\
                        <user_message>\nthe task\n</user_message>";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn mixed_structured_full_pipeline_tagged() {
        // End-to-end tagged rendering: graph + memory + scope + user message.
        let scope = FileScope {
            owned_paths: vec!["src/lib.rs".into()],
            ..Default::default()
        };
        let mut sel = PlannerSelections::default();
        sel.graph_selections.push(structured_graph_selection(
            "src/lib.rs",
            crate::repo_map::GraphDecision::FullAttach,
            "  OWN src/lib.rs",
            500,
        ));
        sel.memory_selections.push(structured_memory_selection(
            42,
            "repo",
            "key invariant",
            1.5,
        ));

        let rendered = render_swarm_prompt(&sel, &scope, "do the task");
        let expected = "<repo_outline>\n  OWN src/lib.rs\n</repo_outline>\n\n\
                        <project_memory>\nrepo|key invariant|s1.50\n</project_memory>\n\n\
                        <file_scope>\n**Owned paths** (read/write):\n- `src/lib.rs`\n</file_scope>\n\n\
                        <user_message>\ndo the task\n</user_message>";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn pre_rendered_blocks_pass_through_unchanged() {
        // Pre-rendered graph/memory selections (id/path = None) already
        // carry their own framing — the renderer must not double-wrap them.
        // The chat-injection path is the producer today; this pins the
        // contract so additions to that path don't get auto-wrapped.
        let scope = FileScope {
            owned_paths: vec!["src/lib.rs".into()],
            read_only_paths: vec!["Cargo.toml".into()],
            ..Default::default()
        };
        let outline = "[Repo outline]\nlib.rs";
        let impact = "[Impact analysis] lib.rs touches main.rs";
        let memory = "<project_memory>\n- [repo] lesson: past lesson\n</project_memory>";
        let task = "do task";

        let mut selections = PlannerSelections::default();
        selections
            .graph_selections
            .push(graph_outline_selection(outline, 1000));
        selections
            .graph_selections
            .push(graph_outline_selection(impact, 0));
        selections.memory_selections.push(memory_selection(memory));

        let rendered = render_swarm_prompt(&selections, &scope, task);
        let expected = format!(
            "{outline}\n\n{impact}\n\n{memory}\n\n\
             <file_scope>\n**Owned paths** (read/write):\n- `src/lib.rs`\n**Read-only paths**:\n- `Cargo.toml`\n</file_scope>\n\n\
             <user_message>\n{task}\n</user_message>"
        );
        assert_eq!(rendered, expected);
    }
}
