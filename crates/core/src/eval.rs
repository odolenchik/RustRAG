/// Retrieval evaluation metrics for assessing search quality.
///
/// Provides Mean Reciprocal Rank (MRR) and hit-rate metrics that measure how
/// well the retriever surfaces relevant chunks relative to ground-truth labels.

use crate::vector_store::SearchResult;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Ground truth for evaluation
// ---------------------------------------------------------------------------

/// A single (query, expected_chunk_id) label pair used to compute MRR.
#[derive(Debug, Clone)]
pub struct Label {
    /// The query text.
    pub query: String,
    /// ID of the chunk that *should* appear in the top results for this query.
    /// Multiple ground-truth IDs may be listed; a hit counts if **any** appears
    /// within `top_k` ranked positions.
    pub expected_ids: Vec<String>,
}

/// Evaluation results returned by [`evaluate_mrr`].
#[derive(Debug, Clone)]
pub struct EvaluationReport {
    /// Number of labelled queries processed.
    pub query_count: usize,
    /// Mean Reciprocal Rank across all labels.  Reciprocal rank for a single
    /// label is `1 / rank_of_first_relevant` — if the relevant chunk never
    /// appears in top_k, RR = 0.
    pub mrr: f64,
    /// Number of queries where at least one ground-truth chunk appeared in
    /// the returned results (hit-rate @top_k).
    pub hits: usize,
    /// Breakdown per query — useful for spotting systematic failures.
    pub details: Vec<EvaluationDetail>,
}

/// Per-query evaluation detail.
#[derive(Debug, Clone)]
pub struct EvaluationDetail {
    pub query: String,
    /// Position of the first relevant chunk (1-based), or 0 if none found.
    pub rank: usize,
    /// Whether any ground-truth chunk appeared in results.
    pub hit: bool,
}

/// Compute Mean Reciprocal Rank for a set of labelled queries.
///
/// For each label the caller supplies `retrieve_fn`, which should return the
/// ranked results **at the requested top_k** (the function is called with the
/// same `top_k` value as will be used in evaluation).  The retriever's output
/// is checked against `expected_ids`; the reciprocal rank for this query is
/// `1 / position_of_first_matching_chunk`, where position is 1-based.  If no
/// matching chunk is found, RR = 0.
///
/// MRR = mean(reciprocal_ranks).
pub fn evaluate_mrr<F>(labels: &[Label], top_k: usize, retrieve_fn: F) -> EvaluationReport
where
    F: Fn(&str, usize) -> Vec<SearchResult>,
{
    let mut details: Vec<EvaluationDetail> = Vec::new();
    let mut mrr_sum = 0.0;
    let mut hits = 0usize;

    for label in labels {
        let results = retrieve_fn(&label.query, top_k);
        let rank = find_rank_of_first_relevant(&results, &label.expected_ids, top_k);
        let hit = rank > 0;

        if hit {
            mrr_sum += 1.0 / (rank as f64);
            hits += 1;
        }

        details.push(EvaluationDetail {
            query: label.query.clone(),
            rank,
            hit,
        });
    }

    let mrr = if labels.is_empty() {
        0.0
    } else {
        mrr_sum / (labels.len() as f64)
    };

    EvaluationReport {
        query_count: labels.len(),
        mrr,
        hits,
        details,
    }
}

/// Return the 1-based position of the first chunk whose `id` matches any in
/// `expected_ids`, or **0** if none match.
fn find_rank_of_first_relevant(
    results: &[SearchResult],
    expected_ids: &[String],
    top_k: usize,
) -> usize {
    let limit = top_k.min(results.len());
    for i in 0..limit {
        if expected_ids.contains(&results[i].id) {
            return i + 1; // 1-based rank
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Chunk integrity diagnostics
// ---------------------------------------------------------------------------

/// Diagnostics about how the indexer split source files into chunks.
#[derive(Debug, Clone)]
pub struct ChunkDiagnostics {
    /// Total number of chunks produced.
    pub chunk_count: usize,
    /// Number of source files that were indexed.
    pub file_count: usize,
    /// For each symbol kind present in the index, how many chunks were found.
    pub kinds_breakdown: std::collections::HashMap<String, usize>,
    /// Average number of text lines per chunk (estimated from line_start..line_end).
    pub avg_lines_per_chunk: f64,
    /// Median overlap between adjacent chunks in the same file (0 if < 2 chunks/file).
    pub median_overlap_between_chunks: f64,
    /// Number of chunks that contain the "---" separator injected by `apply_overlap`.
    /// Chunks with separators have been given parent context from neighbours.
    pub chunks_with_parent_context: usize,
}

/// Compute diagnostics about chunking quality from a list of chunks.
///
/// Reports on:
/// - distribution of symbol kinds
/// - average and median chunk sizes (in lines)
/// - whether overlap separators were injected (indicates good context preservation)
pub fn chunk_diagnostics(chunks: &[crate::indexer::Chunk]) -> ChunkDiagnostics {
    if chunks.is_empty() {
        return ChunkDiagnostics {
            chunk_count: 0,
            file_count: 0,
            kinds_breakdown: std::collections::HashMap::new(),
            avg_lines_per_chunk: 0.0,
            median_overlap_between_chunks: 0.0,
            chunks_with_parent_context: 0,
        };
    }

    let mut kinds: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut line_lengths: Vec<usize> = Vec::with_capacity(chunks.len());
    let mut has_separator_count = 0;

    // Group chunks by file for overlap computation
    let mut files: std::collections::HashMap<PathBuf, Vec<&crate::indexer::Chunk>> =
        std::collections::HashMap::new();

    for chunk in chunks {
        kinds.entry(chunk.symbol_kind_name().to_string())
            .or_default();
        *kinds.entry(chunk.symbol_kind_name().to_string()).or_default() += 1;

        let lines_in_chunk = chunk.line_end.saturating_sub(chunk.line_start);
        line_lengths.push(lines_in_chunk);

        if chunk.text.contains("---") {
            has_separator_count += 1;
        }

        files.entry(chunk.file_path.clone())
            .or_default()
            .push(chunk);
    }

    // Sort keys for deterministic output
    let mut sorted_kinds: Vec<_> = kinds.into_iter().collect();
    sorted_kinds.sort_by_key(|(k, _)| k.clone());

    // Compute median overlap between adjacent chunks in the same file
    let overlaps: Vec<f64> = {
        let mut all_overlaps = Vec::new();
        for chunk_list in files.values() {
            if chunk_list.len() >= 2 {
                let mut sorted_chunks: Vec<_> = chunk_list.into_iter().collect();
                sorted_chunks.sort_by_key(|c| c.line_start);
                // Adjacent chunks within the same file may have gaps or overlaps.
                // Overlap here means previous.end > current.start (positive overlap).
                for w in sorted_chunks.windows(2) {
                    let prev_end = w[0].line_end;
                    let cur_start = w[1].line_start;
                    if prev_end > cur_start {
                        all_overlaps.push((prev_end - cur_start) as f64);
                    } else {
                        all_overlaps.push(0.0);
                    }
                }
            }
        }
        sort_asc(&mut all_overlaps);
        all_overlaps
    };

    let median_overlap = if overlaps.is_empty() {
        0.0
    } else {
        percentile_50(&overlaps)
    };

    let avg_lines: f64 = line_lengths.iter().map(|&l| l as f64).sum::<f64>() / line_lengths.len() as f64;

    ChunkDiagnostics {
        chunk_count: chunks.len(),
        file_count: files.len(),
        kinds_breakdown: sorted_kinds.into_iter().collect(),
        avg_lines_per_chunk: avg_lines,
        median_overlap_between_chunks: median_overlap,
        chunks_with_parent_context: has_separator_count,
    }
}

/// Sort a slice of f64 values in ascending order (in-place).
fn sort_asc(v: &mut Vec<f64>) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
}

/// Compute the p-th percentile from an already-sorted slice.
fn percentile_50(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (sorted.len() as f64 * 0.5) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector_store::SearchResult;

    /// Build a mock SearchResult with the given id and score.
    fn mock_result(id: &str, score: f32) -> SearchResult {
        SearchResult {
            id: id.to_string(),
            file_path: PathBuf::from("test.rs"),
            line_start: 1,
            line_end: 10,
            module_name: "test".into(),
            symbol_kind: Some(crate::indexer::SymbolKind::Function),
            text: format!("fn {}() {{}}", id.replace('_', "-")),
            score,
        }
    }

    #[test]
    fn test_mrr_perfect_ranking() {
        let labels = vec![Label {
            query: "find foo".to_string(),
            expected_ids: vec!["chunk_foo".into()],
        }];

        // Retrieval function that always returns the relevant chunk first
        let results = |query: &str, _top_k: usize| -> Vec<SearchResult> {
            if query == "find foo" {
                vec![
                    mock_result("chunk_foo", 0.9),
                    mock_result("chunk_bar", 0.5),
                ]
            } else {
                vec![]
            }
        };

        let report = evaluate_mrr(&labels, 10, results);
        assert_eq!(report.query_count, 1);
        // Reciprocal rank = 1/1 = 1.0 → MRR = 1.0
        assert!((report.mrr - 1.0).abs() < 1e-9, "MRR should be 1.0");
        assert_eq!(report.hits, 1);
    }

    #[test]
    fn test_mrr_relevant_chunk_not_found() {
        let labels = vec![Label {
            query: "find bar".to_string(),
            expected_ids: vec!["chunk_bar".into()],
        }];

        // Retrieval returns unrelated results only
        let results = |query: &str, _top_k: usize| -> Vec<SearchResult> {
            if query == "find bar" {
                vec![mock_result("chunk_unrelated", 0.8)]
            } else {
                vec![]
            }
        };

        let report = evaluate_mrr(&labels, 10, results);
        assert_eq!(report.query_count, 1);
        // Relevant chunk not in top_k → RR = 0
        assert!(report.mrr < 1e-9, "MRR should be ~0 when irrelevant result returned");
        assert_eq!(report.hits, 0);
    }

    #[test]
    fn test_mrr_partial_ranking() {
        let labels = vec![Label {
            query: "find bar".to_string(),
            expected_ids: vec!["chunk_bar".into()],
        }];

        // Retrieval returns 3 results; relevant chunk is at position 2 (rank=2)
        let results = |query: &str, _top_k: usize| -> Vec<SearchResult> {
            if query == "find bar" {
                vec![
                    mock_result("chunk_first", 0.9),
                    mock_result("chunk_bar", 0.6), // rank 2
                    mock_result("chunk_third", 0.4),
                ]
            } else {
                vec![]
            }
        };

        let report = evaluate_mrr(&labels, 10, results);
        // RR = 1/2 = 0.5 → MRR = 0.5
        assert!(
            (report.mrr - 0.5).abs() < 1e-9,
            "MRR should be 0.5 when relevant chunk is at rank 2"
        );
        assert_eq!(report.hits, 1);
    }

    #[test]
    fn test_mrr_multiple_labels_average() {
        let labels = vec![
            Label {
                query: "query_a".into(),
                expected_ids: vec!["chunk_a".into()],
            },
            Label {
                query: "query_b".into(),
                expected_ids: vec!["chunk_b".into()],
            },
        ];

        let results = |query: &str, _top_k: usize| -> Vec<SearchResult> {
            // Each query gets its own relevant chunk at rank 1 + irrelevant filler
            match query {
                "query_a" => vec![mock_result("chunk_a", 0.9), mock_result("other_a", 0.5)],
                "query_b" => vec![mock_result("chunk_b", 0.85), mock_result("other_b", 0.4)],
                _ => vec![],
            }
        };

        let report = evaluate_mrr(&labels, 10, results);
        assert_eq!(report.query_count, 2);
        // Both hits at rank 1 → MRR = (1+1)/2 = 1.0
        assert!((report.mrr - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_chunk_diagnostics_empty() {
        let diags = chunk_diagnostics(&[]);
        assert_eq!(diags.chunk_count, 0);
        assert_eq!(diags.file_count, 0);
    }

    #[test]
    fn test_chunk_diagnostics_with_separators() {
        use crate::indexer::Chunk;
        let chunks = vec![
            Chunk {
                file_path: PathBuf::from("a.rs"),
                line_start: 0,
                line_end: 20,
                module_name: "func_a".into(),
                symbol_kind: SymbolKind::Function,
                text: "fn a() {} \n---\ncontext_from_neighbor\n---\nfn a() {}".to_string(),
            },
            Chunk {
                file_path: PathBuf::from("a.rs"),
                line_start: 20,
                line_end: 40,
                module_name: "func_b".into(),
                symbol_kind: SymbolKind::Function,
                text: "fn b() {}".to_string(), // no separator
            },
        ];

        let diags = chunk_diagnostics(&chunks);
        assert_eq!(diags.chunk_count, 2);
        assert_eq!(diags.file_count, 1);
        assert_eq!(diags.chunks_with_parent_context, 1);
    }

    #[test]
    fn test_chunk_diagnostics_kind_breakdown() {
        use crate::indexer::Chunk;
        let chunks = vec![
            Chunk {
                file_path: PathBuf::from("a.rs"),
                line_start: 0,
                line_end: 10,
                module_name: "f".into(),
                symbol_kind: SymbolKind::Function,
                text: "fn f() {}".to_string(),
            },
            Chunk {
                file_path: PathBuf::from("b.rs"),
                line_start: 0,
                line_end: 10,
                module_name: "impl".into(),
                symbol_kind: SymbolKind::ImplBlock,
                text: "impl X {}".to_string(),
            },
        ];

        let diags = chunk_diagnostics(&chunks);
        assert_eq!(diags.chunk_count, 2);
        // Should have both Function and ImplBlock kinds
        assert!(
            diags.kinds_breakdown.contains_key("Function"),
            "Should report Function kind"
        );
        assert!(
            diags.kinds_breakdown.contains_key("ImplBlock"),
            "Should report ImplBlock kind"
        );
    }

    // Re-export SymbolKind for tests that need it.
    use crate::indexer::SymbolKind;
}
