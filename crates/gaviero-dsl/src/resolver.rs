//! `include` resolution.
//!
//! Reads an entry `.gaviero` script and recursively splices in any files
//! reached via top-level `include "path"` statements. Paths are resolved
//! relative to the directory of the file containing the include. Cycles are
//! detected via canonicalized [`PathBuf`]s.
//!
//! Output:
//! - A flattened [`Script`] whose `items` no longer contain [`Item::Include`].
//! - A `sources: Vec<(filename, content)>` parallel array. Every top-level
//!   declaration in the flattened script has its `file_id` set to its index
//!   in `sources`, so the compiler can route diagnostics to the right
//!   [`miette::NamedSource`].
//!
//! The entry file is always `sources[0]`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chumsky::span::SimpleSpan;
use miette::NamedSource;

use crate::ast::{Item, Script};
use crate::error::{DslError, DslErrors};
use crate::lexer;
use crate::parser;

/// Resolve all `include` directives starting from `entry_path`.
///
/// Returns the merged script (no `Include` items remain) and the per-file
/// source map. Errors point back at the actual file/line that triggered
/// them — lex/parse errors in an included file carry that file's
/// [`NamedSource`], not the entry file's.
pub fn resolve(entry_path: &Path) -> Result<(Script, Vec<(String, String)>), DslErrors> {
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut path_to_id: HashMap<PathBuf, u32> = HashMap::new();
    let mut visiting: Vec<PathBuf> = Vec::new();
    let mut merged_items: Vec<Item> = Vec::new();
    let mut errors: Vec<DslError> = Vec::new();

    visit_file(
        entry_path,
        None, // no parent include statement for the entry file
        None, // no parent file_id
        &mut sources,
        &mut path_to_id,
        &mut visiting,
        &mut merged_items,
        &mut errors,
    );

    if !errors.is_empty() {
        return Err(DslErrors::new(errors));
    }
    Ok((Script { items: merged_items }, sources))
}

#[allow(clippy::too_many_arguments)]
fn visit_file(
    raw_path: &Path,
    include_span: Option<SimpleSpan>,
    parent_file_id: Option<u32>,
    sources: &mut Vec<(String, String)>,
    path_to_id: &mut HashMap<PathBuf, u32>,
    visiting: &mut Vec<PathBuf>,
    merged_items: &mut Vec<Item>,
    errors: &mut Vec<DslError>,
) {
    // Canonicalize so symlinks and `./` / `..` variants of the same file are
    // treated as a single node — this is what makes cycle detection sound.
    let canonical = match raw_path.canonicalize() {
        Ok(p) => p,
        Err(io_err) => {
            errors.push(make_include_error(
                include_span,
                parent_file_id,
                sources,
                raw_path,
                format!("cannot resolve include `{}`: {}", raw_path.display(), io_err),
            ));
            return;
        }
    };

    if visiting.contains(&canonical) {
        let chain = visiting
            .iter()
            .chain(std::iter::once(&canonical))
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(" → ");
        errors.push(make_include_error(
            include_span,
            parent_file_id,
            sources,
            raw_path,
            format!("include cycle detected: {}", chain),
        ));
        return;
    }

    // De-duplicate: a file already merged once is silently skipped on
    // subsequent includes — `include "common.gaviero"` from two libraries
    // should be idempotent, not produce duplicate-name errors.
    if path_to_id.contains_key(&canonical) {
        return;
    }

    let content = match std::fs::read_to_string(&canonical) {
        Ok(s) => s,
        Err(io_err) => {
            errors.push(make_include_error(
                include_span,
                parent_file_id,
                sources,
                raw_path,
                format!("reading `{}`: {}", canonical.display(), io_err),
            ));
            return;
        }
    };

    let filename = canonical.display().to_string();
    let file_id = sources.len() as u32;
    sources.push((filename.clone(), content.clone()));
    path_to_id.insert(canonical.clone(), file_id);

    // Lex + parse this file. Errors carry the file's own NamedSource.
    let (tokens, lex_errors) = lexer::lex(&content);
    if !lex_errors.is_empty() {
        for span in lex_errors {
            errors.push(DslError::Lex {
                src: NamedSource::new(&filename, content.clone()),
                span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
            });
        }
        return;
    }

    let (ast, parse_errors) = parser::parse(&tokens, &content, &filename);
    if !parse_errors.is_empty() {
        errors.extend(parse_errors);
        return;
    }
    let Some(ast) = ast else {
        return;
    };

    // Recurse into includes BEFORE appending this file's own items, so
    // included items appear before the including file's items in the merged
    // stream. (Order is irrelevant for correctness — the compiler treats
    // top-level decls as a set — but earlier-declared = first-seen, which
    // makes "later wins" duplicate semantics intuitive if we ever change.)
    visiting.push(canonical.clone());
    let parent_dir = canonical.parent().unwrap_or(Path::new("."));
    for item in ast.items {
        match item {
            Item::Include(inc) => {
                let resolved = parent_dir.join(&inc.path);
                visit_file(
                    &resolved,
                    Some(inc.path_span),
                    Some(file_id),
                    sources,
                    path_to_id,
                    visiting,
                    merged_items,
                    errors,
                );
            }
            mut other => {
                set_file_id(&mut other, file_id);
                merged_items.push(other);
            }
        }
    }
    visiting.pop();
}

/// Stamp `file_id` onto every top-level decl inside `item`. Vars carry no
/// span (compile-time substitution only) so they get implicit fid 0.
fn set_file_id(item: &mut Item, file_id: u32) {
    match item {
        Item::Client(c) => c.file_id = file_id,
        Item::Agent(a) => a.file_id = file_id,
        Item::Workflow(w) => w.file_id = file_id,
        Item::Prompt(p) => p.file_id = file_id,
        Item::TierAlias(t) => t.file_id = file_id,
        Item::Vars(_) => {}
        Item::Include(_) => unreachable!("includes are resolved away before reaching merged_items"),
    }
}

/// Build an `Include` diagnostic that points at the offending include
/// statement (when known). Falls back to the include's own file when the
/// include is the entry script (which has no parent).
fn make_include_error(
    include_span: Option<SimpleSpan>,
    parent_file_id: Option<u32>,
    sources: &[(String, String)],
    raw_path: &Path,
    reason: String,
) -> DslError {
    if let (Some(span), Some(fid)) = (include_span, parent_file_id) {
        let (filename, content) = &sources[fid as usize];
        DslError::Include {
            src: NamedSource::new(filename, content.clone()),
            span: (span.start, span.end.saturating_sub(span.start).max(1)).into(),
            reason,
        }
    } else {
        // Entry-file failure (e.g. missing entry path). No parent source to
        // anchor against, so synthesize a minimal NamedSource.
        DslError::Include {
            src: NamedSource::new(raw_path.display().to_string(), String::new()),
            span: (0, 1).into(),
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn resolves_simple_include() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "lib.gaviero",
            r#"client base { tier cheap model "claude:sonnet" }"#,
        );
        let entry = write_file(
            tmp.path(),
            "main.gaviero",
            r#"
                include "lib.gaviero"
                agent worker { client base prompt "hi" }
                workflow w { steps [worker] }
            "#,
        );
        let (script, sources) = resolve(&entry).unwrap();
        assert_eq!(sources.len(), 2);
        // Items: 1 client (from lib) + 1 agent + 1 workflow (from entry)
        let mut clients = 0;
        let mut agents = 0;
        let mut workflows = 0;
        for item in &script.items {
            match item {
                Item::Client(c) => {
                    clients += 1;
                    assert_eq!(c.file_id, 1, "client originates from included lib");
                }
                Item::Agent(a) => {
                    agents += 1;
                    assert_eq!(a.file_id, 0, "agent originates from entry");
                }
                Item::Workflow(w) => {
                    workflows += 1;
                    assert_eq!(w.file_id, 0);
                }
                _ => {}
            }
        }
        assert_eq!(clients, 1);
        assert_eq!(agents, 1);
        assert_eq!(workflows, 1);
    }

    #[test]
    fn detects_cycle() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "a.gaviero",
            r#"include "b.gaviero""#,
        );
        write_file(
            tmp.path(),
            "b.gaviero",
            r#"include "a.gaviero""#,
        );
        let entry = tmp.path().join("a.gaviero");
        let err = resolve(&entry).unwrap_err();
        let any_cycle = err
            .errors
            .iter()
            .any(|e| matches!(e, DslError::Include { reason, .. } if reason.contains("cycle")));
        assert!(any_cycle, "expected cycle error, got {:?}", err.errors);
    }

    #[test]
    fn missing_include_reports_with_parent_span() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = write_file(
            tmp.path(),
            "main.gaviero",
            r#"include "does_not_exist.gaviero""#,
        );
        let err = resolve(&entry).unwrap_err();
        assert!(err.errors.iter().any(|e| matches!(e, DslError::Include { .. })));
    }

    #[test]
    fn duplicate_include_is_deduplicated() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "common.gaviero",
            r#"client base { tier cheap model "claude:sonnet" }"#,
        );
        write_file(
            tmp.path(),
            "lib1.gaviero",
            r#"include "common.gaviero""#,
        );
        write_file(
            tmp.path(),
            "lib2.gaviero",
            r#"include "common.gaviero""#,
        );
        let entry = write_file(
            tmp.path(),
            "main.gaviero",
            r#"
                include "lib1.gaviero"
                include "lib2.gaviero"
            "#,
        );
        let (script, sources) = resolve(&entry).unwrap();
        // common.gaviero, lib1.gaviero, lib2.gaviero, main.gaviero = 4 sources max,
        // but common is reached only once → single client decl.
        let client_count = script
            .items
            .iter()
            .filter(|i| matches!(i, Item::Client(_)))
            .count();
        assert_eq!(client_count, 1);
        // Sources may include common.gaviero exactly once even if reached via
        // two paths.
        let common_count = sources
            .iter()
            .filter(|(name, _)| name.ends_with("common.gaviero"))
            .count();
        assert_eq!(common_count, 1);
    }

    #[test]
    fn relative_paths_resolve_against_including_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "lib/inner.gaviero",
            r#"client base { tier cheap model "claude:sonnet" }"#,
        );
        write_file(
            tmp.path(),
            "lib/outer.gaviero",
            // outer.gaviero is in lib/, so "inner.gaviero" resolves to lib/inner.gaviero
            r#"include "inner.gaviero""#,
        );
        let entry = write_file(
            tmp.path(),
            "main.gaviero",
            r#"include "lib/outer.gaviero""#,
        );
        let (script, _) = resolve(&entry).unwrap();
        let client_count = script
            .items
            .iter()
            .filter(|i| matches!(i, Item::Client(_)))
            .count();
        assert_eq!(client_count, 1);
    }
}
