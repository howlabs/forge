use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;
use tracing::debug;

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
        let full_path = self.project_path.join(path);
        debug!("Reading file: {}", full_path.display());
        std::fs::read_to_string(full_path)
            .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", path, e))
    }

    /// Write a file
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
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
            // Network-off mode: restrict network access
            Command::new("unshare")
                .arg("-n")
                .arg("-r")
                .arg("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&self.project_path)
                .output()?
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
            return Err(anyhow::anyhow!(
                "Old text not found in file: {}",
                path
            ));
        }

        std::fs::write(&full_path, new_content)
            .map_err(|e| anyhow::anyhow!("Failed to write edited file {}: {}", path, e))
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
}
