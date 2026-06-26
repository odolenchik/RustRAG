//! Persistent JSONL-based vector store with BM25 text scoring and cosine similarity.
//!
//! Documents are stored as line-delimited JSON in an `index.jsonl` file. The crate provides
//! hybrid search combining precomputed embedding similarity (cosine) with on-the-fly BM25
//! text scoring over the indexed document corpus.

#![warn(missing_docs)]

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use rust_rag_indexer::Chunk;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::RwLock;

pub use rust_rag_error::RagCoreError;

// ---------------------------------------------------------------------------
// Document storage types
// ---------------------------------------------------------------------------

/// A document in the vector store with its embedding and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    /// Unique identifier for this document.
    pub id: String,
    /// The source code chunk this document represents.
    pub chunk: Chunk,
    /// Embedding vector (must match the dimensionality of the embedding model).
    pub embedding: Vec<f32>,
}

/// Cache entry for lazy-loaded documents with mtime-based invalidation.
struct DocCacheEntry {
    mtime: u64,
    documents: Vec<serde_json::Value>,
    /// Precomputed L2 norms of each document's embedding vector.
    norms: Vec<f32>,
}

impl std::fmt::Debug for DocCacheEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocCacheEntry")
            .field("documents", &"[..]")
            .field("norms", &self.norms.len())
            .finish()
    }
}

/// Cached BM25 inverted index entry — invalidated when the index file changes.
struct Bm25CacheEntry {
    file_mtime: u64,
    doc_count: usize,
    inverted_index: InvertedIndex,
    doc_stats: HashMap<String, DocStat>,
}

/// Persistent JSONL-based vector store for RustRAG indexing.
pub struct VectorStore {
    /// Path to the directory containing index.jsonl and related files.
    pub path: PathBuf,
    cache: RwLock<Option<DocCacheEntry>>,
    bm25_cache: RwLock<Option<Bm25CacheEntry>>,
}

impl VectorStore {
    /// Open or create a vector store at the given directory.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, RagCoreError> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;
        #[cfg(unix)]
        Self::set_restricted_permissions(path);
        Ok(VectorStore {
            path: path.to_path_buf(),
            cache: RwLock::new(None),
            bm25_cache: RwLock::new(None),
        })
    }

    /// Set directory permissions to 0700 (owner-only) on Unix systems.
    #[cfg(unix)]
    fn set_restricted_permissions(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)) {
            eprintln!(
                "[warning] Failed to set restrictive permissions on {}: {}",
                path.display(),
                e
            );
        }
    }

    /// No-op on non-Unix platforms.
    #[cfg(not(unix))]
    fn set_restricted_permissions(_path: &std::path::Path) {}

    /// Set restrictive permissions (0600) on a file — used for index/cache JSONL files.
    fn restrict_file_permissions(path: &std::path::Path) {
        #[cfg(unix)]
        if let Err(e) = std::fs::set_permissions(path, PermissionsExt::from_mode(0o600)) {
            tracing::warn!(
                "[warning] Failed to set file permissions on {}: {}",
                path.display(),
                e
            );
        }
    }

    /// Get the default `.rustrag` path inside a workspace.
    pub fn for_workspace(workspace_root: impl AsRef<std::path::Path>) -> Self {
        let dir = workspace_root.as_ref().join(".rustrag");
        VectorStore::open(&dir).unwrap_or_else(|e| {
            panic!(
                "Failed to create vector store directory '{}': {}",
                dir.display(),
                e
            )
        })
    }

    /// Insert documents into the vector store.
    pub fn insert_documents(&self, documents: &[Document]) -> Result<(), RagCoreError> {
        if documents.is_empty() {
            return Ok(());
        }

        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() {
            std::fs::write(&index_path, "")?;
            Self::restrict_file_permissions(&index_path);
        }

        let file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(index_path)?;
        let mut writer = std::io::BufWriter::new(file);
        for doc in documents {
            let value = serde_json::json!({
                "id": doc.id,
                "file_path": doc.chunk.file_path.to_string_lossy(),
                "line_start": doc.chunk.line_start,
                "line_end": doc.chunk.line_end,
                "module_name": doc.chunk.module_name,
                "symbol_kind": &doc.chunk.symbol_kind,
                "text": doc.chunk.text,
                "embedding": &doc.embedding,
            });
            let line = serde_json::to_string(&value)?;
            writeln!(writer, "{}", line)?;
        }
        writer.flush()?;

        Ok(())
    }

    /// Remove documents matching the given IDs from index.jsonl (atomic replace).
    pub fn remove_documents(&self, ids: &[String]) -> Result<(), RagCoreError> {
        if ids.is_empty() {
            return Ok(());
        }
        let id_set: HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();

        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&index_path)?;
        let mut kept_lines: Vec<String> = Vec::new();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(doc_id) = value["id"].as_str() {
                    if id_set.contains(doc_id) {
                        continue;
                    }
                }
            }
            kept_lines.push(line.to_string());
        }

        let tmp_path = self.path.join("index.jsonl.tmp");
        std::fs::write(&tmp_path, kept_lines.join("\n"))?;
        std::fs::rename(&tmp_path, &index_path)?;

        Ok(())
    }

    /// List all document IDs currently stored in the index.
    pub fn list_document_ids(&self) -> Result<Vec<String>, RagCoreError> {
        let docs = self.load_documents()?;
        Ok(docs
            .iter()
            .filter_map(|v| v["id"].as_str().map(String::from))
            .collect())
    }

    /// Lazy-load documents from index.jsonl, using mtime-based cache.
    fn load_documents(&self) -> Result<Vec<serde_json::Value>, RagCoreError> {
        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() {
            return Ok(Vec::new());
        }

        let current_mtime = std::fs::metadata(&index_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|d| d.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);

        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = &*cache {
                if current_mtime == Some(entry.mtime) {
                    return Ok(entry.documents.clone());
                }
            }
        }

        let content = std::fs::read_to_string(&index_path)?;
        let documents: Vec<serde_json::Value> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).map_err(|e| {
                    RagCoreError::VectorStore(
                        format!("parse JSON on line: {}", e),
                        Box::new(std::io::Error::other(e)),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        if let Some(mtime) = current_mtime {
            let mut cache = self.cache.write().unwrap();
            let mut norms = Vec::with_capacity(documents.len());
            for doc in &documents {
                let norm = if let Some(embedding) = doc.get("embedding") {
                    if let Some(embed_arr) = embedding.as_array() {
                        let embed_f32: Vec<f32> = embed_arr
                            .iter()
                            .filter_map(|v| v.as_f64())
                            .map(|f| f as f32)
                            .collect();
                        if embed_f32.is_empty() {
                            0.0
                        } else {
                            let sum_sq: f32 = embed_f32.iter().map(|&x| x * x).sum();
                            sum_sq.sqrt()
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                norms.push(norm);
            }
            *cache = Some(DocCacheEntry {
                mtime,
                documents: documents.clone(),
                norms,
            });
        }

        Ok(documents)
    }

    /// Invalidate both the document cache and BM25 inverted index cache.
    pub fn invalidate_cache(&self) {
        let mut doc_cache = self.cache.write().unwrap();
        *doc_cache = None;
        let mut bm25_cache = self.bm25_cache.write().unwrap();
        *bm25_cache = None;
    }

    /// Search by embedding vector. Returns top-k results with relevance scores.
    pub fn search_by_embedding(
        &self,
        query_vec: &[f32],
        top_k: usize,
    ) -> Result<Vec<SearchResult>, RagCoreError> {
        self.hybrid_search_internal(query_vec, "", top_k, 1.0, None, true)
    }

    /// Hybrid search combining vector similarity with BM25 text scoring.
    pub fn hybrid_search(
        &self,
        query_vec: &[f32],
        query_text: &str,
        top_k: usize,
        alpha: f64,
        filters: Option<&SearchFilters>,
    ) -> Result<Vec<SearchResult>, RagCoreError> {
        self.hybrid_search_internal(
            query_vec,
            query_text,
            top_k,
            alpha.clamp(0.0, 1.0),
            filters,
            false,
        )
    }

    /// Internal hybrid search shared by both public methods.
    fn hybrid_search_internal(
        &self,
        query_vec: &[f32],
        query_text: &str,
        top_k: usize,
        alpha: f64,
        filters: Option<&SearchFilters>,
        pure_vector: bool,
    ) -> Result<Vec<SearchResult>, RagCoreError> {
        let documents = self.load_documents()?;
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        // Calculate norms from documents to avoid double loading
        let mut norms = Vec::with_capacity(documents.len());
        for doc in &documents {
            let norm = if let Some(embedding) = doc.get("embedding") {
                if let Some(embed_arr) = embedding.as_array() {
                    let embed_f32: Vec<f32> = embed_arr
                        .iter()
                        .filter_map(|v| v.as_f64())
                        .map(|f| f as f32)
                        .collect();
                    if embed_f32.is_empty() {
                        0.0
                    } else {
                        let sum_sq: f32 = embed_f32.iter().map(|&x| x * x).sum();
                        sum_sq.sqrt()
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };
            norms.push(norm);
        }

        let (inverted, doc_stats) = self.get_bm25_cache(&documents)?;

        let query_tokens = tokenize(query_text);

        let total_docs = documents.len();
        let avgdl: f64 =
            doc_stats.values().map(|s| s.doc_len).sum::<f64>() / total_docs.max(1) as f64;

        let query_mag = cosine_similarity(query_vec, query_vec).abs().max(1e-10);

        type DocScore = (f64, f32, f64, usize);
        let mut scored: Vec<DocScore> = Vec::new();

        for (idx, doc) in documents.iter().enumerate() {
            let doc_id = doc["id"].as_str().unwrap_or("").to_string();

            let vec_score_val: f32 = if let Some(embedding) = doc.get("embedding") {
                if let Some(embed_arr) = embedding.as_array() {
                    let embed_f32: Vec<f32> = embed_arr
                        .iter()
                        .filter_map(|v| v.as_f64())
                        .map(|f| f as f32)
                        .collect();
                    if embed_f32.is_empty() {
                        0.0
                    } else {
                        let dot: f32 = query_vec
                            .iter()
                            .zip(embed_f32.iter())
                            .map(|(q, e)| q * e)
                            .sum();
                        let doc_norm = norms[idx];
                        if doc_norm == 0.0 || query_mag == 0.0 {
                            0.0
                        } else {
                            dot / (query_mag * doc_norm)
                        }
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            };

            let bm25_val: f64 = if !pure_vector && !query_tokens.is_empty() && !inverted.is_empty()
            {
                let mut doc_bm25: f64 = 0.0;
                for token in &query_tokens {
                    if let Some(postings) = inverted.get(token.as_str()) {
                        let posting = postings.iter().find(|p| p.doc_id == doc_id);
                        if let Some(p) = posting {
                            let df = postings.len() as u64;
                            doc_bm25 += bm25_term_score(
                                p.tf,
                                doc_stats.get(&doc_id).map(|s| s.doc_len).unwrap_or(0.0),
                                avgdl,
                                df,
                                total_docs,
                            );
                        }
                    }
                }
                doc_bm25
            } else {
                0.0
            };

            let bm25_normalized = if total_docs > 0 && avgdl > 0.0 {
                (bm25_val / avgdl).max(0.0)
            } else {
                0.0
            };

            let combined = alpha * vec_score_val as f64 + (1.0 - alpha) * bm25_normalized;
            scored.push((combined, vec_score_val, bm25_val, idx));
        }

        // Partial sort to get top-k
        if scored.len() <= top_k {
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            scored.select_nth_unstable_by(top_k, |a, b| {
                b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(top_k);
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        }

        let mut results: Vec<SearchResult> = Vec::new();
        for (_, vec_sim, bm25_val, idx) in scored {
            let doc = &documents[idx];

            if let Some(flt) = filters {
                if !flt.matches(doc) {
                    continue;
                }
            }

            let doc_id_for_err = doc["id"].as_str().unwrap_or("").to_string();
            let file_path_for_err = doc["file_path"].as_str().unwrap_or("").to_string();

            let symbol_kind = match doc["symbol_kind"].as_str() {
                Some(s) => match s.to_lowercase().as_str() {
                    "function" => Some(SymbolKind::Function),
                    "implblock" => Some(SymbolKind::ImplBlock),
                    "unsaferegion" => Some(SymbolKind::UnsafeRegion),
                    "traitimpl" => Some(SymbolKind::TraitImpl),
                    "module" => Some(SymbolKind::Module),
                    "struct" => Some(SymbolKind::Struct),
                    "enum" => Some(SymbolKind::Enum),
                    "macro" => Some(SymbolKind::Macro),
                    _other => {
                        return Err(RagCoreError::UnknownSymbolKind(
                            s.to_string(),
                            doc_id_for_err.clone(),
                            file_path_for_err.clone(),
                        ))
                    }
                },
                None => None,
            };

            // Calculate confidence score based on:
            // 1. Normalized combined score (higher score = higher confidence)
            // 2. Agreement between vector and BM25 scores when both available
            let mut confidence = vec_sim.clamp(0.0, 1.0); // Base confidence from normalized score

            // If we have both vector and BM25 scores, boost confidence when they agree
            if let (Some(vector_score), Some(bm25_score)) = (Some(vec_sim), Some(bm25_val as f32)) {
                // Calculate agreement (1.0 - normalized difference)
                let diff = (vector_score - bm25_score).abs();
                let agreement = 1.0 - diff.clamp(0.0, 1.0);
                // Boost confidence by up to 0.2 based on agreement
                confidence = confidence + (agreement * 0.2);
                // Ensure confidence stays in [0, 1] range
                confidence = confidence.clamp(0.0, 1.0);
            }

            results.push(SearchResult {
                id: doc["id"].as_str().unwrap_or("").to_string(),
                file_path: PathBuf::from(doc["file_path"].as_str().unwrap_or("")),
                line_start: doc["line_start"].as_u64().unwrap_or(0) as usize,
                line_end: doc["line_end"].as_u64().unwrap_or(0) as usize,
                module_name: doc["module_name"].as_str().unwrap_or("").to_string(),
                symbol_kind,
                text: doc["text"].as_str().unwrap_or("").to_string(),
                score: vec_sim,
                vector_score: Some(vec_sim),
                bm25_score: Some(bm25_val as f32),
                confidence,
            });
        }

        Ok(results)
    }

    fn build_inverted_index(
        &self,
        documents: &[serde_json::Value],
    ) -> Result<(InvertedIndex, HashMap<String, DocStat>), RagCoreError> {
        let mut inverted: InvertedIndex = HashMap::new();
        let mut doc_stats: HashMap<String, DocStat> = HashMap::new();

        for doc in documents {
            let text = doc["text"].as_str().unwrap_or("");
            let doc_id = doc["id"].as_str().unwrap_or("").to_string();
            if doc_id.is_empty() || text.trim().is_empty() {
                continue;
            }

            let tokens = tokenize(text);
            let doc_len = tokens.len() as f64;

            let mut tf_map: HashMap<String, f64> = HashMap::new();
            for token in &tokens {
                *tf_map.entry(token.clone()).or_default() += 1.0;
            }

            doc_stats.insert(doc_id.clone(), DocStat { doc_len });

            let terms: HashSet<String> = tokens.into_iter().collect();
            for term in terms {
                let tf = *tf_map.get(&term).unwrap_or(&0.0);
                inverted.entry(term).or_default().push(Posting {
                    doc_id: doc_id.clone(),
                    tf,
                });
            }
        }

        Ok((inverted, doc_stats))
    }

    fn get_bm25_cache(
        &self,
        documents: &[serde_json::Value],
    ) -> Result<(InvertedIndex, HashMap<String, DocStat>), RagCoreError> {
        let index_path = self.path.join("index.jsonl");

        let (current_mtime, doc_count) = if index_path.exists() {
            let meta = std::fs::metadata(&index_path)?;
            let mtime = meta
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64;
            (mtime, documents.len())
        } else {
            (0, 0)
        };

        {
            let cache = self.bm25_cache.read().unwrap();
            if let Some(entry) = &*cache {
                if entry.file_mtime == current_mtime && entry.doc_count == doc_count {
                    return Ok((entry.inverted_index.clone(), entry.doc_stats.clone()));
                }
            }
        }

        let (inverted, doc_stats) = self.build_inverted_index(documents)?;

        if !documents.is_empty() {
            let mut cache = self.bm25_cache.write().unwrap();
            *cache = Some(Bm25CacheEntry {
                file_mtime: current_mtime,
                doc_count,
                inverted_index: inverted.clone(),
                doc_stats: doc_stats.clone(),
            });
        }

        Ok((inverted, doc_stats))
    }
}

// ---------------------------------------------------------------------------
// BM25 structures
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Posting {
    doc_id: String,
    tf: f64,
}

type InvertedIndex = HashMap<String, Vec<Posting>>;

#[derive(Clone)]
struct DocStat {
    doc_len: f64,
}

const BM25_K1: f64 = 1.5;
const BM25_B: f64 = 0.75;

fn bm25_term_score(tf: f64, doc_len: f64, avgdl: f64, df: u64, total_docs: usize) -> f64 {
    if tf == 0.0 || df as usize >= total_docs {
        return 0.0;
    }

    let idf = ((total_docs as f64 - df as f64 + 0.5) / (df as f64 + 0.5))
        .ln()
        .max(1e-10);

    let tf_component =
        tf / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len.max(1.0) / avgdl.max(1.0)));

    idf * tf_component
}

// ---------------------------------------------------------------------------
// Tokenization
// ---------------------------------------------------------------------------

fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Search filters
// ---------------------------------------------------------------------------

use rust_rag_indexer::SymbolKind;

/// Optional metadata filters for search queries.
#[derive(Default, Clone)]
pub struct SearchFilters {
    /// Filter to only include documents from files with this extension (e.g., "rs").
    pub file_extension: Option<String>,
    /// Filter to only include documents of this symbol kind.
    pub symbol_kind: Option<String>,
}

impl SearchFilters {
    fn matches(&self, doc: &serde_json::Value) -> bool {
        if let Some(ext) = &self.file_extension {
            let actual_path = doc["file_path"].as_str().unwrap_or("");
            let actual_ext = std::path::Path::new(actual_path)
                .extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default();
            if actual_ext.as_ref() != ext {
                return false;
            }
        }
        if let Some(kind_str) = &self.symbol_kind {
            let stored_kind_str = doc
                .get("symbol_kind")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let stored_kind = match parse_symbol_kind(stored_kind_str) {
                Ok(k) => k,
                Err(_) => return false, // Invalid stored symbol kind
            };
            let filter_kind = match parse_symbol_kind(kind_str) {
                Ok(k) => k,
                Err(_) => return false, // Invalid filter symbol kind
            };
            if stored_kind != filter_kind {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// SearchResult with custom deserializer for SymbolKind
// ---------------------------------------------------------------------------

/// Parse a string into a SymbolKind enum.
///
/// # Arguments
///
/// * `s` - String representation of symbol kind (e.g., "function", "struct")
///
/// # Returns
///
/// * `Ok(SymbolKind)` if parsing succeeds
/// * `Err(RagCoreError::UnknownSymbolKind)` if parsing fails
pub fn parse_symbol_kind(s: &str) -> Result<SymbolKind, RagCoreError> {
    match s.to_lowercase().as_str() {
        "function" => Ok(SymbolKind::Function),
        "implblock" => Ok(SymbolKind::ImplBlock),
        "unsaferegion" => Ok(SymbolKind::UnsafeRegion),
        "traitimpl" => Ok(SymbolKind::TraitImpl),
        "module" => Ok(SymbolKind::Module),
        "struct" => Ok(SymbolKind::Struct),
        "enum" => Ok(SymbolKind::Enum),
        "macro" => Ok(SymbolKind::Macro),
        other => Err(RagCoreError::UnknownSymbolKind(
            other.to_string(),
            String::new(),
            String::new(),
        )),
    }
}

/// A search result returned by the vector store.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    /// Unique document ID.
    pub id: String,
    /// Source file path.
    pub file_path: PathBuf,
    /// Start line number.
    pub line_start: usize,
    /// End line number.
    pub line_end: usize,
    /// Module name of the symbol.
    pub module_name: String,
    /// Symbol kind (may be None if not deserializable).
    pub symbol_kind: Option<SymbolKind>,
    /// Source text snippet.
    pub text: String,
    /// Combined score from hybrid search (alpha * vector + (1-alpha) * BM25).
    pub score: f32,
    /// Vector similarity score (cosine similarity).
    pub vector_score: Option<f32>,
    /// BM25 text score.
    pub bm25_score: Option<f32>,
    /// Confidence score indicating reliability of the result (0.0-1.0).
    /// Higher values indicate greater confidence in the result's relevance.
    pub confidence: f32,
}

impl<'de> Deserialize<'de> for SearchResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SearchResultHelper {
            id: String,
            file_path: PathBuf,
            line_start: usize,
            line_end: usize,
            module_name: String,
            symbol_kind: Option<String>,
            text: String,
            score: f32,
            vector_score: Option<f32>,
            bm25_score: Option<f32>,
            confidence: Option<f32>,
        }

        let helper = SearchResultHelper::deserialize(deserializer)?;
        Ok(SearchResult {
            id: helper.id,
            file_path: helper.file_path,
            line_start: helper.line_start,
            line_end: helper.line_end,
            module_name: helper.module_name,
            symbol_kind: match helper.symbol_kind {
                Some(s) => Some(parse_symbol_kind(&s).map_err(serde::de::Error::custom)?),
                None => None,
            },
            text: helper.text,
            score: helper.score,
            vector_score: helper.vector_score,
            bm25_score: helper.bm25_score,
            confidence: helper.confidence.unwrap_or(0.0),
        })
    }
}

// ---------------------------------------------------------------------------
// Cosine similarity helpers
// ---------------------------------------------------------------------------

/// Compute cosine similarity between two vectors. Returns a value in [-1, 1].
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (mag_a * mag_b)
}

/// Compute cosine similarity with a precomputed query magnitude.
pub fn cosine_similarity_with_precomputed(a: &[f32], query_mag: f32, b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if query_mag == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    dot / (query_mag * mag_b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_rag_indexer::SymbolKind;

    fn make_test_vector_store() -> (tempfile::TempDir, VectorStore) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();
        let store = VectorStore::open(path).expect("should create store");

        let chunk = Chunk {
            file_path: PathBuf::from("test.rs"),
            line_start: 1,
            line_end: 5,
            module_name: "test".to_string(),
            symbol_kind: SymbolKind::Function,
            text: "fn test_fn() -> &'static str { \"hello\" }".to_string(),
            max_nesting_depth: None,
        };

        let embedding = vec![0.1; 384];
        let doc = Document {
            id: "test_doc_1".into(),
            chunk,
            embedding,
        };

        store.insert_documents(&[doc]).expect("should insert");

        (dir, store)
    }

    #[test]
    fn test_vector_store_roundtrip() {
        let (_dir, store) = make_test_vector_store();

        let query_vec: Vec<f32> = vec![1.0; 384];
        let results = store
            .search_by_embedding(&query_vec, 5)
            .expect("should search");

        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result.file_path.display().to_string(), "test.rs");
        assert_eq!(result.line_start, 1);
        assert!(!result.text.is_empty());
    }

    #[test]
    fn test_vector_store_empty_search() {
        let dir = tempfile::tempdir().unwrap();
        let store = VectorStore::open(dir.path()).expect("should create empty store");

        let query_vec: Vec<f32> = vec![1.0; 384];
        let results = store
            .search_by_embedding(&query_vec, 5)
            .expect("should search");
        assert!(results.is_empty());
    }

    #[test]
    fn test_cosine_similarity_identical_vectors() {
        let v = vec![1.0; 384];
        let sim = cosine_similarity(&v, &v);
        // f32 accumulation over 384 elements gives ~5e-8 error; use a tolerant threshold.
        assert!((sim - 1.0).abs() < 1e-7);
    }

    #[test]
    fn test_cosine_similarity_orthogonal_vectors() {
        let mut a = vec![0.0; 384];
        a[0] = 1.0;
        let mut b = vec![0.0; 384];
        b[1] = 1.0;
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-9);
    }

    #[test]
    fn test_cosine_similarity_opposite_vectors() {
        let a: Vec<f32> = vec![1.0, 2.0, 3.0];
        let b: Vec<f32> = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim < -0.99);
    }

    #[test]
    fn test_cosine_similarity_empty_vectors() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![1.0; 384];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_cosine_similarity_mismatched_lengths() {
        let a = vec![1.0; 100];
        let b = vec![1.0; 200];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }
}
