//! Headless execution mode for CI/CD
//!
//! Provides non-interactive execution with machine-readable output.

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Configuration for headless exec mode
#[derive(Debug, Clone)]
pub struct ExecConfig {
    pub task: String,
    pub verify: bool,
    pub output_format: String, // "json" | "text"
    pub trace: bool,
}

/// Result of exec run (machine-readable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResult {
    pub success: bool,
    pub task_id: String,
    pub duration_ms: u64,
    pub steps_completed: usize,
    pub verify_passed: bool,
    pub error_message: Option<String>,
}

impl ExecResult {
    /// Get exit code for CI/CD
    pub fn exit_code(&self) -> i32 {
        if self.success && self.verify_passed {
            0
        } else {
            1
        }
    }

    /// Format result as JSON
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Format result as text
    pub fn to_text(&self) -> String {
        format!(
            "Task completed: {}\nSuccess: {}\nVerify passed: {}\nDuration: {}ms\nSteps completed: {}",
            self.task_id, self.success, self.verify_passed, self.duration_ms, self.steps_completed
        )
    }
}

/// Run Forge in headless exec mode
pub async fn run_exec(config: ExecConfig) -> anyhow::Result<ExecResult> {
    let start = Instant::now();

    // For v0.190.0, simulate execution
    // Real implementation would use Orchestrator and Verifier
    let task_id = uuid::Uuid::new_v4().to_string();

    // Simulate work
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let duration = start.elapsed().as_millis() as u64;

    Ok(ExecResult {
        success: true,
        task_id,
        duration_ms: duration,
        steps_completed: 1,
        verify_passed: config.verify, // Mock: always pass if verify enabled
        error_message: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_config() {
        let config = ExecConfig {
            task: "Fix the bug".to_string(),
            verify: true,
            output_format: "json".to_string(),
            trace: false,
        };
        assert_eq!(config.task, "Fix the bug");
        assert!(config.verify);
    }

    #[test]
    fn test_exec_result_serialization() {
        let result = ExecResult {
            success: true,
            task_id: "test-123".to_string(),
            duration_ms: 1000,
            steps_completed: 5,
            verify_passed: true,
            error_message: None,
        };
        let json = result.to_json().unwrap();
        println!("JSON: {}", json);
        assert!(json.contains("success"));
        assert!(json.contains("test-123"));
    }

    #[test]
    fn test_exec_result_exit_code() {
        let result = ExecResult {
            success: true,
            task_id: "test".to_string(),
            duration_ms: 1000,
            steps_completed: 1,
            verify_passed: true,
            error_message: None,
        };
        assert_eq!(result.exit_code(), 0);

        let result_fail = ExecResult {
            success: false,
            task_id: "test".to_string(),
            duration_ms: 1000,
            steps_completed: 1,
            verify_passed: false,
            error_message: Some("Error".to_string()),
        };
        assert_eq!(result_fail.exit_code(), 1);
    }

    #[test]
    fn test_exec_result_text_format() {
        let result = ExecResult {
            success: true,
            task_id: "test-abc".to_string(),
            duration_ms: 500,
            steps_completed: 3,
            verify_passed: true,
            error_message: None,
        };
        let text = result.to_text();
        assert!(text.contains("test-abc"));
        assert!(text.contains("Success: true"));
        assert!(text.contains("500ms"));
    }

    #[tokio::test]
    async fn test_run_exec() {
        let config = ExecConfig {
            task: "Test task".to_string(),
            verify: true,
            output_format: "json".to_string(),
            trace: false,
        };

        let result = run_exec(config).await.unwrap();
        assert!(result.success);
        assert_eq!(result.steps_completed, 1);
        assert!(result.duration_ms > 0);
    }
}
