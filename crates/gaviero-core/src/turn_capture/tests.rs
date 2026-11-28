use super::*;

struct Ws {
    _dir: tempfile::TempDir,
    root: PathBuf,
    cap: Arc<TurnCapture>,
}

fn ws() -> Ws {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("ws");
    std::fs::create_dir_all(&root).unwrap();
    let cap = TurnCapture::for_workspace(&root);
    Ws {
        _dir: dir,
        root,
        cap,
    }
}

impl Ws {
    fn scope(&self) -> CaptureScope {
        CaptureScope {
            roots: vec![self.root.clone()],
            ..CaptureScope::default()
        }
    }

    fn write(&self, rel: &str, body: &str) {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn read(&self, rel: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(rel)).ok()
    }

    fn turn(&self, id: &str, edit: impl FnOnce(&Ws)) -> TurnChangeSet {
        let h = self.cap.begin(id, Some("conv"), self.scope()).unwrap();
        edit(self);
        self.cap.end(h, TurnOutcome::Completed).unwrap().set
    }
}

fn kinds(set: &TurnChangeSet) -> Vec<(String, ChangeKind)> {
    set.files.iter().map(|c| (c.rel.clone(), c.kind)).collect()
}

/// Make the blob store unwritable: a plain file where `objects/` must go.
fn break_store(w: &Ws) {
    let objects = w.root.join(".gaviero/turns/objects");
    let _ = std::fs::remove_dir_all(&objects);
    std::fs::create_dir_all(objects.parent().unwrap()).unwrap();
    std::fs::write(&objects, "not a directory").unwrap();
}

#[test]
fn begin_fails_when_the_blob_store_cannot_be_written() {
    let w = ws();
    w.write("a.txt", "v1\n");
    break_store(&w);
    // An error sends the caller to the Write Gate; an Ok with `a.txt`
    // missing from the baseline would let edits to it land unreviewed.
    assert!(w.cap.begin("t1", Some("conv"), w.scope()).is_err());
}

#[test]
fn a_store_that_breaks_mid_turn_still_reports_the_change() {
    let w = ws();
    w.write("a.txt", "v1\n");
    let h = w.cap.begin("t1", Some("conv"), w.scope()).unwrap();
    w.write("a.txt", "v2\n");
    break_store(&w);
    let set = w.cap.end(h, TurnOutcome::Completed).unwrap().set;
    assert_eq!(kinds(&set), vec![("a.txt".into(), ChangeKind::Modified)]);
    assert!(set.warnings.iter().any(|m| m.contains("without its content")));
}

#[test]
fn detects_add_modify_delete_rename_made_by_anything() {
    let w = ws();
    w.write("keep.txt", "same\n");
    w.write("mod.txt", "before\n");
    w.write("del.txt", "doomed\n");
    w.write("old.txt", "moved\n");

    let set = w.turn("t1", |w| {
        // What a shell command would do — no tool channel involved.
        w.write("mod.txt", "after\n");
        w.write("new.txt", "fresh\n");
        std::fs::remove_file(w.root.join("del.txt")).unwrap();
        std::fs::rename(w.root.join("old.txt"), w.root.join("moved.txt")).unwrap();
    });

    assert_eq!(
        kinds(&set),
        vec![
            ("del.txt".into(), ChangeKind::Deleted),
            ("mod.txt".into(), ChangeKind::Modified),
            ("moved.txt".into(), ChangeKind::Added),
            ("new.txt".into(), ChangeKind::Added),
            ("old.txt".into(), ChangeKind::Deleted),
        ]
    );
    assert!(set.files.iter().all(|c| c.revertible));
}

#[test]
fn revert_restores_every_kind_byte_exact() {
    let w = ws();
    w.write("mod.txt", "no trailing newline");
    w.write("del.txt", "doomed\n");
    let set = w.turn("t1", |w| {
        w.write("mod.txt", "changed\n");
        w.write("new.txt", "fresh\n");
        std::fs::remove_file(w.root.join("del.txt")).unwrap();
    });
    for c in &set.files {
        assert_eq!(
            revert_file(&w.cap, c, false).unwrap(),
            RevertOutcome::Reverted,
            "{}",
            c.rel
        );
    }
    assert_eq!(w.read("mod.txt").as_deref(), Some("no trailing newline"));
    assert_eq!(w.read("del.txt").as_deref(), Some("doomed\n"));
    assert_eq!(w.read("new.txt"), None);
}

#[test]
fn revert_refuses_a_drifted_file_unless_forced() {
    let w = ws();
    w.write("a.txt", "v1\n");
    let set = w.turn("t1", |w| w.write("a.txt", "v2\n"));
    w.write("a.txt", "v3 user edit\n");
    let c = &set.files[0];
    assert!(has_drifted(c));
    assert_eq!(
        revert_file(&w.cap, c, false).unwrap(),
        RevertOutcome::Drifted
    );
    assert_eq!(w.read("a.txt").as_deref(), Some("v3 user edit\n"));
    assert_eq!(
        revert_file(&w.cap, c, true).unwrap(),
        RevertOutcome::Reverted
    );
    assert_eq!(w.read("a.txt").as_deref(), Some("v1\n"));
}

#[test]
fn hunk_revert_keeps_the_other_hunks() {
    let w = ws();
    let before: String = (0..20).map(|i| format!("line {i}\n")).collect();
    w.write("f.txt", &before);
    let set = w.turn("t1", |w| {
        let after = before
            .replace("line 1\n", "LINE 1\n")
            .replace("line 18\n", "LINE 18\n");
        w.write("f.txt", &after);
    });
    let c = &set.files[0];
    let hunks = file_hunks(&w.cap, c).unwrap();
    assert_eq!(hunks.len(), 2);
    assert_eq!(
        revert_hunks(&w.cap, c, &[1], false).unwrap(),
        RevertOutcome::Reverted
    );
    let now = w.read("f.txt").unwrap();
    assert!(
        now.contains("LINE 1\n") && now.contains("line 18\n"),
        "{now}"
    );
}

#[test]
fn unchanged_tree_and_rewritten_identical_bytes_are_empty() {
    let w = ws();
    w.write("a.txt", "same\n");
    let set = w.turn("t1", |w| w.write("a.txt", "same\n"));
    assert!(set.is_empty(), "{set:?}");
}

#[test]
fn same_tick_rewrite_with_equal_size_is_still_detected() {
    let w = ws();
    w.write("a.txt", "aaaa\n");
    let p = w.root.join("a.txt");
    let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
    let set = w.turn("t1", |w| {
        w.write("a.txt", "bbbb\n");
        // Force the mtime back: only the racy guard can catch this.
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    });
    assert_eq!(kinds(&set), vec![("a.txt".into(), ChangeKind::Modified)]);
}

#[test]
fn host_writes_are_not_attributed_to_the_agent() {
    let w = ws();
    w.write("editor.txt", "v1\n");
    w.write("agent.txt", "v1\n");
    let set = w.turn("t1", |w| {
        w.write("editor.txt", "saved by user\n");
        w.cap
            .ledger()
            .record(&w.root.join("editor.txt"), Some(b"saved by user\n"));
        w.write("agent.txt", "v2\n");
    });
    assert_eq!(
        kinds(&set),
        vec![("agent.txt".into(), ChangeKind::Modified)]
    );
}

#[test]
fn sensitive_paths_are_auto_reverted() {
    let w = ws();
    w.write(".gitignore", ".env\n");
    w.write(".env", "SECRET=old\n");
    let mut set = w.turn("t1", |w| {
        w.write(".env", "SECRET=new\n");
        w.write("ok.txt", "x\n");
    });
    w.cap.auto_revert_sensitive(&mut set);
    assert_eq!(kinds(&set), vec![("ok.txt".into(), ChangeKind::Added)]);
    assert_eq!(set.auto_reverted, vec![".env".to_string()]);
    assert_eq!(w.read(".env").as_deref(), Some("SECRET=old\n"));
}

#[test]
fn edits_in_included_ignored_folders_are_reviewed_and_revertible() {
    let w = ws();
    w.write(".gitignore", "plans/\ntarget/\n");
    w.write("plans/next.md", "v1\n");
    w.write("target/debug/out", "old\n");
    let scope = CaptureScope {
        include_ignored: vec!["plans".into()],
        ..w.scope()
    };
    let h = w.cap.begin("t1", Some("conv"), scope).unwrap();
    w.write("plans/next.md", "v2\n");
    w.write("plans/new.md", "fresh\n");
    w.write("target/debug/out", "rebuilt\n");
    let set = w.cap.end(h, TurnOutcome::Completed).unwrap().set;
    assert_eq!(
        kinds(&set),
        vec![
            ("plans/new.md".into(), ChangeKind::Added),
            ("plans/next.md".into(), ChangeKind::Modified),
        ]
    );
    let modified = set.files.iter().find(|c| c.rel == "plans/next.md").unwrap();
    assert!(modified.revertible);
    assert_eq!(revert_file(&w.cap, modified, false).unwrap(), RevertOutcome::Reverted);
    assert_eq!(w.read("plans/next.md").as_deref(), Some("v1\n"));
}

#[test]
fn overlapping_turns_mark_each_other() {
    let w = ws();
    w.write("shared.txt", "v1\n");
    w.write("a_only.txt", "v1\n");
    let a = w.cap.begin("turn-a", Some("conv-a"), w.scope()).unwrap();
    let b = w.cap.begin("turn-b", Some("conv-b"), w.scope()).unwrap();
    w.write("shared.txt", "from a\n");
    w.write("a_only.txt", "from a\n");
    let a_end = w.cap.end(a, TurnOutcome::Completed).unwrap();
    w.cap
        .save_pending(&PendingReview::new(a_end.set.clone()))
        .unwrap();
    w.write("shared.txt", "from b\n");
    let b_end = w.cap.end(b, TurnOutcome::Completed).unwrap();

    // A filesystem diff cannot attribute a write inside two open windows: B
    // sees A's `a_only.txt` write too. Both reviews carry the mark instead.
    for rel in ["shared.txt", "a_only.txt"] {
        let in_b = b_end.set.files.iter().find(|c| c.rel == rel).unwrap();
        assert_eq!(in_b.overlap_with, vec!["turn-a".to_string()], "{rel}");
    }
    assert_eq!(b_end.overlapped_reviews, vec!["turn-a".to_string()]);
    let a_pending = w.cap.load_pending();
    for c in &a_pending[0].set.files {
        assert_eq!(c.overlap_with, vec!["turn-b".to_string()], "{}", c.rel);
    }
}

#[test]
fn a_turn_that_ended_before_another_began_is_not_an_overlap() {
    let w = ws();
    w.write("f.txt", "v1\n");
    let a = w.turn("turn-a", |w| w.write("f.txt", "a\n"));
    w.cap.save_pending(&PendingReview::new(a)).unwrap();
    let b = w.turn("turn-b", |w| w.write("f.txt", "b\n"));
    assert!(b.files[0].overlap_with.is_empty());
}

#[test]
fn background_saves_never_land_after_the_archive() {
    let w = ws();
    w.write("f.txt", "v1\n");
    let set = w.turn("t1", |w| w.write("f.txt", "v2\n"));
    let review = PendingReview::new(set);
    for _ in 0..20 {
        w.cap.save_pending_later(review.clone());
    }
    w.cap.archive_later(ReviewedTurn {
        set: review.set.clone(),
        resolved: Default::default(),
    });
    w.cap.flush();
    assert!(w.cap.load_pending().is_empty());
    assert_eq!(w.cap.recent_reviewed().len(), 1);
}

#[test]
fn resolve_applies_decisions_and_archives() {
    let w = ws();
    w.write("keep.txt", "k1\n");
    w.write("back.txt", "b1\n");
    let set = w.turn("t1", |w| {
        w.write("keep.txt", "k2\n");
        w.write("back.txt", "b2\n");
    });
    let mut review = PendingReview::new(set);
    w.cap.save_pending(&review).unwrap();
    let back_key = review
        .set
        .files
        .iter()
        .find(|c| c.rel == "back.txt")
        .unwrap()
        .key();
    review
        .decisions
        .insert(back_key.clone(), FileDecision::Revert);

    let done = w.cap.resolve(&review, &HashSet::new());
    assert_eq!(
        done.resolved.get(&back_key),
        Some(&ResolvedDecision::Reverted)
    );
    assert_eq!(w.read("keep.txt").as_deref(), Some("k2\n"));
    assert_eq!(w.read("back.txt").as_deref(), Some("b1\n"));
    assert!(w.cap.load_pending().is_empty());
    assert_eq!(w.cap.recent_reviewed().len(), 1);

    // The revert is a host write: the next turn does not report it.
    let next = w.turn("t2", |_| {});
    assert!(next.is_empty(), "{next:?}");
}

#[test]
fn retention_keeps_the_last_five_reviewed_turns() {
    let w = ws();
    w.write("f.txt", "0\n");
    for i in 1..=7 {
        let set = w.turn(&format!("t{i}"), |w| w.write("f.txt", &format!("{i}\n")));
        w.cap.resolve(&PendingReview::new(set), &HashSet::new());
    }
    let recent = w.cap.recent_reviewed();
    assert_eq!(recent.len(), RETAINED_TURNS);
    assert_eq!(recent[0].set.turn_id, "t7");
    // Everything is inside the grace window so far; collect as if it had passed.
    assert!(w.cap.store().contains(&store::hash_bytes(b"0\n")));
    w.cap.gc_with_grace(std::time::Duration::ZERO);
    // t3's pre-image ("2\n") is still recoverable; t1's ("0\n") is gone.
    assert!(w.cap.store().contains(&store::hash_bytes(b"2\n")));
    assert!(!w.cap.store().contains(&store::hash_bytes(b"0\n")));
}

#[test]
fn between_turn_changes_are_reported_at_the_next_begin() {
    let w = ws();
    w.write("a.txt", "1\n");
    let _ = w.turn("t1", |_| {});
    w.write("a.txt", "user edit\n");
    let h = w.cap.begin("t2", None, w.scope()).unwrap();
    assert_eq!(h.between_turns, vec!["a.txt".to_string()]);
    let set = w.cap.end(h, TurnOutcome::Completed).unwrap().set;
    assert!(set.is_empty());
}

#[cfg(unix)]
fn mode_of(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o7777
}

#[cfg(unix)]
fn chmod(p: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(unix)]
#[test]
fn revert_of_a_delete_restores_the_pre_turn_mode() {
    let w = ws();
    w.write("run.sh", "#!/bin/sh\necho hi\n");
    w.write("secret.conf", "k=v\n");
    chmod(&w.root.join("run.sh"), 0o755);
    chmod(&w.root.join("secret.conf"), 0o640);
    let set = w.turn("t1", |w| {
        std::fs::remove_file(w.root.join("run.sh")).unwrap();
        std::fs::remove_file(w.root.join("secret.conf")).unwrap();
    });
    for c in &set.files {
        assert_eq!(c.kind, ChangeKind::Deleted, "{}", c.rel);
        assert_eq!(revert_file(&w.cap, c, false).unwrap(), RevertOutcome::Reverted);
    }
    assert_eq!(w.read("run.sh").as_deref(), Some("#!/bin/sh\necho hi\n"));
    assert_eq!(mode_of(&w.root.join("run.sh")), 0o755);
    assert_eq!(mode_of(&w.root.join("secret.conf")), 0o640);
}

#[cfg(unix)]
#[test]
fn revert_of_an_edit_restores_the_pre_turn_mode() {
    let w = ws();
    w.write("run.sh", "#!/bin/sh\n");
    chmod(&w.root.join("run.sh"), 0o755);
    let set = w.turn("t1", |w| {
        w.write("run.sh", "#!/bin/sh\nrm -rf build\n");
        chmod(&w.root.join("run.sh"), 0o644);
    });
    assert_eq!(revert_file(&w.cap, &set.files[0], false).unwrap(), RevertOutcome::Reverted);
    assert_eq!(mode_of(&w.root.join("run.sh")), 0o755);
}

#[cfg(unix)]
#[test]
fn a_chmod_between_turns_reaches_the_baseline() {
    let w = ws();
    w.write("run.sh", "#!/bin/sh\n");
    let p = w.root.join("run.sh");
    // Old enough that the next begin reuses the entry instead of rehashing.
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
    chmod(&p, 0o644);
    let _ = w.turn("t1", |_| {});
    // A chmod changes neither size nor mtime.
    chmod(&p, 0o755);
    let set = w.turn("t2", |w| std::fs::remove_file(w.root.join("run.sh")).unwrap());
    assert_eq!(set.files[0].before.as_ref().unwrap().mode, Some(0o755));
    revert_file(&w.cap, &set.files[0], false).unwrap();
    assert_eq!(mode_of(&p), 0o755);
}

#[cfg(unix)]
#[test]
fn a_restore_without_a_recorded_mode_is_not_private() {
    // Records written before modes were captured: the restored file gets the
    // mode any new file would, not the temp file's 0600.
    let w = ws();
    let plain = w.root.join("plain.txt");
    std::fs::write(&plain, "x").unwrap();
    let restored = w.root.join("restored.txt");
    store::restore_write(&restored, b"x", None).unwrap();
    assert_eq!(mode_of(&restored), mode_of(&plain));
}

// ── Write-claim attribution ──────────────────────────────────────────────

use crate::write_claims::{ClaimJournal, ClaimWriter, FileWrite, WriterKind};

fn journal(w: &Ws) -> ClaimJournal {
    ClaimJournal::at(w._dir.path().join("claims"))
}

/// Open a turn attributed against `journal`, with its own `ChatTurn` writer.
fn open_attributed(w: &Ws, journal: &ClaimJournal, id: &str) -> (TurnHandle, Arc<ClaimWriter>) {
    let h = w.cap.begin(id, Some("conv"), w.scope()).unwrap();
    let writer = journal
        .register(
            &format!("chat-test-{id}"),
            WriterKind::ChatTurn,
            "chat 'test'",
            w.scope().roots,
        )
        .unwrap();
    let (roots, entries) = h.claim_baseline();
    writer.set_baseline(roots, entries);
    (h.attribute(journal.clone(), writer.clone()), writer)
}

fn attributed_turn(
    w: &Ws,
    journal: &ClaimJournal,
    id: &str,
    edit: impl FnOnce(&Ws, &ClaimWriter),
) -> TurnChangeSet {
    let (h, writer) = open_attributed(w, journal, id);
    edit(w, &writer);
    w.cap.end(h, TurnOutcome::Completed).unwrap().set
}

fn file<'a>(set: &'a TurnChangeSet, rel: &str) -> &'a FileChange {
    set.files
        .iter()
        .find(|c| c.rel == rel)
        .unwrap_or_else(|| panic!("{rel} not in {:?}", kinds(set)))
}

/// A writer registered from here on starts strictly later.
fn tick() {
    std::thread::sleep(std::time::Duration::from_millis(2));
}

const CLI: &str = "gaviero-cli 'x' · u1";

#[test]
fn foreign_claimed_change_is_left_out() {
    let w = ws();
    w.write("a.txt", "v1\n");
    w.write("b.txt", "v1\n");
    let j = journal(&w);
    let cli = j
        .register("cli-u1", WriterKind::SwarmAgent, CLI, vec![w.root.clone()])
        .unwrap();
    let set = attributed_turn(&w, &j, "t1", |w, _| {
        w.write("a.txt", "mine\n");
        w.write("b.txt", "theirs\n");
        cli.record(&FileWrite::unknown(w.root.join("b.txt")));
    });
    assert_eq!(kinds(&set), vec![("a.txt".into(), ChangeKind::Modified)]);
    assert_eq!(
        set.left_out,
        vec![LeftOut {
            rel: "b.txt".into(),
            root: w.root.clone(),
            writer: CLI.into(),
        }]
    );
    assert_eq!(file(&set, "a.txt").maybe_from, vec![CLI.to_string()]);
}

#[test]
fn own_claimed_change_is_kept_without_tags_even_with_others_active() {
    let w = ws();
    w.write("a.txt", "v1\n");
    let j = journal(&w);
    let _cli = j
        .register("cli-u1", WriterKind::SwarmAgent, CLI, vec![w.root.clone()])
        .unwrap();
    let set = attributed_turn(&w, &j, "t1", |w, me| {
        w.write("a.txt", "mine\n");
        me.record(&FileWrite::unknown(w.root.join("a.txt")));
    });
    assert_eq!(kinds(&set), vec![("a.txt".into(), ChangeKind::Modified)]);
    let a = file(&set, "a.txt");
    assert!(a.maybe_from.is_empty());
    assert!(a.overlap_with.is_empty());
    assert!(set.left_out.is_empty());
    assert!(!set.warnings.iter().any(|m| m.contains("⚠shared")));
}

#[test]
fn both_claimed_change_is_kept_and_overlap_marked_with_label() {
    let w = ws();
    w.write("a.txt", "v1\n");
    let j = journal(&w);
    let cli = j
        .register("cli-u1", WriterKind::SwarmAgent, CLI, vec![w.root.clone()])
        .unwrap();
    let set = attributed_turn(&w, &j, "t1", |w, me| {
        w.write("a.txt", "mine\n");
        me.record(&FileWrite::unknown(w.root.join("a.txt")));
        w.write("a.txt", "theirs\n");
        cli.record(&FileWrite::unknown(w.root.join("a.txt")));
    });
    assert_eq!(kinds(&set), vec![("a.txt".into(), ChangeKind::Modified)]);
    assert_eq!(file(&set, "a.txt").overlap_with, vec![CLI.to_string()]);
    assert!(set.left_out.is_empty());
}

#[test]
fn later_writer_noop_claim_cannot_hide_this_turns_change() {
    let w = ws();
    w.write("c.txt", "v1\n");
    let j = journal(&w);
    let set = attributed_turn(&w, &j, "t1", |w, _| {
        // The turn changes c.txt without a claim (Bash, say).
        w.write("c.txt", "mine\n");
        tick();
        // A writer started mid-turn claims a no-op write of those bytes.
        let late = j
            .register("late", WriterKind::SwarmAgent, "late", vec![w.root.clone()])
            .unwrap();
        let path = w.root.join("c.txt");
        late.record(&FileWrite {
            path: path.clone(),
            before: crate::write_claims::Before::Hash(hash_file(&path).unwrap()),
        });
    });
    assert_eq!(kinds(&set), vec![("c.txt".into(), ChangeKind::Modified)]);
    assert!(set.left_out.is_empty());
    assert_eq!(file(&set, "c.txt").maybe_from, vec!["late".to_string()]);
}

#[test]
fn host_writer_claim_hides_like_the_ledger() {
    let w = ws();
    w.write("editor.txt", "v1\n");
    w.write("agent.txt", "v1\n");
    let j = journal(&w);
    let set = attributed_turn(&w, &j, "t1", |w, _| {
        tick();
        // Another TUI's editor (not this capture's ledger), started mid-turn:
        // the host is trusted without a before-hash.
        let editor = j
            .register("editor-9", WriterKind::Host, "editor", vec![w.root.clone()])
            .unwrap();
        w.write("editor.txt", "saved by user\n");
        editor.record(&FileWrite::unknown(w.root.join("editor.txt")));
        w.write("agent.txt", "v2\n");
    });
    assert_eq!(
        kinds(&set),
        vec![("agent.txt".into(), ChangeKind::Modified)]
    );
    assert_eq!(set.left_out.len(), 1);
    assert_eq!(set.left_out[0].rel, "editor.txt");
    assert_eq!(set.left_out[0].writer, "editor");
    // The editor is never named as a possible source.
    assert!(file(&set, "agent.txt").maybe_from.is_empty());

    // This capture's own host writes reach the journal as `editor` claims.
    let host = j
        .register("editor-1", WriterKind::Host, "editor", vec![w.root.clone()])
        .unwrap();
    w.cap.set_host_writer(host);
    let p = w.root.join("saved.txt");
    std::fs::write(&p, "x").unwrap();
    w.cap.record_host_write(&p, Some(b"x"));
    assert!(w.cap.ledger().attributes(&p, Some(&store::hash_bytes(b"x")), 0));
    let snap = j.snapshot(0);
    let rec = snap
        .writers
        .iter()
        .find(|r| r.info.id == "editor-1")
        .unwrap();
    assert_eq!(rec.claims.len(), 1);
    assert_eq!(rec.claims[0].key, crate::write_claims::claim_key(&p));
}

#[test]
fn attribution_off_reports_everything_like_today() {
    let w = ws();
    w.write("a.txt", "v1\n");
    w.write("b.txt", "v1\n");
    let j = journal(&w);
    let cli = j
        .register("cli-u1", WriterKind::SwarmAgent, CLI, vec![w.root.clone()])
        .unwrap();
    // No `.attribute(..)`.
    let set = w.turn("t1", |w| {
        w.write("a.txt", "mine\n");
        w.write("b.txt", "theirs\n");
        cli.record(&FileWrite::unknown(w.root.join("b.txt")));
    });
    assert_eq!(
        kinds(&set),
        vec![
            ("a.txt".into(), ChangeKind::Modified),
            ("b.txt".into(), ChangeKind::Modified),
        ]
    );
    assert!(set.files.iter().all(|c| c.maybe_from.is_empty()));
    assert!(set.left_out.is_empty());
    assert!(set.warnings.is_empty());
}

#[test]
fn shared_warning_names_the_writers() {
    let w = ws();
    let j = journal(&w);
    let _cli = j
        .register("cli-u1", WriterKind::SwarmAgent, CLI, vec![w.root.clone()])
        .unwrap();
    let _swarm = j
        .register(
            "swarm-u2",
            WriterKind::SwarmAgent,
            "SWARM 'y' · u2",
            vec![w.root.clone()],
        )
        .unwrap();
    let set = attributed_turn(&w, &j, "t1", |w, _| {
        w.write("one.txt", "1\n");
        w.write("two.txt", "2\n");
    });
    assert_eq!(set.files.len(), 2);
    assert!(
        set.warnings.contains(
            &"2 files marked ⚠shared changed while SWARM 'y' · u2, gaviero-cli 'x' · u1 also ran — they may not come from this agent"
                .to_string()
        ),
        "{:?}",
        set.warnings
    );

    let w = ws();
    let j = journal(&w);
    let _cli = j
        .register("cli-u1", WriterKind::SwarmAgent, CLI, vec![w.root.clone()])
        .unwrap();
    let set = attributed_turn(&w, &j, "t1", |w, _| w.write("one.txt", "1\n"));
    assert!(
        set.warnings.contains(
            &format!("1 file marked ⚠shared changed while {CLI} also ran — they may not come from this agent")
        ),
        "{:?}",
        set.warnings
    );
}

#[test]
fn attribution_does_not_affect_in_process_overlap_of_kept_files() {
    let w = ws();
    w.write("common.txt", "v1\n");
    let j = journal(&w);
    let (a, a_writer) = open_attributed(&w, &j, "turn-a");
    tick();
    let (b, b_writer) = open_attributed(&w, &j, "turn-b");
    w.write("a.txt", "from a\n");
    a_writer.record(&FileWrite::unknown(w.root.join("a.txt")));
    w.write("b.txt", "from b\n");
    b_writer.record(&FileWrite::unknown(w.root.join("b.txt")));
    // Nobody claims this one (a shell command).
    w.write("common.txt", "shell\n");

    let a_end = w.cap.end(a, TurnOutcome::Completed).unwrap();
    w.cap
        .save_pending(&PendingReview::new(a_end.set.clone()))
        .unwrap();
    let b_end = w.cap.end(b, TurnOutcome::Completed).unwrap();

    let rels = |s: &TurnChangeSet| s.files.iter().map(|c| c.rel.clone()).collect::<Vec<_>>();
    assert_eq!(rels(&a_end.set), vec!["a.txt", "common.txt"]);
    assert_eq!(rels(&b_end.set), vec!["b.txt", "common.txt"]);
    assert_eq!(a_end.set.left_out[0].rel, "b.txt");
    assert_eq!(b_end.set.left_out[0].rel, "a.txt");

    // Only the file both turns kept is overlap-marked.
    assert!(file(&b_end.set, "b.txt").overlap_with.is_empty());
    assert_eq!(
        file(&b_end.set, "common.txt").overlap_with,
        vec!["turn-a".to_string()]
    );
    assert_eq!(b_end.overlapped_reviews, vec!["turn-a".to_string()]);
    let a_pending = w.cap.load_pending();
    let a_files = &a_pending[0].set.files;
    for c in a_files {
        let want: Vec<String> = if c.rel == "common.txt" {
            vec!["turn-b".into()]
        } else {
            vec![]
        };
        assert_eq!(c.overlap_with, want, "{}", c.rel);
    }
}
