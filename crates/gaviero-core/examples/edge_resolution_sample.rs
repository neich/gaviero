//! Fixed-seed samples of resolved references for hand labelling
//! (edge-resolution plan E7 and receiver-typing plan R0/R7; the oracle
//! fallback).
//!
//! **Stratified** (default):
//!
//! ```text
//! edge_resolution_sample <root> <graph-db> <out.jsonl> [<seed>]
//! ```
//!
//! Builds the graph of `<root>` into `<graph-db>` (a temp path), resolves
//! every Rust / TypeScript / Python reference again **without** the
//! ambiguity cap, and buckets the cross-file results by how many target
//! files they reach (`n`). With cap `k`, buckets `n <= k` become edges and
//! the rest are dropped, so one labelled sample prices caps 1, 2 and 3.
//! Strata: `n=1` 110, `n=2` 40, `n=3` 25, `n>=4` 25. One JSON line per
//! sampled reference; a final `{"population": …}` line has the bucket sizes
//! for weighting.
//!
//! **Uniform method-call sample** (true recall):
//!
//! ```text
//! edge_resolution_sample <root> <graph-db> <out.jsonl> --all-method-refs <n> <seed>
//! ```
//!
//! Draws `n` Rust method-call references (`x.foo()`) uniformly, whatever
//! their outcome under the real cap (edge, local, unresolved, no edge,
//! dropped), with ±3 lines of context. A final line has the population.
//!
//! **Re-score**:
//!
//! ```text
//! edge_resolution_sample <root> <graph-db> <out.jsonl> --rescore <sample.jsonl>
//! ```
//!
//! Re-resolves exactly the references of an earlier sample with the current
//! code (uncapped; matched by file, line, name, kind, path and style) and
//! writes their new targets under the old ids; a reference that no longer
//! yields targets gets `"n": 0`. Lets a labelled sample price a change.

use std::collections::BTreeMap;
use std::io::Write;

use gaviero_core::repo_map::edges::{CallStyle, RawReference, RefKind, extract_references_for_extension};
use gaviero_core::repo_map::extraction::ExtractionOutcome;
use gaviero_core::repo_map::graph_builder::build_graph_at;
use gaviero_core::repo_map::resolve::{
    FileScope, Lang, Outcome, ResolveContext, lang_of, sample_key, take_by_key,
};

const STRATA: [(&str, usize); 4] = [("n1", 110), ("n2", 40), ("n3", 25), ("n4plus", 25)];

/// (file, line, name, kind, path, style): identifies a reference across
/// code versions (a key can still repeat: the same call twice on a line).
type Key = (String, u64, String, String, String, String);

enum Mode {
    Stratified { seed: u64 },
    MethodRefs { n: usize, seed: u64 },
    Rescore { ids: std::collections::HashMap<Key, Vec<u64>> },
}

fn kind_str(k: RefKind) -> &'static str {
    match k {
        RefKind::Import => "Imports",
        RefKind::Call => "Calls",
        RefKind::Implements => "Implements",
    }
}

fn style_str(s: CallStyle) -> &'static str {
    match s {
        CallStyle::Bare => "bare",
        CallStyle::Scoped => "scoped",
        CallStyle::Method => "method",
    }
}

fn key_of(file: &str, r: &RawReference) -> Key {
    (
        file.to_string(),
        r.line as u64 + 1,
        r.target_name.clone(),
        kind_str(r.kind).to_string(),
        r.path.join("::"),
        style_str(r.style).to_string(),
    )
}

fn targets_json(targets: &std::collections::BTreeSet<(String, Option<String>)>) -> Vec<serde_json::Value> {
    targets
        .iter()
        .take(8)
        .map(|(f, name)| serde_json::json!({"file": f, "item": name}))
        .collect()
}

fn parse_mode(args: &[String]) -> anyhow::Result<Mode> {
    Ok(match (args.get(3).map(String::as_str), args.get(4), args.get(5)) {
        (Some("--rescore"), Some(path), _) => {
            let mut ids = std::collections::HashMap::new();
            for line in std::fs::read_to_string(path)?.lines() {
                let v: serde_json::Value = serde_json::from_str(line)?;
                if let Some(id) = v["id"].as_u64() {
                    let key = (
                        v["file"].as_str().unwrap_or_default().to_string(),
                        v["line"].as_u64().unwrap_or_default(),
                        v["name"].as_str().unwrap_or_default().to_string(),
                        v["kind"].as_str().unwrap_or_default().to_string(),
                        v["path"].as_str().unwrap_or_default().to_string(),
                        v["style"].as_str().unwrap_or_default().to_string(),
                    );
                    ids.entry(key).or_insert_with(Vec::new).push(id);
                }
            }
            Mode::Rescore { ids }
        }
        (Some("--all-method-refs"), Some(n), Some(seed)) => Mode::MethodRefs {
            n: n.parse()?,
            seed: seed.parse()?,
        },
        (Some("--all-method-refs"), _, _) => anyhow::bail!("--all-method-refs <n> <seed>"),
        (Some(seed), _, _) => Mode::Stratified { seed: seed.parse()? },
        (None, _, _) => Mode::Stratified { seed: 0xE7 },
    })
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (root, db, out) = match args.as_slice() {
        [r, d, o, ..] => (std::path::PathBuf::from(r), std::path::PathBuf::from(d), std::path::PathBuf::from(o)),
        _ => anyhow::bail!(
            "usage: edge_resolution_sample <root> <graph-db> <out.jsonl> [<seed> | --rescore <sample.jsonl> | --all-method-refs <n> <seed>]"
        ),
    };
    let mode = parse_mode(&args)?;

    let (store, _) = build_graph_at(&root, &db, &[])?;
    let uncapped = ResolveContext::build(&store, &root)?.with_max_target_files(usize::MAX);
    let capped = ResolveContext::build(&store, &root)?;

    let mut buckets: BTreeMap<&str, Vec<(u64, serde_json::Value)>> = BTreeMap::new();
    let mut population_all: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rescored: BTreeMap<u64, serde_json::Value> = BTreeMap::new();
    let mut method_refs: Vec<(u64, serde_json::Value)> = Vec::new();

    let mut files = store.all_file_paths()?;
    files.sort();
    for file in files {
        let Some(lang) = lang_of(&file) else { continue };
        let eligible = store
            .file_extraction(&file)?
            .and_then(|r| ExtractionOutcome::parse(&r.outcome))
            .is_some_and(|o| o.allows_edges());
        if !eligible {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(root.join(&file)) else { continue };
        let lines: Vec<&str> = content.lines().collect();
        let ext = file.rsplit('.').next().unwrap_or("");
        let refs = extract_references_for_extension(ext, &content);
        let scope = FileScope::build(lang, &refs);
        for r in &refs {
            let kind = kind_str(r.kind);
            let style = style_str(r.style);
            let line = r.line.to_string();
            let base = serde_json::json!({
                "file": file,
                "line": r.line + 1,
                "source": lines.get(r.line).map(|l| l.trim()).unwrap_or(""),
                "kind": kind,
                "style": style,
                "name": r.target_name,
                "path": r.path.join("::"),
                "receiver": r.receiver,
                "module": r.module,
            });

            if let Mode::MethodRefs { seed, .. } = &mode {
                if lang != Lang::Rust || r.style != CallStyle::Method {
                    continue;
                }
                let outcome = capped.resolve(&file, lang, &scope, r);
                let (outcome_s, step, shown) = match &outcome {
                    Outcome::Files { step, targets } => ("edge", *step, targets_json(targets)),
                    Outcome::Local => ("local", "", vec![]),
                    Outcome::Unresolved => ("unresolved", "", vec![]),
                    Outcome::NoEdge { reason } => ("no_edge", *reason, vec![]),
                    Outcome::Ambiguous { step, .. } => ("dropped", *step, vec![]),
                };
                let lo = r.line.saturating_sub(3);
                let hi = (r.line + 4).min(lines.len());
                let context: Vec<String> = (lo..hi).map(|i| format!("{}: {}", i + 1, lines[i])).collect();
                let mut item = base;
                item["outcome"] = outcome_s.into();
                item["step"] = step.into();
                item["n"] = shown.len().into();
                item["targets"] = shown.into();
                item["context"] = context.into();
                method_refs.push((sample_key(*seed, &[&file, &line, &r.target_name, kind]), item));
                continue;
            }

            let Outcome::Files { step, targets } = uncapped.resolve(&file, lang, &scope, r) else {
                continue;
            };
            let target_files: std::collections::BTreeSet<&String> = targets.iter().map(|(f, _)| f).collect();
            let n = target_files.len();
            let bucket = match n {
                1 => "n1",
                2 => "n2",
                3 => "n3",
                _ => "n4plus",
            };
            let shown = targets_json(&targets);
            match &mode {
                Mode::Rescore { ids } => {
                    for id in ids.get(&key_of(&file, r)).into_iter().flatten() {
                        rescored.insert(*id, serde_json::json!({"id": id, "n": n, "step": step, "targets": shown}));
                    }
                    *population_all.entry(bucket).or_default() += 1;
                }
                Mode::Stratified { seed } => {
                    let mut item = base;
                    item["bucket"] = bucket.into();
                    item["step"] = step.into();
                    item["n"] = n.into();
                    item["targets"] = shown.into();
                    buckets
                        .entry(bucket)
                        .or_default()
                        .push((sample_key(*seed, &[&file, &line, &r.target_name, kind]), item));
                }
                Mode::MethodRefs { .. } => unreachable!(),
            }
        }
    }

    let mut w = std::io::BufWriter::new(std::fs::File::create(&out)?);
    match mode {
        Mode::Rescore { ids } => {
            let mut sorted: Vec<u64> = ids.values().flatten().copied().collect();
            sorted.sort_unstable();
            for id in &sorted {
                let item = rescored
                    .remove(id)
                    .unwrap_or_else(|| serde_json::json!({"id": id, "n": 0, "targets": []}));
                writeln!(w, "{item}")?;
            }
            writeln!(w, "{}", serde_json::json!({ "population": population_all }))?;
        }
        Mode::MethodRefs { n, seed } => {
            let population = method_refs.len();
            for (i, mut item) in take_by_key(method_refs, n).into_iter().enumerate() {
                item["id"] = (i + 1).into();
                writeln!(w, "{item}")?;
            }
            writeln!(w, "{}", serde_json::json!({ "population": {"method_refs": population}, "seed": seed }))?;
        }
        Mode::Stratified { seed } => {
            let mut population = serde_json::Map::new();
            let mut id = 0;
            for (bucket, size) in STRATA {
                let items = buckets.remove(bucket).unwrap_or_default();
                population.insert(bucket.to_string(), items.len().into());
                for mut item in take_by_key(items, size) {
                    id += 1;
                    item["id"] = id.into();
                    writeln!(w, "{item}")?;
                }
            }
            writeln!(w, "{}", serde_json::json!({ "population": population, "seed": seed }))?;
        }
    }
    Ok(())
}
