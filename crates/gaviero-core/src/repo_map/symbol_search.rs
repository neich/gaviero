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
}
