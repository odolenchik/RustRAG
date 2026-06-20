use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Write as IoWrite};

/// MCP (Model Context Protocol) server over JSON-RPC 2.0 / stdio.
/// Implements: initialize handshake, notifications/initialized, tools/list, tools/call with JSON Schema validation.
/// Supports batch requests per JSON-RPC 2.0 spec.

// ---- JSON-RPC types -------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)] // deserialized but not used locally — MCP handles protocol versioning
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
    id: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    result: Option<Value>,
    error: Option<JsonRpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

// ---- MCP protocol helpers --------------------------------------------------

const MCP_VERSION: &str = "2024-11-05";

fn ok_response(id: Option<Value>, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(result),
        error: None,
        id,
    }
}

fn err_response(id: Option<Value>, code: i32, message: &str) -> JsonRpcResponse {
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
    fn new(workspace_root: &std::path::Path) -> Self {
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

#[derive(Debug, Deserialize)]
struct InitializeParams {
    #[allow(dead_code)] // deserialized but not used — MCP protocol version is fixed
    #[serde(default)]
    protocol_version: String,
}

#[allow(dead_code)] // notification handler registered but MCP stdio transport doesn't always send it
fn handle_notification_initialized(_params: Value) -> Result<()> {
    Ok(())
}

fn handle_tools_list(state: &McpState) -> Result<Value> {
    state.require_initialized()?;
    Ok(serde_json::json!({
        "tools": [
            {
                "name": "rag_search",
                "description": "Search for code chunks by semantic similarity. Returns raw relevant snippets without LLM generation.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "The search query" },
                        "top_k": { "type": "integer", "description": "Number of results to return (default 5)", "minimum": 1, "maximum": 100 }
                    },
                    "required": ["query"]
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
                "description": "Read the full content of a file within the workspace. Path is relative to the workspace root.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file_path": { "type": "string", "description": "Relative path to the file from workspace root (e.g., 'crates/server/src/lib.rs')" }
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
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "query": { "type": "string", "maxLength": 4096 },
            "top_k": { "type": "integer", "minimum": 1, "maximum": 100 }
        },
        "required": ["query"]
    });
    validate_tool_input(&schema, args).map_err(|e| anyhow::anyhow!("Invalid arguments: {}", e))?;

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

    let top_k: usize = args["top_k"].as_u64().map(|n| n as usize).unwrap_or(5);

    if !(1..=100).contains(&top_k) {
        return Err(anyhow::anyhow!("top_k must be between 1 and 100"));
    }

    let embedding = rust_rag_core::embedding::embed(&query)?;
    let store = rust_rag_core::vector_store::VectorStore::open(store_path)?;
    let results = store.hybrid_search(&embedding, &query, top_k, 0.7, None)?;

    Ok(serde_json::json!({
        "content": results_to_string(&results),
    }))
}

// ---- New tool: rag_workspace_info -------------------------------------------

fn rag_workspace_info_tool(args: &Value) -> Value {
    let detail_level: &str = args.get("detail_level").and_then(|v| v.as_str()).unwrap_or("summary");
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
        Err(e) => return serde_json::json!({"workspace_root": ws_root.to_string_lossy().to_string(), "error": format!("Failed to read Cargo.toml: {}", e), "crates": []}),
    };

    let cargo_value: toml::Value = match cargo_content.parse() {
        Ok(v) => v,
        Err(e) => return serde_json::json!({"workspace_root": ws_root.to_string_lossy().to_string(), "error": format!("Failed to parse Cargo.toml: {}", e), "crates": []}),
    };

    let member_dirs: Vec<String> = if let Some(workspace) = cargo_value.get("workspace") {
        workspace.get("members").and_then(|m| m.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).map(String::from).collect())
            .unwrap_or_default()
    } else { vec![".".to_string()] };

    let mut crates_info: Vec<Value> = Vec::new();

    for member_pattern in &member_dirs {
        let full_pattern = ws_root.join(member_pattern);
        let matches = match glob::glob(&full_pattern.to_string_lossy()) {
            Ok(m) => m.filter_map(|e| e.ok()).collect::<Vec<_>>(),
            Err(_) => Vec::new(),
        };

        for member_dir in &matches {
            if !member_dir.is_dir() || !member_dir.join("Cargo.toml").exists() { continue; }

            let content = match std::fs::read_to_string(member_dir.join("Cargo.toml")) { Ok(c) => c, Err(_) => continue };
            let mv: toml::Value = match content.parse() { Ok(v) => v, Err(_) => continue };

            let mut ci: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
            ci.insert("name".to_string(), Value::String(
                mv.get("package").and_then(|p| p.get("name")).and_then(|n| n.as_str()).unwrap_or("unknown").to_string()
            ));

            let rel_path = match member_dir.strip_prefix(&ws_root) {
                Ok(p) if p.as_os_str().is_empty() || p == std::path::Path::new(".") => ".".to_string(),
                Ok(p) => p.to_string_lossy().to_string(),
                Err(_) => member_dir.to_string_lossy().to_string(),
            };
            ci.insert("path".to_string(), Value::String(rel_path));

            if detail_level == "full" {
                let deps: Vec<String> = mv.get("dependencies").and_then(|d| d.as_table())
                    .map(|t| t.iter().filter(|(n, _)| !n.starts_with("rust-rag")).map(|(n, _)| n.clone()).collect())
                    .unwrap_or_default();
                ci.insert("dependencies".to_string(), serde_json::to_value(deps).unwrap_or(Value::Null));

                let readme_path = member_dir.join("README.md");
                if readme_path.exists() {
                    if let Ok(rc) = std::fs::read_to_string(&readme_path) {
                        ci.insert("readme".to_string(), Value::String(if rc.chars().count() > 2000 { format!("{}\n... (truncated)", &rc.chars().take(2000).collect::<String>()) } else { rc }));
                    }
                }
            }

            crates_info.push(Value::Object(serde_json::Map::from_iter(ci)));
        }
    }

    // Check for implicit members (non-member dirs with Cargo.toml)
    if member_dirs.len() == 1 && member_dirs[0] == "." {
        let entries = match std::fs::read_dir(&ws_root) { Ok(e) => e.filter_map(|x| x.ok()).collect::<Vec<_>>(), Err(_) => Vec::new() };
        for entry in &entries {
            if !entry.path().is_dir() || entry.path() == ws_root { continue; }
            if entry.path().join("Cargo.toml").exists() {
                let covered = crates_info.iter().any(|c| c.get("path").and_then(|p| p.as_str()) == Some(&entry.path().strip_prefix(&ws_root).unwrap_or(&entry.path()).to_string_lossy()));
                if !covered {
                    crates_info.push(serde_json::json!({"name": entry.file_name().to_string_lossy(), "path": entry.path().strip_prefix(&ws_root).unwrap_or(&entry.path()).to_string_lossy()}));
                }
            }
        }
    }

    serde_json::json!({ "workspace_root": ws_root.to_string_lossy().to_string(), "crates": crates_info })
}

// ---- New tool: rag_file_read -------------------------------------------------

fn rag_file_read_tool(args: &Value) -> Result<Value> {
    let schema = serde_json::json!({"type":"object","properties":{"file_path":{"type":"string"}},"required":["file_path"]});
    validate_tool_input(&schema, args).map_err(|e| anyhow::anyhow!("Invalid arguments: {}", e))?;

    let file_path_str: String = args["file_path"].as_str().ok_or_else(|| anyhow::anyhow!("Missing or invalid 'file_path' argument"))?.to_string();

    let ws_root = match std::env::var("RUSRAG_WORKSPACE") { Ok(p) => p, Err(_) => std::env::current_dir()?.to_string_lossy().to_string() };
    let resolved = std::path::PathBuf::from(&ws_root).join(&file_path_str);

    let canonical_ws = std::fs::canonicalize(&ws_root).unwrap_or_else(|_| std::path::PathBuf::from(&ws_root));
    let canonical_file = match std::fs::canonicalize(&resolved) { Ok(p) => p, Err(_) => return Ok(serde_json::json!({"error": format!("File not found: {}", file_path_str), "file_path": file_path_str})) };

    if !canonical_file.starts_with(&canonical_ws) {
        return Ok(serde_json::json!({"error": format!("Access denied: file outside workspace root"), "file_path": file_path_str}));
    }

    let content = match std::fs::read_to_string(&canonical_file) {
        Ok(c) => c,
        Err(e) => return Ok(serde_json::json!({"error": format!("Failed to read: {}", e), "file_path": file_path_str})),
    };

    if content.len() > 100_000 {
        return Ok(serde_json::json!({"error": format!("File too large: {} bytes (max 100KB)", content.len()), "file_path": file_path_str, "content_length": content.len()}));
    }

    Ok(serde_json::json!({ "file_path": file_path_str, "content": content, "content_length": content.len() }))
}

fn results_to_string(results: &[rust_rag_core::vector_store::SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }
    let mut output = String::new();
    for (i, r) in results.iter().enumerate() {
        output.push_str(&format!(
            "[{}] Score: {:.3} | {}:{}\n{}\n",
            i + 1,
            r.score,
            r.file_path.display(),
            r.line_start,
            r.text
        ));
    }
    output
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

async fn dispatch_request(
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
            Err(e) => Some(err_response(id, -32603, &format!("Internal error: {}", e.to_string().chars().take(256).collect::<String>()))),
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
        Err(e) => Some(err_response(id, -32603, &format!("Internal error: {}", e.to_string().chars().take(256).collect::<String>()))),
    }
}
