use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub llm: LlmConfig,
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
}

fn default_top_k() -> usize {
    5
}

impl Config {
    /// Load config from `.rustrag.toml` in the given directory (workspace root).
    pub fn load(workspace_root: &Path) -> Result<Self> {
        let config_path = workspace_root.join(".rustrag.toml");
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load config from the current directory or any ancestor.
    pub fn find() -> Result<Self> {
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
}
