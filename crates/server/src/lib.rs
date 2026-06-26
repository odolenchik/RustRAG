pub mod mcp;

use anyhow::Result;
use axum::{
    body::Body,
    extract::Query,
    http::StatusCode,
    response::Json as JsonResponse,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use rust_rag_core::{semantic_cache::SemanticCache, vector_store::VectorStore};
use rust_rag_llm::ChatBackend;
use std::io::BufRead;
use std::net::{SocketAddr, ToSocketAddrs};
use url::Url;

// logging handled by tracing crate (already imported via #[tracing::instrument])
use serde::Deserialize;
use std::{collections::VecDeque, path::Path, sync::Arc, time::Instant};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Sliding-window rate limiter — tracks request timestamps per client IP.
pub struct RateLimiter {
    /// Per-client deque of request timestamps.
    clients: std::sync::Mutex<std::collections::HashMap<String, VecDeque<Instant>>>,
    max_per_window: usize,
    window_secs: u64,
}

impl RateLimiter {
    /// Create a new rate limiter with the given requests-per-minute budget.
    pub fn new(per_minute: u32) -> Self {
        Self {
            clients: std::sync::Mutex::new(std::collections::HashMap::new()),
            max_per_window: per_minute as usize,
            window_secs: 60,
        }
    }

    /// Check whether the given client is allowed. Prunes stale entries first.
    pub fn check(&self, key: &str) -> bool {
        let now = Instant::now();
        let window = std::time::Duration::from_secs(self.window_secs);
        let mut map = self.clients.lock().unwrap();

        // Clean up expired entries for this client.
        if let Some(queue) = map.get_mut(key) {
            while let Some(&front) = queue.front() {
                if now.duration_since(front) >= window {
                    queue.pop_front();
                } else {
                    break;
                }
            }
            // Check against the budget.
            if queue.len() < self.max_per_window {
                queue.push_back(now);
                true
            } else {
                false
            }
        } else {
            map.insert(key.to_string(), [now].into());
            true
        }
    }

    /// Get the client key from headers (X-Forwarded-For / X-Real-IP) or a placeholder.
    pub fn resolve_key(headers: &axum::http::HeaderMap, fallback: &str) -> String {
        // Prefer explicit proxy headers for correctness behind load balancers.
        if let Some(forwarded) = headers.get("X-Forwarded-For").and_then(|h| h.to_str().ok()) {
            return forwarded
                .split(',')
                .next()
                .unwrap_or(fallback)
                .trim()
                .to_string();
        }
        if let Some(real_ip) = headers.get("X-Real-Ip").and_then(|h| h.to_str().ok()) {
            return real_ip.trim().to_string();
        }
        fallback.to_string()
    }
}

/// Maximum allowed length for search queries and questions to prevent resource exhaustion.
const MAX_QUERY_LENGTH_CHARS: usize = 4096;

/// Default maximum size (in bytes) of the assembled context sent to the LLM.
const DEFAULT_MAX_CONTEXT_SIZE: usize = 12_000;

/// Sanitize an error message for exposure over HTTP/MCP by truncating long messages and masking internal paths.
fn sanitize_error(e: &dyn std::fmt::Display) -> String {
    let msg = format!("{}", e);
    // Truncate to 512 chars to prevent oversized payloads / prompt injection from errors.
    let truncated: String = msg.chars().take(512).collect();
    // Mask internal user paths like `/home/user/.cache/huggingface/...` → `~/.cache/huggingface/...`
    let masked = truncated.replace(std::env::var("HOME").as_deref().unwrap_or("~"), "~");
    masked
}

/// Request state shared across handlers.
pub struct AppState {
    pub store: std::sync::Arc<VectorStore>,
    /// Sliding-window rate limiter (per-client). Cloned via Arc for shared access.
    pub rate_limiter: Arc<RateLimiter>,
    /// Shared HTTP client for LLM requests — enables connection pooling, with timeouts.
    pub http_client: std::sync::Arc<reqwest::Client>,
    /// Resolved LLM endpoint (from config / env / default).
    pub llm_endpoint: String,
    /// Resolved LLM model name (from config / env / default).
    pub llm_model: String,
    /// Optional API key for Bearer token authentication. Empty string means no auth required.
    pub api_key: Option<String>,
    /// Maximum context size in bytes sent to the LLM.
    pub max_context_size: usize,
    /// Semantic cache for LLM answers (empty cache when disabled).
    pub semantic_cache: Arc<SemanticCache>,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            store: std::sync::Arc::clone(&self.store),
            rate_limiter: Arc::clone(&self.rate_limiter),
            http_client: std::sync::Arc::clone(&self.http_client),
            llm_endpoint: self.llm_endpoint.clone(),
            llm_model: self.llm_model.clone(),
            api_key: self.api_key.clone(),
            max_context_size: self.max_context_size,
            semantic_cache: Arc::clone(&self.semantic_cache),
        }
    }
}

impl AppState {
    /// Resolve LLM endpoint with priority: config > LLAMA_ENDPOINT env > default.
    fn resolve_endpoint() -> String {
        let cfg = rust_rag_core::config::Config::find().ok();
        std::env::var("LLAMA_ENDPOINT")
            .ok()
            .or_else(|| cfg.as_ref().and_then(|c| c.llm_config().endpoint.clone()))
            .unwrap_or_else(|| "http://localhost:8080".to_string())
    }

    /// Resolve LLM model with priority: env > config > auto-detect from /v1/models > default.
    fn resolve_model(endpoint: &str) -> String {
        let cfg = rust_rag_core::config::Config::find().ok();

        // Priority 1: LLAMA_MODEL env var (highest).
        if let Ok(model) = std::env::var("LLAMA_MODEL") {
            return model;
        }

        // Priority 2: config file.
        if let Some(c) = cfg.as_ref().and_then(|c| c.llm_config().model.clone()) {
            if !c.is_empty() {
                return c;
            }
        }

        // Priority 3: auto-detect from LLM server's /v1/models endpoint.
        let normalized_endpoint = if endpoint.starts_with("http") {
            // Strip path, keep only origin (e.g. http://localhost:8080/chat/completions → http://localhost:8080)
            endpoint
                .trim_end_matches('/')
                .trim_end_matches("/chat/completions")
                .trim_end_matches("/v1/chat/completions")
                .to_string()
        } else {
            format!("http://{}/v1/models", endpoint)
        };

        if let Ok(client) = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
        {
            if let Ok(resp) = client.get(&normalized_endpoint).send() {
                if resp.status().is_success() {
                    if let Ok(json) = resp.json::<serde_json::Value>() {
                        if let Some(models) = json["data"].as_array() {
                            if let Some(first_model) = models.first() {
                                if let Some(name) = first_model.get("id").and_then(|v| v.as_str()) {
                                    tracing::info!(
                                        "Auto-detected LLM model from /v1/models: {}",
                                        name
                                    );
                                    return name.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }

        // Priority 4: hardcoded default.
        "default-rag-model".to_string()
    }

    /// Resolve optional API key from RUSRAG_API_KEY env var.
    fn resolve_api_key() -> Option<String> {
        std::env::var("RUSRAG_API_KEY")
            .ok()
            .filter(|key| !key.is_empty())
    }

    /// Build a shared `reqwest::Client` with connection pooling and production-ready timeouts.
    /// If SSRF strict mode is enabled (via RUSRAG_SSRF_STRICT), validates that the endpoint
    /// does not resolve to a private or link-local IP address.
    fn build_http_client(endpoint: &str) -> Result<Arc<reqwest::Client>, anyhow::Error> {
        // SSRF strict mode: reject private IPs instead of merely warning (env: RUSRAG_SSRF_STRICT=1).
        let strict_mode = std::env::var("RUSRAG_SSRF_STRICT").is_ok_and(|v| !v.is_empty());

        if strict_mode {
            Self::check_ssrf_strict(endpoint)?;
        }

        let client = reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            // Timeout for the entire HTTP request (connect + TLS + send + receive).
            // This prevents a single LLM call from hanging forever.
            .timeout(std::time::Duration::from_secs(300))
            // Per-connection read timeout — no chunk should take longer than 60s to arrive.
            .read_timeout(std::time::Duration::from_secs(60))
            // Limit redirects to prevent redirect-based SSRF attacks (max 5 hops).
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;

        Ok(Arc::new(client))
    }

    fn extract_host(endpoint: &str) -> Result<String, anyhow::Error> {
        // Parse the endpoint as a URL to get the host.
        let url = Url::parse(endpoint)?;
        Ok(url.host_str().unwrap_or("").to_string())
    }

    fn check_ssrf_strict(endpoint: &str) -> Result<(), anyhow::Error> {
        let host = Self::extract_host(endpoint)?;
        // If the host is empty, we cannot check.
        if host.is_empty() {
            return Ok(());
        }

        // Look up IP addresses for the host.
        let ips = host.to_socket_addrs()?;
        for ip in ips {
            match ip {
                SocketAddr::V4(ipv4) => {
                    if ipv4.ip().is_private() {
                        return Err(anyhow::anyhow!(
                            "SSRF strict mode: resolved IPv4 address {} is private",
                            ipv4
                        ));
                    }
                }
                SocketAddr::V6(ipv6) => {
                    // Check for unique local (fc00::/7), link-local (fe80::/10), and loopback (::1).
                    if ipv6.ip().is_unicast_link_local()
                        || ipv6.ip().is_unique_local()
                        || ipv6.ip().is_loopback()
                    {
                        return Err(anyhow::anyhow!(
                            "SSRF strict mode: resolved IPv6 address {} is local/link-local/unique local",
                            ipv6
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolve max context size from env override, config file, or default.
    fn resolve_max_context_size() -> usize {
        std::env::var("RUSRAG_MAX_CONTEXT_SIZE")
            .ok()
            .and_then(|v| v.parse().ok())
            .or_else(|| {
                rust_rag_core::config::Config::find()
                    .ok()
                    .as_ref()
                    .map(|c| {
                        c.llm_config()
                            .max_context_size
                            .unwrap_or(DEFAULT_MAX_CONTEXT_SIZE)
                    })
            })
            .unwrap_or(DEFAULT_MAX_CONTEXT_SIZE)
    }

    /// Resolve semantic cache config: enabled + TTL from environment or config file.
    fn resolve_semantic_cache(rustrag_dir: &Path) -> Arc<SemanticCache> {
        let config = rust_rag_core::config::Config::find().ok();
        let env_enabled = std::env::var("RUSRAG_SEMANTIC_CACHE_ENABLED")
            .ok()
            .map(|v| v == "1" || v.to_lowercase() == "true")
            .unwrap_or_else(|| {
                config
                    .as_ref()
                    .and_then(|c| {
                        c.semantic_cache_config()
                            .enabled
                            .then_some(c.semantic_cache_config().enabled)
                    })
                    .unwrap_or(false)
            });

        let ttl = std::env::var("RUSRAG_SEMANTIC_CACHE_TTL")
            .ok()
            .and_then(|v| v.parse().ok())
            .or_else(|| config.as_ref().map(|c| c.semantic_cache_config().ttl_secs));

        if env_enabled {
            Arc::new(SemanticCache::open(rustrag_dir, ttl))
        } else {
            // Return a disabled cache that never matches.
            Arc::new(SemanticCache::disabled())
        }
    }

    /// Create app state from a workspace root that has an index.
    pub fn from_workspace(workspace_root: &Path, rate_limit_per_min: u32) -> Result<Self> {
        let store_path = workspace_root.join(".rustrag");
        if !store_path.exists() {
            anyhow::bail!(
                "No vector store found at {}. Run `rust-rag index <path>` first.",
                workspace_root.display()
            );
        }
        let store = VectorStore::open(&store_path)?;
        let endpoint = Self::resolve_endpoint();
        Ok(Self {
            store: std::sync::Arc::new(store),
            rate_limiter: Arc::new(RateLimiter::new(rate_limit_per_min)),
            http_client: Self::build_http_client(&endpoint)?,
            llm_endpoint: endpoint.clone(),
            llm_model: Self::resolve_model(&endpoint),
            api_key: Self::resolve_api_key(),
            max_context_size: Self::resolve_max_context_size(),
            semantic_cache: Self::resolve_semantic_cache(&store_path),
        })
    }

    /// Create app state from a custom path.
    pub fn from_path(path: &Path, rate_limit_per_min: u32) -> Result<Self> {
        let store = VectorStore::open(path)?;
        let endpoint = Self::resolve_endpoint();
        Ok(Self {
            store: std::sync::Arc::new(store),
            rate_limiter: Arc::new(RateLimiter::new(rate_limit_per_min)),
            http_client: Self::build_http_client(&endpoint)?,
            llm_endpoint: endpoint.clone(),
            llm_model: Self::resolve_model(&endpoint),
            api_key: Self::resolve_api_key(),
            max_context_size: Self::resolve_max_context_size(),
            semantic_cache: Self::resolve_semantic_cache(path),
        })
    }

    /// Truncate context string so it fits within `max_context_size` bytes,
    /// preserving complete chunk blocks (each starts with `[file_path:line]`).
    pub fn trim_context(&self, context: String) -> String {
        if context.len() <= self.max_context_size {
            return context;
        }

        let mut result = String::new();
        let mut total = 0usize;

        // Each chunk block starts with `[file_path:line]`
        let separator = "\n\n";
        for part in context.split(separator) {
            let block_len = if result.is_empty() {
                part.len()
            } else {
                separator.len() + part.len()
            };

            // Don't add this chunk if it would overflow. The first chunk is
            // always included (the `result.is_empty()` guard) so that even a
            // tiny budget yields at least one complete chunk.
            if !result.is_empty() && total + block_len > self.max_context_size {
                break;
            }

            if !result.is_empty() {
                result.push_str(separator);
            }
            result.push_str(part);
            total += block_len;
        }

        // Truncate the last chunk to fit exactly, then add ellipsis
        if result.len() > self.max_context_size {
            let target = self.max_context_size - "…".len();
            if let Some(trunc_pos) = result.char_indices().take(target).last() {
                result.truncate(trunc_pos.0);
            }
            result.push('…');
        }

        result
    }
}

/// Validate that a string parameter does not exceed the maximum allowed length.
fn validate_query_length(value: &str, field_name: &str) -> Result<(), (StatusCode, String)> {
    if value.len() > MAX_QUERY_LENGTH_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Field '{}' exceeds maximum length of {} characters (got {})",
                field_name,
                MAX_QUERY_LENGTH_CHARS,
                value.len()
            ),
        ));
    }
    Ok(())
}

/// Extract the Bearer token from the Authorization header, if present.
fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

/// Enforce Bearer token auth on a request. Returns 401 if unauthorized, or None if authorized/public.
fn enforce_auth(
    headers: &axum::http::HeaderMap,
    api_key: &Option<String>,
    path: &str,
) -> Option<StatusCode> {
    // /status is always public — no auth required
    if path == "/status" {
        return None;
    }

    let expected = {
        let key = api_key.as_ref()?.as_str();
        key
    };

    let provided = extract_bearer_token(headers);
    if provided.as_deref() != Some(expected) {
        return Some(StatusCode::UNAUTHORIZED);
    }

    None // authorized
}

/// Build the API router.
pub fn build_router(state: AppState) -> Router {
    // Restrictive CORS: only allow same-origin requests (local dev / IDE plugins)
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::exact(axum::http::HeaderValue::from_static(
            "http://127.0.0.1",
        )))
        .allow_methods(vec![axum::http::Method::GET, axum::http::Method::POST]);

    Router::new()
        .route("/status", get(status_handler))
        .route("/search", post(search_handler))
        .route("/query", post(query_handler))
        .route("/query/stream", get(query_stream_handler))
        .layer(cors)
        .with_state(state)
        .fallback(handler_not_found)
}

/// Request body for `/search` — JSON payload with query and optional top_k.
#[derive(Debug, Deserialize)]
struct SearchQueryBody {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

fn default_top_k() -> usize {
    5
}

/// Fallback handler for unmatched routes.
async fn handler_not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Not Found")
}

/// GET /status — returns index metadata (no sensitive paths exposed).
#[tracing::instrument(level = "info", skip_all, fields(path = "/status"))]
async fn status_handler(state: axum::extract::State<AppState>) -> JsonResponse<serde_json::Value> {
    let file_path = state.0.store.path.join("index.jsonl");
    let total_chunks = match std::fs::File::open(file_path) {
        Ok(file) => {
            let reader = std::io::BufReader::new(file);
            reader
                .lines()
                .filter(|line| line.as_ref().map(|l| !l.trim().is_empty()).unwrap_or(false))
                .count()
        }
        Err(_) => 0,
    };

    JsonResponse(serde_json::json!({
        "total_chunks": total_chunks,
        "endpoint": state.0.llm_endpoint.clone(),
    }))
}

/// POST /search — semantic search over indexed chunks (no LLM).
#[tracing::instrument(level = "info", skip_all)]
async fn search_handler(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    Json(params): Json<SearchQueryBody>,
) -> impl axum::response::IntoResponse {
    // Enforce Bearer token auth when API key is configured
    if let Some(unauth_status) = enforce_auth(&headers, &state.api_key, "/search") {
        return (
            unauth_status,
            serde_json::to_string(&serde_json::json!({"error": "Unauthorized"})).unwrap(),
        );
    }

    // Sliding-window rate limit check (per-client IP)
    let client_key = RateLimiter::resolve_key(&headers, "127.0.0.1");
    if !state.rate_limiter.check(&client_key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::to_string(&serde_json::json!({"error": "Rate limit exceeded"})).unwrap(),
        );
    }

    // Validate query length to prevent resource exhaustion / prompt injection
    if let Err((_status, msg)) = validate_query_length(&params.query, "query") {
        return (
            StatusCode::BAD_REQUEST,
            serde_json::to_string(&serde_json::json!({"error": msg})).unwrap(),
        );
    }

    let query_embedding =
        match rust_rag_core::embedding::embed(&params.query) {
            Ok(v) => v,
            Err(e) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::to_string(
                    &serde_json::json!({"error": format!("Embed failed: {}", sanitize_error(&e))}),
                )
                .unwrap(),
            ),
        };

    let results =
        state
            .0
            .store
            .hybrid_search(&query_embedding, &params.query, params.top_k, 0.7, None);

    match results {
        Ok(results) => (
            StatusCode::OK,
            serde_json::to_string(&serde_json::json!({ "results": results })).unwrap(),
        ),
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::to_string(
                    &serde_json::json!({"error": format!("Search failed: {}", sanitize_error(&e))}),
                )
                .unwrap(),
            )
        }
    }
}

/// POST /query — full RAG: search + LLM answer with citations.
#[tracing::instrument(level = "info", skip_all, fields(question))]
async fn query_handler(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<QueryBody>,
) -> impl axum::response::IntoResponse {
    // Enforce Bearer token auth when API key is configured
    if let Some(unauth_status) = enforce_auth(&headers, &state.api_key, "/query") {
        return (
            unauth_status,
            serde_json::to_string(&serde_json::json!({"error": "Unauthorized"})).unwrap(),
        );
    }

    // Sliding-window rate limit check (per-client IP)
    let client_key = RateLimiter::resolve_key(&headers, "127.0.0.1");
    if !state.rate_limiter.check(&client_key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::to_string(&serde_json::json!({"error": "Rate limit exceeded"})).unwrap(),
        );
    }

    // Validate question length to prevent resource exhaustion / prompt injection
    if let Err((_, msg)) = validate_query_length(&body.question, "question") {
        return (
            StatusCode::BAD_REQUEST,
            serde_json::to_string(&serde_json::json!({"error": msg})).unwrap(),
        );
    }

    let config = rust_rag_core::config::Config::find().unwrap_or_default();
    let top_k = config.llm_config().top_k;

    // Check semantic cache before doing search + LLM call.
    if let Some(cached) = state.semantic_cache.lookup(&body.question) {
        return (
            StatusCode::OK,
            serde_json::to_string(&serde_json::json!({
                "answer": cached,
                "citations": Vec::<serde_json::Value>::new(),
                "cached": true,
            }))
            .unwrap(),
        );
    }

    let query_embedding =
        match rust_rag_core::embedding::embed(&body.question) {
            Ok(v) => v,
            Err(e) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::to_string(
                    &serde_json::json!({"error": format!("Embed failed: {}", sanitize_error(&e))}),
                )
                .unwrap(),
            ),
        };

    let results = state
        .0
        .store
        .hybrid_search(&query_embedding, &body.question, top_k, 0.7, None);

    let results_vec: Vec<_> =
        match results {
            Ok(r) => r,
            Err(e) => return (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::to_string(
                    &serde_json::json!({"error": format!("Search failed: {}", sanitize_error(&e))}),
                )
                .unwrap(),
            ),
        };

    // Build context from search results and trim to max_context_size
    let raw_context: String = results_vec
        .iter()
        .map(|r| format!("[{}:{}]\n{}", r.file_path.display(), r.line_start, r.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let context = state.0.trim_context(raw_context);

    let system_prompt = rust_rag_core::constants::DEFAULT_SYSTEM_PROMPT;
    let user_message = format!("Question: {}\n\nRelevant code:\n{}", body.question, context);

    let endpoint = state.0.llm_endpoint.clone();
    let model = state.0.llm_model.clone();
    let http_client = std::sync::Arc::clone(&state.0.http_client);
    let answer_result = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        rust_rag_llm::ollama_client::LlmClient::chat_with_http_client(
            http_client,
            &endpoint,
            &model,
            system_prompt,
            &user_message,
        ),
    )
    .await;

    let (answer_text, was_ok) = match &answer_result {
        Ok(Ok(a)) => (a.clone(), true),
        Ok(Err(e)) => (format!("LLM error: {}", e), false),
        Err(_) => ("LLM server busy (request timed out)".to_string(), false),
    };

    // On successful LLM response, write back to semantic cache for future lookups.
    if was_ok {
        let _ = state
            .semantic_cache
            .write_back(&body.question, &answer_text);
    }

    let citations: Vec<_> = results_vec
        .iter()
        .map(|r| {
            serde_json::json!({
                "file_path": r.file_path.to_string_lossy(),
                "line_start": r.line_start,
                "line_end": r.line_end,
                "text": r.text,
            })
        })
        .collect();

    (
        StatusCode::OK,
        serde_json::to_string(&serde_json::json!({
            "answer": answer_text,
            "citations": citations,
        }))
        .unwrap(),
    )
}

#[derive(Debug, Deserialize)]
struct QueryBody {
    question: String,
}

/// Request parameters for `/query/stream` (SSE).
#[derive(Debug, Deserialize)]
struct QueryStreamQuery {
    question: String,
    #[allow(dead_code)] // used to be read from query params; now uses config default
    #[serde(default = "default_top_k")]
    top_k: usize,
}

#[tracing::instrument(level = "info", skip_all, fields(question))]
async fn query_stream_handler(
    state: axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<QueryStreamQuery>,
) -> axum::response::Response {
    // Enforce Bearer token auth when API key is configured
    if let Some(unauth_status) = enforce_auth(&headers, &state.api_key, "/query/stream") {
        return axum::response::Response::builder()
            .status(unauth_status)
            .body(Body::empty())
            .unwrap();
    }

    // Sliding-window rate limit check (per-client IP)
    let client_key = RateLimiter::resolve_key(&headers, "127.0.0.1");
    if !state.rate_limiter.check(&client_key) {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::TOO_MANY_REQUESTS)
            .body(Body::empty())
            .unwrap();
    }

    // Validate question length to prevent resource exhaustion / prompt injection
    if let Err((_, msg)) = validate_query_length(&params.question, "question") {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::BAD_REQUEST)
            .body(Body::from(
                serde_json::to_string(&serde_json::json!({"error": msg})).unwrap_or_default(),
            ))
            .unwrap();
    }

    // Check semantic cache before doing search + LLM call.
    if let Some(cached) = state.semantic_cache.lookup(&params.question) {
        return axum::response::Response::builder()
            .status(axum::http::StatusCode::OK)
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .body(Body::from_stream(async_stream::stream! {
                let cached_chunks: Vec<_> = cached.as_bytes().chunks(512).collect();
                for chunk in cached_chunks {
                    yield Ok::<_, axum::BoxError>(bytes::Bytes::from(chunk.to_vec()));
                }
            }))
            .unwrap_or_else(|_| axum::response::Response::new(Body::from("error")));
    }

    let config = rust_rag_core::config::Config::find().unwrap_or_default();
    let top_k = config.llm_config().top_k;

    let query_embedding = match rust_rag_core::embedding::embed(&params.question) {
        Ok(v) => v,
        Err(_) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap()
        }
    };

    let results =
        match state
            .0
            .store
            .hybrid_search(&query_embedding, &params.question, top_k, 0.7, None)
        {
            Ok(r) => r,
            Err(_) => {
                return axum::response::Response::builder()
                    .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .unwrap()
            }
        };

    // Build context and trim to max_context_size
    let raw_context: String = results
        .iter()
        .map(|r| format!("[{}:{}]\n{}", r.file_path.display(), r.line_start, r.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let context = state.0.trim_context(raw_context);

    let system_prompt = rust_rag_core::constants::DEFAULT_SYSTEM_PROMPT;
    let user_message = format!(
        "Question: {}\n\nRelevant code:\n{}",
        params.question, context
    );

    // Create an mpsc channel bridge: LLM stream -> channel -> axum Body -> SSE response
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, axum::BoxError>>(16);

    {
        let tx_clone = tx.clone();
        // Spawn the LLM streaming task directly as async — no nested runtime needed.
        // Use shared HTTP client from AppState for connection pooling.
        // Wrap in tokio::time::timeout so a hung model still terminates after 5 minutes.
        // Also collect all text chunks to build a full answer for cache write-back.
        let endpoint = state.0.llm_endpoint.clone();
        let model = state.0.llm_model.clone();
        let http_client = std::sync::Arc::clone(&state.0.http_client);
        let question_for_cache = params.question.clone();
        let semantic_cache_clone = Arc::clone(&state.semantic_cache);
        let full_answer = std::sync::RwLock::new(String::new());

        tokio::spawn(async move {
            let client = rust_rag_llm::ollama_client::LlmClient::new_with_http_client(
                &endpoint,
                &model,
                http_client,
            );
            // Timeout the entire streaming call — prevents leaked SSE connections.
            if tokio::time::timeout(std::time::Duration::from_secs(300), async {
                let mut stream = client.complete_stream_chunks(system_prompt, &user_message);
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(text) => {
                            // Append to the full answer buffer (single-writer, safe).
                            if let Ok(mut buf) = full_answer.write() {
                                buf.push_str(&text);
                            }
                            let sse = format!("data: {}\n\n", text);
                            let _ = tx_clone
                                .send(Ok(bytes::Bytes::from(sse.into_bytes())))
                                .await;
                        }
                        Err(e) => {
                            let err_sse = format!("event: error\ndata: {}\n\n", e);
                            let _ = tx_clone
                                .send(Ok(bytes::Bytes::from(err_sse.into_bytes())))
                                .await;
                            break;
                        }
                    }
                }
            })
            .await
            .is_err()
            {
                // Stream timed out — send a terminal event.
                let err_sse = "event: error\ndata: LLM response timed out\n\n";
                let _ = tx_clone
                    .send(Ok(bytes::Bytes::from(err_sse.as_bytes().to_vec())))
                    .await;
            }

            // After streaming completes, write back to semantic cache.
            if let Ok(answer) = full_answer.read() {
                let question = question_for_cache.clone();
                let answer = answer.clone();
                tokio::spawn(async move {
                    let _ = semantic_cache_clone.write_back(&question, &answer);
                });
            }
        });
    }

    axum::response::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .header("X-Accel-Buffering", "no")
        .body(Body::from_stream(async_stream::stream! {
            let mut rx = rx;
            while let Some(item) = rx.recv().await {
                yield item;
            }
        }))
        .unwrap_or_else(|_| axum::response::Response::new(Body::from("error")))
}
