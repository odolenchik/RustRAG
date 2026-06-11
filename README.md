# RustRAG v0.7.14

**Local RAG for Rust codebases — full offline embeddings, hybrid BM25+vector search, MCP server.**

A self-hosted Retrieval-Augmented Generation tool built specifically for analyzing Rust Cargo workspaces. Index any workspace using AST-level semantic chunking (tree-sitter), embed with a local ONNX model, and query via CLI, interactive TUI, HTTP API, or MCP protocol — entirely offline once models are downloaded.

## Features

- **AST-aware indexing** — tree-sitter-rust parses source files into semantic chunks (functions, impl blocks, unsafe regions, traits, modules, structs, enums, macros) instead of naive fixed-size text splits
- **Hybrid search (BM25 + vector)** — full inverted index with standard BM25 scoring combined with cosine similarity via configurable alpha blending (~0.7 recommended)
- **Fully local embeddings** — fastembed ONNX runtime runs `bge-small-en-v1.5` locally; no external API calls needed for embedding computation
- **Embedding cache with model versioning** — JSONL-based persistent cache includes a model_id marker; automatically invalidated when the embedding model changes (different weights or dimensions), preventing stale results
- **Automatic model download** — if no local model is found, `rust-rag` auto-downloads `bge-small-en-v1.5` (~127 MB) from HuggingFace to `~/.cache/huggingface/hub/`; supports both flat and standard HF cache layouts
- **Configurable chunk overlap** — adjacent AST-extracted chunks within a file get overlapping context lines to preserve cross-boundary semantics (function calls, macro invocations spanning chunk edges)
- **Call graph analysis** — AST-based call edge extraction using `ra_ap_syntax` (rust-analyzer's syntax crate); parses each chunk to find CallExpr nodes and extract callee names
- **MCP server** — Model Context Protocol stdio server exposing `rag_search` and `rag_query` tools for AI coding agents (Claude Desktop, Cursor, Windsurf, etc.)
- **Interactive TUI** — ratatui-based terminal interface with scrollable results, real-time streaming LLM answers, and keyboard navigation
- **HTTP API + CORS** — axum server with `/search`, `/query`, `/query/stream`, and `/status` endpoints; SSE streaming support; cross-origin support for browser clients
- **Incremental indexing** — SHA-256-based file change detection with O(1) comparison (no redundant full-file scans); only re-indexes changed/new/deleted files. Stored in `.rustrag/index_state.json` for persistence across invocations.
- **Symbol search** — `rust-rag symbol <name>` finds symbols by name across the index, showing kind (Function/ImplBlock/etc.), file path and line number.
- **Configurable via TOML** — `.rustrag.toml` at workspace root controls embedding model path, LLM endpoint/model, top_k, chunk overlap

## Architecture

Five independent crates in a Cargo workspace:

| Crate | Purpose |
|-------|---------|
| `rust-rag-core` | Core engine: indexing (tree-sitter), embedding (fastembed/ONNX), vector store (JSONL + BM25), retrieval, call graph analysis, incremental state management, config |
| `rust-rag-cli` | CLI binary (`rust-rag`) with subcommands for index/ask/chat/reindex/info/clean/symbol |
| `rust-rag-server` | HTTP API server (axum) and MCP stdio server; exposes search, query, and status endpoints |
| `rust-rag-llm` | LLM client abstraction supporting OpenAI-compatible / Ollama backends with SSE streaming support |
| `rust-rag-tui` | Interactive terminal UI built on ratatui + crossterm with scrollable results and answer pane |

## Quick Start

### Prerequisites

- **Rust 1.85+** (MSRV: edition 2021)
- A running LLM server at the configured endpoint (e.g., Ollama on `localhost:11434`, or llama.cpp with `/chat/completions`)
- ~127 MB for embedding model (`bge-small-en-v1.5` ONNX)

### Installation

```bash
cargo build --release
# Produces: target/release/rust-rag (CLI/TUI) and target/release/rust-rag-serve (HTTP/MCP server)
```

### Configure

Copy the example config to your workspace root:

```bash
cp .rustrag.toml.example .rustrag.toml
```

Edit `.rustrag.toml` for your embedding model path, LLM endpoint, and other settings. Environment variables take precedence where applicable (`RUSRAG_MODEL_PATH`, `LLAMA_ENDPOINT`, `LLAMA_MODEL`).

### Index a workspace

```bash
./target/release/rust-rag index /path/to/cargo/workspace

# Force full re-index (ignores incremental detection)
./target/release/rust-rag index --force /path/to/cargo/workspace
```

This runs the full pipeline: AST extraction → embedding → vector store creation with caching. Subsequent runs only process changed/new/deleted files.

### Ask a question

```bash
# Single query (returns LLM answer)
./target/release/rust-rag ask "How does the call graph work?" -p /workspace/path

# JSON output for scripting
./target/release/rust-rag ask --json "How does the call graph work?" -p /workspace/path

# Interactive TUI chat session
./target/release/rust-rag chat -p /workspace
```

### Start HTTP API server

```bash
# Defaults to port 8090; set workspace via env var or CWD
./target/release/rust-rag-serve serve --port 8090

# MCP stdio server (for AI coding agents)
RUSRAG_WORKSPACE=/workspace/path ./target/release/rust-rag-serve mcp
```

## CLI Reference

| Command | Description | Usage |
|---------|-------------|-------|
| `index <path>` | Full indexing pipeline: AST extraction → ONNX embedding → vector store creation | `rust-rag index /workspace` |
| `reindex <path>` | Remove old `.rustrag/` directory, then run full indexing pipeline | `rust-rag reindex /workspace` |
| `info [-p path]` | Show metadata: total indexed chunks and unique files list | `rust-rag info -p /workspace` |
| `clean [-p path]` | Remove `.rustrag/` directory entirely | `rust-rag clean -p /workspace` |
| `ask <query> [-p path] [--stream] [--json]` | Ask a question; returns LLM answer with cited source locations. Use --stream for incremental streaming output, --json for structured JSON output suitable for scripting | `rust-rag ask "Where is config loaded?" --json` |
| `chat [-p path]` | Interactive TUI chat session with scrollable results and LLM answers (supports live streaming) | `rust-rag chat -p /workspace/path` |
| `symbol <name> [-p path] [--json]` | Search for a symbol by name in the indexed workspace, showing kind, file path and line number. Use --json for structured output | `rust-rag symbol "search_symbol" --json` |
| `info [-p path] [--json]` | Show metadata: total indexed chunks and unique files list. Use --json for structured output | `rust-rag info --json` |

## Server API

### HTTP Endpoints

**GET `/status`** — Workspace metadata (total chunks, index path)

**POST `/search`** — Hybrid semantic search
```json
// Request
{"query": "embedding model initialization", "top_k": 5}

// Response
{
  "results": [
    {
      "id": "chunk_path/file.rs_42",
      "file_path": "...",
      "line_start": 42,
      "line_end": 50,
      "module_name": "init_embedding_model",
      "symbol_kind": "Function",
      "text": "...",
      "score": 0.87
    }
  ]
}
```

**POST `/query`** — Full RAG with LLM answer and citations
```json
// Request
{"question": "How does the embedding cache work?"}

// Response
{
  "answer": "The EmbedCache stores...",
  "citations": [
    { "file_path": "...", "line_start": 30, "line_end": 45, "text": "..." }
  ]
}
```

**GET `/query/stream`** — Streamed LLM answer via Server-Sent Events (SSE). Returns incremental text chunks with `text/event-stream` content type. Query params: `question`, `top_k`.

```bash
curl -H 'Accept: text/event-stream' "http://localhost:8090/query/stream?question=How+does+embedding+work"
# SSE stream of LLM response tokens
```


### MCP Protocol

Implements **MCP stdio transport** (protocol version `2024-11-05`) with two tools:

| Tool | Description | Arguments |
|------|-------------|-----------|
| `rag_search` | Search for code chunks by semantic similarity. Returns raw snippets with BM25+vector scores, file paths, line numbers, and metadata | `query` (string, max 4096 chars) — search query; `top_k` (integer, 1–100, default 5) — number of results to return |
| `rag_query` | Full RAG pipeline: retrieves relevant chunks via hybrid search, builds context, and queries the LLM for an answer with source citations | `question` (string, max 4096 chars) — question about the indexed codebase |

The MCP server exposes itself over stdio: send JSON-RPC 2.0 requests on stdin and read responses from stdout. Supports `initialize`, `notifications/initialized`, `tools/list`, and `tools/call` methods.

## Model Auto-Download

When no local model is found, `rust-rag download` (or the first search/query command) automatically downloads `bge-small-en-v1.5` from HuggingFace:

```bash
# Manual download to a specific directory
./target/release/rust-rag download ~/.cache/huggingface/hub/

# Or just run any command — it will prompt and auto-download
./target/release/rust-rag index /path/to/workspace
```

The model is saved in the standard HuggingFace cache layout. You can also set `RUSRAG_MODEL_PATH` to point to a custom directory containing `model.onnx`, `tokenizer.json`, etc.

## Configuration

All settings in `.rustrag.toml` at workspace root:

```toml
[embedding]
# Directory containing model.onnx, tokenizer.json, config.json, etc.
model_path = "./Download"

# Adjacent lines to include before/after each chunk (0 = exact AST boundaries)
chunk_overlap = 3

[llm]
# OpenAI-compatible /chat/completions endpoint (supports Ollama too)
endpoint = "http://localhost:8080"
model = "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf"
top_k = 5
```

**Environment variable overrides** (highest priority):

| Variable | Applies To | Default |
|----------|-----------|---------|
| `RUSRAG_MODEL_PATH` | Embedding model path (above config file) | — |
| `LLAMA_ENDPOINT` | LLM HTTP endpoint | from config or default |
| `LLAMA_MODEL` | LLM model name | from config or default |
| `RUSRAG_WORKSPACE` | Server workspace root | current directory |

## TUI Keyboard Shortcuts

| Key | Action |
|-----|--------|
| Any printable char | Append to query input |
| Backspace | Remove last character |
| Enter | Run search with current query |
| `q` / `Q` | Quit application |
| Esc | Clear results, return to idle state |
| Up/Down arrows | Navigate result list one item at a time |
| PageUp / BackTab | Scroll up by 5 items |
| PageDown | Scroll down by 5 items |
| Home | Jump to first result / top of answer pane |
| End | Jump to last result / bottom of answer pane |

## Testing

```bash
cargo test --package rust-rag-core
# Runs 35 tests covering: indexing, incremental state management, vector store roundtrip,
# cosine similarity, hybrid search alpha blending, BM25 scoring, filters (symbol kind + file extension),
# document removal, edge cases
```

## How It Works

1. **Indexing** — `walkdir` walks workspace members; tree-sitter-rust parses each `.rs` file into an AST; semantic nodes are extracted as chunks with metadata (file path, line range, module name, symbol kind). SHA-256 hashes tracked in `.rustrag/index_state.json`.
2. **Incremental indexing** — on subsequent runs, file hashes are compared against stored state via O(1) lookup; only new/changed/deleted files trigger re-parsing and embedding. Removed files' documents are deleted from the index atomically.
3. **Chunk overlap** — after extraction, adjacent chunks within the same file get context lines from neighbors using an efficient byte-offset-to-line mapping (O(log N) binary search); each file is read only once regardless of chunk count.
4. **Embedding** — each chunk text is embedded via fastembed's ONNX runtime; results are cached in JSONL with model_id versioning for automatic invalidation on model change. Batch embedding processes all chunks in a single ONNX inference call instead of N individual calls.
5. **Vector store** — embeddings + metadata stored as JSONL with an in-memory BM25 inverted index built lazily at query time; documents are cached with mtime-based invalidation to avoid re-parsing on every search.
6. **Retrieval** — hybrid search combines cosine similarity (vector) and BM25 text scoring via alpha-weighted blend; each document's vector similarity is computed once and reused for both ranking and result display. Filters by symbol kind or file extension applied post-ranking.
7. **LLM answer** — retrieved context is assembled with citations and sent to the configured LLM endpoint; supports both full-response and SSE streaming modes.

## License

MIT
