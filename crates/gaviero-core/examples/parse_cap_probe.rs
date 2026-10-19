//! One symbol-extraction measurement per process, for the parse-cap
//! benchmark (`research/parse_cap_bench.py`, which runs this repeatedly and
//! reads each process's peak working set from the OS).
//!
//! ```text
//! parse_cap_probe <file> <cap-bytes>                    # policy + parse under a cap
//! parse_cap_probe <file> <cap-bytes> --parse-generated  # same, without the
//!                                                       # generated-source exclusion
//! parse_cap_probe <file> baseline                       # read only (memory floor)
//! ```
//!
//! Prints one JSON line: bytes, cap, outcome, symbol count, parse time.
//! The 2 s deadline of the real policy applies; the cap is the variable.

use std::time::Instant;

use gaviero_core::repo_map::extraction::{self, ExtractionOutcome};

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(path), Some(cap)) = (args.next(), args.next()) else {
        eprintln!("usage: parse_cap_probe <file> <cap-bytes|baseline>");
        std::process::exit(2);
    };
    let path = std::path::PathBuf::from(path);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_string();
    let content = match extraction::read_source(&path) {
        Ok(c) => c,
        Err(outcome) => {
            println!(r#"{{"outcome":"{}"}}"#, outcome.as_str());
            return;
        }
    };
    if cap == "baseline" {
        println!(
            r#"{{"bytes":{},"cap":"baseline","outcome":"baseline","symbols":0,"parse_ms":0}}"#,
            content.len()
        );
        return;
    }
    let cap: usize = cap.parse().expect("cap must be an integer byte count");
    let parse_generated = args.next().as_deref() == Some("--parse-generated");
    let started = Instant::now();
    let out = if !parse_generated && extraction::is_generated_source(&ext, &content) {
        extraction::Extraction {
            outcome: ExtractionOutcome::GeneratedExcluded,
            symbols: Vec::new(),
        }
    } else if content.len() > cap {
        extraction::Extraction {
            outcome: ExtractionOutcome::ParseSizeExcluded,
            symbols: Vec::new(),
        }
    } else {
        extraction::parse_with_deadline(&ext, &content, &mut || {
            started.elapsed().as_micros() as u64 > extraction::PARSE_TIMEOUT_MICROS
        })
    };
    let parse_ms = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        r#"{{"bytes":{},"cap":{},"outcome":"{}","symbols":{},"parse_ms":{:.3}}}"#,
        content.len(),
        cap,
        out.outcome.as_str(),
        out.symbols.len(),
        parse_ms
    );
}
