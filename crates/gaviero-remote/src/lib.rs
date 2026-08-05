//! Gaviero Remote protocol: wire DTOs, envelopes, and (feature `server`)
//! the WSS sidecar server.
//!
//! The normative contract is [`PROTOCOL.md`](../PROTOCOL.md); the committed
//! `protocol.schema.json` is generated from these types (Rust is the source
//! of truth). This crate must never depend on `gaviero-core` or
//! `gaviero-tui` — DTOs are remote-owned.

pub mod dto;
pub mod envelope;
pub mod schema;
pub mod version;

#[cfg(feature = "server")]
pub mod server;

pub use envelope::{ClientEnvelope, ClientFrame, ServerEnvelope, ServerFrame};
pub use version::{PROTOCOL_VERSION, ProtocolVersion};

/// WebSocket path the sidecar serves.
pub const WS_PATH: &str = "/v1/ws";
/// Required WebSocket subprotocol.
pub const SUBPROTOCOL: &str = "gaviero.v1";

/// Application close codes (4000–4999). Committed in PROTOCOL.md.
pub mod close_code {
    pub const UNAUTHORIZED: u16 = 4001;
    pub const UNSUPPORTED_VERSION: u16 = 4002;
    pub const PROTOCOL_ERROR: u16 = 4003;
    pub const FRAME_TOO_LARGE: u16 = 4004;
    pub const REPLACED: u16 = 4005;
    pub const TOKEN_ROTATED: u16 = 4006;
    pub const SERVER_SHUTDOWN: u16 = 4007;
    pub const SLOW_CLIENT: u16 = 4008;
}
