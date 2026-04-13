use anyhow::Result;

use super::{create_backend, AgentBackend, BackendConfig, Capabilities, CompletionRequest};

const HISTORY_TRUNCATION_CHARS: usize = 2000;
const DEFAULT_OLLAMA_BASE_URL: &str = "http://localhost:11434";
const SUPPORTED_PROVIDER_PREFIXES: &[&str] =
    &["claude", "claude-code", "codex", "codex-cli", "ollama", "local"];

pub fn build_enriched_prompt(
    prompt: &str,
    conversation_history: &[(String, String)],
    file_refs: &[(String, String)],
) -> String {
    let mut parts = Vec::new();

    if !conversation_history.is_empty() {
        parts.push("Previous conversation:\n".to_string());
        for (role, content) in conversation_history {
            let truncated: String = content.chars().take(HISTORY_TRUNCATION_CHARS).collect();
            let ellipsis = if content.chars().count() > HISTORY_TRUNCATION_CHARS {
                "..."
            } else {
                ""
            };
            parts.push(format!("[{}]: {}{}\n", role, truncated, ellipsis));
        }
        parts.push("---\n".to_string());
    }

    if !file_refs.is_empty() {
        parts.push("Referenced files:\n".to_string());
        for (path, content) in file_refs {
            parts.push(format!(
                "--- {} ---\n{}\n--- end {} ---\n",
                path, content, path
            ));
        }
    }

    parts.push(prompt.to_string());
    parts.join("\n")
}

pub fn default_editor_system_prompt(capabilities: &Capabilities) -> String {
    let tool_clause = if capabilities.tool_use {
        "Use the available tools to inspect the workspace before making changes when more context is needed."
    } else {
        "You do not have direct repo tools in this session, so rely on the provided prompt context and referenced files."
    };

    format!(
        "You are a coding assistant working inside the gaviero editor.\n\n{}\n\
         When proposing file changes, emit complete <file path=\"relative/path\">...</file> blocks \
         so the editor can review them before applying.",
        tool_clause
    )
}

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
            model: if m.is_empty() { None } else { Some(m.to_string()) },
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

        assert!(prompt.contains("Previous conversation"));
        assert!(prompt.contains("[user]: first question"));
        assert!(prompt.contains("--- src/lib.rs ---"));
        assert!(prompt.contains("Implement it"));
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
            "codex:gpt-5-codex",
            "codex-cli:o4-mini",
        ] {
            validate_model_spec(spec).unwrap();
        }
    }

    #[test]
    fn test_backend_config_for_model_parses_codex_prefix() {
        let config = backend_config_for_model("codex:gpt-5-codex", None);
        assert_eq!(
            config,
            BackendConfig::Codex {
                model: Some("gpt-5-codex".into())
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
}
