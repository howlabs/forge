//! Plugin types and manifest definitions

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Semantic version for plugins
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl<'de> Deserialize<'de> for PluginVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        PluginVersion::parse(&s).map_err(serde::de::Error::custom)
    }
}

impl Serialize for PluginVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl PluginVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("Invalid version format: {}", s));
        }
        Ok(Self {
            major: parts[0].parse().map_err(|e| format!("Invalid major: {}", e))?,
            minor: parts[1].parse().map_err(|e| format!("Invalid minor: {}", e))?,
            patch: parts[2].parse().map_err(|e| format!("Invalid patch: {}", e))?,
        })
    }
}

impl std::fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Plugin installation status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginStatus {
    Installed,
    Disabled,
    Failed(String),
    Updating,
}

/// Plugin metadata (who, when, what)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub author: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub installed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Plugin dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginDependency {
    pub name: String,
    pub version_req: String,
    #[serde(default)]
    pub optional: bool,
}

/// Plugin manifest (forge-plugin.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: PluginVersion,
    pub description: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<PluginDependency>,
    #[serde(default)]
    pub provides_tools: Vec<String>,
    #[serde(default)]
    pub provides_prompts: Vec<String>,
    #[serde(default)]
    pub provides_resources: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub config_schema: Option<serde_json::Value>,
    pub metadata: PluginMetadata,
    #[serde(default)]
    pub entry_point: Option<String>,
}

impl PluginManifest {
    pub fn parse(content: &str) -> Result<Self, String> {
        toml::from_str(content).map_err(|e| format!("Failed to parse plugin manifest: {}", e))
    }

    pub fn serialize(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize plugin manifest: {}", e))
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.name.is_empty() {
            errors.push("Plugin name cannot be empty".into());
        }
        if self.description.is_empty() {
            errors.push("Plugin description cannot be empty".into());
        }
        if self.metadata.author.is_empty() {
            errors.push("Plugin author cannot be empty".into());
        }
        errors
    }
}

/// Installed plugin info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub status: PluginStatus,
    pub install_path: PathBuf,
    pub config: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_version_parse() {
        let v = PluginVersion::parse("1.2.3").unwrap();
        assert_eq!(v, PluginVersion::new(1, 2, 3));
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_plugin_version_invalid() {
        assert!(PluginVersion::parse("1.2").is_err());
        assert!(PluginVersion::parse("a.b.c").is_err());
    }

    #[test]
    fn test_plugin_version_ordering() {
        let v1 = PluginVersion::new(1, 0, 0);
        let v2 = PluginVersion::new(1, 1, 0);
        let v3 = PluginVersion::new(2, 0, 0);
        assert!(v1 < v2);
        assert!(v2 < v3);
    }

    #[test]
    fn test_plugin_manifest_parse() {
        let toml_content = r#"
name = "test-plugin"
version = "1.0.0"
description = "A test plugin"
keywords = ["test", "example"]
provides_tools = ["test_tool"]
provides_prompts = ["test_prompt"]

[metadata]
author = "Test Author"
homepage = "https://example.com"
license = "MIT"
installed_at = "2024-01-01T00:00:00Z"
"#;
        let manifest = PluginManifest::parse(toml_content).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, PluginVersion::new(1, 0, 0));
        assert_eq!(manifest.provides_tools, vec!["test_tool"]);
        assert_eq!(manifest.metadata.author, "Test Author");
    }

    #[test]
    fn test_plugin_manifest_validate() {
        let manifest = PluginManifest {
            name: "".into(),
            version: PluginVersion::new(0, 1, 0),
            description: "".into(),
            keywords: vec![],
            dependencies: vec![],
            provides_tools: vec![],
            provides_prompts: vec![],
            provides_resources: vec![],
            hooks: vec![],
            config_schema: None,
            metadata: PluginMetadata {
                author: "".into(),
                homepage: None,
                repository: None,
                license: None,
                installed_at: "".into(),
                updated_at: None,
            },
            entry_point: None,
        };
        let errors = manifest.validate();
        assert_eq!(errors.len(), 3);
    }

    #[test]
    fn test_plugin_manifest_roundtrip() {
        let manifest = PluginManifest {
            name: "my-plugin".into(),
            version: PluginVersion::new(0, 1, 0),
            description: "My plugin".into(),
            keywords: vec!["plugin".into()],
            dependencies: vec![PluginDependency {
                name: "core".into(),
                version_req: ">=0.100.0".into(),
                optional: false,
            }],
            provides_tools: vec!["my_tool".into()],
            provides_prompts: vec![],
            provides_resources: vec![],
            hooks: vec!["PreEdit".into()],
            config_schema: None,
            metadata: PluginMetadata {
                author: "Author".into(),
                homepage: None,
                repository: None,
                license: Some("MIT".into()),
                installed_at: "2024-01-01T00:00:00Z".into(),
                updated_at: None,
            },
            entry_point: None,
        };
        let serialized = manifest.serialize().unwrap();
        let parsed = PluginManifest::parse(&serialized).unwrap();
        assert_eq!(parsed.name, "my-plugin");
        assert_eq!(parsed.dependencies.len(), 1);
    }
}
