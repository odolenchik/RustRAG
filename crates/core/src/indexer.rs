use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// Apply configured overlap to chunks within each file.
/// Expands boundaries by including adjacent lines from neighboring chunks,
/// so context at chunk boundaries is preserved for the LLM.
fn apply_overlap(chunks: &mut Vec<Chunk>) {
    let overlap = crate::config::Config::find()
        .ok()
        .map(|c| c.embedding.chunk_overlap)
        .unwrap_or(0);

    if overlap == 0 || chunks.is_empty() {
        return;
    }

    // Group indices by file path
    let mut file_indices: std::collections::HashMap<PathBuf, Vec<usize>> = std::collections::HashMap::new();
    for idx in 0..chunks.len() {
        file_indices.entry(chunks[idx].file_path.clone()).or_default().push(idx);
    }

    for (_path, mut indices) in file_indices {
        if indices.len() <= 1 {
            continue;
        }

        // Sort by line_start to ensure correct neighbor relationships
        indices.sort_by_key(|&i| chunks[i].line_start);

        let n = indices.len();
        for i in 0..n {
            let ci = indices[i];

            // Extend START backwards: read trailing lines from previous chunk's region
            if i > 0 {
                let prev_idx = indices[i - 1];
                let prev_end_line = chunks[prev_idx].line_end;
                let cur_start_line = chunks[ci].line_start;

                if prev_end_line < cur_start_line {
                    if let Ok(content) = std::fs::read_to_string(&chunks[ci].file_path) {
                        let all_lines: Vec<&str> = content.lines().collect();
                        let end = (prev_end_line + overlap).min(cur_start_line);

                        // Read context lines that fall between previous chunk and current chunk's start,
                        // but don't exceed the previous chunk's own region.
                        // We take lines from prev_end_line up to min(prev_end_line + overlap, cur_start_line)
                        let safe_end = (prev_end_line + overlap).min(cur_start_line);
                        let context_lines: Vec<String> = (prev_end_line..safe_end)
                            .filter(|&l| l < all_lines.len())
                            .map(|l| all_lines[l].to_string())
                            .collect();

                        if !context_lines.is_empty() {
                            // Insert a separator line, then prepend context
                            let ctx = context_lines.join("\n");
                            chunks[ci].text = format!("{}\n---\n{}", ctx, chunks[ci].text);
                            chunks[ci].line_start = prev_end_line;

                            // Also update the embedding cache key — text has changed now
                        }
                    }
                }
            }

            // Extend END forwards: read leading lines from next chunk's region
            if i + 1 < n {
                let ni = indices[i + 1];
                let cur_end_line = chunks[ci].line_end;
                let next_start_line = chunks[ni].line_start;

                if cur_end_line < next_start_line {
                    if let Ok(content) = std::fs::read_to_string(&chunks[ci].file_path) {
                        let all_lines: Vec<&str> = content.lines().collect();
                        // Take lines from current chunk's end up to min(current_end + overlap, next_chunk_start)
                        let safe_end = (cur_end_line + overlap).min(next_start_line);

                        let appended: String = (cur_end_line..safe_end)
                            .filter(|&l| l < all_lines.len())
                            .map(|l| all_lines[l].to_string())
                            .collect::<Vec<_>>()
                            .join("\n");

                        if !appended.is_empty() {
                            chunks[ci].text = format!("{}\n---\n{}", chunks[ci].text, appended);
                        }

                        // Update line_end to reflect the extension
                        let ext_lines: usize = (cur_end_line..safe_end).filter(|&l| l < all_lines.len()).count();
                        if ext_lines > 0 {
                            chunks[ci].line_end += ext_lines;
                        }
                    }
                }
            }
        }
    }
}

/// Walk a Cargo workspace directory and extract all code chunks.
pub fn index_workspace(root: &Path) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();

    let manifest = root.join("Cargo.toml");
    if !manifest.exists() {
        return Err(anyhow::anyhow!(
            "No Cargo.toml found at {}",
            root.display()
        ));
    }

    let cargo_content = std::fs::read_to_string(&manifest)?;
    let cargo_toml: toml::Value = cargo_content.parse()?;

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

    Ok(chunks)
}

fn extract_workspace_members(cargo: &toml::Value, root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Some(workspace) = cargo.get("workspace") {
        if let Some(members) = workspace.get("members").and_then(|v| v.as_array()) {
            for member in members {
                if let Some(name) = member.as_str() {
                    paths.push(PathBuf::from(name));
                }
            }
        }
    }

    if paths.is_empty() {
        paths.push(root.to_path_buf());
    }

    paths
}

fn collect_rs_files(dir: &Path, chunks: &mut Vec<Chunk>) -> Result<()> {
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

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        parse_and_extract(&content, path, chunks)?;
    }

    Ok(())
}

fn parse_and_extract(content: &str, file_path: &Path, chunks: &mut Vec<Chunk>) -> Result<()> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .context("Failed to set tree-sitter-rust language")?;

    let tree = parser
        .parse(content, None)
        .context("Failed to parse source file with tree-sitter")?;

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
            let name = node.child_by_field_name("name")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<anon>"))
                .unwrap_or("<anon>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte() as usize,
                line_end: node.end_byte() as usize,
                module_name: format!("{}/{}", module_prefix, name),
                symbol_kind: SymbolKind::Function,
                text: content[node.start_byte()..node.end_byte()].to_string(),
            });
        }
        "impl_item" => {
            let self_ty = node.child_by_field_name("self_type")
                .map(|n| n.utf8_text(content.as_bytes()).unwrap_or("<unknown>"))
                .unwrap_or("<unknown>")
                .to_string();

            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte() as usize,
                line_end: node.end_byte() as usize,
                module_name: format!("{}/impl {}", module_prefix, self_ty),
                symbol_kind: SymbolKind::ImplBlock,
                text: content[node.start_byte()..node.end_byte()].to_string(),
            });
        }
        "unsafe_block" => {
            chunks.push(Chunk {
                file_path: file_path.to_path_buf(),
                line_start: node.start_byte() as usize,
                line_end: node.end_byte() as usize,
                module_name: format!("{}/unsafe", module_prefix),
                symbol_kind: SymbolKind::UnsafeRegion,
                text: content[node.start_byte()..node.end_byte()].to_string(),
            });
        }
        "mod_item" => {
            let name = node.child_by_field_name("name")
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
