//! gaviero-relay wire protocol v1 — the ONE definition shared by the relay
//! (`crates/gaviero-relay`) and the host-side client
//! ([`crate::relay_client`], feature `server`). Normative prose:
//! `crates/gaviero-relay/PROTOCOL.md`.
//!
//! * Host control: WebSocket `GET /v1/host`, JSON text frames
//!   ([`HostMsg`] up, [`RelayMsg`] down).
//! * Host data: WebSocket `GET /v1/data/{nonce}`, binary frames carrying
//!   the phone's raw TLS bytes in both directions.
//! * Phone notifications: `GET /v1/phone/stream?since=<id>&epoch=<e>`, one
//!   [`PhoneEvent`] JSON object per line (NDJSON), keepalive lines included.
//! * Phone live control: raw TLS on the relay's splice port, routed by SNI.
//!
//! Every HTTP/WebSocket endpoint requires `Authorization: Bearer <secret>`.
//! Plain serde types: available without the `server` feature.

use serde::{Deserialize, Serialize};

/// Path prefix; bump on incompatible changes.
pub const VERSION_PREFIX: &str = "/v1";

/// What a phone notification is about. The phone renders the text from
/// names it already knows; the relay never sees prompts or answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    AgentFinished,
    AgentWaiting,
}

/// One routable gaviero instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteInfo {
    /// Full TLS server name the phone uses, e.g.
    /// `0b7d245998c0e8c3-desk.gaviero.example.com`. Never resolved in DNS.
    pub route: String,
    pub machine: String,
    pub workspace_id: String,
    pub display_name: String,
}

/// Host → relay, on the control WebSocket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMsg {
    /// Claim a route. The newest registration of a route wins.
    Register {
        #[serde(flatten)]
        info: RouteInfo,
    },
    Unregister { route: String },
    /// Queue a notification for every phone.
    Notify {
        machine: String,
        workspace_id: String,
        conv: String,
        kind: NotifyKind,
    },
}

/// Relay → host, on the control WebSocket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayMsg {
    Registered { route: String },
    Rejected { route: String, reason: String },
    /// A phone is waiting on this route: open `GET /v1/data/{nonce}`.
    Dial { route: String, nonce: String },
    Error { message: String },
}

/// Relay → phone, one per line on `/v1/phone/stream`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PhoneEvent {
    Notify {
        /// Monotonic within one relay epoch; pass the last seen id as `since`.
        id: u64,
        /// Unix seconds when the relay received it.
        ts: u64,
        machine: String,
        workspace_id: String,
        conv: String,
        kind: NotifyKind,
    },
    /// Every currently registered route (sent on connect and on change).
    /// `epoch` changes every time the relay process starts: notification
    /// ids restart at 1, so a phone that sees a new epoch must forget its
    /// stored `since` (and pass `epoch` back so the relay ignores a stale one).
    Presence { epoch: String, routes: Vec<RouteInfo> },
    Keepalive {},
}

/// A route is a lowercase DNS name: labels of `[a-z0-9-]`, 1–63 chars,
/// total ≤ 253. When the relay knows its public host, the route must be a
/// direct child of it (`<label>.<public_host>`) and never the host itself.
pub fn validate_route(route: &str, public_host: Option<&str>) -> Result<(), String> {
    if route.is_empty() || route.len() > 253 {
        return Err("route must be 1-253 characters".into());
    }
    for label in route.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !ok {
            return Err(format!("invalid DNS label {label:?}"));
        }
    }
    if let Some(host) = public_host {
        let Some(label) = route.strip_suffix(host).and_then(|p| p.strip_suffix('.')) else {
            return Err(format!("route must be <label>.{host}"));
        };
        if label.contains('.') {
            return Err(format!("route must be a single label under {host}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_shapes_are_stable() {
        let reg = HostMsg::Register {
            info: RouteInfo {
                route: "abc-desk.relay.test".into(),
                machine: "desk".into(),
                workspace_id: "abc".into(),
                display_name: "gaviero".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&reg).unwrap(),
            serde_json::json!({"type":"register","route":"abc-desk.relay.test","machine":"desk","workspace_id":"abc","display_name":"gaviero"})
        );
        let note = HostMsg::Notify {
            machine: "desk".into(),
            workspace_id: "abc".into(),
            conv: "c1".into(),
            kind: NotifyKind::AgentWaiting,
        };
        assert_eq!(
            serde_json::to_value(&note).unwrap(),
            serde_json::json!({"type":"notify","machine":"desk","workspace_id":"abc","conv":"c1","kind":"agent_waiting"})
        );
        let dial: RelayMsg =
            serde_json::from_str(r#"{"type":"dial","route":"r","nonce":"n"}"#).unwrap();
        assert_eq!(dial, RelayMsg::Dial { route: "r".into(), nonce: "n".into() });
        assert_eq!(
            serde_json::to_string(&PhoneEvent::Keepalive {}).unwrap(),
            r#"{"type":"keepalive"}"#
        );
        assert_eq!(
            serde_json::to_value(PhoneEvent::Presence { epoch: "e".into(), routes: vec![] }).unwrap(),
            serde_json::json!({"type":"presence","epoch":"e","routes":[]})
        );
    }

    #[test]
    fn routes_are_validated() {
        let host = Some("gaviero.example.com");
        assert!(validate_route("abc-desk.gaviero.example.com", host).is_ok());
        assert!(validate_route("gaviero.example.com", host).is_err());
        assert!(validate_route("a.b.gaviero.example.com", host).is_err());
        assert!(validate_route("abc-desk.evil.com", host).is_err());
        assert!(validate_route("UPPER.example.com", None).is_err());
        assert!(validate_route("-bad.example.com", None).is_err());
        assert!(validate_route("ok.example.com", None).is_ok());
    }
}
