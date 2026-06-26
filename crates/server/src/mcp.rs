use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write as IoWrite};

/// MCP (Model Context Protocol) server over JSON-RPC 2.0 / stdio.
/// Implements: initialize handshake, notifications/initialized, tools/list, tools/call with JSON Schema validation.
/// Supports batch requests per JSON-RPC 2.0 spec.

// ---- JSON-RPC types -------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[allow(dead_code)] // deserialized but not used locally — MCP handles protocol versioning
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    id: Option<Value>,
}

#[derive(Serialize)]
pub struct JsonRpcResponse {
    jsonrpc: String,
    result: Option<Value>,
    error: Option<JsonRpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
}

#[derive(Serialize)]
pub struct JsonRpcError {
    code: i32,
    message: String,
}

// ---- MCP protocol helpers --------------------------------------------------

const MCP_VERSION: &str = "2024-11-05";

pub fn ok_response(id: Option<Value>, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(result),
        error: None,
        id,
    }
}

pub fn err_response(id: Option<Value>, code: i32, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
        }),
        id,
    }
}

// ---- MCP server state ------------------------------------------------------

/// Shared state for the MCP server (holds path to vector store).
pub struct McpState {
    pub store_path: std::path::PathBuf,
    initialized: std::sync::atomic::AtomicBool,
}

impl McpState {
    pub fn new(workspace_root: &std::path::Path) -> Self {
        let store_path = workspace_root.join(".rustrag");
        Self {
            store_path: store_path.canonicalize().unwrap_or(store_path),
            initialized: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn require_initialized(&self) -> Result<()> {
        if !self.initialized.load(std::sync::atomic::Ordering::SeqCst) {
            anyhow::bail!("Not initialized. Call 'initialize' first.");
        }
        Ok(())
    }
}

// ---- MCP methods -----------------------------------------------------------

fn handle_tools_list(state: &McpState) -> Result<Value> {
    state.require_initialized()?;
    Ok(serde_json::json!({
        "tools": [
            {
                "name": "rag_search",
                "description": "Search for code chunks by semantic similarity. Returns raw relevant snippets without LLM generation. Supports both single and batch queries.",
                "inputSchema": {
                    "type": "object",
                    "description": "Search parameters - either 'query' (single) or 'queries' (batch) must be provided",
                    "oneOf": [
                        {
                            "type": "object",
                            "properties": {
                                "query": { "type": "string", "maxLength": 4096, "description": "The search query" },
                                "top_k": { "type": "integer", "description": "Number of results to return per query (default 5)", "minimum": 1, "maximum": 100 },
                                "filters": {
                                    "type": "object",
                                    "description": "Optional filters to refine search results",
                                    "properties": {
                                        "file_extension": { "type": "string", "description": "Filter by file extension (e.g., \"rs\")" },
                                        "symbol_kind": { "type": "string", "description": "Filter by symbol kind (e.g., \"function\", \"struct\")" },
                                        "crates": {
                                            "type": "array",
                                            "description": "Filter by crate names",
                                            "items": { "type": "string" }
                                        }
                                    },
                                    "additionalProperties": false
                                }
                            },
                            "required": ["query"]
                        },
                        {
                            "type": "object",
                            "properties": {
                                "queries": {
                                    "type": "array",
                                    "description": "Multiple search queries to execute in batch",
                                    "items": {
                                        "type": "string",
                                        "maxLength": 4096
                                    },
                                    "minItems": 1,
                                    "maxItems": 10
                                },
                                "top_k": { "type": "integer", "description": "Number of results to return per query (default 5)", "minimum": 1, "maximum": 100 },
                                "filters": {
                                    "type": "object",
                                    "description": "Optional filters to refine search results (applied to all queries)",
                                    "properties": {
                                        "file_extension": { "type": "string", "description": "Filter by file extension (e.g., \"rs\")" },
                                        "symbol_kind": { "type": "string", "description": "Filter by symbol kind (e.g., \"function\", \"struct\")" },
                                        "crates": {
                                            "type": "array",
                                            "description": "Filter by crate names",
                                            "items": { "type": "string" }
                                        }
                                    },
                                    "additionalProperties": false
                                }
                            },
                            "required": ["queries"]
                        }
                    ]
                }
            },

            {
                "name": "rag_workspace_info",
                "description": "Get structured information about the workspace: list of all crates, their paths, dependencies from Cargo.toml, and README.md content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "detail_level": {
                            "type": "string",
                            "enum": ["summary", "full"],
                            "description": "Level of detail: 'summary' for crate names and paths, 'full' also includes dependencies from Cargo.toml"
                        }
                    },
                    "required": []
                }
            },
            {
                "name": "rag_file_read",
                "description": "Read the full content of a file within the workspace. Path is relative to the workspace root. Optionally specify line_start and line_end to read a specific range.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Relative path to the file from workspace root (e.g., 'crates/server/src/lib.rs')" },
                        "line_start": { "type": "integer", "description": "Starting line number (1-indexed, inclusive)", "minimum": 1 },
                        "line_end": { "type": "integer", "description": "Ending line number (1-indexed, inclusive)", "minimum": 1 }
                    },
                    "required": ["file_path"]
                }
            }
        ]
    }))
}

async fn handle_tools_call(params: Value, store_path: &std::path::Path) -> Result<Value> {
    let call_params: ToolCallParams = serde_json::from_value(params)?;

    match call_params.name.as_str() {
        "rag_search" => rag_search_tool(&call_params.arguments, store_path),
        "rag_workspace_info" => Ok(rag_workspace_info_tool(&call_params.arguments)),
        "rag_file_read" => Ok(rag_file_read_tool(&call_params.arguments)?),
        _ => anyhow::bail!("Unknown tool: {}", call_params.name),
    }
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

// ---- JSON Schema validation ------------------------------------------------

fn validate_tool_input(schema: &Value, args: &Value) -> Result<(), String> {
    let props = schema
        .get("properties")
        .ok_or("Missing properties in schema")?;
    let required_fields: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    // Check required fields.
    for field in &required_fields {
        let has_field = args
            .as_object()
            .map(|o| o.contains_key(*field))
            .unwrap_or(false);
        if !has_field {
            return Err(format!("Missing required field: {}", field));
        }
    }

    // Args must be an object.
    if !args.is_object() {
        return Err("Arguments must be a JSON object".to_string());
    }

    // Validate types for provided fields.
    if let Some(obj) = args.as_object() {
        for (key, value) in obj.iter() {
            if let Some(prop_schema) = props.get(key) {
                let expected_type = prop_schema.get("type").and_then(|t| t.as_str());
                if let Some(expected) = expected_type {
                    match expected {
                        "string" => {
                            if !value.is_string() {
                                return Err(format!(
                                    "Field '{}' expected string, got {}",
                                    key,
                                    type_name(value)
                                ));
                            }
                        }
                        "integer" | "number" => {
                            if !value.is_number() {
                                return Err(format!(
                                    "Field '{}' expected number, got {}",
                                    key,
                                    type_name(value)
                                ));
                            }
                        }
                        "boolean" => {
                            if !value.is_boolean() {
                                return Err(format!(
                                    "Field '{}' expected boolean, got {}",
                                    key,
                                    type_name(value)
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

fn type_name(v: &Value) -> &'static str {
    if v.is_string() {
        "string"
    } else if v.is_array() {
        "array"
    } else if v.is_object() {
        "object"
    } else if v.is_boolean() {
        "boolean"
    } else if v.is_number() {
        "number"
    } else {
        "unknown"
    }
}

// ---- Tool implementations --------------------------------------------------

fn rag_search_tool(args: &Value, store_path: &std::path::Path) -> Result<Value> {
    // Define schema for single query (backward compatibility)
    let single_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "maxLength": 4096 },
            "top_k": { "type": "integer", "minimum": 1, "maximum": 100 },
            "filters": {
                "type": "object",
                "properties": {
                    "file_extension": { "type": "string" },
                    "symbol_kind": { "type": "string" },
                    "crates": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "additionalProperties": false
            }
        },
        "required": ["query"]
    });

    // Define schema for batch queries
    let batch_schema = serde_json::json!({
        "type": "object",
        "properties": {
            "queries": {
                "type": "array",
                "items": {
                    "type": "string",
                    "maxLength": 4096
                },
                "minItems": 1,
                "maxItems": 10
            },
            "top_k": { "type": "integer", "minimum": 1, "maximum": 100 },
            "filters": {
                "type": "object",
                "properties": {
                    "file_extension": { "type": "string" },
                    "symbol_kind": { "type": "string" },
                    "crates": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "additionalProperties": false
            }
        },
        "required": ["queries"]
    });

    // Validate against either schema
    let is_single = args.get("query").is_some();
    let is_batch = args.get("queries").is_some();

    if !is_single && !is_batch {
        return Err(anyhow::anyhow!(
            "Either 'query' or 'queries' must be provided"
        ));
    }

    if is_single && is_batch {
        return Err(anyhow::anyhow!(
            "Provide either 'query' or 'queries', not both"
        ));
    }

    let schema = if is_single {
        &single_schema
    } else {
        &batch_schema
    };
    validate_tool_input(schema, args).map_err(|e| anyhow::anyhow!("Invalid arguments: {}", e))?;

    let top_k: usize = args["top_k"].as_u64().map(|n| n as usize).unwrap_or(5);
    if !(1..=100).contains(&top_k) {
        return Err(anyhow::anyhow!("top_k must be between 1 and 100"));
    }

    // Parse filters if provided
    let filters = if let Some(filters_val) = args.get("filters") {
        if !filters_val.is_object() {
            return Err(anyhow::anyhow!("filters must be an object"));
        }
        let file_extension = filters_val
            .get("file_extension")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let symbol_kind = filters_val
            .get("symbol_kind")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // crates is ignored for now, but we could validate it's an array if present
        if let Some(crates_val) = filters_val.get("crates") {
            if !crates_val.is_array() {
                return Err(anyhow::anyhow!("crates must be an array if provided"));
            }
            // We ignore crates filtering for now
        }
        Some(rust_rag_core::vector_store::SearchFilters {
            file_extension,
            symbol_kind,
        })
    } else {
        None
    };

    // Open vector store once
    let store = rust_rag_core::vector_store::VectorStore::open(store_path)?;

    if is_single {
        // Handle single query (backward compatibility)
        let query: String = args["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'query' argument"))?
            .to_string();
        if query.len() > 4096 {
            return Err(anyhow::anyhow!(
                "Query exceeds maximum length of 4096 characters (got {})",
                query.len()
            ));
        }

        let embedding = rust_rag_core::embedding::embed(&query)?;
        let results = store.hybrid_search(&embedding, &query, top_k, 0.7, filters.as_ref())?;

        // Convert results to structured JSON format
        let results_json: Value = serde_json::to_value(&results)
            .map_err(|e| anyhow::anyhow!("Failed to serialize search results: {}", e))?;

        Ok(serde_json::json!({
            "results": results_json,
            "query": query,
        }))
    } else {
        // Handle batch queries
        let queries_val = args["queries"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'queries' argument"))?;

        let mut queries = Vec::new();

        // Collect and validate all queries
        for (i, q_val) in queries_val.iter().enumerate() {
            let query_str = q_val
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Query at index {} is not a valid string", i))?
                .to_string();
            if query_str.len() > 4096 {
                return Err(anyhow::anyhow!(
                    "Query at index {} exceeds maximum length of 4096 characters (got {})",
                    i,
                    query_str.len()
                ));
            }
            queries.push(query_str);
        }

        // Batch compute embeddings for efficiency
        let query_refs: Vec<&str> = queries.iter().map(|s| s.as_str()).collect();
        let batch_embeddings = rust_rag_core::embedding::embed_batch(&query_refs)?;

        // Process each query
        let mut batch_results = Vec::new();
        for (i, (query, embedding)) in queries.into_iter().zip(batch_embeddings).enumerate() {
            let results = store.hybrid_search(&embedding, &query, top_k, 0.7, filters.as_ref())?;

            // Convert results to structured JSON format
            let results_json: Value = serde_json::to_value(&results).map_err(|e| {
                anyhow::anyhow!("Failed to serialize search results for query {}: {}", i, e)
            })?;

            batch_results.push(serde_json::json!({
                "query": query,
                "results": results_json,
            }));
        }

        Ok(serde_json::json!({
            "batch_results": batch_results,
        }))
    }
}

// ---- New tool: rag_workspace_info -------------------------------------------

fn rag_workspace_info_tool(args: &Value) -> Value {
    let detail_level: &str = args
        .get("detail_level")
        .and_then(|v| v.as_str())
        .unwrap_or("summary");
    let ws_root = match std::env::var("RUSRAG_WORKSPACE") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => std::env::current_dir().unwrap_or_default(),
    };

    let cargo_toml_path = ws_root.join("Cargo.toml");
    if !cargo_toml_path.exists() {
        return serde_json::json!({"workspace_root": ws_root.to_string_lossy().to_string(), "error": "No Cargo.toml found", "crates": []});
    }

    let cargo_content = match std::fs::read_to_string(&cargo_toml_path) {
        Ok(c) => c,
        Err(e) => {
            return serde_json::json!({"workspace_root": ws_root.to_string_lossy().to_string(), "error": format!("Failed to read Cargo.toml: {}", e), "crates": []})
        }
    };

    let cargo_value: toml::Value = match cargo_content.parse() {
        Ok(v) => v,
        Err(e) => {
            return serde_json::json!({"workspace_root": ws_root.to_string_lossy().to_string(), "error": format!("Failed to parse Cargo.toml: {}", e), "crates": []})
        }
    };

    let member_dirs: Vec<String> = if let Some(workspace) = cargo_value.get("workspace") {
        workspace
            .get("members")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![".".to_string()]
    };

    let mut crates_info: Vec<Value> = Vec::new();

    for member_pattern in &member_dirs {
        let full_pattern = ws_root.join(member_pattern);
        let matches = match glob::glob(&full_pattern.to_string_lossy()) {
            Ok(m) => m.filter_map(|e| e.ok()).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };

        for member_dir in &matches {
            if !member_dir.is_dir() || !member_dir.join("Cargo.toml").exists() {
                continue;
            }

            let content = match std::fs::read_to_string(member_dir.join("Cargo.toml")) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mv: toml::Value = match content.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };

            let mut ci: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
            ci.insert(
                "name".to_string(),
                Value::String(
                    mv.get("package")
                        .and_then(|p| p.get("name"))
                        .and_then(|n| n.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                ),
            );

            let rel_path = match member_dir.strip_prefix(&ws_root) {
                Ok(p) if p.as_os_str().is_empty() || p == std::path::Path::new(".") => {
                    ".".to_string()
                }
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => member_dir.to_string_lossy().to_string(),
            };
            ci.insert("path".to_string(), Value::String(rel_path));

            if detail_level == "full" {
                let deps: Vec<String> = mv
                    .get("dependencies")
                    .and_then(|d| d.as_table())
                    .map(|t| {
                        t.iter()
                            .filter(|(n, _)| !n.starts_with("rust-rag"))
                            .map(|(n, _)| n.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                ci.insert(
                    "dependencies".to_string(),
                    serde_json::to_value(deps).unwrap_or(Value::Null),
                );

                let readme_path = member_dir.join("README.md");
                if readme_path.exists() {
                    if let Ok(rc) = std::fs::read_to_string(&readme_path) {
                        ci.insert(
                            "readme".to_string(),
                            Value::String(if rc.chars().count() > 2000 {
                                format!(
                                    "{}\n... (truncated)",
                                    &rc.chars().take(2000).collect::<String>()
                                )
                            } else {
                                rc
                            }),
                        );
                    }
                }
            }

            crates_info.push(Value::Object(serde_json::Map::from_iter(ci)));
        }
    }

    // Check for implicit members (non-member dirs with Cargo.toml)
    if member_dirs.len() == 1 && member_dirs[0] == "." {
        let entries = match std::fs::read_dir(&ws_root) {
            Ok(e) => e.filter_map(|x| x.ok()).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };
        for entry in &entries {
            if !entry.path().is_dir() || entry.path() == ws_root {
                continue;
            }
            if entry.path().join("Cargo.toml").exists() {
                let covered = crates_info.iter().any(|c| {
                    c.get("path").and_then(|p| p.as_str())
                        == Some(
                            &entry
                                .path()
                                .strip_prefix(&ws_root)
                                .unwrap_or(&entry.path())
                                .to_string_lossy(),
                        )
                });
                if !covered {
                    crates_info.push(serde_json::json!({"name": entry.file_name().to_string_lossy(), "path": entry.path().strip_prefix(&ws_root).unwrap_or(&entry.path()).to_string_lossy()}));
                }
            }
        }
    }

    serde_json::json!({ "workspace_root": ws_root.to_string_lossy().to_string(), "crates": crates_info })
}

// ---- New tool: rag_file_read -------------------------------------------------

pub fn rag_file_read_tool(args: &Value) -> Result<Value> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "file_path": { "type": "string" },
            "line_start": { "type": "integer", "minimum": 1 },
            "line_end": { "type": "integer", "minimum": 1 }
        },
        "required": ["file_path"]
    });
    validate_tool_input(&schema, args).map_err(|e| anyhow::anyhow!("Invalid arguments: {}", e))?;

    let file_path_str: String = args["file_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid 'file_path' argument"))?
        .to_string();

    let ws_root = match std::env::var("RUSRAG_WORKSPACE") {
        Ok(p) => p,
        Err(_) => std::env::current_dir()?.to_string_lossy().to_string(),
    };
    let resolved = std::path::PathBuf::from(&ws_root).join(&file_path_str);

    let canonical_ws =
        std::fs::canonicalize(&ws_root).unwrap_or_else(|_| std::path::PathBuf::from(&ws_root));
    let canonical_file = match std::fs::canonicalize(&resolved) {
        Ok(p) => p,
        Err(_) => {
            return Err(anyhow::anyhow!("File not found: {}", file_path_str));
        }
    };

    if !canonical_file.starts_with(&canonical_ws) {
        return Err(anyhow::anyhow!(
            "Access denied: file outside workspace root"
        ));
    }

    // Read the file and optionally extract line range
    let content = match std::fs::read_to_string(&canonical_file) {
        Ok(c) => c,
        Err(e) => {
            return Err(anyhow::anyhow!("Failed to read: {}", e));
        }
    };

    if content.len() > 100_000 {
        return Err(anyhow::anyhow!(
            "File too large: {} bytes (max 100KB)",
            content.len()
        ));
    }

    // Handle line range if specified
    let (final_content, line_start, line_end) = if let (Some(line_start_val), Some(line_end_val)) =
        (args.get("line_start"), args.get("line_end"))
    {
        let start = line_start_val.as_u64().map(|n| n as usize).unwrap_or(1);
        let end = line_end_val
            .as_u64()
            .map(|n| n as usize)
            .unwrap_or(usize::MAX);

        // Validate line range
        if start < 1 {
            return Err(anyhow::anyhow!("line_start must be >= 1"));
        }
        if end < start {
            return Err(anyhow::anyhow!("line_end must be >= line_start"));
        }

        // Split content into lines
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        // Adjust end to not exceed total lines
        let actual_end = std::cmp::min(end, total_lines);

        // If start is beyond total lines, return empty content
        if start > total_lines {
            ("".to_string(), start, 0)
        } else {
            // Extract the line range (1-indexed to 0-indexed conversion)
            let selected_lines: String = lines[start.saturating_sub(1)..actual_end].join("\n");
            (selected_lines, start, actual_end)
        }
    } else {
        // No line range specified, return full content (backward compatibility)
        (content.clone(), 1, content.lines().count())
    };

    Ok(serde_json::json!({
        "file_path": file_path_str,
        "content": final_content,
        "content_length": final_content.len(),
        "line_range": {
            "start": line_start,
            "end": line_end,
            "total_lines": content.lines().count()
        }
    }))
}

// ---- Main MCP server loop --------------------------------------------------

/// Run the MCP server — reads JSON-RPC requests from stdin, writes responses to stdout.
pub async fn run_mcp_server(workspace_root: &std::path::Path) -> Result<()> {
    let state = std::sync::Arc::<std::sync::Mutex<McpState>>::new(std::sync::Mutex::new(
        McpState::new(workspace_root),
    ));

    loop {
        // Read a single line from stdin.
        let mut line = String::new();
        match io::stdin().lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }

        // Parse single request or batch.
        let requests: Vec<JsonRpcRequest> = match serde_json::from_str(&trimmed) {
            Ok(req) => vec![req], // single request
            Err(_) => match serde_json::from_str::<Vec<JsonRpcRequest>>(&trimmed) {
                Ok(batch) => batch, // batch of requests
                Err(e) => {
                    let resp = err_response(None, -32700, &format!("Parse error: {}", e));
                    writeln!(io::stdout(), "{}", serde_json::to_string(&resp).unwrap()).ok();
                    io::stdout().flush().ok();
                    continue;
                }
            },
        };

        // Dispatch each request.
        for req in requests {
            let response = dispatch_request(req, &state).await;
            if let Some(resp) = response {
                writeln!(io::stdout(), "{}", serde_json::to_string(&resp).unwrap()).ok();
                io::stdout().flush().ok();
            }
        }
    }

    Ok(())
}

pub async fn dispatch_request(
    req: JsonRpcRequest,
    state: &std::sync::Arc<std::sync::Mutex<McpState>>,
) -> Option<JsonRpcResponse> {
    let method = req.method;
    let params = req.params.clone();
    let id = req.id.clone();

    // Handle "tools/call" outside the lock since rag_query_tool spawns async work.
    if method == "tools/call" {
        // Clone the path out of the guard so we can release the lock before awaiting.
        let store_path = state.lock().unwrap().store_path.clone();
        let result = handle_tools_call(params, &store_path).await;
        return match result {
            Ok(v) => Some(ok_response(id, v)),
            Err(e) => Some(err_response(
                id,
                -32603,
                &format!(
                    "Internal error: {}",
                    e.to_string().chars().take(256).collect::<String>()
                ),
            )),
        };
    }

    // All other methods are synchronous — hold the lock for the entire handling.
    let guard = state.lock().unwrap();
    let result: Result<Value> = match method.as_str() {
        "initialize" => {
            if !guard.store_path.join("index.jsonl").exists() {
                Err(anyhow::anyhow!(
                    "No index found. Run `rust-rag index <path>` first."
                ))
            } else {
                // Auto-initialize: set the flag after successful initialize handshake.
                drop(guard);
                let g = state.lock().unwrap();
                g.initialized
                    .store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(serde_json::json!({
                    "protocolVersion": MCP_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "rust-rag-mcp", "version": "0.1.0" }
                }))
            }
        }
        "notifications/initialized" => {
            // Set initialized flag atomically — no need to hold the Mutex for a boolean.
            guard
                .initialized
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(serde_json::json!({}))
        }
        "tools/list" => handle_tools_list(&guard),
        _ => Err(anyhow::anyhow!("Method not found: {}", method)),
    };

    match result {
        Ok(v) => Some(ok_response(id, v)),
        Err(e) => Some(err_response(
            id,
            -32603,
            &format!(
                "Internal error: {}",
                e.to_string().chars().take(256).collect::<String>()
            ),
        )),
    }
}
