# Binary Example

This is a simple Rust binary demonstrating async HTTP request (mock) and data processing.

## Using with RustRag

From the RustRag root directory:

```bash
# Index this workspace
./target/release/rust-rag index examples/bin_example

# Ask questions
./target/release/rust-rag ask "What does the User struct contain?" -p examples/bin_example
./target/release/rust-rag ask "How does the process_posts function work?" -p examples/bin_example
./target/release/rust-rag ask "What is the purpose of the fetch_user function?" -p examples/bin_example

# Search for symbols
./target/release/rust-rag symbol User -p examples/bin_example
./target/release/rust-rag symbol fetch_user -p examples/bin_example
./target/release/rust-rag symbol process_posts -p examples/bin_example
./target/release/rust-rag symbol Post -p examples/bin_example

# Start interactive chat
./target/release/rust-rag chat -p examples/bin_example
