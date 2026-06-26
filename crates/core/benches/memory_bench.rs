#![allow(clippy::needless_range_loop)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_rag_core::indexer;
use std::time::Duration;

/// Generate a test workspace with `file_count` Rust files for memory benchmarking.
fn generate_test_workspace(dir: &tempfile::TempDir, file_count: usize) {
    let src_dir = dir.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "memory_bench_workspace"
version = "0.1.0"
edition = "2021""#,
    )
    .unwrap();

    for i in 0..file_count {
        let file_path = src_dir.join(format!("lib_{}.rs", i));
        let content = generate_rust_code(5); // ~5 AST nodes per file, modest size
        std::fs::write(&file_path, &content)
            .unwrap_or_else(|_| panic!("failed to write {}", file_path.display()));
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
        // Generate ~30 lines per node to simulate real-world Rust code size
        let struct_name = format!("MemoryBenchmarkStruct{}", i);
        let fn_name = format!("bench_method_{}", i);

        code.push_str(&format!(
            "pub struct {} {{\n    data: Vec<u8>,\n    metadata: std::collections::HashMap<String, String>,\n}}\n\n",
            struct_name
        ));
        code.push_str(&format!(
            "impl {} {{\n    pub fn {}(&self) -> Result<String, anyhow::Error> {{\n",
            struct_name, fn_name
        ));
        code.push_str(
            "        let result = self.data.iter().map(|b| *b as char).collect::<String>();\n",
        );
        code.push_str("        let meta_count = self.metadata.len();\n");
        let ok_line = format!(r#"        Ok(format!("{} count={{}} size={{}}"#, fn_name);
        code.push_str(&ok_line);
        code.push_str(r#", meta_count, result.len()))"#);
        code.push_str(";\n");
        code.push_str("    }\n\n    pub fn process(&self, input: &[u8]) -> Vec<u8> {\n");
        code.push_str("        input.iter().map(|b| b.wrapping_add(1)).collect()\n    }\n}\n\n");
    }
    code
}

/// Pre-warm the vector store: index workspace + insert all documents.
fn prewarm(dir: &tempfile::TempDir) -> (rust_rag_core::vector_store::VectorStore, Vec<f32>) {
    let chunks = indexer::index_workspace(dir.path()).expect("should index");

    let query_vec: Vec<f32> = vec![0.5; 384];

    let store_dir = tempfile::tempdir().expect("should create store dir");
    let store =
        rust_rag_core::vector_store::VectorStore::open(&store_dir).expect("should open store");

    // Insert all documents in one batch (no per-bench-iteration overhead)
    let _ = chunks.first();
    if !chunks.is_empty() {
        let batch: Vec<_> = chunks
            .iter()
            .take(32)
            .map(|c| rust_rag_core::vector_store::Document {
                id: format!("chunk_{}", c.module_name),
                chunk: c.clone(),
                embedding: (0..384)
                    .map(|i| if i % 2 == 0 { 0.7 } else { 0.1 })
                    .collect(),
            })
            .collect();
        store.insert_documents(&batch).expect("should insert");

        if chunks.len() > 32 {
            let remaining: Vec<_> = chunks
                .iter()
                .skip(32)
                .map(|c| rust_rag_core::vector_store::Document {
                    id: format!("chunk_{}", c.module_name),
                    chunk: c.clone(),
                    embedding: (0..384)
                        .map(|i| if i % 2 == 0 { 0.7 } else { 0.1 })
                        .collect(),
                })
                .collect();
            store.insert_documents(&remaining).expect("should insert");
        }

        // first batch already populated everything we need for search measurement
    }

    (store, query_vec)
}

fn memory_benchmark(c: &mut Criterion) {
    // Use very tiny workloads — tree-sitter parsing is slow.
    let file_counts = [10, 30];

    for &count in &file_counts {
        let mut group = c.benchmark_group(format!("memory_usage_{}_files", count));
        group.sample_size(10);
        group.warm_up_time(Duration::from_millis(50));
        group.measurement_time(Duration::from_millis(500));
        group.bench_function("search", |b| {
            // Pre-warm once outside the iter — measure only search memory behavior
            let dir = tempfile::tempdir().expect("should create temp dir");
            generate_test_workspace(&dir, count);

            let (store, query_vec) = prewarm(&dir);

            b.iter(|| {
                let _results = store
                    .hybrid_search(
                        black_box(&query_vec),
                        black_box("memory benchmark"),
                        black_box(10),
                        black_box(0.7),
                        None,
                    )
                    .unwrap();
            });
        });
    }
}

criterion_group!(benches, memory_benchmark);
criterion_main!(benches);
