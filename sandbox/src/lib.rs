use anyhow::Result;
use std::path::Component;
use std::path::PathBuf;
use std::process::Command;
use tracing::{debug, warn};

/// Sandbox for safe code execution (v0.100.0: network-off)
pub struct Sandbox {
    project_path: PathBuf,
    network_mode: String,
}

impl Sandbox {
    pub fn new(project_path: impl Into<PathBuf>, network_mode: impl Into<String>) -> Result<Self> {
        let path = project_path.into();
        let mode = network_mode.into();
        debug!("Creating sandbox with network mode: {}", mode);
        Ok(Self {
            project_path: path,
            network_mode: mode,
        })
    }

    /// List files in the project
    pub async fn list_files(&self) -> Result<Vec<String>> {
        debug!("Listing files in: {}", self.project_path.display());

        let output = Command::new("find")
            .arg(&self.project_path)
            .arg("-type")
            .arg("f")
            .arg("-name")
            .arg("*.rs")
            .output()?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("find command failed"));
        }

        let files = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.to_string())
            .collect();

        Ok(files)
    }

    /// Read a file
    pub async fn read_file(&self, path: &str) -> Result<String> {
        self.validate_relative_path(path)?;
        let full_path = self.project_path.join(path);
        debug!("Reading file: {}", full_path.display());

        let canonical = full_path
            .canonicalize()
            .map_err(|_| anyhow::anyhow!("File not found: {}", path))?;
        let project_path = self.project_path.canonicalize()?;
        if !canonical.starts_with(&project_path) {
            return Err(anyhow::anyhow!("Path traversal not allowed: {}", path));
        }

        std::fs::read_to_string(canonical)
            .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", path, e))
    }

    /// Write a file
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        self.validate_relative_path(path)?;
        let full_path = self.project_path.join(path);
        debug!("Writing file: {}", full_path.display());

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&full_path, content)
            .map_err(|e| anyhow::anyhow!("Failed to write file {}: {}", path, e))
    }

    /// Run a command (network-off if configured)
    pub async fn run_command(&self, command: &str) -> Result<String> {
        debug!("Running command: {}", command);

        let output = if self.network_mode == "off" {
            let result = Command::new("unshare")
                .arg("--net")
                .arg("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.project_path)
                .output();

            match result {
                Ok(output) => output,
                Err(_) => {
                    warn!("unshare not available, running without network isolation");
                    Command::new("sh")
                        .arg("-c")
                        .arg(command)
                        .current_dir(&self.project_path)
                        .output()?
                }
            }
        } else {
            // Normal mode: allow network
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.project_path)
                .output()?
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!(
                "Command failed with status {}: {}",
                output.status,
                stderr
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    }

    /// Apply a diff edit to a file
    pub async fn diff_edit(&self, path: &str, old_text: &str, new_text: &str) -> Result<()> {
        let full_path = self.project_path.join(path);
        debug!("Applying diff edit to: {}", full_path.display());

        let content = std::fs::read_to_string(&full_path)?;
        let new_content = content.replacen(old_text, new_text, 1);

        if new_content == content {
            return Err(anyhow::anyhow!("Old text not found in file: {}", path));
        }

        std::fs::write(&full_path, new_content)
            .map_err(|e| anyhow::anyhow!("Failed to write edited file {}: {}", path, e))
    }

    fn validate_relative_path(&self, path: &str) -> Result<()> {
        let path = std::path::Path::new(path);
        if path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(anyhow::anyhow!(
                "Path traversal not allowed: {}",
                path.display()
            ));
        }
        Ok(())
    }

    /// Create a test sandbox
    #[cfg(test)]
    pub fn test() -> Self {
        Self {
            project_path: PathBuf::from("/tmp/forge-test"),
            network_mode: "off".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_creation() {
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();
        assert_eq!(sandbox.project_path, PathBuf::from("/tmp/test"));
        assert_eq!(sandbox.network_mode, "off");
    }

    #[test]
    fn test_sandbox_test() {
        let sandbox = Sandbox::test();
        assert_eq!(sandbox.project_path, PathBuf::from("/tmp/forge-test"));
    }

    #[tokio::test]
    async fn test_path_traversal_blocked() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();

        let result = sandbox.read_file("../etc/passwd").await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Path traversal not allowed"));
    }

    #[tokio::test]
    async fn test_path_within_sandbox_allowed() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("allowed.txt"), "ok").unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();

        let content = sandbox.read_file("allowed.txt").await.unwrap();

        assert_eq!(content, "ok");
    }
}
