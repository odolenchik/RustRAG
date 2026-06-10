# Changelog

## [Unreleased]

### Added
- **CI/CD pipeline** (`.github/workflows/ci.yml`) with automated formatting, build, tests, clippy (`-D warnings`), and security audit on every push/PR
- **LLM endpoint URL validation** (`validation.rs`) — blocks non-http(s) schemes, loopback, private IPs, cloud metadata URLs for SSRF protection
- **BM25 inverted index caching** (`Bm25CacheEntry`, `get_bm25_cache()`) in `VectorStore` — avoids rebuilding the BM25 index on every query, dramatically improving search latency
- **Rate limiting via Semaphore** (`AppState::rate_limiter`) — prevents thundering herd on `/search` and `/query` endpoints
- **Request body size limits** on server API via `tower-http` middleware
- **Path canonicalization** in CLI — `std::fs::canonicalize()` prevents path traversal vulnerabilities
- **Modular TUI components** (`ui/editor.rs`, `ui/transcript.rs`) — extracted from monolithic 508-line `app.rs` into independently testable renderers

### Changed
- Unified `OutputMode` enum and `run_ask_impl()` in CLI crate — replaced duplicated ask/ask_json/ask_stream/ask_stream_json implementations with a single unified handler delegating to mode-specific branches
- Moved `DEFAULT_SYSTEM_PROMPT` constant from 7+ duplicated locations into `crates/core/src/constants.rs` for single source of truth
- Removed redundant `cmd/*.rs` dispatcher files — CLI main.rs now calls crate functions directly

### Fixed
- SSRF vulnerability: LLM endpoints now validated before client creation, blocking private IPs and cloud metadata URLs
- TUI App struct split into component modules (editor, transcript) for better maintainability

## [0.7.8] - 2026-06-08

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

### Added
- **Incremental indexing** — `index_workspace()` now detects changed/new/deleted files by SHA-256 hash comparison; only re-indexes modified files instead of rebuilding the entire index. Saved state stored in `.rustrag/index_state.json`. (~3× faster on subsequent runs for large workspaces)
- **`--force` flag** to `index` subcommand — force a full re-index even if no file changes detected
- **`symbol <name>` command** — search indexed workspace for symbols by name, returns module path, symbol kind (Function/ImplBlock/etc.), file path and line number. Matches against module_name and chunk text with case-insensitive substring matching.
- **`State::compare()`** — computes new/changed/removed files between saved state and current filesystem; also tracks removed chunk IDs for stale document cleanup
- **VectorStore `remove_documents()` and `list_document_ids()`** — atomic replace of index.jsonl to remove stale documents when files are deleted or modified

### Fixed
- Chunk ID format consistency across `compare()`/`update_files()` (prefix `"chunk_{path}_"`) — was causing stale chunk detection failures in incremental state tracking
- `changed_files` variable properly returned from compare() instead of being lost

---

## [0.7.6] - Initial Release Notes

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
