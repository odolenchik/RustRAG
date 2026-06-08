use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fastembed::{TextEmbedding, UserDefinedEmbeddingModel, InitOptionsUserDefined, TokenizerFiles, Pooling, read_file_to_bytes};

/// Resolve the directory containing model files.
/// Prefers `RUSRAG_MODEL_PATH` env var, then config file, falls back to searching for Download/ by walking up from CARGO_MANIFEST_DIR.
fn model_dir() -> PathBuf {
    // 1) Explicit env var (highest priority)
    if let Ok(path) = std::env::var("RUSRAG_MODEL_PATH") {
        return PathBuf::from(path);
    }

    // 2) Config file via config::Config::find()
    if let Ok(config) = crate::config::Config::find() {
        if let Some(ref path_str) = config.embedding.model_path {
            return PathBuf::from(path_str);
        }
    }

    // 3) Walk up from CARGO_MANIFEST_DIR looking for a directory containing model.onnx
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in start.ancestors() {
        if ancestor.join("Download").join("model.onnx").exists() {
            return ancestor.join("Download");
        }
        // Also check common alternative locations
        if ancestor.join("model.onnx").exists() {
            return ancestor.to_path_buf();
        }
    }

    // 4) Final fallback: one level up from CARGO_MANIFEST_DIR (covers single-crate layout)
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CARGO_MANIFEST_DIR should have a parent")
        .join("Download")
}

/// Initialize the embedding model from local ONNX + tokenizer files.
fn init_embedder(model_dir: &Path) -> Result<TextEmbedding> {
    let onnx_bytes = std::fs::read(model_dir.join("model.onnx"))
        .context("Failed to read model.onnx")?;

    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read_file_to_bytes(&model_dir.join("tokenizer.json")).context("Failed to read tokenizer.json")?,
        config_file: read_file_to_bytes(&model_dir.join("config.json")).context("Failed to read config.json")?,
        special_tokens_map_file: read_file_to_bytes(&model_dir.join("special_tokens_map.json")).context("Failed to read special_tokens_map.json")?,
        tokenizer_config_file: read_file_to_bytes(&model_dir.join("tokenizer_config.json")).context("Failed to read tokenizer_config.json")?,
    };

    let user_model = UserDefinedEmbeddingModel::new(onnx_bytes, tokenizer_files)
        .with_pooling(Pooling::Cls);

    TextEmbedding::try_new_from_user_defined(user_model, InitOptionsUserDefined::default())
        .context("Failed to initialize user-defined embedding model")
}

/// Lazy-initialized singleton embedder — loads ONNX model once on first use.
static EMBEDDER: LazyLock<TextEmbedding> = LazyLock::new(|| {
    init_embedder(&model_dir()).expect("Embedding model initialization failed")
});

/// Embed a single text chunk into a vector using the local embedding model.
pub fn embed(text: &str) -> Result<Vec<f32>> {
    let result = EMBEDDER.embed(vec![text.to_string()], None /* batch_size */)
        .context("Failed to compute embedding")?;

    // Result is Vec<Embedding> — iterate over batches, flatten to Vec<f32>
    Ok(result.into_iter().flatten().collect())
}

/// Embed multiple texts in a single ONNX inference call. Returns one vector per input text.
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>> {
    let strings: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
    let results = EMBEDDER.embed(strings, None)
        .context("Failed to compute batch embedding")?;

    Ok(results.into_iter().collect())
}

/// Compute a simple hash of text for cache key generation.
fn hash_text(text: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Embedding cache — stores (text_hash -> embedding) pairs in a JSONL file.
/// Reduces redundant ONNX inference during re-indexing.
pub struct EmbedCache {
    path: PathBuf,
}

impl EmbedCache {
    /// Open the embed cache for a workspace's `.rustrag` directory.
    pub fn open(rustrag_dir: &Path) -> Self {
        Self { path: rustrag_dir.join("embed_cache.jsonl") }
    }

    /// Look up cached embeddings for texts, returning (Vec<Option<Vec<f32>>>).
    /// Returns None for uncached entries.
    pub fn lookup(&self, texts: &[&str]) -> Result<Vec<Option<Vec<f32>>>> {
        let cache = self.read_cache()?;
        Ok(texts.iter()
            .map(|t| cache.get(&hash_text(t)).cloned())
            .collect())
    }

    /// Write new embeddings for previously uncached entries. Returns the count of cached hits.
    pub fn write_back(&self, texts: &[&str], embeddings: &[Vec<f32>], hit_count: &mut usize) -> Result<()> {
        if texts.is_empty() || embeddings.is_empty() { return Ok(()); }

        let mut cache = self.read_cache()?;
        for (text, embedding) in texts.iter().zip(embeddings.iter()) {
            if cache.contains_key(&hash_text(text)) {
                *hit_count += 1;
            } else {
                cache.insert(hash_text(text), embedding.clone());
            }
        }

        let file = std::fs::OpenOptions::new().write(true).create(true).open(&self.path)?;
        let mut writer = std::io::BufWriter::new(file);
        for (k, v) in &cache {
            let line = serde_json::json!({ "hash": k, "embedding": v });
            writeln!(writer, "{}", serde_json::to_string(&line).unwrap())?;
        }
        writer.flush()?;
        Ok(())
    }

    fn read_cache(&self) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        let mut cache = std::collections::HashMap::new();
        if !self.path.exists() { return Ok(cache); }
        let content = std::fs::read_to_string(&self.path)?;
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                if let (Some(hash), Some(embed)) = (entry["hash"].as_str(), entry.get("embedding")) {
                    let vec: Vec<f32> = embed.as_array().unwrap_or(&Vec::new())
                        .iter()
                        .filter_map(|v| v.as_f64())
                        .map(|f| f as f32)
                        .collect();
                    cache.insert(hash.to_string(), vec);
                }
            }
        }
        Ok(cache)
    }

    /// Clear the embed cache file.
    pub fn clear(&self) -> Result<()> {
        if self.path.exists() { std::fs::remove_file(&self.path)?; }
        Ok(())
    }
}

/// Get the path where embedding models are stored (kept for backward compatibility).
pub fn model_cache_dir() -> Result<PathBuf> {
    let base = dirs::cache_dir()
        .or_else(|| dirs::data_local_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp/.rustrag/cache"));

    let dir = base.join("rustrag");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Download the bge-small-en-v1.5 model files from HuggingFace and save them to a target directory.
pub fn download_model(target: &Path) -> Result<()> {
    let repo = "BAAI/bge-small-en-v1.5";
    let revision = "main";

    // Files in the root of the repository + inside onnx/ subdirectory.
    // Our embedder expects these exact filenames in one directory.
    let files: Vec<(&str, &str)> = vec![
        ("config.json", "config.json"),
        ("special_tokens_map.json", "special_tokens_map.json"),
        ("tokenizer.json", "tokenizer.json"),
        ("tokenizer_config.json", "tokenizer_config.json"),
        ("onnx/model.onnx", "model.onnx"), // flatten onnx/ -> root
    ];

    let client = reqwest::blocking::Client::builder()
        .user_agent("RustRag/0.7.9")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    std::fs::create_dir_all(target)?;

    for (remote_path, local_name) in &files {
        println!("Downloading {}...", remote_path);
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            repo, revision, remote_path
        );

        let response = client.get(&url).send()?;
        if !response.status().is_success() {
            anyhow::bail!("Failed to download {}: HTTP {}", remote_path, response.status());
        }

        let bytes = response.bytes()?;
        println!(
            "  -> {} ({} bytes)",
            local_name,
            bytes.len()
                .try_into()
                .unwrap_or(std::u64::MAX)
        );
        std::fs::write(target.join(local_name), &bytes)?;
    }

    println!("Model files saved to: {}", target.display());
    Ok(())
}
