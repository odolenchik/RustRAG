//! Call graph construction from indexed chunks using ra_ap_syntax AST analysis.

#![warn(missing_docs)]

use petgraph::graph::{Graph, NodeIndex};
use ra_ap_syntax::{ast::AstNode, SourceFile};
use rust_rag_indexer::Chunk;
use std::collections::HashMap;

pub use rust_rag_error::RagCoreError;

/// Alias for the call graph return type — a graph of [`Symbol`] nodes with float-weighted edges and a name-to-index map.
type CallGraphResult = (Graph<Symbol, f32>, HashMap<String, NodeIndex>);

/// A node in the call graph — represents a symbol (function, method, etc.)
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Symbol {
    /// Display name of the symbol.
    pub name: String,
    /// Source file path where this symbol is defined.
    pub file_path: String,
}

/// Build a call graph from indexed chunks using ra_ap_syntax AST analysis.
pub fn build_call_graph(
    chunks: &[Chunk],
) -> Result<CallGraphResult, RagCoreError> {
    let mut graph = Graph::<Symbol, f32>::new();
    let mut name_to_index: HashMap<String, NodeIndex> = HashMap::new();

    for chunk in chunks {
        let node_key = format!(
            "{}:{}:{}:{}",
            file_stem(&chunk.file_path),
            chunk.line_start,
            chunk.module_name,
            chunk.symbol_kind_name()
        );
        name_to_index.entry(node_key).or_insert_with(|| {
            graph.add_node(Symbol {
                name: chunk.module_name.clone(),
                file_path: chunk.file_path.to_string_lossy().to_string(),
            })
        });
    }

    let edges = extract_call_edges(chunks)?;

    for (from_key, to_keys) in &edges {
        if let Some(from_idx) = name_to_index.get(from_key) {
            for to_key in to_keys {
                if let Some(to_idx) = name_to_index.get(to_key) {
                    graph.add_edge(*from_idx, *to_idx, 1.0);
                }
            }
        }
    }

    Ok((graph, name_to_index))
}

fn extract_call_edges(chunks: &[Chunk]) -> Result<HashMap<String, Vec<String>>, RagCoreError> {
    let mut edges = HashMap::<String, Vec<String>>::new();

    for chunk in chunks {
        if !matches!(
            chunk.symbol_kind,
            rust_rag_indexer::SymbolKind::Function | rust_rag_indexer::SymbolKind::ImplBlock
        ) {
            continue;
        }

        let key = format!(
            "{}:{}:{}:{}",
            file_stem(&chunk.file_path),
            chunk.line_start,
            chunk.module_name,
            chunk.symbol_kind_name()
        );

        let call_names = parse_call_exprs(&chunk.text);

        if !call_names.is_empty() {
            edges.insert(key, call_names);
        }
    }

    Ok(edges)
}

fn parse_call_exprs(text: &str) -> Vec<String> {
    let parsed = SourceFile::parse(text);
    let root = parsed.tree();

    let mut seen: std::collections::HashSet<String> = Default::default();
    let mut callees: Vec<String> = Vec::new();
    for node in root.syntax().descendants() {
        if let Some(call_expr) = ra_ap_syntax::ast::CallExpr::cast(node.clone()) {
            if let Some(expr) = call_expr.expr() {
                let name = expr.to_string();
                if !is_trivial_call(&name) && seen.insert(name.clone()) {
                    callees.push(name);
                }
            }
        }
    }

    callees
}

fn is_trivial_call(name: &str) -> bool {
    let name = name.trim();
    matches!(
        name,
        "true" | "false" | "self" | "Self" | "super" | "_" | ""
    )
}

fn file_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_trivial_call() {
        assert!(is_trivial_call("true"));
        assert!(is_trivial_call("false"));
        assert!(is_trivial_call("self"));
        assert!(is_trivial_call("Self"));
        assert!(is_trivial_call("super"));
        assert!(!is_trivial_call("foo"));
    }

    #[test]
    fn test_file_stem() {
        let path = std::path::Path::new("/some/path/myfile.rs");
        assert_eq!(file_stem(path), "myfile");
    }
}
