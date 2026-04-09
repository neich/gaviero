//! Indent query loading and caching.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, bail};

/// Cache of compiled indent queries, keyed by language name.
pub struct IndentQueryCache {
    cache: HashMap<String, Option<Arc<tree_sitter::Query>>>,
}

impl IndentQueryCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Get or load the indent query for a language.
    ///
    /// Returns `None` if no `indents.scm` file exists for the language
    /// (the caller should fall back to bracket counting).
    /// Caches the result so subsequent calls are free.
    pub fn get_or_load(
        &mut self,
        lang_name: &str,
        ts_language: &tree_sitter::Language,
    ) -> Option<Arc<tree_sitter::Query>> {
        if let Some(cached) = self.cache.get(lang_name) {
            return cached.clone();
        }

        let result = load_indent_query(lang_name, ts_language);
        match result {
            Ok(query) => {
                let arc = Arc::new(query);
                self.cache.insert(lang_name.to_string(), Some(arc.clone()));
                Some(arc)
            }
            Err(e) => {
                tracing::debug!("No indent query for {}: {}", lang_name, e);
                self.cache.insert(lang_name.to_string(), None);
                None
            }
        }
    }
}

/// Load and compile an indent query for a language.
fn load_indent_query(lang_name: &str, ts_language: &tree_sitter::Language) -> Result<tree_sitter::Query> {
    let scm = find_indent_query_file(lang_name)?;
    tree_sitter::Query::new(ts_language, &scm)
        .map_err(|e| anyhow::anyhow!("indent query error for {}: {}", lang_name, e))
}

/// Find an `indents.scm` file for a language using the shared query loader.
fn find_indent_query_file(lang: &str) -> Result<String> {
    crate::query_loader::find_query_file(lang, "indents.scm", |l, _| bundled_indent_query(l))
}

/// Bundled indent queries compiled into the binary.
fn bundled_indent_query(lang: &str) -> Result<String> {
    match lang {
        "rust" => Ok(include_str!("../../../../queries/rust/indents.scm").to_string()),
        "javascript" => Ok(include_str!("../../../../queries/javascript/indents.scm").to_string()),
        "typescript" => Ok(include_str!("../../../../queries/typescript/indents.scm").to_string()),
        "c" => Ok(include_str!("../../../../queries/c/indents.scm").to_string()),
        "cpp" => Ok(include_str!("../../../../queries/cpp/indents.scm").to_string()),
        "java" => Ok(include_str!("../../../../queries/java/indents.scm").to_string()),
        "json" => Ok(include_str!("../../../../queries/json/indents.scm").to_string()),
        "html" => Ok(include_str!("../../../../queries/html/indents.scm").to_string()),
        "css" => Ok(include_str!("../../../../queries/css/indents.scm").to_string()),
        "toml" => Ok(include_str!("../../../../queries/toml/indents.scm").to_string()),
        "bash" => Ok(include_str!("../../../../queries/bash/indents.scm").to_string()),
        "latex" => Ok(include_str!("../../../../queries/latex/indents.scm").to_string()),
        "gaviero" => Ok(include_str!("../../../../queries/gaviero/indents.scm").to_string()),
        _ => bail!("no bundled indents.scm for {}", lang),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree_sitter::language_for_extension;

    #[test]
    fn test_indent_queries_compile_for_all_languages() {
        let languages = [
            ("rs", "rust"),
            ("js", "javascript"),
            ("ts", "typescript"),
            ("c", "c"),
            ("cpp", "cpp"),
            ("java", "java"),
            ("json", "json"),
            ("html", "html"),
            ("css", "css"),
            ("toml", "toml"),
            ("sh", "bash"),
            ("tex", "latex"),
            ("gaviero", "gaviero"),
        ];

        let mut cache = IndentQueryCache::new();
        for (ext, lang_name) in &languages {
            let ts_lang = language_for_extension(ext)
                .unwrap_or_else(|| panic!("no grammar for {}", ext));
            let query = cache.get_or_load(lang_name, &ts_lang);
            assert!(
                query.is_some(),
                "indent query for {} should compile",
                lang_name
            );
        }
    }
}
