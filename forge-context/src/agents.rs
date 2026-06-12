//! AGENTS.md layered discovery (pain #10)
//!
//! Discovers AGENTS.md files from current directory up to root.
//! Nearest file wins (layered discovery). Treats AGENTS.md as optional/advisory.

use anyhow::Result;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Parsed AGENTS.md file
#[derive(Debug, Clone)]
pub struct AgentsFile {
    pub content: String,
    pub scope: String,
    pub path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct AgentsFrontmatter {
    scope: Option<String>,
}

impl AgentsFile {
    /// Parse AGENTS.md content with frontmatter
    pub fn parse(file_path: impl Into<PathBuf>, content: &str) -> Result<Self> {
        let file_path = file_path.into();

        // Split frontmatter and content
        let parts: Vec<&str> = content.splitn(3, "---").collect();

        let (scope, content) = if parts.len() >= 3 {
            let frontmatter: AgentsFrontmatter = serde_yaml::from_str(parts[1])?;
            (frontmatter.scope.unwrap_or("global".to_string()), parts[2].to_string())
        } else {
            ("global".to_string(), content.to_string())
        };

        Ok(Self {
            content,
            scope,
            path: file_path,
        })
    }
}

/// Layered AGENTS.md discovery (nearest file wins)
pub struct AgentsDiscovery {
    root_path: PathBuf,
}

impl AgentsDiscovery {
    /// Create new AGENTS.md discovery
    pub fn new(root_path: PathBuf) -> Self {
        Self { root_path }
    }

    /// Discover AGENTS.md files from current dir up to root
    /// Returns files ordered from nearest to root
    pub fn discover_from(&self, start_dir: &Path) -> Result<Vec<AgentsFile>> {
        let mut files = Vec::new();
        let mut current = Some(start_dir.to_path_buf());

        while let Some(dir) = current {
            let agents_path = dir.join("AGENTS.md");

            if agents_path.exists() {
                let content = std::fs::read_to_string(&agents_path)?;
                let mut file = AgentsFile::parse(&agents_path, &content)?;
                file.path = agents_path;
                files.push(file);
            }

            // Move up to parent
            current = dir.parent().map(|p| p.to_path_buf());

            // Stop if we've reached above root_path
            if let Some(ref dir) = current {
                // If we've gone above the root_path, stop
                if dir.starts_with(&self.root_path) {
                    continue;
                } else {
                    break;
                }
            }
        }

        Ok(files)
    }

    /// Get nearest AGENTS.md
    pub fn get_nearest(&self, start_dir: &Path) -> Result<Option<AgentsFile>> {
        let files = self.discover_from(start_dir)?;
        Ok(files.first().cloned())
    }

    /// Check if AGENTS.md exists at any level
    pub fn has_agents_file(&self, start_dir: &Path) -> bool {
        self.get_nearest(start_dir).ok().flatten().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agents_file_parsing() {
        let content = r#"---
scope: local
---

This is local AGENTS.md"#;
        let file = AgentsFile::parse(PathBuf::from("test.md"), content).unwrap();
        assert_eq!(file.scope, "local");
        assert!(file.content.contains("local AGENTS.md"));
    }

    #[test]
    fn test_agents_file_no_frontmatter() {
        let content = "Plain AGENTS.md without frontmatter";
        let file = AgentsFile::parse(PathBuf::from("test.md"), content).unwrap();
        assert_eq!(file.scope, "global");
        assert_eq!(file.content, content);
    }

    #[test]
    fn test_agents_file_empty_frontmatter() {
        let content = r#"---
---

Empty frontmatter"#;
        let file = AgentsFile::parse(PathBuf::from("test.md"), content).unwrap();
        assert_eq!(file.scope, "global");
    }

    #[test]
    fn test_layered_discovery() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let subdir = root.join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        // Root AGENTS.md
        std::fs::write(
            root.join("AGENTS.md"),
            r#"---
scope: global
---

Global agents"#,
        ).unwrap();

        // Subdir AGENTS.md
        std::fs::write(
            subdir.join("AGENTS.md"),
            r#"---
scope: local
---

Local agents"#,
        ).unwrap();

        let discovery = AgentsDiscovery::new(root.to_path_buf());
        let result = discovery.discover_from(&subdir).unwrap();

        // Should find local AGENTS.md first, then global
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].scope, "local"); // Nearest wins
        assert_eq!(result[1].scope, "global");
    }

    #[test]
    fn test_get_nearest() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();
        let subdir = root.join("nested");
        std::fs::create_dir_all(&subdir).unwrap();

        // Only root AGENTS.md
        std::fs::write(
            root.join("AGENTS.md"),
            "Root agents",
        ).unwrap();

        let discovery = AgentsDiscovery::new(root.to_path_buf());
        let result = discovery.get_nearest(&subdir).unwrap();

        assert!(result.is_some());
        assert_eq!(result.unwrap().scope, "global");
    }

    #[test]
    fn test_has_agents_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let discovery = AgentsDiscovery::new(root.to_path_buf());

        // No AGENTS.md
        assert!(!discovery.has_agents_file(root));

        // Add AGENTS.md
        std::fs::write(
            root.join("AGENTS.md"),
            "Test agents",
        ).unwrap();

        assert!(discovery.has_agents_file(root));
    }

    #[test]
    fn test_optional_agents_md() {
        // Should not fail when AGENTS.md is missing
        let temp_dir = tempfile::tempdir().unwrap();
        let root = temp_dir.path();

        let discovery = AgentsDiscovery::new(root.to_path_buf());
        let result = discovery.discover_from(root);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }
}
