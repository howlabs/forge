//! Skills discovery and loading
//!
//! Discovers and loads skills from the skills directory.

use super::types::Skill;
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

/// Discover and load skills from directory
pub struct SkillDiscovery {
    skills_dir: PathBuf,
}

impl SkillDiscovery {
    /// Create new skill discovery for given directory
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }

    /// Discover all skills in the skills directory
    pub fn discover(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();

        if !self.skills_dir.exists() {
            return Ok(skills);
        }

        for entry in fs::read_dir(&self.skills_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                let content = fs::read_to_string(&path)?;
                let skill = Skill::parse(&path, &content)?;
                skills.push(skill);
            }
        }

        Ok(skills)
    }

    /// Get skill by name
    pub fn get_skill(&self, name: &str) -> Result<Option<Skill>> {
        let skills = self.discover()?;
        Ok(skills.into_iter().find(|s| s.name == name))
    }

    /// Check if a skill exists
    pub fn has_skill(&self, name: &str) -> bool {
        self.get_skill(name).ok().flatten().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_discovery_empty_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        let skills = discovery.discover().unwrap();

        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn test_skill_discovery_with_skills() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        // Create test skill
        std::fs::write(
            skills_dir.join("test.md"),
            r#"---
name: test
description: Test skill
---

Test prompt"#,
        ).unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        let skills = discovery.discover().unwrap();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "test");
    }

    #[test]
    fn test_skill_discovery_multiple_skills() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        // Create multiple skills
        std::fs::write(
            skills_dir.join("skill1.md"),
            r#"---
name: skill1
description: First skill
---

Prompt 1"#,
        ).unwrap();

        std::fs::write(
            skills_dir.join("skill2.md"),
            r#"---
name: skill2
description: Second skill
---

Prompt 2"#,
        ).unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        let skills = discovery.discover().unwrap();

        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_skill_get_by_name() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("target.md"),
            r#"---
name: target-skill
description: Target skill
---

Target prompt"#,
        ).unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        let skill = discovery.get_skill("target-skill").unwrap();

        assert!(skill.is_some());
        assert_eq!(skill.unwrap().name, "target-skill");
    }

    #[test]
    fn test_skill_get_nonexistent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        let skill = discovery.get_skill("nonexistent").unwrap();

        assert!(skill.is_none());
    }

    #[test]
    fn test_skill_has_skill() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        std::fs::write(
            skills_dir.join("exists.md"),
            r#"---
name: exists
description: Exists skill
---

Exists"#,
        ).unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        assert!(discovery.has_skill("exists"));
        assert!(!discovery.has_skill("notexists"));
    }

    #[test]
    fn test_skill_discovery_nonexistent_dir() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("nonexistent");

        let discovery = SkillDiscovery::new(skills_dir);
        let skills = discovery.discover().unwrap();

        assert_eq!(skills.len(), 0);
    }

    #[test]
    fn test_skill_discovery_ignores_non_markdown() {
        let temp_dir = tempfile::tempdir().unwrap();
        let skills_dir = temp_dir.path().join("skills");
        std::fs::create_dir(&skills_dir).unwrap();

        // Create non-markdown file
        std::fs::write(skills_dir.join("not_skill.txt"), "Not a skill").unwrap();

        let discovery = SkillDiscovery::new(skills_dir);
        let skills = discovery.discover().unwrap();

        assert_eq!(skills.len(), 0);
    }
}
