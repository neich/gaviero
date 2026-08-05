//! A1 verification: fixture round-trips, schema equality, forward-compat
//! decoding, major rejection, and the UTF-8 offset contract of the
//! `message_complete` fixture.

use std::fs;
use std::path::{Path, PathBuf};

use gaviero_remote::envelope::{
    ClientDecode, ClientEnvelope, ServerDecode, ServerEnvelope, decode_client_frame,
    decode_server_frame,
};
use gaviero_remote::version::{PROTOCOL_VERSION, ProtocolVersion};
use serde_json::Value;

fn fixture_dir(side: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(side)
}

fn fixtures(side: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = fs::read_dir(fixture_dir(side))
        .expect("fixture dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).expect("read fixture"))
        })
        .collect();
    out.sort();
    out
}

/// Every client fixture decodes, re-encodes to the identical JSON value
/// (field names and enum values — ordering-independent), and round-trips
/// through the typed envelope.
#[test]
fn client_fixtures_round_trip() {
    let fixtures = fixtures("client");
    assert_eq!(fixtures.len(), 13, "one fixture per client frame type");
    for (name, text) in fixtures {
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let env: ClientEnvelope = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
        let back = serde_json::to_value(&env).unwrap();
        assert_eq!(back, parsed, "{name}: re-encoded JSON differs");
        let again: ClientEnvelope = serde_json::from_value(back).unwrap();
        assert_eq!(again, env, "{name}: typed round-trip differs");
    }
}

#[test]
fn server_fixtures_round_trip() {
    let fixtures = fixtures("server");
    assert_eq!(fixtures.len(), 20, "one fixture per server frame type");
    for (name, text) in fixtures {
        let parsed: Value = serde_json::from_str(&text).unwrap();
        let env: ServerEnvelope = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{name}: decode failed: {e}"));
        let back = serde_json::to_value(&env).unwrap();
        assert_eq!(back, parsed, "{name}: re-encoded JSON differs");
    }
}

/// An unknown server event (a future minor version) is ignorable, never an
/// error — and an unknown client command is classified so the server can
/// answer `unknown_type` with the right `command_id`.
#[test]
fn unknown_frame_types_are_classified_not_fatal() {
    let json = r#"{
        "version": {"major":1,"minor":1},
        "instance_id": "a3f9c2e14b7d8650",
        "seq": 99, "revision": 20,
        "type": "sparkline_update",
        "payload": { "points": [1,2,3] }
    }"#;
    match decode_server_frame(json).expect("classified, not an error") {
        ServerDecode::UnknownType { frame_type } => assert_eq!(frame_type, "sparkline_update"),
        other => panic!("expected UnknownType, got {other:?}"),
    }

    let json = r#"{
        "version": {"major":1,"minor":1},
        "instance_id": "a3f9c2e14b7d8650",
        "command_id": "cmd-77",
        "type": "rotate_token",
        "payload": {}
    }"#;
    match decode_client_frame(json).expect("classified, not an error") {
        ClientDecode::UnknownType { frame_type, command_id } => {
            assert_eq!(frame_type, "rotate_token");
            assert_eq!(command_id.as_deref(), Some("cmd-77"));
        }
        other => panic!("expected UnknownType, got {other:?}"),
    }
}

/// A malformed payload for a *known* type stays an error — it must not be
/// silently swallowed as forward-compat.
#[test]
fn malformed_known_payload_is_an_error() {
    let json = r#"{
        "version": {"major":1,"minor":0},
        "instance_id": "a3f9c2e14b7d8650",
        "command_id": "cmd-78",
        "type": "send_prompt",
        "payload": { "conv_id": 7 }
    }"#;
    assert!(decode_client_frame(json).is_err());
}

/// Unknown payload fields (minor-version additions) are ignored.
#[test]
fn unknown_payload_fields_are_ignored() {
    let text = fs::read_to_string(fixture_dir("server").join("hello.json")).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["payload"]["shiny_new_field"] = Value::from("ignored");
    v["payload"]["workspace"]["color"] = Value::from("teal");
    let env: ServerEnvelope = serde_json::from_value(v).expect("unknown payload fields ignored");
    let gaviero_remote::envelope::ServerFrame::Hello(h) = &env.frame else {
        panic!("expected hello");
    };
    assert_eq!(h.workspace.id, "4b156f1de41da274");
}

#[test]
fn incompatible_major_is_rejected() {
    let theirs = ProtocolVersion { major: 2, minor: 0 };
    assert!(PROTOCOL_VERSION.check_compatible(&theirs).is_err());
    assert!(
        PROTOCOL_VERSION
            .check_compatible(&ProtocolVersion { major: 1, minor: 3 })
            .is_ok()
    );
}

/// PROTOCOL.md promises the message_complete fixture's offsets are valid
/// UTF-8 byte offsets: every offset on a char boundary, the block slice is
/// the fenced block, spans inside the block.
#[test]
fn message_complete_fixture_offsets_are_utf8_correct() {
    let text = fs::read_to_string(fixture_dir("server").join("message_complete.json")).unwrap();
    let env: ServerEnvelope = serde_json::from_str(&text).unwrap();
    let gaviero_remote::envelope::ServerFrame::MessageComplete(mc) = &env.frame else {
        panic!("expected message_complete");
    };
    let content = &mc.message.content;
    assert!(
        content.chars().any(|c| !c.is_ascii()),
        "fixture must contain non-ASCII content — the offset bug is invisible in ASCII"
    );
    for block in &mc.message.code_blocks {
        let (start, end) = (block.start_byte as usize, block.end_byte as usize);
        assert!(content.is_char_boundary(start) && content.is_char_boundary(end));
        let slice = &content[start..end];
        assert!(slice.starts_with("```"), "block range includes the opening fence");
        for span in &block.spans {
            let (s, e) = (span.start_byte as usize, span.end_byte as usize);
            assert!(content.is_char_boundary(s) && content.is_char_boundary(e));
            assert!(s >= start && e <= end, "span inside its block");
        }
    }
}

/// The committed schema matches the DTOs. Regenerate with
/// `cargo test -p gaviero-remote regenerate_schema -- --ignored`.
#[test]
fn schema_matches_committed() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("protocol.schema.json");
    let committed: Value = serde_json::from_str(
        &fs::read_to_string(&path).expect("protocol.schema.json is committed"),
    )
    .unwrap();
    assert_eq!(
        gaviero_remote::schema::protocol_schema(),
        committed,
        "DTOs changed: run `cargo test -p gaviero-remote regenerate_schema -- --ignored` and review the diff"
    );
}

/// Explicit regeneration path — the only test allowed to write the tree.
#[test]
#[ignore = "rewrites protocol.schema.json; run explicitly after a DTO change"]
fn regenerate_schema() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("protocol.schema.json");
    let mut out = serde_json::to_string_pretty(&gaviero_remote::schema::protocol_schema()).unwrap();
    out.push('\n');
    fs::write(&path, out).unwrap();
}
