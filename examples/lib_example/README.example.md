# Library Example

This is a simple Rust library demonstrating structs, functions, and unit tests.

## Using with RustRag

From the RustRag root directory:

```bash
# Index this workspace
./target/release/rust-rag index examples/lib_example

# Ask questions
./target/release/rust-rag ask "What does the AppConfig struct contain?" -p examples/lib_example
./target/release/rust-rag ask "How does the factorial function work?" -p examples/lib_example
./target/release/rust-rag ask "What is a palindrome?" -p examples/lib_example

# Search for symbols
./target/release/rust-rag symbol AppConfig -p examples/lib_example
./target/release/rust-rag symbol factorial -p examples/lib_example
./target/release/rust-rag symbol is_palindrome -p examples/lib_example

# Start interactive chat
./target/release/rust-rag chat -p examples/lib_example
