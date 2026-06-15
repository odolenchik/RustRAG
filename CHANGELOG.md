# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

_Note: v0.7.8 through 0.7.14 are documented below._

## [0.7.15] - 2026-06-15

### Added
- **Semantic LLM answer cache** — caches answers in `semantic_cache.jsonl`; on repeated or semantically similar questions (cosine similarity ≥ 0.85), returns the cached answer instantly without an LLM call. TTL-based expiry with configurable duration. Opt-in via config or env vars (`RUSRAG_SEMANTIC_CACHE_ENABLED`, `RUSRAG_SEMANTIC_CACHE_TTL`). New module: `crates/core/src/semantic_cache.rs`.
- **Sliding-window rate limiter** — per-client IP rate limiting on HTTP API endpoints (excluding `/status`), replacing the broken Semaphore that never replenished. Implemented as a custom ~50-line struct with no external dependencies.
- **LLM request timeouts** — 2-minute timeout for full responses and 5-minute timeout for streaming sessions, preventing hung LLM calls from consuming resources indefinitely. Per-chunk read timeout (60s) on the shared HTTP client.
- **Context size limit** — maximum assembled context sent to the LLM (default: 12 KB); preserves complete chunk blocks without partial splits. Configurable via `RUSRAG_MAX_CONTEXT_SIZE` env var or `[llm].max_context_size` in config file.
- **Bearer token authentication** for HTTP API endpoints (all except `/status`) via `RUSRAG_API_KEY` environment variable.
- New tests: 10 new server handler tests (rate limiter, auth, context trimming), 7 new semantic cache tests (exact match, similarity lookup, TTL expiry, persistence, disabled mode).

### Changed
- `/query` and `/query/stream` endpoints now check the semantic cache before performing search + LLM call. On a successful response, the answer is written back to the cache for future lookups.
- Config module (`crates/core/src/config.rs`) adds `SemanticCacheConfig` with `enabled: bool` and `ttl_secs: u64` fields under `[semantic_cache]` TOML section.
- `AppState` now includes a shared `Arc<SemanticCache>` instance, created from config/env or disabled by default.

### Fixed
- Rate limiter no longer blocks all requests after the first — the previous Semaphore-based approach never replenished permits, effectively disabling any request beyond budget limit

## [0.7.14] - 2026-06-11

### Added
- **CI/CD pipeline** (`.github/workflows/ci.yml`) with automated formatting, build, tests, clippy (`-D warnings`), and security audit on every push/PR
- **HTTP API Bearer token authentication** via `RUSRAG_API_KEY` environment variable — all endpoints except `/status` require `Authorization: Bearer <key>` header when key is configured
- **CONTRIBUTING.md** — developer guide with build, test, linting, and PR submission instructions
- **SECURITY.md** — security policy with vulnerability reporting process and supported versions table

### Changed
- Embedding cache `write_back()` now uses atomic file replacement (temp file + rename) instead of direct overwrite to prevent corruption on crash
- LLM endpoint URL validation enhanced: now resolves hostnames to IPs and checks all resolved addresses for DNS rebinding protection; blocks localhost-style hostnames (`.localhost`, `.local`)

### Fixed
- SSRF vulnerability: LLM endpoints now validated before client creation, blocking private IPs and cloud metadata URLs with full DNS rebinding protection via hostname-to-IP resolution
- Changelog duplicate entries between versions 0.7.6 and 0.7.8 removed for clarity

## [0.7.13] - 2026-06-11

### Added
- **Atomic file replacement** for embedding cache (`embed_cache.jsonl`) using temp file + rename pattern to prevent corruption on crash
- New unit tests for call graph module: `test_callgraph_parse_call_exprs_finds_function_calls`, `test_build_creates_nodes_for_all_chunks`, `test_ignores_non_function_chunks`
- New indexer tests covering all symbol kinds (Function, ImplBlock, UnsafeRegion, TraitImpl, Struct, Enum, Module, Macro), empty file handling, and deduplication

### Fixed
- LLM endpoint URL validation now includes DNS rebinding protection by resolving hostnames to IPs before checking addresses

## [0.7.12] - 2026-06-11

### Added
- **HTTP API Bearer token authentication** via `RUSRAG_API_KEY` environment variable — all endpoints except `/status` require `Authorization: Bearer <key>` header when key is configured
- **CONTRIBUTING.md** — developer guide with build, test, linting, and PR submission instructions
- **SECURITY.md** — security policy with vulnerability reporting process and supported versions table

### Fixed
- SSRF vulnerability: LLM endpoints now validated before client creation, blocking private IPs and cloud metadata URLs; added DNS rebinding protection via hostname resolution

## [0.7.11] - 2026-06-11

### Added
- **CI/CD pipeline** (`.github/workflows/ci.yml`) with automated formatting, build, tests, clippy (`-D warnings`), and security audit on every push/PR

## [0.7.8] - 2026-06-09

### Added
- **Incremental indexing** — `index_workspace()` now detects changed/new/deleted files by SHA-256 hash comparison; only re-indexes modified files instead of rebuilding the entire index. Saved state stored in `.rustrag/index_state.json`. (~3× faster on subsequent runs for large workspaces)
- **`--force` flag** to `index` subcommand — force a full re-index even if no file changes detected
- **`symbol <name>` command** — search indexed workspace for symbols by name, returns module path, symbol kind (Function/ImplBlock/etc.), file path and line number. Matches against module_name and chunk text with case-insensitive substring matching.
- **`State::compare()`** — computes new/changed/removed files between saved state and current filesystem; also tracks removed chunk IDs for stale document cleanup
- **VectorStore `remove_documents()` and `list_document_ids()`** — atomic replace of index.jsonl to remove stale documents when files are deleted or modified

### Fixed
- Chunk ID format consistency across `compare()`/`update_files()` (prefix `"chunk_{path}_"`) — was causing stale chunk detection failures in incremental state tracking
- `changed_files` variable properly returned from compare() instead of being lost

## [0.7.6] - 2026-06-08

### Added
- AST-aware indexing using tree-sitter-rust: semantic chunking by functions, impl blocks, unsafe regions, traits, modules, structs, enums, macros
- Hybrid search combining BM25 (text) and cosine similarity (vector embeddings) with configurable alpha blending (~0.7 recommended)
- Fully local ONNX-based embeddings via fastembed (`bge-small-en-v1.5`) — no external API calls needed
- Embedding cache (JSONL-based persistent cache) preventing redundant ONNX inference across re-indexes
- Configurable chunk overlap for preserving cross-boundary semantics between adjacent AST chunks
- Call graph analysis with AST-based call edge extraction using `ra_ap_syntax` (rust-analyzer's syntax crate)
- MCP stdio server exposing `rag_search` and `rag_query` tools for AI coding agents (Claude Desktop, Cursor, Windsurf, etc.)
- Interactive TUI built on ratatui + crossterm with scrollable results, LLM answer pane, keyboard navigation
- HTTP API server (axum) with `/search`, `/query`, `/status` endpoints and CORS support
- TOML-based configuration via `.rustrag.toml` controlling embedding model path, LLM endpoint/model, top_k, chunk overlap

### Architecture
Five independent crates: `rust-rag-core`, `rust-rag-cli`, `rust-rag-server`, `rust-rag-llm`, `rust-rag-tui`
