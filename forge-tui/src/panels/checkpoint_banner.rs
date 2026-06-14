//! Checkpoint banner - shows when long-horizon resume is available
//!
//! Displays a banner at the top of the TUI when a checkpoint is available
//! for resume, allowing one-key resume (Forge's long-horizon checkpointing).

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

/// Checkpoint banner for resume availability
pub struct CheckpointBanner {
    /// Task ID that can be resumed
    task_id: String,
    /// Whether banner is visible
    visible: bool,
}

impl CheckpointBanner {
    /// Create new checkpoint banner
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            visible: true,
        }
    }

    /// Hide the banner
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Show the banner
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Check if banner is visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Render the checkpoint banner
    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible {
            return;
        }

        let text = vec![Line::from(vec![
            Span::styled("⚡ ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!(
                    "Checkpoint available: {} - Press 'R' to resume",
                    self.task_id
                ),
                Style::default().fg(Color::Yellow),
            ),
        ])];

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title_style(Style::default().fg(Color::Yellow)),
        );

        f.render_widget(paragraph, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_banner_creation() {
        let banner = CheckpointBanner::new("task-123".to_string());
        assert!(banner.is_visible());
        assert_eq!(banner.task_id, "task-123");
    }

    #[test]
    fn test_hide_show() {
        let mut banner = CheckpointBanner::new("task-456".to_string());
        banner.hide();
        assert!(!banner.is_visible());
        banner.show();
        assert!(banner.is_visible());
    }
}
