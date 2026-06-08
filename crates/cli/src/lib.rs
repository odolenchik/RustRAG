pub mod cmd;

use anyhow::Result;
use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use futures_util::StreamExt;
use rust_rag_llm::ChatBackend;
use rust_rag_core::{state, vector_store};

/// Run the index pipeline on a workspace directory with incremental updates.
pub fn index_workspace(path: &str) -> Result<()> {
    let workspace_root = std::path::PathBuf::from(path);
    if !workspace_root.exists() {
        anyhow::bail!("Workspace path does not exist: {}", path);
    }

    println!("Indexing workspace at: {}", path);

    // Step 1: Collect all .rs file paths and compute their hashes.
    let store_path = workspace_root.join(".rustrag");
    let current_files = collect_file_hashes(&workspace_root)?;
    let total_files = current_files.len();

    if total_files == 0 {
        println!("No Rust source files found in workspace.");
        return Ok(());
    }

    // Step 2: Load existing state (if any).
    let saved_state = state::IndexState::load(&store_path)?;

    // Step 3: Determine which files need re-indexing.
    let (new_files, changed_files, removed_chunk_ids) = saved_state.compare(&current_files);

    // If nothing has changed since last index, skip.
    if new_files.is_empty() && changed_files.is_empty() {
        println!("No changes detected. Index is up to date.");
        return Ok(());
    }

    let files_to_reindex: HashMap<_, _> = current_files
        .iter()
        .filter(|(p, _)| new_files.contains(p) || changed_files.contains(p))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    println!(
        "Detected {} new/changed files out of {} total. Re-indexing only changed files.",
        new_files.len(),
        total_files
    );

    // Step 4: Parse AST for changed/new files and build chunks.
    let mut all_chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();

    for (file_path, _hash) in &files_to_reindex {
        let content = std::fs::read_to_string(file_path)?;
        // Parse only this file's AST nodes.
        rust_rag_core::indexer::parse_and_extract(&content, file_path, &mut all_chunks)?;
    }

    // Apply overlap across all collected chunks.
    rust_rag_core::indexer::apply_overlap(&mut all_chunks);

    let total_reindexed = all_chunks.len();
    println!("Parsed {} chunks from changed files.", total_reindexed);

    if all_chunks.is_empty() {
        return Ok(());
    }

    // Step 5: Embed only the re-chunked text using cache.
    let texts: Vec<&str> = all_chunks.iter().map(|c| c.text.as_str()).collect();
    let embed_cache = rust_rag_core::embedding::EmbedCache::open(&store_path);
    let cached = embed_cache.lookup(&texts)?;
    let hit_count = cached.iter().filter_map(|x| x.clone()).count();

    let mut all_embeddings: Vec<Vec<f32>> = vec![Vec::new(); texts.len()];
    for (i, opt) in cached.into_iter().enumerate() {
        if let Some(embedding) = opt {
            all_embeddings[i] = embedding;
        }
    }

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
    }

    // Persist cache.
    let _ = embed_cache.write_back(&texts, &all_embeddings.iter().filter_map(|e| Some(e.clone())).collect::<Vec<_>>(), &mut 0);

    // Step 6: Build documents and remove stale entries from index.
    let mut new_documents = Vec::new();
    for (i, chunk) in all_chunks.iter().enumerate() {
        let doc_id = format!("chunk_{}_{}", chunk.file_path.to_string_lossy(), chunk.line_start);
        new_documents.push(vector_store::Document {
            id: doc_id.clone(),
            chunk: chunk.clone(),
            embedding: all_embeddings[i].clone(),
        });
    }

    // Remove stale documents from index for changed files and deleted files.
    let store = vector_store::VectorStore::for_workspace(&workspace_root);

    // Collect IDs to remove: old chunks for changed files + removed file chunks.
    let mut stale_ids: Vec<String> = Vec::new();

    // For changed files, the old chunk IDs need to be replaced.
    for p in &changed_files {
        let ids: Vec<String> = saved_state.chunk_ids.iter()
            .filter(|id| id.starts_with(&format!("chunk_{}_", p.display())))
            .cloned()
            .collect();
        stale_ids.extend(ids);
    }

    // For removed files, use the pre-computed chunk IDs from compare().
    stale_ids.extend(removed_chunk_ids);

    if !stale_ids.is_empty() {
        println!("Removing {} stale document(s) from index...", stale_ids.len());
        store.remove_documents(&stale_ids)?;
    } else if new_files.is_empty() && changed_files.is_empty() {
        println!("No changes detected. Index is up to date.");
        return Ok(());
    }

    // Insert new documents.
    if !new_documents.is_empty() {
        store.insert_documents(&new_documents)?;
    }

    // Insert new documents.
    if !new_documents.is_empty() {
        store.insert_documents(&new_documents)?;
    }

    // Step 7: Update index state.
    let mut updated_state = saved_state;
    updated_state.update_files(current_files);
    updated_state.save(&store_path)?;

    println!(
        "Index complete. {} files processed, {} chunks re-indexed.",
        total_files, total_reindexed
    );

    Ok(())
}

/// Walk the workspace and collect all .rs file paths with their SHA-256 hashes.
fn collect_file_hashes(root: &Path) -> Result<HashMap<std::path::PathBuf, String>> {
    let mut files = HashMap::new();
    // Use indexer's logic to find member paths, then walk each src/ directory
    let manifest = root.join("Cargo.toml");
    if !manifest.exists() {
        return Ok(files);
    }

    let cargo_content = std::fs::read_to_string(&manifest)?;
    let cargo_toml: toml::Value = cargo_content.parse()?;

    // Reuse extract_workspace_members logic from indexer module
    let member_paths = rust_rag_core::indexer::extract_workspace_members(&cargo_toml, root);

    for member_path in member_paths {
        let src_dir = member_path.join("src");
        if !src_dir.exists() {
            continue;
        }
        collect_rs_hashes(&src_dir, &mut files)?;
    }
    Ok(files)
}

fn collect_rs_hashes(dir: &Path, files: &mut HashMap<std::path::PathBuf, String>) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir).min_depth(1).max_depth(5).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() || path.extension() != Some("rs".as_ref()) {
            continue;
        }
        match state::IndexState::compute_file_hash(path) {
            Ok(hash) => { files.insert(path.to_path_buf(), hash); }
            Err(_) => {} // skip unreadable files
        }
    }
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
    print!("\nAsking LLM...\n\n");
    let response = rust_rag_llm::ollama_client::LlmClient::chat(&system_prompt, &user_message)?;
    println!("{}", response);

    Ok(())
}

/// Run the ask pipeline with streaming output.
pub async fn ask_stream(query: &str, workspace_root: Option<&str>) -> Result<()> {
    let ws = if let Some(path) = workspace_root {
        std::path::PathBuf::from(path)
    } else {
        std::env::current_dir()?
    };

    let cfg = rust_rag_core::config::Config::load(&ws).unwrap_or_default();
    let top_k: usize = cfg.llm_config().top_k;

    let store_path = ws.join(".rustrag");
    let index_path = store_path.join("index.jsonl");

    if !index_path.exists() {
        anyhow::bail!("No index found. Run `rust-rag index <path>` first.");
    }

    let embedding: Vec<f32> = rust_rag_core::embedding::embed(query)?;
    let store = rust_rag_core::vector_store::VectorStore::open(&store_path)?;
    let results = store.hybrid_search(&embedding, query, top_k, 0.7, None)?;

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

    let system_prompt = "You are a Rust code analysis assistant. Answer questions based on the provided code snippets. Always cite file paths and line numbers when referencing code.";
    let user_message = format!(
        "Question: {}\n\nRelevant code:\n{}",
        query, context
    );

    print!("\nAsking LLM...\n\n");
    let client = rust_rag_llm::ollama_client::LlmClient::default();
    let mut stream = client.complete_stream_chunks(system_prompt, &user_message);

    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(text) => print!("{}", text),
            Err(e) => {
                println!("\nError: {}", e);
                break;
            }
        }
        std::io::stdout().lock().flush()?;
    }
    println!();

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


/// Search for a symbol by name in the indexed workspace.
pub fn search_symbol(query: &str, workspace_root: Option<&str>) -> Result<()> {
    let ws = if let Some(path) = workspace_root {
        std::path::PathBuf::from(path)
    } else {
        std::env::current_dir()?
    };

    let store_path = ws.join(".rustrag");
    let index_path = store_path.join("index.jsonl");

    if !index_path.exists() {
        anyhow::bail!("No index found. Run `rust-rag index <path>` first.");
    }

    let content = std::fs::read_to_string(&index_path)?;
    let mut matches: Vec<serde_json::Value> = Vec::new();

    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let doc: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Match against module_name (which includes the symbol name) and text content
        let module_name = doc["module_name"].as_str().unwrap_or("");
        let text = doc["text"].as_str().unwrap_or("");

        if module_name.to_lowercase().contains(&query.to_lowercase())
            || text.contains(query)
        {
            matches.push(doc);
        }
    }

    // Deduplicate by document ID
    let mut seen_ids: std::collections::HashSet<String> = Default::default();
    matches.retain(|doc| {
        let id = doc["id"].as_str().unwrap_or("").to_string();
        if seen_ids.contains(&id) { false } else { seen_ids.insert(id); true }
    });

    if matches.is_empty() {
        println!("No symbol found matching '{}'.", query);
        return Ok(());
    }

    // Sort by file path for consistent output
    matches.sort_by(|a, b| a["file_path"]
        .as_str()
        .unwrap_or("")
        .cmp(&b["file_path"].as_str().unwrap_or("")));

    println!("Found {} result(s) for '{}':\n", matches.len(), query);
    for (i, doc) in matches.iter().enumerate() {
        let module_name = doc["module_name"].as_str().unwrap_or("<unknown>");
        let symbol_kind = doc.get("symbol_kind").and_then(|v| v.as_str()).unwrap_or("?");

        println!("{}: {} [{}] — {}:{}",
            i + 1,
            module_name,
            symbol_kind,
            doc["file_path"].as_str().unwrap_or("<unknown>"),
            doc["line_start"]
        );
    }

    Ok(())
}
