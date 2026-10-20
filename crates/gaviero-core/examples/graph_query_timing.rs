//! Time the two stages of a `blast_radius` call against an existing graph
//! database: the impact CTE and the mode-weighted PageRank ranking.
//!
//! ```text
//! graph_query_timing <graph-db> <depth> <mode> <path>...
//! ```

use std::time::Instant;

use gaviero_core::repo_map::store::{BlastRadiusMode, EdgeWeights, GraphStore, WalkDirection};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [db, depth, mode, paths @ ..] = args.as_slice() else {
        anyhow::bail!("usage: graph_query_timing <graph-db> <depth> <mode> <path>...");
    };
    let store = GraphStore::open(std::path::Path::new(db))?;
    let mode = BlastRadiusMode::parse(mode);
    let seeds: Vec<&str> = paths.iter().map(String::as_str).collect();
    let (nodes, edges) = store.stats()?;

    let t = Instant::now();
    let impact = store.impact_radius_with_mode(&seeds, depth.parse()?, mode)?;
    let impact_ms = t.elapsed().as_secs_f64() * 1000.0;
    eprintln!(
        "impact: {} affected in {impact_ms:.1} ms ({nodes} nodes, {edges} edges)",
        impact.affected_files.len()
    );

    let mut to_rank: Vec<String> = impact.changed_files.clone();
    for f in &impact.affected_files {
        if !to_rank.contains(f) {
            to_rank.push(f.clone());
        }
    }
    let t = Instant::now();
    let ranks = gaviero_core::repo_map::rank_files_with_weights(
        &store,
        &seeds,
        &to_rank,
        EdgeWeights::default_for(mode),
        WalkDirection::Dependents,
        gaviero_core::repo_map::SpecificityConfig::default(),
    )?;
    let rank_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        r#"{{"nodes":{nodes},"edges":{edges},"affected":{},"impact_ms":{impact_ms:.1},"ranked":{},"rank_ms":{rank_ms:.1}}}"#,
        impact.affected_files.len(),
        ranks.len()
    );
    Ok(())
}
