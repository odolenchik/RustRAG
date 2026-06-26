//! Typed errors for rust-rag.
//!
//! Library-level public APIs return `Result<T, RagCoreError>`. Callers can match on
//! variants to make decisions (e.g. retry on I/O, skip missing config). Binary crates
//! typically wrap these with `.map_err(anyhow::Error::from)` and add context for the user.

#![warn(missing_docs)]

use std::path::PathBuf;

/// All error types produced by the rust-rag library ecosystem.
#[derive(Debug, thiserror::Error)]
pub enum RagCoreError {
    /// The requested workspace does not contain a Cargo.toml at the given path.
    #[error("no Cargo.toml found at {0}")]
    MissingCargoToml(PathBuf),

    /// TOML configuration file parse / read failure.
    #[error("config error: {0}")]
    Config(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// A source file could not be read from disk.
    #[error("failed to read file '{0}': {1}")]
    FileRead(PathBuf, #[source] Box<dyn std::error::Error + Send + Sync>),

    /// Tree-sitter parsing failed for a Rust source file.
    #[error("tree-sitter parse error in '{0}': {1}")]
    ParseError(PathBuf, #[source] Box<dyn std::error::Error + Send + Sync>),

    /// An ONNX / embedding model read or initialisation failure.
    #[error("embedding model error: {0}")]
    Embedding(String, #[source] Box<dyn std::error::Error + Send + Sync>),

    /// A persistent vector-store operation failed (read, write, corrupt index).
    #[error("vector store error: {0}")]
    VectorStore(String, #[source] Box<dyn std::error::Error + Send + Sync>),

    /// The index contains a SymbolKind value that cannot be parsed.
    #[error("unknown symbol kind '{0}' in index (document id: {1}, file: {2})")]
    UnknownSymbolKind(String, String, String),

    /// State save / load failure on disk.
    #[error("state error: {0}")]
    State(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// Call graph construction failed.
    #[error("call graph error: {0}")]
    CallGraph(String, #[source] Box<dyn std::error::Error + Send + Sync>),

    /// The embedding model could not be found and download also failed.
    #[error("embedding model not found; attempted download from {url}: {cause}")]
    ModelNotFound {
        /// URL where the model was fetched from.
        url: String,
        /// Cause of the download failure.
        cause: Box<dyn std::error::Error + Send + Sync>,
    },

    /// A generic I/O error wrapping a `std::io::Error`.
    #[error("{0}")]
    Io(#[source] std::io::Error),

    /// An internal / unexpected error.
    #[error("internal error: {0}")]
    Internal(String, #[source] Box<dyn std::error::Error + Send + Sync>),
}

impl RagCoreError {
    /// Wrap a [`std::io::Error`] with context describing the operation.
    pub fn io<E: Into<std::io::Error>>(op: &str, err: E) -> Self {
        let err = err.into();
        if op.is_empty() {
            RagCoreError::Io(err)
        } else {
            RagCoreError::Io(std::io::ErrorKind::Other.into())
        }
    }

    /// Return the semantic category of this error for high-level matching.
    pub fn kind(&self) -> ErrorKind {
        match self {
            RagCoreError::MissingCargoToml(_) => ErrorKind::MissingCargoToml,
            RagCoreError::Config(..) => ErrorKind::Config,
            RagCoreError::FileRead(..) | RagCoreError::ParseError(..) => ErrorKind::Io,
            RagCoreError::Embedding(..) => ErrorKind::Embedding,
            RagCoreError::VectorStore(..) => ErrorKind::VectorStore,
            RagCoreError::UnknownSymbolKind(..) => ErrorKind::CorruptIndex,
            RagCoreError::State(..) => ErrorKind::Io,
            RagCoreError::CallGraph(..) | RagCoreError::Internal(..) => ErrorKind::Internal,
            RagCoreError::ModelNotFound { .. } => ErrorKind::ModelNotFound,
            RagCoreError::Io(_) => ErrorKind::Io,
        }
    }
}

/// High-level error categories useful for user-facing messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// No Cargo.toml was found in the workspace root.
    MissingCargoToml,
    /// TOML configuration file parse / read failure.
    Config,
    /// General I/O error wrapping a `std::io::Error`.
    Io,
    /// Embedding model read or initialisation failure.
    Embedding,
    /// Persistent vector-store operation failed.
    VectorStore,
    /// The index contains an unexpected SymbolKind value.
    CorruptIndex,
    /// An internal / unexpected error.
    Internal,
    /// The embedding model could not be found and download also failed.
    ModelNotFound,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::MissingCargoToml => write!(f, "missing Cargo.toml"),
            ErrorKind::Config => write!(f, "configuration error"),
            ErrorKind::Io => write!(f, "I/O error"),
            ErrorKind::Embedding => write!(f, "embedding model error"),
            ErrorKind::VectorStore => write!(f, "vector store error"),
            ErrorKind::CorruptIndex => write!(f, "corrupt index"),
            ErrorKind::Internal => write!(f, "internal error"),
            ErrorKind::ModelNotFound => write!(f, "embedding model not found"),
        }
    }
}

/// Helper to convert a `Result<T, RagCoreError>` into `anyhow::Result<T>`.
pub fn wrap_core_result<T>(result: Result<T, RagCoreError>) -> anyhow::Result<T> {
    result.map_err(|e| anyhow::anyhow!("[RagCore] {}", e))
}

/// Convenience extension: wrap a `std::io::Error` as the variant-specific error.
pub trait IoContext<T> {
    /// Wrap this `Result` by mapping its `io::Error` through the provided closure.
    fn wrapped(
        self,
        kind: impl FnOnce(std::io::Error) -> Box<dyn std::error::Error + Send + Sync>,
    ) -> Result<T, RagCoreError>;
}

impl<T> IoContext<T> for Result<T, std::io::Error> {
    fn wrapped(
        self,
        kind: impl FnOnce(std::io::Error) -> Box<dyn std::error::Error + Send + Sync>,
    ) -> Result<T, RagCoreError> {
        self.map_err(|e| (kind)(e).into())
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for RagCoreError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        let err_any = err.as_ref();
        let name = std::any::type_name_of_val(err_any);
        if name.contains("ParseIntError") || name.contains("toml::de") {
            RagCoreError::Config(err)
        } else if name.contains("io::Error") || name.contains("std::io") {
            RagCoreError::State(err)
        } else {
            RagCoreError::Internal(format!("unexpected error: {}", err), err)
        }
    }
}

impl From<std::io::Error> for RagCoreError {
    fn from(err: std::io::Error) -> Self {
        RagCoreError::Io(err)
    }
}

impl From<std::time::SystemTimeError> for RagCoreError {
    fn from(err: std::time::SystemTimeError) -> Self {
        RagCoreError::Io(std::io::Error::new(std::io::ErrorKind::Other, err))
    }
}

impl From<std::fmt::Error> for RagCoreError {
    fn from(err: std::fmt::Error) -> Self {
        RagCoreError::Io(std::io::Error::new(std::io::ErrorKind::Other, err))
    }
}

impl From<reqwest::Error> for RagCoreError {
    fn from(err: reqwest::Error) -> Self {
        RagCoreError::Embedding(
            format!("HTTP request failed: {}", err),
            Box::new(std::io::Error::other(err)),
        )
    }
}

impl From<serde_json::Error> for RagCoreError {
    fn from(err: serde_json::Error) -> Self {
        RagCoreError::VectorStore(
            format!("JSON parse error: {}", err),
            Box::new(std::io::Error::other(err)),
        )
    }
}
