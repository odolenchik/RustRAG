use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

const CHUNK_PREFIX: &str = concat!("chunk", "_");

/// Metadata about a single indexed file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub sha256: String,
}

/// Persistent state tracking what has been indexed.
/// Stored as `index_state.json` inside `.rustrag/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexState {
    pub version: u32,
    /// Maps file paths to their SHA-256 hash at indexing time.
    #[serde(default)]
    pub files: HashMap<PathBuf, FileMetadata>,
    /// All chunk IDs that belong to this index state.
    #[serde(default)]
    pub chunk_ids: Vec<String>,
}

impl IndexState {
    pub fn new() -> Self {
        Self {
            version: 1,
            files: HashMap::new(),
            chunk_ids: Vec::new(),
        }
    }
}

impl Default for IndexState {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexState {
    /// Load state from disk. Returns a default (empty) state if the file doesn't exist.
    pub fn load(state_path: &Path) -> Result<Self> {
        let path = state_path.join("index_state.json");
        if !path.exists() {
            return Ok(Self::new());
        }
        let content = fs::read_to_string(&path)?;
        let state: IndexState = serde_json::from_str(&content)?;
        Ok(state)
    }

    /// Persist state to disk (atomic via temp file + rename).
    pub fn save(&self, state_path: &Path) -> Result<()> {
        let path = state_path.join("index_state.json");
        let tmp_path = state_path.join("index_state.json.tmp");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(&tmp_path, content.as_bytes())?;
        fs::rename(&tmp_path, &path)?;
        Ok(())
    }

    /// Compute SHA-256 hash of a file's contents.
    pub fn compute_file_hash(path: &Path) -> Result<String> {
        let mut hasher = Sha256::new();
        let reader = fs::File::open(path)?;
        let mut buf_reader = BufReader::new(reader);
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            let bytes_read = buf_reader.read(&mut buf)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buf[..bytes_read]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    }

    /// Compare current workspace files against saved state.
    /// Returns (new_files, changed_files, unchanged_file_paths, removed_chunk_ids).
    pub fn compare(
        &self,
        current_files: &HashMap<PathBuf, String>, // path -> sha256 of current content
    ) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<String>) {
        let mut new_files = Vec::new();
        let mut changed_files = Vec::new();
        let mut removed_chunk_ids = Vec::new();

        for (path, current_hash) in current_files {
            match self.files.get::<PathBuf>(path) {
                Some(saved) if saved.sha256 == *current_hash => {} // unchanged
                Some(_saved) => changed_files.push(path.clone()),  // hash changed
                None => new_files.push(path.clone()),              // new file
            }
        }

        // Find removed files (in state but not in current)
        for path in self.files.keys() {
            if !current_files.contains_key(path) {
                let prefix = format!("{}{}", CHUNK_PREFIX, path.display());
                removed_chunk_ids.extend(
                    self.chunk_ids
                        .iter()
                        .filter(|id| id.starts_with(&prefix))
                        .cloned(),
                );
            }
        }

        (new_files, changed_files, removed_chunk_ids)
    }

    /// Update state with new file hashes.
    pub fn update_files(&mut self, files: HashMap<PathBuf, String>) {
        let mut all_chunk_ids = Vec::new();
        for (path, hash) in &files {
            self.files.insert(
                path.clone(),
                FileMetadata {
                    sha256: hash.clone(),
                },
            );
            // Collect chunk IDs that would be created from this file (line_start = 0 placeholder)
            let cid = format!("{}{}_{}", CHUNK_PREFIX, path.display(), 0);
            all_chunk_ids.push(cid);
        }
        for path in self.files.keys() {
            if !files.contains_key(path) {
                let prefix = format!("{}{}", CHUNK_PREFIX, path.display());
                self.chunk_ids.retain(|id| !id.starts_with(&prefix));
            }
        }
        self.chunk_ids = all_chunk_ids;
    }

    /// Check if any files have changed since last index.
    pub fn has_changes(&self, current_files: &HashMap<PathBuf, String>) -> bool {
        let (new_files, changed_files, _) = self.compare(current_files);
        !new_files.is_empty() || !changed_files.is_empty()
    }
}
