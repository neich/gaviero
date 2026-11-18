//! The MCP side of the ask-user channel: one interactive tool, `ask_user_question`.
//!
//! `dsh:` speaks ACP, and dsh's ACP profile deliberately does not surface any
//! user-question presentation (its own package contract lists elicitation as
//! unsupported; the `tool-ask-user` tool is mounted only in the SDK app). So a
//! structured question cannot cross the ACP wire. It *can* cross the MCP one,
//! which dsh already mounts: dsh's `session/new` hands gaviero's loopback HTTP
//! endpoint to the child as an `mcpServers[]` entry, and dsh calls tools over
//! it like any other MCP client.
//!
//! The tool parks the *same* overlay the TUI already renders for Claude and for
//! the in-process loop. That works because the overlay is keyed on the input
//! shape ([`crate::ask_user`]), not on a tool name, so this module contributes
//! no UI: it translates a transport-neutral request into the existing
//! [`AcpObserver::on_permission_request`](crate::observer::AcpObserver::on_permission_request)
//! oneshot and turns the answer back into a tool result.
//!
//! ## Why a header decides whether the tool exists
//!
//! A question needs somewhere to be shown. The server is one
//! `GavieroMcpServer` shared by every connection (shim, loopback HTTP, Codex,
//! Claude, …), so "does this connection have a chat conversation behind it" is
//! not a property of the server. rmcp 1.5 inserts the `http::request::Parts` of
//! every streamable-HTTP request into `RequestContext.extensions`
//! (`transport/streamable_http_server/tower.rs`), so a per-request header —
//! [`CONVERSATION_HEADER`], set per `dsh:` session by
//! [`crate::agent_session::agent_client_protocol::dsh`] — identifies the
//! conversation. Absent (stdio, named pipe, any other HTTP client), the tool is
//! simply not listed, which is also what keeps its ~150–250 prompt tokens off
//! every other provider's turn.

use rmcp::model::Tool;

use crate::observer::PermissionDecision;

#[cfg(test)]
use std::sync::Arc;

/// The MCP tool name. Namespaced by clients as `mcp__gaviero__ask_user_question`.
pub const TOOL_ASK_USER_QUESTION: &str = "ask_user_question";

/// Per-session request header carrying the chat conversation that owns the
/// question. Lower-case because HTTP header names are case-insensitive and the
/// `http` crate normalises to this form.
pub const CONVERSATION_HEADER: &str = "x-gaviero-conversation";

/// Where a parked question goes.
///
/// The server holds this as `Arc<dyn UserQuestionSink>` and deliberately knows
/// nothing else about the host: in the TUI it forwards to the event loop, in
/// tests it is a fake. `respond` is the same oneshot
/// [`crate::observer::AcpObserver::on_permission_request`] takes, so the answer
/// path is byte-identical to Claude's and the in-process loop's — including
/// `updated_input["answers"]`.
///
/// A dropped `respond` sender means "the question was cancelled", exactly as
/// the observer contract documents.
pub trait UserQuestionSink: Send + Sync {
    fn ask(
        &self,
        conv_id: &str,
        input: &serde_json::Value,
        respond: tokio::sync::oneshot::Sender<PermissionDecision>,
    );
}

/// The conversation id a request is tagged with, if any.
///
/// Returns `None` for a missing header, a non-ASCII value, or one that is empty
/// or only whitespace — all of which mean "this connection has no conversation
/// to ask".
pub fn conversation_from(parts: &http::request::Parts) -> Option<&str> {
    parts
        .headers
        .get(CONVERSATION_HEADER)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// The `tools/list` entry for `ask_user_question`.
///
/// Built straight from [`crate::ask_user`] rather than through
/// `listed_tools()`'s `strip_schema_descriptions`: the per-option `description`
/// strings are *data the model must fill in*, not prose about the schema, and
/// dropping them would leave a question whose choices cannot be described.
pub fn tool_definition() -> Tool {
    let serde_json::Value::Object(schema) = crate::ask_user::ask_tool_schema() else {
        unreachable!("ask_tool_schema always returns an object");
    };
    Tool::new(
        TOOL_ASK_USER_QUESTION,
        crate::ask_user::ASK_TOOL_DESCRIPTION,
        schema,
    )
}

/// How a [`FakeQuestionSink`] answers. Test-only.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum FakeQuestionReply {
    /// Answer every question with its first option — what `answers_map` does
    /// in the TUI.
    FirstOption,
    /// Refuse to answer.
    Deny(&'static str),
    /// Allow the call but supply no `answers` — a host with no question UI.
    NoAnswers,
}

/// A sink that records what it was asked and answers per [`FakeQuestionReply`].
#[cfg(test)]
pub(crate) struct FakeQuestionSink {
    reply: FakeQuestionReply,
    seen: std::sync::Mutex<Vec<(String, serde_json::Value)>>,
}

#[cfg(test)]
impl FakeQuestionSink {
    pub(crate) fn new(reply: FakeQuestionReply) -> Arc<Self> {
        Arc::new(Self {
            reply,
            seen: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// `(conversation id, input)` for every question this sink was handed.
    pub(crate) fn seen(&self) -> Vec<(String, serde_json::Value)> {
        self.seen.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl UserQuestionSink for FakeQuestionSink {
    fn ask(
        &self,
        conv_id: &str,
        input: &serde_json::Value,
        respond: tokio::sync::oneshot::Sender<PermissionDecision>,
    ) {
        self.seen
            .lock()
            .unwrap()
            .push((conv_id.to_string(), input.clone()));
        let decision = match self.reply {
            FakeQuestionReply::FirstOption => {
                let mut updated = input.clone();
                let mut answers = serde_json::Map::new();
                for question in input
                    .get("questions")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let text = question
                        .get("question")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    let label = question
                        .get("options")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|options| options.first())
                        .and_then(|option| option.get("label"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    answers.insert(
                        text.to_string(),
                        serde_json::Value::String(label.to_string()),
                    );
                }
                if let Some(object) = updated.as_object_mut() {
                    object.insert("answers".into(), serde_json::Value::Object(answers));
                }
                PermissionDecision::Allow {
                    updated_input: Some(updated),
                }
            }
            FakeQuestionReply::Deny(message) => PermissionDecision::deny_with_message(message),
            FakeQuestionReply::NoAnswers => PermissionDecision::Allow {
                updated_input: Some(input.clone()),
            },
        };
        let _ = respond.send(decision);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(header: Option<&str>) -> http::request::Parts {
        let mut builder = http::Request::builder()
            .method("POST")
            .uri("http://127.0.0.1:9/mcp");
        if let Some(value) = header {
            builder = builder.header(CONVERSATION_HEADER, value);
        }
        builder.body(()).unwrap().into_parts().0
    }

    #[test]
    fn conversation_header_is_read_when_present() {
        assert_eq!(
            conversation_from(&parts(Some("conv-1"))),
            Some("conv-1"),
            "a dsh session tags every request with its conversation"
        );
    }

    #[test]
    fn conversation_header_absent_means_no_conversation() {
        assert_eq!(conversation_from(&parts(None)), None);
    }

    #[test]
    fn empty_or_whitespace_conversation_is_no_conversation() {
        assert_eq!(conversation_from(&parts(Some(""))), None);
        assert_eq!(conversation_from(&parts(Some("   "))), None);
        assert_eq!(
            conversation_from(&parts(Some("  conv-2  "))),
            Some("conv-2"),
            "surrounding whitespace is not part of the id"
        );
    }

    #[test]
    fn tool_definition_carries_the_shared_name_description_and_schema() {
        let tool = tool_definition();
        assert_eq!(tool.name, TOOL_ASK_USER_QUESTION);
        assert_eq!(
            tool.description.as_deref(),
            Some(crate::ask_user::ASK_TOOL_DESCRIPTION)
        );
        let schema = serde_json::Value::Object((*tool.input_schema).clone());
        assert_eq!(schema, crate::ask_user::ask_tool_schema());
        // The per-option `description` is data the model fills in. It must
        // survive `tools/list`, which strips schema descriptions elsewhere.
        let text = schema.to_string();
        assert!(text.contains("What choosing this option means."), "{text}");
        assert!(text.contains("multiSelect"), "{text}");
    }
}
