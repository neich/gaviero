//! Live A/B for PUSH→PULL Phase 1: thin anchor (1200) vs full push (8000).
//!
//! For each selected code prompt and each budget, we hand `claude` the budgeted
//! `<repo_outline>` (seeded with the prompt's primary gold file — the realistic
//! "open file"), wired to ONLY the gaviero read-only MCP tools
//! (`node_doc`/`blast_radius`/`memory_search`), and score:
//!   1. pulls    — which gold files it fetched via the tools (from MCP telemetry),
//!   2. answer   — whether the final answer names the gold files/symbols,
//!   3. judge    — an LLM PASS/FAIL on whether the answer used the right code.
//! Then we compare the anchor arm vs the push arm.
//!
//! Safety: `claude` runs in an empty temp cwd with NO file tools (only the 3
//! MCP tools), so it cannot read or write the working tree. Repo access is
//! solely through the in-process gaviero MCP server, so the telemetry captures
//! the full pull trace.
//!
//! Run: `cargo run -p gaviero-cli --example anchor_ab_live -- [repo] [fixture]`
//! Env: `GAVIERO_AB_LIVE_LIMIT=N` caps the number of cases (smoke test).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use gaviero_core::mcp::{
    GavieroMcpServer, McpToolCallObserver, NdjsonTelemetrySink, spawn_mcp_server,
};
use gaviero_core::memory::embedder::{Embedder, NullEmbedder};
use gaviero_core::memory::eval::{EvalCase, GoldRef, load_fixture};
use gaviero_core::memory::{MemoryStores, RerankConfig, RetrievalConfig};
use gaviero_core::repo_map::RepoMap;

const ANCHOR: usize = 1200;
const PUSH: usize = 8000;
const MODEL: &str = "sonnet";
const MAX_MULTI: usize = 12;
const MAX_CONTROLS: usize = 3;
// Generous per-arm safety stop: genuine answers finish in ~250-400s; this only
// trips on a truly stuck run so one hang can't stall the whole batch.
const RUN_TIMEOUT: Duration = Duration::from_secs(600);

/// The Phase-1 retrieval stanza, used as the system prompt for BOTH arms so the
/// only variable is the outline budget.
const SYSTEM: &str = "You are a coding assistant. You have read-only repository tools. \
The <repo_outline> you were given is a thin index — file paths with top symbol names, \
not full code. Before answering, read what you need with node_doc(path); use \
blast_radius(path) for callers, affected files, and missing tests; use memory_search for \
prior decisions. Be efficient — pull only the files you actually need, then answer; do \
not read the whole repository. Answer briefly (a few sentences) by naming the exact files \
and symbols involved and how you would change them. Do not ask the user to paste code you \
can retrieve.";

#[derive(Debug, Clone)]
struct ArmResult {
    label: String,
    budget: usize,
    answered: bool,
    pull_count: usize,
    pulled_gold_files: usize,
    gold_files_total: usize,
    answer_coverage: f32,
    judge_pass: Option<bool>,
    answer_chars: usize,
}

#[derive(Debug, Clone)]
struct CaseResult {
    id: String,
    seed: String,
    multi_file: bool,
    arms: Vec<ArmResult>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let repo = PathBuf::from(args.get(1).map(String::as_str).unwrap_or(".")).canonicalize()?;
    let fixture = args
        .get(2)
        .cloned()
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("crates/gaviero-core/eval/code_prompts.jsonl"));
    let limit: usize = std::env::var("GAVIERO_AB_LIVE_LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    let offset: usize = std::env::var("GAVIERO_AB_LIVE_OFFSET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let cases = load_fixture(&fixture).context("loading fixture")?;
    let selected = select_cases(&cases, offset, limit);
    eprintln!(
        "[ab-live] {} cases selected ({} multi-file + controls); model={MODEL} arms={ANCHOR}/{PUSH}",
        selected.len(),
        selected.iter().filter(|(_, m)| *m).count()
    );

    eprintln!("[ab-live] building RepoMap…");
    let repo_map = {
        let repo = repo.clone();
        tokio::task::spawn_blocking(move || RepoMap::build(&repo, &[]))
            .await
            .context("repomap join")??
    };

    // In-process gaviero MCP server: telemetry → temp ndjson, on a temp socket.
    let tmp = std::env::temp_dir().join(format!("gaviero-ab-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let ndjson = tmp.join("mcp_calls.ndjson");
    let socket = tmp.join("mcp.sock");
    let claude_cwd = tmp.join("cwd");
    std::fs::create_dir_all(&claude_cwd)?;

    let embedder: Arc<dyn Embedder> = Arc::new(NullEmbedder::default());
    let stores = MemoryStores::for_tests_in_memory(embedder).context("in-memory stores")?;
    let sink: Arc<dyn McpToolCallObserver> = Arc::new(NdjsonTelemetrySink::new(ndjson.clone()));
    let server = GavieroMcpServer::new(
        stores,
        repo.clone(),
        sink,
        RetrievalConfig::default(),
        RerankConfig::default(),
        None,
    );
    eprintln!("[ab-live] warming MCP graph (build_graph)…");
    server.warmup().await;
    let _handle = spawn_mcp_server(server, &socket).context("spawn mcp server")?;

    let shim = repo.join("target/debug/gaviero-mcp-shim");
    anyhow::ensure!(
        shim.exists(),
        "shim missing — run `cargo build -p gaviero-mcp-shim` first ({})",
        shim.display()
    );
    let mcp_cfg = tmp.join("mcp.json");
    std::fs::write(
        &mcp_cfg,
        serde_json::to_string(&serde_json::json!({
            "mcpServers": {
                "gaviero": {
                    "command": shim.to_string_lossy(),
                    "args": ["--socket", socket.to_string_lossy()],
                }
            }
        }))?,
    )?;

    // Results accumulator across chunks (one JSON line per finished case);
    // truncated only on the first chunk so resumed offsets append.
    let ndjson_out = repo.join("plans/pull_bootstrap/phase1-anchor-ab-live.ndjson");
    if offset == 0 {
        let _ = std::fs::remove_file(&ndjson_out);
    }

    let mut results: Vec<CaseResult> = Vec::new();
    for (i, (case, multi)) in selected.iter().enumerate() {
        let Some(seed) = primary_gold_file(case) else {
            continue;
        };
        eprintln!(
            "[ab-live] (#{}) {} [{}] seed={}",
            offset + i + 1,
            case.id,
            if *multi { "multi" } else { "single" },
            seed
        );
        let mut arms = Vec::new();
        let mut arm_answers = Vec::new();
        for (label, budget) in [("anchor", ANCHOR), ("push", PUSH)] {
            let outline = build_outline(&repo_map, &seed, budget);
            let prompt = format!("{outline}\n\nTask: {}", case.query);
            let before = ndjson_lines(&ndjson);
            let answer = run_claude_with_tools(&prompt, &claude_cwd, &mcp_cfg).await;
            let pulls = ndjson_new_pulls(&ndjson, before);
            let arm = score_arm(case, label, budget, answer.as_deref(), &pulls);
            eprintln!(
                "      {label:<6} pulls={:<2} pulled_gold={}/{} ans_cov={:.2} answered={}",
                arm.pull_count, arm.pulled_gold_files, arm.gold_files_total, arm.answer_coverage, arm.answered
            );
            arm_answers.push(answer.unwrap_or_default());
            arms.push(arm);
        }
        // Judge inline so each case is fully scored before we persist it.
        for (ai, arm) in arms.iter_mut().enumerate() {
            if arm.answered {
                arm.judge_pass = judge(case, &arm_answers[ai], &claude_cwd).await;
            }
        }
        let cr = CaseResult {
            id: case.id.clone(),
            seed,
            multi_file: *multi,
            arms,
        };
        eprintln!(
            "      → judge anchor={:?} push={:?}",
            cr.arms.first().and_then(|a| a.judge_pass),
            cr.arms.get(1).and_then(|a| a.judge_pass)
        );
        append_case_ndjson(&ndjson_out, &cr); // persist immediately (resumable)
        results.push(cr);
    }

    report(&results, &repo, &fixture);

    // Completion sentinel: written only after every selected case is fully
    // scored + judged, so a watcher (or a later session) can detect "done"
    // without parsing the log. Holds the aggregate verdict for convenience.
    let done = repo.join("plans/pull_bootstrap/phase1-anchor-ab-live.DONE");
    let _ = std::fs::write(
        &done,
        format!(
            "completed {} cases at {}\n",
            results.len(),
            chrono_now()
        ),
    );
    eprintln!("[ab-live] ALL DONE — sentinel → {}", done.display());
    Ok(())
}

/// Minimal UTC timestamp without pulling a dep (the example avoids chrono).
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{secs}")
}

/// Append one finished case as a single JSON line to the cross-chunk accumulator.
fn append_case_ndjson(path: &Path, cr: &CaseResult) {
    use std::io::Write;
    let doc = serde_json::json!({
        "id": cr.id,
        "seed": cr.seed,
        "multi_file": cr.multi_file,
        "arms": cr.arms.iter().map(|a| serde_json::json!({
            "label": a.label,
            "budget": a.budget,
            "answered": a.answered,
            "pull_count": a.pull_count,
            "pulled_gold_files": a.pulled_gold_files,
            "gold_files_total": a.gold_files_total,
            "answer_coverage": a.answer_coverage,
            "judge_pass": a.judge_pass,
            "answer_chars": a.answer_chars,
        })).collect::<Vec<_>>(),
    });
    if let Ok(line) = serde_json::to_string(&doc) {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Multi-file cases first (≥2 gold files), then single-file controls. The
/// ordered set is stable, so `offset`/`limit` carve out resumable chunks.
fn select_cases(cases: &[EvalCase], offset: usize, limit: usize) -> Vec<(EvalCase, bool)> {
    let file_count = |c: &EvalCase| {
        c.gold_must
            .iter()
            .filter(|g| matches!(g, GoldRef::File(_)))
            .count()
    };
    let mut multi: Vec<EvalCase> = cases.iter().filter(|c| file_count(c) >= 2).cloned().collect();
    let mut single: Vec<EvalCase> =
        cases.iter().filter(|c| file_count(c) == 1).cloned().collect();
    multi.truncate(MAX_MULTI);
    single.truncate(MAX_CONTROLS);
    let mut out: Vec<(EvalCase, bool)> = Vec::new();
    out.extend(multi.into_iter().map(|c| (c, true)));
    out.extend(single.into_iter().map(|c| (c, false)));
    out.into_iter().skip(offset).take(limit).collect()
}

fn primary_gold_file(case: &EvalCase) -> Option<String> {
    case.gold_must.iter().find_map(|g| match g {
        GoldRef::File(p) => Some(p.trim_start_matches("./").replace('\\', "/")),
        _ => None,
    })
}

fn build_outline(repo_map: &RepoMap, seed: &str, budget: usize) -> String {
    let candidates = repo_map.rank_for_agent_structured(std::slice::from_ref(&seed.to_string()), budget);
    let lines: Vec<String> = candidates.iter().map(|c| c.rendered_line.clone()).collect();
    format!("<repo_outline>\n{}\n</repo_outline>", lines.join("\n"))
}

fn ndjson_lines(path: &Path) -> usize {
    std::fs::read_to_string(path)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// New tool-call file paths logged since `before` lines (node_doc.path +
/// blast_radius.paths[]).
fn ndjson_new_pulls(path: &Path, before: usize) -> Vec<String> {
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in body.lines().filter(|l| !l.trim().is_empty()).skip(before) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let input = v.get("input");
        if let Some(p) = input.and_then(|i| i.get("path")).and_then(|p| p.as_str()) {
            out.push(p.replace('\\', "/"));
        }
        if let Some(arr) = input.and_then(|i| i.get("paths")).and_then(|p| p.as_array()) {
            for p in arr.iter().filter_map(|p| p.as_str()) {
                out.push(p.replace('\\', "/"));
            }
        }
    }
    out
}

fn score_arm(
    case: &EvalCase,
    label: &str,
    budget: usize,
    answer: Option<&str>,
    pulls: &[String],
) -> ArmResult {
    let gold_files: Vec<String> = case
        .gold_must
        .iter()
        .filter_map(|g| match g {
            GoldRef::File(p) => Some(p.trim_start_matches("./").replace('\\', "/")),
            _ => None,
        })
        .collect();
    let pulled_gold = gold_files
        .iter()
        .filter(|gf| pulls.iter().any(|p| p == *gf || p.ends_with(gf.as_str())))
        .count();

    let (answered, answer_coverage, answer_chars) = match answer {
        Some(text) => {
            let lower = text.to_lowercase();
            let mut covered = 0usize;
            let mut total = 0usize;
            for g in &case.gold_must {
                match g {
                    GoldRef::File(p) => {
                        total += 1;
                        let stem = Path::new(p)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(p);
                        if lower.contains(&stem.to_lowercase()) {
                            covered += 1;
                        }
                    }
                    GoldRef::Symbol(s) => {
                        total += 1;
                        let last = s.rsplit("::").next().unwrap_or(s);
                        if text.contains(last) {
                            covered += 1;
                        }
                    }
                    _ => {}
                }
            }
            let cov = if total == 0 {
                0.0
            } else {
                covered as f32 / total as f32
            };
            (true, cov, text.len())
        }
        None => (false, 0.0, 0),
    };

    ArmResult {
        label: label.to_string(),
        budget,
        answered,
        pull_count: pulls.len(),
        pulled_gold_files: pulled_gold,
        gold_files_total: gold_files.len(),
        answer_coverage,
        judge_pass: None,
        answer_chars,
    }
}

/// Run claude with ONLY the gaviero MCP tools (read-only), prompt via stdin.
async fn run_claude_with_tools(prompt: &str, cwd: &Path, mcp_cfg: &Path) -> Option<String> {
    let out = claude_json(
        prompt,
        cwd,
        &[
            "--mcp-config",
            mcp_cfg.to_str().unwrap(),
            "--strict-mcp-config",
            "--allowedTools",
            "mcp__gaviero__node_doc,mcp__gaviero__blast_radius,mcp__gaviero__memory_search",
            "--disallowedTools",
            "Write,Edit,MultiEdit,Bash,Read,Glob,Grep,NotebookEdit,WebFetch,WebSearch,TodoWrite",
        ],
        SYSTEM,
    )
    .await?;
    Some(strip_annotations(out))
}

/// Plain claude call with no tools (used for judging).
async fn claude_plain(prompt: &str, cwd: &Path) -> Option<String> {
    claude_json(prompt, cwd, &["--disallowedTools", "Write,Edit,MultiEdit,Bash,Read,Glob,Grep"], "You are a strict grader. Reply exactly PASS or FAIL on the first line.").await
}

/// Spawn `claude -p --output-format json`, prompt via stdin, return `.result`.
async fn claude_json(prompt: &str, cwd: &Path, extra: &[&str], system: &str) -> Option<String> {
    use tokio::io::AsyncWriteExt;
    use tokio::process::Command;

    let mut cmd = Command::new("claude");
    cmd.current_dir(cwd)
        .arg("-p")
        .arg("--model")
        .arg(MODEL)
        .arg("--output-format")
        .arg("json")
        .arg("--append-system-prompt")
        .arg(system);
    for a in extra {
        cmd.arg(a);
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());

    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(prompt.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let out = tokio::time::timeout(RUN_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    if v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) {
        return None;
    }
    v.get("result")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string())
}

fn strip_annotations(text: String) -> String {
    match text.split_once("<turn_annotations>") {
        Some((head, _)) => head.trim_end().to_string(),
        None => text,
    }
}

fn gold_summary(case: &EvalCase) -> String {
    let mut parts = Vec::new();
    for g in &case.gold_must {
        match g {
            GoldRef::File(p) => parts.push(format!("file {p}")),
            GoldRef::Symbol(s) => parts.push(format!("symbol {s}")),
            _ => {}
        }
    }
    parts.join("; ")
}

async fn judge(case: &EvalCase, answer: &str, cwd: &Path) -> Option<bool> {
    if answer.is_empty() {
        return Some(false);
    }
    let prompt = format!(
        "QUESTION:\n{}\n\nGROUND TRUTH (the answer must correctly identify and reason about these):\n{}\n\nASSISTANT ANSWER:\n{}\n\nReply PASS if the answer correctly identifies and uses the right files/symbols for the task, else FAIL. One word, first line.",
        case.query,
        gold_summary(case),
        answer
    );
    let verdict = claude_plain(&prompt, cwd).await?;
    Some(verdict.trim_start().to_uppercase().starts_with("PASS"))
}

fn report(results: &[CaseResult], repo: &Path, fixture: &Path) {
    println!("\n─── LIVE thin-anchor A/B (claude:{MODEL}, gaviero MCP tools only) ───");
    println!("repo    : {}", repo.display());
    println!("fixture : {}", fixture.display());
    println!("A = anchor {ANCHOR} tok   B = push {PUSH} tok   cases = {}", results.len());
    println!();

    let agg = |label: &str| {
        let arms: Vec<&ArmResult> = results
            .iter()
            .flat_map(|c| c.arms.iter())
            .filter(|a| a.label == label)
            .collect();
        let n = arms.len().max(1) as f32;
        let answered = arms.iter().filter(|a| a.answered).count();
        let mean_pull = arms.iter().map(|a| a.pull_count as f32).sum::<f32>() / n;
        let pull_recall = {
            let (num, den): (usize, usize) = arms
                .iter()
                .map(|a| (a.pulled_gold_files, a.gold_files_total))
                .fold((0, 0), |(x, y), (p, t)| (x + p, y + t));
            if den == 0 { 0.0 } else { num as f32 / den as f32 }
        };
        let mean_cov = arms.iter().map(|a| a.answer_coverage).sum::<f32>() / n;
        let judged: Vec<bool> = arms.iter().filter_map(|a| a.judge_pass).collect();
        let judge_rate = if judged.is_empty() {
            f32::NAN
        } else {
            judged.iter().filter(|b| **b).count() as f32 / judged.len() as f32
        };
        (answered, mean_pull, pull_recall, mean_cov, judge_rate)
    };

    let (aa, ap, apr, ac, aj) = agg("anchor");
    let (ba, bp, bpr, bc, bj) = agg("push");
    println!("{:<8} {:>8} {:>9} {:>11} {:>9} {:>9}", "arm", "answered", "mean_pull", "pull_recall", "ans_cov", "judge");
    println!("{:<8} {:>8} {:>9.1} {:>11.3} {:>9.3} {:>9.3}", "anchor", format!("{aa}/{}", results.len()), ap, apr, ac, aj);
    println!("{:<8} {:>8} {:>9.1} {:>11.3} {:>9.3} {:>9.3}", "push", format!("{ba}/{}", results.len()), bp, bpr, bc, bj);
    println!();
    let margin = 0.10_f32;
    let judge_ni = aj.is_nan() || bj.is_nan() || aj >= bj - margin;
    let cov_ni = ac >= bc - margin;
    println!(
        "VERDICT: {} (judge Δ={:+.3}, ans_cov Δ={:+.3}, margin {margin})",
        if judge_ni && cov_ni { "anchor NON-INFERIOR" } else { "anchor REGRESSES" },
        aj - bj,
        ac - bc
    );

    let out = repo.join("plans/pull_bootstrap/phase1-anchor-ab-live.json");
    let doc = serde_json::json!({
        "model": MODEL,
        "anchor_budget": ANCHOR,
        "push_budget": PUSH,
        "cases": results.iter().map(|c| serde_json::json!({
            "id": c.id,
            "seed": c.seed,
            "multi_file": c.multi_file,
            "arms": c.arms.iter().map(|a| serde_json::json!({
                "label": a.label,
                "budget": a.budget,
                "answered": a.answered,
                "pull_count": a.pull_count,
                "pulled_gold_files": a.pulled_gold_files,
                "gold_files_total": a.gold_files_total,
                "answer_coverage": a.answer_coverage,
                "judge_pass": a.judge_pass,
                "answer_chars": a.answer_chars,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    if let Ok(json) = serde_json::to_string_pretty(&doc) {
        let _ = std::fs::write(&out, json);
        eprintln!("[ab-live] per-case JSON → {}", out.display());
    }
}
