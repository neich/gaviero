//! Streamable-HTTP transport for the in-process MCP client, over the
//! workspace's `reqwest 0.12`.
//!
//! rmcp ships a ready-made `StreamableHttpClient` impl, but only behind its
//! `transport-streamable-http-client-reqwest` feature, which pulls `reqwest
//! 0.13` beside the `0.12` every other gaviero crate uses. The trait is the
//! documented extension point ("choose your preferred HTTP client … by
//! implementing the `StreamableHttpClient` trait"), so this is a port of
//! rmcp 1.5.0's `transport/common/reqwest/streamable_http_client.rs`
//! (MIT / Apache-2.0) onto 0.12 — the request-builder and response APIs it
//! uses are identical across the two majors. The OAuth `www-authenticate`
//! handling is reduced to a plain error: gaviero's extra servers carry no
//! interactive auth flow, and a bearer token rides `auth_header`.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use futures::stream::BoxStream;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderName, HeaderValue};
use rmcp::model::{ClientJsonRpcMessage, JsonRpcMessage, ServerJsonRpcMessage};
use rmcp::transport::common::http_header::{
    EVENT_STREAM_MIME_TYPE, HEADER_LAST_EVENT_ID, HEADER_SESSION_ID, JSON_MIME_TYPE,
};
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::{Sse, SseStream};

/// `reqwest 0.12` client implementing rmcp's [`StreamableHttpClient`].
#[derive(Clone)]
pub struct ReqwestHttpClient(reqwest::Client);

impl ReqwestHttpClient {
    /// Idle pooling is disabled for the same reason rmcp's default client
    /// disables it: reusing a connection whose previous SSE body was not fully
    /// drained stalls on delayed ACKs.
    pub fn new() -> Self {
        Self(
            reqwest::Client::builder()
                .pool_max_idle_per_host(0)
                .build()
                .unwrap_or_default(),
        )
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

type HttpError = StreamableHttpError<reqwest::Error>;

fn client_err(e: reqwest::Error) -> HttpError {
    StreamableHttpError::Client(e)
}

fn apply_headers(
    mut builder: reqwest::RequestBuilder,
    custom_headers: HashMap<HeaderName, HeaderValue>,
) -> reqwest::RequestBuilder {
    for (name, value) in custom_headers {
        builder = builder.header(name, value);
    }
    builder
}

fn is_mime(content_type: &[u8], mime: &str) -> bool {
    content_type.starts_with(mime.as_bytes())
}

/// A non-success body that is a JSON-RPC *error* is surfaced as that error
/// (rmcp then reports it as an `McpError`) rather than lost as a transport
/// failure.
fn parse_json_rpc_error(body: &str) -> Option<ServerJsonRpcMessage> {
    match serde_json::from_str::<ServerJsonRpcMessage>(body) {
        Ok(message @ JsonRpcMessage::Error(_)) => Some(message),
        _ => None,
    }
}

impl StreamableHttpClient for ReqwestHttpClient {
    type Error = reqwest::Error;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, HttpError> {
        let mut request = self
            .0
            .get(uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "))
            .header(HEADER_SESSION_ID, session_id.as_ref());
        if let Some(last_event_id) = last_event_id {
            request = request.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(token) = auth_token {
            request = request.bearer_auth(token);
        }
        let response = apply_headers(request, custom_headers)
            .send()
            .await
            .map_err(client_err)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        let response = response.error_for_status().map_err(client_err)?;
        match response.headers().get(CONTENT_TYPE) {
            Some(ct)
                if is_mime(ct.as_bytes(), EVENT_STREAM_MIME_TYPE)
                    || is_mime(ct.as_bytes(), JSON_MIME_TYPE) => {}
            other => {
                return Err(StreamableHttpError::UnexpectedContentType(
                    other.map(|ct| String::from_utf8_lossy(ct.as_bytes()).into_owned()),
                ));
            }
        }
        Ok(SseStream::from_bytes_stream(response.bytes_stream()).boxed())
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), HttpError> {
        let mut request = self
            .0
            .delete(uri.as_ref())
            .header(HEADER_SESSION_ID, session.as_ref());
        if let Some(token) = auth_token {
            request = request.bearer_auth(token);
        }
        let response = apply_headers(request, custom_headers)
            .send()
            .await
            .map_err(client_err)?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            tracing::debug!("MCP server does not support deleting the session");
            return Ok(());
        }
        response.error_for_status().map_err(client_err)?;
        Ok(())
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        let mut request = self
            .0
            .post(uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "));
        if let Some(token) = auth_token {
            request = request.bearer_auth(token);
        }
        let session_was_attached = session_id.is_some();
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = apply_headers(request, custom_headers)
            .json(&message)
            .send()
            .await
            .map_err(client_err)?;

        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == reqwest::StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|ct| String::from_utf8_lossy(ct.as_bytes()).into_owned());
        let session_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<failed to read response body>".to_owned());
            if content_type
                .as_deref()
                .is_some_and(|ct| is_mime(ct.as_bytes(), JSON_MIME_TYPE))
                && let Some(message) = parse_json_rpc_error(&body)
            {
                return Ok(StreamableHttpPostResponse::Json(message, session_id));
            }
            return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
                format!("HTTP {status}: {body}"),
            )));
        }

        match content_type.as_deref() {
            Some(ct) if is_mime(ct.as_bytes(), EVENT_STREAM_MIME_TYPE) => {
                let stream = SseStream::from_bytes_stream(response.bytes_stream()).boxed();
                Ok(StreamableHttpPostResponse::Sse(stream, session_id))
            }
            Some(ct) if is_mime(ct.as_bytes(), JSON_MIME_TYPE) => {
                // A 200 to a notification may carry a body that is not a
                // JSON-RPC message; rmcp treats that as accepted, and so do we.
                match response.json::<ServerJsonRpcMessage>().await {
                    Ok(message) => Ok(StreamableHttpPostResponse::Json(message, session_id)),
                    Err(e) => {
                        tracing::warn!(
                            "MCP JSON response is not a JSON-RPC message, treating as accepted: {e}"
                        );
                        Ok(StreamableHttpPostResponse::Accepted)
                    }
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_rpc_error_bodies_are_recognised() {
        let body =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid Request"}}"#;
        assert!(parse_json_rpc_error(body).is_some());
        assert!(parse_json_rpc_error(r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#).is_none());
        assert!(parse_json_rpc_error("not json").is_none());
    }
}
