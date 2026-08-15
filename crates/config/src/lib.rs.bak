//! TOML configuration loading for rust-rag.

#![warn(missing_docs)]

use serde::Deserialize;
use std::path::Path;

pub use rust_rag_error::RagCoreError;

/// Default TTL for semantic cache entries (1 hour in seconds).
pub const DEFAULT_TTL_SECS: u64 = 3600;

/// Top-level configuration loaded from `.rustrag.toml`.
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    /// Embedding model and chunking settings.
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// LLM endpoint and model name settings.
    #[serde(default)]
    pub llm: LlmConfig,
    /// Semantic cache configuration for caching LLM answers.
    #[serde(default)]
    pub semantic_cache: SemanticCacheConfig,
}

/// Configuration for the embedding model pipeline.
#[derive(Debug, Deserialize, Default)]
pub struct EmbeddingConfig {
    /// Path to the ONNX model directory (overrides env/config lookup).
    pub model_path: Option<String>,

    /// Number of adjacent lines to include before and after each AST-extracted chunk.
    #[serde(default)]
    pub chunk_overlap: usize,
}

impl EmbeddingConfig {
    /// Validate the configuration values.
    ///
    /// # Returns
    /// Ok(()) if valid, Err with descriptive message if invalid
    pub fn validate(&self) -> Result<(), String> {
        // chunk_overlap should be reasonable (let's say max 100 lines to prevent excessive memory usage)
        if self.chunk_overlap > 100 {
            return Err(format!(
                "chunk_overlap ({}) is too large (maximum 100)",
                self.chunk_overlap
            ));
        }
        Ok(())
    }
}

/// Configuration for LLM endpoint access.
#[derive(Debug, Deserialize, Default)]
pub struct LlmConfig {
    /// Base URL of the LLM API endpoint (e.g. OpenAI-compatible server).
    pub endpoint: Option<String>,
    /// Model name to use when querying the LLM endpoint.
    pub model: Option<String>,
    /// Maximum number of results to return from the LLM context.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Maximum size in bytes of the assembled context sent to the LLM.
    pub max_context_size: Option<usize>,
}

impl LlmConfig {
    /// Validate the configuration values.
    ///
    /// # Returns
    /// Ok(()) if valid, Err with descriptive message if invalid
    pub fn validate(&self) -> Result<(), String> {
        // top_k should be positive
        if self.top_k == 0 {
            return Err("top_k must be greater than 0".to_string());
        }
        // max_context_size if set should be positive
        if let Some(size) = self.max_context_size {
            if size == 0 {
                return Err("max_context_size must be greater than 0 if set".to_string());
            }
        }
        Ok(())
    }
}

/// Configuration for semantic caching of LLM answers.
#[derive(Debug, Deserialize)]
pub struct SemanticCacheConfig {
    /// Enable semantic caching of LLM answers (default: false).
    #[serde(default)]
    pub enabled: bool,
    /// Time-to-live in seconds for cached entries (default: 3600 = 1 hour).
    #[serde(default = "default_cache_ttl")]
    pub ttl_secs: u64,
}

impl SemanticCacheConfig {
    /// Validate the configuration values.
    ///
    /// # Returns
    /// Ok(()) if valid, Err with descriptive message if invalid
    pub fn validate(&self) -> Result<(), String> {
        // ttl_secs should be positive
        if self.ttl_secs == 0 {
            return Err("ttl_secs must be greater than 0".to_string());
        }
        Ok(())
    }
}

impl Default for SemanticCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            ttl_secs: default_cache_ttl(),
        }
    }
}

fn default_cache_ttl() -> u64 {
    3600
}

fn default_top_k() -> usize {
    5
}

impl Config {
    /// Load config from `.rustrag.toml` in the given directory (workspace root).
    pub fn load(workspace_root: &Path) -> Result<Self, RagCoreError> {
        let config_path = workspace_root.join(".rustrag.toml");
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path).map_err(|e| {
            RagCoreError::Config(Box::new(std::io::Error::other(format!(
                "reading '{}': {}",
                config_path.display(),
                e
            ))))
        })?;
        let config: Config = toml::from_str(&content).map_err(|e| {
            RagCoreError::Config(Box::new(std::io::Error::other(format!(
                "parsing '{}': {}",
                config_path.display(),
                e
            ))))
        })?;
        Ok(config)
    }

    /// Load config from the current directory or any ancestor.
    pub fn find() -> Result<Self, RagCoreError> {
        for dir in std::env::current_dir()?.ancestors() {
            let config_path = dir.join(".rustrag.toml");
            if config_path.exists() {
                return Self::load(dir);
            }
        }
        Ok(Self::default())
    }

    /// Return a reference to the embedding configuration.
    pub fn embedding_config(&self) -> &EmbeddingConfig {
        &self.embedding
    }

    /// Return a reference to the LLM configuration.
    pub fn llm_config(&self) -> &LlmConfig {
        &self.llm
    }

    /// Return a reference to the semantic cache configuration.
    pub fn semantic_cache_config(&self) -> &SemanticCacheConfig {
        &self.semantic_cache
    }

    /// Validate all configuration values.
    ///
    /// # Returns
    /// Ok(()) if valid, Err with descriptive message if invalid
    pub fn validate(&self) -> Result<(), String> {
        self.embedding.validate()?;
        self.llm.validate()?;
        self.semantic_cache.validate()?;
        Ok(())
    }
}
