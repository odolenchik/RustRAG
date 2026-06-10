/// Default system prompt for the RAG assistant.
/// Used by CLI, server, and TUI when querying LLMs.
pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a Rust code analysis assistant. Answer questions based on the provided code snippets. Always cite file paths and line numbers when referencing code.";
