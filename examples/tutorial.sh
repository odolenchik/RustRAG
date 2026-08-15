#!/bin/bash
# Tutorial: Using RustRag with an Example Workspace

echo "=== RustRag Tutorial ==="
echo

# Step 1: Index the workspace
echo "Step 1: Indexing the example workspace..."
./target/release/rust-rag index examples/example-workspace
echo

# Step 2: Show index information
echo "Step 2: Showing index information..."
./target/release/rust-rag info -p examples/example-workspace
echo

# Step 3: Search for symbols
echo "Step 3: Searching for symbols..."
echo "Finding 'User' struct:"
./target/release/rust-rag symbol User -p examples/example-workspace
echo
echo "Finding 'Post' struct:"
./target/release/rust-rag symbol Post -p examples/example-workspace
echo
echo "Finding 'factorial' function:"
./target/release/rust-rag symbol factorial -p examples/example-workspace
echo

# Step 4: Show chunking statistics
echo "Step 4: Showing chunking statistics..."
./target/release/rust-rag stats -p examples/example-workspace
echo

# Step 5: Demonstrate MCP server (instructions)
echo "Step 5: To use with AI coding agents via MCP:"
echo "  1. In another terminal, run:"
echo "     ./target/release/rust-rag-serve mcp examples/example-workspace"
echo "  2. Configure your AI agent to use this MCP server"
echo "  3. The agent will have access to:"
echo "     - rag_search: Search for code snippets"
echo "     - rag_workspace_info: Get workspace structure"
echo "     - rag_file_read: Read files in the workspace"
echo

echo "=== Tutorial Complete ==="
echo "You can now:"
echo "  - Ask questions: ./target/release/rust-rag ask \"Your question\" -p examples/example-workspace"
echo "  - Start chat: ./target/release/rust-rag chat -p examples/example-workspace"
echo "  - Reindex after changes: ./target/release/rust-rag reindex examples/example-workspace"
