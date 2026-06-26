//! AST-based source-code indexing for Rust workspaces.
//!
//! This crate parses Rust source files with tree-sitter, extracts AST-level chunks
//! (functions, impl blocks, unsafe regions, etc.), and applies configurable overlap
//! to preserve context at chunk boundaries.

#![warn(missing_docs)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A unit of code to be embedded — typically a function, impl block, or unsafe region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    /// File path where this chunk originates.
    pub file_path: PathBuf,
    /// Start byte offset (used as line_start in practice).
    pub line_start: usize,
    /// End byte offset (used as line_end in practice).
    pub line_end: usize,
    /// Qualified module name for this symbol.
    pub module_name: String,
    /// Kind of AST node extracted.
    pub symbol_kind: SymbolKind,
    /// Source text of the chunk.
    pub text: String,
    /// Maximum AST nesting depth within this chunk (used for quality diagnostics).
    #[serde(default)]
    pub max_nesting_depth: Option<usize>,
}

impl Chunk {
    /// Create a new chunk without nesting-depth tracking.
    pub fn new(
        file_path: PathBuf,
        line_start: usize,
        line_end: usize,
        module_name: String,
        symbol_kind: SymbolKind,
        text: String,
    ) -> Self {
        Self {
            file_path,
            line_start,
            line_end,
            module_name,
            symbol_kind,
            text,
            max_nesting_depth: None,
        }
    }

    /// Return the human-readable name for this chunk's symbol kind.
    pub fn symbol_kind_name(&self) -> &'static str {
        self.symbol_kind.as_str()
    }
}

/// The kind of AST node that was extracted as a chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    /// Free function or method.
    Function,
    /// Impl block for a type.
    ImplBlock,
    /// Unsafe region.
    UnsafeRegion,
    /// Trait definition.
    Trait,
    /// Trait implementation.
    TraitImpl,
    /// Module item.
    Module,
    /// Struct definition.
    Struct,
    /// Union definition.
    Union,
    /// Enum definition.
    Enum,
    /// Macro definition.
    Macro,
    /// Constant item.
    Constant,
    /// Static item.
    Static,
}

impl SymbolKind {
    /// Returns the lowercase string name used in JSONL storage.
    pub const fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::ImplBlock => "implblock",
            SymbolKind::UnsafeRegion => "unsaferegion",
            SymbolKind::Trait => "trait",
            SymbolKind::TraitImpl => "traitimpl",
            SymbolKind::Module => "module",
            SymbolKind::Struct => "struct",
            SymbolKind::Union => "union",
            SymbolKind::Enum => "enum",
            SymbolKind::Macro => "macro",
            SymbolKind::Constant => "constant",
            SymbolKind::Static => "static",
        }
    }
}

/// Re-export the error type from core for convenience.
pub use rust_rag_error::RagCoreError;

// ---------------------------------------------------------------------------
// Walk a Cargo workspace directory and extract all code chunks.
// ---------------------------------------------------------------------------

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

    apply_overlap(&mut chunks);

    tracing::info!(chunks = chunks.len(), "indexing complete");

    Ok(chunks)
}

/// Extract workspace member paths from a Cargo.toml toml::Value.
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
            paths.push(PathBuf::from(pattern));
        }
    }

    if paths.is_empty() && !raw_paths.is_empty() {
        for p in &raw_paths {
            paths.push(root.join(p));
        }
    } else if paths.is_empty() {
        paths.push(root.to_path_buf());
    }

    paths
}

/// Collect Rust source files from a directory and parse them into chunks.
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

/// Parse a Rust source file and extract AST-level chunks.
#[tracing::instrument(level = "debug", skip(chunks), fields(file = %file_path.display(), content_len = content.len()))]
pub fn parse_and_extract(
    content: &str,
    file_path: &Path,
    chunks: &mut Vec<Chunk>,
) -> Result<(), RagCoreError> {
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
            Box::new(std::io::Error::other(
                "tree-sitter parse failed (unreachable)",
            )),
        )
    })?;

    extract_nodes(tree.root_node(), content, file_path, "", chunks);

    Ok(())
}

/// Apply configured overlap to chunks within each file.
#[tracing::instrument(level = "debug", skip(chunks), fields(chunk_count = chunks.len()))]
pub fn apply_overlap(chunks: &mut [Chunk]) {
    let overlap = rust_rag_config::Config::find()
        .ok()
        .map(|c| c.embedding.chunk_overlap)
        .unwrap_or(0);

    if overlap == 0 || chunks.is_empty() {
        return;
    }

    // Group indices by file path
    let mut file_indices: HashMap<PathBuf, Vec<usize>> = HashMap::new();
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

        indices.sort_by_key(|&i| chunks[i].line_start);

        let n = indices.len();

        let mut content_cache: HashMap<PathBuf, Option<String>> = Default::default();

        for i in 0..n {
            let ci = indices[i];
            let file_path = chunks[ci].file_path.clone();

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

            let mut byte_offsets: Vec<usize> = Vec::with_capacity(total_lines + 1);
            let mut offset = 0;
            byte_offsets.push(0);
            for line in &lines {
                offset += line.len() + 1;
                byte_offsets.push(offset);
            }

            let byte_to_line = |byte_offset: usize| -> usize {
                if byte_offset == 0 {
                    return 0;
                }
                byte_offsets
                    .partition_point(|&bo| bo <= byte_offset)
                    .saturating_sub(1)
            };

            let mut chunk = chunks[ci].clone();

            // Extend START backwards
            if i > 0 {
                let prev_idx = indices[i - 1];
                let prev_end_byte = chunks[prev_idx].line_end;
                let cur_start_byte = chunks[ci].line_start;

                if prev_end_byte < cur_start_byte {
                    let prev_end_line = byte_to_line(prev_end_byte);
                    let cur_start_line = byte_to_line(cur_start_byte);

                    let safe_end = (prev_end_line + overlap).min(cur_start_line);
                    let context_lines: Vec<&str> = (prev_end_line..safe_end)
                        .filter(|&l| l < total_lines)
                        .map(|l| lines[l])
                        .collect();

                    if !context_lines.is_empty() {
                        let ctx = context_lines.join("\n");
                        chunk.text = format!("{}\n---\n{}", ctx, chunk.text);
                        chunk.line_start = prev_end_byte;
                    }
                }
            }

            // Extend END forwards
            if i + 1 < n {
                let ni = indices[i + 1];
                let cur_end_byte = chunks[ci].line_end;
                let next_start_byte = chunks[ni].line_start;

                if cur_end_byte < next_start_byte {
                    let cur_end_line = byte_to_line(cur_end_byte);
                    let next_start_line = byte_to_line(next_start_byte);

                    let safe_end = (cur_end_line + overlap).min(next_start_line);

                    let appended: String = (cur_end_line..safe_end)
                        .filter(|&l| l < total_lines)
                        .map(|l| lines[l])
                        .collect::<Vec<_>>()
                        .join("\n");

                    if !appended.is_empty() {
                        chunk.text = format!("{}\n---\n{}", chunk.text, appended);
                    }

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

// ---------------------------------------------------------------------------
// Internal AST extraction helpers (tree-sitter)
// ---------------------------------------------------------------------------

fn compute_nesting_depth(node: tree_sitter::Node<'_>) -> usize {
    let kind = node.kind();

    match kind {
        "block" | "function_item" | "impl_item" | "mod_item" | "unsafe_block"
        | "struct_expression" | "tuple_struct" | "for_statement" | "while_statement"
        | "loop_statement" => {
            let mut max_child = 0;
            for child in node.children(&mut node.walk()) {
                let d = compute_nesting_depth(child);
                if d > max_child {
                    max_child = d;
                }
            }
            1 + max_child
        }
        _ => {
            let mut max_child = 0;
            for child in node.children(&mut node.walk()) {
                let d = compute_nesting_depth(child);
                if d > max_child {
                    max_child = d;
                }
            }
            max_child
        }
    }
}

fn is_atomic_kind(kind: &str) -> bool {
    matches!(
        kind,
        "macro_invocation"
            | "use_declaration"
            | "use_tree"
            | "extern_crate_declaration"
            | "attribute"
    )
}

fn extract_nodes(
    node: tree_sitter::Node<'_>,
    content: &str,
    file_path: &Path,
    module_prefix: &str,
    chunks: &mut Vec<Chunk>,
) {
    let kind = node.kind();

    if kind == "macro_definition" {
        let name = node
            .child_by_field_name("name")
            .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
            .unwrap_or("<anon>")
            .to_string();

        chunks.push(Chunk {
            file_path: file_path.to_path_buf(),
            line_start: node.start_byte(),
            line_end: node.end_byte(),
            module_name: format!("{}/macro {}", module_prefix, name),
            symbol_kind: SymbolKind::Macro,
            text: content[node.start_byte()..node.end_byte()].to_string(),
            max_nesting_depth: Some(compute_nesting_depth(node)),
        });
        return;
    }

    if is_atomic_kind(kind) {
        return;
    }

    match kind {
        "function_item" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            if node.parent().is_some_and(|p| p.kind() == "impl_item") {
                return;
            }

            let max_depth = compute_nesting_depth(node);

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/{}", module_prefix, name),
                symbol_kind: SymbolKind::Function,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(max_depth),
            });
        }
        "impl_item" => {
            let self_ty = node
                .child_by_field_name("self_type")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<unknown>"))
                .unwrap_or("<unknown>")
                .to_string();

            let symbol_kind = if kind.contains("trait") {
                SymbolKind::TraitImpl
            } else {
                SymbolKind::ImplBlock
            };

            let module_name = format!("{}/impl {}", module_prefix, self_ty);
            let text = content[node.start_byte()..node.end_byte()].to_string();
            let max_depth = compute_nesting_depth(node) + 1;

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name,
                symbol_kind,
                text,
                max_nesting_depth: Some(max_depth),
            });
        }
        "unsafe_block" => {
            let max_depth = compute_nesting_depth(node);

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/unsafe", module_prefix),
                symbol_kind: SymbolKind::UnsafeRegion,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(max_depth),
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
        "struct_item" => {
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
                symbol_kind: SymbolKind::Struct,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(compute_nesting_depth(node)),
            });
        }
        "enum_item" => {
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
                symbol_kind: SymbolKind::Enum,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(compute_nesting_depth(node)),
            });
        }
        "trait_item" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/trait {}", module_prefix, name),
                symbol_kind: SymbolKind::Trait,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(compute_nesting_depth(node)),
            });
        }
        "union_item" => {
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
                symbol_kind: SymbolKind::Union,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(compute_nesting_depth(node)),
            });
        }
        "constant_item" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/constant {}", module_prefix, name),
                symbol_kind: SymbolKind::Constant,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(compute_nesting_depth(node)),
            });
        }
        "static_item" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte(),
                line_end: node.end_byte(),
                module_name: format!("{}/static {}", module_prefix, name),
                symbol_kind: SymbolKind::Static,
                text: content[node.start_byte()..node.end_byte()].to_string(),
                max_nesting_depth: Some(compute_nesting_depth(node)),
            });
        }
        _ => {
            for child in node.children(&mut node.walk()) {
                extract_nodes(child, content, file_path, module_prefix, chunks);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path();

        std::fs::write(
            path.join("Cargo.toml"),
            r#"[package]
name = "test_pkg"
version = "0.1.0"
edition = "2021"

[dependencies]"#,
        )
        .unwrap();

        std::fs::create_dir_all(path.join("src")).unwrap();
        std::fs::write(
            path.join("src/lib.rs"),
            r#"pub fn hello_world() -> &'static str {
    "hello"
}

pub struct Counter {
    count: u32,
}

impl Counter {
    pub fn new() -> Self {
        Counter { count: 0 }
    }

    pub fn increment(&mut self) {
        self.count += 1;
    }
}"#,
        )
        .unwrap();

        dir
    }

    #[test]
    fn test_index_workspace_finds_chunks() {
        let dir = make_test_workspace();
        let chunks = index_workspace(dir.path()).expect("should index");
        assert!(!chunks.is_empty(), "Should find at least one chunk");

        let has_function = chunks.iter().any(|c| c.symbol_kind == SymbolKind::Function);
        let has_impl = chunks
            .iter()
            .any(|c| c.symbol_kind == SymbolKind::ImplBlock);
        assert!(has_function, "Should extract function chunk");
        assert!(has_impl, "Should extract impl block chunk");

        for chunk in &chunks {
            assert!(!chunk.text.is_empty(), "Chunk text should not be empty");
            assert!(
                !chunk.module_name.is_empty(),
                "Module name should not be empty"
            );
            assert!(chunk.line_start <= chunk.line_end, "line_start <= line_end");
        }
    }

    #[test]
    fn test_index_workspace_missing_cargo_toml() {
        let dir = tempfile::tempdir().unwrap();
        let result = index_workspace(dir.path());
        assert!(result.is_err(), "Should fail without Cargo.toml");
    }
}
