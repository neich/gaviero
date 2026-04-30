use anyhow::Result;

use crate::context_planner::PlannerSelections;
use crate::types::FileScope;

use super::{AgentBackend, BackendConfig, Capabilities, CompletionRequest, create_backend};

const HISTORY_TRUNCATION_CHARS: usize = 2000;
const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
const SUPPORTED_PROVIDER_PREFIXES: &[&str] = &[
    "claude",
    "claude-code",
    "codex",
    "codex-cli",
    "ollama",
    "local",
];

pub fn build_enriched_prompt(
    prompt: &str,
    conversation_history: &[(String, String)],
    file_refs: &[(String, String)],
) -> String {
    // `prompt` at TOP keeps user question inside Claude Read 2000-line window
    // when blob is spilled to .gaviero/tmp/prompt-*.md on bootstrap turns.
    // Scaffolding labels use caveman sigils (U:/A:/Files:/@path) per project
    // convention — saves tokens; agent infers structure from indentation.
    let mut parts = Vec::new();
    parts.push(prompt.to_string());

    if !conversation_history.is_empty() {
        parts.push("\nPrevConv:\n".to_string());
        for (role, content) in conversation_history {
            let truncated: String = content.chars().take(HISTORY_TRUNCATION_CHARS).collect();
            let ellipsis = if content.chars().count() > HISTORY_TRUNCATION_CHARS {
                "..."
            } else {
                ""
            };
            let sigil = role_sigil(role);
            parts.push(format!("{}: {}{}\n", sigil, truncated, ellipsis));
        }
    }

    if !file_refs.is_empty() {
        parts.push("\nFiles:\n".to_string());
        for (path, content) in file_refs {
            parts.push(format!("@{}\n{}\n/@{}\n", path, content, path));
        }
    }

    parts.join("\n")
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
        "When proposing file changes, emit complete <file path=\"relative/path\">...</file> \
         blocks so the editor can review them before applying.\n\n"
    } else if capabilities.tool_use {
        "When you need to change files, use the Write, Edit, or MultiEdit tools. \
         Do not paste file contents inline as a substitute for a tool call — only the \
         tool-call channel is reviewed by the editor.\n\n"
    } else {
        ""
    };

    format!(
        "You are a coding assistant working inside the gaviero editor.\n\n{}\n{}{ann}",
        tool_clause,
        file_clause,
        ann = TURN_ANNOTATIONS_CONVENTION,
    )
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

    if let Some(model) = trimmed
        .strip_prefix("codex-cli:")
        .or_else(|| trimmed.strip_prefix("codex:"))
    {
        let m = model.trim();
        return BackendConfig::Codex {
            model: if m.is_empty() {
                None
            } else {
                Some(m.to_string())
            },
        };
    }

    let claude_model = trimmed
        .strip_prefix("claude-code:")
        .or_else(|| trimmed.strip_prefix("claude:"))
        .unwrap_or(trimmed);

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

    if let Some((prefix, remainder)) = trimmed.split_once(':') {
        match prefix {
            "ollama" | "local" | "claude" | "claude-code" | "codex" | "codex-cli" => {
                if remainder.trim().is_empty() {
                    anyhow::bail!("model spec '{}' is missing a model name", trimmed);
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
    }

    Ok(())
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
    t.starts_with("codex:") || t.starts_with("codex-cli:")
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

    let scope_clause = scope.to_prompt_clause();
    if !scope_clause.is_empty() {
        parts.push(format!("[File scope]:\n{}", scope_clause));
    }

    parts.push(task_text.to_string());

    parts.join("\n\n")
}

/// Format graph selections into the legacy `## Repository context:` block.
///
/// Public so the chat adapter (`gaviero_tui::app::session`) can reuse it
/// instead of duplicating the structured-vs-pre-rendered logic.
pub fn render_graph_block(
    graph_selections: &[crate::context_planner::GraphSelection],
) -> Option<String> {
    let structured: Vec<&crate::context_planner::GraphSelection> = graph_selections
        .iter()
        .filter(|g| g.path.is_some())
        .collect();
    let pre_rendered: Vec<&crate::context_planner::GraphSelection> = graph_selections
        .iter()
        .filter(|g| g.path.is_none())
        .collect();

    let mut chunks: Vec<String> = Vec::new();

    if !structured.is_empty() {
        let lines: Vec<String> = structured
            .iter()
            .filter(|g| !g.content.is_empty())
            .map(|g| g.content.clone())
            .collect();
        if !lines.is_empty() {
            chunks.push(format!("Repo:\n{}", lines.join("\n")));
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
        let mut block = String::from("Mem:\n");
        for m in structured {
            let ns = m.namespace.as_deref().unwrap_or("");
            let score = m.score.unwrap_or(0.0);
            block.push_str(&format!("{}|{}|s{:.2}\n", ns, m.content, score));
        }
        chunks.push(block);
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
    fn test_build_enriched_prompt_includes_history_and_refs() {
        let prompt = build_enriched_prompt(
            "Implement it",
            &[("user".into(), "first question".into())],
            &[("src/lib.rs".into(), "fn demo() {}".into())],
        );

        // Caveman scaffolding: PrevConv:/U:/A: sigils, @path/@/path file fences.
        assert!(prompt.contains("PrevConv:"));
        assert!(prompt.contains("U: first question"));
        assert!(prompt.contains("@src/lib.rs\n"));
        assert!(prompt.contains("/@src/lib.rs"));
        // Prompt at TOP, not appended after context.
        assert!(prompt.starts_with("Implement it"));
    }

    #[test]
    fn test_backend_config_for_model_defaults_to_claude() {
        let config = backend_config_for_model("sonnet", None);
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
            "sonnet",
            "opus",
            "claude:sonnet",
            "claude-code:haiku",
            "ollama:qwen2.5-coder:7b",
            "local:qwen2.5-coder:14b",
            "codex:gpt-5.5",
            "codex-cli:o4-mini",
        ] {
            validate_model_spec(spec).unwrap();
        }
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
        assert!(is_codex_model("codex-cli:o4-mini"));
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

    // ── M1 byte-identity adapter tests ────────────────────────────
    //
    // These pin the V9 §11 M1 acceptance gate ("M0 metrics unchanged"):
    // `render_swarm_prompt` consuming planner-produced selections must
    // emit the exact same string the legacy `runner::build_prompt` produced
    // for matching inputs.

    use crate::context_planner::{
        GraphSelection, GraphSelectionKind, MemorySelection, PlannerSelections,
    };
    use crate::types::FileScope;

    fn legacy_build_prompt_equivalent(
        graph_outline: Option<&str>,
        impact_text: Option<&str>,
        memory_ctx: Option<&str>,
        scope: &FileScope,
        task_text: &str,
    ) -> String {
        // Mirror runner.rs:340-389 build_prompt parts assembly exactly.
        let mut parts: Vec<String> = Vec::new();
        if let Some(o) = graph_outline {
            if !o.is_empty() {
                parts.push(o.to_string());
            }
        }
        if let Some(t) = impact_text {
            parts.push(t.to_string());
        }
        if let Some(m) = memory_ctx {
            if !m.is_empty() {
                parts.push(m.to_string());
            }
        }
        let scope_clause = scope.to_prompt_clause();
        if !scope_clause.is_empty() {
            parts.push(format!("[File scope]:\n{}", scope_clause));
        }
        parts.push(task_text.to_string());
        parts.join("\n\n")
    }

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
    fn render_swarm_prompt_byte_matches_legacy_minimal() {
        let scope = FileScope::default();
        let task = "do the thing";
        let selections = PlannerSelections::default();

        let rendered = render_swarm_prompt(&selections, &scope, task);
        let legacy = legacy_build_prompt_equivalent(None, None, None, &scope, task);
        assert_eq!(rendered, legacy);
    }

    #[test]
    fn render_swarm_prompt_byte_matches_legacy_with_graph_and_scope() {
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
        let legacy = legacy_build_prompt_equivalent(Some(outline), None, None, &scope, task);
        assert_eq!(rendered, legacy);
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
    fn m3_structured_graph_renders_caveman_block() {
        // Structured per-file selections collapse into the caveman `Repo:` block.
        // Per-row sigils (`OWN`, `(s0.92)`) live in `repo_map::rank_for_agent_structured`;
        // here we pin the block header + line concatenation.
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
        let expected = "Repo:\n  OWN src/lib.rs\n  src/util.rs (foo, bar)\n\nthe task";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn m3_structured_memory_renders_caveman_block() {
        // Caveman per-entry format: `{ns}|{content}|s{score:.2}\n`.
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
        // Memory block ends with `\n` after the last entry; `parts.join("\n\n")`
        // therefore produces three consecutive newlines before the task.
        let expected = "Mem:\n\
                        workspace|remember to use git2|s3.05\n\
                        workspace|tests must hit real db|s2.42\n\
                        \n\nthe task";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn m3_mixed_structured_full_pipeline_caveman() {
        // End-to-end caveman rendering: structured graph + structured memory +
        // scope clause + task.
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
        let expected = "Repo:\n  OWN src/lib.rs\n\n\
                        Mem:\nrepo|key invariant|s1.50\n\
                        \n\n[File scope]:\n**Owned paths** (read/write):\n- `src/lib.rs`\n\
                        \n\ndo the task";
        assert_eq!(rendered, expected);
    }

    #[test]
    fn render_swarm_prompt_byte_matches_legacy_full_inputs() {
        // Pins the full insertion order: graph_outline → impact → memory → scope → task.
        // Exact same order runner::build_prompt uses.
        let scope = FileScope {
            owned_paths: vec!["src/lib.rs".into()],
            read_only_paths: vec!["Cargo.toml".into()],
            ..Default::default()
        };
        let outline = "[Repo outline]\nlib.rs";
        let impact = "[Impact analysis] lib.rs touches main.rs";
        let memory = "[Memory context]:\n- past lesson";
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
        let legacy =
            legacy_build_prompt_equivalent(Some(outline), Some(impact), Some(memory), &scope, task);
        assert_eq!(
            rendered, legacy,
            "swarm adapter must be byte-identical to legacy build_prompt"
        );
    }
}
