//! Tokenize every file under a corpus dir with each given tokenizer.json and
//! print one digest line per tokenizer, so two builds (onig vs fancy-regex)
//! can be compared. Usage: tok_parity <corpus-dir> <tokenizer.json>...
use std::hash::{Hash, Hasher};

fn files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs" || x == "md" || x == "py") {
            out.push(p);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut corpus = Vec::new();
    files(std::path::Path::new(&args[0]), &mut corpus);
    corpus.sort();
    let texts: Vec<String> = corpus
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|t| t.lines().map(str::to_string).collect::<Vec<_>>())
        .collect();
    for tok_path in &args[1..] {
        let tok = tokenizers::Tokenizer::from_file(tok_path).unwrap();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut n = 0usize;
        for t in &texts {
            let enc = tok.encode(t.as_str(), true).unwrap();
            enc.get_ids().hash(&mut h);
            n += enc.get_ids().len();
        }
        println!("{tok_path}\tlines={}\ttokens={n}\tdigest={:016x}", texts.len(), h.finish());
    }
}
