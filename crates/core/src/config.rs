use crate::error::RagCoreError;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    /// Semantic cache configuration for caching LLM answers.
    #[serde(default)]
    pub semantic_cache: SemanticCacheConfig,
}

#[derive(Debug, Deserialize, Default)]
pub struct EmbeddingConfig {
    pub model_path: Option<String>,

    /// Number of adjacent lines to include before and after each AST-extracted chunk.
    /// Helps preserve context at chunk boundaries where a function call or macro invocation
    /// might be split between chunks. Set to 0 for no overlap (exact AST node boundaries).
    #[serde(default)]
    pub chunk_overlap: usize,
}

#[derive(Debug, Deserialize, Default)]
pub struct LlmConfig {
    pub endpoint: Option<String>,
    pub model: Option<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Maximum size in bytes of the assembled context sent to the LLM.
    /// Set to 0 or omit for the default (12 KB).
    pub max_context_size: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SemanticCacheConfig {
    /// Enable semantic caching of LLM answers (default: false).
    #[serde(default)]
    pub enabled: bool,
    /// Time-to-live in seconds for cached entries (default: 3600 = 1 hour).
    #[serde(default = "default_cache_ttl")]
    pub ttl_secs: u64,
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
    /// Returns a default (empty) config when no file is found so that callers
    /// can continue without it.
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

    pub fn embedding_config(&self) -> &EmbeddingConfig {
        &self.embedding
    }

    pub fn llm_config(&self) -> &LlmConfig {
        &self.llm
    }

    pub fn semantic_cache_config(&self) -> &SemanticCacheConfig {
        &self.semantic_cache
    }
}
