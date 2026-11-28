use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::*;
use crate::turn_capture::now_ns;
use crate::turn_capture::store::hash_bytes;

type R = anyhow::Result<()>;

struct Fx {
    _tmp: tempfile::TempDir,
    journal: ClaimJournal,
    ws: PathBuf,
}

fn fx() -> anyhow::Result<Fx> {
    let tmp = tempfile::tempdir()?;
    let journal = ClaimJournal::at(tmp.path().join("claims"));
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(&ws)?;
    let ws = std::fs::canonicalize(&ws)?;
    Ok(Fx {
        _tmp: tmp,
        journal,
        ws,
    })
}

fn write(p: &Path, s: &str) {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, s).unwrap();
}

fn h(s: &str) -> String {
    hash_bytes(s.as_bytes())
}

fn claims_of<'a>(snap: &'a JournalSnapshot, id: &str) -> &'a [Claim] {
    &snap
        .writers
        .iter()
        .find(|w| w.info.id == id)
        .expect("writer in snapshot")
        .claims
}

/// Give the next writer a strictly later start than everything so far.
fn tick() {
    std::thread::sleep(Duration::from_millis(2));
}

#[test]
fn register_writes_info_and_finish_sets_ended() -> R {
    let f = fx()?;
    let w = f
        .journal
        .register("chat-1-t1", WriterKind::ChatTurn, "chat 'x'", vec![f.ws.clone()])?;
    let path = f.journal.dir().join("chat-1-t1.writer.json");
    let info: WriterInfo = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(&info, w.info());
    assert_eq!(info.kind, WriterKind::ChatTurn);
    assert_eq!(info.label, "chat 'x'");
    assert_eq!(info.pid, std::process::id());
    assert_eq!(info.roots, vec![f.ws.clone()]);
    assert_eq!(info.ended_ns, None);

    w.finish();
    let info: WriterInfo = serde_json::from_slice(&std::fs::read(&path)?)?;
    let ended = info.ended_ns.expect("finish sets ended_ns");
    assert!(ended >= info.started_ns);
    // A second finish writes nothing.
    w.finish();
    let again: WriterInfo = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(again.ended_ns, Some(ended));

    // Dropping a writer finishes it too.
    let w2 = f
        .journal
        .register("chat-1-t2", WriterKind::ChatTurn, "chat 'y'", vec![])?;
    drop(w2);
    let info: WriterInfo =
        serde_json::from_slice(&std::fs::read(f.journal.dir().join("chat-1-t2.writer.json"))?)?;
    assert!(info.ended_ns.is_some());
    Ok(())
}

#[test]
fn record_appends_claims_with_after_hash_and_absent_for_deleted() -> R {
    let f = fx()?;
    let w = f
        .journal
        .register("w", WriterKind::SwarmAgent, "w", vec![f.ws.clone()])?;
    let a = f.ws.join("a.txt");
    write(&a, "one");
    w.record(&FileWrite {
        path: a.clone(),
        before: Before::Absent,
    });
    std::fs::remove_file(&a)?;
    w.record(&FileWrite {
        path: a.clone(),
        before: Before::Hash(h("one")),
    });

    let lines = std::fs::read_to_string(f.journal.dir().join("w.claims.ndjson"))?;
    assert_eq!(lines.lines().count(), 2);
    let snap = f.journal.snapshot(0);
    let claims = claims_of(&snap, "w");
    assert_eq!(claims.len(), 2);
    assert_eq!(claims[0].key, claim_key(&a));
    assert_eq!(claims[0].before, Before::Absent);
    assert_eq!(claims[0].after.as_deref(), Some(h("one").as_str()));
    assert_eq!(claims[1].before, Before::Hash(h("one")));
    assert_eq!(claims[1].after, None);
    assert!(claims[0].at_ns <= claims[1].at_ns);
    Ok(())
}

#[test]
fn record_resolves_unknown_before_from_baseline() -> R {
    let f = fx()?;
    let w = f
        .journal
        .register("w", WriterKind::ChatTurn, "w", vec![f.ws.clone()])?;
    let known = f.ws.join("known.txt");
    let fresh = f.ws.join("sub").join("fresh.txt");
    let outside_dir = f._tmp.path().join("elsewhere");
    let outside = outside_dir.join("o.txt");
    let mut entries = HashMap::new();
    entries.insert(claim_key(&known), Before::Hash(h("old")));
    w.set_baseline(vec![f.ws.clone()], entries);
    // A second baseline is ignored.
    w.set_baseline(vec![outside_dir.clone()], HashMap::new());

    write(&known, "new");
    write(&fresh, "new");
    write(&outside, "new");
    w.record_all(&[
        FileWrite::unknown(&known),
        FileWrite::unknown(&fresh),
        FileWrite::unknown(&outside),
    ]);
    let snap = f.journal.snapshot(0);
    let claims = claims_of(&snap, "w");
    assert_eq!(claims[0].before, Before::Hash(h("old")));
    assert_eq!(claims[1].before, Before::Absent);
    assert_eq!(claims[2].before, Before::Unknown);
    Ok(())
}

#[test]
fn snapshot_skips_half_written_lines_and_old_writers() -> R {
    let f = fx()?;
    let old = f
        .journal
        .register("old", WriterKind::SwarmAgent, "old", vec![f.ws.clone()])?;
    old.finish();
    tick();
    let since = now_ns();
    tick();
    let w = f
        .journal
        .register("w", WriterKind::SwarmAgent, "w", vec![f.ws.clone()])?;
    let a = f.ws.join("a.txt");
    write(&a, "x");
    w.record(&FileWrite::unknown(&a));
    // A reader may see a half-written last line.
    {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(f.journal.dir().join("w.claims.ndjson"))?;
        file.write_all(b"{\"key\":\"/half")?;
    }
    std::fs::write(f.journal.dir().join("junk.writer.json"), b"not json")?;

    let snap = f.journal.snapshot(since);
    let ids: Vec<&str> = snap.writers.iter().map(|w| w.info.id.as_str()).collect();
    assert_eq!(ids, vec!["w"], "ended-before-since and unparsable writers skipped");
    assert_eq!(claims_of(&snap, "w").len(), 1);
    assert_eq!(snap.writers[0].effective_end_ns, None, "live pid, not finished");

    // Claims older than `since` are dropped from the snapshot.
    let later = now_ns() + 1;
    let snap = f.journal.snapshot(later);
    assert!(claims_of(&snap, "w").is_empty());
    Ok(())
}

#[test]
fn gc_removes_writers_past_ttl_and_dead_pids() -> R {
    let f = fx()?;
    std::fs::create_dir_all(f.journal.dir())?;
    let dead = WriterInfo {
        id: "dead".into(),
        kind: WriterKind::SwarmAgent,
        label: "dead".into(),
        pid: 0,
        roots: vec![f.ws.clone()],
        started_ns: 1,
        ended_ns: None,
    };
    let wpath = f.journal.dir().join("dead.writer.json");
    let cpath = f.journal.dir().join("dead.claims.ndjson");
    std::fs::write(&wpath, serde_json::to_vec(&dead)?)?;
    std::fs::write(&cpath, b"")?;
    let old = SystemTime::now() - Duration::from_secs(7 * 60 * 60);
    for p in [&wpath, &cpath] {
        std::fs::File::options()
            .write(true)
            .open(p)?
            .set_modified(old)?;
    }
    // A dead writer with fresh files and a live one survive.
    let recent = WriterInfo {
        id: "recent".into(),
        ..dead.clone()
    };
    std::fs::write(
        f.journal.dir().join("recent.writer.json"),
        serde_json::to_vec(&recent)?,
    )?;
    let live = f
        .journal
        .register("live", WriterKind::ChatTurn, "live", vec![])?;
    // `register` already collected the expired writer.
    assert!(!wpath.exists() && !cpath.exists());

    // Re-plant it and collect explicitly.
    std::fs::write(&wpath, serde_json::to_vec(&dead)?)?;
    std::fs::File::options()
        .write(true)
        .open(&wpath)?
        .set_modified(old)?;
    assert_eq!(f.journal.gc(now_ns()), 1);
    assert!(!wpath.exists());
    assert!(f.journal.dir().join("recent.writer.json").exists());
    assert!(f.journal.dir().join("live.writer.json").exists());
    drop(live);
    Ok(())
}

#[cfg(unix)]
#[test]
fn claim_key_matches_through_a_symlinked_parent() -> R {
    let f = fx()?;
    let real = f.ws.join("real");
    std::fs::create_dir_all(&real)?;
    let link = f.ws.join("link");
    std::os::unix::fs::symlink(&real, &link)?;
    assert_eq!(claim_key(&link.join("a.txt")), claim_key(&real.join("a.txt")));
    assert_eq!(
        claim_key(&real.join("x").join("..").join("a.txt")),
        claim_key(&real.join("a.txt"))
    );
    // A missing parent falls back to the lexical path.
    let missing = f.ws.join("nope").join("b.txt");
    assert_eq!(
        claim_key(&missing),
        missing.to_string_lossy().replace('\\', "/")
    );
    Ok(())
}

/// `own` starts; `make_other` registers the other writer; it claims `path`.
struct Scene {
    f: Fx,
    own: std::sync::Arc<ClaimWriter>,
    start: i64,
}

fn scene() -> anyhow::Result<Scene> {
    let f = fx()?;
    let own = f
        .journal
        .register("own", WriterKind::ChatTurn, "chat 'me'", vec![f.ws.clone()])?;
    let start = own.info().started_ns;
    Ok(Scene { f, own, start })
}

#[test]
fn classify_foreign_when_claimer_started_first() -> R {
    let f = fx()?;
    let other = f
        .journal
        .register("cli", WriterKind::SwarmAgent, "gaviero-cli 'x' · u1", vec![f.ws.clone()])?;
    tick();
    let own = f
        .journal
        .register("own", WriterKind::ChatTurn, "chat 'me'", vec![f.ws.clone()])?;
    let start = own.info().started_ns;
    let b = f.ws.join("b.txt");
    write(&b, "theirs");
    // Unknown before: only the started-first guard proves it.
    other.record(&FileWrite::unknown(&b));
    let snap = f.journal.snapshot(start);
    assert_eq!(
        snap.classify("own", start, now_ns(), &b, Some(&h("theirs"))),
        Verdict::Foreign("gaviero-cli 'x' · u1".into())
    );
    Ok(())
}

#[test]
fn classify_foreign_when_claim_proves_change_even_if_claimer_started_later() -> R {
    let s = scene()?;
    tick();
    let other = s
        .f
        .journal
        .register("late", WriterKind::SwarmAgent, "late", vec![s.f.ws.clone()])?;
    let b = s.f.ws.join("b.txt");
    write(&b, "old");
    let w = FileWrite::before_now(&b);
    write(&b, "new");
    other.record(&w);
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &b, Some(&h("new"))),
        Verdict::Foreign("late".into())
    );
    // A new file (before Absent) proves the change too.
    let c = s.f.ws.join("c.txt");
    let w = FileWrite::before_now(&c);
    assert_eq!(w.before, Before::Absent);
    write(&c, "made");
    other.record(&w);
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &c, Some(&h("made"))),
        Verdict::Foreign("late".into())
    );
    drop(s.own);
    Ok(())
}

#[test]
fn classify_unclaimed_when_later_writer_claims_identical_noop() -> R {
    let s = scene()?;
    let c = s.f.ws.join("c.txt");
    // This turn changes c.txt without a claim (Bash, say).
    write(&c, "mine");
    tick();
    // A writer started mid-turn claims a no-op write of the same bytes.
    let late = s
        .f
        .journal
        .register("late", WriterKind::SwarmAgent, "late", vec![s.f.ws.clone()])?;
    late.record(&FileWrite {
        path: c.clone(),
        before: Before::Hash(h("mine")),
    });
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &c, Some(&h("mine"))),
        Verdict::Unclaimed(vec!["late".into()])
    );
    // Unknown before from a later writer proves nothing either.
    late.record(&FileWrite::unknown(&c));
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &c, Some(&h("mine"))),
        Verdict::Unclaimed(vec!["late".into()])
    );
    drop(s.own);
    Ok(())
}

#[test]
fn classify_shared_when_both_claimed() -> R {
    let s = scene()?;
    let other = s
        .f
        .journal
        .register("o", WriterKind::SwarmAgent, "other", vec![s.f.ws.clone()])?;
    let a = s.f.ws.join("a.txt");
    write(&a, "1");
    s.own.record(&FileWrite::unknown(&a));
    write(&a, "2");
    other.record(&FileWrite::unknown(&a));
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &a, Some(&h("2"))),
        Verdict::Shared(vec!["other".into()])
    );
    // Only this writer: Own.
    let b = s.f.ws.join("b.txt");
    write(&b, "x");
    s.own.record(&FileWrite::unknown(&b));
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &b, Some(&h("x"))),
        Verdict::Own
    );
    Ok(())
}

#[test]
fn classify_host_claim_hides_on_hash_match() -> R {
    let s = scene()?;
    tick();
    let editor = s
        .f
        .journal
        .register("editor-1", WriterKind::Host, "editor", vec![s.f.ws.clone()])?;
    let a = s.f.ws.join("a.txt");
    write(&a, "saved");
    // Started after the turn, before unknown: the editor is trusted anyway.
    editor.record(&FileWrite::unknown(&a));
    let snap = s.f.journal.snapshot(s.start);
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &a, Some(&h("saved"))),
        Verdict::Foreign("editor".into())
    );
    // Content moved on since the save: not the editor's, and the editor is
    // never listed as a possible source.
    assert_eq!(
        snap.classify("own", s.start, now_ns(), &a, Some(&h("later"))),
        Verdict::Unclaimed(vec![])
    );
    drop(s.own);
    Ok(())
}

#[test]
fn classify_hash_mismatch_is_unclaimed_tagged() -> R {
    let f = fx()?;
    let other = f
        .journal
        .register("o", WriterKind::SwarmAgent, "other", vec![f.ws.clone()])?;
    tick();
    let own = f
        .journal
        .register("own", WriterKind::ChatTurn, "chat 'me'", vec![f.ws.clone()])?;
    let start = own.info().started_ns;
    let a = f.ws.join("a.txt");
    write(&a, "theirs");
    other.record(&FileWrite::unknown(&a));
    // Something changed it again afterwards.
    write(&a, "after");
    let snap = f.journal.snapshot(start);
    assert_eq!(
        snap.classify("own", start, now_ns(), &a, Some(&h("after"))),
        Verdict::Unclaimed(vec!["other".into()])
    );
    // A deleted file whose claim says it holds content: same.
    std::fs::remove_file(&a)?;
    assert_eq!(
        snap.classify("own", start, now_ns(), &a, None),
        Verdict::Unclaimed(vec!["other".into()])
    );
    Ok(())
}

#[test]
fn classify_unclaimed_lists_only_active_agent_writers_under_root() -> R {
    let s = scene()?;
    let elsewhere = s.f._tmp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere)?;
    let _editor = s
        .f
        .journal
        .register("editor-1", WriterKind::Host, "editor", vec![s.f.ws.clone()])?;
    let _under = s
        .f
        .journal
        .register("u", WriterKind::SwarmAgent, "under", vec![s.f.ws.clone()])?;
    let _away = s
        .f
        .journal
        .register("x", WriterKind::SwarmAgent, "away", vec![elsewhere.clone()])?;
    let ended = s
        .f
        .journal
        .register("e", WriterKind::SwarmRun, "ended", vec![s.f.ws.clone()])?;
    ended.finish();
    let a = s.f.ws.join("a.txt");
    write(&a, "x");
    tick();
    let snap = s.f.journal.snapshot(s.start);
    let now = now_ns();
    // "ended" finished inside the window, so it still counts.
    assert_eq!(
        snap.classify("own", s.start, now, &a, Some(&h("x"))),
        Verdict::Unclaimed(vec!["ended".into(), "under".into()])
    );
    // A window starting after "ended" finished leaves it out.
    let snap = s.f.journal.snapshot(now);
    assert_eq!(
        snap.classify("own", now, now_ns(), &a, Some(&h("x"))),
        Verdict::Unclaimed(vec!["under".into()])
    );
    // A writer whose roots don't hold the path counts once it claims it.
    let away = s
        .f
        .journal
        .register("x2", WriterKind::SwarmAgent, "away2", vec![elsewhere])?;
    away.record(&FileWrite {
        path: a.clone(),
        before: Before::Hash(h("x")),
    });
    let snap = s.f.journal.snapshot(s.start);
    match snap.classify("own", s.start, now_ns(), &a, Some(&h("x"))) {
        Verdict::Unclaimed(labels) => assert!(labels.contains(&"away2".to_string())),
        other => panic!("expected Unclaimed, got {other:?}"),
    }
    drop(s.own);
    Ok(())
}

#[test]
fn proves_change_needs_a_known_differing_before() {
    assert!(!proves_change(&Before::Unknown, Some("a")));
    assert!(proves_change(&Before::Absent, Some("a")));
    assert!(!proves_change(&Before::Absent, None));
    assert!(proves_change(&Before::Hash("a".into()), Some("b")));
    assert!(proves_change(&Before::Hash("a".into()), None));
    assert!(!proves_change(&Before::Hash("a".into()), Some("a")));
}

#[test]
fn run_claims_register_run_and_agent_writers() -> R {
    let f = fx()?;
    let cfg = SwarmClaimConfig {
        journal: f.journal.clone(),
        label: "gaviero-cli 'fix'".into(),
        consume: true,
    };
    let run = RunClaims::start(&cfg, "r1", &f.ws).expect("run writer");
    assert_eq!(
        run.run_writer.id(),
        format!("swarm-{}-r1", std::process::id())
    );
    assert_eq!(run.run_writer.info().kind, WriterKind::SwarmRun);
    let a = run.register_agent("u1", &f.ws).expect("agent writer");
    let b = run.register_agent("u1", &f.ws).expect("agent writer");
    assert_eq!(a.writer.id(), format!("{}-u1-0", run.run_writer.id()));
    assert_eq!(b.writer.id(), format!("{}-u1-1", run.run_writer.id()));
    assert_eq!(a.writer.info().label, "gaviero-cli 'fix' · u1");
    assert_eq!(a.writer.info().kind, WriterKind::SwarmAgent);
    assert!(a.consume);
    Ok(())
}
