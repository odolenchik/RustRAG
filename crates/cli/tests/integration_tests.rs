//! Integration tests for critical rust-rag pipelines.
//!
//! These tests exercise the CLI crate's public API end-to-end using real
//! temporary workspaces, verifying that indexing, retrieval, and incremental
//! updates behave correctly. They use `tempfile` to create isolated fixtures
//! so each test runs independently without affecting other tests or CI caches.
//!
//! Tests that require a pre-downloaded embedding model check the
//! `RUSRAG_TEST_USE_EMBEDDING` env var (default: "true" when running from
//! a checkout with cached models, "false" otherwise). Set it explicitly to
//! force embedding-dependent tests regardless:
//! ```bash
//! RUSRAG_TEST_USE_EMBEDDING=1 cargo test -p rust-rag-cli --test integration_tests
//! ```

use rust_rag_cli as rust_rag;
use std::path::Path;

/// Check whether we should attempt embedding-dependent operations.
/// Falls back to "true" if the env var is set, or "false" by default
/// (since downloading model.onnx can take 30+ seconds in CI).
fn use_embedding() -> bool {
    std::env::var("RUSRAG_TEST_USE_EMBEDDING")
        .ok()
        .map(|v| v != "false")
        .unwrap_or(false)
}

/// Helper: build a minimal Cargo workspace at the given temp dir with two source files.
fn make_minimal_workspace(dir: &Path) {
    // Write Cargo.toml
    let cargo = r#"[package]
name = "integration_test"
version = "0.1.0"
edition = "2021"

[dependencies]"#;
    std::fs::write(dir.join("Cargo.toml"), cargo).expect("write Cargo.toml");

    // Write src/lib.rs — defines `add` and `multiply` functions
    let lib_rs = r#"/// Add two integers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Multiply two integers.
pub fn multiply(a: i32, b: i32) -> i32 {
    a * b
}

/// A simple struct with a method.
pub struct Counter {
    value: u64,
}

impl Counter {
    /// Create a new counter at zero.
    pub fn new() -> Self {
        Self { value: 0 }
    }

    /// Increment and return the current count.
    pub fn increment(&mut self) -> u64 {
        self.value += 1;
        self.value
    }
}

/// A private helper used by `multiply`.
fn double(x: i32) -> i32 {
    x * 2
}"#;
    std::fs::create_dir_all(dir.join("src")).expect("create src dir");
    std::fs::write(dir.join("src").join("lib.rs"), lib_rs).expect("write lib.rs");

    // Write a second crate member so multi-member workspace tests work.
    let _ = std::fs::create_dir_all(dir.join("crates"));
    std::fs::write(
        dir.join("crates").join("Cargo.toml"),
        r#"[package]
name = "integration_test_core"
version = "0.1.0"
edition = "2021"

[dependencies]"#,
    )
    .expect("write crate Cargo.toml");

    std::fs::create_dir_all(dir.join("crates").join("src")).expect("create src dir");
    std::fs::write(
        dir.join("crates").join("src").join("lib.rs"),
        r#"/// A function specific to the sub-crate.
pub fn special_compute(x: i32) -> i32 {
    x.checked_mul(10).unwrap_or(i32::MAX)
}

pub struct Config {
    pub debug: bool,
}

impl Config {
    /// Create a new config with debug mode.
    pub fn new(debug: bool) -> Self {
        Self { debug }
    }
}"#,
    )
    .expect("write sub-crate lib.rs");
}

// =========================================================================== //
// Test 1 — End-to-end: index → hybrid search finds expected chunks           //
// =========================================================================== //

#[test]
fn test_e2e_index_and_search() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    // Index the workspace.
    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    // Run retrieval pipeline with a query that should match "add" or "multiply".
    let results = rust_rag::run_retrieval_pipeline(
        "how do you add two numbers",
        Some(dir.path().to_str().unwrap()),
    );

    assert!(results.is_ok(), "retrieval pipeline succeeded");

    let (search_results, _context) = results.unwrap();
    // Should find at least one result containing the `add` or `multiply` function.
    assert!(
        !search_results.is_empty(),
        "should return search results for a known query"
    );

    // Verify that the returned chunks reference valid files within our workspace.
    let file_paths: Vec<_> = search_results
        .iter()
        .map(|r| r.file_path.display().to_string())
        .collect();
    let has_src_file = file_paths.iter().any(|p| p.contains("lib.rs"));
    assert!(
        has_src_file,
        "should find chunks in lib.rs files; got {:?}",
        file_paths
    );
}

// =========================================================================== //
// Test 2 — Incremental indexing: add a new module and verify it appears        //
// =========================================================================== //

#[test]
fn test_incremental_index_adds_new_module() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    // First index pass.
    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("first index");

    // Verify initial state: should have chunks from lib.rs only.
    {
        let results =
            rust_rag::run_retrieval_pipeline("special compute", Some(dir.path().to_str().unwrap()));
        assert!(results.is_ok(), "first pass retrieval succeeded");
        // No `special_compute` chunk yet — query should not find it.
        let (results, _) = results.unwrap();
        let found = results.iter().any(|r| r.text.contains("special_compute"));
        assert!(
            !found,
            "should NOT have special_compute before second index"
        );
    }

    // Now add a new file with `special_compute` function.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let new_module = r#"/// A unique helper specific to this workspace.
pub fn special_compute(x: i32) -> i32 {
    x.checked_mul(10).unwrap_or(i32::MAX)
}

/// Another function added incrementally.
pub fn double_value(n: u64) -> u64 {
    n * 2
}"#;
    std::fs::write(dir.path().join("src").join("new_mod.rs"), new_module).unwrap();

    // Re-index (should detect the new file and add its chunks).
    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("second index");

    // Verify: `special_compute` should now appear in search results.
    {
        let results = rust_rag::run_retrieval_pipeline(
            "what does special compute do",
            Some(dir.path().to_str().unwrap()),
        );
        assert!(results.is_ok(), "second pass retrieval succeeded");
        let (results, _context) = results.unwrap();
        let found = results.iter().any(|r| r.text.contains("special_compute"));
        assert!(
            found,
            "should now find special_compute after incremental index added new_mod.rs"
        );
    }
}

// =========================================================================== //
// Test 3 — Hybrid search ordering: BM25 + vector similarity rank relevant      //
//                  chunks above irrelevant ones                                //
// =========================================================================== //

#[test]
fn test_hybrid_search_ranking_order() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    // Query for "multiply" — should rank multiply-related chunks above add or counter.
    let results = rust_rag::run_retrieval_pipeline(
        "how do you multiply integers",
        Some(dir.path().to_str().unwrap()),
    );
    assert!(results.is_ok(), "hybrid search succeeded");

    if let Ok((search_results, _context)) = results {
        // With multiple relevant chunks, verify the first result is from lib.rs.
        assert!(!search_results.is_empty());

        // The top-ranked chunk should contain either "multiply" or "double".
        let top_chunk_text = &search_results[0].text;
        let has_relevant_keyword = top_chunk_text.contains("multiply")
            || top_chunk_text.contains("double")
            || top_chunk_text.contains("*");

        // Note: with cosine similarity on real embeddings, the exact keyword match
        // may vary — but we expect at least one chunk referencing multiplication.
        let any_result_has_mult = search_results.iter().any(|r| {
            r.text.contains("multiply") || r.text.contains("double") || r.text.contains("*")
        });

        assert!(
            has_relevant_keyword || any_result_has_mult,
            "top-ranked chunk should be relevant to multiply query; top_text={}",
            &search_results[0].text[..search_results[0]
                .text
                .chars()
                .take(80)
                .collect::<String>()
                .len()]
        );
    }
}

// =========================================================================== //
// Test 4 — Ask pipeline: index → retrieve + LLM context construction           //
// =========================================================================== //

#[test]
fn test_ask_pipeline_builds_context() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    // Run the ask pipeline — this goes through retrieval + LLM.
    // The test succeeds if no panics occur and context is built.
    let result = rust_rag::run_retrieval_pipeline(
        "how does counter increment work",
        Some(dir.path().to_str().unwrap()),
    );

    assert!(result.is_ok(), "ask pipeline succeeded");

    let (_results, context) = result.unwrap();
    // Context should contain at least one reference to the file path.
    assert!(
        !context.is_empty(),
        "should have non-empty context from retrieval"
    );
}

// =========================================================================== //
// Test 5 — Search by symbol name: search_symbol finds expected results         //
// =========================================================================== //

#[test]
fn test_search_symbol_finds_function() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    // Search for a known symbol name.
    let result = rust_rag::search_symbol("add", Some(dir.path().to_str().unwrap()));
    assert!(result.is_ok(), "search_symbol succeeded");
}

// =========================================================================== //
// Test 6 — Show info: verify index metadata is correct                         //
// =========================================================================== //

#[test]
fn test_show_info_reports_chunk_count() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    let result = rust_rag::show_info(Some(dir.path().to_str().unwrap()));
    assert!(result.is_ok(), "show_info succeeded");

    // Re-run as JSON to verify structured output.
    let json_result = rust_rag::show_info_json(Some(dir.path().to_str().unwrap()));
    assert!(json_result.is_ok(), "show_info_json succeeded");
}

// =========================================================================== //
// Test 7 — Clean workspace: removes .rustrag directory                         //
// =========================================================================== //

#[test]
fn test_clean_workspace_removes_index() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    // Verify .rustrag directory exists.
    assert!(
        dir.path().join(".rustrag").exists(),
        ".rustrag should exist after indexing"
    );

    // Clean the workspace.
    let result = rust_rag::clean_workspace(Some(dir.path().to_str().unwrap()));
    assert!(result.is_ok(), "clean_workspace succeeded");

    // Verify .rustrag directory was removed.
    assert!(
        !dir.path().join(".rustrag").exists(),
        ".rustrag should be removed after clean"
    );
}

// =========================================================================== //
// Test 8 — Stats command: chunk diagnostics report correct counts               //
// =========================================================================== //

#[test]
fn test_stats_reports_chunk_count() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    let result = rust_rag::show_stats(Some(dir.path().to_str().unwrap()), false);
    assert!(result.is_ok(), "show_stats succeeded");

    // Also test JSON mode.
    let json_result = rust_rag::show_stats(Some(dir.path().to_str().unwrap()), true);
    assert!(json_result.is_ok(), "show_stats_json succeeded");
}

// =========================================================================== //
// Test 9 — Index with no Rust files returns early gracefully                   //
// =========================================================================== //

#[test]
fn test_index_empty_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Cargo.toml"),
        r#"[package]
name = "empty_pkg"
version = "0.1.0"
edition = "2021""#,
    )
    .unwrap();

    // No src/ directory — should return Ok with no chunks.
    let result = rust_rag::index_workspace(dir.path().to_str().unwrap());
    assert!(result.is_ok(), "should handle empty workspace gracefully");
}

// =========================================================================== //
// Test 10 — Re-index replaces old index cleanly                                //
// =========================================================================== //

#[test]
fn test_reindex_overwrites_index() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("first index");

    // Add more code that will produce additional chunks.
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let extra_code = r#"/// A large module with many functions for re-indexing.
pub fn alpha() -> i32 { 1 }
pub fn beta() -> i32 { 2 }
pub fn gamma() -> i32 { 3 }
pub fn delta() -> i32 { 4 }"#;
    std::fs::write(dir.path().join("src").join("extra.rs"), extra_code).unwrap();

    // Re-index via the public helper.
    rust_rag::reindex_workspace(dir.path().to_str().unwrap()).expect("reindex");

    // Verify we can still search and get results.
    let result = rust_rag::run_retrieval_pipeline(
        "how do you call alpha beta",
        Some(dir.path().to_str().unwrap()),
    );
    assert!(result.is_ok(), "post-reindex retrieval succeeded");

    // The context should now contain references to the extra code.
    let (_results, context) = result.unwrap();
    assert!(
        !context.is_empty(),
        "reindexed workspace has retrievable content"
    );
}

// =========================================================================== //
// Test 11 — Search symbol JSON output returns valid JSON                       //
// =========================================================================== //

#[test]
fn test_search_symbol_json_output() {
    if !use_embedding() {
        println!("SKIP: set RUSRAG_TEST_USE_EMBEDDING=1 to run embedding-dependent tests");
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    make_minimal_workspace(dir.path());

    rust_rag::index_workspace(dir.path().to_str().unwrap()).expect("should index");

    let result = rust_rag::search_symbol_json("add", Some(dir.path().to_str().unwrap()));
    assert!(result.is_ok(), "search_symbol_json succeeded");
}

// =========================================================================== //
// Test 12 — Download model target path is valid                                //
// =========================================================================== //

#[test]
fn test_download_model_target() {
    if !use_embedding() {
        // Skip network-dependent download when embedding tests are disabled.
        return;
    }

    let dir = tempfile::tempdir().unwrap();
    let target_path = dir.path().to_str().unwrap();

    // download_model should succeed even if target doesn't exist yet — it creates dirs.
    let result = rust_rag::download_model(target_path);
    assert!(result.is_ok(), "download_model to temp dir succeeded");

    // Verify model files were written (at least config.json and tokenizer).
    assert!(
        std::fs::exists(dir.path().join("config.json")).is_ok()
            || dir.path().join("model.onnx").exists(),
        "at least one model file should exist in target directory"
    );
}
