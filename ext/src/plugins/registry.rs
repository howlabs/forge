//! Plugin registry for install/uninstall/list/search/update

use super::types::*;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Plugin registry managing installed plugins
pub struct PluginRegistry {
    plugins_dir: PathBuf,
    installed: HashMap<String, InstalledPlugin>,
}

impl PluginRegistry {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self {
            plugins_dir,
            installed: HashMap::new(),
        }
    }

    pub fn plugins_dir(&self) -> &Path {
        &self.plugins_dir
    }

    pub fn list(&self) -> Vec<&InstalledPlugin> {
        self.installed.values().collect()
    }

    pub fn get(&self, name: &str) -> Option<&InstalledPlugin> {
        self.installed.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut InstalledPlugin> {
        self.installed.get_mut(name)
    }

    pub fn install(&mut self, manifest: PluginManifest) -> Result<()> {
        let name = manifest.name.clone();
        let install_path = self.plugins_dir.join(&name);

        let plugin = InstalledPlugin {
            manifest,
            status: PluginStatus::Installed,
            install_path,
            config: serde_json::json!({}),
        };

        self.installed.insert(name, plugin);
        Ok(())
    }

    pub fn uninstall(&mut self, name: &str) -> Result<()> {
        self.installed
            .remove(name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;
        Ok(())
    }

    pub fn enable(&mut self, name: &str) -> Result<()> {
        let plugin = self
            .installed
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;
        plugin.status = PluginStatus::Installed;
        Ok(())
    }

    pub fn disable(&mut self, name: &str) -> Result<()> {
        let plugin = self
            .installed
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;
        plugin.status = PluginStatus::Disabled;
        Ok(())
    }

    pub fn search(&self, query: &str) -> Vec<&InstalledPlugin> {
        let query_lower = query.to_lowercase();
        self.installed
            .values()
            .filter(|p| {
                p.manifest.name.to_lowercase().contains(&query_lower)
                    || p.manifest.description.to_lowercase().contains(&query_lower)
                    || p.manifest
                        .keywords
                        .iter()
                        .any(|k| k.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    pub fn update_config(&mut self, name: &str, config: serde_json::Value) -> Result<()> {
        let plugin = self
            .installed
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;
        plugin.config = config;
        Ok(())
    }

    pub fn provides_tools(&self) -> Vec<String> {
        self.installed
            .values()
            .filter(|p| matches!(p.status, PluginStatus::Installed))
            .flat_map(|p| p.manifest.provides_tools.clone())
            .collect()
    }

    pub fn provides_prompts(&self) -> Vec<String> {
        self.installed
            .values()
            .filter(|p| matches!(p.status, PluginStatus::Installed))
            .flat_map(|p| p.manifest.provides_prompts.clone())
            .collect()
    }

    pub fn provides_resources(&self) -> Vec<String> {
        self.installed
            .values()
            .filter(|p| matches!(p.status, PluginStatus::Installed))
            .flat_map(|p| p.manifest.provides_resources.clone())
            .collect()
    }

    pub fn count(&self) -> usize {
        self.installed.len()
    }

    pub fn is_installed(&self, name: &str) -> bool {
        self.installed.contains_key(name)
    }

    pub fn load_from_dir(&mut self) -> Result<()> {
        if !self.plugins_dir.exists() {
            return Ok(());
        }

        for entry in std::fs::read_dir(&self.plugins_dir)? {
            let entry = entry?;
            let path = entry.path();
            let manifest_path = path.join("forge-plugin.toml");
            if manifest_path.exists() {
                let content = std::fs::read_to_string(&manifest_path)?;
                match PluginManifest::parse(&content) {
                    Ok(manifest) => {
                        let name = manifest.name.clone();
                        let plugin = InstalledPlugin {
                            manifest,
                            status: PluginStatus::Installed,
                            install_path: path,
                            config: serde_json::json!({}),
                        };
                        self.installed.insert(name, plugin);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load plugin from {:?}: {}", manifest_path, e);
                    }
                }
            }
        }
        Ok(())
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new(PathBuf::from(".forge/plugins"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.into(),
            version: PluginVersion::new(0, 1, 0),
            description: format!("Test plugin {}", name),
            keywords: vec![],
            dependencies: vec![],
            provides_tools: vec![format!("{}_tool", name)],
            provides_prompts: vec![],
            provides_resources: vec![],
            hooks: vec![],
            config_schema: None,
            metadata: PluginMetadata {
                author: "Test".into(),
                homepage: None,
                repository: None,
                license: None,
                installed_at: "2024-01-01T00:00:00Z".into(),
                updated_at: None,
            },
            entry_point: None,
        }
    }

    #[test]
    fn test_registry_creation() {
        let registry = PluginRegistry::default();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_registry_install() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("my-plugin")).unwrap();
        assert_eq!(registry.count(), 1);
        assert!(registry.is_installed("my-plugin"));
    }

    #[test]
    fn test_registry_uninstall() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("my-plugin")).unwrap();
        registry.uninstall("my-plugin").unwrap();
        assert_eq!(registry.count(), 0);
        assert!(!registry.is_installed("my-plugin"));
    }

    #[test]
    fn test_registry_uninstall_not_found() {
        let mut registry = PluginRegistry::default();
        assert!(registry.uninstall("nonexistent").is_err());
    }

    #[test]
    fn test_registry_enable_disable() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("my-plugin")).unwrap();

        registry.disable("my-plugin").unwrap();
        let plugin = registry.get("my-plugin").unwrap();
        assert!(matches!(plugin.status, PluginStatus::Disabled));

        registry.enable("my-plugin").unwrap();
        let plugin = registry.get("my-plugin").unwrap();
        assert!(matches!(plugin.status, PluginStatus::Installed));
    }

    #[test]
    fn test_registry_search() {
        let mut registry = PluginRegistry::default();
        let mut m1 = test_manifest("code-formatter");
        m1.description = "Format code nicely".into();
        let mut m2 = test_manifest("deploy-runner");
        m2.description = "Run deployments".into();
        let mut m3 = test_manifest("lint-checker");
        m3.description = "Check code style".into();
        registry.install(m1).unwrap();
        registry.install(m2).unwrap();
        registry.install(m3).unwrap();

        let results = registry.search("deploy");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].manifest.name, "deploy-runner");
    }

    #[test]
    fn test_registry_provides_tools() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("plugin-a")).unwrap();
        registry.install(test_manifest("plugin-b")).unwrap();

        let tools = registry.provides_tools();
        assert_eq!(tools.len(), 2);
        assert!(tools.contains(&"plugin-a_tool".to_string()));
        assert!(tools.contains(&"plugin-b_tool".to_string()));
    }

    #[test]
    fn test_registry_disabled_not_in_provides() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("plugin-a")).unwrap();
        registry.disable("plugin-a").unwrap();

        let tools = registry.provides_tools();
        assert_eq!(tools.len(), 0);
    }

    #[test]
    fn test_registry_update_config() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("my-plugin")).unwrap();

        let config = serde_json::json!({ "key": "value" });
        registry.update_config("my-plugin", config.clone()).unwrap();

        let plugin = registry.get("my-plugin").unwrap();
        assert_eq!(plugin.config, config);
    }

    #[test]
    fn test_registry_list() {
        let mut registry = PluginRegistry::default();
        registry.install(test_manifest("a")).unwrap();
        registry.install(test_manifest("b")).unwrap();

        let list = registry.list();
        assert_eq!(list.len(), 2);
    }
}
