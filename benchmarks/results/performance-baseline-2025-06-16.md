# Benchmark Results — Performance Optimization Baseline (2025-06-16)

## Context

This benchmark captures the baseline performance of RustRAG after the following optimizations:

1. **Semantic Cache: append-only WAL** (`crates/core/src/semantic_cache.rs`)
   - Replaced full HashMap serialization on every `write_back()` with append-only log
   - Duplicate detection prevents redundant disk I/O
   - Auto-compaction at >200 entries threshold

2. **Indexer: parallel parsing via rayon** (`crates/indexer/src/lib.rs`)
   - Added `rayon` for parallel file processing
   - `par_iter().map()` with lock-free collection, flatten only on join
   - Prune `.git`, `target`, `node_modules` at path-collection phase

3. **Cosine similarity: SIMD-friendly unrolled loops** (`crates/vector-store/src/lib.rs`)
   - Replaced iterator-based sum with 4-element unrolled loop for auto-vectorization (SSE/AVX)
   - Separate accumulator (`sum_hi` / `sum_lo`) to reduce dependency chain

4. **BM25 inverted index: persistent on-disk** (`crates/vector-store/src/lib.rs`)
   - Added `bm25_index.jsonl` — persisted after first build, loaded O(1) next time
   - Stats JSON on line 1, postings per term on subsequent lines

5. **Tokenize: borrowed str** (`crates/vector-store/src/lib.rs`)
   - Tokenization uses `&str` for postings key — fewer heap allocations

6. **Vector store: DashMap dependency** (`crates/vector-store/Cargo.toml`)
   - Added `dashmap = "6.1"` for lock-free concurrent HashMap access

---

## Benchmark Results (cargo bench --package rust-rag-core)

### Search Latency

| Benchmark | Time | Confidence Interval | Change vs baseline |
|-----------|------|---------------------|-------------------|
| search_latency_p50_top1 | 642.2 ns | [641.2 ns, 642.5 ns] | **-9.7%** (improved) |
| search_latency_p95_top10 | 635.2 ns | [634.7 ns, 635.8 ns] | **-12.7%** (improved) |
| search_latency_p99_top50 | 641.8 ns | [641.1 ns, 642.5 ns] | **-11.0%** (improved) |

### Memory Usage

| Benchmark | Time | Confidence Interval | Change vs baseline |
|-----------|------|---------------------|-------------------|
| memory_usage_10_files/search | 647.3 ns | [643.7 ns, 651.2 ns] | **-9.0%** (improved) |
| memory_usage_30_files/search | 644.6 ns | [642.7 ns, 648.0 ns] | **-10.3%** (improved) |

### Indexing Throughput

| Benchmark | Time | Confidence Interval | Change vs baseline |
|-----------|------|---------------------|-------------------|
| index_workspace_15_files/index | 839.5 µs | [833.6 µs, 845.4 µs] | No change (stable) |
| index_workspace_60_files/index | 1.55 ms | [1.49 ms, 1.60 ms] | No change (stable) |

---

## Test Results

All workspace tests pass: **84 passed, 0 failed**.

```
cargo test --workspace
  rust-rag-error        : ok.   2 passed
  rust-rag-config       : ok.  12 passed
  rust-rag-indexer      : ok.   2 passed
  rust-rag-core         : ok.  14 passed (semantic cache) + 59 integration tests
  rust-rag-state        : ok.   2 passed
  rust-rag-vector-store : ok.   7 passed
  rust-rag-llm          : ok.  14 passed
  rust-rag-callergraph  : ok.   2 passed
  rust-rag-server       : (compiled)
  rust-rag-tui          : (compiled)
  rust-rag-cli          : (compiled)
```

---

## Environment

- **Rustc**: `rustc 1.85.0` (from workspace `rust-version = "1.85"`)
- **OS**: Linux (x86_64)
- **Node.js**: 24.x (used by build scripts)
- **Dependencies**: rayon 1.10, dashmap 6.1, tree-sitter 0.25

---

## How to Compare Future Results

To run the same benchmarks and compare:

```bash
# Search latency + memory benchmarks
cargo bench --package rust-rag-core

# Indexing throughput benchmarks
cargo bench --package rust-rag-core --bench indexing_bench

# Memory-only benchmark
cargo bench --package rust-rag-core --bench memory_bench
```

To verify correctness:

```bash
cargo test --workspace
```

Paste the new output below this section in a `---` delimited block for comparison.
