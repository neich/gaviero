//! Dual-era MCP stdio handshake (legacy rmcp vs 2026-07-28 probes).
//!
//! rmcp 1.5 is an initialize-first (legacy) server. Dual-era clients
//! — Claude Code / Cursor after the 2026-07-28 spec — **SHOULD** probe
//! with `server/discover` before any other request on stdio. rmcp
//! treats that as a failed handshake (`expect initialized request`) and
//! drops the connection, so the client never gets to send `initialize`.
//!
//! The spec's fallback rule: any non-modern error (or timeout) means
//! "server is legacy — fall back to `initialize`" on the **same**
//! stream. We answer `server/discover` with JSON-RPC `-32601` (method
//! not found), keep the connection open, and replay the first
//! non-discover message into rmcp.

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Cap for a single handshake line. Discover / initialize payloads are
/// tiny; this is a runaway-client guard, not a protocol limit.
const MAX_HANDSHAKE_LINE: usize = 1024 * 1024;

/// Drain `server/discover` probes from `read`, answering each with a
/// method-not-found error on `write`.
///
/// Returns the first non-discover line (including its trailing newline,
/// plus any extra bytes already pulled from the stream) so the caller
/// can prepend them for rmcp. An empty vec means the peer closed during
/// the probe (no session to hand off).
pub(crate) async fn absorb_discover_probes<R, W>(
    read: &mut R,
    write: &mut W,
) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut pending = Vec::new();
    loop {
        if let Some(newline_at) = pending.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = pending.drain(..=newline_at).collect();
            if is_blank(&line) {
                continue;
            }
            if is_server_discover(&line) {
                if let Some(id) = request_id(&line) {
                    tracing::debug!(
                        target: "mcp_server",
                        id = %id,
                        "answered pre-initialize server/discover with method-not-found \
                         (legacy MCP handshake)"
                    );
                    write.write_all(&method_not_found_response(&id)).await?;
                    write.flush().await?;
                }
                continue;
            }
            line.extend_from_slice(&pending);
            return Ok(line);
        }

        let mut chunk = [0u8; 1024];
        let n = read.read(&mut chunk).await?;
        if n == 0 {
            if pending.is_empty() {
                return Ok(Vec::new());
            }
            if is_blank(&pending) {
                return Ok(Vec::new());
            }
            if is_server_discover(&pending) {
                if let Some(id) = request_id(&pending) {
                    write.write_all(&method_not_found_response(&id)).await?;
                    write.flush().await?;
                }
                return Ok(Vec::new());
            }
            return Ok(std::mem::take(&mut pending));
        }
        pending.extend_from_slice(&chunk[..n]);
        if pending.len() > MAX_HANDSHAKE_LINE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "MCP handshake line exceeded 1 MiB",
            ));
        }
    }
}

fn is_blank(line: &[u8]) -> bool {
    without_crlf(line).iter().all(|b| b.is_ascii_whitespace())
}

fn without_crlf(line: &[u8]) -> &[u8] {
    let mut s = line;
    if s.last() == Some(&b'\n') {
        s = &s[..s.len() - 1];
    }
    if s.last() == Some(&b'\r') {
        s = &s[..s.len() - 1];
    }
    s
}

fn parse_line(line: &[u8]) -> Option<Value> {
    serde_json::from_slice(without_crlf(line)).ok()
}

fn is_server_discover(line: &[u8]) -> bool {
    parse_line(line)
        .and_then(|v| {
            v.get("method")
                .and_then(Value::as_str)
                .map(|m| m == "server/discover")
        })
        .unwrap_or(false)
}

fn request_id(line: &[u8]) -> Option<Value> {
    parse_line(line).and_then(|v| v.get("id").cloned())
}

fn method_not_found_response(id: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32601,
            "message": "Method not found"
        }
    }))
    .expect("json! error object is always serializable");
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const DISCOVER: &[u8] = br#"{"jsonrpc":"2.0","id":"server-discover-probe-1","method":"server/discover","params":{}}"#;
    const INITIALIZE: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;

    fn line(msg: &[u8]) -> Vec<u8> {
        let mut v = msg.to_vec();
        v.push(b'\n');
        v
    }

    async fn roundtrip(client_writes: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let (client, server) = tokio::io::duplex(8192);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (mut client_r, mut client_w) = tokio::io::split(client);

        let absorb = absorb_discover_probes(&mut rx, &mut tx);
        let write = async {
            client_w.write_all(client_writes).await.unwrap();
            client_w.shutdown().await.unwrap();
        };
        let (leftover, _) = tokio::join!(absorb, write);
        let leftover = leftover.unwrap();
        // EOF the client read half so we can collect whatever we wrote
        // without waiting forever on an open server socket.
        drop(tx);
        drop(rx);
        let mut written = Vec::new();
        client_r.read_to_end(&mut written).await.unwrap();
        (leftover, written)
    }

    fn parse_json_line(bytes: &[u8]) -> Value {
        serde_json::from_slice(without_crlf(bytes)).expect("json line")
    }

    #[tokio::test]
    async fn initialize_first_is_passed_through_unmodified() {
        let payload = line(INITIALIZE);
        let (leftover, written) = roundtrip(&payload).await;
        assert_eq!(leftover, payload);
        assert!(written.is_empty(), "no discover → no response: {written:?}");
    }

    #[tokio::test]
    async fn discover_then_initialize_keeps_the_connection() {
        let mut payload = line(DISCOVER);
        payload.extend_from_slice(&line(INITIALIZE));
        let (leftover, written) = roundtrip(&payload).await;
        assert_eq!(leftover, line(INITIALIZE));

        let err = parse_json_line(&written);
        assert_eq!(err["id"], "server-discover-probe-1");
        assert_eq!(err["error"]["code"], -32601);
        assert!(
            err.get("result").is_none(),
            "must not look like a modern DiscoverResult: {err}"
        );
    }

    #[tokio::test]
    async fn two_discovers_then_initialize() {
        let mut payload = line(DISCOVER);
        payload.extend_from_slice(
            br#"{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{}}
"#,
        );
        payload.extend_from_slice(&line(INITIALIZE));
        let (leftover, written) = roundtrip(&payload).await;
        assert_eq!(leftover, line(INITIALIZE));
        let responses: Vec<&[u8]> = written
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(responses.len(), 2, "{written:?}");
        assert_eq!(
            parse_json_line(responses[0])["id"],
            "server-discover-probe-1"
        );
        assert_eq!(parse_json_line(responses[1])["id"], 2);
    }

    #[tokio::test]
    async fn numeric_discover_id_is_echoed() {
        let payload = line(br#"{"jsonrpc":"2.0","id":7,"method":"server/discover"}"#);
        let (_, written) = roundtrip(&payload).await;
        assert_eq!(parse_json_line(&written)["id"], 7);
    }

    #[tokio::test]
    async fn discover_notification_has_no_response() {
        let mut payload = line(br#"{"jsonrpc":"2.0","method":"server/discover","params":{}}"#);
        payload.extend_from_slice(&line(INITIALIZE));
        let (leftover, written) = roundtrip(&payload).await;
        assert_eq!(leftover, line(INITIALIZE));
        assert!(written.is_empty(), "{written:?}");
    }

    #[tokio::test]
    async fn ping_is_not_absorbed() {
        let ping = line(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        let (leftover, written) = roundtrip(&ping).await;
        assert_eq!(leftover, ping);
        assert!(written.is_empty());
    }
}
