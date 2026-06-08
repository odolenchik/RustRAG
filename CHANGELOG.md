# Changelog

## [Unreleased]

### Added
- **Incremental indexing** — `index_workspace()` now detects changed/new/deleted files by SHA-256 hash comparison; only re-indexes modified files instead of rebuilding the entire index. Saved state stored in `.rustrag/index_state.json`. (~3× faster on subsequent runs for large workspaces)
- **`--force` flag** to `index` subcommand — force a full re-index even if no file changes detected
- **`symbol <name>` command** — search indexed workspace for symbols by name, returns module path, symbol kind (Function/ImplBlock/etc.), file path and line number. Matches against module_name and chunk text with case-insensitive substring matching.
- **`State::compare()`** — computes new/changed/removed files between saved state and current filesystem; also tracks removed chunk IDs for stale document cleanup
- **VectorStore `remove_documents()` and `list_document_ids()`** — atomic replace of index.jsonl to remove stale documents when files are deleted or modified

### Changed
- Version bumped from 0.7.7 → 0.7.8
- `apply_overlap` and `extract_workspace_members` promoted from pub(crate) to pub for external crate access
- `IndexState.files`, `chunk_ids`, and `FileMetadata` made public for test access

### Fixed
- Chunk ID format consistency across `compare()`/`update_files()` (prefix `"chunk_{path}_"`) — was causing stale chunk detection failures in incremental state tracking
- `changed_files` variable properly returned from compare() instead of being lost

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
