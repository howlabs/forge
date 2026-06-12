//! Agent activity panel - live status of parallel subagents
//!
//! This panel displays the real-time status of parallel subagents,
//! which is one of Forge's key differentiators. Shows:
//! - Agent ID and task description
//! - Current state (Pending, Running, Verifying, Done, Failed)
//! - Git worktree path
//! - Progress indicators

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use std::collections::HashMap;

/// Agent activity panel
pub struct AgentActivityPanel {
    /// Agent states indexed by agent ID
    agents: HashMap<String, AgentState>,
    /// Currently selected agent (for details)
    selected_agent: Option<String>,
}

/// State of a single agent
#[derive(Debug, Clone)]
pub struct AgentState {
    /// Agent ID
    pub id: String,
    /// Task description
    pub task: String,
    /// Git worktree path
    pub worktree: String,
    /// Current status
    pub status: AgentStatus,
    /// Progress (0.0 to 1.0)
    pub progress: f32,
    /// Number of tool calls executed
    pub tool_calls: u32,
    /// Elapsed time (seconds)
    pub elapsed: u32,
}

/// Agent status
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AgentStatus {
    /// Agent created but not started
    Pending,
    /// Agent is executing
    Running,
    /// Agent is post-execution verification
    Verifying,
    /// Agent completed successfully
    Done,
    /// Agent failed
    Failed,
}

impl AgentStatus {
    /// Get display color for status
    fn color(self) -> Color {
        match self {
            AgentStatus::Pending => Color::Gray,
            AgentStatus::Running => Color::Blue,
            AgentStatus::Verifying => Color::Yellow,
            AgentStatus::Done => Color::Green,
            AgentStatus::Failed => Color::Red,
        }
    }

    /// Get display character for status
    fn icon(self) -> &'static str {
        match self {
            AgentStatus::Pending => "○",
            AgentStatus::Running => "●",
            AgentStatus::Verifying => "◐",
            AgentStatus::Done => "✓",
            AgentStatus::Failed => "✗",
        }
    }
}

impl AgentActivityPanel {
    /// Create new agent activity panel
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            selected_agent: None,
        }
    }

    /// Update agent status
    pub fn update_status(&mut self, id: String, status: String) {
        let agent = self.agents.entry(id.clone()).or_insert_with(|| {
            AgentState {
                id: id.clone(),
                task: "Unknown task".to_string(),
                worktree: String::new(),
                status: AgentStatus::Pending,
                progress: 0.0,
                tool_calls: 0,
                elapsed: 0,
            }
        });

        // Parse status string to enum
        agent.status = match status.as_str() {
            "Running" => AgentStatus::Running,
            "Verifying" => AgentStatus::Verifying,
            "Done" => AgentStatus::Done,
            "Failed" => AgentStatus::Failed,
            _ => AgentStatus::Pending,
        };
    }

    /// Add or update an agent
    pub fn update_agent(&mut self, state: AgentState) {
        self.agents.insert(state.id.clone(), state);
    }

    /// Remove an agent (when completed/failed)
    pub fn remove_agent(&mut self, id: &str) {
        self.agents.remove(id);
        if self.selected_agent.as_ref() == Some(&id.to_string()) {
            self.selected_agent = None;
        }
    }

    /// Clear all agents
    pub fn clear(&mut self) {
        self.agents.clear();
        self.selected_agent = None;
    }

    /// Get number of active agents (running or verifying)
    pub fn active_count(&self) -> usize {
        self.agents.values()
            .filter(|a| matches!(a.status, AgentStatus::Running | AgentStatus::Verifying))
            .count()
    }

    /// Render the agent panel
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let mut lines = Vec::new();

        // Header
        let active_count = self.active_count();
        lines.push(Line::from(vec![
            Span::styled(
                format!("Parallel Agents ({} active)", active_count),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
        ]));

        lines.push(Line::from(""));

        // Sort agents by status (active first)
        let mut agents: Vec<_> = self.agents.values().collect();
        agents.sort_by_key(|a| {
            match a.status {
                AgentStatus::Running | AgentStatus::Verifying => 0,
                AgentStatus::Pending => 1,
                AgentStatus::Done => 2,
                AgentStatus::Failed => 3,
            }
        });

        // Render each agent
        for agent in agents {
            let status_color = agent.status.color();
            let status_icon = agent.status.icon();

            lines.push(Line::from(vec![
                Span::styled(status_icon, Style::default().fg(status_color)),
                Span::raw(" "),
                Span::styled(&agent.id, Style::default().fg(Color::White)),
                Span::raw(": "),
                Span::styled(
                    &agent.task,
                    Style::default().fg(Color::Gray),
                ),
            ]));

            // Progress bar for running agents
            if matches!(agent.status, AgentStatus::Running | AgentStatus::Verifying) {
                let progress_width = 20;
                let filled = (agent.progress * progress_width as f32) as usize;
                let progress_bar_content = format!("{}{}", "█".repeat(filled), "─".repeat((progress_width as usize).saturating_sub(filled)));
                let percentage = format!(" {}%", (agent.progress * 100.0) as u32);

                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(progress_bar_content, Style::default().fg(status_color)),
                    Span::styled(percentage, Style::default().fg(status_color)),
                ]));
            }

            // Additional info for selected agent
            if self.selected_agent.as_ref() == Some(&agent.id) {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("Worktree: {}", agent.worktree),
                        Style::default().fg(Color::Gray),
                    ),
                ]));

                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("Tool calls: {} | Elapsed: {}s", agent.tool_calls, agent.elapsed),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
            }

            lines.push(Line::from(""));
        }

        // Empty state
        if self.agents.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(
                    "No parallel agents running",
                    Style::default().fg(Color::Gray),
                ),
            ]));
        }

        let paragraph = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
                    .title("Agent Activity"),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_panel_creation() {
        let panel = AgentActivityPanel::new();
        assert_eq!(panel.active_count(), 0);
    }

    #[test]
    fn test_update_status() {
        let mut panel = AgentActivityPanel::new();
        panel.update_status("agent-1".to_string(), "Running".to_string());
        assert_eq!(panel.active_count(), 1);
    }

    #[test]
    fn test_clear() {
        let mut panel = AgentActivityPanel::new();
        panel.update_status("agent-1".to_string(), "Running".to_string());
        panel.clear();
        assert_eq!(panel.active_count(), 0);
    }
}