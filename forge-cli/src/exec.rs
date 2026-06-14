//! Headless execution mode for CI/CD
//!
//! Provides non-interactive execution with machine-readable output.

use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Configuration for headless exec mode
#[derive(Debug, Clone)]
#[allow(dead_code)]
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
    use context::ContextEngine;
    use forge_core::EventLoop;
    use provider::anthropic::AnthropicProvider;
    use sandbox::Sandbox;
    use verify::BuildVerifier;

    let start = Instant::now();
    let task_id = uuid::Uuid::new_v4().to_string();

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
    let provider = AnthropicProvider::new(&api_key, "claude-opus-4-5")?;

    let workdir = std::env::current_dir()?;
    let context = ContextEngine::new(&workdir)?;
    let sandbox = Sandbox::new(&workdir, "on")?;

    let mut event_loop = EventLoop::new(provider, context, sandbox, config.task.clone());

    let (steps, verify_passed) = if config.verify {
        let verifier = BuildVerifier::new();
        match event_loop.run_with_verify(&verifier, &workdir, 5).await {
            Ok(steps) => (steps, true),
            Err(e) => {
                return Ok(ExecResult {
                    success: false,
                    task_id,
                    duration_ms: start.elapsed().as_millis() as u64,
                    steps_completed: 0,
                    verify_passed: false,
                    error_message: Some(e.to_string()),
                });
            }
        }
    } else {
        let steps = event_loop.run().await?;
        (steps, false)
    };

    let duration = start.elapsed().as_millis() as u64;

    Ok(ExecResult {
        success: true,
        task_id,
        duration_ms: duration,
        steps_completed: steps,
        verify_passed,
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
        std::env::remove_var("ANTHROPIC_API_KEY");
        let config = ExecConfig {
            task: "Test task".to_string(),
            verify: true,
            output_format: "json".to_string(),
            trace: false,
        };

        let result = run_exec(config).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("ANTHROPIC_API_KEY not set"));
    }
}
