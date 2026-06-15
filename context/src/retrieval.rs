//! Hybrid retrieval primitives: BM25, RRF fusion, graph boost, lexical
//! reranker.
//!
//! All functions are pure and dependency-free.  The orchestration that
//! ties them into `ContextEngine::retrieve_hybrid` lives in `engine.rs`.

use std::collections::HashMap;

use crate::graph::{EdgeKind, KnowledgeGraph, SymbolId};
use crate::vector::Chunk;

// =============================================================================
// Config
// =============================================================================

/// Tunable knobs for the hybrid retrieval pipeline.
#[derive(Debug, Clone)]
pub struct RetrievalConfig {
    /// Final number of results returned to the caller.
    pub top_k: usize,
    /// Number of candidates fetched from *each* ranker before fusion.
    pub candidate_k: usize,
    /// Reciprocal-Rank-Fusion constant.  Higher `k` flattens the rank
    /// signal; 60 is the standard value from the literature.
    pub rrf_k: usize,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            top_k: 10,
            candidate_k: 50,
            rrf_k: 60,
        }
    }
}

// =============================================================================
// BM25
// =============================================================================

/// Compute IDF for each query term across the provided chunks.
///
/// Standard formula: `ln((N - df + 0.5) / (df + 0.5) + 1)`.
pub fn compute_idf(query_terms: &[String], chunks: &[&Chunk]) -> HashMap<String, f32> {
    let n = chunks.len() as f32;
    let mut df: HashMap<&str, usize> = HashMap::new();

    for term in query_terms {
        for chunk in chunks {
            if chunk.text.contains(term.as_str()) {
                *df.entry(term.as_str()).or_insert(0) += 1;
            }
        }
    }

    query_terms
        .iter()
        .map(|t| {
            let df_val = df.get(t.as_str()).copied().unwrap_or(0) as f32;
            let idf = ((n - df_val + 0.5) / (df_val + 0.5) + 1.0).ln();
            (t.clone(), idf)
        })
        .collect()
}

/// BM25 score for a single document.
///
/// Standard parameters: k1 = 1.5, b = 0.75.  Average document length
/// is estimated at 50 tokens (typical code chunk).
pub fn bm25_score(query_terms: &[String], doc: &str, idf: &HashMap<String, f32>) -> f32 {
    let doc_len = doc.split_whitespace().count() as f32;
    let avg_dl = 50.0_f32;
    let k1 = 1.5_f32;
    let b = 0.75_f32;
    let mut score = 0.0_f32;

    for term in query_terms {
        let tf = doc.matches(term.as_str()).count() as f32;
        if tf == 0.0 {
            continue;
        }
        let idf_val = idf.get(term.as_str()).copied().unwrap_or(0.0);
        let norm = 1.0 - b + b * doc_len / avg_dl;
        score += idf_val * (tf * (k1 + 1.0)) / (tf + k1 * norm);
    }
    score
}

// =============================================================================
// RRF fusion
// =============================================================================

/// Reciprocal Rank Fusion score for a single rank position.
///
/// `score = 1.0 / (k + rank)` where `rank` is 0-indexed.
pub fn rrf_score(rank: usize, k: usize) -> f32 {
    1.0 / (k as f32 + rank as f32 + 1.0)
}

// =============================================================================
// Graph boost
// =============================================================================

/// Boost score for a chunk based on graph-structural proximity to
/// query-matched symbols.
///
/// - Direct symbol match → 1.0
/// - 1-hop caller/callee/reference/contains/implements → 0.5
/// - No relationship → 0.0
pub fn graph_boost(
    chunk_id: u64,
    query_sym_ids: &[SymbolId],
    graph: &KnowledgeGraph,
    chunk_symbols: &HashMap<u64, Vec<String>>,
) -> f32 {
    if query_sym_ids.is_empty() {
        return 0.0;
    }

    let Some(sym_names) = chunk_symbols.get(&chunk_id) else {
        return 0.0;
    };

    let chunk_sym_ids: Vec<SymbolId> = sym_names
        .iter()
        .flat_map(|name| graph.find_by_name(name))
        .collect();

    if chunk_sym_ids.is_empty() {
        return 0.0;
    }

    let hops = [
        EdgeKind::Calls,
        EdgeKind::References,
        EdgeKind::Contains,
        EdgeKind::Implements,
    ];

    for &qsid in query_sym_ids {
        for &csid in &chunk_sym_ids {
            if qsid == csid {
                return 1.0;
            }
            for kind in &hops {
                if graph.neighbors(qsid, *kind).contains(&csid) {
                    return 0.5;
                }
                if graph.neighbors(csid, *kind).contains(&qsid) {
                    return 0.5;
                }
            }
        }
    }
    0.0
}

// =============================================================================
// Lexical reranker
// =============================================================================

/// Token-overlap score: fraction of query terms found in the document.
///
/// Case-insensitive substring match (good enough for code identifiers).
pub fn lexical_score(query_terms: &[String], doc: &str) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let doc_lower = doc.to_lowercase();
    let hits = query_terms
        .iter()
        .filter(|t| doc_lower.contains(&t.to_lowercase()))
        .count();
    hits as f32 / query_terms.len() as f32
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: u64, text: &str) -> Chunk {
        Chunk {
            id,
            file: std::path::PathBuf::from("test.rs"),
            start_line: 1,
            end_line: 5,
            text: text.to_string(),
        }
    }

    #[test]
    fn bm25_scores_relevant_chunk_higher() {
        let terms = vec!["alpha".to_string()];
        let c1 = chunk(1, "fn alpha() { alpha() }");
        let c2 = chunk(2, "fn beta() { beta() }");
        let chunks = vec![&c1, &c2];
        let idf = compute_idf(&terms, &chunks);

        let s1 = bm25_score(&terms, &c1.text, &idf);
        let s2 = bm25_score(&terms, &c2.text, &idf);
        assert!(s1 > s2, "alpha chunk should score higher: {s1} vs {s2}");
    }

    #[test]
    fn bm25_empty_query_returns_zero() {
        let terms: Vec<String> = vec![];
        let c = chunk(1, "fn alpha() {}");
        let chunks = vec![&c];
        let idf = compute_idf(&terms, &chunks);
        assert_eq!(bm25_score(&terms, &c.text, &idf), 0.0);
    }

    #[test]
    fn rrf_fuses_two_rankings() {
        // Chunk in both vector rank 0 and BM25 rank 0
        let s = rrf_score(0, 60) + rrf_score(0, 60);
        assert!((s - 2.0 / 61.0).abs() < 1e-6);
    }

    #[test]
    fn rrf_one_missing_gives_score() {
        // Chunk only in vector rank 0, not in BM25 → rank = candidate_k
        let s = rrf_score(0, 60) + rrf_score(49, 60);
        assert!(s > 0.0, "should still have positive score");
    }

    #[test]
    fn graph_boost_direct_match() {
        use crate::graph::KnowledgeGraph;
        use std::path::PathBuf;

        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("a.rs"),
            "fn hello() {}\n",
            &crate::lang::LanguageRegistry::new(),
        )
        .unwrap();

        let hello_id = g.find_by_name("hello")[0];
        let mut chunk_syms = std::collections::HashMap::new();
        chunk_syms.insert(1u64, vec!["hello".to_string()]);

        let boost = graph_boost(1, &[hello_id], &g, &chunk_syms);
        assert_eq!(boost, 1.0);
    }

    #[test]
    fn graph_boost_one_hop() {
        use crate::graph::KnowledgeGraph;
        use std::path::PathBuf;

        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("a.rs"),
            "fn caller() { callee() }\nfn callee() {}\n",
            &crate::lang::LanguageRegistry::new(),
        )
        .unwrap();

        let caller_id = g.find_by_name("caller")[0];
        // chunk has the callee symbol
        let mut chunk_syms = std::collections::HashMap::new();
        chunk_syms.insert(1u64, vec!["callee".to_string()]);

        let boost = graph_boost(1, &[caller_id], &g, &chunk_syms);
        assert_eq!(boost, 0.5);
    }

    #[test]
    fn graph_boost_no_relation() {
        use crate::graph::KnowledgeGraph;
        use std::path::PathBuf;

        let mut g = KnowledgeGraph::new();
        g.add_file(
            &PathBuf::from("a.rs"),
            "fn alpha() {}\nfn beta() {}\n",
            &crate::lang::LanguageRegistry::new(),
        )
        .unwrap();

        let alpha_id = g.find_by_name("alpha")[0];
        let mut chunk_syms = std::collections::HashMap::new();
        chunk_syms.insert(1u64, vec!["beta".to_string()]);

        let boost = graph_boost(1, &[alpha_id], &g, &chunk_syms);
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn lexical_full_overlap() {
        let terms = vec!["fn".to_string(), "alpha".to_string()];
        let score = lexical_score(&terms, "fn alpha() {}");
        assert_eq!(score, 1.0);
    }

    #[test]
    fn lexical_partial_overlap() {
        let terms = vec!["fn".to_string(), "alpha".to_string(), "missing".to_string()];
        let score = lexical_score(&terms, "fn alpha() {}");
        assert!((score - 2.0 / 3.0).abs() < 1e-6);
    }
}
