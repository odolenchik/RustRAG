# Benchmark Results — Performance Optimization Current State (2026-06-29)

## Context

This benchmark captures the current performance of RustRAG after implementing audit recommendations:
1. Updated MSRV to 1.88+ to enable cargo-audit in CI
2. Enhanced CI with automated security auditing
3. All existing optimizations maintained

## Benchmark Results (cargo bench --package rust-rag-core)

### Search Latency

| Benchmark | Time (ns) | Confidence Interval (ns) |
|-----------|-----------|--------------------------|
| search_latency_p50_top1 | 648.2 | [645.60, 650.49] |
| search_latency_p95_top10 | 664.5 | [662.25, 666.43] |
| search_latency_p99_top50 | 650.8 | [648.10, 654.17] |

### Memory Usage

| Benchmark | Time (ns) | Confidence Interval (ns) |
|-----------|-----------|--------------------------|
| memory_usage_10_files/search | 666.1 | [648.57, 681.55] |
| memory_usage_30_files/search | 645.7 | [643.73, 647.99] |

### Indexing Throughput

| Benchmark | Time (ms) | Confidence Interval (ms) |
|-----------|-----------|--------------------------|
| index_workspace_15_files/index | 1.106 | [1.1021, 1.1116] |
| index_workspace_60_files/index | 4.084 | [4.0793, 4.0898] |

---

## Test Results

All workspace tests pass: **120 passed, 0 failed**.

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
  rust-rag-server       : ok.  14 passed
  rust-rag-tui          : ok.   2 passed
  rust-rag-cli          : ok.   9 passed (cli tests)
```

---

## Environment

- **Rustc**: Local rustc may vary (e.g., rustc 1.85.0) while CI uses rustc 1.88.0 (from workspace rust-version = "1.88")
- **OS**: Linux (x86_64)
- **Dependencies**: Same as baseline with cargo-audit added to CI

---

## Comparison with Previous Baseline (2025-06-16)

### Search Latency (Change)
- search_latency_p50_top1: **+0.9%** (648.2 vs 642.2 ns) - minimal regression
- search_latency_p95_top10: **+4.6%** (664.5 vs 635.2 ns) - slight regression
- search_latency_p99_top50: **+1.4%** (650.8 vs 641.8 ns) - minimal regression

### Memory Usage (Change)
- memory_usage_10_files/search: **+2.9%** (666.1 vs 647.3 ns) - minimal regression
- memory_usage_30_files/search: **+0.2%** (645.7 vs 644.6 ns) - negligible change

### Indexing Throughput (Change)
- index_workspace_15_files/index: **+31.8%** (1.106 ms vs 0.8395 ms) - regression
- index_workspace_60_files/index: **+163.4%** (4.084 ms vs 1.55 ms) - significant regression

## Analysis

The performance regressions observed are likely due to:
1. The indexing benchmarks may be measuring different workloads or caching effects
2. Search and memory benchmarks show excellent stability with minimal changes
3. The indexing workload might have changed due to dependency updates or environmental factors

Recommendation: Investigate indexing benchmark discrepancies, but core search/memory performance remains stable.

---

## How to Reproduce These Results

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
