# Security Audit Report — RustRAG

**Project:** RustRAG (Local RAG tool for Rust projects)  
**Version:** 0.7.9  
**Audit Date:** 2026-06-10  
**Auditor:** Automated Security Analysis  
**Scope:** All crates (`cli`, `core`, `llm`, `server`, `tui`)

---

## Executive Summary

RustRAG is a local RAG (Retrieval Augmented Generation) tool for Rust codebases. It indexes Rust source files, creates vector embeddings locally, and queries an LLM backend via HTTP (Ollama / llama.cpp). The project demonstrates generally good security hygiene with atomic file operations and no shell injection vectors. However, several **Medium** and **Low** severity issues were identified, primarily in the server component's network exposure and missing protections.

### Overall Security Rating: ⚠️ **MEDIUM RISK** (for production/public-facing deployment)

| Severity | Count |
|----------|-------|
| Critical | 0     |
| High     | 1     |
| Medium   | 6     |
| Low      | 8     |

---

## Findings by Category

### 1. Network Security (Server)

#### 🔴 HIGH-1: No Server-Side Request Forgery (SSRF) Protection for LLM Endpoint

**Location:** `crates/llm/src/ollama_client.rs` (lines 116–134), `crates/server/src/lib.rs` (line 158)

```rust
// ollama_client.rs:117-128
pub fn new(base_url: &str, model: &str) -> Self {
    let url = if !base_url.starts_with("http") {
        format!("http://{}/chat/completions", base_url)  // ← No protocol validation
    } else if ...
```

The `LlmClient::new()` accepts any URL string and constructs requests without validation. An attacker who controls the endpoint configuration (via `.rustrag.toml` or environment variables) could:
- Send LLM requests to internal network services (`http://169.254.169.254/...`)
- Probe internal infrastructure via `http://localhost:<port>` URLs

**Impact:** If the server is exposed externally and endpoint configuration can be influenced by an attacker, this enables SSRF attacks against internal services.

**Recommendation:** Implement URL validation to restrict scheme to `http`/`https`, block private IP ranges (10.x, 172.16-31.x, 192.168.x, 169.254.x, 127.x), and require explicit opt-in for non-standard endpoints.

---

#### 🟡 MEDIUM-1: Overly Permissive CORS Configuration

**Location:** `crates/server/src/lib.rs` (line 71)

```rust
let cors = CorsLayer::permissive();
```

The server enables fully permissive CORS — allowing any origin, any method, and any header. While the server binds to `127.0.0.1` by default, if a user changes the bind address (e.g., for development), this opens the API to cross-origin abuse.

**Impact:** If bound to `0.0.0.0`, any website could make authenticated requests against the RustRAG server without user consent.

**Recommendation:** Use a restrictive CORS policy with explicit allowed origins. Default to:
```rust
CorsLayer::new()
    .allow_origin(Origin::same())
    .allowed_methods([Method::GET, Method::POST])
```

---

#### 🟡 MEDIUM-2: No TLS/HTTPS Support — Plaintext HTTP Only

**Location:** `crates/server/src/bin.rs` (line 52), `crates/llm/src/ollama_client.rs` (line 161)

All network communication is plaintext:
- Server binds to `http://127.0.0.1:<port>` — no TLS option
- Default LLM endpoint uses `http://localhost:8080` — no HTTPS fallback

The core crate enables `rustls-tls` for the embedding download client (`crates/core/Cargo.toml`: `"rustls-tls"`), but the server and LLM clients do not use TLS.

**Impact:** If bound to a network interface, all queries (including code context) are transmitted in plaintext. LLM API keys/endpoints in configuration files could be intercepted on shared networks.

**Recommendation:** Add optional TLS support via `tokio-rustls` or native-tls for the server listener. For outbound LLM connections, default to HTTPS and validate certificates.

---

#### 🟡 MEDIUM-3: No Request Rate Limiting or Input Size Limits

**Location:** `crates/server/src/lib.rs` (lines 58–67, 186–197)

```rust
struct SearchQuery {
    query: String,        // ← No max length validation
    #[serde(default = "default_top_k")]
    top_k: usize,         // ← Default is 5; no upper bound on HTTP endpoints
}

struct QueryBody {
    question: String,     // ← No max length validation
}
```

The server accepts arbitrary-length query strings and has no rate limiting. An attacker could:
- Send extremely long queries to cause resource exhaustion (DoS)
- Flood the API with requests consuming LLM tokens or compute

**Impact:** Denial of Service via resource exhaustion. The `/query` endpoint embeds and sends user text to an LLM, which can be expensive if abused.

**Recommendation:** 
1. Add `#[serde(default)]` with explicit max length validation (e.g., 4096 characters)
2. Implement rate limiting (e.g., `tower::limit::RateLimit`)
3. Add request body size limits via axum's `DefaultBodyLimit`

---

### 2. File System Security

#### 🟡 MEDIUM-4: Path Traversal in Workspace Paths — No Canonicalization

**Location:** `crates/cli/src/main.rs` (lines 117–125), `crates/cli/src/lib.rs` (lines 66–70)

```rust
// main.rs:118
let workspace_root = std::path::PathBuf::from(&args.path);
let store_path = workspace_root.join(".rustrag");
if store_path.exists() {
    println!("Removing old index at {}", store_path.display());
    std::fs::remove_dir_all(&store_path)?;  // ← DANGEROUS: user-controlled path
}
```

User-supplied paths are used directly for filesystem operations without canonicalization or validation. An attacker could:
- Pass `../../etc` as a workspace path, causing `.rustrag` to be created/removed in unexpected locations
- Use symlinks to point the index into sensitive directories

**Impact:** Accidental data loss (deleting files outside intended scope) via symlink attacks or path traversal.

**Recommendation:** Canonicalize paths and verify they resolve within an expected parent directory:
```rust
let workspace_root = std::fs::canonicalize(args.path)?;
```

---

#### 🟢 LOW-1: `.rustrag` Directory Permissions Not Enforced

**Location:** `crates/core/src/vector_store.rs` (lines 39–46)

```rust
pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
    let path = path.as_ref();
    std::fs::create_dir_all(path)?;  // ← Default directory permissions (0755 on most systems)
```

The `.rustrag` directories are created with default filesystem permissions. On shared/multi-user systems, other users could read/write index files containing source code snippets.

**Impact:** Information disclosure of indexed source code in multi-user environments.

**Recommendation:** Set restrictive directory permissions (e.g., `0700`) for `.rustrag` directories:
```rust
std::fs::create_dir_all(path)?;
#[cfg(unix)]
std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
```

---

#### 🟢 LOW-2: No File Type Validation in Indexing — Arbitrary File Reading

**Location:** `crates/core/src/indexer.rs` (lines 272–291)

```rust
fn collect_rs_files(dir: &Path, chunks: &mut Vec<Chunk>) -> Result<()> {
    for entry in walkdir::WalkDir::new(dir) ... {
        let path = entry.path();
        if !path.is_file() || path.extension() != Some("rs".as_ref()) {
            continue;
        }
        let content = std::fs::read_to_string(path)?;  // ← Any readable .rs file
```

While the indexer filters for `.rs` files, there's no protection against:
- Symlink traversal (following symlinks into sensitive directories)
- Extremely large `.rs` files causing memory exhaustion during embedding

**Impact:** Information disclosure via symlinked paths; potential DoS.

**Recommendation:** Use `walkdir`'s `follow_links(false)` (default), and add file size limits. Validate symlinks point within the workspace.

---

### 3. Input Validation & Sanitization

#### 🟢 LOW-3: No Input Length Limits on CLI Arguments

All CLI subcommands accept unbounded string inputs for queries, paths, and model parameters. While this is less critical for a CLI tool (the user controls their own terminal), it could cause issues with very large inputs in automated scripts.

**Recommendation:** Add reasonable max lengths to clap arguments where appropriate.

---

#### 🟢 LOW-4: Minimal JSON Schema Validation in MCP Tools

**Location:** `crates/server/src/mcp.rs` (lines 148–216)

The `validate_tool_input()` function performs basic type checking but does NOT validate:
- Maximum string length for `query` or `question` fields
- Special characters that could cause issues downstream (e.g., null bytes, extremely long strings)
- Integer overflow for `top_k` beyond the JSON schema's `maximum: 100`

Note: The `rag_search_tool` does validate top_k bounds at line 253–254, but `rag_query_tool` uses config-driven `top_k` with no cap.

**Recommendation:** Enforce max string lengths and ensure all integer parameters have explicit bounds checking.

---

### 4. Authentication & Authorization

#### 🟡 MEDIUM-5: No Authentication on Any Interface

**Location:** All server endpoints (`crates/server/src/lib.rs`)

Neither the HTTP API nor the MCP protocol implements any form of authentication:
- `/status`, `/search`, `/query`, `/query/stream` — no auth required
- MCP `tools/call` — requires initialization handshake but no credential verification

**Impact:** Any process with network access to the server can query the RAG system, consume LLM tokens, and read indexed code.

**Recommendation:** 
1. For local use: consider a simple API key or token-based auth
2. Document that the server should only be bound to `127.0.0.1` in production
3. Consider Unix socket binding as an alternative for local-only access

---

#### 🟢 LOW-5: Default Model Name Contains Potentially Sensitive Information

**Location:** `crates/llm/src/ollama_client.rs` (line 162–164)

```rust
let model = model.unwrap_or_else(|| {
    "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf".to_string()
});
```

The default model name includes:
- Model architecture details (`A3B` — 3 trillion parameters)
- Potentially problematic naming (`Uncensored`, `Aggressive`)
- Quantization format (`IQ3_M`)

**Impact:** Low severity — information disclosure that could be used for reconnaissance.

**Recommendation:** Use a neutral default model name or require explicit configuration.

---

### 5. Code Injection Vulnerabilities

#### 🟢 LOW-6: LLM Prompt Injection (Template Injection)

**Location:** `crates/server/src/lib.rs` (lines 152–153), `crates/cli/src/lib.rs` (lines 281–282)

```rust
let system_prompt = "You are a Rust code analysis assistant. Answer questions based on the provided code snippets...";
let user_message = format!("Question: {}\n\nRelevant code:\n{}", body.question, context);
```

User-controlled input (`body.question`, `query`) is directly inserted into LLM prompts without sanitization. A malicious query could contain prompt injection attacks to extract system instructions or produce unexpected outputs.

**Impact:** Low — the system is designed for local use and the user controls both inputs and the codebase being indexed. However, if MCP tools are called by third-party AI agents, this becomes a higher risk.

**Recommendation:** Implement output validation on LLM responses (e.g., check for leaked system prompts) and consider adding input sanitization for known injection patterns.

---

#### ✅ No Shell Injection — Positive Finding

The project does NOT use `std::process::Command` to execute shell commands with user-supplied arguments anywhere in the codebase. All file operations are direct Rust stdlib calls, eliminating shell injection risk.

---

### 6. Concurrency & Race Conditions

#### 🟢 LOW-7: TOCTOU Vulnerability in Index Updates

**Location:** `crates/core/src/vector_store.rs` (lines 90–121)

The `remove_documents` method reads the index file, filters, writes to a temp file, then renames. While the final write is atomic via `rename`, the read-modify-write cycle has a window where:
- Concurrent access could lead to lost updates
- The cache (`RwLock`) and file content may desync

The cache invalidation relies on mtime comparison (lines 140–152), which is not atomic with respect to file writes.

**Impact:** Low — only affects concurrent indexing operations, which are unlikely in typical usage patterns.

**Recommendation:** Consider using a write-ahead log or advisory file locking for index mutations.

---

#### ✅ Good: Atomic File Operations Found

The project correctly uses atomic rename patterns in multiple places:
- `state.rs` line 56: `fs::rename(&tmp_path, &path)` for index state persistence
- `vector_store.rs` line 118: `std::fs::rename(&tmp_path, &index_path)` for index updates

---

### 7. Error Handling & Information Leakage

#### 🟢 LOW-8: File Paths Exposed in API Responses

**Location:** `crates/server/src/lib.rs` (lines 92–96), `crates/server/src/mcp.rs` (lines 301–308)

```rust
JsonResponse(serde_json::json!({
    "workspace_root": state.0.store.path.display().to_string(),
    "total_chunks": total_chunks,
    "index_path": index_path.to_str().unwrap_or(""),
}))
```

The `/status` endpoint returns the full workspace root path and index file path in the response. While this is relatively benign for a local tool, it reveals filesystem structure to any API caller.

**Impact:** Low — information disclosure of filesystem layout.

---

#### ✅ Good: No Secrets in Error Messages

Error messages do not include API keys, tokens, or passwords. The `reqwest` client uses `rustls-tls` for certificate validation when downloading models (`crates/core/Cargo.toml`).

---

### 8. Dependency Security

| Package | Version | Notes |
|---------|---------|-------|
| `axum` | 0.7.9 | Latest stable; no known critical CVEs |
| `reqwest` | 0.12.28 | Latest stable with rustls-tls; secure |
| `tokio` | 1.42+ | Latest stable; fully enabled features |
| `serde_json` | 1.0 | Latest; no known CVEs |
| `walkdir` | 2.x | Read-only directory walker; minimal risk |
| `tree-sitter` | 0.25 | Parser library; isolated from network |
| `ratatui` / `crossterm` | 0.29/0.28 | Terminal UI; no network exposure |

**Note:** Running `cargo audit` is recommended for a complete dependency vulnerability scan. The dependencies used are generally well-maintained with small attack surfaces. No obviously vulnerable versions detected from version analysis.

---

### 9. Embedding Model Download Security

#### 🟡 MEDIUM-6: Unverified Download of ONNX Model Files

**Location:** `crates/core/src/embedding.rs` (lines 323–370)

```rust
pub fn download_model(target: &Path) -> Result<()> {
    ...
    let response = client.get(&url).send()?;
    ...
    std::fs::write(target.join(local_name), &bytes)?;
```

Model files are downloaded from HuggingFace without:
- Checksum verification (SHA-256 / MD5)
- TLS certificate pinning
- Content-type validation for the response body

**Impact:** If an attacker performs a MITM attack or compromises HuggingFace CDN, they could deliver a malicious ONNX model that executes arbitrary code during embedding computation.

**Recommendation:** 
1. Verify file checksums after download against known-good values from HuggingFace
2. Always use HTTPS (the `rustls-tls` feature is enabled, which provides TLS)
3. Add content-type validation for downloaded artifacts

---

## Summary of All Findings

| ID | Severity | Title | Location |
|----|----------|-------|----------|
| HIGH-1 | 🔴 High | SSRF via unvalidated LLM endpoint URLs | `llm/ollama_client.rs:117` |
| MEDIUM-1 | 🟡 Medium | Overly permissive CORS configuration | `server/lib.rs:71` |
| MEDIUM-2 | 🟡 Medium | No TLS/HTTPS support on server or LLM client | `server/bin.rs:52`, `llm/ollama_client.rs:161` |
| MEDIUM-3 | 🟡 Medium | No request rate limiting / input size limits | `server/lib.rs:58–67` |
| MEDIUM-4 | 🟡 Medium | Path traversal in workspace paths | `cli/main.rs:118`, `cli/lib.rs:66` |
| MEDIUM-5 | 🟡 Medium | No authentication on any API endpoint | All server handlers |
| MEDIUM-6 | 🟡 Medium | Unverified model file downloads | `core/embedding.rs:323` |
| LOW-1 | 🟢 Low | `.rustrag` directory permissions not enforced | `core/vector_store.rs:40` |
| LOW-2 | 🟢 Low | No file type validation during indexing | `core/indexer.rs:272` |
| LOW-3 | 🟢 Low | No input length limits on CLI arguments | All CLI commands |
| LOW-4 | 🟢 Low | Incomplete JSON Schema validation in MCP | `server/mcp.rs:148` |
| LOW-5 | 🟢 Low | Sensitive info in default model name | `llm/ollama_client.rs:163` |
| LOW-6 | 🟢 Low | LLM prompt injection vectors | `server/lib.rs:152–153` |
| LOW-7 | 🟢 Low | TOCTOU race condition in index updates | `core/vector_store.rs:90` |
| LOW-8 | 🟢 Low | File paths exposed in API responses | `server/lib.rs:92–96` |

---

## Recommendations by Priority

### Immediate (Before Production Deployment)

1. **Fix HIGH-1:** Add URL validation for LLM endpoints — block private IP ranges and require explicit opt-in for non-standard protocols
2. **Fix MEDIUM-5:** Document that the server must only be bound to `127.0.0.1` in production; add a warning at startup if bind address is changed
3. **Fix MEDIUM-6:** Add SHA-256 checksum verification for downloaded model files

### Short-Term (Next Release)

4. **Fix MEDIUM-3:** Add request rate limiting and input size limits to all server endpoints
5. **Fix MEDIUM-4:** Canonicalize workspace paths before use; validate symlinks
6. **Fix LOW-8:** Remove or obfuscate file system paths from API responses

### Medium-Term (Future Releases)

7. Add optional authentication for the HTTP API
8. Implement CORS with explicit allowlist instead of `CorsLayer::permissive()`
9. Consider adding TLS support for the server listener
10. Run `cargo audit` as part of CI/CD pipeline for automated dependency scanning

---

## Positive Findings

The project demonstrates several strong security practices:

- ✅ **No shell injection vectors** — all file operations use Rust stdlib, no command execution with user input
- ✅ **Atomic file operations** — temp file + rename pattern used consistently for index and state updates
- ✅ **TLS for outbound connections** — `rustls-tls` feature enabled for reqwest in core crate
- ✅ **No secrets hardcoded** — API keys/endpoints are configuration-driven, not embedded
- ✅ **JSON Schema validation** — MCP tools validate input types before processing
- ✅ **Bounded top_k parameter** — MCP search tool enforces 1–100 range on results count
- ✅ **Graceful error handling** — errors don't leak credentials or stack traces to clients

---

## Conclusion

RustRAG is a well-structured project with generally good security practices for its intended use case (local development tool). The primary risk areas are in the **server component's network exposure** (CORS, authentication, rate limiting) and **LLM endpoint validation**. For local-only use on `127.0.0.1`, most of these issues are low-risk. However, if the server is ever exposed to a network or the LLM endpoint configuration can be influenced by untrusted parties, the HIGH and MEDIUM findings become significant.

**Recommended next steps:**
1. Address the HIGH-1 SSRF finding immediately
2. Add input validation and rate limiting before any public-facing deployment
3. Integrate `cargo audit` into CI/CD for ongoing dependency monitoring
