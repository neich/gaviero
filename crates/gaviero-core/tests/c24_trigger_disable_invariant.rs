//! C2.4 / CI grep check: `schema::drop_history_immutable_triggers(`
//! must only be called from `compress_history_row` (C1.4) and
//! `redact_history_row` (C2.4). Adding any third callsite weakens the
//! History append-only invariant — fail the build before that lands.
//!
//! This walks every `.rs` file under `crates/` and counts call-form
//! matches. The count is checked against the expected number (2);
//! the test prints the offending file paths when the count drifts
//! so a reviewer can locate the new caller immediately.

use std::path::{Path, PathBuf};

fn collect_rust_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if p.is_dir() {
            // Skip target/, .git/, and `tests/` (the meta-test in tests/
            // contains the search string as a literal — false-positive).
            if name == "target" || name == ".git" || name == "node_modules" || name == "tests" {
                continue;
            }
            collect_rust_files(&p, out);
        } else if name.ends_with(".rs") {
            out.push(p);
        }
    }
}

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to the crate; walk up to the workspace.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    while p.pop() {
        if p.join("Cargo.toml").exists() && p.join("crates").exists() {
            return p;
        }
    }
    panic!("could not find workspace root from {}", env!("CARGO_MANIFEST_DIR"));
}

#[test]
fn c24_drop_history_immutable_triggers_call_form_callsites() {
    let root = workspace_root();
    let crates = root.join("crates");
    let mut files = Vec::new();
    collect_rust_files(&crates, &mut files);

    let mut hits: Vec<(PathBuf, usize)> = Vec::new();
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        let n = src.matches("schema::drop_history_immutable_triggers(").count();
        if n > 0 {
            hits.push((f.clone(), n));
        }
    }

    let total: usize = hits.iter().map(|(_, n)| *n).sum();
    // Phase 4 of the tier-review action plan split the monolithic
    // `store.rs` into `store/mod.rs` + sibling submodules under
    // `store/`. Allowed callers (`compress_history_row`,
    // `redact_history_row`) will eventually live in
    // `store/compression_ops.rs` and `store/deletions_ops.rs`. The
    // invariant we still enforce: every call must live somewhere
    // under `crates/gaviero-core/src/memory/store/` — never anywhere
    // else, never any third caller. Update the expected count when
    // the C1.3 trigger contract grows a new sanctioned escape hatch.
    let store_dir = crates.join("gaviero-core/src/memory/store");
    let only_in_store_subtree = hits.iter().all(|(p, _)| p.starts_with(&store_dir));

    if !(total == 2 && only_in_store_subtree) {
        panic!(
            "C2.4 invariant violated: schema::drop_history_immutable_triggers(...) must only \
             appear (in call form) inside compress_history_row + redact_history_row under \
             {expected}. Saw {total} match(es): {hits:#?}",
            expected = store_dir.display(),
        );
    }
}
