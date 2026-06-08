use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::RwLock;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A document in the vector store with its embedding and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub chunk: crate::indexer::Chunk,
    pub embedding: Vec<f32>,
}

/// Cache entry for lazy-loaded documents with mtime-based invalidation.
struct DocCacheEntry {
    mtime: u64,
    documents: Vec<serde_json::Value>,
}

impl std::fmt::Debug for DocCacheEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocCacheEntry").field("documents", &"[..]").finish()
    }
}

/// Persistent JSONL-based vector store for RustRAG indexing.
pub struct VectorStore {
    pub path: PathBuf,
    /// Cache for lazy-loaded documents to avoid re-reading index.jsonl on every search.
    cache: RwLock<Option<DocCacheEntry>>,
}

impl VectorStore {
   /// Open or create a vector store at the given directory.
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        let path = path.as_ref();
        std::fs::create_dir_all(path)?;
        Ok(VectorStore {
            path: path.to_path_buf(),
            cache: RwLock::new(None),
        })
    }

    /// Get the default .rustrag path inside a workspace.
    pub fn for_workspace(workspace_root: impl AsRef<std::path::Path>) -> Self {
        let dir = workspace_root.as_ref().join(".rustrag");
        VectorStore::open(&dir).expect("Failed to create vector store directory")
    }

    /// Insert documents into the vector store.
    pub fn insert_documents(&self, documents: &[Document]) -> Result<()> {
        if documents.is_empty() {
            return Ok(());
        }

        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() {
            std::fs::write(&index_path, "")?;
        }

        let file = std::fs::OpenOptions::new().append(true).create(true).open(index_path)?;
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
    pub fn remove_documents(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let id_set: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();

        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() {
            return Ok(());
        }

        // Read all lines, filter out matching IDs
        let content = std::fs::read_to_string(&index_path)?;
        let mut kept_lines: Vec<String> = Vec::new();
        for line in content.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(doc_id) = value["id"].as_str() {
                    if id_set.contains(doc_id) {
                        continue; // remove this document
                    }
                }
            }
            kept_lines.push(line.to_string());
        }

        // Atomic replace: write to temp, then rename
        let tmp_path = self.path.join("index.jsonl.tmp");
        std::fs::write(&tmp_path, kept_lines.join("\n"))?;
        std::fs::rename(&tmp_path, &index_path)?;

        Ok(())
    }

   /// List all document IDs currently stored in the index.
    pub fn list_document_ids(&self) -> Result<Vec<String>> {
        let docs = self.load_documents()?;
        Ok(docs.iter()
            .filter_map(|v| v["id"].as_str().map(String::from))
            .collect())
    }

    /// Lazy-load documents from index.jsonl, using mtime-based cache.
    fn load_documents(&self) -> Result<Vec<serde_json::Value>> {
        let index_path = self.path.join("index.jsonl");
        if !index_path.exists() { return Ok(Vec::new()); }

        // Get current file mtime for cache invalidation
        let current_mtime = std::fs::metadata(&index_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|d| d.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64);

        {
            let cache = self.cache.read().unwrap();
            if let Some(entry) = &*cache {
                // Check mtime — if file hasn't changed, return cached docs
                if current_mtime == Some(entry.mtime) {
                    return Ok(entry.documents.clone());
                }
            }
        }

        // Cache miss or stale — read and parse the file
        let content = std::fs::read_to_string(&index_path)?;
        let documents: Vec<serde_json::Value> = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line))
            .collect::<Result<Vec<_>, _>>()?;

       // Update cache and return a clone for the caller
        if let Some(mtime) = current_mtime {
            let mut cache = self.cache.write().unwrap();
            *cache = Some(DocCacheEntry { mtime, documents: documents.clone() });
        }

        Ok(documents)
    }

    /// Invalidate the document cache (called after index updates).
    pub fn invalidate_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        *cache = None;
    }

    /// Search by embedding vector. Returns top-k results with relevance scores.
    pub fn search_by_embedding(&self, query_vec: &[f32], top_k: usize) -> Result<Vec<SearchResult>> {
        self.hybrid_search_internal(query_vec, "", top_k, 1.0, None, true)
    }

    /// Hybrid search combining vector similarity with BM25 text scoring.
    /// `alpha` in [0,1]: 1.0 = pure vector, 0.0 = pure BM25, ~0.7 = recommended blend.
    pub fn hybrid_search(
        &self,
        query_vec: &[f32],
        query_text: &str,
        top_k: usize,
        alpha: f64,
        filters: Option<&SearchFilters>,
    ) -> Result<Vec<SearchResult>> {
        self.hybrid_search_internal(query_vec, query_text, top_k, alpha.clamp(0.0, 1.0), filters, false)
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
    ) -> Result<Vec<SearchResult>> {
      let documents = self.load_documents()?;

        if documents.is_empty() {
            return Ok(Vec::new());
        }

        // Build BM25 inverted index in-memory (fast for typical workspaces)
        let (inverted, doc_stats): (InvertedIndex, HashMap<String, DocStat>) = self.build_inverted_index(&documents)?;

        // Tokenize query text for BM25
        let query_tokens = tokenize(query_text);

        // Compute average document length for BM25 normalization
        let total_docs = documents.len();
        let avgdl: f64 = doc_stats.values().map(|s| s.doc_len as f64).sum::<f64>() / total_docs.max(1) as f64;

        // Score each document with both vector similarity and BM25.
        // Stores (combined_score, vec_similarity_f32, original_index) so we don't recompute cosine sim.
        type DocScore = (f64, f32, usize); // (combined_score, vector_sim_for_result, original_index)
        let mut scored: Vec<DocScore> = Vec::new();

        for (idx, doc) in documents.iter().enumerate() {
            let doc_id = doc["id"].as_str().unwrap_or("").to_string();

            // --- Vector similarity score (computed once per document) ---
            let vec_score_val: f32 = if let Some(embedding) = doc.get("embedding") {
                if let Some(embed_arr) = embedding.as_array() {
                    let embed_f32: Vec<f32> = embed_arr.iter().filter_map(|v| v.as_f64()).map(|f| f as f32).collect();
                    cosine_similarity(query_vec, &embed_f32)
                } else { 0.0 }
            } else { 0.0 };

            // --- BM25 score ---
            let bm25_val: f64 = if !pure_vector && !query_tokens.is_empty() && !inverted.is_empty() {
                let mut doc_bm25: f64 = 0.0;
                for token in &query_tokens {
                    if let Some(postings) = inverted.get(token.as_str()) {
                        // Find this document's term frequency in postings
                        let posting = postings.iter().find(|p| p.doc_id == doc_id);
                        if let Some(p) = posting {
                            let df = postings.len() as u64;
                            doc_bm25 += bm25_term_score(p.tf, doc_stats.get(&doc_id).map(|s| s.doc_len).unwrap_or(0.0), avgdl, df, total_docs);
                        }
                    }
                }
                doc_bm25
            } else { 0.0 };

            // Normalize BM25 to [0,1] range
            let bm25_normalized = if avgdl > 0.0 { (bm25_val / avgdl).max(0.0) } else { 0.0 };

            // --- Combine scores ---
            let combined = alpha * vec_score_val as f64 + (1.0 - alpha) * bm25_normalized;
            scored.push((combined, vec_score_val, idx));
        }

        // Sort by combined score descending
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Apply filters and build SearchResult objects (reuse precomputed similarity)
        let mut results: Vec<SearchResult> = Vec::new();
        for (_, vec_sim, idx) in scored {
            if results.len() >= top_k { break; }

            let doc = &documents[idx];

            // Apply metadata filters before including result
            if let Some(flt) = filters {
                if !flt.matches(doc) {
                    continue;
                }
            }

            results.push(SearchResult {
                id: doc["id"].as_str().unwrap_or("").to_string(),
                file_path: PathBuf::from(doc["file_path"].as_str().unwrap_or("")),
                line_start: doc["line_start"].as_u64().unwrap_or(0) as usize,
                line_end: doc["line_end"].as_u64().unwrap_or(0) as usize,
                module_name: doc["module_name"].as_str().unwrap_or("").to_string(),
                symbol_kind: serde_json::from_value(doc["symbol_kind"].clone()).ok()
                    .map(|v: SymbolKindWrapper| v.0),
                text: doc["text"].as_str().unwrap_or("").to_string(),
                score: vec_sim,
            });
        }

        Ok(results)
    }

    /// Build an inverted index from documents for BM25 scoring.
    fn build_inverted_index(
        &self,
        documents: &[serde_json::Value],
    ) -> Result<(InvertedIndex, HashMap<String, DocStat>)> {
        let mut inverted: InvertedIndex = HashMap::new();
        let mut doc_stats: HashMap<String, DocStat> = HashMap::new();

        for doc in documents {
            let text = doc["text"].as_str().unwrap_or("");
            let doc_id = doc["id"].as_str().unwrap_or("").to_string();
            if doc_id.is_empty() || text.trim().is_empty() { continue; }

            // Tokenize: lowercase + split on non-alphanumeric (keep underscores for Rust identifiers)
            let tokens = tokenize(text);
            let doc_len = tokens.len() as f64;

            // Count term frequencies for this document
            let mut tf_map: HashMap<String, f64> = HashMap::new();
            for token in &tokens {
                *tf_map.entry(token.clone()).or_default() += 1.0;
            }

            doc_stats.insert(doc_id.clone(), DocStat { doc_len });

            // Add to inverted index (one posting per unique term per document)
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
}

// ---------------------------------------------------------------------------
// BM25 inverted index structures
// ---------------------------------------------------------------------------

/// A term posting: which document ids contain this term and how often.
#[derive(Debug)]
struct Posting {
    doc_id: String,
    tf: f64, // term frequency in this document
}

/// Inverted index entry: term -> postings list.
type InvertedIndex = HashMap<String, Vec<Posting>>;

/// Per-document statistics needed for BM25 normalization.
#[derive(Debug)]
struct DocStat {
    doc_len: f64, // number of tokens in this document
}

/// BM25 parameters — standard values from Robertson et al.
const BM25_K1: f64 = 1.5;
const BM25_B: f64 = 0.75;

/// Compute BM25 score for a single term in a single document.
fn bm25_term_score(tf: f64, doc_len: f64, avgdl: f64, df: u64, total_docs: usize) -> f64 {
    if tf == 0.0 || df as usize >= total_docs { return 0.0; }

    // IDF component
    let idf = ((total_docs as f64 - df as f64 + 0.5) / (df as f64 + 0.5)).ln().max(1e-10);

    // TF component with length normalization
    let tf_component = tf / (tf + BM25_K1 * (1.0 - BM25_B + BM25_B * doc_len.max(1.0) / avgdl.max(1.0)));

    idf * tf_component
}

// ---------------------------------------------------------------------------
// Tokenization helper
// ---------------------------------------------------------------------------

/// Tokenize text into lowercase alphanumeric tokens for BM25.
/// Splits on whitespace and punctuation, keeps Rust identifiers intact.
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|s| s.len() > 1) // skip single-char tokens (noise)
        .map(|s| s.to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Search filters
// ---------------------------------------------------------------------------

/// Optional metadata filters for search queries.
#[derive(Default, Clone)]
pub struct SearchFilters {
    pub file_extension: Option<String>,
    pub symbol_kind: Option<SymbolKind>,
}

impl SearchFilters {
    fn matches(&self, doc: &serde_json::Value) -> bool {
        if let Some(ext) = &self.file_extension {
            let actual_path = doc["file_path"].as_str().unwrap_or("");
            let actual_ext = std::path::Path::new(actual_path)
                .extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default();
            if actual_ext.as_ref() != ext { return false; }
        }
        if let Some(kind) = &self.symbol_kind {
            let stored = doc.get("symbol_kind").and_then(|v| v.as_str());
            if stored.map(|s| s.to_lowercase()) != Some(kind.symbol_name().to_lowercase()) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// SearchResult & SymbolKind
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub file_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub module_name: String,
    pub symbol_kind: Option<SymbolKind>,
    pub text: String,
    pub score: f32,
}

/// Symbol kind for deserialization from JSON (stored as strings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    ImplBlock,
    UnsafeRegion,
    TraitImpl,
    Module,
    Struct,
    Enum,
    Macro,
}

impl SymbolKind {
    fn symbol_name(&self) -> &'static str {
        match self {
            SymbolKind::Function => "Function",
            SymbolKind::ImplBlock => "ImplBlock",
            SymbolKind::UnsafeRegion => "UnsafeRegion",
            SymbolKind::TraitImpl => "TraitImpl",
            SymbolKind::Module => "Module",
            SymbolKind::Struct => "Struct",
            SymbolKind::Enum => "Enum",
            SymbolKind::Macro => "Macro",
        }
    }
}

// Wrapper for serde round-trip
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SymbolKindWrapper(pub SymbolKind);

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
