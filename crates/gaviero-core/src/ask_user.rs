//! The one question contract shared by both ask-user surfaces.
//!
//! Claude Code ships an `AskUserQuestion` tool natively. gaviero has two
//! channels that need to speak its wire shape, and they must not drift:
//!
//! * the in-process loop (`deepseek:` / `ollama:`) exposes it as a tool
//!   ([`crate::agent_session::tool_agent::tools::ask`]);
//! * the MCP server exposes it to `dsh:` as `ask_user_question`
//!   ([`crate::mcp::ask`]).
//!
//! Both park the *same* TUI overlay, because the overlay is keyed on the
//! input shape (`PendingPermission::new` → `AskUserQuestionState::from_input`)
//! rather than on a tool name. So the shape — questions of
//! `{ question, header, multiSelect, options[{ label, description }] }`, and
//! answers returned as an `answers` object keyed by question text — lives
//! here once, together with the ceiling checks that stop a model from
//! producing a form the overlay cannot render.
//!
//! The schema is deliberately camelCase: `multiSelect`, not `multi_select`.
//! Claude's shape is the one the TUI parses, and `mcp::ask` must not
//! normalise it into something else on the way through MCP.

use serde_json::{Value, json};

/// Claude's documented ceiling. Enforced (not just advertised) because the
/// overlay auto-grows to fit the questions and a model that ignores the schema
/// would otherwise produce an unusable full-screen form.
pub const MAX_QUESTIONS: usize = 4;
/// Claude's documented ceiling per question.
pub const MAX_OPTIONS: usize = 4;

/// What the model reads when deciding whether to call the tool. Shared so the
/// in-process schema and the MCP `tools/list` entry cannot say different
/// things about the same capability.
pub const ASK_TOOL_DESCRIPTION: &str = "Ask the user one or more multiple-choice questions when you need a \
     decision you cannot make from the repository alone. Use this instead of \
     guessing, and instead of writing the question as prose — the user gets a \
     selectable list and the answers come back to you as text.";

/// The `parameters` object of the `AskUserQuestion` schema.
///
/// Callers wrap it differently — the in-process loop as
/// `{type:"function", function:{name, description, parameters}}`, MCP as a
/// bare `inputSchema` — so only the parameters object lives here.
pub fn ask_tool_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_QUESTIONS,
                "description": "The questions to ask, at most 4.",
                "items": {
                    "type": "object",
                    "properties": {
                        "question": {
                            "type": "string",
                            "description": "The question to ask, in the user's language."
                        },
                        "header": {
                            "type": "string",
                            "description": "Short label (max ~12 chars) for the question."
                        },
                        "multiSelect": {
                            "type": "boolean",
                            "description": "True when more than one option may be selected."
                        },
                        "options": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": MAX_OPTIONS,
                            "description": "The selectable answers, at most 4.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "label": {
                                        "type": "string",
                                        "description": "Short name for the option."
                                    },
                                    "description": {
                                        "type": "string",
                                        "description": "What choosing this option means."
                                    }
                                },
                                "required": ["label", "description"]
                            }
                        }
                    },
                    "required": ["question", "header", "multiSelect", "options"]
                }
            }
        },
        "required": ["questions"]
    })
}

/// First question's text, for the one-line summary the host shows.
pub fn first_question(args: &Value) -> Option<&str> {
    args.get("questions")?
        .as_array()?
        .first()?
        .get("question")?
        .as_str()
}

/// Reject anything the overlay would not be able to render as a form.
///
/// `AskUserQuestionState::from_input` silently drops questions with no options
/// and returns `None` when every question is dropped. Without this check that
/// turns into a dead end the model cannot diagnose: the user sees a y/n overlay
/// for a question, answers it, and the tool reports "no answers". Failing
/// before the prompt means the model gets a message it can act on instead.
pub fn validate_questions(args: &Value) -> Result<(), String> {
    let Some(questions) = args.get("questions").and_then(Value::as_array) else {
        return Err(
            "missing required argument 'questions' (expected a non-empty array)".to_string(),
        );
    };
    if questions.is_empty() {
        return Err("'questions' must contain at least one question".to_string());
    }
    if questions.len() > MAX_QUESTIONS {
        return Err(format!(
            "'questions' holds {} entries; at most {MAX_QUESTIONS} are supported",
            questions.len()
        ));
    }
    for (i, q) in questions.iter().enumerate() {
        let n = i + 1;
        if q.get("question")
            .and_then(Value::as_str)
            .is_none_or(|s| s.trim().is_empty())
        {
            return Err(format!(
                "question {n} is missing a non-empty 'question' string"
            ));
        }
        let Some(options) = q.get("options").and_then(Value::as_array) else {
            return Err(format!(
                "question {n} is missing an 'options' array (at least 2 are required)"
            ));
        };
        if options.is_empty() {
            return Err(format!("question {n} has an empty 'options' array"));
        }
        if options.len() > MAX_OPTIONS {
            return Err(format!(
                "question {n} holds {} options; at most {MAX_OPTIONS} are supported",
                options.len()
            ));
        }
        for (j, opt) in options.iter().enumerate() {
            if opt
                .get("label")
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err(format!(
                    "question {n} option {} is missing a non-empty 'label' string",
                    j + 1
                ));
            }
        }
    }
    Ok(())
}

/// Render the host's `answers` object as the tool result the model reads back.
///
/// Questions are matched by **text**, not by position: `answers` is a JSON
/// object keyed by question text, and object key order is not guaranteed to
/// survive the round trip (it is alphabetical unless `serde_json` was built with
/// `preserve_order`). Printing each pair keeps the association unambiguous.
///
/// `None` means "the host allowed the call but supplied no answer" — callers
/// must report that as an error, never as a success.
pub fn answers_from(updated_input: Option<&Value>) -> Option<String> {
    let answers = updated_input?.get("answers")?.as_object()?;
    if answers.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(answers.len());
    for (question, answer) in answers {
        let value = match answer {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        lines.push(format!("{question}\n  → {value}"));
    }
    Some(format!("Answers from the user:\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_questions() -> Value {
        json!({
            "questions": [
                {
                    "question": "Which database?",
                    "header": "DB",
                    "multiSelect": false,
                    "options": [
                        { "label": "Postgres", "description": "relational" },
                        { "label": "SQLite", "description": "embedded" }
                    ]
                },
                {
                    "question": "Which targets?",
                    "header": "Targets",
                    "multiSelect": true,
                    "options": [
                        { "label": "api", "description": "the server" },
                        { "label": "cli", "description": "the client" }
                    ]
                }
            ]
        })
    }

    #[test]
    fn validate_rejects_shapes_the_overlay_could_not_render() {
        assert!(validate_questions(&json!({})).is_err());
        assert!(validate_questions(&json!({ "questions": [] })).is_err());
        assert!(validate_questions(&json!({ "questions": "nope" })).is_err());
        // A question with no options is the dead-end case: the overlay would
        // drop it (and, if it were the only question, show a y/n prompt).
        assert!(
            validate_questions(&json!({
                "questions": [{ "question": "Q?", "options": [] }]
            }))
            .is_err()
        );
        assert!(
            validate_questions(&json!({
                "questions": [{ "question": "Q?" }]
            }))
            .is_err()
        );
        assert!(
            validate_questions(&json!({
                "questions": [{
                    "question": "Q?",
                    "options": [{ "description": "no label" }]
                }]
            }))
            .is_err()
        );
        assert!(
            validate_questions(&json!({
                "questions": [{ "question": "  ", "options": [{ "label": "A" }] }]
            }))
            .is_err()
        );
        assert!(validate_questions(&two_questions()).is_ok());
        // `multiSelect` absent is fine — the overlay defaults it to false.
        assert!(
            validate_questions(&json!({
                "questions": [{
                    "question": "Q?",
                    "options": [{ "label": "A" }, { "label": "B" }]
                }]
            }))
            .is_ok()
        );
    }

    #[test]
    fn validate_enforces_the_declared_ceilings() {
        let opt = |i: usize| json!({ "label": format!("o{i}") });
        let q =
            |n: usize| json!({ "question": "Q?", "options": (0..n).map(opt).collect::<Vec<_>>() });
        let many = json!({ "questions": (0..MAX_QUESTIONS + 1).map(|_| q(2)).collect::<Vec<_>>() });
        assert!(validate_questions(&many).is_err());
        let wide = json!({ "questions": [q(MAX_OPTIONS + 1)] });
        assert!(validate_questions(&wide).is_err());
    }

    #[test]
    fn answers_from_tolerates_a_missing_or_empty_answers_object() {
        assert!(answers_from(None).is_none());
        assert!(answers_from(Some(&json!({}))).is_none());
        assert!(answers_from(Some(&json!({ "answers": {} }))).is_none());
        assert!(answers_from(Some(&json!({ "answers": { "Q?": "A" } }))).is_some());
    }

    #[test]
    fn first_question_reads_the_prompt_text_only() {
        assert_eq!(first_question(&two_questions()), Some("Which database?"));
        assert_eq!(first_question(&json!({})), None);
        assert_eq!(first_question(&json!({ "questions": [] })), None);
    }

    #[test]
    fn the_shared_schema_uses_the_camel_case_keys_the_overlay_parses() {
        let text = ask_tool_schema().to_string();
        assert!(text.contains("multiSelect"), "{text}");
        assert!(!text.contains("multi_select"), "{text}");
        assert!(text.contains("\"description\""), "{text}");
    }
}
