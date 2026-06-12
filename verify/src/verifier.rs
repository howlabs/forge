//! Verifier implementation for v0.180.0
//!
//! Runs build + test verification in worktrees

use forge_agents::traits::Verifier;
use forge_agents::types::VerifyReport;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::Path;
use std::process::Command;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Build and test verifier
pub struct BuildVerifier {
    /// Timeout in seconds
    timeout_seconds: u64,
}

impl BuildVerifier {
    /// Create a new verifier
    pub fn new(timeout_seconds: u64) -> Self {
        Self { timeout_seconds }
    }

    /// Run cargo build
    fn run_build(&self, workdir: &Path) -> Result<String> {
        info!("Running cargo build in {}", workdir.display());

        let start = Instant::now();
        let output = Command::new("cargo")
            .args(["build", "--quiet"])
            .current_dir(workdir)
            .output()?;

        let duration = start.elapsed();

        if output.status.success() {
            debug!("Build successful ({}ms)", duration.as_millis());
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Build failed: {}", stderr);
            Err(anyhow::anyhow!("Build failed: {}", stderr))
        }
    }

    /// Run cargo test
    fn run_test(&self, workdir: &Path) -> Result<String> {
        info!("Running cargo test in {}", workdir.display());

        let start = Instant::now();
        let output = Command::new("cargo")
            .args(["test", "--quiet"])
            .current_dir(workdir)
            .output()?;

        let duration = start.elapsed();

        if output.status.success() {
            debug!("Tests passed ({}ms)", duration.as_millis());
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("Tests failed: {}", stderr);
            Err(anyhow::anyhow!("Tests failed: {}", stderr))
        }
    }
}

#[async_trait]
impl Verifier for BuildVerifier {
    async fn verify(&self, workdir: &Path) -> Result<VerifyReport> {
        info!("Verifying workdir: {}", workdir.display());

        let start = Instant::now();

        // Run build
        let build_result = self.run_build(workdir);
        let build_logs = match &build_result {
            Ok(logs) => logs.clone(),
            Err(e) => e.to_string(),
        };

        // Run tests only if build succeeded
        let test_result = if build_result.is_ok() {
            self.run_test(workdir)
        } else {
            Err(anyhow::anyhow!("Skipped due to build failure"))
        };

        let test_logs = match &test_result {
            Ok(logs) => logs.clone(),
            Err(e) => e.to_string(),
        };

        let duration = start.elapsed();
        let passed = build_result.is_ok() && test_result.is_ok();

        let logs = format!(
            "=== BUILD ==={}\n=== TESTS ==={}",
            build_logs, test_logs
        );

        Ok(VerifyReport {
            passed,
            logs,
            duration_ms: duration.as_millis() as u64,
        })
    }

    async fn quick_check(&self, workdir: &Path) -> Result<bool> {
        debug!("Quick check for {}", workdir.display());

        // Check for Cargo.toml
        if !workdir.join("Cargo.toml").exists() {
            return Ok(false);
        }

        // Check for src directory
        if !workdir.join("src").exists() {
            return Ok(false);
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verifier_creation() {
        let verifier = BuildVerifier::new(300);
        assert_eq!(verifier.timeout_seconds, 300);
    }

    #[tokio::test]
    async fn test_quick_check_missing_cargo_toml() {
        let verifier = BuildVerifier::new(300);
        let result = verifier.quick_check(Path::new("/tmp/nonexistent")).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }
}
