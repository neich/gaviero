//! Edge-resolution metrics for a real tree (edge-resolution plan, E0/E7).
//!
//! Builds the code graph of `<root>` into `<graph-db>` (a temp path, never
//! the workspace's own graph) and prints one JSON object: edge counts per
//! kind, how many `Calls` references fan out to 1 / 2–9 / ≥10 target files,
//! the share of `Calls` edges that come from ambiguous references, distinct
//! file pairs, the most fanned-out names, build time, and the depth-1/2
//! reach of each seed set.
//!
//! ```text
//! edge_resolution_report <root> <graph-db> [<blast-inputs.json>]
//! ```
//!
//! `blast-inputs.json` is the X8 input list (`[{"tool": "blast_radius",
//! "args": {"paths": [...]}}, ...]`); its `blast_radius` entries are the
//! seed sets.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use gaviero_core::repo_map::graph_builder::build_graph_at;
use gaviero_core::repo_map::store::BlastRadiusMode;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, db) = match args.as_slice() {
        [root, db, ..] => (std::path::PathBuf::from(root), std::path::PathBuf::from(db)),
        _ => anyhow::bail!("usage: edge_resolution_report <root> <graph-db> [<blast-inputs.json>]"),
    };
    let seeds: Vec<Vec<String>> = match args.get(2) {
        Some(p) => {
            let calls: Vec<serde_json::Value> = serde_json::from_str(&std::fs::read_to_string(p)?)?;
            calls
                .iter()
                .filter(|c| c["tool"] == "blast_radius")
                .filter_map(|c| c["args"]["paths"].as_array())
                .map(|ps| ps.iter().filter_map(|p| p.as_str().map(String::from)).collect())
                .collect()
        }
        None => Vec::new(),
    };

    let started = Instant::now();
    let (store, result) = build_graph_at(&root, &db, &[])?;
    let build_secs = started.elapsed().as_secs_f64();
    // The resolver context alone (indexes + type facts), as built in phase 2.
    let started = Instant::now();
    drop(gaviero_core::repo_map::resolve::ResolveContext::build(&store, &root)?);
    let resolve_ctx_secs = started.elapsed().as_secs_f64();

    let conn = rusqlite::Connection::open(&db)?;
    let mut per_kind = BTreeMap::new();
    {
        let mut stmt = conn.prepare("SELECT kind, COUNT(*) FROM edges GROUP BY kind")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for row in rows {
            let (k, n) = row?;
            per_kind.insert(k, n);
        }
    }
    // One `Calls` reference = (source file, called name). Its fan-out is the
    // number of distinct target files it was linked to.
    let mut fanout: Vec<(String, String, i64, i64)> = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT e.file_path, e.source_qn, COUNT(DISTINCT n.file_path), COUNT(*)
             FROM edges e JOIN nodes n ON n.qualified_name = e.target_qn
             WHERE e.kind = 'Calls' AND n.file_path != e.file_path
             GROUP BY e.file_path, e.source_qn",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        for row in rows {
            fanout.push(row?);
        }
    }
    let refs = fanout.len();
    let one = fanout.iter().filter(|f| f.2 == 1).count();
    let some = fanout.iter().filter(|f| (2..10).contains(&f.2)).count();
    let many = fanout.iter().filter(|f| f.2 >= 10).count();
    let call_edges: i64 = fanout.iter().map(|f| f.3).sum();
    let ambiguous_edges: i64 = fanout.iter().filter(|f| f.2 > 1).map(|f| f.3).sum();
    let mut by_name: HashMap<String, i64> = HashMap::new();
    for (_, qn, files, _) in &fanout {
        let name = qn.rsplit("::").next().unwrap_or(qn).to_string();
        let e = by_name.entry(name).or_default();
        *e = (*e).max(*files);
    }
    let mut top: Vec<(String, i64)> = by_name.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top.truncate(20);
    let file_pairs: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT DISTINCT e.file_path, n.file_path FROM edges e
         JOIN nodes n ON n.qualified_name = e.target_qn
         WHERE e.kind != 'Defines' AND n.file_path != e.file_path)",
        [],
        |r| r.get(0),
    )?;
    let files: i64 = conn.query_row("SELECT COUNT(DISTINCT file_path) FROM nodes", [], |r| r.get(0))?;

    let mut reach = Vec::new();
    for seed in &seeds {
        let refs: Vec<&str> = seed.iter().map(String::as_str).collect();
        let d1 = store.impact_radius_with_mode(&refs, 1, BlastRadiusMode::All)?;
        let d2 = store.impact_radius_with_mode(&refs, 2, BlastRadiusMode::All)?;
        reach.push(serde_json::json!({
            "seeds": seed,
            "depth1": d1.affected_files.len(),
            "depth2": d2.affected_files.len(),
        }));
    }

    let mut dropped: Vec<(&String, &usize)> = result.resolve.dropped_names.iter().collect();
    dropped.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    dropped.truncate(25);
    let mut method_global: Vec<(&String, &usize)> = result.resolve.method_global_names.iter().collect();
    method_global.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let method_global_total: usize = method_global.iter().map(|(_, n)| **n).sum();
    method_global.truncate(30);
    let resolve = serde_json::json!({
        "resolved_by_step": result.resolve.resolved,
        "local": result.resolve.local,
        "unresolved": result.resolve.unresolved,
        "no_edge": result.resolve.no_edge,
        "receiver_shapes": result.resolve.receiver_shapes,
        "dropped_ambiguous": result.resolve.dropped_ambiguous,
        "top_dropped_names": dropped,
        "method_global_name_edges": method_global_total,
        "top_method_global_names": method_global,
    });
    let build = format!(
        "scanned {} changed {} unchanged {} removed {} unreadable {} reresolved {} policy_rebuild {} nodes {} edges {}",
        result.files_scanned,
        result.files_changed,
        result.files_unchanged,
        result.files_removed,
        result.files_unreadable,
        result.files_reresolved,
        result.policy_rebuild,
        result.total_nodes,
        result.total_edges,
    );
    let out = serde_json::json!({
        "files": files,
        "edges": per_kind,
        "calls_refs": refs,
        "calls_refs_1_target": one,
        "calls_refs_2_9_targets": some,
        "calls_refs_10plus_targets": many,
        "calls_edges": call_edges,
        "calls_edges_from_ambiguous_refs": ambiguous_edges,
        "calls_ambiguous_share": if call_edges > 0 { ambiguous_edges as f64 / call_edges as f64 } else { 0.0 },
        "file_pairs": file_pairs,
        "top_fanout_names": top,
        "build_secs": build_secs,
        "resolve_ctx_secs": resolve_ctx_secs,
        "build": build,
        "resolve": resolve,
        "reach": reach,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
