//! The symbol-extraction policy shared by the in-memory [`super::RepoMap`]
//! builder and the persistent graph builder, with a typed outcome.
//!
//! An empty symbol list used to mean any of "unsupported language", "over
//! the parse cap", "the parse timed out", or "parsed fine, no definitions".
//! [`extract`] keeps them apart so the graph can persist *why* a file has no
//! symbols ([`ExtractionOutcome`], stored per path by the graph builder).
//!
//! **What each limit governs.** The read limit ([`MAX_FILE_BYTES`]) and read
//! failures stop everything: no content, no symbols, no edges. The parse cap
//! ([`MAX_PARSE_BYTES`]) and the timeout govern *symbol* extraction only;
//! reference (edge) extraction still parses those files, as it always has.
//! Generated-source exclusion governs both.
//!
//! Bump [`EXTRACTION_POLICY_VERSION`] whenever a limit, an exclusion or the
//! extraction semantics change: the graph builder re-extracts every file,
//! unchanged content included, when the stored version differs.

use std::time::Instant;

use crate::tree_sitter::language_for_extension;

use super::Symbol;

/// Largest file read for the repo map / graph (1 MB). Larger files are
/// skipped entirely — binary data, build artifacts, dumps.
pub const MAX_FILE_BYTES: u64 = 1_000_000;

/// Largest content handed to tree-sitter for symbol extraction (400 KB).
///
/// Chosen by measurement (`research/parse_cap_bench.py`, release build,
/// 2026-10-18): the smallest candidate that covers this repo's largest
/// hand-written sources (215–369 KB; worst 92 ms, +10 MB peak working set)
/// and keeps a pathological 360 KB nested input within the 2 s deadline and
/// a 256 MiB peak budget (342 ms, +95 MB). Generated sources are excluded
/// by header instead ([`is_generated_source`]), which is what the old
/// 200 KB cap was really protecting against.
pub const MAX_PARSE_BYTES: usize = 400_000;

/// Tree-sitter parse timeout against pathological inputs (2 seconds).
pub const PARSE_TIMEOUT_MICROS: u64 = 2_000_000;

/// Version of the policy above, persisted in `graph_meta` under
/// [`EXTRACTION_POLICY_META_KEY`]. `"1"`: 200 KB cap, no generated
/// exclusion. `"2"`: 400 KB cap plus the `@generated` header exclusion.
/// `"3"`: symbol owners, bodiless trait methods, re-export rows and
/// qualified, import-aware edge resolution ([`super::resolve`]).
/// `"4"`: receiver typing — type facts, per-file imports, receiver
/// expressions ([`super::rust_walk`], `resolve::receiver`).
pub const EXTRACTION_POLICY_VERSION: &str = "5";

/// `graph_meta` key holding the policy version the stored graph was
/// completely extracted with.
pub const EXTRACTION_POLICY_META_KEY: &str = "extraction_policy_version";

/// Why a path has the symbols it has. Persisted as [`Self::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionOutcome {
    /// Parsed; at least one definition.
    Symbols,
    /// Parsed; no definitions (e.g. only `mod` items or comments).
    NoDefinitions,
    /// No tree-sitter grammar for the extension.
    UnsupportedGrammar,
    /// The grammar is known but the parser rejected it.
    LanguageSetupFailed,
    /// The parse deadline triggered.
    ParseTimeout,
    /// The parser returned no tree without the deadline triggering.
    ParseFailed,
    /// The file could not be read as UTF-8 text.
    ReadFailed,
    /// Over [`MAX_FILE_BYTES`]; not read.
    ReadSizeExcluded,
    /// Read, but over [`MAX_PARSE_BYTES`]; symbols not extracted.
    ParseSizeExcluded,
    /// Recognised as generated source; symbols and edges not extracted.
    GeneratedExcluded,
}

impl ExtractionOutcome {
    pub const ALL: [Self; 10] = [
        Self::Symbols,
        Self::NoDefinitions,
        Self::UnsupportedGrammar,
        Self::LanguageSetupFailed,
        Self::ParseTimeout,
        Self::ParseFailed,
        Self::ReadFailed,
        Self::ReadSizeExcluded,
        Self::ParseSizeExcluded,
        Self::GeneratedExcluded,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Symbols => "symbols",
            Self::NoDefinitions => "no_definitions",
            Self::UnsupportedGrammar => "unsupported_grammar",
            Self::LanguageSetupFailed => "language_setup_failed",
            Self::ParseTimeout => "parse_timeout",
            Self::ParseFailed => "parse_failed",
            Self::ReadFailed => "read_failed",
            Self::ReadSizeExcluded => "read_size_excluded",
            Self::ParseSizeExcluded => "parse_size_excluded",
            Self::GeneratedExcluded => "generated_excluded",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// Whether reference (edge) extraction may parse this file. Only
    /// generated source is withheld; see the module docs.
    pub fn allows_edges(self) -> bool {
        !matches!(
            self,
            Self::GeneratedExcluded | Self::ReadFailed | Self::ReadSizeExcluded
        )
    }
}

/// Symbols plus the outcome that produced them.
#[derive(Debug, Clone)]
pub struct Extraction {
    pub outcome: ExtractionOutcome,
    pub symbols: Vec<Symbol>,
}

impl Extraction {
    fn empty(outcome: ExtractionOutcome) -> Self {
        Self {
            outcome,
            symbols: Vec::new(),
        }
    }
}

/// Lines searched for a generator header.
const GENERATED_HEADER_LINES: usize = 5;

/// Whether `content` declares itself generated: a comment line within the
/// first [`GENERATED_HEADER_LINES`] lines carrying the `@generated` marker
/// (tree-sitter writes `/* Automatically @generated by tree-sitter … */`;
/// the same convention is used by rustfmt, Buck and many codegen tools).
///
/// Deliberately narrow: a file name such as `parser.c`, or the marker
/// anywhere past the header or outside a comment, is not enough, so a
/// hand-written file is never excluded by accident. Only for extensions
/// with a grammar; others are never parsed anyway.
pub fn is_generated_source(ext: &str, content: &str) -> bool {
    if language_for_extension(ext).is_none() {
        return false;
    }
    content.lines().take(GENERATED_HEADER_LINES).any(|line| {
        let t = line.trim_start();
        let is_comment = ["//", "/*", "*", "#", "--", "<!--"]
            .iter()
            .any(|p| t.starts_with(p));
        is_comment && t.contains("@generated")
    })
}

/// Read a source file under the read limit, or say why not.
pub fn read_source(path: &std::path::Path) -> Result<String, ExtractionOutcome> {
    let meta = std::fs::metadata(path).map_err(|_| ExtractionOutcome::ReadFailed)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(ExtractionOutcome::ReadSizeExcluded);
    }
    std::fs::read_to_string(path).map_err(|_| ExtractionOutcome::ReadFailed)
}

/// Extract symbols from already-read `content` under the full policy:
/// grammar check, generated-source check, parse cap, then a parse bounded
/// by the 2 s deadline.
pub fn extract(ext: &str, content: &str) -> Extraction {
    if language_for_extension(ext).is_none() {
        return Extraction::empty(ExtractionOutcome::UnsupportedGrammar);
    }
    if is_generated_source(ext, content) {
        return Extraction::empty(ExtractionOutcome::GeneratedExcluded);
    }
    if content.len() > MAX_PARSE_BYTES {
        return Extraction::empty(ExtractionOutcome::ParseSizeExcluded);
    }
    let started = Instant::now();
    parse_with_deadline(ext, content, &mut || {
        started.elapsed().as_micros() as u64 > PARSE_TIMEOUT_MICROS
    })
}

/// Parse `content` with no size cap, cancelling when `deadline_passed`
/// returns true (tree-sitter polls it periodically during the parse). A
/// cancelled parse is [`ExtractionOutcome::ParseTimeout`]; a parse that
/// yields no tree otherwise is [`ExtractionOutcome::ParseFailed`].
/// Recoverable syntax errors still produce a tree and are not failures.
pub fn parse_with_deadline(
    ext: &str,
    content: &str,
    deadline_passed: &mut dyn FnMut() -> bool,
) -> Extraction {
    let Some(language) = language_for_extension(ext) else {
        return Extraction::empty(ExtractionOutcome::UnsupportedGrammar);
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return Extraction::empty(ExtractionOutcome::LanguageSetupFailed);
    }
    let bytes = content.as_bytes();
    let len = bytes.len();
    let cancelled = std::cell::Cell::new(false);
    let mut progress = |_: &tree_sitter::ParseState| -> bool {
        let stop = deadline_passed();
        if stop {
            cancelled.set(true);
        }
        stop
    };
    let opts = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    let tree = parser.parse_with_options(
        &mut |i, _| if i < len { &bytes[i..] } else { &[] },
        None,
        Some(opts),
    );
    let Some(tree) = tree else {
        return Extraction::empty(if cancelled.get() {
            ExtractionOutcome::ParseTimeout
        } else {
            ExtractionOutcome::ParseFailed
        });
    };
    let mut symbols = Vec::new();
    super::builder::collect_symbols(tree.root_node(), bytes, &mut symbols);
    Extraction {
        outcome: if symbols.is_empty() {
            ExtractionOutcome::NoDefinitions
        } else {
            ExtractionOutcome::Symbols
        },
        symbols,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_names_round_trip() {
        for o in ExtractionOutcome::ALL {
            assert_eq!(ExtractionOutcome::parse(o.as_str()), Some(o));
        }
        assert_eq!(ExtractionOutcome::parse("unknown"), None);
    }

    #[test]
    fn unsupported_grammar_differs_from_valid_zero_definitions() {
        assert_eq!(
            extract("xyz", "pub fn a() {}").outcome,
            ExtractionOutcome::UnsupportedGrammar
        );
        let none = extract("rs", "// only a comment\nmod inner;\n");
        assert_eq!(none.outcome, ExtractionOutcome::NoDefinitions);
        assert!(none.symbols.is_empty());
        let some = extract("rs", "pub fn a() {}\n");
        assert_eq!(some.outcome, ExtractionOutcome::Symbols);
        assert_eq!(some.symbols[0].name, "a");
    }

    #[test]
    fn parse_cap_is_inclusive_and_excludes_above() {
        let line = "pub fn f() {}\n";
        let at = format!("{}{}", line, " ".repeat(MAX_PARSE_BYTES - line.len()));
        assert_eq!(at.len(), MAX_PARSE_BYTES);
        assert_eq!(extract("rs", &at).outcome, ExtractionOutcome::Symbols);
        let above = format!("{at} ");
        assert_eq!(
            extract("rs", &above).outcome,
            ExtractionOutcome::ParseSizeExcluded
        );
    }

    #[test]
    fn generated_detection_needs_a_header_comment() {
        let tree_sitter_header =
            "/* Automatically @generated by tree-sitter v0.25.10 */\n\n#include \"tree_sitter/parser.h\"\n";
        assert!(is_generated_source("c", tree_sitter_header));
        assert!(is_generated_source("rs", "// @generated by build.rs\npub fn f() {}\n"));
        assert!(is_generated_source("py", "#!/usr/bin/env python\n# @generated\ndef f(): pass\n"));
        // A hand-written parser.c is not generated just by its name.
        assert!(!is_generated_source("c", "/* parser for the config DSL */\nint parse(void);\n"));
        // The marker outside a comment, or past the header, does not count.
        assert!(!is_generated_source("rs", "const TAG: &str = \"@generated\";\n"));
        let late = format!("{}// @generated\n", "fn f() {}\n".repeat(10));
        assert!(!is_generated_source("rs", &late));
        // No grammar, no exclusion.
        assert!(!is_generated_source("md", "<!-- @generated -->\n"));

        let out = extract("c", tree_sitter_header);
        assert_eq!(out.outcome, ExtractionOutcome::GeneratedExcluded);
        assert!(out.symbols.is_empty());
        assert!(!out.outcome.allows_edges());
        // The lookalike is parsed, not excluded. (C definitions carry their
        // name under `declarator`, not `name`, so the walk finds none —
        // a pre-existing limit, not part of this policy.)
        assert_eq!(
            extract("c", "/* parser for the config DSL */\nint parse(void) { return 0; }\n")
                .outcome,
            ExtractionOutcome::NoDefinitions,
            "lookalike stays eligible"
        );
    }

    /// Deep nesting under the parse cap must not overflow the stack (the
    /// walk used to recurse once per tree level). Runs on a 1 MiB thread so
    /// a regression fails here instead of only on the main thread's 8 MiB.
    #[test]
    fn deeply_nested_input_extracts_without_overflowing() {
        let depth = 60_000;
        let src = format!(
            "pub fn before() {{}}\npub fn deep() -> i32 {{ {}1{} }}\npub fn after() {{}}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        );
        assert!(src.len() <= MAX_PARSE_BYTES);
        let out = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || extract("rs", &src))
            .unwrap()
            .join()
            .expect("extraction must not overflow");
        assert_eq!(out.outcome, ExtractionOutcome::Symbols);
        let names: Vec<&str> = out.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["before", "deep", "after"], "pre-order preserved");
    }

    /// A hand-written file between the old 200 KB cap and the new one is
    /// now extracted.
    #[test]
    fn handwritten_source_above_the_old_cap_is_extracted() {
        let mut src = String::from("pub fn gv_first() {}\n");
        while src.len() <= 250_000 {
            src.push_str("pub fn gv_filler() { let _ = 1; }\n");
        }
        src.push_str("pub fn gv_last() {}\n");
        assert!(src.len() > 200_000 && src.len() <= MAX_PARSE_BYTES);
        let out = extract("rs", &src);
        assert_eq!(out.outcome, ExtractionOutcome::Symbols);
        assert_eq!(out.symbols.last().unwrap().name, "gv_last");
    }

    #[test]
    fn syntax_errors_still_extract() {
        let out = extract("rs", "pub fn good() {}\nfn broken( {\n");
        assert_eq!(out.outcome, ExtractionOutcome::Symbols);
        assert!(out.symbols.iter().any(|s| s.name == "good"));
    }

    /// Injected cancellation, no wall clock: the hook says "deadline
    /// passed" on its first poll.
    #[test]
    fn a_triggered_deadline_is_a_timeout() {
        let content = "pub fn f() { let x = 1 + 2; }\n".repeat(5_000);
        let out = parse_with_deadline("rs", &content, &mut || true);
        assert_eq!(out.outcome, ExtractionOutcome::ParseTimeout);
        assert!(out.symbols.is_empty());
        let out = parse_with_deadline("rs", &content, &mut || false);
        assert_eq!(out.outcome, ExtractionOutcome::Symbols);
    }

    #[test]
    fn read_source_reports_size_and_read_failures() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.rs");
        std::fs::write(&big, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert_eq!(read_source(&big), Err(ExtractionOutcome::ReadSizeExcluded));
        let binary = dir.path().join("bad.rs");
        std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x80]).unwrap();
        assert_eq!(read_source(&binary), Err(ExtractionOutcome::ReadFailed));
        assert_eq!(
            read_source(&dir.path().join("absent.rs")),
            Err(ExtractionOutcome::ReadFailed)
        );
        let ok = dir.path().join("ok.rs");
        std::fs::write(&ok, "fn a() {}").unwrap();
        assert_eq!(read_source(&ok).as_deref(), Ok("fn a() {}"));
    }
}
