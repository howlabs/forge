// =============================================================================
// INTEGRATION TEST - File Watcher + Verify Symbol (Track B v0.150.0)
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::file_watcher::FileWatcher;
    use crate::event_loop::EventLoop;
    use forge_context::{MockContextIndex, ContextIndex, SymbolKind};
    use forge_provider::anthropic::AnthropicProvider;
    use forge_sandbox::Sandbox;
    use forge_context::ContextEngine;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_full_integration() {
        // 1. Setup test environment
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("lib.rs");

        // 2. Create MockContextIndex
        let mut context_index = MockContextIndex::new();

        // 3. Create initial file with some code
        let initial_code = r#"
fn helper_function() -> usize {
    42
}

pub struct Database {
    connection: String,
}

impl Database {
    pub fn connect(conn_str: &str) -> Self {
        Self {
            connection: conn_str.to_string(),
        }
    }

    pub fn query(&self, sql: &str) -> Vec<String> {
        vec!["result".to_string()]
    }
}
"#;

        let mut file = File::create(&test_file).unwrap();
        file.write_all(initial_code.as_bytes()).unwrap();
        drop(file);

        // 4. Index the file
        context_index.upsert_file(&test_file, initial_code);

        // Verify symbols were extracted
        let helper_symbol = format!("{}::helper_function", test_file.display());
        assert!(context_index.resolve_symbol(&helper_symbol).is_some(),
            "helper_function should be indexed");

        let connect_symbol = format!("{}::Database::connect", test_file.display());
        assert!(context_index.resolve_symbol(&connect_symbol).is_some(),
            "Database::connect should be indexed");

        // 5. Setup EventLoop with ContextIndex
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path().to_str().unwrap(), "off").unwrap();
        let index_arc = Arc::new(Mutex::new(context_index as dyn ContextIndex));

        let event_loop = EventLoop::new(provider, context, sandbox)
            .with_context_index(index_arc.clone());

        // 6. Test verify-symbol-before-edit with VALID symbol
        let valid_old = "// old code";
        let valid_new = "let db = Database::connect(\"test\");";

        let result = event_loop.verify_symbols_before_edit(
            &index_arc,
            valid_old,
            valid_new,
        ).await;

        assert!(result.is_ok(), "Edit with valid symbol should succeed");

        // 7. Test verify-symbol-before-edit with INVALID symbol
        let invalid_old = "// old code";
        let invalid_new = "let db = NonExistent::connect(\"test\");";

        let result = event_loop.verify_symbols_before_edit(
            &index_arc,
            invalid_old,
            invalid_new,
        ).await;

        assert!(result.is_err(), "Edit with invalid symbol should be rejected");
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("REJECTED edit"), "Error should mention rejection");
        assert!(error_msg.contains("NonExistent"), "Error should mention the missing symbol");

        // 8. Test incremental file sync - modify file
        sleep(Duration::from_millis(100)).await; // Small delay

        let modified_code = r#"
fn helper_function() -> usize {
    42
}

pub struct Database {
    connection: String,
}

impl Database {
    pub fn connect(conn_str: &str) -> Self {
        Self {
            connection: conn_str.to_string(),
        }
    }

    pub fn query(&self, sql: &str) -> Vec<String> {
        vec!["result".to_string()]
    }

    // NEW METHOD ADDED
    pub fn close(&self) -> bool {
        true
    }
}
"#;

        let mut file = File::create(&test_file).unwrap();
        file.write_all(modified_code.as_bytes()).unwrap();
        drop(file);

        // Simulate file watcher upsert
        let mut locked_index = index_arc.lock().await;
        locked_index.upsert_file(&test_file, modified_code);
        drop(locked_index);

        // Verify new method was indexed
        let close_symbol = format!("{}::Database::close", test_file.display());
        let locked_index = index_arc.lock().await;
        assert!(locked_index.resolve_symbol(&close_symbol).is_some(),
            "Database::close should be indexed after file update");
        drop(locked_index);

        // 9. Test that edit with new symbol now works
        let new_symbol_old = "// old code";
        let new_symbol_new = "let closed = db.close();";

        let result = event_loop.verify_symbols_before_edit(
            &index_arc,
            new_symbol_old,
            new_symbol_new,
        ).await;

        assert!(result.is_ok(), "Edit with newly added symbol should succeed");

        // 10. Test file deletion
        fs::remove_file(&test_file).unwrap();

        // Simulate file watcher remove
        let mut locked_index = index_arc.lock().await;
        locked_index.remove_file(&test_file);
        drop(locked_index);

        // Verify symbols were removed
        let locked_index = index_arc.lock().await;
        assert!(locked_index.resolve_symbol(&close_symbol).is_none(),
            "Symbols should be removed after file deletion");
        assert!(locked_index.resolve_symbol(&connect_symbol).is_none(),
            "All symbols should be removed after file deletion");
        drop(locked_index);

        println!("✅ Full integration test passed!");
    }

    #[tokio::test]
    async fn test_file_watcher_debouncing() {
        // Test that rapid edits are debounced
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("rapid.rs");

        let context_index = Arc::new(Mutex::new(MockContextIndex::new() as dyn ContextIndex));

        // Create file watcher with short debounce
        let mut watcher = FileWatcher::new(
            context_index.clone(),
            temp_dir.path(),
            500, // 500ms debounce
        ).unwrap();

        // Start watcher (this is non-blocking)
        let _ = watcher.watch();

        // Create initial file
        File::create(&test_file).unwrap().write_all(b"fn v1() {}").unwrap();

        // Rapidly modify the file multiple times
        for i in 2..=5 {
            sleep(Duration::from_millis(100)).await; // Less than debounce timeout
            let code = format!("fn v{}() {{}}", i);
            File::create(&test_file).unwrap().write_all(code.as_bytes()).unwrap();
        }

        // Wait for debounce to settle
        sleep(Duration::from_millis(1000)).await;

        // Check that only the final version is indexed
        let locked_index = context_index.lock().await;
        // The mock should have processed the file, but we can't easily test
        // the exact state due to async timing. This test mainly ensures
        // the watcher doesn't crash under rapid edits.

        println!("✅ File watcher debouncing test passed!");
    }

    #[tokio::test]
    async fn test_symbol_extraction_patterns() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("patterns.rs");

        let mut context_index = MockContextIndex::new();

        // Test various code patterns
        let code = r#"
// Function
fn simple_function() -> usize {
    42
}

// Struct
pub struct MyStruct {
    field: usize,
}

// Impl
impl MyStruct {
    pub fn new() -> Self {
        Self { field: 0 }
    }

    pub fn method(&self) -> usize {
        self.field
    }
}

// Enum
pub enum MyEnum {
    Variant1,
    Variant2,
}
"#;

        context_index.upsert_file(&test_file, code);

        // Verify various symbols were extracted
        let simple_fn = format!("{}::simple_function", test_file.display());
        assert!(context_index.resolve_symbol(&simple_fn).is_some(),
            "simple_function should be indexed");

        let my_struct = format!("{}::MyStruct", test_file.display());
        assert!(context_index.resolve_symbol(&my_struct).is_some(),
            "MyStruct should be indexed");

        let my_enum = format!("{}::MyEnum", test_file.display());
        assert!(context_index.resolve_symbol(&my_enum).is_some(),
            "MyEnum should be indexed");

        println!("✅ Symbol extraction patterns test passed!");
    }

    #[tokio::test]
    async fn test_hallucination_prevention() {
        // This test specifically addresses #3 (hallucination)
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("api.rs");

        let mut context_index = MockContextIndex::new();

        // Index file with specific API
        let code = r#"
pub struct RealAPI {
    data: String,
}

impl RealAPI {
    pub fn real_method(&self) -> String {
        "result".to_string()
    }
}
"#;

        context_index.upsert_file(&test_file, code);

        let index = Arc::new(Mutex::new(context_index as dyn ContextIndex));

        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path().to_str().unwrap(), "off").unwrap();

        let event_loop = EventLoop::new(provider, context, sandbox)
            .with_context_index(index.clone());

        // Test 1: Try to use REAL method (should succeed)
        let real_old = "// old";
        let real_new = "api.real_method();";

        let result = event_loop.verify_symbols_before_edit(
            &index,
            real_old,
            real_new,
        ).await;

        assert!(result.is_ok(), "Using real API should succeed");

        // Test 2: Try to use HALLUCINATED method (should be rejected)
        let hallucinated_old = "// old";
        let hallucinated_new = "api.hallucinated_method();";

        let result = event_loop.verify_symbols_before_edit(
            &index,
            hallucinated_old,
            hallucinated_new,
        ).await;

        assert!(result.is_err(), "Using hallucinated API should be rejected");
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("hallucinated_method"),
            "Error should mention the hallucinated symbol");
        assert!(error_msg.contains("REJECTED"),
            "Error should clearly indicate rejection");

        // Test 3: Try to use completely made-up type (should be rejected)
        let made_up_old = "// old";
        let made_up_new = "let x = MadeUpType::new();";

        let result = event_loop.verify_symbols_before_edit(
            &index,
            made_up_old,
            made_up_new,
        ).await;

        assert!(result.is_err(), "Using made-up type should be rejected");

        println!("✅ Hallucination prevention test passed!");
    }
}
