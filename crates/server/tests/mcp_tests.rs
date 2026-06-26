use std::sync::Mutex;
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn create_test_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    // Create a test file with known content for testing line ranges
    std::fs::create_dir_all(path.join("src")).unwrap();
    std::fs::write(
        path.join("src/test.rs"),
        r#"// Line 1
fn function_a() -> i32 {  // Line 2
    42                    // Line 3
}                       // Line 4

// Line 5
struct TestStruct {       // Line 6
    field: i32,           // Line 7
}                         // Line 8

// Line 9
fn function_b() {         // Line 10
    let x = 1;            // Line 11
    let y = 2;            // Line 12
    println!("{}", x + y);// Line 13
}                         // Line 14
"#,
    )
    .unwrap();

    dir
}

fn with_lock<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f()
}

#[test]
fn test_rag_file_read_full_file() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test reading full file without line range
        let args = json!({
            "file_path": "src/test.rs"
        });

        let result = rag_file_read_tool(&args).expect("Should succeed");

        // Verify response structure
        assert!(result.is_object());
        let obj = result.as_object().unwrap();

        assert_eq!(
            obj.get("file_path").unwrap().as_str().unwrap(),
            "src/test.rs"
        );
        assert!(obj.get("content").unwrap().is_string());
        assert_eq!(
            obj.get("content_length").unwrap().as_u64().unwrap() as usize,
            obj.get("content").unwrap().as_str().unwrap().len()
        );

        // Verify line range info for full file
        let line_range = obj.get("line_range").unwrap().as_object().unwrap();
        assert_eq!(line_range.get("start").unwrap().as_u64().unwrap(), 1);
        assert_eq!(line_range.get("end").unwrap().as_u64().unwrap(), 16);
        assert_eq!(line_range.get("total_lines").unwrap().as_u64().unwrap(), 16);

        // Verify content contains expected text
        let content = obj.get("content").unwrap().as_str().unwrap();
        assert!(content.contains("fn function_a()"));
        assert!(content.contains("struct TestStruct"));
        assert!(content.contains("fn function_b()"));

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}

#[test]
fn test_rag_file_read_line_range_start_middle() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test reading lines 6-11 (should include struct definition and field)
        // Line 6: "// Line 5"
        // Line 7: "struct TestStruct {       // Line 6"
        // Line 8: "    field: i32,           // Line 7"
        // Line 9: "}                         // Line 8"
        // Line 10: "" (empty line)
        // Line 11: "// Line 9"
        let args = json!({
            "file_path": "src/test.rs",
            "line_start": 6,
            "line_end": 11
        });

        let result = rag_file_read_tool(&args).expect("Should succeed");

        // Verify response structure
        assert!(result.is_object());
        let obj = result.as_object().unwrap();

        assert_eq!(
            obj.get("file_path").unwrap().as_str().unwrap(),
            "src/test.rs"
        );

        // Verify line range info
        let line_range = obj.get("line_range").unwrap().as_object().unwrap();
        assert_eq!(line_range.get("start").unwrap().as_u64().unwrap(), 6);
        assert_eq!(line_range.get("end").unwrap().as_u64().unwrap(), 11);
        assert_eq!(line_range.get("total_lines").unwrap().as_u64().unwrap(), 16);

        // Verify content contains expected lines
        let content = obj.get("content").unwrap().as_str().unwrap();
        assert_eq!(content.lines().count(), 6); // 6,7,8,9,10,11 = 6 lines

        // Should contain the struct definition and field
        assert!(content.contains("struct TestStruct"));
        assert!(content.contains("field: i32"));

        // Should NOT contain function_a or function_b
        assert!(!content.contains("function_a"));
        assert!(!content.contains("function_b"));

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}

#[test]
fn test_rag_file_read_line_range_single_line() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test reading a single line (line 8 - the field line)
        let args = json!({
            "file_path": "src/test.rs",
            "line_start": 8,
            "line_end": 8
        });

        let result = rag_file_read_tool(&args).expect("Should succeed");

        // Verify response structure
        assert!(result.is_object());
        let obj = result.as_object().unwrap();

        // Verify line range info
        let line_range = obj.get("line_range").unwrap().as_object().unwrap();
        assert_eq!(line_range.get("start").unwrap().as_u64().unwrap(), 8);
        assert_eq!(line_range.get("end").unwrap().as_u64().unwrap(), 8);
        assert_eq!(line_range.get("total_lines").unwrap().as_u64().unwrap(), 16);

        // Verify content is exactly one line
        let content = obj.get("content").unwrap().as_str().unwrap();
        assert_eq!(content.lines().count(), 1);
        assert_eq!(content, "    field: i32,           // Line 7");

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}

#[test]
fn test_rag_file_read_line_range_beyond_file_bounds() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test reading beyond file bounds (start beyond end of file)
        let args = json!({
            "file_path": "src/test.rs",
            "line_start": 20,  // File only has 16 lines
            "line_end": 25
        });

        let result = rag_file_read_tool(&args).expect("Should succeed");

        // Verify response structure
        assert!(result.is_object());
        let obj = result.as_object().unwrap();

        // Verify line range info - should show empty content
        let line_range = obj.get("line_range").unwrap().as_object().unwrap();
        assert_eq!(line_range.get("start").unwrap().as_u64().unwrap(), 20);
        assert_eq!(line_range.get("end").unwrap().as_u64().unwrap(), 0); // Adjusted to 0 when start > total
        assert_eq!(line_range.get("total_lines").unwrap().as_u64().unwrap(), 16);

        // Verify content is empty
        let content = obj.get("content").unwrap().as_str().unwrap();
        assert_eq!(content, "");
        assert_eq!(obj.get("content_length").unwrap().as_u64().unwrap(), 0);

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}

#[test]
fn test_rag_file_read_invalid_line_range() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test invalid line range (end < start)
        let args = json!({
            "file_path": "src/test.rs",
            "line_start": 10,
            "line_end": 5
        });

        let result = rag_file_read_tool(&args);
        assert!(
            result.is_err(),
            "Should return error for invalid line range"
        );

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}

#[test]
fn test_rag_file_read_line_start_less_than_one() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test invalid line start (< 1)
        let args = json!({
            "file_path": "src/test.rs",
            "line_start": 0,
            "line_end": 5
        });

        let result = rag_file_read_tool(&args);
        assert!(result.is_err(), "Should return error for line_start < 1");

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}

#[test]
fn test_rag_file_read_backward_compatibility() {
    with_lock(|| {
        let dir = create_test_workspace();

        // Set workspace root
        std::env::set_var("RUSRAG_WORKSPACE", dir.path().to_string_lossy().to_string());

        // Import the MCP server functions
        use rust_rag_server::mcp::rag_file_read_tool;
        use serde_json::json;

        // Test with old format (no line range parameters) - should still work
        let args = json!({
            "file_path": "src/test.rs"
            // No line_start or line_end specified
        });

        let result = rag_file_read_tool(&args).expect("Should succeed");

        // Verify response structure
        assert!(result.is_object());
        let obj = result.as_object().unwrap();

        assert_eq!(
            obj.get("file_path").unwrap().as_str().unwrap(),
            "src/test.rs"
        );
        assert!(obj.get("content").unwrap().is_string());

        // Verify line range info shows full file
        let line_range = obj.get("line_range").unwrap().as_object().unwrap();
        assert_eq!(line_range.get("start").unwrap().as_u64().unwrap(), 1);
        assert_eq!(line_range.get("end").unwrap().as_u64().unwrap(), 16);
        assert_eq!(line_range.get("total_lines").unwrap().as_u64().unwrap(), 16);

        // Clean up
        std::env::remove_var("RUSRAG_WORKSPACE");
    });
}
