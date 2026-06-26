use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_rag_core::indexer;
use std::time::Duration;
use tempfile::TempDir;

/// Generate Rust source files with realistic content for benchmarking.
fn generate_rust_files(dir: &TempDir, file_count: usize) {
    // Members live at workspace root level (same as Cargo.toml), not under src/
    for member_idx in 0..3usize {
        let member_dir = dir.path().join(format!("member_{}", member_idx));
        std::fs::create_dir_all(&member_dir)
            .unwrap_or_else(|_| panic!("failed to create {}", member_dir.display()));
        // Write a minimal Cargo.toml for each member so it's a valid crate root
        let cargo_path = member_dir.join("Cargo.toml");
        if !cargo_path.exists() {
            std::fs::write(
                &cargo_path,
                r#"[package]
name = "member_0"
version = "0.1.0"
edition = "2021""#,
            )
            .unwrap();
        }
    }

    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[workspace]
members = ["member_0", "member_1", "member_2"]

[workspace.package]
version = "0.1.0"
edition = "2021""#,
    )
    .expect("should write Cargo.toml");

    for i in 0..file_count {
        let member_idx = i % 3;
        let member_name = format!("member_{}", member_idx);
        let file_num = (i / 3) + 1;
        // Source files live under src/ inside each member crate
        let src_dir = dir.path().join(format!("{}/src", member_name));
        std::fs::create_dir_all(&src_dir)
            .unwrap_or_else(|_| panic!("failed to create {}", src_dir.display()));

        let file_path = src_dir.join(format!("lib_{}.rs", file_num));

        // Generate a chunk of Rust code with functions, impls, structs, etc.
        let content = generate_rust_code(5); // ~5 AST nodes per file for realistic size
        std::fs::write(&file_path, &content)
            .unwrap_or_else(|_| panic!("failed to write {}", file_path.display()));
    }

    // Write .rustrag.toml with overlap=0 for fastest parsing (overlap adds I/O)
    std::fs::write(
        dir.path().join(".rustrag.toml"),
        "[embedding]\nchunk_overlap = 0",
    )
    .unwrap();
}

fn generate_rust_code(num_nodes: usize) -> String {
    let mut code = String::new();
    for i in 0..num_nodes {
        // Each "node" is a realistic Rust construct ~15-30 lines
        let struct_name = format!("Struct{}", i);
        let fn_name = format!("method_{}", i);

        code.push_str(&format!(
            "pub struct {} {{\n    field_a: String,\n    field_b: usize,\n}}\n\n",
            struct_name
        ));
        code.push_str(&format!(
            "impl {} {{\n    pub fn {}(&self) -> Result<String, anyhow::Error> {{\n",
            struct_name, fn_name
        ));
        code.push_str(
            "        let result = self.field_a.clone() + &format!(\"value={}\", self.field_b);\n",
        );
        code.push_str("        Ok(result)\n    }\n}\n\n");
    }
    code
}

fn indexing_benchmark(c: &mut Criterion) {
    // Use tiny workloads — tree-sitter parsing is slow, keep files small.
    let sizes = [5, 20]; // ~5 and ~20 files per member crate (3 crates total)

    for &size in &sizes {
        let mut group = c.benchmark_group(format!("index_workspace_{}_files", size * 3));
        group.sample_size(10);
        group.warm_up_time(Duration::from_millis(50));
        group.measurement_time(Duration::from_millis(500));

        group.bench_function("index", |b| {
            let dir = tempfile::tempdir().expect("should create temp dir");
            generate_rust_files(&dir, size);

            b.iter(|| {
                let _chunks = indexer::index_workspace(black_box(dir.path()));
            });
        });
    }
}

criterion_group!(benches, indexing_benchmark);
criterion_main!(benches);
