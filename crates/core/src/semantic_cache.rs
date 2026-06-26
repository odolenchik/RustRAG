#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::error::RagCoreError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};

/// Default TTL for cache entries (1 hour in seconds).
pub const DEFAULT_TTL_SECS: u64 = 3600;

    /// Compute L2 norm of a vector.
    fn compute_norm(v: &[f32]) -> f32 {
        let sum_sq: f32 = v.iter().map(|&x| x * x).sum();
        sum_sq.sqrt()
    }

/// A single semantic-cache entry: the stored question embedding, the original question,
/// the LLM answer, and an expiration timestamp (Unix epoch, seconds since 1970-01-01).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    /// SHA-256 hash of the original question text (for exact-match dedup).
    question_hash: String,
    /// The embedding vector of the cached question.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[serde(default)]
    embedding: Vec<f32>,
    /// Precomputed L2 norm of the embedding vector.
    #[serde(default)]
    embedding_norm: f32,
    /// The original question text (human-readable log / debug).
    question: String,
    /// The LLM's answer text.
    answer: String,
    /// Unix timestamp in seconds when this entry expires.
    expires_at: u64,
}

/// A semantic cache for LLM answers backed by an in-memory HashMap and a JSONL file on disk.
///
/// On lookup:
///   1. Hash the incoming question to check for exact-match hits.
///   2. If no exact hit, embed the question and find the most similar cached entry by cosine similarity.
///   3. Return the cached answer if similarity exceeds the threshold (0.85).
///
/// On write-back:
///   1. Compute embedding of the question via ONNX inference.
///   2. Store it alongside the answer with an expiration timestamp.
pub struct SemanticCache {
    /// In-memory cache keyed by a hash of the original question text.
    entries: std::sync::Mutex<HashMap<String, CacheEntry>>,
    /// Path to the persistent JSONL file backing the cache.
    path: PathBuf,
    /// TTL in seconds (how long an entry lives after being cached).
    ttl_secs: u64,
}

impl SemanticCache {
    /// Create a disabled semantic cache that always returns `None` on lookup and never writes back.
    pub fn disabled() -> Self {
        Self {
            entries: std::sync::Mutex::new(HashMap::new()),
            path: PathBuf::from(":disabled:"),
            ttl_secs: 0,
        }
    }

    /// Open or create a semantic cache for the given `.rustrag` directory.
    pub fn open(rustrag_dir: &Path, ttl_secs: Option<u64>) -> Self {
        let path = rustrag_dir.join("semantic_cache.jsonl");
        let ttl_secs = ttl_secs.unwrap_or(DEFAULT_TTL_SECS);

        // Load existing entries from disk into memory.
        let mut entries = HashMap::new();
        if let Ok(entries_map) = load_jsonl(&path, &ttl_secs) {
            entries = entries_map;
        }

        Self {
            path,
            entries: std::sync::Mutex::new(entries),
            ttl_secs,
        }
    }

    /// Look up a cached answer for the given question.
    ///
    /// Returns `Some(answer)` if an exact or semantically similar entry is found and not expired.
    /// Returns `None` otherwise (cache miss).
    pub fn lookup(&self, question: &str) -> Option<String> {
        // Quick bail for disabled cache.
        if self.path.as_os_str() == ":disabled:" {
            return None;
        }

        let question_hash = sha256_hex(question);
        let mut entries = self.entries.lock().unwrap();

        // 1. Exact match check first.
        if let Some(entry) = entries.get(&question_hash) {
            if !is_expired(entry.expires_at) {
                return Some(entry.answer.clone());
            }
            // Expired — evict and continue to semantic search below.
            entries.remove(&question_hash);
        }

        drop(entries);

        // 2. Semantic search: find the most similar non-expired entry.
        let embedding = match crate::embedding::embed(question) {
            Ok(v) => v,
            Err(_) => return None,
        };
        let query_norm = compute_norm(&embedding);
        let threshold: f32 = 0.85; // cosine similarity threshold for a cache hit
        let mut best_hash: Option<String> = None;
        let mut best_sim: f32 = -1.0;

        {
            let entries = self.entries.lock().unwrap();
            for entry in entries.values() {
                if is_expired(entry.expires_at) || entry.embedding.is_empty() {
                    continue;
                }
                if query_norm == 0.0 || entry.embedding_norm == 0.0 {
                    // similarity will be zero; skip unless we want to consider zero similarity
                    // but threshold is 0.85, so zero won't match; we can continue to save computation
                    continue;
                }
                let dot: f32 = embedding.iter().zip(entry.embedding.iter()).map(|(a, b)| a * b).sum();
                let sim = dot / (query_norm * entry.embedding_norm);
                if sim > threshold && sim > best_sim {
                    best_sim = sim;
                    best_hash = Some(entry.question_hash.clone());
                }
            }
        }

        match best_hash {
            Some(ref hash) => {
                let entries = self.entries.lock().unwrap();
                if let Some(entry) = entries.get(hash) {
                    if !is_expired(entry.expires_at) {
                        return Some(entry.answer.clone());
                    }
                }
                None
            }
            None => None,
        }
    }

    /// Store a question/answer pair in the cache.
    pub fn write_back(&self, question: &str, answer: &str) -> Result<(), RagCoreError> {
        // Disabled cache silently ignores writes.
        if self.path.as_os_str() == ":disabled:" {
            return Ok(());
        }

        let embedding = crate::embedding::embed(question)?;
        let embedding_norm = compute_norm(&embedding);
        let now_secs = current_unix_timestamp();
        let question_hash = sha256_hex(question);

        // Acquire the lock, insert, then persist.
        {
            let mut entries = self.entries.lock().unwrap();
            let entry = CacheEntry {
                question_hash: question_hash.clone(),
                embedding,
                embedding_norm,
                question: question.to_string(),
                answer: answer.to_string(),
                expires_at: now_secs + self.ttl_secs,
            };
            entries.insert(question_hash, entry);

            // Persist the full map to disk.
            persist_jsonl(&self.path, &entries)?;
        }

        Ok(())
    }

    /// Clear the semantic cache (both in-memory and on-disk).
    pub fn clear(&self) -> Result<(), RagCoreError> {
        let mut entries = self.entries.lock().unwrap();
        entries.clear();
        if self.path.exists() && self.path.as_os_str() != ":disabled:" {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

/// Check if a cache entry has expired.
fn is_expired(expires_at: u64) -> bool {
    current_unix_timestamp() >= expires_at
}

/// Get the current Unix timestamp in seconds (UTC).
fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Compute SHA-256 hex digest of text for cache key generation.
fn sha256_hex(data: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Load cache entries from a JSONL file into an in-memory HashMap.
fn load_jsonl(path: &Path, _ttl_secs: &u64) -> Result<HashMap<String, CacheEntry>, RagCoreError> {
    let mut map = HashMap::new();
    if !path.exists() {
        return Ok(map);
    }

    let content = std::fs::read_to_string(path)?;
    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        // Skip model_id marker lines (for compatibility with embed_cache.jsonl format).
        if line.starts_with('#') {
            continue;
        }

        if let Ok(entry) = serde_json::from_str::<CacheEntry>(line) {
            map.insert(entry.question_hash.clone(), entry);
        }
    }
    Ok(map)
}

/// Persist the in-memory cache to a JSONL file atomically.
fn persist_jsonl(
    path: &Path,
    entries_map: &HashMap<String, CacheEntry>,
) -> Result<(), RagCoreError> {
    let mut content = String::with_capacity(entries_map.len().max(1) * 256);
    for entry in entries_map.values() {
        let line = serde_json::to_string(&entry).unwrap_or_default();
        writeln!(content, "{}", line)?;
    }

    // Atomic write: temp file + rename (POSIX atomic).
    let tmp_path = path.with_extension("jsonl.tmp");
    std::fs::write(&tmp_path, &content)?;
    #[cfg(unix)]
    if let Err(e) = std::fs::set_permissions(path, PermissionsExt::from_mode(0o600)) {
        tracing::warn!(
            "[warning] Failed to set file permissions on {}: {}",
            path.display(),
            e
        );
    }
    let _ = std::fs::rename(tmp_path, path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cache_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    /// Helper to create a SemanticCache with a very short TTL for testing.
    fn make_cache(ttl_secs: u64) -> (SemanticCache, TempDir) {
        let dir = cache_dir();
        let cache = SemanticCache::open(dir.path(), Some(ttl_secs));
        (cache, dir)
    }

    #[test]
    fn test_write_and_exact_lookup() {
        let (cache, _dir) = make_cache(DEFAULT_TTL_SECS);
        cache
            .write_back("what is rust", "Rust is a systems programming language.")
            .unwrap();

        // Exact match should hit.
        assert!(cache.lookup("what is rust").is_some());
        assert_eq!(
            cache.lookup("what is rust"),
            Some("Rust is a systems programming language.".to_string())
        );
    }

    #[test]
    fn test_no_exact_match() {
        let (cache, _dir) = make_cache(DEFAULT_TTL_SECS);
        cache.write_back("hello world", "world hello").unwrap();

        // Completely different question should not have an exact match.
        assert!(cache.lookup("what is the weather").is_none());
    }

    #[test]
    fn test_semantic_similarity_lookup() {
        let (cache, _dir) = make_cache(DEFAULT_TTL_SECS);
        cache
            .write_back(
                "how do you define a function in rust",
                "Use the `fn` keyword followed by the name and parentheses.",
            )
            .unwrap();

        // Semantically similar question should return the cached answer.
        let result = cache.lookup("what is the syntax for defining functions in Rust");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            "Use the `fn` keyword followed by the name and parentheses."
        );
    }

    #[test]
    fn test_ttl_expiry() {
        let (cache, _dir) = make_cache(0); // TTL=0 means immediately expired.
        cache.write_back("test question", "test answer").unwrap();

        // Should be expired immediately since ttl is 0.
        assert!(cache.lookup("test question").is_none());
    }

    #[test]
    fn test_clear() {
        let (cache, _dir) = make_cache(DEFAULT_TTL_SECS);
        cache.write_back("question", "answer").unwrap();
        cache.clear().unwrap();

        assert!(cache.lookup("question").is_none());
    }

    #[test]
    fn test_persistence_across_instances() {
        let dir = cache_dir();
        // Create and write to a cache.
        let cache1 = SemanticCache::open(dir.path(), Some(DEFAULT_TTL_SECS));
        cache1
            .write_back("persisted question", "persisted answer")
            .unwrap();
        drop(cache1);

        // Open a new instance — should load from disk.
        let cache2 = SemanticCache::open(dir.path(), Some(DEFAULT_TTL_SECS));
        assert_eq!(
            cache2.lookup("persisted question"),
            Some("persisted answer".to_string())
        );
    }

    #[test]
    fn test_disabled_cache() {
        let cache = SemanticCache::disabled();
        // Disabled cache always returns None.
        assert!(cache.lookup("anything").is_none());
        // write_back on disabled cache should succeed silently.
        assert!(cache.write_back("q", "a").is_ok());
    }
}
