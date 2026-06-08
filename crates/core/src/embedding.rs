use anyhow::{anyhow, Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use fastembed::{TextEmbedding, UserDefinedEmbeddingModel, InitOptionsUserDefined, TokenizerFiles, Pooling, read_file_to_bytes};

/// Standard HuggingFace cache directory for downloaded models.
fn hf_cache_model_dir() -> Option<PathBuf> {
    // HF stores downloaded models under ~/.cache/huggingface/hub/
    let home = std::env::var("HOME").ok()?;
    let hub = PathBuf::from(&home).join(".cache/huggingface/hub");

    if !hub.exists() { return None; }

    // Look for model.onnx in the hub root (flat layout from manual copy)
    if hub.join("model.onnx").exists() {
        return Some(hub);
    }

    // Check subdirectories: models--Xenova--bge-small-en-v1.5/snapshots/*/onnx/
    for entry in std::fs::read_dir(&hub).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }

        // Check snapshots/*/onnx/model.onnx (standard HF layout)
        if let Ok(snapshots) = std::fs::read_dir(&path) {
            for snapshot in snapshots.flatten() {
                let snap_path = snapshot.path();
                if !snap_path.is_dir() { continue; }

                // Canonicalize to resolve symlinks and .. components
                if let Ok(resolved) = snap_path.canonicalize() {
                    if resolved.join("onnx").join("model.onnx").exists() {
                        return Some(resolved.join("onnx"));
                    }
                }
            }
        }

        // Also check direct model.onnx in repo directory (alternative layout)
        if path.join("model.onnx").exists() {
            return Some(path);
        }
    }

    None
}

/// Resolve the directory containing model files.
/// Priority: env var > config file > HF cache > project-local Download/ > error.
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

    // 3) Standard HuggingFace cache (~/.cache/huggingface/hub/) — works for any user, any machine
    if let Some(hf_dir) = hf_cache_model_dir() {
        return hf_dir;
    }

    // 4) Project-local Download/ (for development / bundled builds)
    let start = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in start.ancestors() {
        if ancestor.join("Download").join("model.onnx").exists() {
            return ancestor.join("Download");
        }
    }

    // 5) Absolute fallback — will fail gracefully on init with a clear error message
    PathBuf::from("/usr/local/share/rustrag/models")
}

/// Initialize the embedding model from local ONNX + tokenizer files.
fn try_init_embedder(model_dir: &Path) -> Result<TextEmbedding> {
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

/// Initialize the embedding model. If not found in any location, attempts auto-download from HuggingFace.
fn init_embedder() -> Result<TextEmbedding> {
    let dir = model_dir();

    if let Ok(em) = try_init_embedder(&dir) {
        return Ok(em);
    }

    // Model not found — attempt auto-download to HF cache
    let home = std::env::var("HOME").ok();
    let hf_target = match home.as_ref() {
        Some(h) => PathBuf::from(h).join(".cache/huggingface/hub"),
        None => anyhow::bail!(
            "Cannot determine HOME to download model.\n\
             Please download manually:\n\n  rust-rag download ~/.cache/huggingface/hub/\n\
             \nOr set RUSRAG_MODEL_PATH."
        ),
    };

    println!("Model not found, downloading from HuggingFace...");
    if let Err(e) = download_model(&hf_target) {
        anyhow::bail!(
            "Failed to auto-download model: {e}\n\n\
             Please try manually:\n  rust-rag download ~/.cache/huggingface/hub/"
        );
    }

    println!("Model downloaded. Trying again...");
    try_init_embedder(&hf_target).context("Failed to load embedding model after download")
}

/// Lazy-initialized singleton embedder — loads ONNX model once on first use.
static EMBEDDER: LazyLock<TextEmbedding> = LazyLock::new(|| {
    init_embedder().expect("Embedding model initialization failed")
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
    /// Current model identifier used to invalidate stale caches.
    /// Changes when the underlying ONNX model changes (different weights, dimensions).
    fn model_id() -> String {
        // Use a simple hash of the embedding dimension as a proxy for "model identity".
        // Different models produce different vector sizes, so this catches most changes.
        // For a more robust solution, store a model version string in config.
        format!("bge-small-en-v1.5")
    }

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

  fn read_cache(&self) -> Result<std::collections::HashMap<String, Vec<f32>>> {
        let mut cache = std::collections::HashMap::new();
        if !self.path.exists() { return Ok(cache); }

        let current_model_id = Self::model_id();
        let content = std::fs::read_to_string(&self.path)?;

        // First line may be a model_id marker (starts with "#model_id=")
        let mut lines = content.lines().peekable();
        if let Some(first_line) = lines.peek() {
            if first_line.starts_with("#model_id=") {
                let stored_model_id = first_line.trim_start_matches("#model_id=").to_string();
                if stored_model_id != current_model_id {
                    // Model changed — cache is stale, return empty to force regeneration
                    eprintln!("[rustrag] Embedding cache invalidated: model_id mismatch ({} != {})", stored_model_id, current_model_id);
                    return Ok(cache);
                }
                lines.next(); // skip the marker line
            }
        }

        for line in lines.filter(|l| !l.trim().is_empty()) {
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

        // Write model_id marker as first line
        writeln!(writer, "#model_id={}", Self::model_id())?;

        for (k, v) in &cache {
            let line = serde_json::json!({ "hash": k, "embedding": v });
            writeln!(writer, "{}", serde_json::to_string(&line).unwrap())?;
        }
        writer.flush()?;
        Ok(())
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
