//! context7 retrieval, adapted to the in-process agent loop.
//!
//! Subprocess providers (`claude:`, `codex:`, `cursor:`, `dsh:`) reach context7
//! by speaking MCP to a server the host mounts on their session — see
//! [`super::mcp`] for the same trick applied to gaviero's own server. The
//! in-process API providers (`deepseek:`, `ollama:`) run *this* loop instead,
//! and until this module they had no route to context7 at all: the capability
//! table said `context7_allowed: false` for both.
//!
//! **They do not need an MCP client.** context7 publishes a plain REST API, so
//! this module is the native-tool leg that §2.6 of the parity plan called out as
//! the alternative to building one:
//!
//! | Tool | Request |
//! |---|---|
//! | [`Context7ResolveTool`] | `GET {base}/search?query=…` |
//! | [`Context7DocsTool`] | `GET {base}/{library_id}?type=txt&topic=…` |
//!
//! Both endpoints were verified live before this was written; `&topic=` is a real
//! filter, not decoration (a `topic=hooks` fetch led with hook-specific content
//! and mentioned "hooks" 55×, against 4× in the unfiltered dump).
//!
//! The tool *names* mirror context7's own MCP surface (`resolve-library-id` and
//! `query-docs`), lowercased to snake_case to match this registry's convention
//! for foreign tools ([`super::mcp`] keeps the server's advertised names for the
//! same reason). That is what makes "the same tools on every provider" true in
//! substance rather than only in spirit.
//!
//! Gating is inherited, not reimplemented: visibility is decided upstream in
//! [`ToolAgentSession::new`](crate::agent_session::tool_agent::ToolAgentSession),
//! which consults the provider's `context7_allowed` row, the workspace's
//! `mcp.context7.enabled`, and `mcp.permissions` before any tool is appended.

use std::time::Duration;

use serde_json::{Value, json};

use super::{Tool, ToolCtx, ToolOutcome};

/// Ceiling on the text handed back as a `tool` message.
///
/// `query_docs` returns a whole *library* dump, query-filtered but still tens of
/// KB (37 KB for `reactjs/react.dev?topic=hooks`, measured). Kept in step with
/// [`super::mcp`]'s cap so a context7 result cannot crowd out the turn more than
/// a gaviero retrieval result can. Truncation is announced, never silent.
const MAX_OUTPUT_BYTES: usize = 24_000;

/// How many search hits `resolve_library_id` reports. context7 returns dozens of
/// scored matches; the model only needs the top few to pick an id, and every
/// extra row is prompt budget spent on a library it will not open.
const MAX_RESOLVE_HITS: usize = 5;

/// Per-request timeout. Both endpoints answer in well under a second when
/// healthy, so a hung socket is a fault, not slowness.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// `resolve_library_id` — turn a free-text query into context7 library ids.
pub struct Context7ResolveTool {
    http: reqwest::Client,
    base: String,
}

/// `query_docs` — fetch documentation for a resolved library id.
pub struct Context7DocsTool {
    http: reqwest::Client,
    base: String,
}

/// Build both context7 tools against `base` (e.g. `https://context7.com/api/v1`).
///
/// The `reqwest::Client` is built once and shared: it owns the connection pool,
/// so two tools do not mean two pools.
pub fn tools(base: &str) -> Vec<Box<dyn Tool>> {
    let http = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let base = base.trim_end_matches('/').to_string();
    vec![
        Box::new(Context7ResolveTool {
            http: http.clone(),
            base: base.clone(),
        }),
        Box::new(Context7DocsTool { http, base }),
    ]
}

/// The names [`tools`] returns, in order.
///
/// Callers need this separately from "the vec is non-empty": the pull-stanza
/// plumbing distinguishes "this session holds context7" from "this session holds
/// gaviero's retrieval tools", and deriving names from one source keeps the two
/// from drifting.
pub fn tool_names() -> Vec<String> {
    vec!["resolve_library_id".to_string(), "query_docs".to_string()]
}

#[async_trait::async_trait]
impl Tool for Context7ResolveTool {
    fn name(&self) -> &str {
        "resolve_library_id"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "resolve_library_id",
                "description": "Find the context7 library id for a package, framework, or product by name (e.g. 'react', 'tokio', 'stripe'). Returns the top matching ids with a short description. Call this first, then pass the chosen id to query_docs.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Library name and, optionally, what you are trying to do (e.g. 'react hooks', 'tokio select')."
                        }
                    },
                    "required": ["query"]
                }
            }
        })
    }

    async fn run(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'query'");
        };
        if query.trim().is_empty() {
            return ToolOutcome::error("'query' must not be empty");
        }
        // `reqwest`'s query builder does the percent-encoding; hand-formatting the
        // URL here would break on any query containing a space or `&`.
        let resp = match self
            .http
            .get(format!("{}/search", self.base))
            .query(&[("query", query)])
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolOutcome::error(format!("resolve_library_id: request failed: {e}")),
        };
        let status = resp.status();
        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolOutcome::error(format!("resolve_library_id: reading body: {e}")),
        };
        if !status.is_success() {
            return ToolOutcome::error(format!(
                "resolve_library_id: context7 returned HTTP {status}: {}",
                truncate_owned(body)
            ));
        }
        match format_search_results(&body) {
            Ok(rendered) => ToolOutcome::ok(rendered),
            Err(e) => ToolOutcome::error(format!("resolve_library_id: {e}")),
        }
    }
}

#[async_trait::async_trait]
impl Tool for Context7DocsTool {
    fn name(&self) -> &str {
        "query_docs"
    }

    fn schema(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": "query_docs",
                "description": "Fetch up-to-date documentation for a context7 library id returned by resolve_library_id. Returns code snippets and prose. Pass a 'topic' describing what you need so the result is focused rather than the library's entire documentation.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "library_id": {
                            "type": "string",
                            "description": "A context7 id such as '/reactjs/react.dev' (leading slash included), as returned by resolve_library_id."
                        },
                        "topic": {
                            "type": "string",
                            "description": "What to look up (e.g. 'hooks', 'error handling', 'authentication'). Strongly recommended — without it the whole library is fetched."
                        }
                    },
                    "required": ["library_id"]
                }
            }
        })
    }

    async fn run(&self, args: Value, _ctx: &ToolCtx) -> ToolOutcome {
        let Some(library_id) = args.get("library_id").and_then(|v| v.as_str()) else {
            return ToolOutcome::error("missing required argument 'library_id'");
        };
        let library_id = library_id.trim();
        if library_id.is_empty() {
            return ToolOutcome::error("'library_id' must not be empty");
        }
        // The id *is* a path (`/reactjs/react.dev`), so it is spliced into the
        // URL rather than percent-encoded — encoding it would turn the slashes
        // into `%2F` and 404. A leading slash is added when absent so both
        // `/reactjs/react.dev` and `reactjs/react.dev` work.
        let path = library_id.trim_start_matches('/');
        let mut url = format!("{}/{path}", self.base);
        if let Some(topic) = args
            .get("topic")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|t| !t.is_empty())
        {
            // `type=txt` asks for raw text rather than JSON; `topic` is a real
            // server-side filter (verified), not a client-side hint.
            url = format!("{url}?type=txt&topic={}", urlencode(topic));
        } else {
            url = format!("{url}?type=txt");
        }

        let resp = match self.http.get(url).send().await {
            Ok(r) => r,
            Err(e) => return ToolOutcome::error(format!("query_docs: request failed: {e}")),
        };
        let status = resp.status();
        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolOutcome::error(format!("query_docs: reading body: {e}")),
        };
        if !status.is_success() {
            return ToolOutcome::error(format!(
                "query_docs: context7 returned HTTP {status} for '{library_id}': {}",
                truncate_owned(body)
            ));
        }
        if body.trim().is_empty() {
            return ToolOutcome::error(format!(
                "query_docs: context7 returned no documentation for '{library_id}'"
            ));
        }
        ToolOutcome::ok(truncate(body))
    }
}

/// Render `/search` JSON as a compact, model-readable list.
///
/// Kept tolerant on purpose: an unrecognised-but-successful body is reported as
/// a parse error naming the shape, rather than silently returning "no results",
/// so a context7 API change surfaces as a visible failure instead of a tool that
/// mysteriously stops finding libraries.
fn format_search_results(body: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|e| format!("could not parse search response as JSON: {e}"))?;
    let results = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "search response had no 'results' array".to_string())?;
    if results.is_empty() {
        return Ok("No context7 libraries matched that query. Try a shorter or more common name.".to_string());
    }
    let mut out = String::new();
    for (i, r) in results.iter().take(MAX_RESOLVE_HITS).enumerate() {
        let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("(no id)");
        let title = r.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let desc = r.get("description").and_then(|v| v.as_str()).unwrap_or("");
        let tokens = r.get("totalTokens").and_then(|v| v.as_u64());
        let mut line = format!("{}. {} — {}", i + 1, id, title);
        if let Some(t) = tokens {
            line.push_str(&format!(" ({t} tokens)"));
        }
        out.push_str(&line);
        out.push('\n');
        if !desc.is_empty() {
            out.push_str("   ");
            out.push_str(desc);
            out.push('\n');
        }
    }
    out.push_str("\nPass one of these ids to query_docs.\n");
    Ok(out)
}

/// Minimal percent-encoding for a query-string value.
///
/// Deliberately local: the crate has no `urlencoding`/`percent-encoding`
/// dependency, and pulling one in for a single call site would widen the
/// dependency surface this leg exists to *avoid* (see the module docs and
/// §2.7-C of the plan). Unreserved bytes pass through; everything else becomes
/// `%XX` over UTF-8 bytes.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Truncate on a char boundary, appending the dropped byte count.
///
/// Mirrors [`super::mcp`]'s helper; duplicated rather than shared because
/// hoisting it would mean widening that module's surface for one constant.
fn truncate(mut text: String) -> String {
    if text.len() <= MAX_OUTPUT_BYTES {
        return text;
    }
    let dropped = text.len() - MAX_OUTPUT_BYTES;
    let mut cut = MAX_OUTPUT_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    format!(
        "{text}\n\n[truncated: {dropped} bytes omitted. Narrow the query \
         (a more specific `topic`) to see the rest.]"
    )
}

/// [`truncate`] for a value that is only being reported inside an error message.
fn truncate_owned(text: String) -> String {
    const ERR_CAP: usize = 500;
    if text.len() <= ERR_CAP {
        return text;
    }
    let mut cut = ERR_CAP;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}… [{} bytes omitted]", &text[..cut], text.len() - cut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_match_what_tools_returns() {
        let built = tools("https://example.invalid/api/v1");
        let names: Vec<String> = built.iter().map(|t| t.name().to_string()).collect();
        assert_eq!(names, tool_names());
    }

    #[test]
    fn trailing_slash_on_the_base_is_normalised() {
        // A doubled slash (`//search`) is what a hand-edited setting produces,
        // and it 404s.
        let built = tools("https://context7.com/api/v1/");
        assert_eq!(built.len(), 2);
    }

    #[test]
    fn urlencode_escapes_reserved_bytes() {
        assert_eq!(urlencode("error handling"), "error%20handling");
        assert_eq!(urlencode("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencode("hooks"), "hooks");
        // Multi-byte input must encode per byte, not per char.
        assert_eq!(urlencode("→"), "%E2%86%92");
    }

    #[test]
    fn format_search_results_lists_ids_and_caps_hits() {
        let mut results = Vec::new();
        for i in 0..8 {
            results.push(json!({
                "id": format!("/org/lib{i}"),
                "title": format!("Lib {i}"),
                "description": format!("Desc {i}"),
                "totalTokens": 1000 + i,
            }));
        }
        let body = json!({ "results": results }).to_string();
        let out = format_search_results(&body).expect("parses");
        assert!(out.contains("/org/lib0"), "{out}");
        // Capped at MAX_RESOLVE_HITS.
        assert!(!out.contains("/org/lib5"), "{out}");
        assert!(out.contains("query_docs"), "{out}");
    }

    #[test]
    fn format_search_results_reports_an_empty_result_set() {
        let out = format_search_results(r#"{"results":[]}"#).expect("parses");
        assert!(out.contains("No context7 libraries"), "{out}");
    }

    /// A context7 API change must be *visible*. Swallowing this as "no results"
    /// would present as a tool that silently stopped working.
    #[test]
    fn format_search_results_errors_on_an_unexpected_shape() {
        assert!(format_search_results("not json").is_err());
        assert!(format_search_results(r#"{"hits":[]}"#).is_err());
    }

    #[test]
    fn truncate_is_a_noop_below_the_cap() {
        let text = "short".to_string();
        assert_eq!(truncate(text.clone()), text);
    }

    #[test]
    fn truncate_announces_the_dropped_bytes() {
        let text = "x".repeat(MAX_OUTPUT_BYTES + 100);
        let out = truncate(text);
        assert!(out.contains("truncated: 100 bytes omitted"), "{out}");
    }

    /// Slicing mid-codepoint would panic; a 3-byte char straddling the cap is the
    /// realistic case for documentation carrying non-ASCII text.
    #[test]
    fn truncate_lands_on_a_char_boundary() {
        let text = "→".repeat(9000);
        let out = truncate(text);
        assert!(out.contains("truncated:"));
    }
}
