//! Verification and checkpointing (v0.180.0)
//!
//! Build + test verification and crash-safe checkpoint storage

use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, info};

pub mod checkpoint_store;
pub mod verifier;

// Re-export implementations
pub use checkpoint_store::FileCheckpointStore;
pub use verifier::{detect_verify_commands, resolve_verify_commands, BuildVerifier};

// Re-export shared types and traits from forge-agents
pub use agents::{Checkpoint, Task, TaskStatus, VerifyReport};
pub use agents::{CheckpointStore, Orchestrator, Verifier};

// Legacy verification loop (kept for compatibility)
/// Verify loop: run tests and build before reporting done
pub struct VerifyLoop {
    project_path: PathBuf,
}

impl VerifyLoop {
    pub fn new(project_path: impl Into<PathBuf>) -> Self {
        Self {
            project_path: project_path.into(),
        }
    }

    /// Run cargo test
    pub async fn run_tests(&self) -> Result<bool> {
        info!("Running cargo test...");

        let output = Command::new("cargo")
            .arg("test")
            .current_dir(&self.project_path)
            .output()?;

        let success = output.status.success();

        if success {
            info!("Tests passed");
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!("Tests failed: {}", stderr);
        }

        Ok(success)
    }

    /// Run cargo build
    pub async fn run_build(&self) -> Result<bool> {
        info!("Running cargo build...");

        let output = Command::new("cargo")
            .arg("build")
            .current_dir(&self.project_path)
            .output()?;

        let success = output.status.success();

        if success {
            info!("Build succeeded");
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!("Build failed: {}", stderr);
        }

        Ok(success)
    }

    /// Run full verification (test + build)
    pub async fn verify(&self) -> Result<bool> {
        info!("Starting verification loop...");

        let tests_passed = self.run_tests().await?;
        let build_passed = self.run_build().await?;

        Ok(tests_passed && build_passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_creation() {
        let verify = VerifyLoop::new("/tmp/test");
        assert_eq!(verify.project_path, PathBuf::from("/tmp/test"));
    }
}
