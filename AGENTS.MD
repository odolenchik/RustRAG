# RustRAG Agent Guide

Reply in the same language as the user.

## Working Principles

- Treat code, not documentation, as the source of truth.
- Before making changes, read the relevant crate and understand its boundaries.
- Keep changes focused on the task; do not slip in unrelated refactors.
- Prefer idiomatic Rust — `cargo clippy` should be happy with your code.

## Workspace Map

Five independent crates in a Cargo workspace (see `Cargo.toml`):

| Crate | Purpose | Public API | Entry point |
|-------|---------|------------|-------------|
| `crates/core` | Core engine: indexing (tree-sitter), embedding (fastembed/ONNX), vector store (JSONL + BM25), semantic LLM answer cache, retrieval, call graph analysis, config, state management | `rust_rag_core::*` | lib.rs |
| `crates/cli` | CLI binary (`rust-rag`) with subcommands via clap — index/reindex/info/clean/ask/chat/symbol/stats/download | `rust_rag_cli::` | main.rs (binary) |
| `crates/server` | HTTP API server (axum) and MCP stdio server; exposes search, query, status endpoints + MCP protocol tools | `rust_rag_server::*` | lib.rs → build_router / mcp module |
| `crates/llm` | LLM client abstraction supporting OpenAI-compatible / Ollama backends with SSE streaming support | `rust_rag_llm::*` | ollama_client.rs, validation.rs |
| `crates/tui` | Interactive terminal UI built on ratatui + crossterm — app state, transcript rendering, editor input, theme system | `rust_rag_tui::run()` | lib.rs → app.rs, ui/ |

**Dependency direction:** cli → core, server → core+llm, tui → core. No crate depends on `server`. `llm` is independent of all others except through config.

## Key Files and Artifacts

- `.rustrag.toml` — TOML config (embedding model path, LLM endpoint/model, top_k, chunk_overlap, semantic cache settings). Env vars (`RUSRAG_MODEL_PATH`, `LLAMA_ENDPOINT`, `LLAMA_MODEL`, `RUSRAG_SEMANTIC_CACHE_ENABLED`, `RUSRAG_SEMANTIC_CACHE_TTL`) override config.
- `.rustrag/` — index directory: vector store JSONL, BM25 inverted index, incremental state (`index_state.json`). Created by `rust-rag index`.
- `crates/core/src/indexer.rs` — AST extraction via tree-sitter-rust, semantic chunking with overlap.
- `crates/core/src/vector_store.rs` — hybrid search (BM25 + cosine similarity), document CRUD, filtering.
- `crates/core/src/embedding.rs` — ONNX embedding via fastembed, JSONL caching with model_id versioning.
- `crates/core/src/semantic_cache.rs` — Semantic LLM answer cache: exact + cosine-similarity lookup, TTL expiry, JSONL persistence (`semantic_cache.jsonl`).
- `crates/server/src/lib.rs` — HTTP handlers (`/status`, `/search`, `/query`, `/query/stream`), AppState, auth, rate limiting, semantic cache integration.
- `crates/server/src/mcp.rs` — MCP stdio protocol implementation (JSON-RPC 2.0, tools/list, tools/call).
- `crates/tui/src/app.rs` — ratatui app state, query input, result navigation.
- `crates/tui/src/ui/transcript.rs` — transcript/answer rendering in TUI.

## Rust Conventions

- **Error handling**: Use `anyhow::Result<T>` for external-facing functions with `.context()` or `.with_context()`. Use `thiserror` for crate-specific error types (`#[derive(thiserror::Error)]`).
- **Naming**: snake_case for functions/variables, CamelCase for structs/enums/traits, UPPER_SNAKE_CASE for constants. Crate names use kebab-case in Cargo.toml but snake_case in code (`rust_rag_core`).
- **Async**: `tokio` runtime. Use `#[tokio::main]` only at binary entry points (`main.rs`, `bin.rs`). Use `spawn_blocking` for CPU-bound work (embeddings, ONNX inference).
- **Modules**: One public type per module file where practical. Re-export via lib.rs with `pub use`. Keep internal modules private unless cross-crate.
- **Immutability**: Prefer `Arc<T>` over `Mutex<T>` when shared read-only state. Use `&str` not `String` for function parameters that don't need ownership.
- **Lifetimes**: Minimize where possible — prefer owned types with clear lifetimes (`Arc`, `Box`) over borrowing in complex structs like `AppState`.

## Workflow Requirements

**Build:**
```bash
cargo build --release    # produces target/release/rust-rag and rust-rag-serve
```

**Test:**
```bash
cargo test               # all workspace tests
cargo test --package rust-rag-core   # core engine only
```

**Lint:**
```bash
cargo clippy             # must pass with no warnings
rustc --version          # MSRV: 1.85, edition 2021
```

**Run locally (dev):**
```bash
cargo run -- index /path/to/workspace
cargo run -- ask "query" -p /workspace/path
cargo run --bin rust-rag-serve serve   # HTTP/MCP server on port default
```

## Testing Rules

- When adding a feature, add tests for it. Test the public API of each crate.
- Core engine tests live in `crates/core/tests/` — integration-style roundtrip tests (index → store → search).
- CLI tests go in `crates/cli/tests/integration_tests.rs`.
- Server handler tests belong in `crates/server/tests/`.
- TUI component tests belong in `crates/tui/src/ui/mod.rs` or separate test modules within the crate.

## Configuration Rules

- Config resolution priority: env var > `.rustrag.toml` > hardcoded defaults.
- All config paths must be resolved lazily — never at compile time.
- The LLM endpoint default is `http://localhost:8080`; model name default is `"default-rag-model"`.
- Embedding model auto-downloads to `~/.cache/huggingface/hub/` if not found locally.
