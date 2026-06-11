/// Integration tests for the HTTP API server.
///
/// These tests modify environment variables, so they must run sequentially.
/// We use a module-level mutex to serialize access.
use rust_rag_server::AppState;
use std::sync::Mutex;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn create_test_store() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.jsonl"), "").unwrap_or_default();
    dir
}

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
