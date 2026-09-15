//! Child-stdio JSON-RPC (newline-delimited), Codex `app-server` style.
//!
//! `crate::acp` is the legacy Claude NDJSON transport. This module is the
//! Agent Client Protocol JSON-RPC loop: correlated requests, notifications,
//! and *incoming* requests the child makes of the client (`fs/*`,
//! `session/request_permission`).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{Mutex, mpsc, oneshot};

const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug)]
pub struct IncomingRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

#[derive(Clone)]
pub struct JsonRpcHandle {
    stdin: Arc<Mutex<BufWriter<ChildStdin>>>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>>,
    next_id: Arc<AtomicU64>,
}

#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

pub struct JsonRpcChild {
    child: Child,
    pub handle: JsonRpcHandle,
    pub incoming: mpsc::Receiver<IncomingRequest>,
    pub notifications: mpsc::Receiver<Value>,
    reader: tokio::task::JoinHandle<()>,
}

impl JsonRpcHandle {
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.request_timeout(method, params, DEFAULT_REQUEST_TIMEOUT)
            .await
    }

    pub async fn request_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id();
        let id_key = id.to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id_key.clone(), tx);
        }
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_line(&msg).await?;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(Ok(v))) => Ok(v),
            Ok(Ok(Err(e))) => Err(anyhow!("ACP RPC {method} error {}: {}", e.code, e.message)),
            Ok(Err(_)) => Err(anyhow!("ACP RPC {method}: caller dropped")),
            Err(_) => {
                self.pending.lock().await.remove(&id_key);
                Err(anyhow!("ACP RPC {method} timed out after {timeout:?}"))
            }
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<()> {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_line(&msg).await
    }

    pub async fn respond(&self, id: Value, result: Value) -> Result<()> {
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        }))
        .await
    }

    pub async fn respond_error(&self, id: Value, code: i64, message: impl Into<String>) -> Result<()> {
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": message.into() },
        }))
        .await
    }

    async fn write_line(&self, msg: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(&line).await.context("ACP stdin write")?;
        stdin.flush().await.context("ACP stdin flush")?;
        Ok(())
    }
}

impl JsonRpcChild {
    pub async fn spawn(mut cmd: tokio::process::Command) -> Result<Self> {
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().context("spawning ACP agent")?;
        let stdout = child.stdout.take().context("ACP stdout unavailable")?;
        let stdin = child.stdin.take().context("ACP stdin unavailable")?;
        let stderr = child.stderr.take();

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!(target: "dsh_acp", "agent stderr: {line}");
                }
            });
        }

        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (incoming_tx, incoming_rx) = mpsc::channel(64);
        let (notif_tx, notif_rx) = mpsc::channel(256);
        let pending_reader = pending.clone();

        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    tracing::warn!(target: "dsh_acp", "non-JSON ACP line: {line}");
                    continue;
                };
                dispatch_line(value, &pending_reader, &incoming_tx, &notif_tx).await;
            }
            let mut pending = pending_reader.lock().await;
            for (_, tx) in pending.drain() {
                let _ = tx.send(Err(RpcError {
                    code: -1,
                    message: "ACP agent stdout closed".into(),
                }));
            }
        });

        Ok(Self {
            child,
            handle: JsonRpcHandle {
                stdin: Arc::new(Mutex::new(BufWriter::new(stdin))),
                pending,
                next_id: Arc::new(AtomicU64::new(1)),
            },
            incoming: incoming_rx,
            notifications: notif_rx,
            reader,
        })
    }

    pub async fn kill(&mut self) {
        self.reader.abort();
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

impl Drop for JsonRpcChild {
    fn drop(&mut self) {
        self.reader.abort();
        let _ = self.child.start_kill();
    }
}

fn id_key(id: &Value) -> String {
    match id {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

async fn dispatch_line(
    value: Value,
    pending: &Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>>,
    incoming: &mpsc::Sender<IncomingRequest>,
    notifs: &mpsc::Sender<Value>,
) {
    let method = value.get("method").and_then(|m| m.as_str());
    let id = value.get("id").cloned();
    if let Some(method) = method {
        if let Some(id) = id {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let _ = incoming
                .send(IncomingRequest {
                    id,
                    method: method.to_string(),
                    params,
                })
                .await;
            return;
        }
        let _ = notifs.send(value).await;
        return;
    }
    if let Some(id) = id {
        let key = id_key(&id);
        let mut pending = pending.lock().await;
        if let Some(tx) = pending.remove(&key) {
            if let Some(err) = value.get("error") {
                let _ = tx.send(Err(RpcError {
                    code: err.get("code").and_then(|c| c.as_i64()).unwrap_or(-1),
                    message: err
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("error")
                        .to_string(),
                }));
            } else {
                let result = value.get("result").cloned().unwrap_or(Value::Null);
                let _ = tx.send(Ok(result));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn dispatch_correlates_response_to_pending() {
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (incoming_tx, mut incoming_rx) = mpsc::channel(4);
        let (notif_tx, mut notif_rx) = mpsc::channel(4);
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert("7".into(), tx);

        dispatch_line(
            json!({"jsonrpc":"2.0","id":7,"result":{"ok":true}}),
            &pending,
            &incoming_tx,
            &notif_tx,
        )
        .await;
        let v = rx.await.unwrap().unwrap();
        assert_eq!(v["ok"], true);
        assert!(incoming_rx.try_recv().is_err());
        assert!(notif_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn dispatch_routes_notification_and_incoming_request() {
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, RpcError>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (incoming_tx, mut incoming_rx) = mpsc::channel(4);
        let (notif_tx, mut notif_rx) = mpsc::channel(4);

        dispatch_line(
            json!({"jsonrpc":"2.0","method":"session/update","params":{"x":1}}),
            &pending,
            &incoming_tx,
            &notif_tx,
        )
        .await;
        let n = notif_rx.recv().await.unwrap();
        assert_eq!(n["method"], "session/update");

        dispatch_line(
            json!({"jsonrpc":"2.0","id":99,"method":"fs/read_text_file","params":{"path":"a.rs"}}),
            &pending,
            &incoming_tx,
            &notif_tx,
        )
        .await;
        let req = incoming_rx.recv().await.unwrap();
        assert_eq!(req.method, "fs/read_text_file");
        assert_eq!(req.params["path"], "a.rs");
    }

    #[test]
    fn id_key_formats_number_and_string() {
        assert_eq!(id_key(&json!(7)), "7");
        assert_eq!(id_key(&json!("abc")), "abc");
    }
}
