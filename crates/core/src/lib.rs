//! rust-rag-core — orchestration layer that re-exports and wires together the sub-crates.

#![warn(missing_docs)]

// Re-export error types (exposed as submodule for backward compat)
/// Error types from `rust_rag_error`.
pub mod error {
    pub use rust_rag_error::*;
}

pub use rust_rag_error::{ErrorKind, RagCoreError};
pub use rust_rag_error::wrap_core_result;

// Re-export indexer types
/// Indexer types from `rust_rag_indexer`.
pub mod indexer {
    pub use rust_rag_indexer::*;
}

// Re-export vector-store types
/// Vector store types from `rust_rag_vector_store`.
pub mod vector_store {
    pub use rust_rag_vector_store::*;
}

// Re-export embedding types
/// Embedding types from `rust_rag_embedding`.
pub mod embedding {
    pub use rust_rag_embedding::*;
}

// Re-export state types
/// State management types from `rust_rag_state`.
pub mod state {
    pub use rust_rag_state::*;
}

// Re-export callgraph types
/// Call graph construction types from `rust_rag_callgraph`.
pub mod callgraph {
    pub use rust_rag_callgraph::*;
}

// Config module (TOML loader)
/// Configuration loading types from `rust_rag_config`.
pub mod config {
    pub use rust_rag_config::*;
}

/// Semantic cache — kept inline in core because it bridges embedding + vector_store.
pub mod semantic_cache;

/// Retrieval orchestration layer that ties the sub-crates together.
pub mod retrieval;

/// Tracing initialisation for rust-rag.
pub mod tracing;

/// Default system prompt for the RAG assistant.
pub mod constants {
    /// Default system prompt for the RAG assistant.
    pub const DEFAULT_SYSTEM_PROMPT: &str = "You are a Rust code analysis assistant. Answer questions based on the provided code snippets. Always cite file paths and line numbers when referencing code.";
}

/// Evaluation metrics (MRR, chunk diagnostics). Kept inline since it depends on vector_store types.
pub mod eval;
