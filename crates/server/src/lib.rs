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
use rust_rag_core::{config, vector_store::VectorStore};
use rust_rag_llm::ChatBackend;

use serde::Deserialize;
use std::{path::Path, sync::Arc};
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;

/// Request state shared across handlers.
pub struct AppState {
    pub store: std::sync::Arc<VectorStore>,
    /// Rate limiter: permits per minute for non-stream endpoints.
    pub rate_limiter: Arc<Semaphore>,
    /// Shared HTTP client for LLM requests — enables connection pooling.
    pub http_client: std::sync::Arc<reqwest::Client>,
    /// Resolved LLM endpoint (from config / env / default).
    pub llm_endpoint: String,
    /// Resolved LLM model name (from config / env / default).
    pub llm_model: String,
}

impl Clone for AppState {
    fn clone(&self) -> Self {
        Self {
            store: std::sync::Arc::clone(&self.store),
            rate_limiter: Arc::clone(&self.rate_limiter),
            http_client: std::sync::Arc::clone(&self.http_client),
            llm_endpoint: self.llm_endpoint.clone(),
            llm_model: self.llm_model.clone(),
        }
    }
}

impl AppState {
    /// Resolve LLM endpoint with priority: config > LLAMA_ENDPOINT env > default.
    fn resolve_endpoint() -> String {
        let cfg = rust_rag_core::config::Config::find().ok();
        std::env::var("LLAMA_ENDPOINT")
            .ok()
            .or_else(|| {
                cfg.as_ref().and_then(|c| c.llm_config().endpoint.clone())
            })
            .unwrap_or_else(|| "http://localhost:8080".to_string())
    }

    /// Resolve LLM model with priority: config > LLAMA_MODEL env > default.
    fn resolve_model() -> String {
        let cfg = rust_rag_core::config::Config::find().ok();
        std::env::var("LLAMA_MODEL")
            .ok()
            .or_else(|| {
                cfg.as_ref().and_then(|c| c.llm_config().model.clone())
            })
            .unwrap_or_else(|| {
                "Qwen3.6-35B-A3B-Uncensored-HauhauCS-Aggressive-IQ3_M.gguf".to_string()
            })
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
        Ok(Self {
            store: std::sync::Arc::new(store),
            rate_limiter: Arc::new(Semaphore::new(rate_limit_per_min as usize)),
            http_client: rust_rag_llm::ollama_client::LlmClient::shared_http_client(),
            llm_endpoint: Self::resolve_endpoint(),
            llm_model: Self::resolve_model(),
        })
    }

    /// Create app state from a custom path.
    pub fn from_path(path: &Path, rate_limit_per_min: u32) -> Result<Self> {
        let store = VectorStore::open(path)?;
        Ok(Self {
            store: std::sync::Arc::new(store),
            rate_limiter: Arc::new(Semaphore::new(rate_limit_per_min as usize)),
            http_client: rust_rag_llm::ollama_client::LlmClient::shared_http_client(),
            llm_endpoint: Self::resolve_endpoint(),
            llm_model: Self::resolve_model(),
        })
    }
}

/// Request parameters for `/search`.
#[derive(Debug, Deserialize)]
struct SearchQuery {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

fn default_top_k() -> usize {
    5
}

/// Build the API router.
pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::permissive();

    Router::new()
        .route("/status", get(status_handler))
        .route("/search", post(search_handler))
        .route("/query", post(query_handler))
        .route("/query/stream", get(query_stream_handler))
        .layer(cors)
        .with_state(state)
}

/// GET /status — returns index metadata.
async fn status_handler(state: axum::extract::State<AppState>) -> JsonResponse<serde_json::Value> {
    let index_path = state.0.store.path.join("index.jsonl");
    let content = if index_path.exists() {
        std::fs::read_to_string(&index_path).unwrap_or_default()
    } else {
        String::new()
    };
    let total_chunks = content.lines().filter(|l| !l.trim().is_empty()).count();

    JsonResponse(serde_json::json!({
        "workspace_root": state.0.store.path.display().to_string(),
        "total_chunks": total_chunks,
        "index_path": index_path.to_str().unwrap_or(""),
    }))
}

/// POST /search — semantic search over indexed chunks (no LLM).
async fn search_handler(
    state: axum::extract::State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<JsonResponse<serde_json::Value>, StatusCode> {
    if state.rate_limiter.clone().try_acquire_owned().is_err() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let query_embedding = match rust_rag_core::embedding::embed(&params.query) {
        Ok(v) => v,
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let results =
        state
            .0
            .store
            .hybrid_search(&query_embedding, &params.query, params.top_k, 0.7, None);

    match results {
        Ok(results) => Ok(JsonResponse(serde_json::json!({ "results": results }))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// POST /query — full RAG: search + LLM answer with citations.
async fn query_handler(
    state: axum::extract::State<AppState>,
    Json(body): Json<QueryBody>,
) -> Result<JsonResponse<serde_json::Value>, StatusCode> {
    if state.rate_limiter.clone().try_acquire_owned().is_err() {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    let config = config::Config::find().unwrap_or_default();
    let top_k = config.llm_config().top_k;

    // Embed the question using core embedding singleton
    let query_embedding = match rust_rag_core::embedding::embed(&body.question) {
        Ok(v) => v,
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let results = state
        .0
        .store
        .hybrid_search(&query_embedding, &body.question, top_k, 0.7, None);

    let results_vec: Vec<_> = match results {
        Ok(r) => r,
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    // Build context from search results
    let context: String = results_vec
        .iter()
        .map(|r| format!("[{}:{}]\n{}", r.file_path.display(), r.line_start, r.text))
        .collect::<Vec<_>>()
        .join("\n\n");

    let system_prompt = rust_rag_core::constants::DEFAULT_SYSTEM_PROMPT;
    let user_message = format!("Question: {}\n\nRelevant code:\n{}", body.question, context);

  // Call LLM using the shared HTTP client from AppState for connection pooling.
    let endpoint = state.0.llm_endpoint.clone();
    let model = state.0.llm_model.clone();
    let http_client = std::sync::Arc::clone(&state.0.http_client);
    let answer = tokio::task::spawn_blocking(move || {
        rust_rag_llm::ollama_client::LlmClient::chat_with_http_client(http_client, &endpoint, &model, system_prompt, &user_message)
    })
    .await;

    let answer_text = match answer {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => format!("LLM error: {}", e),
        Err(_) => "LLM server busy".to_string(),
    };

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

    Ok(JsonResponse(serde_json::json!({
        "answer": answer_text,
        "citations": citations,
    })))
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

async fn query_stream_handler(
    state: axum::extract::State<AppState>,
    Query(params): Query<QueryStreamQuery>,
) -> axum::response::Response {
    let config = config::Config::find().unwrap_or_default();
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

    let context: String = results
        .iter()
        .map(|r| format!("[{}:{}]\n{}", r.file_path.display(), r.line_start, r.text))
        .collect::<Vec<_>>()
        .join("\n\n");

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
        let endpoint = state.0.llm_endpoint.clone();
        let model = state.0.llm_model.clone();
        let http_client = std::sync::Arc::clone(&state.0.http_client);
        tokio::spawn(async move {
            let client = rust_rag_llm::ollama_client::LlmClient::new_with_http_client(
                &endpoint, &model, http_client,
            );
            let mut stream = client.complete_stream_chunks(system_prompt, &user_message);
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(text) => {
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
