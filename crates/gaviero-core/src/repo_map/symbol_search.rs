//! Brute-force cosine search over `symbol_docs` embeddings (S2.2 / PR-3).

use super::store::{GraphStore, SymbolDoc};

/// One ranked symbol-search hit.
#[derive(Debug, Clone)]
pub struct ScoredSymbolDoc {
    pub doc: SymbolDoc,
    pub score: f32,
}

/// Rank symbol sidecar rows by cosine similarity to `query_embedding`.
/// Rows without an embedding BLOB are skipped.
pub fn search_symbol_docs(
    store: &GraphStore,
    query_embedding: &[f32],
    limit: usize,
) -> anyhow::Result<Vec<ScoredSymbolDoc>> {
    let mut scored: Vec<ScoredSymbolDoc> = store
        .all_symbol_docs()?
        .into_iter()
        .filter_map(|doc| {
            let emb = doc.embedding.as_ref()?;
            Some(ScoredSymbolDoc {
                score: cosine_similarity(query_embedding, emb),
                doc,
            })
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    Ok(scored)
}

/// G2 / OD-2: refuse cross-model cosine. `stamp` is the sidecar's
/// `graph_meta("symbol_embedder")`; `query_name` / `memory_name` are
/// built-embedder ids (`Embedder::name()`), never settings aliases. A
/// missing stamp is tolerated only when the query embedder *is* the
/// memory embedder — pre-stamp sidecars were built exactly that way
/// (the old `"inherit"` default); anything else needs a re-enrich.
pub fn check_symbol_embedder_stamp(
    stamp: Option<&str>,
    query_name: &str,
    memory_name: &str,
) -> anyhow::Result<()> {
    match stamp {
        Some(s) if s == query_name => Ok(()),
        Some(s) => anyhow::bail!(
            "symbol_docs sidecar was embedded with `{s}` but this query embeds with \
             `{query_name}` — cross-model cosine is meaningless; re-run \
             `gaviero-cli --graph --enrich` to rebuild symbol vectors"
        ),
        None if query_name == memory_name => Ok(()),
        None => anyhow::bail!(
            "symbol_docs sidecar carries no embedder stamp (predates \
             `repoMap.embedder.model`) while queries embed with `{query_name}` — \
             re-run `gaviero-cli --graph --enrich` to rebuild and stamp symbol vectors"
        ),
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a <= f32::EPSILON || norm_b <= f32::EPSILON {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_map::store::{GraphStore, SymbolDoc};

    #[test]
    fn search_ranks_by_cosine() {
        let store = GraphStore::open_memory().unwrap();
        store
            .upsert_symbol_doc(&SymbolDoc {
                qualified_name: "a.rs::foo".into(),
                file_path: "a.rs".into(),
                file_hash: None,
                signature: "fn foo()".into(),
                bounds: String::new(),
                doc: String::new(),
                role_summary: String::new(),
                embedding: Some(vec![1.0, 0.0, 0.0]),
            })
            .unwrap();
        store
            .upsert_symbol_doc(&SymbolDoc {
                qualified_name: "a.rs::bar".into(),
                file_path: "a.rs".into(),
                file_hash: None,
                signature: "fn bar()".into(),
                bounds: String::new(),
                doc: String::new(),
                role_summary: String::new(),
                embedding: Some(vec![0.0, 1.0, 0.0]),
            })
            .unwrap();
        let hits = search_symbol_docs(&store, &[0.9, 0.1, 0.0], 5).unwrap();
        assert_eq!(hits.len(), 2);
        assert!(hits[0].doc.qualified_name.contains("foo"));
    }

    /// G2 / OD-2 stamp semantics: match passes; mismatch and
    /// missing-stamp-with-divergent-embedder fail with the re-enrich
    /// remedy; missing stamp under the legacy inherit setup passes.
    #[test]
    fn embedder_stamp_check_semantics() {
        use super::check_symbol_embedder_stamp as check;
        assert!(
            check(
                Some("jina-embeddings-v2-base-code"),
                "jina-embeddings-v2-base-code",
                "nomic-embed-text-v1.5"
            )
            .is_ok()
        );
        assert!(check(None, "nomic-embed-text-v1.5", "nomic-embed-text-v1.5").is_ok());

        let err = check(
            Some("nomic-embed-text-v1.5"),
            "jina-embeddings-v2-base-code",
            "nomic-embed-text-v1.5",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("--graph --enrich"), "{err}");

        let err = check(
            None,
            "jina-embeddings-v2-base-code",
            "nomic-embed-text-v1.5",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("no embedder stamp"), "{err}");
    }
}
