//! Command palette - slash commands for Forge TUI
//!
//! Provides keyboard-driven commands:
//! - /model - Change model
//! - /context - Manage context
//! - /agents - Manage agents
//! - /resume - Resume from checkpoint
//! - /diff - Show diff viewer
//! - /plan - Enter plan mode
//! - /review - Request code review
//! - /init - Initialize project
//! - /help - Show help

use serde::{Deserialize, Serialize};

/// Available slash commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SlashCommand {
    /// Change model: /model <model_name>
    Model { model: String },
    /// Manage context: /context <add|remove|list> [path]
    Context { action: String, path: Option<String> },
    /// Manage agents: /agents <list|kill> [agent_id]
    Agents { action: String, agent_id: Option<String> },
    /// Resume from checkpoint: /resume <task_id>
    Resume { task_id: String },
    /// Show diff viewer: /diff [file_path]
    Diff { file_path: Option<String> },
    /// Enter plan mode: /plan
    Plan,
    /// Request code review: /review [file_path]
    Review { file_path: Option<String> },
    /// Initialize project: /init
    Init,
    /// Show help: /help
    Help,
}

impl SlashCommand {
    /// Parse command from input string
    pub fn parse(input: &str) -> Option<Self> {
        let parts: Vec<&str> = input.split_whitespace().collect();
        if parts.is_empty() || !parts[0].starts_with('/') {
            return None;
        }

        let command = parts[0].trim_start_matches('/');

        Some(match command {
            "model" if parts.len() > 1 => SlashCommand::Model {
                model: parts[1].to_string(),
            },
            "context" if parts.len() > 1 => SlashCommand::Context {
                action: parts[1].to_string(),
                path: parts.get(2).map(|s| s.to_string()),
            },
            "agents" if parts.len() > 1 => SlashCommand::Agents {
                action: parts[1].to_string(),
                agent_id: parts.get(2).map(|s| s.to_string()),
            },
            "resume" if parts.len() > 1 => SlashCommand::Resume {
                task_id: parts[1].to_string(),
            },
            "diff" => SlashCommand::Diff {
                file_path: parts.get(1).map(|s| s.to_string()),
            },
            "plan" => SlashCommand::Plan,
            "review" => SlashCommand::Review {
                file_path: parts.get(1).map(|s| s.to_string()),
            },
            "init" => SlashCommand::Init,
            "help" => SlashCommand::Help,
            _ => return None,
        })
    }

    /// Get command description for help
    pub fn description(&self) -> String {
        match self {
            SlashCommand::Model { .. } => "Change AI model (e.g., /model claude-3-5-sonnet)".to_string(),
            SlashCommand::Context { .. } => "Manage context (add/remove/list files)".to_string(),
            SlashCommand::Agents { .. } => "Manage parallel agents (list/kill)".to_string(),
            SlashCommand::Resume { .. } => "Resume from checkpoint".to_string(),
            SlashCommand::Diff { .. } => "Show diff viewer".to_string(),
            SlashCommand::Plan => "Enter plan mode (no edits)".to_string(),
            SlashCommand::Review { .. } => "Request code review".to_string(),
            SlashCommand::Init => "Initialize Forge in current directory".to_string(),
            SlashCommand::Help => "Show this help message".to_string(),
        }
    }
}

/// Command palette state
pub struct CommandPalette {
    /// Current command input
    input: String,
    /// Active command being edited
    active_command: Option<SlashCommand>,
    /// Whether palette is visible
    visible: bool,
}

impl CommandPalette {
    /// Create new command palette
    pub fn new() -> Self {
        Self {
            input: String::new(),
            active_command: None,
            visible: false,
        }
    }

    /// Show the command palette
    pub fn show(&mut self) {
        self.visible = true;
        self.input.clear();
        self.active_command = None;
    }

    /// Hide the command palette
    pub fn hide(&mut self) {
        self.visible = false;
        self.input.clear();
        self.active_command = None;
    }

    /// Add character to input
    pub fn add_char(&mut self, c: char) {
        if self.visible {
            self.input.push(c);
        }
    }

    /// Remove last character
    pub fn remove_char(&mut self) {
        if self.visible && !self.input.is_empty() {
            self.input.pop();
        }
    }

    /// Get current input
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Parse and execute current command
    pub fn execute(&mut self) -> Option<SlashCommand> {
        if !self.visible || self.input.is_empty() {
            return None;
        }

        let command = SlashCommand::parse(&self.input);
        if let Some(cmd) = &command {
            self.active_command = Some(cmd.clone());
        }

        self.hide();
        command
    }

    /// Get available commands for help
    pub fn available_commands() -> Vec<&'static str> {
        vec![
            "/model <name>",
            "/context <add|remove|list> [path]",
            "/agents <list|kill> [id]",
            "/resume <task_id>",
            "/diff [path]",
            "/plan",
            "/review [path]",
            "/init",
            "/help",
        ]
    }
}

impl Default for CommandPalette {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_model_command() {
        let cmd = SlashCommand::parse("/model claude-3-5-sonnet");
        assert!(matches!(cmd, Some(SlashCommand::Model { .. })));
    }

    #[test]
    fn test_parse_context_command() {
        let cmd = SlashCommand::parse("/context add src/main.rs");
        assert!(matches!(cmd, Some(SlashCommand::Context { .. })));
    }

    #[test]
    fn test_parse_plan_command() {
        let cmd = SlashCommand::parse("/plan");
        assert!(matches!(cmd, Some(SlashCommand::Plan)));
    }

    #[test]
    fn test_parse_invalid_command() {
        let cmd = SlashCommand::parse("/invalid command");
        assert!(cmd.is_none());
    }

    #[test]
    fn test_command_palette_creation() {
        let palette = CommandPalette::new();
        assert!(!palette.visible);
        assert!(palette.input().is_empty());
    }

    #[test]
    fn test_show_hide() {
        let mut palette = CommandPalette::new();
        palette.show();
        assert!(palette.visible);
        palette.hide();
        assert!(!palette.visible);
    }

    #[test]
    fn test_add_remove_char() {
        let mut palette = CommandPalette::new();
        palette.show();
        palette.add_char('a');
        palette.add_char('b');
        assert_eq!(palette.input(), "ab");
        palette.remove_char();
        assert_eq!(palette.input(), "a");
    }
}