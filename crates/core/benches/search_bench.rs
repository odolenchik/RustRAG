use criterion::{Criterion, black_box, criterion_group, criterion_main};
use rust_rag_core::indexer;
use std::time::Duration;

/// Generate a test workspace with `file_count` Rust files.
fn generate_test_workspace(dir: &tempfile::TempDir, file_count: usize) {
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "bench_workspace"
version = "0.1.0"
edition = "2021""#,
    )
    .unwrap();

    for i in 0..file_count {
        let file_path = src_dir.join(format!("lib_{}.rs", i));
        let content = generate_rust_code(3); // ~3 AST nodes per file (small)
        std::fs::write(&file_path, &content).unwrap_or_else(|_| {
            panic!("failed to write {}", file_path.display())
        });
    }

    std::fs::write(
        dir.path().join(".rustrag.toml"),
        "[embedding]\nchunk_overlap = 0",
    )
    .unwrap();
}

fn generate_rust_code(num_nodes: usize) -> String {
    let mut code = String::new();
    for i in 0..num_nodes {
        // Generate realistic Rust struct + impl block (~12 lines each)
        let struct_name = format!("BenchmarkStruct{}", i);
        let fn_name = format!("bench_method_{}", i);

        code.push_str(&format!(
            "pub struct {} {{\n    data: Vec<u8>,\n}}\n\n",
            struct_name
        ));
        code.push_str("impl ");
        code.push_str(&struct_name);
        code.push_str(r#" {
    pub fn "#);
        code.push_str(&fn_name);
        code.push_str(r#"(&self) -> String {
        self.data.iter().map(|b| *b as char).collect()
    }
}

"#);
    }
    code
}

/// Populate a VectorStore with all chunks from the given workspace.
fn populate_store(dir: &tempfile::TempDir) -> (rust_rag_core::vector_store::VectorStore, Vec<f32>) {
    let chunks = indexer::index_workspace(dir.path()).expect("should index");
    assert!(!chunks.is_empty(), "should produce chunks for benchmarking");

    // Use a stable query vector pattern — cosine similarity to document embeddings.
    let query_vec: Vec<f32> = vec![0.5; 384];

    let store_dir = tempfile::tempdir().expect("should create store dir");
    let store = rust_rag_core::vector_store::VectorStore::open(&store_dir).expect("should open store");

    // Batch insert all chunks with deterministic embeddings (32 at a time)
    let chunk_batch: Vec<_> = chunks.iter().take(32).map(|c| {
        rust_rag_core::vector_store::Document {
            id: format!("chunk_{}", c.module_name),
            chunk: c.clone(),
            embedding: (0..384).map(|i| if i % 2 == 0 { 0.7 } else { 0.1 }).collect(),
        }
    }).collect();
    store.insert_documents(&chunk_batch).expect("should insert");

    // If total chunks > 32, add remaining documents in a second batch
    if chunks.len() > 32 {
        let remaining: Vec<_> = chunks.iter().skip(32).map(|c| {
            rust_rag_core::vector_store::Document {
                id: format!("chunk_{}", c.module_name),
                chunk: c.clone(),
                embedding: (0..384).map(|i| if i % 2 == 0 { 0.7 } else { 0.1 }).collect(),
            }
        }).collect();
        store.insert_documents(&remaining).expect("should insert");
    }

    (store, query_vec)
}

fn search_benchmark(c: &mut Criterion) {
    // Use very few files — indexing is tree-sitter slow.
    let file_count = 10;
    let dir = tempfile::tempdir().expect("should create temp dir");
    generate_test_workspace(&dir, file_count);

    // Build and populate the store once (outside iteration)
    let (store, query_vec) = populate_store(&dir);

    {
        let mut group = c.benchmark_group("search_latency_p50_top1");
        group.sample_size(10);
       group.warm_up_time(Duration::from_millis(50));
        group.measurement_time(Duration::from_millis(200));
        group.bench_function("search", |b| {
            b.iter(|| {
                let _results = black_box(store.hybrid_search(
                    black_box(&query_vec),
                    black_box("benchmark query"),
                    black_box(1), // top_k=1 -> p50 latency (fast path)
                    black_box(0.7), // alpha blending
                    None,
                )).unwrap();
            });
        });
    }

    {
        let mut group = c.benchmark_group("search_latency_p95_top10");
        group.sample_size(10);
        group.warm_up_time(Duration::from_millis(50));
        group.measurement_time(Duration::from_millis(200));
        group.bench_function("search", |b| {
            b.iter(|| {
                let _results = black_box(store.hybrid_search(
                    black_box(&query_vec),
                    black_box("benchmark query"),
                    black_box(10), // top_k=10 -> p95 latency
                    black_box(0.7),
                    None,
                )).unwrap();
            });
        });
    }

    {
        let mut group = c.benchmark_group("search_latency_p99_top50");
        group.sample_size(10);
        group.warm_up_time(Duration::from_millis(50));
        group.measurement_time(Duration::from_millis(200));
        group.bench_function("search", |b| {
            b.iter(|| {
                let _results = black_box(store.hybrid_search(
                    black_box(&query_vec),
                    black_box("benchmark query"),
                    black_box(50), // top_k=50 -> tail latency (p99)
                    black_box(0.7),
                    None,
                )).unwrap();
            });
        });
    }
}

criterion_group!(benches, search_benchmark);
criterion_main!(benches);
