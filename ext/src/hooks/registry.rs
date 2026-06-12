//! Hook registry for loading and managing hooks
//!
//! Loads user-defined hooks from forge.toml configuration.

use super::types::HookEvent;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Registry of user-defined hooks
pub struct HookRegistry {
    hooks: HashMap<String, Vec<PathBuf>>,
}

impl HookRegistry {
    /// Create new empty hook registry
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Register a hook script for an event type
    pub fn register_hook(&mut self, event_type: &str, script_path: PathBuf) {
        self.hooks
            .entry(event_type.to_string())
            .or_default()
            .push(script_path);
    }

    /// Load hooks from forge.toml config
    pub fn load_from_config(config: &toml::Table) -> Result<Self> {
        let mut registry = Self::new();

        if let Some(hooks_table) = config.get("hooks") {
            if let Some(hooks) = hooks_table.as_table() {
                for (event, scripts) in hooks {
                    if let Some(scripts_array) = scripts.as_array() {
                        for script in scripts_array {
                            if let Some(script_str) = script.as_str() {
                                registry.register_hook(event, PathBuf::from(script_str));
                            }
                        }
                    }
                }
            }
        }

        Ok(registry)
    }

    /// Get all hooks for an event type
    pub fn get_hooks(&self, event_type: &str) -> Vec<PathBuf> {
        self.hooks
            .get(event_type)
            .cloned()
            .unwrap_or_default()
    }

    /// Trigger all hooks for an event (placeholder for v0.190.0)
    pub async fn trigger(&self, event: &HookEvent) -> Result<()> {
        let event_type = match event {
            HookEvent::TaskCreated(_) => "TaskCreated",
            HookEvent::TaskCompleted(_) => "TaskCompleted",
            HookEvent::PreEdit(_) => "PreEdit",
            HookEvent::PostVerify(_) => "PostVerify",
        };

        let hooks = self.get_hooks(event_type);
        let payload = serde_json::to_string(event)?;

        for hook_script in hooks {
            if hook_script.exists() {
                // Execute hook script with payload as stdin
                // For v0.190.0, just log (full execution in next task)
                tracing::info!("Executing hook: {:?} for event: {}", hook_script, event_type);
                tracing::debug!("Hook payload: {}", payload);
            } else {
                tracing::warn!("Hook script not found: {:?}", hook_script);
            }
        }

        Ok(())
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::types::TaskCreatedEvent;

    #[test]
    fn test_hook_registry_creation() {
        let registry = HookRegistry::new();
        assert_eq!(registry.get_hooks("TaskCreated").len(), 0);
    }

    #[test]
    fn test_hook_registry_register() {
        let mut registry = HookRegistry::new();
        registry.register_hook("TaskCreated", PathBuf::from("/path/to/hook.sh"));
        assert_eq!(registry.get_hooks("TaskCreated").len(), 1);
    }

    #[test]
    fn test_hook_registry_multiple_hooks() {
        let mut registry = HookRegistry::new();
        registry.register_hook("TaskCreated", PathBuf::from("/hook1.sh"));
        registry.register_hook("TaskCreated", PathBuf::from("/hook2.sh"));
        assert_eq!(registry.get_hooks("TaskCreated").len(), 2);
    }

    #[test]
    fn test_hook_registry_load_from_config() {
        let config_toml = r#"
[hooks]
TaskCreated = ["/hook1.sh", "/hook2.sh"]
TaskCompleted = ["/hook3.sh"]
"#;

        let config: toml::Table = toml::from_str(config_toml).unwrap();
        let registry = HookRegistry::load_from_config(&config).unwrap();

        assert_eq!(registry.get_hooks("TaskCreated").len(), 2);
        assert_eq!(registry.get_hooks("TaskCompleted").len(), 1);
        assert_eq!(registry.get_hooks("PostVerify").len(), 0);
    }

    #[test]
    fn test_hook_registry_trigger_no_hooks() {
        let registry = HookRegistry::new();
        let event = HookEvent::TaskCreated(TaskCreatedEvent {
            task_id: "test".to_string(),
            description: "Test".to_string(),
        });

        // Should not fail even with no hooks registered
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            registry.trigger(&event).await
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_hook_registry_trigger_with_hooks() {
        let mut registry = HookRegistry::new();

        // Use a temp file that exists
        let temp_dir = tempfile::tempdir().unwrap();
        let hook_path = temp_dir.path().join("test_hook.sh");
        std::fs::write(&hook_path, "#!/bin/bash\necho test").unwrap();

        registry.register_hook("TaskCreated", hook_path);

        let event = HookEvent::TaskCreated(TaskCreatedEvent {
            task_id: "test".to_string(),
            description: "Test".to_string(),
        });

        // Should not fail
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            registry.trigger(&event).await
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_hook_registry_default() {
        let registry = HookRegistry::default();
        assert_eq!(registry.get_hooks("TaskCreated").len(), 0);
    }
}
