//! HTTP side, behind nginx's TLS: host control and data WebSockets, the
//! phone notification stream, and a health check.

use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use futures::{SinkExt, StreamExt, stream};
use serde::Deserialize;
use tokio::sync::{broadcast, mpsc};

use crate::proto::{HostMsg, PhoneEvent, RelayMsg};
use crate::state::Relay;

/// Control frames are small JSON objects.
const MAX_CONTROL_MESSAGE: usize = 64 * 1024;
/// Data frames carry TLS records (≤ 16 KiB each) in ≤ 64 KiB chunks.
const MAX_DATA_MESSAGE: usize = 256 * 1024;
/// Notifications per control connection, refilled every second.
const NOTIFY_BURST: u32 = 20;

pub(crate) fn router(relay: Arc<Relay>) -> Router {
    Router::new()
        .route("/v1/host", get(host_ws))
        .route("/v1/data/{nonce}", get(data_ws))
        .route("/v1/phone/stream", get(phone_stream))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(relay)
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
}

// ── Host control ────────────────────────────────────────────────────

async fn host_ws(
    State(relay): State<Arc<Relay>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    if !relay.authorized(bearer(&headers)) {
        return unauthorized();
    }
    ws.max_message_size(MAX_CONTROL_MESSAGE)
        .on_upgrade(move |socket| host_session(relay, socket))
}

async fn host_session(relay: Arc<Relay>, socket: WebSocket) {
    let conn_id = relay.next_conn_id();
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<RelayMsg>(64);
    let keepalive = relay.cfg.keepalive;

    let writer = async move {
        let mut tick = tokio::time::interval(keepalive);
        tick.tick().await;
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    let Some(msg) = msg else { break };
                    let text = serde_json::to_string(&msg).expect("relay message serializes");
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                _ = tick.tick() => {
                    if sink.send(Message::Ping(Bytes::new())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sink.close().await;
    };

    let reader = {
        let relay = relay.clone();
        let tx = tx.clone();
        async move {
            let mut budget = NOTIFY_BURST;
            let mut refill = tokio::time::Instant::now();
            loop {
                let next = tokio::time::timeout(keepalive * 3, stream.next()).await;
                let msg = match next {
                    Ok(Some(Ok(msg))) => msg,
                    _ => break, // idle, closed, or error
                };
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue, // pings/pongs only prove liveness
                };
                let Ok(parsed) = serde_json::from_str::<HostMsg>(&text) else {
                    let _ = tx
                        .send(RelayMsg::Error {
                            message: "unparseable message".into(),
                        })
                        .await;
                    continue;
                };
                match parsed {
                    HostMsg::Register { info } => {
                        let route = info.route.clone();
                        let reply = match relay.register(conn_id, &tx, info) {
                            Ok(()) => {
                                tracing::info!(conn_id, %route, "route registered");
                                relay.broadcast_presence();
                                RelayMsg::Registered { route }
                            }
                            Err(reason) => RelayMsg::Rejected { route, reason },
                        };
                        let _ = tx.send(reply).await;
                    }
                    HostMsg::Unregister { route } => {
                        relay.unregister(conn_id, &route);
                        relay.broadcast_presence();
                    }
                    HostMsg::Notify {
                        machine,
                        workspace_id,
                        conv,
                        kind,
                    } => {
                        if refill.elapsed() >= std::time::Duration::from_secs(1) {
                            budget = NOTIFY_BURST;
                            refill = tokio::time::Instant::now();
                        }
                        if budget == 0 {
                            continue;
                        }
                        budget -= 1;
                        relay.push_note(machine, workspace_id, conv, kind);
                    }
                }
            }
        }
    };

    drop(tx);
    tokio::select! {
        _ = writer => {}
        _ = reader => {}
    }
    if relay.drop_conn(conn_id) {
        relay.broadcast_presence();
    }
    tracing::info!(conn_id, "host control connection closed");
}

// ── Host data ───────────────────────────────────────────────────────

async fn data_ws(
    State(relay): State<Arc<Relay>>,
    headers: HeaderMap,
    Path(nonce): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    if !relay.authorized(bearer(&headers)) {
        return unauthorized();
    }
    let Some(waiting) = relay.take_pending(&nonce) else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    ws.max_message_size(MAX_DATA_MESSAGE)
        .on_upgrade(move |socket| async move {
            // The splice task owns the socket from here.
            let _ = waiting.send(socket);
        })
}

// ── Phone notification stream ───────────────────────────────────────

#[derive(Deserialize)]
struct StreamQuery {
    since: Option<u64>,
    /// The epoch the phone's `since` belongs to; a different one (relay
    /// restarted) makes `since` meaningless, so everything stored is sent.
    epoch: Option<String>,
}

fn line(event: &PhoneEvent) -> Bytes {
    let mut text = serde_json::to_string(event).expect("phone event serializes");
    text.push('\n');
    Bytes::from(text)
}

async fn phone_stream(
    State(relay): State<Arc<Relay>>,
    headers: HeaderMap,
    Query(query): Query<StreamQuery>,
) -> Response {
    if !relay.authorized(bearer(&headers)) {
        return unauthorized();
    }
    // Subscribe before taking the replay so nothing falls in between.
    let rx = relay.events.subscribe();
    let since = match &query.epoch {
        Some(epoch) if *epoch != relay.epoch => 0,
        _ => query.since.unwrap_or(0),
    };
    // A `since` above anything this process ever issued belongs to an
    // earlier run (a phone that has not learned the epoch yet). Using it as
    // the live-dedupe floor would drop every new notification — treat as 0.
    let since = if since > relay.max_note_id() { 0 } else { since };
    let mut replay = vec![relay.presence()];
    replay.extend(relay.notes_since(since));
    let last_id = replay
        .iter()
        .filter_map(|e| match e {
            PhoneEvent::Notify { id, .. } => Some(*id),
            _ => None,
        })
        .max()
        .unwrap_or(since);
    let replay: Vec<Bytes> = replay.iter().map(line).collect();

    let mut tick = tokio::time::interval(relay.cfg.keepalive);
    tick.reset(); // first keepalive after one period, not immediately
    let live = stream::unfold((rx, tick, last_id), |(mut rx, mut tick, mut last)| async move {
        loop {
            tokio::select! {
                event = rx.recv() => match event {
                    Ok(event) => {
                        if let PhoneEvent::Notify { id, .. } = &event {
                            if *id <= last {
                                continue;
                            }
                            last = *id;
                        }
                        return Some((line(&event), (rx, tick, last)));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                },
                _ = tick.tick() => {
                    return Some((line(&PhoneEvent::Keepalive {}), (rx, tick, last)));
                }
            }
        }
    });
    let body = Body::from_stream(
        stream::iter(replay)
            .chain(live)
            .map(Ok::<Bytes, Infallible>),
    );
    Response::builder()
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Accel-Buffering", "no")
        .body(body)
        .expect("valid response")
}
