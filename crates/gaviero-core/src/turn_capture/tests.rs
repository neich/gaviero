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
            excludes: vec![],
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
