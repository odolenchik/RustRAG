# Workspace Example

This is a Rust workspace containing a library (`mylib`) and a binary (`mybin`) that depends on the library.

## Using with RustRag

From the RustRag root directory:

```bash
# Index this workspace
./target/release/rust-rag index examples/workspace_example

# Ask questions
./target/release/rust-rag ask "What does the AppConfig struct contain?" -p examples/workspace_example
./target/release/rust-rag ask "How does the fibonacci function work?" -p examples/workspace_example
./target/release/rust-rag ask "What does the User struct contain?" -p examples/workspace_example
./target/release/rust-rag ask "How does the fetch_user function work?" -p examples/workspace_example

# Search for symbols
./target/release/rust-rag symbol AppConfig -p examples/workspace_example
./target/release/rust-rag symbol fibonacci -p examples/workspace_example
./target/release/rust-rag symbol User -p examples/workspace_example
./target/release/rust-rag symbol fetch_user -p examples/workspace_example

# Start interactive chat
./target/release/rust-rag chat -p examples/workspace_example
