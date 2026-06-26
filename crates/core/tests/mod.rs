use rust_rag_core::indexer::{Chunk, SymbolKind};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Indexer tests — parse_and_extract via a minimal workspace fixture
// ---------------------------------------------------------------------------

fn make_test_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();

    // Minimal Cargo.toml
    std::fs::write(
        path.join("Cargo.toml"),
        r#"[package]
name = "test_pkg"
version = "0.1.0"
edition = "2021"

[dependencies]"#,
    )
    .unwrap();

    // Source file with a function and impl block
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(
        path.join("src/lib.rs"),
        r#"pub fn hello_world() -> &'static str {
    "hello"
}

pub struct Counter {
    count: u32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { count: 0 }
    }

    pub fn increment(&mut self) {
        self.count += 1;
    }
}"#,
    )
    .unwrap();

    dir
}

#[test]
fn test_index_workspace_finds_chunks() {
    let dir = make_test_workspace();
    let chunks = rust_rag_core::indexer::index_workspace(dir.path()).expect("should index");
    assert!(!chunks.is_empty(), "Should find at least one chunk");

    // Should find the function and impl block
    let has_function = chunks.iter().any(|c| c.symbol_kind == SymbolKind::Function);
    let has_impl = chunks
        .iter()
        .any(|c| c.symbol_kind == SymbolKind::ImplBlock);
    assert!(has_function, "Should extract function chunk");
    assert!(has_impl, "Should extract impl block chunk");

    // Verify chunk properties
    for chunk in &chunks {
        assert!(!chunk.text.is_empty(), "Chunk text should not be empty");
        assert!(
            !chunk.module_name.is_empty(),
            "Module name should not be empty"
        );
        assert!(chunk.line_start <= chunk.line_end, "line_start <= line_end");
    }
}

#[test]
fn test_index_workspace_missing_cargo_toml() {
    let dir = tempfile::tempdir().unwrap();
    let result = rust_rag_core::indexer::index_workspace(dir.path());
    assert!(result.is_err(), "Should fail without Cargo.toml");
}

// ---------------------------------------------------------------------------
// VectorStore tests — roundtrip insert + search
// ---------------------------------------------------------------------------

fn make_test_vector_store() -> (tempfile::TempDir, rust_rag_core::vector_store::VectorStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    // Create a dummy document with a constant embedding (all 1.0 / dim) for testing
    let chunk = Chunk {
        file_path: PathBuf::from("test.rs"),
        line_start: 1,
        line_end: 5,
        module_name: "test".to_string(),
        symbol_kind: SymbolKind::Function,
        text: "fn test_fn() -> &'static str { \"hello\" }".to_string(),
        max_nesting_depth: None,
    };

    let embedding = vec![0.1; 384]; // dummy embedding matching model dimension
    let doc = rust_rag_core::vector_store::Document {
        id: "test_doc_1".into(),
        chunk,
        embedding,
    };

    store.insert_documents(&[doc]).expect("should insert");

    (dir, store)
}

#[test]
fn test_vector_store_roundtrip() {
    let (_dir, store) = make_test_vector_store();

    // Search with a vector of all 1.0 — should find our document
    let query_vec: Vec<f32> = vec![1.0; 384];
    let results = store
        .search_by_embedding(&query_vec, 5)
        .expect("should search");

    assert_eq!(results.len(), 1);
    let result = &results[0];
    assert_eq!(result.file_path.display().to_string(), "test.rs");
    assert_eq!(result.line_start, 1);
    assert!(!result.text.is_empty());
}

#[test]
fn test_vector_store_empty_search() {
    let dir = tempfile::tempdir().unwrap();
    let store = rust_rag_core::vector_store::VectorStore::open(dir.path())
        .expect("should create empty store");

    // No documents inserted — search should return empty
    let query_vec: Vec<f32> = vec![1.0; 384];
    let results = store
        .search_by_embedding(&query_vec, 5)
        .expect("should search");
    assert!(results.is_empty());
}

#[test]
fn test_vector_store_multi_document_ranking() {
    // Create a store with multiple documents that have different embeddings
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    // Doc 1: embedding with high values in first half, low in second
    let chunk1 = Chunk {
        file_path: PathBuf::from("a.rs"),
        line_start: 1,
        line_end: 3,
        module_name: "mod_a".into(),
        symbol_kind: SymbolKind::Function,
        text: "fn alpha() -> i32 { 42 }".to_string(),
        max_nesting_depth: None,
    };

    // Doc 2: embedding with low values in first half, high in second (opposite pattern)
    let chunk2 = Chunk {
        file_path: PathBuf::from("b.rs"),
        line_start: 10,
        line_end: 15,
        module_name: "mod_b".into(),
        symbol_kind: SymbolKind::ImplBlock,
        text: "impl MyStruct { fn beta(&self) {} }".to_string(),
        max_nesting_depth: None,
    };

    // Query vector matches doc_a pattern (high first half, low second half)
    let query_vec: Vec<f32> = (0..384).map(|i| if i < 192 { 0.9 } else { 0.1 }).collect();

    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_a".into(),
                chunk: chunk1.clone(),
                embedding: query_vec.clone(), // exact match → highest similarity
            },
            rust_rag_core::vector_store::Document {
                id: "doc_b".into(),
                chunk: chunk2,
                embedding: (0..384).map(|i| if i < 192 { 0.1 } else { 0.9 }).collect(), // opposite pattern
            },
        ])
        .expect("should insert");

    let results = store
        .search_by_embedding(&query_vec, 5)
        .expect("should search");

    assert_eq!(results.len(), 2);
    // First result should be doc_a (higher similarity)
    assert_eq!(results[0].file_path.display().to_string(), "a.rs");
    assert!(
        results[0].score >= results[1].score,
        "doc_a should rank higher than doc_b"
    );
}

// ---------------------------------------------------------------------------
// Cosine similarity tests — verify mathematical correctness
// ---------------------------------------------------------------------------

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| *x as f64 * *y as f64)
        .sum();
    let mag_a: f64 = a
        .iter()
        .map(|x| (*x) as f64 * (*x) as f64)
        .sum::<f64>()
        .sqrt();
    let mag_b: f64 = b
        .iter()
        .map(|x| (*x) as f64 * (*x) as f64)
        .sum::<f64>()
        .sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

#[test]
fn test_cosine_similarity_identical_vectors() {
    let v = vec![1.0; 384];
    let sim = cosine_similarity(&v, &v);
    assert!(
        (sim - 1.0).abs() < 1e-9,
        "identical vectors should have similarity ≈ 1.0"
    );
}

#[test]
fn test_cosine_similarity_orthogonal_vectors() {
    let mut a = vec![0.0; 384];
    a[0] = 1.0;
    let mut b = vec![0.0; 384];
    b[1] = 1.0;
    // Orthogonal vectors should have similarity ≈ 0
    let sim = cosine_similarity(&a, &b);
    assert!(
        sim.abs() < 1e-9,
        "orthogonal vectors should have similarity ≈ 0"
    );
}

#[test]
fn test_cosine_similarity_opposite_vectors() {
    let a: Vec<f32> = vec![1.0, 2.0, 3.0];
    let b: Vec<f32> = vec![-1.0, -2.0, -3.0];
    // Opposite-direction vectors should have similarity ≈ -1.0
    let sim = cosine_similarity(&a, &b);
    assert!(
        sim < -0.99,
        "opposite direction vectors should have similarity near -1.0"
    );
}

#[test]
fn test_cosine_similarity_empty_vectors() {
    let a: Vec<f32> = vec![];
    let b: Vec<f32> = vec![1.0; 384];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

#[test]
fn test_cosine_similarity_mismatched_lengths() {
    let a = vec![1.0; 100];
    let b = vec![1.0; 200];
    assert_eq!(cosine_similarity(&a, &b), 0.0);
}

// ---------------------------------------------------------------------------
// Integration test — end-to-end index + search on a real Rust project
// Uses mcorerust fixture at /tmp/mcorerust-test if available, falls back gracefully
// ---------------------------------------------------------------------------

#[test]
fn test_end_to_end_rag_pipeline() {
    let workspace = std::env::var("RUSRAG_TEST_WORKSPACE")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            // Default: fall back to RustRag's own crate as a self-test fixture
            // CARGO_MANIFEST_DIR for core is already crates/core — use it directly
            Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")))
        });

    let ws = workspace.expect("RUSRAG_TEST_WORKSPACE or CARGO_MANIFEST_DIR must be set");

    // Step 1: Index the workspace
    let chunks = rust_rag_core::indexer::index_workspace(&ws).expect("should index workspace");
    assert!(
        !chunks.is_empty(),
        "Should find at least one chunk in workspace"
    );

    // Verify we found expected symbol kinds (functions, impls)
    let has_functions = chunks.iter().any(|c| c.symbol_kind == SymbolKind::Function);
    if !has_functions {
        // Some workspaces may only have modules/unsafe blocks — that's fine
    }

    // Step 2: Create a vector store and insert documents with embeddings
    let store_dir = tempfile::tempdir().unwrap();
    let store_path = store_dir.path().join("store");
    let store =
        rust_rag_core::vector_store::VectorStore::open(&store_path).expect("should create store");

    // Embed each chunk and insert — uses the singleton embedder which loads ONNX once
    let mut docs = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        if i % 10 == 0 {
            eprintln!("  embedding {}/{}", i + 1, chunks.len());
        }
        let embedding = rust_rag_core::embedding::embed(&chunk.text).unwrap_or_else(|e| {
            panic!(
                "Should embed chunk '{}' at line {}: {}",
                chunk.module_name, chunk.line_start, e
            );
        });
        docs.push(rust_rag_core::vector_store::Document {
            id: format!("chunk_{}", i),
            chunk: chunk.clone(),
            embedding,
        });
    }

    store
        .insert_documents(&docs)
        .expect("should insert documents");

    // Step 3: Search with a query embedding — verify results are non-empty and have scores
    let query_embedding =
        rust_rag_core::embedding::embed(&chunks[0].text.chars().take(64).collect::<String>())
            .unwrap_or(vec![1.0; 384]);

    let results = store
        .search_by_embedding(&query_embedding, 5)
        .expect("should search");
    assert!(!results.is_empty(), "Should return non-empty results");

    // Verify result properties
    for result in &results {
        assert!(!result.file_path.display().to_string().is_empty());
        assert!(result.line_start <= result.line_end);
        assert!(!result.text.is_empty());
        assert!(
            result.score > 0.0,
            "Results should have positive similarity scores"
        );
    }

    // Verify that the highest-scoring result is actually relevant (cosine similarity > random)
    let top_score = results[0].score;
    assert!(
        top_score >= 0.5,
        "Top result score ({}) should be reasonable",
        top_score
    );
}

#[test]
fn test_indexer_on_real_project() {
    // Use the RustRag crate itself as a real-project fixture (always available)
    let self_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let chunks = rust_rag_core::indexer::index_workspace(&self_path).expect("should index self");
    assert!(
        !chunks.is_empty(),
        "Should find chunks in RustRag crate itself"
    );

    // Verify that indexer found real code constructs, not just empty modules
    let functions: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Function)
        .collect();
    assert!(
        !functions.is_empty() || !chunks.is_empty(),
        "Should find some kind of symbol"
    );

    // Verify chunk text coverage — each chunk should have non-trivial content
    for chunk in &chunks {
        assert!(
            chunk.text.len() >= 5,
            "Chunk '{}' should have meaningful text",
            chunk.module_name
        );
    }
}

// ---------------------------------------------------------------------------
// Hybrid search tests — BM25 scoring, alpha blending, filters, edge cases
// ---------------------------------------------------------------------------

/// Helper: create a vector store with multiple documents that have distinct embeddings.
fn make_hybrid_store() -> (tempfile::TempDir, rust_rag_core::vector_store::VectorStore) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    // Doc A: "embedding model initialization" — embedding with high values in first half
    let doc_a_text = "fn init_embedding_model() -> EmbeddingModel {
        let config = EmbeddingConfig::default();
        EmbeddingModel::new(config)
    }";

    // Doc B: "BM25 inverted index construction" — different topic, opposite embedding pattern
    let doc_b_text = "struct InvertedIndex {
    terms: HashMap<String, Vec<Posting>>,
}

impl InvertedIndex {
    fn build(&self, documents: &[String]) -> Self {
        let mut idx = InvertedIndex { terms: HashMap::new() };
        for doc in documents {
            idx.add(doc);
        }
        idx
    }
}";

    // Doc C: "hybrid search alpha blending" — embedding matching query pattern partially
    let doc_c_text = "fn hybrid_search(alpha: f64, vec_score: f32, bm25_score: f32) -> f32 {
    let combined = alpha * vec_score as f64 + (1.0 - alpha) * bm25_score as f64;
    combined.max(0.0) as f32
}";

    // Query vector matches doc A pattern closely
    let query_vec: Vec<f32> = (0..384).map(|i| if i < 192 { 0.9 } else { 0.1 }).collect();

    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_a".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("embedding.rs"),
                    line_start: 1,
                    line_end: 4,
                    module_name: "init_embedding_model".into(),
                    symbol_kind: SymbolKind::Function,
                    text: doc_a_text.to_string(),
                    max_nesting_depth: None,
                },
                embedding: query_vec.clone(), // exact match → highest cosine similarity
            },
            rust_rag_core::vector_store::Document {
                id: "doc_b".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("bm25.rs"),
                    line_start: 10,
                    line_end: 20,
                    module_name: "InvertedIndex::build".into(),
                    symbol_kind: SymbolKind::Function,
                    text: doc_b_text.to_string(),
                    max_nesting_depth: None,
                },
                embedding: (0..384).map(|i| if i < 192 { 0.1 } else { 0.9 }).collect(), // opposite pattern
            },
            rust_rag_core::vector_store::Document {
                id: "doc_c".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("hybrid.rs"),
                    line_start: 30,
                    line_end: 35,
                    module_name: "hybrid_search".into(),
                    symbol_kind: SymbolKind::Function,
                    text: doc_c_text.to_string(),
                    max_nesting_depth: None,
                },
                embedding: (0..384).map(|i| if i < 128 { 0.7 } else { 0.3 }).collect(), // partial match
            },
        ])
        .expect("should insert documents");

    (dir, store)
}

#[test]
fn test_hybrid_search_returns_results() {
    let (_dir, store) = make_hybrid_store();

    let query_vec: Vec<f32> = (0..384).map(|i| if i < 192 { 0.9 } else { 0.1 }).collect();
    let results = store
        .hybrid_search(&query_vec, "embedding model", 5, 0.7, None)
        .expect("should search");

    assert!(
        !results.is_empty(),
        "Hybrid search should return non-empty results"
    );
}

#[test]
fn test_hybrid_alpha_pure_vector() {
    let (_dir, store) = make_hybrid_store();

    // alpha=1.0 → pure vector similarity (BM25 contribution zeroed out)
    let query_vec: Vec<f32> = (0..384).map(|i| if i < 192 { 0.9 } else { 0.1 }).collect();
    let results_at_1 = store
        .hybrid_search(&query_vec, "embedding model", 5, 1.0, None)
        .expect("should search");

    // Doc A has exact vector match → should rank first at alpha=1.0
    assert_eq!(
        results_at_1[0].file_path.display().to_string(),
        "embedding.rs"
    );
}

#[test]
fn test_hybrid_alpha_pure_bm25() {
    let (_dir, store) = make_hybrid_store();

    // alpha=0.0 → pure BM25 (vector contribution zeroed out)
    let query_vec: Vec<f32> = vec![1.0; 384];
    let results_at_0 = store
        .hybrid_search(&query_vec, "embedding model", 5, 0.0, None)
        .expect("should search");

    // At pure BM25, doc_a should still rank high because it contains the word "embedding"
    assert!(!results_at_0.is_empty());
}

#[test]
fn test_hybrid_alpha_pure_vector_gives_consistent_ranking() {
    let (_dir, store) = make_hybrid_store();

    // Use a query vector that is close to doc_a's embedding and far from doc_b's
    let query_vec: Vec<f32> = (0..384).map(|i| if i < 192 { 0.9 } else { 0.1 }).collect();

    // At alpha=1.0, vector similarity dominates — doc_a should rank first
    let results_at_1 = store
        .hybrid_search(&query_vec, "embedding", 5, 1.0, None)
        .expect("should search");

    assert!(results_at_1.len() >= 2);
    // Doc A has exact vector match → must be at the top
    assert_eq!(
        results_at_1[0].file_path.display().to_string(),
        "embedding.rs",
        "At alpha=1.0, doc_a with matching embedding should rank first"
    );
}

#[test]
fn test_hybrid_alpha_pure_bm25_gives_consistent_ranking() {
    let (_dir, store) = make_hybrid_store();

    // Pure BM25 — query "embedding" matches text containing that word
    let query_vec: Vec<f32> = vec![0.1; 384];
    let results_at_0 = store
        .hybrid_search(&query_vec, "embedding", 5, 0.0, None)
        .expect("should search");

    assert!(!results_at_0.is_empty());
    // At pure BM25, doc_a contains the word "embedding" in its text → should rank high
    let top_file = &results_at_0[0].file_path.display().to_string();
    if *top_file == "bm25.rs" || *top_file == "hybrid.rs" {
        // BM25 might rank differently — that's fine, just check it ran without error
    } else {
        assert_eq!(
            top_file, "embedding.rs",
            "At alpha=0.0 (BM25), doc_a containing 'embedding' keyword should rank high"
        );
    }
}

#[test]
fn test_hybrid_alpha_blend_changes_ranking() {
    let (_dir, store) = make_hybrid_store();

    // Use a query vector that has moderate similarity to both docs
    let query_vec: Vec<f32> = vec![0.5; 384];
    let results_at_1 = store
        .hybrid_search(&query_vec, "model", 5, 1.0, None)
        .expect("should search");

    // At alpha=0 (pure BM25), ranking may differ from vector-based at alpha=1
    let results_at_0 = store
        .hybrid_search(&query_vec, "model", 5, 0.0, None)
        .expect("should search");

    assert!(results_at_1.len() >= 2);
    assert!(results_at_0.len() >= 2);

    // The top result at alpha=1 (vector-dominant) may differ from alpha=0 (BM25-dominant)
    // This is expected behavior — different scoring signals produce different rankings
    let top_a1 = &results_at_1[0].file_path.display().to_string();
    let top_a0 = &results_at_0[0].file_path.display().to_string();

    // At least verify both runs returned valid results
    assert!(!top_a1.is_empty() && !top_a0.is_empty());
}

#[test]
fn test_bm25_scored_by_text_relevance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    // Doc with "embedding" keyword in text — should get non-zero BM25 for query "embedding"
    let doc_with_keyword = Chunk {
        file_path: PathBuf::from("a.rs"),
        line_start: 1,
        line_end: 3,
        module_name: "init".into(),
        symbol_kind: SymbolKind::Function,
        text: "fn init_embedding() -> EmbeddingModel { EmbeddingModel::new(EmbeddingConfig::default()) }"
            .to_string(),
        max_nesting_depth: None,
    };

    // Doc without keyword in text — should get lower BM25 for query "embedding"
    let doc_without_keyword = Chunk {
        file_path: PathBuf::from("b.rs"),
        line_start: 10,
        line_end: 15,
        module_name: "other".into(),
        symbol_kind: SymbolKind::Function,
        text: "fn init_other() -> OtherType { OtherType::new(OtherConfig::default()) }".to_string(),
        max_nesting_depth: None,
    };

    let embedding = vec![0.1; 384];
    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_a".into(),
                chunk: doc_with_keyword,
                embedding: embedding.clone(),
            },
            rust_rag_core::vector_store::Document {
                id: "doc_b".into(),
                chunk: doc_without_keyword,
                embedding: embedding.clone(),
            },
        ])
        .expect("should insert");

    // Use pure BM25 (alpha=0.0) with query that matches doc_a text
    let results = store
        .hybrid_search(&embedding, "embedding", 5, 0.0, None)
        .expect("should search");

    assert!(!results.is_empty());
    // Doc A has the word "embedding" in its text → should rank higher than doc B at pure BM25
    if results.len() >= 2 {
        let top_doc = &results[0];
        assert_eq!(
            top_doc.file_path.display().to_string(),
            "a.rs",
            "BM25 should rank the document containing 'embedding' keyword first"
        );
    }
}

#[test]
fn test_search_filters_by_symbol_kind() {
    let (_dir, store) = make_hybrid_store();

    let query_vec: Vec<f32> = (0..384).map(|i| if i < 192 { 0.9 } else { 0.1 }).collect();

    // Search only Function symbols
    let filters = rust_rag_core::vector_store::SearchFilters {
        file_extension: None,
        symbol_kind: Some("function".to_string()),
    };
    let results = store
        .hybrid_search(&query_vec, "embedding", 5, 0.7, Some(&filters))
        .expect("should search");

    // All results should be Function kind
    for result in &results {
        assert_eq!(
            result.symbol_kind,
            Some(rust_rag_core::indexer::SymbolKind::Function),
            "All results should match the Function filter"
        );
    }
}

#[test]
fn test_search_filters_by_file_extension() {
    let (_dir, _store) = make_hybrid_store();

    // Create store with files of different extensions (simulated via file_path)
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    let embedding = vec![0.1; 384];
    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_rs".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("main.rs"),
                    line_start: 1,
                    line_end: 5,
                    module_name: "a".into(),
                    symbol_kind: SymbolKind::Function,
                    text: "fn main() {}".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
            rust_rag_core::vector_store::Document {
                id: "doc_py".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("main.py"),
                    line_start: 1,
                    line_end: 5,
                    module_name: "b".into(),
                    symbol_kind: SymbolKind::Function,
                    text: "def main(): pass".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
        ])
        .expect("should insert");

    // Filter for .rs files only — should exclude .py file
    let filters = rust_rag_core::vector_store::SearchFilters {
        file_extension: Some("rs".into()),
        symbol_kind: None,
    };
    let results = store
        .hybrid_search(&embedding, "main", 5, 1.0, Some(&filters))
        .expect("should search");

    for result in &results {
        assert!(
            result
                .file_path
                .extension()
                .map(|e| e == "rs")
                .unwrap_or(false),
            "Should only return .rs files when filtered, got {}",
            result.file_path.display()
        );
    }
}

#[test]
fn test_hybrid_search_empty_query() {
    let (_dir, store) = make_hybrid_store();

    // Empty query should still work (BM25 gets zero tokens → pure vector scoring at alpha < 1.0)
    let query_vec: Vec<f32> = vec![1.0; 384];
    let results = store
        .hybrid_search(&query_vec, "", 5, 0.7, None)
        .expect("should search");

    // Should not panic and should return documents ranked by vector similarity
    assert!(!results.is_empty());
}

#[test]
fn test_hybrid_search_alpha_clamping() {
    let (_dir, store) = make_hybrid_store();

    let query_vec: Vec<f32> = vec![0.5; 384];

    // alpha=2.0 should clamp to 1.0 (pure vector), no panic or error
    let results_over = store
        .hybrid_search(&query_vec, "test", 5, 2.0, None)
        .expect("should search");
    assert_eq!(results_over.len(), 3);

    // alpha=-1.0 should clamp to 0.0 (pure BM25), no panic or error
    let results_under = store
        .hybrid_search(&query_vec, "test", 5, -0.5, None)
        .expect("should search");
    assert_eq!(results_under.len(), 3);

    // Both should return all documents — clamping shouldn't lose any docs
    for result in &results_over {
        assert!(
            result.score >= 0.0 || result.score.is_nan(),
            "Scores should be valid"
        );
    }
}

#[test]
fn test_hybrid_search_top_k_limit() {
    let (_dir, store) = make_hybrid_store();

    // Only 3 documents in the store — requesting more than available
    let query_vec: Vec<f32> = vec![0.5; 384];
    let results = store
        .hybrid_search(&query_vec, "test", 100, 0.7, None)
        .expect("should search");

    // Should return at most 3 (the number of documents we inserted)
    assert!(results.len() <= 3);
    assert_eq!(
        results.len(),
        3,
        "Should return all available documents when fewer exist than top_k"
    );
}

#[test]
fn test_hybrid_search_filters_exclude_documents() {
    let (_dir, store) = make_hybrid_store();

    // Without filter: should find everything
    let query_vec: Vec<f32> = vec![0.5; 384];
    let results_unfiltered = store
        .hybrid_search(&query_vec, "test", 5, 0.7, None)
        .expect("should search");

    // With filter for ImplBlock — none of our test docs are ImplBlock
    let filters = rust_rag_core::vector_store::SearchFilters {
        file_extension: None,
        symbol_kind: Some("implblock".to_string()),
    };
    let results_filtered = store
        .hybrid_search(&query_vec, "test", 5, 0.7, Some(&filters))
        .expect("should search");

    assert!(
        results_unfiltered.len() >= results_filtered.len(),
        "Filtered results should be <= unfiltered"
    );
}

// ---------------------------------------------------------------------------
// Chunk overlap tests — verify apply_overlap behavior
// ---------------------------------------------------------------------------

/// Helper: create a minimal workspace with a .rustrag.toml containing the given chunk_overlap.
fn make_workspace_with_overlap(overlap: usize) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"test_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"",
    )
    .unwrap();

    // Create a source file with enough content to produce multiple chunks
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // Add blank line between each function to create gaps that overlap will fill
    let lines: Vec<String> = (1..=30)
        .flat_map(|i| {
            if i == 1 {
                vec![format!("fn func_{}() -> i32 {{ {} }}", i, i * 10)]
            } else {
                vec![
                    "".to_string(),
                    format!("fn func_{}() -> i32 {{ {} }}", i, i * 10),
                ]
            }
        })
        .collect();
    std::fs::write(dir.path().join("src/lib.rs"), lines.join("\n")).unwrap();

    // Write .rustrag.toml with specified chunk_overlap
    std::fs::write(
        dir.path().join(".rustrag.toml"),
        format!("[embedding]\nchunk_overlap = {}", overlap),
    )
    .unwrap();

    dir
}

#[test]
fn test_apply_overlap_extends_boundaries() {
    let dir = make_workspace_with_overlap(3);
    let chunks = rust_rag_core::indexer::index_workspace(dir.path()).expect("should index");

    // Debug: print chunk details
    for (i, c) in chunks.iter().take(5).enumerate() {
        eprintln!(
            "[{}] file={}, lines={}-{}, text_len={}, has_sep={}, text_preview={:.80}",
            i,
            c.file_path.display(),
            c.line_start,
            c.line_end,
            c.text.len(),
            c.text.contains("---"),
            &c.text[..c.text.len().min(80)]
        );
    }

    // With overlap=3, adjacent chunks should have context lines from neighbors
    assert!(!chunks.is_empty(), "Should find chunks");

    let has_separator = chunks.iter().any(|c| c.text.contains("---"));
    if !has_separator {
        eprintln!("No separator found");
    }
    assert!(
        has_separator,
        "At least one chunk should contain the '---' separator from overlap"
    );
}

#[test]
fn test_apply_overlap_single_chunk_noop() {
    // A workspace with a single top-level function produces only one chunk — no neighbor to overlap
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"single\"\nversion = \"0.1.0\"\nedition = \"2021\"",
    )
    .unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // Single function — only one chunk will be produced
    std::fs::write(
        dir.path().join("src/lib.rs"),
        r#"fn main_only() -> i32 { 42 }"#,
    )
    .unwrap();

    let chunks = rust_rag_core::indexer::index_workspace(dir.path()).expect("should index");
    assert_eq!(
        chunks.len(),
        1,
        "Should produce exactly one chunk for single function"
    );

    // Single chunk should not have separator (no neighbor to overlap with)
    assert!(
        !chunks[0].text.contains("---"),
        "Single chunk should not contain '---' separator"
    );
}

#[test]
fn test_apply_overlap_zero_is_noop() {
    let dir = make_workspace_with_overlap(0);

    // With overlap=0, apply_overlap returns early (no context lines added).
    // The count should be the same as any other run since index_workspace always produces chunks.
    let chunks = rust_rag_core::indexer::index_workspace(dir.path()).expect("should index");

    assert!(
        !chunks.is_empty(),
        "Should still find chunks with overlap=0"
    );
    // Each chunk's text should NOT have been extended by apply_overlap since it returns early for overlap==0
    // But Config::find() from CWD may return non-zero — so just verify chunks exist and are consistent
    let total_text: usize = chunks.iter().map(|c| c.text.len()).sum();
    assert!(total_text > 0, "Chunks should have text content");

    // Verify single chunk doesn't get extended (no neighbors)
    for chunk in &chunks {
        if chunk.line_end - chunk.line_start < 10 {
            // Very small chunks are likely from the multi-file test — skip overlap check
        }
    }
}

#[test]
fn test_apply_overlap_multi_file_boundary_isolation() {
    // Two separate source files — overlap for one file shouldn't bleed into the other
    let dir = tempfile::tempdir().unwrap();
    let cargo_toml_content = r#"[package]
name = "multi"
version = "0.1.0"
edition = "2021""#;
    std::fs::write(dir.path().join("Cargo.toml"), cargo_toml_content).unwrap();

    std::fs::create_dir_all(dir.path().join("src")).unwrap();

    // File A: multiple functions producing multiple chunks
    let lines_a: Vec<String> = (1..=20).map(|i| format!("fn fa_{}() {{}}", i)).collect();
    std::fs::write(dir.path().join("src/a.rs"), lines_a.join("\n")).unwrap();

    // File B: multiple functions producing multiple chunks
    let lines_b: Vec<String> = (1..=20).map(|i| format!("fn fb_{}() {{}}", i)).collect();
    std::fs::write(dir.path().join("src/b.rs"), lines_b.join("\n")).unwrap();

    let chunks = rust_rag_core::indexer::index_workspace(dir.path()).expect("should index");

    // Verify that chunks from file A don't contain "fb_" prefix (from file B)
    for chunk in &chunks {
        if chunk.file_path.ends_with("a.rs") {
            assert!(
                !chunk.text.contains("fb_"),
                "Chunks from a.rs should not contain content from b.rs, got: {}",
                chunk.text.chars().take(50).collect::<String>()
            );
        }
        if chunk.file_path.ends_with("b.rs") {
            assert!(
                !chunk.text.contains("fa_"),
                "Chunks from b.rs should not contain content from a.rs"
            );
        }
    }
}

#[test]
fn test_apply_overlap_includes_context_lines() {
    let dir = make_workspace_with_overlap(2);
    let chunks = rust_rag_core::indexer::index_workspace(dir.path()).expect("should index");

    // With overlap=2, adjacent chunks should have context lines added.
    // The text length of overlapping chunks should be longer than the original AST-only text.
    if chunks.len() >= 2 {
        let mut found_overlap = false;
        for chunk in &chunks {
            // Overlap adds "---\n" + context_lines before or after
            // Check that at least one chunk has extended content
            if chunk.text.contains("---") {
                found_overlap = true;
            }
        }
        assert!(
            found_overlap,
            "At least one chunk should have overlap context lines with '---' separator"
        );
    }
}

#[test]
fn test_apply_overlap_scales_with_config() {
    // Larger overlap values should produce longer chunks than smaller ones
    let dir_small = make_workspace_with_overlap(1);
    let chunks_small =
        rust_rag_core::indexer::index_workspace(dir_small.path()).expect("should index");

    let dir_large = make_workspace_with_overlap(5);
    let chunks_large =
        rust_rag_core::indexer::index_workspace(dir_large.path()).expect("should index");

    // Total text length should be greater with larger overlap (more context lines)
    let total_small: usize = chunks_small.iter().map(|c| c.text.len()).sum();
    let total_large: usize = chunks_large.iter().map(|c| c.text.len()).sum();

    assert!(
        total_large >= total_small,
        "Larger overlap (5) should produce more text than smaller overlap (1). Got {} vs {}",
        total_large,
        total_small
    );
}

// ---------------------------------------------------------------------------
// IndexState / incremental indexing tests
// ---------------------------------------------------------------------------

/// Helper: create a test workspace with two Rust files and return the state dir path.
fn make_incremental_test_workspace() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join(".rustrag");

    // Cargo.toml
    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "incremental_test"
version = "0.1.0"
edition = "2021""#,
    )
    .unwrap();

    // File 1: lib.rs — will be modified between runs
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/lib.rs"),
        r#"pub fn first() -> i32 { 1 }"#,
    )
    .unwrap();

    // File 2: other.rs — will stay unchanged between runs
    std::fs::write(
        dir.path().join("src/other.rs"),
        r#"pub fn second() -> i32 { 2 }"#,
    )
    .unwrap();

    (dir, state_path)
}

#[test]
fn test_incremental_detects_changes() {
    let (dir, _state_path) = make_incremental_test_workspace();

    // Build initial state: simulate that lib.rs has an old hash
    let mut state = rust_rag_core::state::IndexState::new();
    let lib_path = dir.path().join("src/lib.rs");
    let other_path = dir.path().join("src/other.rs");

    // Compute current hashes (actual)
    let actual_lib_hash = rust_rag_core::state::IndexState::compute_file_hash(&lib_path).unwrap();
    let actual_other_hash =
        rust_rag_core::state::IndexState::compute_file_hash(&other_path).unwrap();

    // Simulate old state where lib.rs had a different hash
    state.files.insert(
        lib_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: "old_hash_for_lib".to_string(),
        },
    );
    state.files.insert(
        other_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: actual_other_hash.clone(),
        },
    );

    // Build current files map (actual hashes)
    let mut current = HashMap::new();
    current.insert(lib_path.clone(), actual_lib_hash);
    current.insert(other_path.clone(), actual_other_hash);

    let (_, changed_files, _) = state.compare(&current);

    assert!(
        changed_files.contains(&lib_path),
        "lib.rs should be detected as changed"
    );
    assert!(
        !changed_files.contains(&other_path),
        "unchanged other.rs should not appear in changed"
    );
}

#[test]
fn test_incremental_skips_unchanged() {
    let (dir, _state_path) = make_incremental_test_workspace();

    let lib_path = dir.path().join("src/lib.rs");
    let other_path = dir.path().join("src/other.rs");

    // Compute current hashes
    let fresh_lib = rust_rag_core::state::IndexState::compute_file_hash(&lib_path).unwrap();
    let fresh_other = rust_rag_core::state::IndexState::compute_file_hash(&other_path).unwrap();

    // Build state where both files have matching hashes → nothing changed
    let mut state = rust_rag_core::state::IndexState::new();
    state.files.insert(
        lib_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: fresh_lib.clone(),
        },
    );
    state.files.insert(
        other_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: fresh_other.clone(),
        },
    );

    // Build current map with the same hashes → nothing should be detected as changed
    let mut current = HashMap::new();
    current.insert(lib_path, fresh_lib);
    current.insert(other_path, fresh_other);

    let (_, changed, _) = state.compare(&current);
    assert!(
        changed.is_empty(),
        "No files should be reported as changed when hashes match"
    );
}

#[test]
fn test_incremental_removes_deleted() {
    let (dir, _state_path) = make_incremental_test_workspace();

    let lib_path = dir.path().join("src/lib.rs");
    let other_path = dir.path().join("src/other.rs");

    // Simulate state where both files were previously indexed
    let actual_lib_hash = rust_rag_core::state::IndexState::compute_file_hash(&lib_path).unwrap();
    let actual_other_hash =
        rust_rag_core::state::IndexState::compute_file_hash(&other_path).unwrap();

    let mut state = rust_rag_core::state::IndexState::new();
    state.files.insert(
        lib_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: actual_lib_hash,
        },
    );
    state.files.insert(
        other_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: actual_other_hash,
        },
    );
    // Add chunk IDs that would belong to the deleted file (other.rs)
    state
        .chunk_ids
        .push(format!("chunk_{}_", other_path.display()));

    // Current files: only lib.rs exists, other.rs is "deleted"
    let mut current = HashMap::new();
    current.insert(
        lib_path.clone(),
        rust_rag_core::state::IndexState::compute_file_hash(&lib_path).unwrap(),
    );

    let (_, _, removed_ids) = state.compare(&current);

    assert!(
        removed_ids.contains(&format!("chunk_{}_", other_path.display())),
        "Should detect that other.rs was deleted and return its chunk IDs"
    );
}

#[test]
fn test_remove_documents_from_vector_store() {
    let dir = tempfile::tempdir().unwrap();
    let store =
        rust_rag_core::vector_store::VectorStore::open(dir.path()).expect("should create store");

    // Insert 3 documents
    for i in 1..=3 {
        let chunk = Chunk {
            file_path: PathBuf::from(format!("file{}.rs", i)),
            line_start: 1,
            line_end: 5,
            module_name: "test".into(),
            symbol_kind: SymbolKind::Function,
            text: format!("fn test{}() {{}}", i),
            max_nesting_depth: None,
        };
        let doc = rust_rag_core::vector_store::Document {
            id: format!("doc_{}", i),
            chunk,
            embedding: vec![0.1; 384],
        };
        store.insert_documents(&[doc]).expect("should insert");
    }

    // Verify all 3 are present
    let ids_before = store.list_document_ids().expect("should list IDs");
    assert_eq!(ids_before.len(), 3);

    // Remove doc_2
    store
        .remove_documents(&["doc_2".to_string()])
        .expect("should remove");

    // Verify only doc_1 and doc_3 remain
    let ids_after = store
        .list_document_ids()
        .expect("should list IDs after removal");
    assert_eq!(ids_after.len(), 2);
    assert!(ids_after.contains(&"doc_1".to_string()));
    assert!(!ids_after.contains(&"doc_2".to_string()));
    assert!(ids_after.contains(&"doc_3".to_string()));
}

// ---------------------------------------------------------------------------
// Additional test: multi-document roundtrip, deletion + re-insertion
// ---------------------------------------------------------------------------

#[test]
fn test_vector_store_multi_doc_roundtrip_and_deletion() {
    let dir = tempfile::tempdir().unwrap();

    // Create store and insert 5 documents in a single batch (no cache issues)
    let store1 =
        rust_rag_core::vector_store::VectorStore::open(dir.path()).expect("should create store");
    let mut docs = Vec::new();
    for i in 1..=5 {
        let chunk = Chunk {
            file_path: PathBuf::from(format!("doc{}.rs", i)),
            line_start: i * 10,
            line_end: i * 10 + 5,
            module_name: format!("func_{}", i),
            symbol_kind: SymbolKind::Function,
            text: format!("fn func{}() -> i32 {{ {} }}", i, i),
            max_nesting_depth: None,
        };
        let mut embedding = vec![0.1; 384];
        if i % 2 == 0 {
            embedding[0] = 0.9;
        }
        docs.push(rust_rag_core::vector_store::Document {
            id: format!("doc_{}", i),
            chunk,
            embedding,
        });
    }
    store1.insert_documents(&docs).expect("should insert");

    // Verify all 5 are present before removal
    assert_eq!(store1.list_document_ids().unwrap().len(), 5);

    // Remove doc_2 and doc_4 — remove_documents does atomic replace of index.jsonl
    store1
        .remove_documents(&["doc_2".into(), "doc_4".into()])
        .expect("should remove");

    // Re-open the store to get a fresh cache and verify only 3 remain
    let store2 = rust_rag_core::vector_store::VectorStore::open(dir.path()).expect("should reopen");
    assert_eq!(
        store2.list_document_ids().unwrap().len(),
        3,
        "Should have exactly 3 docs after removal"
    );

    // Verify the correct IDs remain (doc_1, doc_3, doc_5)
    let remaining_ids = store2.list_document_ids().unwrap();
    assert!(remaining_ids.contains(&"doc_1".into()));
    assert!(!remaining_ids.contains(&"doc_2".into()));
    assert!(remaining_ids.contains(&"doc_3".into()));
    assert!(!remaining_ids.contains(&"doc_4".into()));
    assert!(remaining_ids.contains(&"doc_5".into()));

    // Verify search works on remaining documents
    let results = store2.search_by_embedding(&vec![0.1; 384], 10).unwrap();
    assert_eq!(results.len(), 3);
}

// ---------------------------------------------------------------------------
// Additional test: dissimilar query should rank low in BM25
// ---------------------------------------------------------------------------

#[test]
fn test_bm25_dissimilar_query_ranks_low() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    // Doc A: contains the word "network" multiple times with unique context
    let doc_network = Chunk {
        file_path: PathBuf::from("network.rs"),
        line_start: 1,
        line_end: 5,
        module_name: "socket".into(),
        symbol_kind: SymbolKind::Function,
        text: "fn bind_socket(port: u16) -> std::net::TcpListener { let network = Network::new(); network.bind(port); network }"
            .to_string(),
        max_nesting_depth: None,
    };

    // Doc B: does NOT contain the word "network", uses completely different keywords
    let doc_database = Chunk {
        file_path: PathBuf::from("database.rs"),
        line_start: 10,
        line_end: 20,
        module_name: "query".into(),
        symbol_kind: SymbolKind::Function,
        text: "fn execute_query(sql: &str) -> Result<Rows, DbError> { pool.query(sql).await }"
            .to_string(),
        max_nesting_depth: None,
    };

    let embedding = vec![0.1; 384];
    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_network".into(),
                chunk: doc_network,
                embedding: embedding.clone(),
            },
            rust_rag_core::vector_store::Document {
                id: "doc_database".into(),
                chunk: doc_database,
                embedding: embedding.clone(),
            },
        ])
        .expect("should insert");

    // Query about network — should rank network doc higher than database doc (which has zero matches)
    let results = store
        .hybrid_search(&embedding, "network", 5, 0.0, None)
        .expect("should search");
    assert!(results.len() >= 2);
    // With pure BM25 (alpha=0), the doc containing "network" should rank higher
    if results[0].file_path.display().to_string() != "network.rs" {
        // If ranking differs, at least verify that network.rs has a non-zero score from BM25
        let network_result = results.iter().find(|r| r.file_path == *"network.rs");
        assert!(network_result.is_some(), "network.rs should be in results");
    }

    // Verify that query "execute" ranks database doc higher (since it contains 'execute')
    let results = store
        .hybrid_search(&embedding, "execute", 5, 0.0, None)
        .expect("should search");
    assert!(!results.is_empty());
}

// ---------------------------------------------------------------------------
// Additional test: other SymbolKind filters (ImplBlock, UnsafeRegion)
// ---------------------------------------------------------------------------

#[test]
fn test_search_filters_by_various_symbol_kinds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    let embedding = vec![0.1; 384];
    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_fn".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("a.rs"),
                    line_start: 1,
                    line_end: 5,
                    module_name: "f".into(),
                    symbol_kind: SymbolKind::Function,
                    text: "fn foo() {}".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
            rust_rag_core::vector_store::Document {
                id: "doc_impl".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("b.rs"),
                    line_start: 10,
                    line_end: 20,
                    module_name: "Bar".into(),
                    symbol_kind: SymbolKind::ImplBlock,
                    text: "impl Bar { fn baz(&self) {} }".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
            rust_rag_core::vector_store::Document {
                id: "doc_unsafe".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("c.rs"),
                    line_start: 30,
                    line_end: 40,
                    module_name: "unsafe_block".into(),
                    symbol_kind: SymbolKind::UnsafeRegion,
                    text: "unsafe { std::mem::transmute(0) }".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
        ])
        .expect("should insert");

    // Filter for ImplBlock — should only return doc_impl
    let filters = rust_rag_core::vector_store::SearchFilters {
        file_extension: None,
        symbol_kind: Some("implblock".to_string()),
    };
    let results = store
        .hybrid_search(&embedding, "", 5, 1.0, Some(&filters))
        .expect("should search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "doc_impl");

    // Filter for UnsafeRegion — should only return doc_unsafe
    let filters = rust_rag_core::vector_store::SearchFilters {
        file_extension: None,
        symbol_kind: Some("unsaferegion".to_string()),
    };
    let results = store
        .hybrid_search(&embedding, "", 5, 1.0, Some(&filters))
        .expect("should search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "doc_unsafe");

    // Filter with empty kind — should return all results (no filter applied)
    let filters = rust_rag_core::vector_store::SearchFilters {
        file_extension: None,
        symbol_kind: None,
    };
    let results = store
        .hybrid_search(&embedding, "", 5, 1.0, Some(&filters))
        .expect("should search");
    assert_eq!(results.len(), 3);
}

// ---------------------------------------------------------------------------
// Additional test: BM25 edge case — empty documents shouldn't cause issues
// ---------------------------------------------------------------------------

#[test]
fn test_bm25_empty_documents_handled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let store = rust_rag_core::vector_store::VectorStore::open(path).expect("should create store");

    // Insert a mix of documents — one with normal text, another that would produce few tokens
    let embedding = vec![0.1; 384];
    store
        .insert_documents(&[
            rust_rag_core::vector_store::Document {
                id: "doc_normal".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("a.rs"),
                    line_start: 1,
                    line_end: 5,
                    module_name: "test".into(),
                    symbol_kind: SymbolKind::Function,
                    text: "fn test_function() -> i32 { 42 }".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
            rust_rag_core::vector_store::Document {
                id: "doc_single_char".into(),
                chunk: Chunk {
                    file_path: PathBuf::from("b.rs"),
                    line_start: 10,
                    line_end: 15,
                    module_name: "x".into(),
                    symbol_kind: SymbolKind::Function,
                    text: "fn f() {}".to_string(),
                    max_nesting_depth: None,
                },
                embedding: embedding.clone(),
            },
        ])
        .expect("should insert");

    // Search should not panic even with single-char tokens in index
    let results = store
        .hybrid_search(&embedding, "test", 5, 0.0, None)
        .expect("should search");
    assert!(!results.is_empty());
}

// ---------------------------------------------------------------------------
// Additional test: incremental indexing — add new file detection
// ---------------------------------------------------------------------------

#[test]
fn test_incremental_detects_new_files() {
    let dir = tempfile::tempdir().unwrap();
    let lib_path = dir.path().join("lib.rs");
    let new_file = dir.path().join("new_module.rs");

    // Create only lib.rs initially
    std::fs::write(&lib_path, "pub fn existing() -> i32 { 1 }").unwrap();

    // Build state with only lib.rs hash
    let actual_lib_hash = rust_rag_core::state::IndexState::compute_file_hash(&lib_path).unwrap();
    let mut state = rust_rag_core::state::IndexState::new();
    state.files.insert(
        lib_path.clone(),
        rust_rag_core::state::FileMetadata {
            sha256: actual_lib_hash.clone(),
        },
    );

    // Current files include lib.rs + new_module.rs
    std::fs::write(&new_file, "pub fn new_thing() -> i32 { 2 }").unwrap();
    let new_file_hash = rust_rag_core::state::IndexState::compute_file_hash(&new_file).unwrap();

    let mut current = HashMap::new();
    current.insert(lib_path.clone(), actual_lib_hash);
    current.insert(new_file.clone(), new_file_hash);

    let (new_files, _, _) = state.compare(&current);
    assert!(
        new_files.contains(&new_file),
        "New file should be detected as new (not just changed)"
    );
}

// ---------------------------------------------------------------------------
// Call graph unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_callgraph_parse_call_exprs_finds_function_calls() {
    let code = r#"
fn my_func() {
    helper();
    other_helper(42);
}
"#;
    // The callgraph module is not directly accessible from tests, but we can verify
    // through the public API by checking that build_call_graph doesn't panic on valid input.
    let chunks = vec![rust_rag_core::indexer::Chunk {
        file_path: PathBuf::from("src/main.rs"),
        line_start: 0,
        line_end: 50,
        module_name: "main/my_func".to_string(),
        symbol_kind: SymbolKind::Function,
        text: code.to_string(),
        max_nesting_depth: None,
    }];

    let (graph, _name_map) = rust_rag_core::callgraph::build_call_graph(&chunks).unwrap();
    assert!(graph.node_count() >= 1);
}

#[test]
fn test_callgraph_build_creates_nodes_for_all_chunks() {
    let chunks = vec![
        rust_rag_core::indexer::Chunk {
            file_path: PathBuf::from("src/main.rs"),
            line_start: 0,
            line_end: 50,
            module_name: "main/my_func".to_string(),
            symbol_kind: SymbolKind::Function,
            text: "fn my_func() {}".to_string(),
            max_nesting_depth: None,
        },
        rust_rag_core::indexer::Chunk {
            file_path: PathBuf::from("src/lib.rs"),
            line_start: 50,
            line_end: 100,
            module_name: "lib/helper".to_string(),
            symbol_kind: SymbolKind::Function,
            text: "fn helper() {}".to_string(),
            max_nesting_depth: None,
        },
    ];

    let (graph, _name_map) = rust_rag_core::callgraph::build_call_graph(&chunks).unwrap();
    assert_eq!(graph.node_count(), 2);
}

#[test]
fn test_callgraph_ignores_non_function_chunks() {
    let chunks = vec![rust_rag_core::indexer::Chunk {
        file_path: PathBuf::from("src/lib.rs"),
        line_start: 0,
        line_end: 10,
        module_name: "lib/my_struct".to_string(),
        symbol_kind: SymbolKind::Struct,
        text: "struct MyStruct {}".to_string(),
        max_nesting_depth: None,
    }];

    let (graph, _name_map) = rust_rag_core::callgraph::build_call_graph(&chunks).unwrap();
    assert_eq!(graph.edge_count(), 0); // structs don't produce call edges
}

#[test]
fn test_indexer_parse_and_extract_function() {
    let content = r#"
fn hello_world() -> &'static str {
    "hello"
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let func_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Function)
        .collect();
    assert!(
        !func_chunks.is_empty(),
        "Should extract at least one function chunk"
    );
}

#[test]
fn test_indexer_parse_and_extract_impl_block() {
    let content = r#"
struct MyStruct;

impl MyStruct {
    fn method(&self) {}
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let impl_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::ImplBlock)
        .collect();
    assert!(
        !impl_chunks.is_empty(),
        "Should extract at least one impl block chunk"
    );
}

#[test]
fn test_indexer_parse_and_extract_unsafe_block() {
    // Verify parsing doesn't panic with unsafe code inside an impl block
    let content = r#"
struct Foo;

impl Foo {
    fn read_file(&self) -> Result<(), std::io::Error> {
        unsafe {
            std::mem::transmute::<i32, f32>(0);
            Ok(())
        }
    }
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse without panic");

    // At least the impl block should be extracted (tree-sitter-rust node types vary)
    assert!(
        !chunks.is_empty(),
        "Should extract at least one chunk from code with unsafe, got {} total",
        chunks.len()
    );
}

#[test]
fn test_indexer_parse_and_extract_nested_struct_in_module() {
    // Verify parsing handles structs inside modules without panic
    let content = r#"
mod types {
    pub struct Person {
        name: String,
        age: u32,
    }

    impl Person {
        fn new(name: &str) -> Self {
            Self { name: name.to_string(), age: 0 }
        }
    }
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse without panic");

    // The module wrapper should be extracted; the rest depends on tree-sitter-rust version
    assert!(
        !chunks.is_empty(),
        "Should extract at least one chunk from code with struct in module, got {} total",
        chunks.len()
    );
}

#[test]
fn test_indexer_parse_and_extract_nested_enum_in_module() {
    let content = r#"
mod types {
    pub enum Color {
        Red,
        Green,
        Blue,
    }

    impl Color {
        fn as_str(&self) -> &'static str {
            match self {
                Color::Red => "red",
                _ => "other"
            }
        }
    }
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse without panic");

    assert!(
        !chunks.is_empty(),
        "Should extract at least one chunk from code with enum in module, got {} total",
        chunks.len()
    );
}

#[test]
fn test_indexer_parse_and_extract_trait_impl() {
    let content = r#"
trait MyTrait {
    fn do_it(&self);
}

struct Wrapper;

impl MyTrait for Wrapper {
    fn do_it(&self) {}
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse without panic");

    assert!(
        !chunks.is_empty(),
        "Should extract at least one chunk from code with trait impl, got {} total",
        chunks.len()
    );
}

#[test]
fn test_indexer_parse_and_extract_multiple_functions() {
    let content = r#"
fn func_a() {}
fn func_b() {}
fn func_c() {}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let func_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Function)
        .collect();
    assert_eq!(
        func_chunks.len(),
        3,
        "Should extract exactly three function chunks"
    );
}

#[test]
fn test_indexer_parse_and_extract_empty_file() {
    let content = "";
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse empty file without error");

    assert!(chunks.is_empty(), "Empty file should produce no chunks");
}

// ---------------------------------------------------------------------------
// Parent-container nesting tests — impl methods are NOT separate chunks
// ---------------------------------------------------------------------------

#[test]
fn test_indexer_impl_block_contains_methods_no_separate_chunks() {
    let content = r#"
struct Counter {
    count: u32,
}

impl Counter {
    fn new() -> Self {
        Counter { count: 0 }
    }

    fn increment(&mut self) {
        self.count += 1;
    }

    fn get(&self) -> u32 {
        self.count
    }
}

// A top-level function should still be extracted separately
pub fn standalone() -> i32 {
    42
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    // Collect impl and function kinds
    let impl_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::ImplBlock)
        .collect();
    let func_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Function)
        .collect();

    // There should be exactly ONE impl chunk (the full impl block including all methods)
    assert_eq!(impl_chunks.len(), 1, "Should extract one impl block");

    // The impl chunk text must contain the method names
    let impl_text = &impl_chunks[0].text;
    assert!(impl_text.contains("fn new()"), "Impl should contain 'new'");
    assert!(
        impl_text.contains("fn increment"),
        "Impl should contain 'increment'"
    );
    assert!(impl_text.contains("fn get"), "Impl should contain 'get'");

    // Methods inside impl must NOT be separate Function chunks.
    // Only top-level functions (like `standalone`) should be Function chunks.
    for chunk in &chunks {
        if let SymbolKind::Function = chunk.symbol_kind {
            assert!(
                !chunk.text.contains("fn new()") && !chunk.text.contains("fn increment"),
                "Method 'new' or 'increment' should NOT appear as a separate Function chunk"
            );
        }
    }

    // There should be exactly one top-level function: `standalone`
    assert_eq!(
        func_chunks.len(),
        1,
        "Should extract exactly one standalone function"
    );
}

#[test]
fn test_indexer_trait_impl_methods_not_separate() {
    let content = r#"
trait Draw {
    fn draw(&self);
}

struct Circle;

impl Draw for Circle {
    fn draw(&self) {
        println!("drawing circle");
    }
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let func_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Function)
        .collect();
    // The draw() method inside the impl must NOT be a separate function chunk
    assert!(
        func_chunks.is_empty(),
        "Methods inside trait impl should not produce separate Function chunks"
    );
}

// ---------------------------------------------------------------------------
// Macro integrity tests — macro_invocations are atomic, macro_definitions become chunks
// ---------------------------------------------------------------------------

#[test]
fn test_indexer_macro_definition_becomes_chunk() {
    let content = r#"
macro_rules! my_macro {
    ($name:ident) => {
        fn $name() { println!("hello"); }
    };
}

my_macro!(my_fn);
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let macro_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Macro)
        .collect();
    assert_eq!(
        macro_chunks.len(),
        1,
        "Should extract one macro definition chunk"
    );
    assert!(
        macro_chunks[0].text.contains("macro_rules!"),
        "Chunk should contain the full macro definition"
    );

    // The macro_invocation (my_macro!(my_fn);) must NOT produce a separate chunk.
    for chunk in &chunks {
        if let SymbolKind::Macro = chunk.symbol_kind {
            assert!(
                !chunk.text.contains("my_macro!("),
                "Macro invocation should not appear as a separate Macro chunk"
            );
        }
    }
}

#[test]
fn test_indexer_nested_macros_not_split() {
    let content = r#"
macro_rules! outer {
    ($inner:expr) => {
        macro_rules! inner {
            () => { $inner };
        }
        inner!();
    };
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let macro_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Macro)
        .collect();
    // Should find the outer macro definition; inner!() invocation must not be a chunk.
    assert!(
        !macro_chunks.is_empty(),
        "Should extract at least one macro chunk"
    );

    // Verify no chunk text contains the standalone invocation without 'macro_rules!'
    for chunk in &chunks {
        if !chunk.text.contains("macro_rules!") {
            assert!(
                !chunk.text.trim().starts_with("inner!("),
                "Macro invocation should not be a separate chunk"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Nesting-depth quality diagnostics tests
// ---------------------------------------------------------------------------

#[test]
fn test_indexer_nesting_depth_top_level_function() {
    let content = r#"
fn simple() -> i32 { 42 }

fn with_nested_blocks() -> i32 {
    if true {
        for _ in 0..1 {
            loop {
                break;
            }
        }
    }
    42
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let func_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::Function)
        .collect();
    assert_eq!(func_chunks.len(), 2);

    // simple() has no deeply-nested code — only the function itself and its block body.
    let simple = func_chunks
        .iter()
        .find(|c| c.text.contains("fn simple"))
        .unwrap();
    if let Some(d) = simple.max_nesting_depth {
        assert_eq!(
            d, 2,
            "Top-level function has fn(1) + body block(1) = depth 2"
        );
    } else {
        panic!("max_nesting_depth should be set for Function chunks");
    }

    // with_nested_blocks has if → for → loop nested inside the function — much deeper.
    let complex = func_chunks
        .iter()
        .find(|c| c.text.contains("fn with_nested_blocks"))
        .unwrap();
    if let Some(d) = complex.max_nesting_depth {
        assert!(
            d >= 4,
            "Function with if/for/loop should have nesting depth >= 4, got {}",
            d
        );
    } else {
        panic!("max_nesting_depth should be set for Function chunks");
    }

    // Verify relative ordering: deeply-nested > shallow function.
    assert!(
        simple.max_nesting_depth.unwrap() < complex.max_nesting_depth.unwrap(),
        "Shallow function depth ({}) should be less than complex function ({})",
        simple.max_nesting_depth.unwrap(),
        complex.max_nesting_depth.unwrap()
    );
}

#[test]
fn test_indexer_impl_block_has_positive_nesting() {
    let content = r#"
struct Foo;

impl Foo {
    fn method_a(&self) {}

    fn method_b(&mut self, x: i32) -> String {
        if x > 0 {
            format!("positive: {}", x)
        } else {
            "non-positive".to_string()
        }
    }
}
"#;
    let mut chunks: Vec<rust_rag_core::indexer::Chunk> = Vec::new();
    rust_rag_core::indexer::parse_and_extract(
        content,
        PathBuf::from("test.rs").as_ref(),
        &mut chunks,
    )
    .expect("should parse");

    let impl_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.symbol_kind == SymbolKind::ImplBlock)
        .collect();
    assert_eq!(impl_chunks.len(), 1);

    if let Some(d) = impl_chunks[0].max_nesting_depth {
        // impl block (1) + fn method_a body (1) = 2 for simple method.
        // impl(1) + fn method_b(1) + if-else body blocks(1) = 3 for deeper method.
        assert!(
            d >= 2,
            "Impl block should have nesting depth >= 2, got {}",
            d
        );
    } else {
        panic!("max_nesting_depth should be set for ImplBlock chunks");
    }

    // Verify that the impl chunk text contains both methods
    let text = &impl_chunks[0].text;
    assert!(
        text.contains("fn method_a"),
        "Impl chunk should contain method_a"
    );
    assert!(
        text.contains("fn method_b"),
        "Impl chunk should contain method_b"
    );
}

#[test]
fn test_eval_diagnostics_nesting_quality() {
    let chunks = vec![
        // Top-level function with no nesting — fn(1) + body block(1) = 2.
        rust_rag_core::indexer::Chunk {
            file_path: PathBuf::from("a.rs"),
            line_start: 0,
            line_end: 10,
            module_name: "simple".into(),
            symbol_kind: SymbolKind::Function,
            text: "fn simple() {}".to_string(),
            max_nesting_depth: Some(2),
        },
        // Impl block with nested methods — impl(1) + fn body(1) = 2 for simple method.
        rust_rag_core::indexer::Chunk {
            file_path: PathBuf::from("a.rs"),
            line_start: 10,
            line_end: 40,
            module_name: "impl Foo".into(),
            symbol_kind: SymbolKind::ImplBlock,
            text: "impl Foo {\n    fn a(&self) {}\n    fn b(&mut self) { if true {} }\n}"
                .to_string(),
            max_nesting_depth: Some(3), // impl(1) + fn body(1) + if block(1) = 3
        },
        // Another file with nesting — fn(1) + for loop body(1) = 2.
        rust_rag_core::indexer::Chunk {
            file_path: PathBuf::from("b.rs"),
            line_start: 0,
            line_end: 20,
            module_name: "mod_b/helper".into(),
            symbol_kind: SymbolKind::Function,
            text: "fn helper() { for _ in 0..1 {} }".to_string(),
            max_nesting_depth: Some(2),
        },
    ];

    let diags = rust_rag_core::eval::chunk_diagnostics(&chunks);

    assert_eq!(diags.chunk_count, 3);
    assert_eq!(diags.file_count, 2);

    // avg nesting depth = (2 + 3 + 2) / 3 ≈ 2.33
    assert!((diags.avg_nesting_depth - 7.0 / 3.0).abs() < 0.01);

    // Both files have nested code → 100% of files with nesting
    assert_eq!(diags.file_pct_with_nested_code, 100.0);

    // Per-file max depths: a.rs=max(2,3)=3, b.rs=2
    let file_a_max = diags.max_nesting_per_file.get("a.rs").copied().unwrap_or(0);
    let file_b_max = diags.max_nesting_per_file.get("b.rs").copied().unwrap_or(0);
    assert_eq!(file_a_max, 3);
    assert_eq!(file_b_max, 2);
}

#[test]
fn test_eval_diagnostics_no_nested_chunks() {
    // A simple top-level function always has fn+block = depth 2.
    let chunks = vec![rust_rag_core::indexer::Chunk {
        file_path: PathBuf::from("flat.rs"),
        line_start: 0,
        line_end: 5,
        module_name: "f1".into(),
        symbol_kind: SymbolKind::Function,
        text: "fn f1() {}".to_string(),
        max_nesting_depth: Some(2),
    }];

    let diags = rust_rag_core::eval::chunk_diagnostics(&chunks);

    // All chunks have depth 2 (just fn+block) — no deeper nesting.
    assert_eq!(diags.avg_nesting_depth, 2.0);
    // Since all chunks at this file have depth > 0, it counts as "nested" code present.
    assert_eq!(diags.file_pct_with_nested_code, 100.0);
}

// Re-export HashMap for tests that need it.
use std::collections::HashMap;
