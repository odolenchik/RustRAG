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
