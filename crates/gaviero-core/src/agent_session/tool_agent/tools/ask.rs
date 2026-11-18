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
//! 2. **The wire shape is Claude's, byte-for-byte** — and it is shared with the
//!    MCP surface for `dsh:` ([`crate::mcp::ask`]) through
//!    [`crate::ask_user`], so the two channels cannot drift apart.
//!    `questions[]` entries are `{ question, header, multiSelect,
//!    options[{ label, description }] }` — the exact shape
//!    `AskUserQuestionState::from_input` parses in the TUI. The overlay
//!    therefore needed no new renderer, and the answers come back in the same
//!    `updated_input.answers` object the desktop and remote paths already agree
//!    on. What *did* have to change is that the TUI used to key the overlay on
//!    the tool *name*; it now keys on the shape (see `PendingPermission::new`),
//!    which is what makes this tool reach the UI at all.
//!
//! Deliberately **not** gated on `ToolCtx::auto_approve`: `/autoapprove` grants
//! *permission to run a tool*, not the answer to a question. Auto-allowing this
//! tool would have to invent an answer, so it always prompts.

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome};
use crate::acp::session::ASK_USER_QUESTION_TOOL;
use crate::ask_user::{self, ASK_TOOL_DESCRIPTION};

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
                "description": ASK_TOOL_DESCRIPTION,
                "parameters": ask_user::ask_tool_schema(),
            }
        })
    }

    async fn run(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        if let Err(e) = ask_user::validate_questions(&args) {
            return ToolOutcome::error(e);
        }

        let Some(observer) = ctx.observer.as_ref() else {
            return ToolOutcome::error(
                "AskUserQuestion is not configured (no interactive host attached)",
            );
        };

        let description = ask_user::first_question(&args).unwrap_or("Clarifying question");
        let (tx, rx) = tokio::sync::oneshot::channel();
        observer.on_permission_request(ASK_USER_QUESTION_TOOL, description, &args, tx);

        let decision = match rx.await {
            Ok(decision) => decision,
            // A dropped sender is a deny (`observer.rs` documents this).
            Err(_) => {
                return ToolOutcome::error("the question was cancelled before it was answered");
            }
        };

        match decision {
            crate::observer::PermissionDecision::Allow { .. } => {
                match ask_user::answers_from(decision.updated_input()) {
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
            crate::observer::PermissionDecision::Deny { message } => ToolOutcome::error(
                message.unwrap_or_else(|| "the user declined to answer".to_string()),
            ),
        }
    }
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
                            answers.insert(text.to_string(), Value::String(label.to_string()));
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
            sensitive: crate::scope_enforcer::SensitivePolicy::default(),
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
        assert_eq!(
            observer.seen_tool.lock().unwrap().as_slice(),
            [ASK_USER_QUESTION_TOOL]
        );
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
            .run(
                two_questions(),
                &ctx_with(Some(Arc::new(DefaultAllowObserver))),
            )
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
    fn the_schema_uses_the_camel_case_keys_the_overlay_parses() {
        let schema = AskQuestionTool.schema();
        let text = schema.to_string();
        assert!(text.contains("multiSelect"), "{text}");
        assert!(!text.contains("multi_select"), "{text}");
        assert_eq!(schema["function"]["name"], ASK_USER_QUESTION_TOOL);
        // The parameters are the shared ones, not a private copy.
        assert_eq!(schema["function"]["parameters"], ask_user::ask_tool_schema());
    }
}
