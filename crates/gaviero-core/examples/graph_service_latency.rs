//! Measure the live graph's costs (section-1b latency probe).
//!
//! ```text
//! graph_service_latency <repo_root> <scratch_dir>
//! ```
//!
//! Never writes inside `<repo_root>`. Prints `key=value` lines:
//! - on the real repo (graph DB in `<scratch_dir>/real.db`): a cold full
//!   build and one `RepoMap::from_store` projection — the per-update floor;
//! - on a synthetic 500-file workspace under `<scratch_dir>/synth`: the
//!   `GraphService` startup, a body-only edit and a definition edit, each
//!   measured from notification to published snapshot.

use std::path::{Path, PathBuf};
use std::time::Instant;

use gaviero_core::repo_map::graph_builder::build_graph_at;
use gaviero_core::repo_map::{GraphFolder, GraphService, RepoMap, SpecificityConfig};

const SYNTH_FILES: usize = 500;

fn ms(started: Instant) -> u128 {
    started.elapsed().as_millis()
}

fn append(path: &Path, text: &str) -> anyhow::Result<()> {
    let mut body = std::fs::read_to_string(path)?;
    body.push_str(text);
    std::fs::write(path, body)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [repo_root, scratch] = args.as_slice() else {
        anyhow::bail!("usage: graph_service_latency <repo_root> <scratch_dir>");
    };
    let repo_root = PathBuf::from(repo_root);
    let scratch = PathBuf::from(scratch);
    std::fs::create_dir_all(&scratch)?;

    // Real repo: cold full build, then one projection.
    let real_db = scratch.join("real.db");
    let _ = std::fs::remove_file(&real_db);
    let started = Instant::now();
    let (store, result) = build_graph_at(&repo_root, &real_db, &[])?;
    println!("real_full_build_ms={}", ms(started));
    let started = Instant::now();
    let map = RepoMap::from_store(&store, SpecificityConfig::default())?;
    println!("real_projection_ms={}", ms(started));
    println!("real_files={}", result.files_scanned);
    println!("real_edges={}", map.edge_count());

    // Synthetic workspace driven through the service.
    let synth = scratch.join("synth");
    let _ = std::fs::remove_dir_all(&synth);
    let src = synth.join("src");
    std::fs::create_dir_all(&src)?;
    for i in 0..SYNTH_FILES {
        let body = if i == 0 {
            "pub fn gv_f0() {}\n".to_string()
        } else {
            format!("pub fn gv_f{i}() {{ gv_f{}(); }}\n", i - 1)
        };
        std::fs::write(src.join(format!("f{i}.rs")), body)?;
    }
    let service = GraphService::new(GraphFolder {
        root: synth.clone(),
        db_path: scratch.join("synth.db"),
        excludes: Vec::new(),
        specificity: SpecificityConfig::default(),
    });
    let started = Instant::now();
    service.snapshot().await;
    println!("synth_startup_ms={}", ms(started));

    let body_file = src.join("f250.rs");
    append(&body_file, "// touched\n")?;
    let started = Instant::now();
    service.notify_changed(std::slice::from_ref(&body_file));
    service.snapshot().await;
    println!("synth_body_edit_ms={}", ms(started));

    let def_file = src.join("f251.rs");
    append(&def_file, "pub fn gv_extra() {}\n")?;
    let started = Instant::now();
    service.notify_changed(std::slice::from_ref(&def_file));
    service.snapshot().await;
    println!("synth_def_edit_ms={}", ms(started));
    Ok(())
}
