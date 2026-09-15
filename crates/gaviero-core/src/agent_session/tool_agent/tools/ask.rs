//! `AskUserQuestion` — clarifying multi-choice questions for the in-process
//! loop (provider-parity Phase 4).
//!
//! Claude Code ships this tool natively; the in-process loop (deepseek /
//! ollama) did not, so those providers could only *narrate* a question in prose
//! and hope the user replied. Claude Code's plan mode and every provider that
//! has a real prompt channel could block on an answer; the in-process loop
//! could not. This closes that asymmetry.
//!
//! Two design choices worth stating, because both are "reuse rather than add":
//!
//! 1. **The permission channel is the question channel.** The tool rides
//!    [`AcpObserver::on_permission_request`](crate::observer::AcpObserver::on_permission_request)
//!    — the same oneshot the `Bash` gate uses — rather than introducing an
//!    `on_question` event. A question *is* a request for the user's input, and
//!    the host already parks one such request at a time and exposes it to the
//!    remote protocol. A second channel would need a second overlay, a second
//!    remote frame, and a second "first valid answer wins" race.
//!
//! 2. **The wire shape is Claude's, byte-for-byte.** `questions[]` entries are
//!    `{ question, header, multiSelect, options[{ label, description }] }` — the
//!    exact shape `AskUserQuestionState::from_input` parses in the TUI. The
//!    overlay therefore needed no new renderer, and the answers come back in the
//!    same `updated_input.answers` object the desktop and remote paths already
//!    agree on. What *did* have to change is that the TUI used to key the
//!    overlay on the tool *name*; it now keys on the shape (see
//!    `PendingPermission::new`), which is what makes this tool reach the UI at
//!    all.
//!
//! Deliberately **not** gated on `ToolCtx::auto_approve`: `/autoapprove` grants
//! *permission to run a tool*, not the answer to a question. Auto-allowing this
//! tool would have to invent an answer, so it always prompts.

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome};
use crate::acp::session::ASK_USER_QUESTION_TOOL;

/// Claude's documented ceiling. Enforced (not just advertised) because the
/// overlay auto-grows to fit the questions and a model that ignores the schema
/// would otherwise produce an unusable full-screen form.
const MAX_QUESTIONS: usize = 4;
/// Claude's documented ceiling per question.
const MAX_OPTIONS: usize = 4;

/// The in-process loop's implementation of Claude's `AskUserQuestion`.
pub struct AskQuestionTool;

#[async_trait::async_trait]
impl Tool for AskQuestionTool {
    fn name(&self) -> &str {
        ASK_USER_QUESTION_TOOL
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": ASK_USER_QUESTION_TOOL,
                "description": "Ask the user one or more multiple-choice questions when you need a \
                    decision you cannot make from the repository alone. Use this instead of \
                    guessing, and instead of writing the question as prose — the user gets a \
                    selectable list and the answers come back to you as text.",
                "parameters": {
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
                }
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(e) = validate_questions(&args) {
            return ToolOutcome::error(e);
        }

        let Some(observer) = ctx.observer.as_ref() else {
            return ToolOutcome::error(
                "AskUserQuestion is not configured (no interactive host attached)",
            );
        };

        let description = first_question(&args).unwrap_or("Clarifying question");
        let (tx, rx) = tokio::sync::oneshot::channel();
        observer.on_permission_request(ASK_USER_QUESTION_TOOL, description, &args, tx);

        let decision = match rx.await {
            Ok(decision) => decision,
            // A dropped sender is a deny (`observer.rs` documents this).
            Err(_) => return ToolOutcome::error("the question was cancelled before it was answered"),
        };

        match decision {
            crate::observer::PermissionDecision::Allow { .. } => {
                match answers_from(decision.updated_input()) {
                    Some(text) => ToolOutcome::ok(text),
                    // The host allowed the call but supplied no answer — e.g. an
                    // observer whose `on_permission_request` is the default
                    // auto-allow. Reporting this as success would let the model
                    // believe a question was answered when nothing was asked.
                    None => ToolOutcome::error(
                        "the question was allowed but the host returned no answers; \
                         ask the user in prose instead",
                    ),
                }
            }
            crate::observer::PermissionDecision::Deny { message } => {
                ToolOutcome::error(message.unwrap_or_else(|| "the user declined to answer".to_string()))
            }
        }
    }
}

/// First question's text, for the one-line summary the host shows.
fn first_question(args: &Value) -> Option<&str> {
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
/// for `AskUserQuestion`, answers it, and the tool reports "no answers". Failing
/// before the prompt means the model gets a message it can act on instead.
fn validate_questions(args: &Value) -> Result<(), String> {
    let Some(questions) = args.get("questions").and_then(Value::as_array) else {
        return Err("missing required argument 'questions' (expected a non-empty array)".to_string());
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
            return Err(format!("question {n} is missing a non-empty 'question' string"));
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
fn answers_from(updated_input: Option<&Value>) -> Option<String> {
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
    use crate::agent_session::tool_agent::policy::ToolPolicy;
    use crate::agent_session::tool_agent::snapshot::TurnSnapshot;
    use crate::observer::{AcpObserver, PermissionDecision};
    use crate::types::FileScope;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use tokio::sync::Mutex as TokioMutex;

    /// Answers like a user would: fills `answers` from the first option of each
    /// question, exactly as `answers_map` does in the TUI.
    struct AnsweringObserver {
        allow: bool,
        seen_tool: Mutex<Vec<String>>,
        seen_input: Mutex<Vec<Value>>,
    }

    impl AnsweringObserver {
        fn new(allow: bool) -> Self {
            Self {
                allow,
                seen_tool: Mutex::new(Vec::new()),
                seen_input: Mutex::new(Vec::new()),
            }
        }
    }

    impl AcpObserver for AnsweringObserver {
        fn on_stream_chunk(&self, _text: &str) {}
        fn on_tool_call_started(&self, _tool_name: &str) {}
        fn on_streaming_status(&self, _status: &str) {}
        fn on_message_complete(&self, _role: &str, _content: &str) {}
        fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}

        fn on_permission_request(
            &self,
            tool_name: &str,
            _description: &str,
            input: &Value,
            respond: tokio::sync::oneshot::Sender<PermissionDecision>,
        ) {
            self.seen_tool.lock().unwrap().push(tool_name.to_string());
            self.seen_input.lock().unwrap().push(input.clone());
            let decision = if self.allow {
                let mut updated = input.clone();
                if let Some(obj) = updated.as_object_mut() {
                    let mut answers = serde_json::Map::new();
                    if let Some(qs) = input.get("questions").and_then(Value::as_array) {
                        for q in qs {
                            let text = q.get("question").and_then(Value::as_str).unwrap_or("");
                            let label = q
                                .get("options")
                                .and_then(Value::as_array)
                                .and_then(|o| o.first())
                                .and_then(|o| o.get("label"))
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            answers.insert(
                                text.to_string(),
                                Value::String(label.to_string()),
                            );
                        }
                    }
                    obj.insert("answers".into(), Value::Object(answers));
                }
                PermissionDecision::Allow {
                    updated_input: Some(updated),
                }
            } else {
                PermissionDecision::deny_with_message("user said no")
            };
            let _ = respond.send(decision);
        }
    }

    /// An observer that takes the trait's default auto-allow — i.e. a host with
    /// no question channel at all.
    struct DefaultAllowObserver;

    impl AcpObserver for DefaultAllowObserver {
        fn on_stream_chunk(&self, _text: &str) {}
        fn on_tool_call_started(&self, _tool_name: &str) {}
        fn on_streaming_status(&self, _status: &str) {}
        fn on_message_complete(&self, _role: &str, _content: &str) {}
        fn on_proposal_deferred(&self, _path: &Path, _old: Option<&str>, _new: &str) {}
    }

    fn ctx_with(observer: Option<Arc<dyn AcpObserver>>) -> ToolCtx {
        ToolCtx {
            workspace_root: PathBuf::from("/ws"),
            additional_roots: vec![],
            scope: FileScope::default(),
            snapshot: Some(Arc::new(TokioMutex::new(TurnSnapshot::new()))),
            policy: ToolPolicy::default(),
            auto_approve: false,
            observer,
        }
    }

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

    #[tokio::test]
    async fn asks_through_the_permission_channel_and_returns_the_answers() {
        let observer = Arc::new(AnsweringObserver::new(true));
        let out = AskQuestionTool
            .run(two_questions(), &ctx_with(Some(observer.clone())))
            .await;
        assert!(!out.is_error, "{}", out.content);
        // Both questions are echoed, matched to their answer.
        assert!(out.content.contains("Which database?"), "{}", out.content);
        assert!(out.content.contains("Postgres"), "{}", out.content);
        assert!(out.content.contains("Which targets?"), "{}", out.content);
        assert!(out.content.contains("api"), "{}", out.content);
        // The host saw this tool name and the exact input, unmodified.
        assert_eq!(observer.seen_tool.lock().unwrap().as_slice(), [ASK_USER_QUESTION_TOOL]);
        assert_eq!(
            observer.seen_input.lock().unwrap()[0],
            two_questions(),
            "the tool must hand the host its input verbatim — the overlay parses it"
        );
    }

    #[tokio::test]
    async fn a_denied_question_is_an_error_carrying_the_host_message() {
        let observer = Arc::new(AnsweringObserver::new(false));
        let out = AskQuestionTool
            .run(two_questions(), &ctx_with(Some(observer)))
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("user said no"), "{}", out.content);
    }

    #[tokio::test]
    async fn an_allowed_question_without_answers_is_an_error() {
        // The trait's default `on_permission_request` auto-allows with no
        // `updated_input`. Reporting success here would tell the model a
        // question had been answered when none was asked.
        let out = AskQuestionTool
            .run(two_questions(), &ctx_with(Some(Arc::new(DefaultAllowObserver))))
            .await;
        assert!(out.is_error);
        assert!(out.content.contains("no answers"), "{}", out.content);
    }

    #[tokio::test]
    async fn no_observer_is_an_error() {
        let out = AskQuestionTool.run(two_questions(), &ctx_with(None)).await;
        assert!(out.is_error);
        assert!(out.content.contains("not configured"), "{}", out.content);
    }

    #[tokio::test]
    async fn auto_approve_does_not_answer_the_question() {
        // `/autoapprove` is permission to *run* a tool; it carries no answer.
        // The prompt must still be issued.
        let observer = Arc::new(AnsweringObserver::new(true));
        let mut ctx = ctx_with(Some(observer.clone()));
        ctx.auto_approve = true;
        let out = AskQuestionTool.run(two_questions(), &ctx).await;
        assert!(!out.is_error, "{}", out.content);
        assert_eq!(observer.seen_tool.lock().unwrap().len(), 1);
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
        let q = |n: usize| json!({ "question": "Q?", "options": (0..n).map(opt).collect::<Vec<_>>() });
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
    fn the_schema_uses_the_camel_case_keys_the_overlay_parses() {
        let schema = AskQuestionTool.schema();
        let text = schema.to_string();
        assert!(text.contains("multiSelect"), "{text}");
        assert!(!text.contains("multi_select"), "{text}");
        assert_eq!(schema["function"]["name"], ASK_USER_QUESTION_TOOL);
    }
}
