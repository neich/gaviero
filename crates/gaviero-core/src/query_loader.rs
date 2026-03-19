//! Shared query file discovery for tree-sitter `.scm` files.
//!
//! Used by both highlight and indent query loading. Searches:
//! 1. `$GAVIERO_QUERIES/{lang}/{file}`
//! 2. Relative to executable
//! 3. Current working directory
//! 4. Compile-time bundled fallback (caller-provided)

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Find a query file on disk, falling back to a bundled version.
///
/// `bundled_fallback` is called with `(lang, file)` and should return
/// the compile-time `include_str!` content, or an error if not bundled.
pub fn find_query_file<F>(lang: &str, file: &str, bundled_fallback: F) -> Result<String>
where
    F: FnOnce(&str, &str) -> Result<String>,
{
    // 1. $GAVIERO_QUERIES env var
    if let Ok(queries_dir) = std::env::var("GAVIERO_QUERIES") {
        let path = PathBuf::from(&queries_dir).join(lang).join(file);
        if path.exists() {
            return std::fs::read_to_string(&path)
                .with_context(|| format!("reading query file: {}", path.display()));
        }
    }

    // 2. Relative to executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            for base in &[exe_dir.join("..").join("queries"), exe_dir.join("queries")] {
                let path = base.join(lang).join(file);
                if path.exists() {
                    return std::fs::read_to_string(&path)
                        .with_context(|| format!("reading query file: {}", path.display()));
                }
            }
        }
    }

    // 3. Current working directory
    let path = PathBuf::from("queries").join(lang).join(file);
    if path.exists() {
        return std::fs::read_to_string(&path)
            .with_context(|| format!("reading query file: {}", path.display()));
    }

    // 4. Compile-time bundled fallback
    bundled_fallback(lang, file)
}
