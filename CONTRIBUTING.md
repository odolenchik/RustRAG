# Contributing to RustRag

Thank you for your interest in contributing! This document covers everything you need to get started.

## Prerequisites

- **Rust**: `>=1.85` (MSRV, set in `Cargo.toml`)
- **pnpm**: Not required for RustRag itself, but used by parent project if developing alongside
- **ONNX Runtime**: Automatically downloaded on first run (~127 MB) or pre-download via `cargo run -- download`

## Project Structure

```
RustRag/
├── crates/                    # Workspace crates
│   ├── core/                  # rust-rag-core — Core engine (indexing, embeddings, search, semantic cache)
│   │   └── tests/mod.rs       # 59+ test functions
│   ├── cli/                   # rust-rag-cli — CLI binary (7 subcommands)
│   ├── server/                # rust-rag-server — HTTP API (axum) + MCP protocol, rate limiting, auth, semantic cache integration
│   ├── llm/                   # rust-rag-llm — LLM client abstraction
│   └── tui/                   # rust-rag-tui — Interactive terminal UI
├── Download/                  # ONNX model files (gitignored)
├── .rustrag.toml              # Active configuration
└── .rustrag.toml.example      # Example config template
```

## Getting Started

### 1. Clone and build

```bash
git clone https://github.com/MoonshotAI/kimi-code.git
cd RustRag   # If this is a separate repo, use its URL instead
cargo build --workspace
```

### 2. Run tests

```bash
# All core crate tests (59+ tests: indexing, incremental state, vector store, cosine similarity, hybrid search, semantic cache)
cargo test --package rust-rag-core

# LLM validation tests (SSRF endpoint URL validation)
cargo test --package rust-rag-llm

# Server handler tests (rate limiter, auth, context trimming)
cargo test --package rust-rag-server

# All workspace tests (108+ total)
cargo test --workspace
```

### 3. Format and lint

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
```

## Development Workflow

1. **Create a feature branch**: `git checkout -b feat/your-feature-name`
2. **Make changes** following the guidelines below
3. **Write tests** for new functionality (see [Testing Guidelines](#testing-guidelines))
4. **Run all checks** before committing:

```bash
cargo fmt --all && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

5. **Submit a PR** with a clear description of changes

## Coding Guidelines

### General Rules

- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- Use `thiserror` for custom error types, not manual `impl Error`
- Prefer `anyhow::Result<T>` at binary/CLI boundaries
- Use `log::debug!()` / `log::info!()` for logging — no hardcoded print statements
- Add `///` documentation comments to all public items (functions, structs, traits)

### Module Responsibilities

- **core** owns: indexing, embeddings, vector storage, retrieval pipeline
- **cli** owns: command parsing and user-facing commands
- **server** owns: HTTP endpoints, MCP protocol server
- **llm** owns: LLM client abstraction and endpoint validation
- **tui** owns: terminal UI components and rendering

### Error Handling

```rust
// Good — derive thiserror
#[derive(Debug, thiserror::Error)]
pub enum IndexingError {
    #[error("failed to parse file: {0}")]
    ParseError(#[from] tree_sitter::Error),
    #[error("file not found: {path}")]
    FileNotFound { path: String },
}

// Good — anyhow at binary boundary
fn run() -> anyhow::Result<()> { ... }
```

## Testing Guidelines

### Test Placement

- **Unit tests**: In the same file as the module (`#[cfg(test)]` mod) for small helpers
- **Integration tests**: In `crates/<crate-name>/tests/mod.rs` or separate test files under `tests/`
- **Server integration tests**: Use `wiremock` to mock LLM backend, test HTTP endpoints in `crates/server/tests/`

### Test Requirements

- Minimum 30% code coverage for all new and modified modules
- Every public function must have at least one test case
- Edge cases must be tested: empty inputs, boundary values, error paths

### Example Test Structure

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_indexer_parses_function() {
        let code = r#"fn hello() {}"#;
        // ... assertions
    }

    #[test]
    fn test_indexer_handles_empty_file() {
        let code = "";
        // ... assertions for edge case
    }
}
```

## Adding a New Crate Dependency

1. Add to the appropriate crate's `Cargo.toml`
2. Run `cargo update --package <crate>` (not `cargo update` — keeps lockfile stable)
3. Ensure the dependency is compatible with MSRV 1.85
4. Document why the dependency is needed in a code comment if non-obvious

## Submitting a PR

### PR Template

```markdown
## What does this PR do?
Brief description of what was changed and why.

## Type of change
- [ ] Bug fix
- [ ] New feature
- [ ] Breaking change
- [ ] Documentation update
- [ ] Tests / CI

## How has this been tested?
Describe how you verified the changes (manual testing, test commands, etc.)

## Checklist
- [ ] `cargo fmt --all` passes
- [ ] `cargo clippy --workspace -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] New tests added for new functionality
- [ ] CHANGELOG.md updated (under [Unreleased])
```

### PR Title Conventions

Use conventional commit style:
- `feat:` — new feature
- `fix:` — bug fix
- `docs:` — documentation only
- `test:` — test changes
- `chore:` — maintenance, dependencies, CI
- `refactor:` — code restructure (no behavior change)

## Releasing

1. Update version in `Cargo.toml` (all crates share the same version)
2. Add release notes to `CHANGELOG.md` under a new `[<version>]` section
3. Run full CI: `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace`
4. Commit and push, create release PR

## Questions?

Open an issue on GitHub or ask in the project discussions. All contributions are welcome!
