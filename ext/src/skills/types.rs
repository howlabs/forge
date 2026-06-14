//! Skills framework types
//!
//! Defines reusable skill structure with prompt and optional tools.

use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

/// Reusable skill with prompt and optional tools
#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub prompt: String,
    pub tools: Vec<String>,
    pub file_path: PathBuf,
}

impl Skill {
    /// Parse skill file (frontmatter + markdown prompt)
    pub fn parse(file_path: impl Into<PathBuf>, content: &str) -> Result<Self> {
        let file_path = file_path.into();

        // Split frontmatter and content
        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            return Err(anyhow::anyhow!("Invalid skill format: missing frontmatter"));
        }

        // Parse frontmatter
        let frontmatter: SkillFrontmatter = serde_yaml::from_str(parts[1])?;
        let prompt = parts[2].to_string();

        Ok(Self {
            name: frontmatter.name,
            description: frontmatter.description,
            prompt,
            tools: frontmatter.tools.unwrap_or_default(),
            file_path,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
    tools: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skill_parsing() {
        let skill_content = r#"---
name: test-skill
description: A test skill
---

This is a test skill prompt."#;

        let skill = Skill::parse("test.md", skill_content).unwrap();
        assert_eq!(skill.name, "test-skill");
        assert_eq!(skill.description, "A test skill");
        assert!(skill.prompt.contains("This is a test skill prompt."));
        assert_eq!(skill.tools.len(), 0);
    }

    #[test]
    fn test_skill_with_tools() {
        let skill_content = r#"---
name: refactor-skill
description: Code refactoring skill
tools:
  - edit
  - search
---

Refactor code to improve clarity and maintainability."#;

        let skill = Skill::parse("refactor.md", skill_content).unwrap();
        assert_eq!(skill.name, "refactor-skill");
        assert_eq!(skill.tools.len(), 2);
        assert!(skill.tools.contains(&"edit".to_string()));
        assert!(skill.tools.contains(&"search".to_string()));
    }

    #[test]
    fn test_skill_invalid_format() {
        let skill_content = "No frontmatter here";
        let result = Skill::parse("invalid.md", skill_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_skill_multiline_prompt() {
        let skill_content = r#"---
name: multi-line
description: Multi line test
---

Line 1
Line 2
Line 3"#;

        let skill = Skill::parse("multi.md", skill_content).unwrap();
        assert!(skill.prompt.contains("Line 1"));
        assert!(skill.prompt.contains("Line 2"));
        assert!(skill.prompt.contains("Line 3"));
    }
}
