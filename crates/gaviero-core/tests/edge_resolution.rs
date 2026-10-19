//! Gold-fixture harness for reference-edge resolution.
//!
//! Each fixture under `tests/fixtures/edge_resolution/<lang>/` is copied
//! into a temp dir and indexed with the real graph builder. Its stored
//! edges are projected to cross-file `(kind, from, to)` pairs and scored
//! against `expected_edges.txt`:
//! - lines `kind<TAB>from<TAB>to` are required edges;
//! - lines starting with `?` are allowed but not required (documented
//!   limits of resolution without type information);
//! - `#` starts a comment.
//!
//! Precision = required hits / (required hits + unexpected edges); recall =
//! required hits / required edges. `Defines` and same-file edges are not
//! scored. Run with `--nocapture` to see the false-positive / negative lists.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use gaviero_core::repo_map::graph_builder::build_graph_at;

type Edge = (String, String, String);

struct Score {
    precision: f64,
    recall: f64,
    false_positives: Vec<Edge>,
    false_negatives: Vec<Edge>,
}

fn fixture_root(lang: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/edge_resolution")
        .join(lang)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let src = entry.path();
        let dst = to.join(entry.file_name());
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else if entry.file_name() != "expected_edges.txt" {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

fn expected(lang: &str) -> (BTreeSet<Edge>, BTreeSet<Edge>) {
    let text = std::fs::read_to_string(fixture_root(lang).join("expected_edges.txt")).unwrap();
    let mut required = BTreeSet::new();
    let mut optional = BTreeSet::new();
    // A known limit must say why: every `?` line follows a `# LIMIT:` comment.
    let mut documented_limit = false;
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if let Some(comment) = line.trim_start().strip_prefix('#') {
            documented_limit |= comment.trim_start().starts_with("LIMIT:");
            continue;
        }
        let (set, rest) = match line.strip_prefix('?') {
            Some(rest) => {
                assert!(documented_limit, "[{lang}] `?` line without a `# LIMIT:` comment: {line:?}");
                (&mut optional, rest)
            }
            None => (&mut required, line),
        };
        documented_limit = false;
        let parts: Vec<&str> = rest.split('\t').collect();
        assert_eq!(parts.len(), 3, "bad expected line: {line:?}");
        set.insert((parts[0].into(), parts[1].into(), parts[2].into()));
    }
    (required, optional)
}

fn actual(lang: &str) -> BTreeSet<Edge> {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("ws");
    copy_dir(&fixture_root(lang), &root);
    let db = tmp.path().join("graph.db");
    let (store, _) = build_graph_at(&root, &db, &[]).unwrap();
    store
        .file_edges()
        .unwrap()
        .into_iter()
        .filter(|(_, _, k)| k.as_str() != "Defines")
        .map(|(from, to, k)| (k.as_str().to_string(), from, to))
        .collect()
}

fn score(lang: &str) -> Score {
    let (required, optional) = expected(lang);
    let got = actual(lang);
    let hits = got.intersection(&required).count();
    let false_positives: Vec<Edge> = got
        .iter()
        .filter(|e| !required.contains(*e) && !optional.contains(*e))
        .cloned()
        .collect();
    let false_negatives: Vec<Edge> = required.difference(&got).cloned().collect();
    let precision = if hits + false_positives.len() == 0 {
        1.0
    } else {
        hits as f64 / (hits + false_positives.len()) as f64
    };
    let recall = if required.is_empty() {
        1.0
    } else {
        hits as f64 / required.len() as f64
    };
    println!(
        "[{lang}] precision {precision:.3} recall {recall:.3} \
         ({hits} hits, {} false positives, {} misses, {} edges stored)",
        false_positives.len(),
        false_negatives.len(),
        got.len()
    );
    for e in &false_positives {
        println!("[{lang}]   FP {}\t{}\t{}", e.0, e.1, e.2);
    }
    for e in &false_negatives {
        println!("[{lang}]   FN {}\t{}\t{}", e.0, e.1, e.2);
    }
    Score {
        precision,
        recall,
        false_positives,
        false_negatives,
    }
}

/// Minimum (precision, recall) per fixture. The E0 baseline was Rust
/// 0.667 / 0.316, TypeScript 1.0 / 0.333, Python 1.0 / 0.111.
const MIN_RUST: (f64, f64) = (1.0, 1.0);
const MIN_TS: (f64, f64) = (1.0, 1.0);
const MIN_PY: (f64, f64) = (1.0, 1.0);

fn check(lang: &str, (min_p, min_r): (f64, f64)) {
    let s = score(lang);
    assert!(
        s.precision >= min_p && s.recall >= min_r,
        "[{lang}] precision {:.3} (min {min_p}) recall {:.3} (min {min_r})\nFP {:?}\nFN {:?}",
        s.precision,
        s.recall,
        s.false_positives,
        s.false_negatives
    );
}

#[test]
fn rust_fixture_edges() {
    check("rust", MIN_RUST);
}

#[test]
fn typescript_fixture_edges() {
    check("ts", MIN_TS);
}

#[test]
fn python_fixture_edges() {
    check("py", MIN_PY);
}

/// Receiver typing (receiver-typing plan R5): method calls resolved through
/// the receiver's type, external receivers yielding no edge.
#[test]
fn rust_receivers_fixture_edges() {
    check("rust_receivers", (1.0, 1.0));
}
