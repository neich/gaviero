//! JSON Schema generation. Rust is the source of truth: the committed
//! `protocol.schema.json` is generated from the DTOs and compared by test.
//! Regenerate with:
//! `cargo test -p gaviero-remote regenerate_schema -- --ignored`

use serde_json::{Value, json};

/// The complete machine-readable protocol contract: both envelope schemas
/// plus the wire version, in one stable document.
pub fn protocol_schema() -> Value {
    let client = schemars::schema_for!(crate::envelope::ClientEnvelope);
    let server = schemars::schema_for!(crate::envelope::ServerEnvelope);
    json!({
        "$comment": "Generated from gaviero-remote DTOs — do not edit by hand.",
        "protocol_version": crate::version::PROTOCOL_VERSION,
        "ws_path": crate::WS_PATH,
        "subprotocol": crate::SUBPROTOCOL,
        "client_envelope": client,
        "server_envelope": server,
    })
}
