//! Headless execution mode for CI/CD
//!
//! Provides non-interactive execution with machine-readable output.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use verify::BuildVerifier;

/// Configuration for headless exec mode
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ExecConfig {
    pub task: String,
    pub project_path: PathBuf,
    pub config_path: PathBuf,
    pub api_key: String,
    pub provider: String,
    pub model: String,
    pub verify: bool,
    pub output_format: String, // "json" | "text"
    pub trace: bool,
}

/// Optional forge.toml shape consumed by exec mode.
#[derive(Debug, Clone, Default, Deserialize)]
struct ForgeToml {
    verify: Option<VerifyToml>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct VerifyToml {
    /// Explicit verify commands. When present, these override auto-detection.
    commands: Option<Vec<String>>,
    /// Enable language/project auto-detection when commands are not provided.
    #[serde(default = "default_true")]
    auto_detect: bool,
    /// Maximum verification repair attempts for the agent loop.
    max_retries: Option<usize>,
}

fn default_true() -> bool {
    true
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

struct CommandVerifier {
    commands: Vec<String>,
}

impl CommandVerifier {
    fn new(commands: Vec<String>) -> Self {
        Self { commands }
    }
}

#[async_trait::async_trait]
impl forge_core::Verifier for CommandVerifier {
    async fn verify(&self, workdir: &Path) -> Result<forge_core::VerifyReport> {
        let start = Instant::now();
        let mut logs = String::new();
        for command in &self.commands {
            logs.push_str(&format!("$ {command}\n"));
            let output = Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(workdir)
                .output()
                .with_context(|| format!("failed to run verify command `{command}`"))?;
            logs.push_str(&String::from_utf8_lossy(&output.stdout));
            logs.push_str(&String::from_utf8_lossy(&output.stderr));
            if !output.status.success() {
                return Ok(forge_core::VerifyReport {
                    passed: false,
                    logs,
                    duration_ms: start.elapsed().as_millis() as u64,
                });
            }
        }

        Ok(forge_core::VerifyReport {
            passed: true,
            logs,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    async fn quick_check(&self, _workdir: &Path) -> Result<bool> {
        Ok(!self.commands.is_empty())
    }
}

/// Run Forge in headless exec mode
pub async fn run_exec(config: ExecConfig) -> anyhow::Result<ExecResult> {
    use context::ContextEngine;
    use forge_core::EventLoop;
    use sandbox::Sandbox;

    let start = Instant::now();
    let task_id = uuid::Uuid::new_v4().to_string();
    let workdir = config.project_path.canonicalize().with_context(|| {
        format!(
            "failed to canonicalize project path {}",
            config.project_path.display()
        )
    })?;

    let provider = crate::create_provider_instance(&config.provider, &config.model, &config.api_key)?;
    let forge_toml = load_forge_toml(&config.config_path)?;
    let verify_commands = resolve_verify_commands(&workdir, forge_toml.as_ref());
    let max_retries = forge_toml
        .as_ref()
        .and_then(|toml| toml.verify.as_ref())
        .and_then(|verify| verify.max_retries)
        .unwrap_or(5);

    let context = ContextEngine::new(&workdir)?;
    let sandbox = Sandbox::new(&workdir, "off")?;
    let mut event_loop = EventLoop::new(provider, context, sandbox, config.task.clone());

    let (steps, verify_passed) = if config.verify {
        if let Some(commands) = verify_commands {
            let verifier = CommandVerifier::new(commands);
            match event_loop
                .run_with_verify(&verifier, &workdir, max_retries)
                .await
            {
                Ok(steps) => (steps, true),
                Err(e) => return failed_result(task_id, start, e),
            }
        } else {
            let verifier = BuildVerifier::new();
            match event_loop
                .run_with_verify(&verifier, &workdir, max_retries)
                .await
            {
                Ok(steps) => (steps, true),
                Err(e) => return failed_result(task_id, start, e),
            }
        }
    } else {
        let steps = event_loop.run().await?;
        (steps, false)
    };

    Ok(ExecResult {
        success: true,
        task_id,
        duration_ms: start.elapsed().as_millis() as u64,
        steps_completed: steps,
        verify_passed,
        error_message: None,
    })
}

fn failed_result(
    task_id: String,
    start: Instant,
    error: anyhow::Error,
) -> anyhow::Result<ExecResult> {
    Ok(ExecResult {
        success: false,
        task_id,
        duration_ms: start.elapsed().as_millis() as u64,
        steps_completed: 0,
        verify_passed: false,
        error_message: Some(error.to_string()),
    })
}



fn load_forge_toml(path: &Path) -> Result<Option<ForgeToml>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config {}", path.display()))?;
    let parsed = toml::from_str(&content)
        .with_context(|| format!("failed to parse config {}", path.display()))?;
    Ok(Some(parsed))
}

fn resolve_verify_commands(workdir: &Path, config: Option<&ForgeToml>) -> Option<Vec<String>> {
    if let Some(verify) = config.and_then(|toml| toml.verify.as_ref()) {
        if let Some(commands) = &verify.commands {
            if !commands.is_empty() {
                return Some(commands.clone());
            }
        }
        if !verify.auto_detect {
            return None;
        }
    }

    detect_verify_commands(workdir)
}

fn detect_verify_commands(workdir: &Path) -> Option<Vec<String>> {
    if workdir.join("Cargo.toml").exists() {
        return Some(vec![
            "cargo build --quiet".into(),
            "cargo test --quiet".into(),
        ]);
    }
    if workdir.join("package.json").exists() {
        let runner = if workdir.join("pnpm-lock.yaml").exists() {
            "pnpm"
        } else if workdir.join("yarn.lock").exists() {
            "yarn"
        } else {
            "npm"
        };
        return Some(match runner {
            "pnpm" => vec!["pnpm test".into(), "pnpm run build".into()],
            "yarn" => vec!["yarn test".into(), "yarn build".into()],
            _ => vec!["npm test".into(), "npm run build".into()],
        });
    }
    if workdir.join("pyproject.toml").exists() || workdir.join("pytest.ini").exists() {
        return Some(vec!["python -m pytest".into()]);
    }
    if workdir.join("go.mod").exists() {
        return Some(vec!["go test ./...".into()]);
    }
    if workdir.join("pom.xml").exists() {
        return Some(vec!["mvn test".into()]);
    }
    if workdir.join("build.gradle").exists() || workdir.join("build.gradle.kts").exists() {
        let runner = if workdir.join("gradlew").exists() {
            "./gradlew test"
        } else {
            "gradle test"
        };
        return Some(vec![runner.into()]);
    }
    if workdir.join("Makefile").exists() || workdir.join("makefile").exists() {
        return Some(vec!["make test".into()]);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_exec_config() {
        let config = ExecConfig {
            task: "Fix the bug".to_string(),
            project_path: PathBuf::from("."),
            config_path: PathBuf::from("forge.toml"),
            api_key: "test".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-opus-4-5".to_string(),
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

    #[test]
    fn detects_cargo_verifier() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname='x'").unwrap();
        assert_eq!(
            detect_verify_commands(dir.path()).unwrap(),
            vec!["cargo build --quiet", "cargo test --quiet"]
        );
    }

    #[test]
    fn config_commands_override_auto_detect() {
        let toml = ForgeToml {
            verify: Some(VerifyToml {
                commands: Some(vec!["just verify".into()]),
                auto_detect: true,
                max_retries: Some(2),
            }),
        };
        assert_eq!(
            resolve_verify_commands(Path::new("."), Some(&toml)).unwrap(),
            vec!["just verify"]
        );
    }
}
