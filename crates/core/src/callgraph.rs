use anyhow::Result;
use petgraph::graph::{Graph, NodeIndex};
use ra_ap_syntax::{ast::AstNode, SourceFile};
use std::collections::HashMap;

/// A node in the call graph — represents a symbol (function, method, etc.)
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Symbol {
    pub name: String,
    pub file_path: String,
}

/// Build a call graph from indexed chunks using ra_ap_syntax AST analysis.
/// For each function/impl chunk, parses its text with ra_ap_syntax and extracts
/// CallExpr nodes to find which functions are called within.
pub fn build_call_graph(
    chunks: &[crate::indexer::Chunk],
) -> Result<(Graph<Symbol, f32>, HashMap<String, NodeIndex>)> {
    let mut graph = Graph::<Symbol, f32>::new();
    let mut name_to_index: HashMap<String, NodeIndex> = HashMap::new();

    // Add all symbols as nodes
    for chunk in chunks {
        let node_key = format!(
            "{}:{}:{}:{}",
            file_stem(&chunk.file_path),
            chunk.line_start,
            chunk.module_name,
            chunk.symbol_kind_name()
        );
        if !name_to_index.contains_key(&node_key) {
            let idx = graph.add_node(Symbol {
                name: chunk.module_name.clone(),
                file_path: chunk.file_path.to_string_lossy().to_string(),
            });
            name_to_index.insert(node_key, idx);
        }
    }

    // Extract call edges using ra_ap_syntax AST analysis
    let edges = extract_call_edges(chunks)?;

    // Add edges based on resolved references
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

/// Extract call edges from chunks using ra_ap_syntax AST parsing.
fn extract_call_edges(chunks: &[crate::indexer::Chunk]) -> Result<HashMap<String, Vec<String>>> {
    let mut edges = HashMap::<String, Vec<String>>::new();

    for chunk in chunks {
        // Only analyze function bodies and impl blocks
        if !matches!(
            chunk.symbol_kind,
            crate::indexer::SymbolKind::Function | crate::indexer::SymbolKind::ImplBlock
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

        // Parse the chunk text with ra_ap_syntax and extract call names
        let call_names = parse_call_exprs(&chunk.text);

        if !call_names.is_empty() {
            edges.insert(key, call_names);
        }
    }

    Ok(edges)
}

/// Parse Rust source text using ra_ap_syntax and collect callee names from CallExpr nodes.
fn parse_call_exprs(text: &str) -> Vec<String> {
    let parsed = SourceFile::parse(text);
    let root = parsed.tree();

    // Walk the AST tree to find all call expressions
    let mut callees: Vec<String> = Vec::new();
    for node in root.syntax().descendants() {
        if let Some(call_expr) = ra_ap_syntax::ast::CallExpr::cast(node.clone()) {
            // CallExpr.expr() returns Option<Expr> — the expression being called
            if let Some(expr) = call_expr.expr() {
                // Expr implements Display — convert to string for callee name
                let name = expr.to_string();
                // Skip trivial calls like `true`, `false`, `self`, `Self`
                if !is_trivial_call(&name) && !callees.contains(&name) {
                    callees.push(name);
                }
            }
        }
    }

    callees
}

/// Check if a call expression name is trivial (not a real function call).
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
