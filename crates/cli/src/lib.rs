pub mod cmd;

use anyhow::Result;
use rust_rag_core::{indexer, vector_store};

/// Run the index pipeline on a workspace directory.
pub fn index_workspace(path: &str) -> Result<()> {
    let workspace_root = std::path::PathBuf::from(path);
    if !workspace_root.exists() {
        anyhow::bail!("Workspace path does not exist: {}", path);
    }

    println!("Indexing workspace at: {}", path);

    // Step 1: Extract chunks from the workspace
    let chunks = indexer::index_workspace(&workspace_root)?;
    println!("Found {} code chunks to index", chunks.len());

    if chunks.is_empty() {
        println!("No Rust source files found in workspace.");
        return Ok(());
    }

    // Step 2: Create vector store
    let store = vector_store::VectorStore::for_workspace(&workspace_root);
    println!("Vector store at: {}", store.path.display());

   // Step 3: Embed all chunks — use cache where possible.
    println!("Embedding {} chunks...", chunks.len());

    let store_path = workspace_root.join(".rustrag");
    let embed_cache = rust_rag_core::embedding::EmbedCache::open(&store_path);
    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();

    // Check cache first.
    let cached = embed_cache.lookup(&texts)?;
    let hit_count = cached.iter().filter_map(|x| x.clone()).count();

    // Build full embeddings array preserving chunk order.
    let mut all_embeddings: Vec<Vec<f32>> = vec![Vec::new(); texts.len()];

    for (i, opt) in cached.into_iter().enumerate() {
        if let Some(embedding) = opt {
            all_embeddings[i] = embedding;
        }
    }

    // Find uncached indices and batch-embed them.
    let uncached_indices: Vec<usize> = (0..texts.len())
        .filter(|i| all_embeddings[*i].is_empty())
        .collect();

    if !uncached_indices.is_empty() {
        println!("  {} already cached, embedding {} new chunks...", hit_count, uncached_indices.len());
        let uncached_texts: Vec<&str> = uncached_indices.iter().map(|&i| texts[i]).collect();
        let new_embeddings = rust_rag_core::embedding::embed_batch(&uncached_texts)?;

        for (j, idx) in uncached_indices.into_iter().enumerate() {
            all_embeddings[idx] = new_embeddings[j].clone();
        }
    } else if hit_count > 0 {
        println!("  All {} chunks served from embedding cache.", texts.len());
    }

    // Persist cache.
    let _ = embed_cache.write_back(&texts, &all_embeddings.iter().filter_map(|e| Some(e.clone())).collect::<Vec<_>>(), &mut 0);

    let mut documents = Vec::new();

    // Safety: embeddings[i] corresponds to chunk i (same order preserved).
    for (i, chunk) in chunks.iter().enumerate() {
        if i % 5 == 0 { println!("  Embedded {}/{}", i + 1, chunks.len()); }

        documents.push(vector_store::Document {
            id: format!("chunk_{}_{}", chunk.file_path.to_string_lossy(), chunk.line_start),
            chunk: chunk.clone(),
            embedding: all_embeddings[i].clone(),
        });
    }

    // Step 4: Save to vector store
    store.insert_documents(&documents)?;
    println!("Index complete. {} documents stored.", documents.len());

    Ok(())
}

/// Run the ask pipeline: retrieve relevant chunks and generate LLM answer.
pub fn ask(query: &str, workspace_root: Option<&str>) -> Result<()> {
    let ws = if let Some(path) = workspace_root {
        std::path::PathBuf::from(path)
    } else {
        std::env::current_dir()?
    };

    // Load config to get top_k and LLM settings
    let cfg = rust_rag_core::config::Config::load(&ws).unwrap_or_default();
    let top_k: usize = cfg.llm_config().top_k;

    // Find the vector store
    let store_path = ws.join(".rustrag");
    let index_path = store_path.join("index.jsonl");

    if !index_path.exists() {
        anyhow::bail!("No index found. Run `rust-rag index <path>` first.");
    }

    // Embed the query and run hybrid search (BM25 + vector) using VectorStore API
    let embedding: Vec<f32> = rust_rag_core::embedding::embed(query)?;
    let store = rust_rag_core::vector_store::VectorStore::open(&store_path)?;
    let results = store.hybrid_search(&embedding, query, top_k, 0.7, None)?;

    // Build context from hybrid search results
    let mut context_parts: Vec<String> = Vec::new();
    for (i, r) in results.iter().enumerate() {
        context_parts.push(format!(
            "[[{}:{}]]\n{}",
            r.file_path.display(), r.line_start, r.text
        ));

        println!("Result {}: hybrid_score={:.3} | {}:{}", i + 1, r.score as f64, r.file_path.display(), r.line_start);
    }

    let context = if context_parts.is_empty() {
        "No relevant code chunks found.".to_string()
    } else {
        context_parts.join("\n\n")
    };

    // Build the prompt for LLM
    let system_prompt = "You are a Rust code analysis assistant. Answer questions based on the provided code snippets. Always cite file paths and line numbers when referencing code.";
    let user_message = format!(
        "Question: {}\n\nRelevant code:\n{}",
        query, context
    );

    // Call LLM — uses config endpoint/model from .rustrag.toml (or env vars / defaults)
    println!("\nAsking LLM...\n");
    let response = rust_rag_llm::ollama_client::LlmClient::chat(&system_prompt, &user_message)?;
    println!("{}", response);

    Ok(())
}

/// Re-index a workspace: delete old index, then run full indexing pipeline.
pub fn reindex_workspace(path: &str) -> Result<()> {
    let workspace_root = std::path::PathBuf::from(path);

    // Remove any existing index first
    let store_path = workspace_root.join(".rustrag");
    if store_path.exists() {
        println!("Removing old index at {}", store_path.display());
        std::fs::remove_dir_all(&store_path)?;
    }

    println!("Re-indexing workspace at: {}", path);
    index_workspace(path)
}

/// Show metadata about the indexed workspace.
pub fn show_info(workspace_path: Option<&str>) -> Result<()> {
    let ws = if let Some(p) = workspace_path {
        std::path::PathBuf::from(p)
    } else {
        std::env::current_dir()?
    };

    let store_path = ws.join(".rustrag");
    if !store_path.exists() {
        println!("No index found at {}. Run `rust-rag index <path>` first.", store_path.display());
        return Ok(());
    }

    let index_path = store_path.join("index.jsonl");
    let content = std::fs::read_to_string(&index_path)?;
    let total_chunks: usize = content.lines().filter(|l| !l.trim().is_empty()).count();

    // Count unique files
    let mut unique_files: Vec<std::path::PathBuf> = Vec::new();
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(fp) = value["file_path"].as_str() {
                unique_files.push(std::path::PathBuf::from(fp));
            }
        }
    }
    unique_files.sort();
    unique_files.dedup();

    println!("Index: {}", store_path.display());
    println!("Total indexed chunks: {}", total_chunks);
    println!("Unique files: {}", unique_files.len());
    for file in &unique_files {
        println!("  - {}", file.display());
    }

    Ok(())
}

/// Remove the .rustrag directory from a workspace.
pub fn clean_workspace(workspace_path: Option<&str>) -> Result<()> {
    let ws = if let Some(p) = workspace_path {
        std::path::PathBuf::from(p)
    } else {
        std::env::current_dir()?
    };

    let store_path = ws.join(".rustrag");
    if !store_path.exists() {
        println!("Nothing to clean at {}", store_path.display());
        return Ok(());
    }

    // Delete all files inside first, then the directory itself
    for entry in std::fs::read_dir(&store_path)? {
        let entry = entry?;
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            std::fs::remove_dir_all(entry.path())?;
        } else {
            std::fs::remove_file(entry.path())?;
        }
    }
    std::fs::remove_dir(&store_path)?;

    println!("Removed {}", store_path.display());
    Ok(())
}

