//! Verifier implementation for v0.180.0
//!
//! Runs build + test verification in worktrees

use agents::traits::Verifier;
use agents::types::VerifyReport;
use anyhow::Result;
use async_trait::async_trait;
use forge_core::{Verifier as CoreVerifier, VerifyReport as CoreVerifyReport};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;
use std::time::Instant;
use tracing::{debug, info};

#[derive(Deserialize)]
struct ForgeToml {
    verify: Option<VerifyToml>,
}

#[derive(Deserialize)]
struct VerifyToml {
    commands: Option<Vec<String>>,
    #[serde(default = "default_true")]
    auto_detect: bool,
}

fn default_true() -> bool {
    true
}

/// Build and test verifier
pub struct BuildVerifier;

impl Default for BuildVerifier {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildVerifier {
    /// Create a new verifier
    pub fn new() -> Self {
        Self
    }
}

fn resolve_verify_commands(workdir: &Path) -> Option<Vec<String>> {
    let mut auto_detect = true;
    if let Ok(content) = std::fs::read_to_string(workdir.join("forge.toml")) {
        if let Ok(toml) = toml::from_str::<ForgeToml>(&content) {
            if let Some(verify) = toml.verify {
                if let Some(cmds) = verify.commands {
                    if !cmds.is_empty() {
                        return Some(cmds);
                    }
                }
                auto_detect = verify.auto_detect;
            }
        }
    }

    if auto_detect {
        detect_verify_commands(workdir)
    } else {
        None
    }
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

#[async_trait]
impl Verifier for BuildVerifier {
    async fn verify(&self, workdir: &Path) -> Result<VerifyReport> {
        info!("Verifying workdir: {}", workdir.display());

        let start = Instant::now();
        let commands = resolve_verify_commands(workdir);
        let mut logs = String::new();

        let passed = if let Some(cmds) = commands {
            let mut all_passed = true;
            for command in cmds {
                logs.push_str(&format!("$ {}\n", command));
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(&command)
                    .current_dir(workdir)
                    .output();

                match output {
                    Ok(out) => {
                        logs.push_str(&String::from_utf8_lossy(&out.stdout));
                        logs.push_str(&String::from_utf8_lossy(&out.stderr));
                        if !out.status.success() {
                            all_passed = false;
                            break;
                        }
                    }
                    Err(e) => {
                        logs.push_str(&format!("Failed to execute command: {}\n", e));
                        all_passed = false;
                        break;
                    }
                }
            }
            all_passed
        } else {
            // Default fallback if no commands configured or detected: run cargo build and cargo test
            logs.push_str("$ cargo build --quiet\n");
            let build_output = Command::new("cargo")
                .args(["build", "--quiet"])
                .current_dir(workdir)
                .output();

            let build_passed = match build_output {
                Ok(out) => {
                    logs.push_str(&String::from_utf8_lossy(&out.stdout));
                    logs.push_str(&String::from_utf8_lossy(&out.stderr));
                    out.status.success()
                }
                Err(e) => {
                    logs.push_str(&format!("Failed to execute cargo build: {}\n", e));
                    false
                }
            };

            if build_passed {
                logs.push_str("$ cargo test --quiet\n");
                let test_output = Command::new("cargo")
                    .args(["test", "--quiet"])
                    .current_dir(workdir)
                    .output();

                match test_output {
                    Ok(out) => {
                        logs.push_str(&String::from_utf8_lossy(&out.stdout));
                        logs.push_str(&String::from_utf8_lossy(&out.stderr));
                        out.status.success()
                    }
                    Err(e) => {
                        logs.push_str(&format!("Failed to execute cargo test: {}\n", e));
                        false
                    }
                }
            } else {
                false
            }
        };

        let duration = start.elapsed();

        Ok(VerifyReport {
            passed,
            logs,
            duration_ms: duration.as_millis() as u64,
        })
    }

    async fn quick_check(&self, workdir: &Path) -> Result<bool> {
        debug!("Quick check for {}", workdir.display());

        // Check forge.toml config
        if let Ok(content) = std::fs::read_to_string(workdir.join("forge.toml")) {
            if let Ok(toml) = toml::from_str::<ForgeToml>(&content) {
                if let Some(verify) = toml.verify {
                    if let Some(cmds) = &verify.commands {
                        if !cmds.is_empty() {
                            return Ok(true);
                        }
                    }
                }
            }
        }

        // Auto-detect check
        if workdir.join("Cargo.toml").exists()
            || workdir.join("package.json").exists()
            || workdir.join("pyproject.toml").exists()
            || workdir.join("pytest.ini").exists()
            || workdir.join("go.mod").exists()
            || workdir.join("pom.xml").exists()
            || workdir.join("build.gradle").exists()
            || workdir.join("build.gradle.kts").exists()
            || workdir.join("Makefile").exists()
            || workdir.join("makefile").exists()
        {
            return Ok(true);
        }

        Ok(false)
    }
}

#[async_trait]
impl CoreVerifier for BuildVerifier {
    async fn verify(&self, workdir: &Path) -> Result<CoreVerifyReport> {
        let report = <Self as Verifier>::verify(self, workdir).await?;
        Ok(CoreVerifyReport {
            passed: report.passed,
            logs: report.logs,
            duration_ms: report.duration_ms,
        })
    }

    async fn quick_check(&self, workdir: &Path) -> Result<bool> {
        <Self as Verifier>::quick_check(self, workdir).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_creation() {
        let _verifier = BuildVerifier::new();
    }

    #[tokio::test]
    async fn test_quick_check_missing_cargo_toml() {
        let verifier = BuildVerifier::new();
        let result = agents::Verifier::quick_check(&verifier, Path::new("/tmp/nonexistent")).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[tokio::test]
    async fn test_auto_detect_yarn() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join("yarn.lock"), "").unwrap();
        let verifier = BuildVerifier::new();
        let result = agents::Verifier::quick_check(&verifier, dir.path()).await;
        assert!(result.unwrap());
        let cmds = resolve_verify_commands(dir.path()).unwrap();
        assert_eq!(cmds, vec!["yarn test", "yarn build"]);
    }

    #[tokio::test]
    async fn test_config_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("forge.toml"), r#"
[verify]
commands = ["echo hello", "echo world"]
"#).unwrap();
        let verifier = BuildVerifier::new();
        let result = agents::Verifier::quick_check(&verifier, dir.path()).await;
        assert!(result.unwrap());
        let cmds = resolve_verify_commands(dir.path()).unwrap();
        assert_eq!(cmds, vec!["echo hello", "echo world"]);
    }
}
