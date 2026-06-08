# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Streaming LLM support across CLI, TUI, and server — `ChatBackend::complete_stream_chunks()` trait method with SSE-based implementation in OllamaClient
- `/query/stream` endpoint to HTTP server for real-time streamed responses via Server-Sent Events
- `--stream` flag on the CLI `ask` command for incremental LLM output
- Live TUI streaming — chat responses rendered incrementally via `TuiEvent::LlmChunk` events with partial answer display and blinking cursor indicator
- 6 new tests for chunk overlap behavior: boundary isolation, context lines, config scaling, single-chunk noop

### Changed
- `indexer::apply_overlap()` rewritten to correctly handle byte-to-line conversion for accurate context line reads between adjacent chunks
- Axum downgraded from v0.8 to v0.7 (for streaming compatibility)

### Added (dependencies)
- `async-stream`, `futures-util`, `bytes`, `http` crates

### Breaking Changes
- `ChatBackend::complete_streaming()` now has a new default implementation that calls `complete_stream_chunks()` — custom backends must implement the new method

## [0.7.7] - 2026-06-08

### Documentation
- Updated README with streaming docs and `/query/stream` endpoint documentation
- Fixed CLI example paths in README (chat command workspace path)
- Added CHANGELOG.md for tracking project changes

---

## [Unreleased] → [0.7.6] — Initial Release Notes

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
