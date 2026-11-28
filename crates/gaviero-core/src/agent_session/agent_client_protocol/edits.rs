//! Which files an ACP agent's settled tool call edited — the write claims
//! for `agy:` / `dsh:` edits made outside the client `fs` channel — and the
//! reconcile filter that keeps other writers' files out of a swarm agent's.

use std::path::PathBuf;

use serde_json::Value;

use crate::write_claims::{ClaimHandle, Verdict};

/// Drop from a swarm agent's git dirty-set reconcile every path another
/// writer provably wrote since the agent's writer started
/// ([`crate::write_claims::JournalSnapshot::classify`] → `Foreign`). Unchanged
/// without a writer or with `consume` off (`turnReview.attribution: false`).
pub(super) fn drop_claimed_elsewhere(
    paths: Vec<PathBuf>,
    claims: Option<&ClaimHandle>,
    until_ns: i64,
) -> Vec<PathBuf> {
    let Some(h) = claims.filter(|h| h.consume) else {
        return paths;
    };
    if paths.is_empty() {
        return paths;
    }
    let start = h.writer.info().started_ns;
    let snap = h.writer.journal().snapshot(start);
    paths
        .into_iter()
        .filter(|p| {
            let current = crate::turn_capture::store::hash_file(p).ok();
            match snap.classify(h.writer.id(), start, until_ns, p, current.as_deref()) {
                Verdict::Foreign(writer) => {
                    tracing::info!(
                        target: "write_claims",
                        path = %p.display(),
                        writer = %writer,
                        "reconcile: left out a file another writer wrote"
                    );
                    false
                }
                _ => true,
            }
        })
        .collect()
}

/// Raw paths a stored `tool_call` update edits (relative or absolute, as the
/// agent sent them), in order, deduplicated. Shell commands and reads claim
/// nothing.
///
/// - agy: `kind: "edit" | "delete" | "move"` with `locations[*].path`, and
///   `rawInput.Path` / `TargetFile` on edits;
/// - dsh: `kind: "other"`, the tool name in `title` — `write` / `edit`
///   (`file_path`), `str_replace_editor` (`path`, mutating commands only),
///   `apply_patch` (every file named in the patch).
pub(super) fn edit_paths(call: &Value) -> Vec<String> {
    let kind = call.get("kind").and_then(Value::as_str).unwrap_or("");
    let title = call.get("title").and_then(Value::as_str).unwrap_or("");
    let raw = call.get("rawInput").unwrap_or(&Value::Null);
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: &str| {
        let p = p.trim();
        if !p.is_empty() && !out.iter().any(|q| q == p) {
            out.push(p.to_string());
        }
    };

    if matches!(kind, "edit" | "delete" | "move")
        && let Some(locations) = call.get("locations").and_then(Value::as_array)
    {
        for loc in locations {
            if let Some(p) = loc.get("path").and_then(Value::as_str) {
                push(p);
            }
        }
    }
    if kind == "edit" || matches!(title, "write" | "edit" | "write_file" | "edit_file") {
        if let Some(p) = ["Path", "path", "file_path", "filePath", "TargetFile"]
            .iter()
            .find_map(|k| raw.get(*k).and_then(Value::as_str))
        {
            push(p);
        }
    }
    if title == "str_replace_editor"
        && matches!(
            raw.get("command").and_then(Value::as_str),
            Some("create" | "str_replace" | "insert")
        )
        && let Some(p) = raw.get("path").and_then(Value::as_str)
    {
        push(p);
    }
    if title == "apply_patch"
        && let Some(patch) = raw.get("input").and_then(Value::as_str)
    {
        for line in patch.lines() {
            let line = line.trim_start();
            if let Some(p) = [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            {
                push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_claims::{ClaimJournal, FileWrite, WriterKind};
    use serde_json::json;

    /// A journal with a chat writer registered before the swarm agent's.
    struct Fx {
        _tmp: tempfile::TempDir,
        ws: std::path::PathBuf,
        chat: std::sync::Arc<crate::write_claims::ClaimWriter>,
        agent: std::sync::Arc<crate::write_claims::ClaimWriter>,
    }

    fn fx() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let journal = ClaimJournal::at(tmp.path().join("claims"));
        let chat = journal
            .register("chat-1", WriterKind::ChatTurn, "chat 'x'", vec![ws.clone()])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let agent = journal
            .register("swarm-1-r-u1-0", WriterKind::SwarmAgent, "SWARM 'r' · u1", vec![ws.clone()])
            .unwrap();
        Fx {
            _tmp: tmp,
            ws,
            chat,
            agent,
        }
    }

    fn now() -> i64 {
        crate::turn_capture::now_ns()
    }

    #[test]
    fn drop_claimed_elsewhere_removes_foreign_claimed_paths() {
        let f = fx();
        let x = f.ws.join("x.txt");
        let y = f.ws.join("y.txt");
        std::fs::write(&x, "chat").unwrap();
        std::fs::write(&y, "agent").unwrap();
        f.chat.record(&FileWrite::unknown(&x));
        let h = ClaimHandle {
            writer: f.agent.clone(),
            consume: true,
        };
        assert_eq!(
            drop_claimed_elsewhere(vec![x, y.clone()], Some(&h), now()),
            vec![y]
        );
    }

    #[test]
    fn drop_claimed_elsewhere_keeps_everything_when_consume_is_off() {
        let f = fx();
        let x = f.ws.join("x.txt");
        std::fs::write(&x, "chat").unwrap();
        f.chat.record(&FileWrite::unknown(&x));
        let h = ClaimHandle {
            writer: f.agent.clone(),
            consume: false,
        };
        assert_eq!(
            drop_claimed_elsewhere(vec![x.clone()], Some(&h), now()),
            vec![x.clone()]
        );
        assert_eq!(drop_claimed_elsewhere(vec![x.clone()], None, now()), vec![x]);
    }

    #[test]
    fn drop_claimed_elsewhere_keeps_own_claimed_paths() {
        let f = fx();
        let x = f.ws.join("x.txt");
        std::fs::write(&x, "both").unwrap();
        // Both claimed it: kept (the agent's own claim wins over a drop).
        f.chat.record(&FileWrite::unknown(&x));
        f.agent.record(&FileWrite::unknown(&x));
        let h = ClaimHandle {
            writer: f.agent.clone(),
            consume: true,
        };
        assert_eq!(
            drop_claimed_elsewhere(vec![x.clone()], Some(&h), now()),
            vec![x]
        );
    }

    #[test]
    fn agy_edit_kind_uses_raw_path() {
        let call = json!({
            "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "File hello.txt",
            "kind": "edit", "rawInput": { "Path": "hello.txt", "Content": "hi" },
            "locations": [{ "path": "/ws/hello.txt" }]
        });
        assert_eq!(edit_paths(&call), vec!["/ws/hello.txt", "hello.txt"]);
        let delete = json!({ "kind": "delete", "locations": [{ "path": "gone.txt" }] });
        assert_eq!(edit_paths(&delete), vec!["gone.txt"]);
        let target = json!({ "kind": "edit", "rawInput": { "TargetFile": "/ws/t.rs" } });
        assert_eq!(edit_paths(&target), vec!["/ws/t.rs"]);
    }

    #[test]
    fn dsh_write_and_edit_use_file_path() {
        let write = json!({ "kind": "other", "title": "write", "rawInput": { "file_path": "note.txt", "content": "hi" } });
        assert_eq!(edit_paths(&write), vec!["note.txt"]);
        let edit = json!({ "kind": "other", "title": "edit", "rawInput": { "file_path": "src/a.rs", "old_string": "a", "new_string": "b" } });
        assert_eq!(edit_paths(&edit), vec!["src/a.rs"]);
        let sre = json!({ "kind": "other", "title": "str_replace_editor", "rawInput": { "command": "str_replace", "path": "b.rs" } });
        assert_eq!(edit_paths(&sre), vec!["b.rs"]);
    }

    #[test]
    fn dsh_apply_patch_lists_every_file() {
        let patch = "*** Begin Patch\n*** Add File: new.rs\n+fn a() {}\n*** Update File: old.rs\n*** Move to: moved.rs\n@@\n-x\n+y\n*** Delete File: dead.rs\n*** Update File: old.rs\n*** End Patch\n";
        let call = json!({ "kind": "other", "title": "apply_patch", "rawInput": { "input": patch } });
        assert_eq!(
            edit_paths(&call),
            vec!["new.rs", "old.rs", "moved.rs", "dead.rs"]
        );
    }

    #[test]
    fn str_replace_editor_view_claims_nothing() {
        let call = json!({ "kind": "other", "title": "str_replace_editor", "rawInput": { "command": "view", "path": "a.rs" } });
        assert!(edit_paths(&call).is_empty());
    }

    #[test]
    fn unknown_tool_claims_nothing() {
        for call in [
            json!({ "kind": "execute", "title": "npm install", "rawInput": { "CommandLine": "npm install" } }),
            json!({ "kind": "other", "title": "bash", "rawInput": { "command": "echo > x", "file_path": "x" } }),
            json!({ "kind": "read", "title": "Read", "rawInput": { "file_path": "README.md" } }),
            json!({ "kind": "other", "title": "read", "locations": [{ "path": "a.rs" }] }),
        ] {
            assert!(edit_paths(&call).is_empty(), "{call}");
        }
    }
}
