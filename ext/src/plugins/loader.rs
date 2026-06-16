//! Plugin loader - handles plugin lifecycle and loading

use super::types::*;
use anyhow::Result;
use std::path::Path;

/// Plugin loader responsible for initializing and managing plugin lifecycle
pub struct PluginLoader;

impl PluginLoader {
    /// Load a plugin from its install path
    pub fn load_plugin(manifest: &PluginManifest, install_path: &Path) -> Result<LoadedPlugin> {
        let mut provided_tools = Vec::new();
        let mut provided_prompts = Vec::new();
        let mut provided_resources = Vec::new();

        for tool in &manifest.provides_tools {
            provided_tools.push(tool.clone());
        }
        for prompt in &manifest.provides_prompts {
            provided_prompts.push(prompt.clone());
        }
        for resource in &manifest.provides_resources {
            provided_resources.push(resource.clone());
        }

        Ok(LoadedPlugin {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            install_path: install_path.to_path_buf(),
            provided_tools,
            provided_prompts,
            provided_resources,
            hooks: manifest.hooks.clone(),
        })
    }

    /// Validate plugin dependencies against installed plugins
    pub fn validate_dependencies(
        manifest: &PluginManifest,
        installed: &[&PluginManifest],
    ) -> Result<Vec<String>> {
        let mut missing = Vec::new();

        for dep in &manifest.dependencies {
            if dep.optional {
                continue;
            }
            let found = installed.iter().any(|m| {
                m.name == dep.name && Self::version_satisfies(&m.version, &dep.version_req)
            });
            if !found {
                missing.push(format!("{} {}", dep.name, dep.version_req));
            }
        }

        Ok(missing)
    }

    fn version_satisfies(version: &PluginVersion, requirement: &str) -> bool {
        if let Some(req_str) = requirement.strip_prefix(">=") {
            if let Ok(req_version) = PluginVersion::parse(req_str) {
                return *version >= req_version;
            }
        } else if let Ok(req_version) = PluginVersion::parse(requirement) {
            return *version == req_version;
        }
        true
    }

    /// Load all plugins from a directory
    pub fn load_from_dir(dir: &Path) -> Result<Vec<LoadedPlugin>> {
        let mut plugins = Vec::new();
        if !dir.exists() {
            return Ok(plugins);
        }
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                let manifest_path = path.join("forge-plugin.toml");
                if manifest_path.exists() {
                    let content = std::fs::read_to_string(&manifest_path)?;
                    match PluginManifest::parse(&content) {
                        Ok(manifest) => {
                            let loaded = Self::load_plugin(&manifest, &path)?;
                            plugins.push(loaded);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to parse plugin manifest {:?}: {}",
                                manifest_path,
                                e
                            );
                        }
                    }
                }
            }
        }
        Ok(plugins)
    }

    /// Validate and load a plugin with full checks
    pub fn validate_and_load(
        manifest: &PluginManifest,
        install_path: &Path,
        installed: &[&PluginManifest],
    ) -> Result<Result<LoadedPlugin, Vec<String>>> {
        let missing = Self::validate_dependencies(manifest, installed)?;
        if !missing.is_empty() {
            return Ok(Err(missing));
        }
        let loaded = Self::load_plugin(manifest, install_path)?;
        Ok(Ok(loaded))
    }
}

/// A loaded plugin with resolved capabilities
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub name: String,
    pub version: PluginVersion,
    pub install_path: std::path::PathBuf,
    pub provided_tools: Vec<String>,
    pub provided_prompts: Vec<String>,
    pub provided_resources: Vec<String>,
    pub hooks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest() -> PluginManifest {
        PluginManifest {
            name: "test-plugin".into(),
            version: PluginVersion::new(1, 0, 0),
            description: "Test".into(),
            keywords: vec![],
            dependencies: vec![PluginDependency {
                name: "core".into(),
                version_req: ">=0.100.0".into(),
                optional: false,
            }],
            provides_tools: vec!["tool_a".into(), "tool_b".into()],
            provides_prompts: vec!["prompt_a".into()],
            provides_resources: vec!["res_a".into()],
            hooks: vec!["PreEdit".into()],
            config_schema: None,
            metadata: PluginMetadata {
                author: "Test".into(),
                homepage: None,
                repository: None,
                license: None,
                installed_at: "".into(),
                updated_at: None,
            },
            entry_point: None,
        }
    }

    #[test]
    fn test_load_plugin() {
        let manifest = test_manifest();
        let loaded = PluginLoader::load_plugin(&manifest, Path::new("/tmp/test-plugin")).unwrap();
        assert_eq!(loaded.name, "test-plugin");
        assert_eq!(loaded.provided_tools.len(), 2);
        assert_eq!(loaded.provided_prompts.len(), 1);
        assert_eq!(loaded.provided_resources.len(), 1);
        assert_eq!(loaded.hooks, vec!["PreEdit"]);
    }

    #[test]
    fn test_validate_dependencies_satisfied() {
        let manifest = test_manifest();
        let core_manifest = PluginManifest {
            name: "core".into(),
            version: PluginVersion::new(0, 100, 0),
            description: "Core".into(),
            keywords: vec![],
            dependencies: vec![],
            provides_tools: vec![],
            provides_prompts: vec![],
            provides_resources: vec![],
            hooks: vec![],
            config_schema: None,
            metadata: PluginMetadata {
                author: "Forge".into(),
                homepage: None,
                repository: None,
                license: None,
                installed_at: "".into(),
                updated_at: None,
            },
            entry_point: None,
        };
        let missing = PluginLoader::validate_dependencies(&manifest, &[&core_manifest]).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn test_validate_dependencies_missing() {
        let manifest = test_manifest();
        let missing = PluginLoader::validate_dependencies(&manifest, &[]).unwrap();
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("core"));
    }

    #[test]
    fn test_validate_optional_dependency() {
        let mut manifest = test_manifest();
        manifest.dependencies[0].optional = true;
        let missing = PluginLoader::validate_dependencies(&manifest, &[]).unwrap();
        assert!(missing.is_empty());
    }

    #[test]
    fn test_version_satisfies_exact() {
        assert!(PluginLoader::version_satisfies(
            &PluginVersion::new(1, 0, 0),
            "1.0.0"
        ));
        assert!(!PluginLoader::version_satisfies(
            &PluginVersion::new(1, 0, 1),
            "1.0.0"
        ));
    }

    #[test]
    fn test_version_satisfies_gte() {
        assert!(PluginLoader::version_satisfies(
            &PluginVersion::new(1, 1, 0),
            ">=1.0.0"
        ));
        assert!(PluginLoader::version_satisfies(
            &PluginVersion::new(1, 0, 0),
            ">=1.0.0"
        ));
        assert!(!PluginLoader::version_satisfies(
            &PluginVersion::new(0, 9, 0),
            ">=1.0.0"
        ));
    }

    #[test]
    fn test_load_from_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let plugin_dir = temp_dir.path().join("my-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();

        let manifest_content = r#"
name = "my-plugin"
version = "1.0.0"
description = "Test plugin"

[metadata]
author = "Test"
installed_at = "2024-01-01T00:00:00Z"
"#;
        std::fs::write(plugin_dir.join("forge-plugin.toml"), manifest_content).unwrap();

        let plugins = PluginLoader::load_from_dir(temp_dir.path()).unwrap();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "my-plugin");
    }

    #[test]
    fn test_load_from_dir_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let plugins = PluginLoader::load_from_dir(temp_dir.path()).unwrap();
        assert_eq!(plugins.len(), 0);
    }

    #[test]
    fn test_validate_and_load_ok() {
        let mut manifest = test_manifest();
        manifest.dependencies = vec![];
        let result =
            PluginLoader::validate_and_load(&manifest, Path::new("/tmp/test"), &[]).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_and_load_missing_deps() {
        let manifest = test_manifest();
        let result =
            PluginLoader::validate_and_load(&manifest, Path::new("/tmp/test"), &[]).unwrap();
        assert!(result.is_err());
        assert!(result.unwrap_err()[0].contains("core"));
    }
}
