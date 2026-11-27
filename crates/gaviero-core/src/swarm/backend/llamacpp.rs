//! `llamacpp:` — a local llama.cpp `llama-server` driven by the in-process
//! tool agent.
//!
//! llama-server speaks the same OpenAI-compatible chat-completions dialect the
//! `deepseek:` client already handles (SSE, `tool_calls`, `reasoning_content`),
//! so the swarm backend is [`super::deepseek::DeepseekBackend`] running the
//! [`ApiDialect::LlamaCpp`](crate::agent_session::tool_agent::config::ApiDialect)
//! dialect. This module holds what is specific to llama.cpp: the context-window
//! default and the effort vocabulary.

/// Context window gaviero assumes for a llama-server: its `-c 65536`, the size
/// the 24 GB local setup runs. The server's real value is `n_ctx` in
/// `GET /props` (or `/v1/models` `meta.n_ctx`); a request past it fails with
/// HTTP 400 `exceed_context_size_error`.
pub const LLAMACPP_DEFAULT_CONTEXT_WINDOW: usize = 65_536;

/// Levels `/effort` and scripts may pick for `llamacpp:`. llama-server copies
/// `reasoning_effort` into the chat template's kwargs and refuses only a level
/// the template lists as rejected — the Qwen3.x templates take
/// `low | medium | high` (`high` is their `xhigh`) and refuse `max` and
/// `minimal` with HTTP 400.
pub const LLAMACPP_EFFORT_LEVELS: &[&str] = &["low", "medium", "high"];

/// Map gaviero's provider-neutral `effort` onto the `reasoning_effort` sent to
/// llama-server. `None` (for `auto`, `off`, unknown) sends no field, so the
/// template's default — or the server's `--chat-template-kwargs` — applies.
/// Levels above `high` clamp to `high`, below `low` to `low`: the same answer
/// [`super::effort::nearest_level`] gives over [`LLAMACPP_EFFORT_LEVELS`].
pub fn reasoning_effort_for_llamacpp(effort: &str) -> Option<&'static str> {
    match effort.trim().to_ascii_lowercase().as_str() {
        "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" | "xhigh" | "max" | "ultra" => Some("high"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_map_agrees_with_nearest_level() {
        let offered: Vec<String> = LLAMACPP_EFFORT_LEVELS.iter().map(|s| s.to_string()).collect();
        for requested in ["minimal", "low", "medium", "high", "xhigh", "max", "ultra"] {
            assert_eq!(
                reasoning_effort_for_llamacpp(requested),
                super::super::effort::nearest_level(requested, &offered),
                "{requested}"
            );
        }
        for unset in ["auto", "off", "", "turbo"] {
            assert_eq!(reasoning_effort_for_llamacpp(unset), None, "{unset:?}");
        }
        assert_eq!(reasoning_effort_for_llamacpp(" MEDIUM "), Some("medium"));
    }
}
