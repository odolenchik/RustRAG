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
    let query_embedding = crate::embedding::embed(query)?;

    // Collect texts that pass the size filter, preserving original order for batch embedding
    let mut valid_indices: Vec<usize> = Vec::new();
    let mut valid_texts: Vec<String> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if !chunk.text.is_empty() && chunk.text.trim().len() >= 4 {
            valid_indices.push(i);
            valid_texts.push(chunk.text.clone());
        }
    }

    // Batch-embed all valid chunks in a single ONNX inference call
    let batch_embeddings = crate::embedding::embed_batch(&valid_texts.iter().map(|t| t.as_str()).collect::<Vec<_>>())?;

    // Map results back to (score, chunk) pairs
    let mut scored: Vec<(f32, usize)> = Vec::new();
    for (batch_i, chunk_idx) in valid_indices.into_iter().enumerate() {
        let score = cosine_similarity_score(&query_embedding, &batch_embeddings[batch_i]);
        scored.push((score, chunk_idx));
    }

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let results: Vec<SearchResult> = scored
        .into_iter()
        .take(top_k)
        .map(|(score, chunk_idx)| {
            let chunk = &chunks[chunk_idx];
            SearchResult {
                id: format!("chunk_{}", chunk.file_path.to_string_lossy()),
                file_path: chunk.file_path.clone(),
                line_start: chunk.line_start,
                line_end: chunk.line_end,
                module_name: chunk.module_name.clone(),
                symbol_kind: (&chunk.symbol_kind).into(),
                text: chunk.text.clone(),
                score,
            }
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

use crate::vector_store::cosine_similarity as cosine_similarity_score;
