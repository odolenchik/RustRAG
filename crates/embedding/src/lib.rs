//! ONNX-based embedding inference with local caching.
//!
//! Loads a BGE model from disk (or downloads it from HuggingFace), provides
//! single/batch text-to-embedding APIs, and caches embeddings in JSONL to
//! avoid redundant ONNX inference during re-indexing.

#![warn(missing_docs)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use rust_rag_error::RagCoreError;

use fastembed::{
    read_file_to_bytes, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};

/// Initialise the embedding model. If not found in any location, attempts auto-download from HuggingFace.
#[tracing::instrument(level = "info", skip())]
fn init_embedder() -> Result<TextEmbedding, RagCoreError> {
    let dir = model_dir();

    if let Ok(em) = try_init_embedder(&dir) {
        return Ok(em);
    }

    let home = std::env::var("HOME").ok();
    let hf_target = match home.as_ref() {
        Some(h) => PathBuf::from(h).join(".cache/huggingface/hub"),
        None => return Err(RagCoreError::Embedding(
            "Cannot determine HOME to download model. Please download manually:\n  rust-rag download ~/.cache/huggingface/hub/\nOr set RUSRAG_MODEL_PATH.".to_string(),
            Box::new(std::io::Error::other("HOME not set")),
        )),
    };

    println!("Model not found, downloading from HuggingFace...");
    if let Err(e) = download_model(&hf_target) {
        return Err(RagCoreError::Embedding(
            format!("Failed to auto-download model: {}\n\nPlease try manually:\n  rust-rag download ~/.cache/huggingface/hub/", e),
            Box::new(std::io::Error::other(e)),
        ));
    }

    println!("Model downloaded. Trying again...");
    try_init_embedder(&hf_target).map_err(|e| {
        RagCoreError::Embedding(
            "Failed to load embedding model after download".to_string(),
            Box::new(std::io::Error::other(format!("{:?}", e))),
        )
    })
}

/// Standard HuggingFace cache directory for downloaded models.
fn hf_cache_model_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let hub = PathBuf::from(&home).join(".cache/huggingface/hub");

    if !hub.exists() {
        return None;
    }

    if hub.join("model.onnx").exists() {
        return Some(hub);
    }

    for entry in std::fs::read_dir(&hub).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        if let Ok(snapshots) = std::fs::read_dir(&path) {
            for snapshot in snapshots.flatten() {
                let snap_path = snapshot.path();
                if !snap_path.is_dir() {
                    continue;
                }

                if let Ok(resolved) = snap_path.canonicalize() {
                    if resolved.join("onnx").join("model.onnx").exists() {
                        return Some(resolved.join("onnx"));
                    }
                }
            }
        }

        if path.join("model.onnx").exists() {
            return Some(path);
        }
    }

    None
}

/// Resolve the directory containing model files.
fn model_dir() -> PathBuf {
    if let Ok(path) = std::env::var("RUSRAG_MODEL_PATH") {
        return PathBuf::from(path);
    }

    if let Ok(config) = rust_rag_config::Config::find() {
        if let Some(ref path_str) = config.embedding.model_path {
            return PathBuf::from(path_str);
        }
    }

    if let Some(hf_dir) = hf_cache_model_dir() {
        return hf_dir;
    }

    PathBuf::from("/usr/local/share/rustrag/models")
}

/// Initialize the embedding model from local ONNX + tokenizer files.
fn try_init_embedder(model_dir: &Path) -> Result<TextEmbedding, RagCoreError> {
    let onnx_bytes = std::fs::read(model_dir.join("model.onnx")).map_err(|e| {
        RagCoreError::Embedding(
            "Failed to read model.onnx".to_string(),
            Box::new(std::io::Error::other(e)),
        )
    })?;

    let tokenizer_files = TokenizerFiles {
        tokenizer_file: read_file_to_bytes(&model_dir.join("tokenizer.json")).map_err(|e| {
            RagCoreError::Embedding(
                "Failed to read tokenizer.json".to_string(),
                Box::new(std::io::Error::other(e)),
            )
        })?,
        config_file: read_file_to_bytes(&model_dir.join("config.json")).map_err(|e| {
            RagCoreError::Embedding(
                "Failed to read config.json".to_string(),
                Box::new(std::io::Error::other(e)),
            )
        })?,
        special_tokens_map_file: read_file_to_bytes(&model_dir.join("special_tokens_map.json"))
            .map_err(|e| {
                RagCoreError::Embedding(
                    "Failed to read special_tokens_map.json".to_string(),
                    Box::new(std::io::Error::other(e)),
                )
            })?,
        tokenizer_config_file: read_file_to_bytes(&model_dir.join("tokenizer_config.json"))
            .map_err(|e| {
                RagCoreError::Embedding(
                    "Failed to read tokenizer_config.json".to_string(),
                    Box::new(std::io::Error::other(e)),
                )
            })?,
    };

    let user_model =
        UserDefinedEmbeddingModel::new(onnx_bytes, tokenizer_files).with_pooling(Pooling::Cls);

    TextEmbedding::try_new_from_user_defined(user_model, InitOptionsUserDefined::default()).map_err(
        |e| {
            RagCoreError::Embedding(
                "Failed to initialize user-defined embedding model".to_string(),
                Box::new(std::io::Error::other(e)),
            )
        },
    )
}

static EMBEDDER: OnceLock<Result<TextEmbedding, RagCoreError>> = OnceLock::new();

fn get_embedder() -> Result<&'static TextEmbedding, RagCoreError> {
    EMBEDDER.get_or_init(init_embedder).as_ref().map_err(|e| {
        RagCoreError::Embedding(
            "Embedding model initialization failed".to_string(),
            Box::new(std::io::Error::other(format!("{:?}", e))),
        )
    })
}

/// Embed a single text chunk into a vector using the local embedding model.
#[tracing::instrument(level = "debug", skip_all, fields(text_len = text.len()))]
pub fn embed(text: &str) -> Result<Vec<f32>, RagCoreError> {
    let result = get_embedder()?
        .embed(vec![text.to_string()], None)
        .map_err(|e| {
            RagCoreError::Embedding(
                "Failed to compute embedding".to_string(),
                Box::new(std::io::Error::other(e)),
            )
        })?;

    Ok(result.into_iter().flatten().collect())
}

/// Embed multiple texts in a single ONNX inference call. Returns one vector per input text.
#[tracing::instrument(level = "debug", skip_all, fields(text_count = texts.len()))]
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Vec<f32>>, RagCoreError> {
    let strings: Vec<String> = texts.iter().map(|t| t.to_string()).collect();
    let results = get_embedder()?.embed(strings, None).map_err(|e| {
        RagCoreError::Embedding(
            "Failed to compute batch embedding".to_string(),
            Box::new(std::io::Error::other(e)),
        )
    })?;

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
#[derive(Debug)]
pub struct EmbedCache {
    path: PathBuf,
}

impl EmbedCache {
    /// Current model identifier used to invalidate stale caches.
    fn model_id() -> String {
        let dir = model_dir();
        let onnx_path = dir.join("model.onnx");

        if !onnx_path.exists() {
            return "no-model-found".to_string();
        }

        let mut hasher = Sha256::new();
        if let Ok(bytes) = std::fs::read(&onnx_path) {
            hasher.update(&bytes);
        }
        format!("{:x}", hasher.finalize())
    }

    /// Open the embed cache for a workspace's `.rustrag` directory.
    pub fn open(rustrag_dir: &Path) -> Self {
        Self {
            path: rustrag_dir.join("embed_cache.jsonl"),
        }
    }

    /// Look up cached embeddings for texts, returning `Vec<Option<Vec<f32>>>`.
    pub fn lookup(&self, texts: &[&str]) -> Result<Vec<Option<Vec<f32>>>, RagCoreError> {
        let cache = self.read_cache()?;
        Ok(texts
            .iter()
            .map(|t| cache.get(&hash_text(t)).cloned())
            .collect())
    }

    fn read_cache(&self) -> Result<HashMap<String, Vec<f32>>, RagCoreError> {
        let mut cache = HashMap::new();
        if !self.path.exists() {
            return Ok(cache);
        }

        let current_model_id = Self::model_id();
        let content = std::fs::read_to_string(&self.path)?;

        let mut lines = content.lines().peekable();
        if let Some(first_line) = lines.peek() {
            if first_line.starts_with("#model_id=") {
                let stored_model_id = first_line.trim_start_matches("#model_id=").to_string();
                if stored_model_id != current_model_id {
                    eprintln!(
                        "[rustrag] Embedding cache invalidated: model_id mismatch ({} != {})",
                        stored_model_id, current_model_id
                    );
                    return Ok(cache);
                }
                lines.next();
            }
        }

        for line in lines.filter(|l| !l.trim().is_empty()) {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
                if let (Some(hash), Some(embed)) = (entry["hash"].as_str(), entry.get("embedding"))
                {
                    let vec: Vec<f32> = embed
                        .as_array()
                        .unwrap_or(&Vec::new())
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
    pub fn write_back(
        &self,
        texts: &[&str],
        embeddings: &[Vec<f32>],
        hit_count: &mut usize,
    ) -> Result<(), RagCoreError> {
        if texts.is_empty() || embeddings.is_empty() {
            return Ok(());
        }

        let mut cache = self.read_cache()?;
        for (text, embedding) in texts.iter().zip(embeddings.iter()) {
            if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(hash_text(text)) {
                e.insert(embedding.clone());
            } else {
                *hit_count += 1;
            }
        }

        let mut content = String::with_capacity(4096);
        writeln!(content, "#model_id={}", Self::model_id())?;
        for (k, v) in &cache {
            let line = serde_json::json!({ "hash": k, "embedding": v });
            writeln!(content, "{}", serde_json::to_string(&line).unwrap())?;
        }

        let tmp_path = self.path.with_extension("jsonl.tmp");
        std::fs::write(&tmp_path, &content)?;
        #[cfg(unix)]
        if let Err(e) = std::fs::set_permissions(&self.path, PermissionsExt::from_mode(0o600)) {
            tracing::warn!(
                "[warning] Failed to set file permissions on {}: {}",
                self.path.display(),
                e
            );
        }
        let _ = std::fs::rename(&tmp_path, &self.path);

        Ok(())
    }

    /// Clear the embed cache file.
    pub fn clear(&self) -> Result<(), RagCoreError> {
        if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

/// Get the path where embedding models are stored (kept for backward compatibility).
pub fn model_cache_dir() -> Result<PathBuf, RagCoreError> {
    let base = dirs::cache_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp/.rustrag/cache"));

    let dir = base.join("rustrag");
    std::fs::create_dir_all(&dir).map_err(|e| {
        RagCoreError::Embedding(
            "Failed to create model cache directory".to_string(),
            Box::new(std::io::Error::other(e)),
        )
    })?;
    Ok(dir)
}

/// Download the bge-small-en-v1.5 model files from HuggingFace and save them to a target directory.
pub fn download_model(target: &Path) -> Result<(), RagCoreError> {
    let repo = "BAAI/bge-small-en-v1.5";

    let files: Vec<(&str, &str)> = vec![
        ("config.json", "config.json"),
        ("special_tokens_map.json", "special_tokens_map.json"),
        ("tokenizer.json", "tokenizer.json"),
        ("tokenizer_config.json", "tokenizer_config.json"),
        ("onnx/model.onnx", "model.onnx"),
    ];

    let client = reqwest::blocking::Client::builder()
        .user_agent("RustRag/0.7.14")
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    let expected_checksums: HashMap<String, String> = parse_expected_checksums();

    std::fs::create_dir_all(target)?;

    for &(remote_path, local_name) in &files {
        println!("Downloading {}...", remote_path);
        let url = format!("https://huggingface.co/{}/resolve/{}", repo, remote_path);

        let response = client.get(&url).send().map_err(|e| {
            RagCoreError::Embedding(
                format!("HTTP request failed: {}", e),
                Box::new(std::io::Error::other(e)),
            )
        })?;
        if !response.status().is_success() {
            return Err(RagCoreError::Embedding(
                format!(
                    "Failed to download {}: HTTP {}",
                    remote_path,
                    response.status()
                ),
                Box::new(std::io::Error::other("HTTP error")),
            ));
        }

        if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
            let ct = content_type.to_str().unwrap_or("");
            if local_name == "model.onnx"
                && !ct.contains("application/octet-stream")
                && !ct.contains("x-application")
            {
                println!(
                    "  [warning] model.onnx returned Content-Type: {} (expected application/octet-stream)",
                    ct
                );
            } else if (local_name.ends_with(".json")) && !ct.contains("application/json") {
                println!(
                    "  [warning] {} returned Content-Type: {} (expected application/json)",
                    local_name, ct
                );
            }
        }

        let bytes = response.bytes()?;

        if let Some(expected_hash) = expected_checksums.get(local_name) {
            let digest = sha256_hex(&bytes);
            if digest[..] != expected_hash[..] {
                return Err(RagCoreError::Embedding(
                    format!(
                        "Checksum mismatch for {}: expected {}, got {}",
                        local_name, expected_hash, digest
                    ),
                    Box::new(std::io::Error::other("checksum")),
                ));
            } else {
                println!("  ✓ Checksum OK");
            }
        }

        std::fs::write(target.join(local_name), &bytes)?;
    }

    println!("Model files saved to: {}", target.display());
    Ok(())
}

fn parse_expected_checksums() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Ok(val) = std::env::var("RUSRAG_MODEL_CHECKSUMS") {
        for line in val.lines().filter(|l| !l.trim().is_empty()) {
            if let Some(rest) = line.strip_prefix("sha256:") {
                if let Some((filename, hash)) = rest.split_once('=') {
                    if !filename.is_empty() && !hash.is_empty() {
                        map.insert(filename.trim().to_string(), hash.trim().to_lowercase());
                    }
                }
            }
        }
    }
    map
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}
