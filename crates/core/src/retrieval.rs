use anyhow::Result;
use std::collections::HashMap;

use crate::indexer::Chunk;
use crate::vector_store::SearchResult;

/// Retrieve relevant chunks for a given query from an existing vector store.
/// Uses hybrid search (BM25 + cosine similarity) by default.
pub fn retrieve(query: &str, embedding: &[f32], vector_store: &crate::vector_store::VectorStore, top_k: usize) -> Result<Vec<SearchResult>> {
    // Hybrid search with alpha=0.7 (70% vector / 30% BM25 blend is a good starting point)
    let results = vector_store.hybrid_search(embedding, query, top_k, 0.7, None)?;
    Ok(results)
}

/// Retrieve relevant chunks from in-memory chunks (no persistent store).
pub fn retrieve_from_chunks(chunks: &[Chunk], query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
    let embedding = crate::embedding::embed(query)?;

    let mut scored: Vec<(f32, Chunk)> = Vec::new();
    for chunk in chunks {
        if chunk.text.is_empty() || chunk.text.trim().len() < 4 {
            continue;
        }
        let chunk_embedding = crate::embedding::embed(&chunk.text)?;
        scored.push((cosine_similarity_score(&embedding, &chunk_embedding), chunk.clone()));
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let results: Vec<SearchResult> = scored
        .into_iter()
        .take(top_k)
        .map(|(score, chunk)| SearchResult {
            id: format!("chunk_{}", chunk.file_path.to_string_lossy()),
            file_path: chunk.file_path.clone(),
            line_start: chunk.line_start,
            line_end: chunk.line_end,
            module_name: chunk.module_name.clone(),
            symbol_kind: match &chunk.symbol_kind {
                crate::indexer::SymbolKind::Function => Some(crate::vector_store::SymbolKind::Function),
                crate::indexer::SymbolKind::ImplBlock => Some(crate::vector_store::SymbolKind::ImplBlock),
                crate::indexer::SymbolKind::UnsafeRegion => Some(crate::vector_store::SymbolKind::UnsafeRegion),
                crate::indexer::SymbolKind::TraitImpl => Some(crate::vector_store::SymbolKind::TraitImpl),
                crate::indexer::SymbolKind::Module => Some(crate::vector_store::SymbolKind::Module),
                crate::indexer::SymbolKind::Struct => Some(crate::vector_store::SymbolKind::Struct),
                crate::indexer::SymbolKind::Enum => Some(crate::vector_store::SymbolKind::Enum),
                crate::indexer::SymbolKind::Macro => Some(crate::vector_store::SymbolKind::Macro),
            },
            text: chunk.text.clone(),
            score,
        })
        .collect();

    Ok(results)
}

/// Hybrid retrieval combining vector similarity with call graph proximity.
pub fn retrieve_hybrid(
    query: &str,
    chunks: &[Chunk],
    _graph: &petgraph::Graph<crate::callgraph::Symbol, f32>,
    _name_to_index: &HashMap<String, usize>,
    top_k: usize,
) -> Result<Vec<SearchResult>> {
    // For MVP, fall back to vector-only search.
    retrieve_from_chunks(chunks, query, top_k)
}

fn cosine_similarity_score(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}
