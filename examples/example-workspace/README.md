# Example Workspace for RustRag

This is a simple example Rust workspace designed to demonstrate RustRag's features.

## Structure

- `src/main.rs` - Application entry point with example structs and functions
- `src/lib.rs` - Library code with configuration, database pool, and utilities
- `benches/bench.rs` - Criterion benchmark examples

## Features Demonstrated

1. **Structs with derives** (`User`, `Post`, `AppConfig`) - Shows RustRag's ability to index struct definitions
2. **Functions with documentation** (`factorial`, `fetch_user`, `process_posts`) - Demonstrates docstring indexing
3. **Async functions** (`fetch_user`) - Shows async support
4. **Unsafe blocks** - Demonstrates unsafe region detection
5. **Modules** (`utils`) - Shows module indexing
6. **Tests** - Unit tests in both main file and lib.rs
7. **Benchmarks** - Criterion benchmarks in the benches directory

## Using with RustRag

To index this workspace with RustRag:

```bash
# From the RustRag root directory
./target/release/rust-rag index /path/to/examples/example-workspace

# Or if you're already in the example workspace directory
../../target/release/rust-rag index .
```

To ask questions about the codebase:

```bash
./target/release/rust-rag ask "What does the User struct contain?" -p .
./target/release/rust-rag ask "How does the factorial function work?" -p .
./target/release/rust-rag ask "What is the purpose of the DatabasePool struct?" -p .
```

To start the interactive chat:

```bash
./target/release/rust-rag chat -p .
```

## Expected Results

After indexing, you should be able to:
- Find the `User` and `Post` structs via symbol search
- Retrieve documentation for functions like `factorial`
- Search for concepts like "database connection pool"
- Use the MCP server with AI coding agents
