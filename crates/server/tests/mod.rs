/// Integration tests for the HTTP API server and MCP server.
///
/// These tests modify environment variables, so they must run sequentially.
/// We use a module-level mutex to serialize access.
use rust_rag_server::{AppState, RateLimiter};
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn create_test_store() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.jsonl"), "").unwrap_or_default();
    dir
}

// ── trim_context tests (use ENV_LOCK to avoid race on env vars) ──────

#[test]
fn test_trim_context_no_truncation_needed() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("RUSRAG_MAX_CONTEXT_SIZE");
    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();

    // Default max is 12_000, our context is tiny.
    let ctx = "[src/lib.rs:10]\nfn hello() {}\n\n[src/main.rs:5]\nfn main() {}".to_string();
    let trimmed = state.trim_context(ctx);
    assert_eq!(trimmed.len(), 59); // unchanged (default budget far exceeds context)

    std::env::remove_var("RUSRAG_MAX_CONTEXT_SIZE");
}

#[test]
fn test_trim_context_fits_exactly() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("RUSRAG_MAX_CONTEXT_SIZE", "100");
    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();

    // Context is 47 bytes, budget is 100 — fits easily.
    let ctx = "[a.rs:1]\nfn a() {}\n\n[b.rs:2]\nfn b() {}".to_string();
    assert!(ctx.len() <= 100);
    let trimmed = state.trim_context(ctx.clone());
    assert_eq!(trimmed, ctx);

    std::env::remove_var("RUSRAG_MAX_CONTEXT_SIZE");
}

#[test]
fn test_trim_context_truncates_at_boundary_preserving_chunks() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("RUSRAG_MAX_CONTEXT_SIZE", "30");
    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();

    let ctx = "[a.rs:1]\nfn a() {}\n\n[b.rs:2]\nfn b() {}\n\n[c.rs:3]\nfn c() {}".to_string();
    assert!(ctx.len() > 30);
    let trimmed = state.trim_context(ctx);

    // Must not contain any "b.rs" or "c.rs" — only the first chunk.
    assert!(!trimmed.contains("b.rs"));
    assert!(!trimmed.contains("c.rs"));
    assert!(trimmed.len() <= 30);

    std::env::remove_var("RUSRAG_MAX_CONTEXT_SIZE");
}

#[test]
fn test_trim_context_ellipsis_on_overflow() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Very small budget — even one chunk doesn't fit fully.
    std::env::set_var("RUSRAG_MAX_CONTEXT_SIZE", "10");
    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();

    let ctx = "[a.rs:1]\nfn a() {}\n\n[b.rs:2]\nfn b() {}".to_string();
    let trimmed = state.trim_context(ctx);

    assert!(trimmed.ends_with('…'));
    assert!(trimmed.len() <= 10);

    std::env::remove_var("RUSRAG_MAX_CONTEXT_SIZE");
}

// ── RateLimiter tests ────────────────────────────────────────────────

#[test]
fn test_rate_limiter_allows_within_budget() {
    let limiter = RateLimiter::new(5); // 5 requests per minute
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
    assert!(limiter.check("client-1"));
}

#[test]
fn test_rate_limiter_rejects_over_budget() {
    let limiter = RateLimiter::new(3); // 3 requests per minute
    assert!(limiter.check("x"));
    assert!(limiter.check("x"));
    assert!(limiter.check("x"));
    assert!(
        !limiter.check("x"),
        "should be rejected after budget exhausted"
    );
}

#[test]
fn test_rate_limiter_independent_per_client() {
    let limiter = RateLimiter::new(2);
    // alice uses her full budget.
    assert!(limiter.check("alice"));
    assert!(limiter.check("alice"));
    // bob is independent — his own budget untouched.
    assert!(limiter.check("bob"));
    // alice exhausted, should be rejected.
    assert!(
        !limiter.check("alice"),
        "alice should be rejected after 2 requests"
    );
    // bob still has one slot left.
    assert!(limiter.check("bob"), "bob should still be allowed");
}

#[test]
fn test_rate_limiter_resolves_key_from_headers() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-Forwarded-For", "10.0.0.5".parse().unwrap());

    let key = RateLimiter::resolve_key(&headers, "fallback");
    assert_eq!(key, "10.0.0.5");
}

#[test]
fn test_rate_limiter_falls_back_to_x_real_ip() {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert("X-Real-Ip", "203.0.113.42".parse().unwrap());

    let key = RateLimiter::resolve_key(&headers, "fallback");
    assert_eq!(key, "203.0.113.42");
}

#[test]
fn test_rate_limiter_uses_fallback_when_no_headers() {
    let headers = axum::http::HeaderMap::new();
    let key = RateLimiter::resolve_key(&headers, "anonymous");
    assert_eq!(key, "anonymous");
}

// ── existing tests ────────────────────────────────────────────────────

#[test]
fn test_app_state_no_api_key_when_not_set() {
    let _guard = ENV_LOCK.lock().unwrap();

    // Ensure no API key is set
    std::env::remove_var("RUSRAG_API_KEY");

    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();
    assert_eq!(state.api_key, None);
}

#[test]
fn test_app_state_resolves_api_key_from_env() {
    let _guard = ENV_LOCK.lock().unwrap();

    std::env::remove_var("RUSRAG_API_KEY");
    std::env::set_var("RUSRAG_API_KEY", "my-secret-key");

    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();
    assert_eq!(state.api_key.as_deref(), Some("my-secret-key"));

    std::env::remove_var("RUSRAG_API_KEY");
}

#[test]
fn test_app_state_ignores_empty_api_key() {
    let _guard = ENV_LOCK.lock().unwrap();

    std::env::remove_var("RUSRAG_API_KEY");
    std::env::set_var("RUSRAG_API_KEY", "");

    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();
    assert_eq!(state.api_key, None);

    std::env::remove_var("RUSRAG_API_KEY");
}

#[test]
fn test_build_router_creates_valid_app() {
    // This test doesn't need env isolation — it uses the current state.
    let dir = create_test_store();
    let state = AppState::from_path(dir.path(), 60).unwrap();
    let _router = rust_rag_server::build_router(state);
}

// ── MCP server tests ────────────────────────────────────────────────
mod mcp_tests;
