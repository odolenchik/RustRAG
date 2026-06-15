use crate::error::RagCoreError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Walk a Cargo workspace directory and extract all code chunks.
#[tracing::instrument(level = "info", skip(root), fields(workspace = %root.display()))]
pub fn index_workspace(root: &Path) -> Result<Vec<Chunk>, RagCoreError> {
    let mut chunks = Vec::new();

    let manifest = root.join("Cargo.toml");
    if !manifest.exists() {
        return Err(RagCoreError::MissingCargoToml(root.to_path_buf()));
    }

    let cargo_content = std::fs::read_to_string(&manifest).map_err(|e| {
        RagCoreError::FileRead(
            manifest.clone(),
            Box::new(std::io::Error::other(format!("reading Cargo.toml: {}", e))),
        )
    })?;
    let cargo_toml: toml::Value = cargo_content.parse().map_err(|e| {
        RagCoreError::FileRead(
            manifest.clone(),
            Box::new(std::io::Error::other(format!("parsing Cargo.toml: {}", e))),
        )
    })?;

    let member_paths = extract_workspace_members(&cargo_toml, root);

    for member_path in member_paths {
        let src_dir = member_path.join("src");
        if !src_dir.exists() {
            continue;
        }
        collect_rs_files(&src_dir, &mut chunks)?;
    }

    // Apply overlap after all AST extraction is done
    apply_overlap(&mut chunks);

    tracing::info!(chunks = chunks.len(), "indexing complete");

    Ok(chunks)
}

/// A unit of code to be embedded — typically a function, impl block, or unsafe region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub file_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub module_name: String,
    pub symbol_kind: SymbolKind,
    pub text: String,
}

impl Chunk {
    pub fn symbol_kind_name(&self) -> &'static str {
        match self.symbol_kind {
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

impl From<&SymbolKind> for Option<SymbolKind> {
    #[inline]
    fn from(kind: &SymbolKind) -> Self {
        Some(kind.clone())
    }
}

impl SymbolKind {
    /// Returns the lowercase string name used in JSONL storage.
    pub const fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::ImplBlock => "implblock",
            SymbolKind::UnsafeRegion => "unsaferegion",
            SymbolKind::TraitImpl => "traitimpl",
            SymbolKind::Module => "module",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Macro => "macro",
        }
    }
}

/// Apply configured overlap to chunks within each file.
/// Expands boundaries by including adjacent lines from neighboring chunks,
/// so context at chunk boundaries is preserved for the LLM.
///
/// Optimizations:
/// - Each file is read only once (not N times).
/// - Byte→line mapping uses a precomputed prefix array of cumulative line lengths — O(1) lookup.
#[allow(dead_code)] // called internally by index_workspace; pub(crate) for test access
#[tracing::instrument(level = "debug", skip(chunks), fields(chunk_count = chunks.len()))]
pub fn apply_overlap(chunks: &mut [Chunk]) {
    let overlap = crate::config::Config::find()
        .ok()
        .map(|c| c.embedding.chunk_overlap)
        .unwrap_or(0);

    if overlap == 0 || chunks.is_empty() {
        return;
    }

    // Group indices by file path
    let mut file_indices: std::collections::HashMap<PathBuf, Vec<usize>> =
        std::collections::HashMap::new();
    for (idx, chunk) in chunks.iter().enumerate() {
        file_indices
            .entry(chunk.file_path.clone())
            .or_default()
            .push(idx);
    }

    for (_path, mut indices) in file_indices {
        if indices.len() <= 1 {
            continue;
        }

        // Sort by line_start to ensure correct neighbor relationships
        indices.sort_by_key(|&i| chunks[i].line_start);

        let n = indices.len();

        // Cache per-file: read once, reuse for all chunks in this file
        let mut content_cache: std::collections::HashMap<PathBuf, Option<String>> =
            Default::default();

        for i in 0..n {
            let ci = indices[i];
            let file_path = chunks[ci].file_path.clone();

            // Read file once per unique path (lazy)
            content_cache
                .entry(file_path.clone())
                .or_insert_with(|| std::fs::read_to_string(&file_path).ok());

            if let Some(None) = content_cache.get(&file_path) {
                eprintln!(
                    "warning: could not read file for overlap expansion: {}",
                    file_path.display()
                );
                continue;
            }
            let Some(content) = &content_cache[&file_path] else {
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            let total_lines = lines.len();

            // Precompute cumulative byte offsets for O(1) byte→line conversion.
            // `byte_offsets[l]` = byte offset of the first byte of line l.
            let mut byte_offsets: Vec<usize> = Vec::with_capacity(total_lines + 1);
            let mut offset = 0;
            byte_offsets.push(0);
            for line in &lines {
                // Each line in `content` is stored without its trailing newline,
                // so the next line starts at current_offset + line.len() + 1 (for \n).
                offset += line.len() + 1;
                byte_offsets.push(offset);
            }

            // Convert a byte offset to a line number using precomputed offsets (binary search, O(log N)).
            // byte_offsets[l] = byte offset of the first character of line l.
            // Find l such that byte_offsets[l] <= byte_offset < byte_offsets[l+1].
            let byte_to_line = |byte_offset: usize| -> usize {
                if byte_offset == 0 {
                    return 0;
                }
                // partition_point returns the first index where predicate is false.
                // Since byte_offsets is sorted ascending, this finds the first position
                // where byte_offsets[pos] > byte_offset, then we subtract 1.
                byte_offsets
                    .partition_point(|&bo| bo <= byte_offset)
                    .saturating_sub(1)
            };

            let mut chunk = chunks[ci].clone();

            // Extend START backwards: read trailing lines from previous chunk's region
            if i > 0 {
                let prev_idx = indices[i - 1];
                let prev_end_byte = chunks[prev_idx].line_end;
                let cur_start_byte = chunks[ci].line_start;

                if prev_end_byte < cur_start_byte {
                    let prev_end_line = byte_to_line(prev_end_byte);
                    let cur_start_line = byte_to_line(cur_start_byte);

                    // Read context lines that fall between previous chunk and current chunk's start,
                    // but don't exceed the previous chunk's own region.
                    let safe_end = (prev_end_line + overlap).min(cur_start_line);
                    let context_lines: Vec<&str> = (prev_end_line..safe_end)
                        .filter(|&l| l < total_lines)
                        .map(|l| lines[l])
                        .collect();

                    if !context_lines.is_empty() {
                        // Insert a separator line, then prepend context
                        let ctx = context_lines.join("\n");
                        chunk.text = format!("{}\n---\n{}", ctx, chunk.text);
                        chunk.line_start = prev_end_byte;
                    }
                }
            }

            // Extend END forwards: read leading lines from next chunk's region
            if i + 1 < n {
                let ni = indices[i + 1];
                let cur_end_byte = chunks[ci].line_end;
                let next_start_byte = chunks[ni].line_start;

                if cur_end_byte < next_start_byte {
                    let cur_end_line = byte_to_line(cur_end_byte);
                    let next_start_line = byte_to_line(next_start_byte);

                    // Take lines from current chunk's end up to min(current_end + overlap, next_chunk_start)
                    let safe_end = (cur_end_line + overlap).min(next_start_line);

                    let appended: String = (cur_end_line..safe_end)
                        .filter(|&l| l < total_lines)
                        .map(|l| lines[l])
                        .collect::<Vec<_>>()
                        .join("\n");

                    if !appended.is_empty() {
                        chunk.text = format!("{}\n---\n{}", chunk.text, appended);
                    }

                    // Update line_end to reflect the extension
                    let ext_lines: usize = (cur_end_line..safe_end)
                        .filter(|&l| l < total_lines)
                        .count();
                    if ext_lines > 0 {
                        chunk.line_end += ext_lines;
                    }
                }
            }

            chunks[ci] = chunk;
        }
    }
}



pub fn extract_workspace_members(cargo: &toml::Value, root: &Path) -> Vec<PathBuf> {
    let mut raw_paths = Vec::new();

    if let Some(workspace) = cargo.get("workspace") {
        if let Some(members) = workspace.get("members").and_then(|v| v.as_array()) {
            for member in members {
                if let Some(name) = member.as_str() {
                    raw_paths.push(name.to_string());
                }
            }
        }
    }

    // Expand glob patterns (e.g., "crates/*") and resolve to absolute paths
    let mut paths: Vec<PathBuf> = Vec::new();
    for pattern in &raw_paths {
        let full_pattern = root.join(pattern);
        if let Ok(paths_found) = glob::glob(&full_pattern.to_string_lossy()) {
            for entry in paths_found.filter_map(|e| e.ok()) {
                if entry.is_dir() {
                    paths.push(entry.canonicalize().unwrap_or(entry));
                }
            }
        } else {
            // Not a glob, add as-is (will be joined with root later)
            paths.push(PathBuf::from(pattern));
        }
    }

    if paths.is_empty() && !raw_paths.is_empty() {
        // Glob didn't match anything, use raw paths joined to root
        for p in &raw_paths {
            paths.push(root.join(p));
        }
    } else if paths.is_empty() {
        paths.push(root.to_path_buf());
    }

    paths
}

#[tracing::instrument(level = "debug", skip(chunks), fields(dir = %dir.display()))]
fn collect_rs_files(dir: &Path, chunks: &mut Vec<Chunk>) -> Result<(), RagCoreError> {
    for entry in walkdir::WalkDir::new(dir)
        .min_depth(1)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() || path.extension() != Some("rs".as_ref()) {
            continue;
        }

        let content = std::fs::read_to_string(path).map_err(|e| {
            RagCoreError::FileRead(
                path.to_path_buf(),
                Box::new(std::io::Error::other(format!("reading file: {}", e))),
            )
        })?;

        parse_and_extract(&content, path, chunks)?;
    }

    Ok(())
}

#[tracing::instrument(level = "debug", skip(chunks), fields(file = %file_path.display(), content_len = content.len()))]
pub fn parse_and_extract(content: &str, file_path: &Path, chunks: &mut Vec<Chunk>) -> Result<(), RagCoreError> {
    let mut parser = tree_sitter::Parser::new();
    if let Err(e) = parser.set_language(&tree_sitter_rust::LANGUAGE.into()) {
        return Err(RagCoreError::ParseError(
            file_path.to_path_buf(),
            Box::new(std::io::Error::other(format!("setting language: {}", e))),
        ));
    }

    let tree = parser.parse(content, None).ok_or_else(|| {
        RagCoreError::ParseError(
            file_path.to_path_buf(),
            Box::new(std::io::Error::other("tree-sitter parse failed (unreachable)")),
        )
    })?;

    extract_nodes(tree.root_node(), content, file_path, "", chunks);

    Ok(())
}

fn extract_nodes(
    node: tree_sitter::Node<'_>,
    content: &str,
    file_path: &Path,
    module_prefix: &str,
    chunks: &mut Vec<Chunk>,
) {
    match node.kind() {
        "function_item" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/{}", module_prefix, name),
                symbol_kind: SymbolKind::Function,
                text: content[node.start_byte()..node.end_byte()].to_string(),
            });
        }
        "impl_item" => {
            let self_ty = node
                .child_by_field_name("self_type")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<unknown>"))
                .unwrap_or("<unknown>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/impl {}", module_prefix, self_ty),
                symbol_kind: SymbolKind::ImplBlock,
                text: content[node.start_byte()..node.end_byte()].to_string(),
            });
        }
        "unsafe_block" => {
            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/unsafe", module_prefix),
                symbol_kind: SymbolKind::UnsafeRegion,
                text: content[node.start_byte()..node.end_byte()].to_string(),
            });
        }
        "mod_item" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            let new_prefix = if module_prefix.is_empty() || module_prefix == "<root>" {
                name.clone()
            } else {
                format!("{}/{}", module_prefix, name)
            };

            for child in node.children(&mut node.walk()) {
                extract_nodes(child, content, file_path, &new_prefix, chunks);
            }
        }
        _ => {
            for child in node.children(&mut node.walk()) {
                extract_nodes(child, content, file_path, module_prefix, chunks);
            }
        }
    }
}
