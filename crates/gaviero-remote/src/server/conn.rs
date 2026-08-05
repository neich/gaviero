//! Per-socket task: reads `client_hello`, registers with the hub, then
//! pumps frames both ways with ping/idle handling. Protocol violations
//! close with the documented 4xxx codes. A connection that fails before
//! registration can never evict the live client.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, interval, timeout};

use super::AppState;
use crate::dto::ClientHello;
use crate::envelope::{ClientDecode, ClientEnvelope, ClientFrame, decode_client_frame};
use crate::version::PROTOCOL_VERSION;
use crate::close_code;

static NEXT_CONN_ID: AtomicU64 = AtomicU64::new(1);

/// Serialized server frame headed for the socket.
#[derive(Debug)]
pub(crate) enum ConnOut {
    Frame(String),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CloseSignal {
    pub code: u16,
    pub reason: &'static str,
}

pub(crate) struct Registration {
    pub conn_id: u64,
    pub client_hello: ClientHello,
    pub outbound_tx: mpsc::Sender<ConnOut>,
    /// Hub-held force-close: eviction (4005), rotation (4006), shutdown
    /// (4007), slow client (4008).
    pub close_tx: watch::Sender<Option<CloseSignal>>,
}

#[derive(Debug)]
pub(crate) enum ConnIn {
    Command { conn_id: u64, envelope: Box<ClientEnvelope> },
    UnknownCommand {
        conn_id: u64,
        frame_type: String,
        command_id: Option<String>,
    },
    Closed { conn_id: u64 },
}

enum HelloOutcome {
    Ok(ClientHello),
    Close(CloseSignal),
    Gone,
}

pub(crate) async fn run(mut socket: WebSocket, state: AppState) {
    // Phase 1: client_hello must be the first frame (§3.5).
    let hello = match timeout(state.hello_timeout, read_client_hello(&mut socket, &state)).await {
        Ok(HelloOutcome::Ok(h)) => h,
        Ok(HelloOutcome::Close(sig)) => {
            let _ = send_close(&mut socket, sig).await;
            return;
        }
        Ok(HelloOutcome::Gone) => return,
        Err(_) => {
            let _ = send_close(
                &mut socket,
                CloseSignal { code: close_code::PROTOCOL_ERROR, reason: "client_hello timeout" },
            )
            .await;
            return;
        }
    };

    // Wrong major closes 4002 *before* registration, so it cannot evict.
    if PROTOCOL_VERSION.check_compatible(&hello.protocol_version).is_err() {
        let _ = send_close(
            &mut socket,
            CloseSignal { code: close_code::UNSUPPORTED_VERSION, reason: "unsupported protocol major" },
        )
        .await;
        return;
    }

    // Phase 2: register; the hub evicts any previous client and sends
    // `hello` as our first outbound frame.
    let conn_id = NEXT_CONN_ID.fetch_add(1, Ordering::Relaxed);
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<ConnOut>(128);
    let (close_tx, mut close_rx) = watch::channel::<Option<CloseSignal>>(None);
    if state
        .registration_tx
        .send(Registration { conn_id, client_hello: hello, outbound_tx, close_tx })
        .await
        .is_err()
    {
        return; // hub gone (shutdown)
    }

    let (mut tx, mut rx) = socket.split();
    let mut ping = interval(state.ping_interval);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_traffic = Instant::now();

    loop {
        tokio::select! {
            biased;

            changed = close_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                let sig = *close_rx.borrow_and_update();
                if let Some(sig) = sig {
                    let _ = tx
                        .send(Message::Close(Some(CloseFrame {
                            code: sig.code,
                            reason: sig.reason.into(),
                        })))
                        .await;
                    break;
                }
            }

            out = outbound_rx.recv() => {
                match out {
                    Some(ConnOut::Frame(text)) => {
                        if tx.send(Message::Text(text.into())).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                }
            }

            msg = rx.next() => {
                let Some(Ok(msg)) = msg else { break };
                last_traffic = Instant::now();
                match msg {
                    Message::Text(text) => {
                        let sig = handle_text(conn_id, text.as_str(), &state).await;
                        if let Some(sig) = sig {
                            let _ = tx
                                .send(Message::Close(Some(CloseFrame {
                                    code: sig.code,
                                    reason: sig.reason.into(),
                                })))
                                .await;
                            break;
                        }
                    }
                    Message::Binary(_) => {
                        let _ = tx
                            .send(Message::Close(Some(CloseFrame {
                                code: close_code::PROTOCOL_ERROR,
                                reason: "binary frames are not part of this protocol".into(),
                            })))
                            .await;
                        break;
                    }
                    Message::Close(_) => break,
                    // Pings are answered by the ws layer; both directions
                    // count as traffic for the idle deadline.
                    Message::Ping(_) | Message::Pong(_) => {}
                }
            }

            _ = ping.tick() => {
                if last_traffic.elapsed() > state.idle_timeout {
                    let _ = tx
                        .send(Message::Close(Some(CloseFrame {
                            code: 1001,
                            reason: "idle timeout".into(),
                        })))
                        .await;
                    break;
                }
                if tx.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }

    // Closing handshake: give the peer a moment to read our close frame —
    // dropping the socket immediately can RST and discard it unread.
    let _ = timeout(Duration::from_millis(250), async {
        while let Some(Ok(_)) = rx.next().await {}
    })
    .await;

    let _ = state.inbound_tx.send(ConnIn::Closed { conn_id }).await;
}

/// Decode one inbound text frame; forward to the hub. Returns a close
/// signal for protocol violations.
async fn handle_text(conn_id: u64, text: &str, state: &AppState) -> Option<CloseSignal> {
    if text.len() > state.max_frame_bytes {
        return Some(CloseSignal { code: close_code::FRAME_TOO_LARGE, reason: "frame too large" });
    }
    match decode_client_frame(text) {
        Ok(ClientDecode::Frame(envelope)) => {
            let _ = state.inbound_tx.send(ConnIn::Command { conn_id, envelope }).await;
            None
        }
        Ok(ClientDecode::UnknownType { frame_type, command_id }) => {
            let _ = state
                .inbound_tx
                .send(ConnIn::UnknownCommand { conn_id, frame_type, command_id })
                .await;
            None
        }
        Err(_) => Some(CloseSignal { code: close_code::PROTOCOL_ERROR, reason: "malformed frame" }),
    }
}

async fn read_client_hello(socket: &mut WebSocket, state: &AppState) -> HelloOutcome {
    loop {
        match socket.recv().await {
            Some(Ok(Message::Text(text))) => {
                if text.len() > state.max_frame_bytes {
                    return HelloOutcome::Close(CloseSignal {
                        code: close_code::FRAME_TOO_LARGE,
                        reason: "frame too large",
                    });
                }
                return match decode_client_frame(text.as_str()) {
                    Ok(ClientDecode::Frame(env)) => match env.frame {
                        ClientFrame::ClientHello(hello) => HelloOutcome::Ok(hello),
                        _ => HelloOutcome::Close(CloseSignal {
                            code: close_code::PROTOCOL_ERROR,
                            reason: "client_hello must be the first frame",
                        }),
                    },
                    _ => HelloOutcome::Close(CloseSignal {
                        code: close_code::PROTOCOL_ERROR,
                        reason: "client_hello must be the first frame",
                    }),
                };
            }
            Some(Ok(Message::Binary(_))) => {
                return HelloOutcome::Close(CloseSignal {
                    code: close_code::PROTOCOL_ERROR,
                    reason: "binary frames are not part of this protocol",
                });
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return HelloOutcome::Gone,
        }
    }
}

async fn send_close(socket: &mut WebSocket, sig: CloseSignal) -> Result<(), axum::Error> {
    let sent = socket
        .send(Message::Close(Some(CloseFrame { code: sig.code, reason: sig.reason.into() })))
        .await;
    // Same RST-avoidance drain as the main loop's exit path.
    let _ = timeout(Duration::from_millis(250), async {
        while let Some(Ok(_)) = socket.recv().await {}
    })
    .await;
    sent
}
