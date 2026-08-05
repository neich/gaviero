//! The `RemoteHub` actor (Plan §2.3, §6): sole owner of the active socket
//! generation, monotonic `seq`, per-conversation chunk buffers, flush
//! timers, the bounded outbound path, and newest-wins eviction. It never
//! receives `&App` or anything that can mutate UI state.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, interval};

use super::conn::{CloseSignal, ConnIn, ConnOut, Registration};
use super::RemoteServerConfig;
use crate::close_code;
use crate::dto::{ErrorCode, Hello};
use crate::envelope::{
    ClientEnvelope, ClientFrame, CommandError, ServerEnvelope, ServerFrame, Snapshot,
};
use crate::version::PROTOCOL_VERSION;

/// ~50 ms chunk/snapshot coalescing window (locked product decision).
const FLUSH_INTERVAL: Duration = Duration::from_millis(50);
/// Per-conversation chunk buffer bound: flush immediately over this.
const CHUNK_BUFFER_FLUSH_BYTES: usize = 64 * 1024;
/// Recent command-id dedupe cache bound (LRU eviction).
const RECENT_COMMAND_IDS: usize = 1024;

/// Host → hub. Sent with `try_send` only; a full channel means the host
/// marks itself snapshot-dirty and moves on (invariant 11).
#[derive(Debug)]
pub enum HubInput {
    /// One projected lifecycle event, stamped with the global revision at
    /// emission time. Chunks are coalesced; everything else keeps order.
    Event { revision: u64, frame: ServerFrame },
    /// Full snapshot. Replaces any queued-but-unsent snapshot (§6.3).
    Snapshot(Box<Snapshot>),
    /// Desktop-only rotation (§3.4): swap the accepted token and close the
    /// live client with 4006.
    TokenRotated { new_token: String },
    /// TUI shutdown: close with 4007 and stop the listener.
    Shutdown,
}

/// Hub → host.
#[derive(Debug)]
pub enum HubOutput {
    /// Decoded, deduplicated, rate-limited command for the TUI reducers.
    Command(Box<ClientEnvelope>),
    /// A client completed the handshake (previous one, if any, was evicted).
    ClientConnected,
    ClientDisconnected,
    /// The new client needs a full snapshot (connect path).
    SnapshotNeeded,
}

struct ActiveConn {
    conn_id: u64,
    outbound_tx: mpsc::Sender<ConnOut>,
    close_tx: watch::Sender<Option<CloseSignal>>,
}

pub(crate) struct RemoteHub {
    instance_id: String,
    hello: Hello,
    token: Arc<Mutex<String>>,
    rate_per_second: u32,

    registration_rx: mpsc::Receiver<Registration>,
    inbound_rx: mpsc::Receiver<ConnIn>,
    input_rx: mpsc::Receiver<HubInput>,
    output_tx: mpsc::Sender<HubOutput>,
    axum_handle: axum_server::Handle,

    seq: u64,
    revision: u64,
    active: Option<ActiveConn>,
    /// Insertion-ordered (conv_id, turn_id) → buffered chunk text.
    chunk_bufs: Vec<((String, String), String)>,
    pending_snapshot: Option<Box<Snapshot>>,
    recent_cmds: VecDeque<String>,
    recent_cmds_set: HashSet<String>,
    bucket_tokens: f64,
    bucket_refilled: Instant,
}

impl RemoteHub {
    pub(crate) fn new(
        config: RemoteServerConfig,
        token: Arc<Mutex<String>>,
        registration_rx: mpsc::Receiver<Registration>,
        inbound_rx: mpsc::Receiver<ConnIn>,
        input_rx: mpsc::Receiver<HubInput>,
        output_tx: mpsc::Sender<HubOutput>,
        axum_handle: axum_server::Handle,
    ) -> Self {
        let rate = config.limits.command_rate_per_second;
        let hello = Hello {
            protocol_version: PROTOCOL_VERSION,
            instance_id: config.instance_id.clone(),
            tui_version: config.tui_version,
            workspace: config.workspace,
            capabilities: config.capabilities,
            confirm_required: config.confirm_required,
            allowed_slash_commands: config.allowed_slash_commands,
            limits: config.limits,
        };
        Self {
            instance_id: config.instance_id,
            hello,
            token,
            rate_per_second: rate,
            registration_rx,
            inbound_rx,
            input_rx,
            output_tx,
            axum_handle,
            seq: 0,
            revision: 0,
            active: None,
            chunk_bufs: Vec::new(),
            pending_snapshot: None,
            recent_cmds: VecDeque::new(),
            recent_cmds_set: HashSet::new(),
            bucket_tokens: rate as f64,
            bucket_refilled: Instant::now(),
        }
    }

    pub(crate) async fn run(mut self) {
        let mut flush = interval(FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                reg = self.registration_rx.recv() => {
                    let Some(reg) = reg else { break };
                    self.on_registration(reg).await;
                }
                cin = self.inbound_rx.recv() => {
                    let Some(cin) = cin else { break };
                    self.on_inbound(cin).await;
                }
                input = self.input_rx.recv() => {
                    // A dropped handle is a shutdown.
                    let input = input.unwrap_or(HubInput::Shutdown);
                    if self.on_input(input).await {
                        break;
                    }
                }
                _ = flush.tick() => self.flush_all(),
            }
        }
    }

    async fn on_registration(&mut self, reg: Registration) {
        // Newest authenticated + version-compatible client wins (§3.5).
        if self.active.take().is_some_and(|old| {
            old.close_tx
                .send(Some(CloseSignal { code: close_code::REPLACED, reason: "replaced" }))
                .is_ok()
        }) {
            let _ = self.output_tx.send(HubOutput::ClientDisconnected).await;
        }
        // Stale buffered stream data predates the snapshot the new client
        // is about to receive.
        self.chunk_bufs.clear();
        tracing::info!(client = %reg.client_hello.client_name, "remote client connected");
        self.active = Some(ActiveConn {
            conn_id: reg.conn_id,
            outbound_tx: reg.outbound_tx,
            close_tx: reg.close_tx,
        });
        self.emit(ServerFrame::Hello(self.hello.clone()));
        let _ = self.output_tx.send(HubOutput::ClientConnected).await;
        let _ = self.output_tx.send(HubOutput::SnapshotNeeded).await;
    }

    async fn on_inbound(&mut self, cin: ConnIn) {
        match cin {
            ConnIn::Closed { conn_id } => {
                if self.active.as_ref().is_some_and(|a| a.conn_id == conn_id) {
                    self.active = None;
                    let _ = self.output_tx.send(HubOutput::ClientDisconnected).await;
                }
            }
            ConnIn::UnknownCommand { conn_id, frame_type, command_id } => {
                if !self.is_active(conn_id) {
                    return;
                }
                tracing::warn!(frame_type, "unknown client command type");
                self.emit_command_error(
                    command_id.unwrap_or_default(),
                    ErrorCode::UnknownType,
                    "unknown command type",
                );
            }
            ConnIn::Command { conn_id, envelope } => {
                if !self.is_active(conn_id) {
                    return;
                }
                self.on_command(*envelope).await;
            }
        }
    }

    async fn on_command(&mut self, envelope: ClientEnvelope) {
        let command_id = envelope.command_id.clone();

        // A post-hello frame must echo our instance id; a second
        // client_hello is likewise a payload error, not a new handshake.
        let instance_ok = envelope.instance_id.as_deref() == Some(self.instance_id.as_str());
        if !instance_ok || matches!(envelope.frame, ClientFrame::ClientHello(_)) {
            self.emit_command_error(
                command_id,
                ErrorCode::InvalidPayload,
                "bad instance_id or repeated client_hello",
            );
            return;
        }

        // Dedupe: a retry after a flaky send is safe — the original
        // response was already correlated, so repeats are dropped silently.
        if self.recent_cmds_set.contains(&command_id) {
            tracing::debug!(%command_id, "duplicate command dropped");
            return;
        }
        self.recent_cmds_set.insert(command_id.clone());
        self.recent_cmds.push_back(command_id.clone());
        if self.recent_cmds.len() > RECENT_COMMAND_IDS
            && let Some(evicted) = self.recent_cmds.pop_front()
        {
            self.recent_cmds_set.remove(&evicted);
        }

        // Token-bucket rate limit (§5.2).
        if !self.take_rate_token() {
            self.emit_command_error(command_id, ErrorCode::RateLimited, "command rate exceeded");
            return;
        }

        if self
            .output_tx
            .try_send(HubOutput::Command(Box::new(envelope)))
            .is_err()
        {
            // Host command queue full: reject rather than buffer without
            // bound (§6.1 "reject/close abusive client").
            self.emit_command_error(command_id, ErrorCode::RateLimited, "command queue full");
        }
    }

    /// Returns true when the hub should stop.
    async fn on_input(&mut self, input: HubInput) -> bool {
        match input {
            HubInput::Event { revision, frame } => {
                self.revision = revision.max(self.revision);
                match frame {
                    ServerFrame::StreamChunk(chunk) => {
                        let key = (chunk.conv_id, chunk.turn_id);
                        if let Some((_, buf)) =
                            self.chunk_bufs.iter_mut().find(|(k, _)| *k == key)
                        {
                            buf.push_str(&chunk.text);
                        } else {
                            self.chunk_bufs.push((key.clone(), chunk.text));
                        }
                        let over = self
                            .chunk_bufs
                            .iter()
                            .find(|(k, _)| *k == key)
                            .is_some_and(|(_, b)| b.len() > CHUNK_BUFFER_FLUSH_BYTES);
                        if over {
                            self.flush_conversation(&key.0);
                        }
                    }
                    frame => {
                        // Order rule (§6.2): a non-chunk event flushes the
                        // chunk run that must precede it.
                        match frame_conv_id(&frame) {
                            Some(conv) => self.flush_conversation(&conv.to_string()),
                            None => self.flush_chunks(),
                        }
                        self.emit(frame);
                    }
                }
                false
            }
            HubInput::Snapshot(snapshot) => {
                // Latest snapshot wins; emitted on the flush tick (§6.3).
                self.pending_snapshot = Some(snapshot);
                false
            }
            HubInput::TokenRotated { new_token } => {
                *self.token.lock().expect("token lock") = new_token;
                self.close_active(CloseSignal {
                    code: close_code::TOKEN_ROTATED,
                    reason: "token rotated",
                })
                .await;
                false
            }
            HubInput::Shutdown => {
                self.close_active(CloseSignal {
                    code: close_code::SERVER_SHUTDOWN,
                    reason: "server shutdown",
                })
                .await;
                self.axum_handle
                    .graceful_shutdown(Some(Duration::from_millis(250)));
                true
            }
        }
    }

    fn is_active(&self, conn_id: u64) -> bool {
        self.active.as_ref().is_some_and(|a| a.conn_id == conn_id)
    }

    async fn close_active(&mut self, sig: CloseSignal) {
        if let Some(old) = self.active.take() {
            let _ = old.close_tx.send(Some(sig));
            let _ = self.output_tx.send(HubOutput::ClientDisconnected).await;
        }
    }

    fn take_rate_token(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.bucket_refilled).as_secs_f64();
        self.bucket_refilled = now;
        let cap = self.rate_per_second as f64;
        self.bucket_tokens = (self.bucket_tokens + elapsed * cap).min(cap);
        if self.bucket_tokens >= 1.0 {
            self.bucket_tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn flush_all(&mut self) {
        self.flush_chunks();
        if let Some(snapshot) = self.pending_snapshot.take() {
            self.revision = snapshot.revision.max(self.revision);
            self.emit(ServerFrame::Snapshot(*snapshot));
        }
    }

    fn flush_chunks(&mut self) {
        for ((conv_id, turn_id), text) in std::mem::take(&mut self.chunk_bufs) {
            self.emit(ServerFrame::StreamChunk(crate::envelope::StreamChunk {
                conv_id,
                turn_id,
                text,
            }));
        }
    }

    /// Flush only the given conversation's buffered runs, preserving their
    /// relative order. Chunks from concurrent conversations stay buffered
    /// (never merged, never reordered within their own conversation).
    fn flush_conversation(&mut self, conv_id: &str) {
        let (flush, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.chunk_bufs)
            .into_iter()
            .partition(|((c, _), _)| c == conv_id);
        self.chunk_bufs = keep;
        for ((conv_id, turn_id), text) in flush {
            self.emit(ServerFrame::StreamChunk(crate::envelope::StreamChunk {
                conv_id,
                turn_id,
                text,
            }));
        }
    }

    fn emit_command_error(&mut self, command_id: String, code: ErrorCode, message: &str) {
        self.emit(ServerFrame::CommandError(CommandError {
            command_id,
            code,
            message: message.to_string(),
        }));
    }

    /// Serialize and queue one frame on the active socket. `seq` is
    /// assigned only here — only frames actually emitted consume sequence
    /// numbers, and the counter continues across socket generations. A full
    /// outbound queue is a slow client: close 4008; it reconnects and
    /// resnapshots.
    fn emit(&mut self, frame: ServerFrame) {
        let Some(active) = &self.active else { return };
        let envelope = ServerEnvelope {
            version: PROTOCOL_VERSION,
            instance_id: self.instance_id.clone(),
            seq: self.seq + 1,
            revision: self.revision,
            frame,
        };
        let text = match serde_json::to_string(&envelope) {
            Ok(t) => t,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize server frame");
                return;
            }
        };
        match active.outbound_tx.try_send(ConnOut::Frame(text)) {
            Ok(()) => self.seq += 1,
            Err(_) => {
                tracing::warn!("outbound queue full — closing slow client");
                if let Some(old) = self.active.take() {
                    let _ = old.close_tx.send(Some(CloseSignal {
                        code: close_code::SLOW_CLIENT,
                        reason: "slow client",
                    }));
                    // try_send: never block the actor on the host channel.
                    let _ = self.output_tx.try_send(HubOutput::ClientDisconnected);
                }
            }
        }
    }
}

/// Conversation a frame belongs to, for the chunk-ordering rule. `None`
/// (hello, snapshot, command results/errors) conservatively flushes all.
fn frame_conv_id(frame: &ServerFrame) -> Option<&str> {
    use ServerFrame as F;
    match frame {
        F::StreamChunk(x) => Some(&x.conv_id),
        F::StreamingStatus(x) => Some(&x.conv_id),
        F::StreamingEnded(x) => Some(&x.conv_id),
        F::ToolCallStarted(x) => Some(&x.conv_id),
        F::MessageComplete(x) => Some(&x.conv_id),
        F::MessagePage(x) => Some(&x.conv_id),
        F::PermissionRequest(x) => Some(&x.conv_id),
        F::PermissionClosed(x) => Some(&x.conv_id),
        F::ConversationStateChanged(x) => Some(&x.conversation.conv_id),
        F::ConversationRemoved(x) => Some(&x.conv_id),
        F::TokenUsage(x) => Some(&x.conv_id),
        F::CostUpdate(x) => Some(&x.conv_id),
        F::ProposalCreated(x) | F::ProposalUpdated(x) | F::ProposalDetail(x) => {
            x.proposal.conv_id.as_deref()
        }
        F::Hello(_)
        | F::Snapshot(_)
        | F::ProposalFinalized(_)
        | F::CommandResult(_)
        | F::CommandError(_) => None,
    }
}
