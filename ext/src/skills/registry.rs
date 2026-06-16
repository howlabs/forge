//! Skills registry - manages skills with versioning and dependencies

use super::types::Skill;
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Skill version
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SkillVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SkillVersion {
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 3 {
            return Err(format!("Invalid version: {}", s));
        }
        Ok(Self {
            major: parts[0]
                .parse::<u32>()
                .map_err(|e: std::num::ParseIntError| e.to_string())?,
            minor: parts[1]
                .parse::<u32>()
                .map_err(|e: std::num::ParseIntError| e.to_string())?,
            patch: parts[2]
                .parse::<u32>()
                .map_err(|e: std::num::ParseIntError| e.to_string())?,
        })
    }
}

impl std::fmt::Display for SkillVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Registered skill with metadata
#[derive(Debug, Clone)]
pub struct RegisteredSkill {
    pub skill: Skill,
    pub version: SkillVersion,
    pub author: String,
    pub tags: Vec<String>,
    pub dependencies: Vec<String>,
}

/// Skills registry
pub struct SkillsRegistry {
    skills: HashMap<String, RegisteredSkill>,
    discovery_paths: Vec<PathBuf>,
}

impl SkillsRegistry {
    pub fn new() -> Self {
        Self {
            skills: HashMap::new(),
            discovery_paths: Vec::new(),
        }
    }

    pub fn add_discovery_path(&mut self, path: PathBuf) {
        self.discovery_paths.push(path);
    }

    pub fn discovery_paths(&self) -> &[PathBuf] {
        &self.discovery_paths
    }

    pub fn register(&mut self, registered: RegisteredSkill) -> Result<()> {
        let name = registered.skill.name.clone();
        self.skills.insert(name, registered);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Result<()> {
        self.skills
            .remove(name)
            .ok_or_else(|| anyhow::anyhow!("Skill not found: {}", name))?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&RegisteredSkill> {
        self.skills.get(name)
    }

    pub fn list(&self) -> Vec<&RegisteredSkill> {
        self.skills.values().collect()
    }

    pub fn search(&self, query: &str) -> Vec<&RegisteredSkill> {
        let query_lower = query.to_lowercase();
        self.skills
            .values()
            .filter(|s| {
                s.skill.name.to_lowercase().contains(&query_lower)
                    || s.skill.description.to_lowercase().contains(&query_lower)
                    || s.tags
                        .iter()
                        .any(|t| t.to_lowercase().contains(&query_lower))
            })
            .collect()
    }

    pub fn by_tag(&self, tag: &str) -> Vec<&RegisteredSkill> {
        self.skills
            .values()
            .filter(|s| s.tags.contains(&tag.to_string()))
            .collect()
    }

    pub fn count(&self) -> usize {
        self.skills.len()
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.skills.contains_key(name)
    }

    pub fn load_from_discovery(&mut self) -> Result<()> {
        for path in &self.discovery_paths.clone() {
            if !path.exists() {
                continue;
            }
            let discovery = super::discovery::SkillDiscovery::new(path.clone());
            for skill in discovery.discover()? {
                let name = skill.name.clone();
                if !self.is_registered(&name) {
                    self.register(RegisteredSkill {
                        skill,
                        version: SkillVersion::new(0, 1, 0),
                        author: "unknown".into(),
                        tags: vec![],
                        dependencies: vec![],
                    })?;
                }
            }
        }
        Ok(())
    }
}

impl Default for SkillsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_skill(name: &str) -> RegisteredSkill {
        let skill_content = format!(
            r#"---
name: {}
description: Test skill {}
---
Test prompt for {}"#,
            name, name, name
        );
        let skill = Skill::parse(format!("{}.md", name), &skill_content).unwrap();
        RegisteredSkill {
            skill,
            version: SkillVersion::new(0, 1, 0),
            author: "Test".into(),
            tags: vec!["test".into()],
            dependencies: vec![],
        }
    }

    #[test]
    fn test_registry_creation() {
        let registry = SkillsRegistry::new();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_registry_register() {
        let mut registry = SkillsRegistry::new();
        registry.register(test_skill("my-skill")).unwrap();
        assert_eq!(registry.count(), 1);
        assert!(registry.is_registered("my-skill"));
    }

    #[test]
    fn test_registry_unregister() {
        let mut registry = SkillsRegistry::new();
        registry.register(test_skill("my-skill")).unwrap();
        registry.unregister("my-skill").unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_registry_unregister_not_found() {
        let mut registry = SkillsRegistry::new();
        assert!(registry.unregister("nonexistent").is_err());
    }

    #[test]
    fn test_registry_search() {
        let mut registry = SkillsRegistry::new();
        let mut s1 = test_skill("code-formatter");
        s1.skill.description = "Format code nicely".into();
        let mut s2 = test_skill("deploy-runner");
        s2.skill.description = "Run deployment steps".into();
        registry.register(s1).unwrap();
        registry.register(s2).unwrap();

        let results = registry.search("deploy");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].skill.name, "deploy-runner");
    }

    #[test]
    fn test_registry_by_tag() {
        let mut registry = SkillsRegistry::new();
        let mut s1 = test_skill("a");
        s1.tags = vec!["format".into()];
        let mut s2 = test_skill("b");
        s2.tags = vec!["test".into()];
        registry.register(s1).unwrap();
        registry.register(s2).unwrap();

        let results = registry.by_tag("test");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].skill.name, "b");
    }

    #[test]
    fn test_registry_list() {
        let mut registry = SkillsRegistry::new();
        registry.register(test_skill("a")).unwrap();
        registry.register(test_skill("b")).unwrap();
        assert_eq!(registry.list().len(), 2);
    }

    #[test]
    fn test_skill_version_parse() {
        let v = SkillVersion::parse("1.2.3").unwrap();
        assert_eq!(v, SkillVersion::new(1, 2, 3));
        assert_eq!(v.to_string(), "1.2.3");
    }

    #[test]
    fn test_skill_version_invalid() {
        assert!(SkillVersion::parse("1.2").is_err());
    }

    #[test]
    fn test_load_from_discovery() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("test.md"),
            r#"---
name: discovered-skill
description: A discovered skill
---

Discovered prompt"#,
        )
        .unwrap();

        let mut registry = SkillsRegistry::new();
        registry.add_discovery_path(skills_dir);
        registry.load_from_discovery().unwrap();

        assert_eq!(registry.count(), 1);
        assert!(registry.is_registered("discovered-skill"));
    }
}
