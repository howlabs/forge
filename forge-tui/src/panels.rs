//! UI panels - conversation, input, diff viewer, agent activity, status bar
//!
//! This module contains all the UI panels that make up the TUI interface.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::app::AppEvent;

pub mod agent_panel;
pub mod checkpoint_banner;
pub mod conversation;
pub mod diff_viewer;
pub mod input;
pub mod status_bar;

use agent_panel::AgentActivityPanel;
use checkpoint_banner::CheckpointBanner;
use conversation::ConversationPanel;
use diff_viewer::DiffViewer;
use input::InputBox;
use status_bar::StatusBar;

/// Collection of all UI panels
pub struct Panels {
    /// Conversation/streaming panel
    pub conversation: ConversationPanel,
    /// Input box for user messages
    pub input: InputBox,
    /// Diff viewer for code changes
    pub diff_viewer: DiffViewer,
    /// Agent activity panel
    pub agent_panel: AgentActivityPanel,
    /// Status bar at bottom
    pub status_bar: StatusBar,
    /// Checkpoint banner (shows when resume available)
    pub checkpoint_banner: Option<CheckpointBanner>,
    /// Event sender
    _event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
    /// Current focus (which panel gets keyboard input)
    focus: Focus,
    /// Plan vs Build mode
    plan_mode: bool,
}

/// Which panel has keyboard focus
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum Focus {
    Input,
    DiffViewer,
    Conversation,
}

impl Panels {
    /// Create new panels collection
    pub fn new(event_tx: tokio::sync::mpsc::UnboundedSender<AppEvent>) -> Self {
        Self {
            conversation: ConversationPanel::new(),
            input: InputBox::new(),
            diff_viewer: DiffViewer::new(),
            agent_panel: AgentActivityPanel::new(),
            status_bar: StatusBar::new(),
            checkpoint_banner: None,
            _event_tx: event_tx,
            focus: Focus::Input,
            plan_mode: false,
        }
    }

    /// Render all panels
    pub fn render(&self, f: &mut Frame, area: Rect) {
        // Main layout
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Checkpoint banner (if visible)
                Constraint::Min(0),    // Main content area
                Constraint::Length(1), // Status bar
            ])
            .split(area);

        // Checkpoint banner (conditional)
        if let Some(banner) = &self.checkpoint_banner {
            banner.render(f, main_chunks[0]);
        }

        // Main content area layout
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(70), // Conversation + Input + Diff
                Constraint::Percentage(30), // Agent panel
            ])
            .split(main_chunks[1]);

        // Left side (Conversation, Input, Diff)
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(50), // Conversation
                Constraint::Percentage(30), // Diff viewer
                Constraint::Percentage(20), // Input box
            ])
            .split(content_chunks[0]);

        // Right side (Agent panel)
        self.agent_panel.render(f, content_chunks[1]);

        // Render left panels
        self.conversation.render(f, left_chunks[0]);
        self.diff_viewer.render(f, left_chunks[1]);
        self.input.render(f, left_chunks[2], self.plan_mode);

        // Status bar
        self.status_bar.render(f, main_chunks[2]);
    }

    /// Handle keyboard event (delegates to focused panel)
    pub fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) {
        match self.focus {
            Focus::Input => self.input.handle_key_event(key),
            Focus::DiffViewer => self.diff_viewer.handle_key_event(key),
            Focus::Conversation => {} // Conversation panel is read-only
        }
    }

    /// Toggle Plan/Build mode
    pub fn toggle_plan_build_mode(&mut self) {
        self.plan_mode = !self.plan_mode;
        // Update input mode
        self.input.set_plan_mode(self.plan_mode);
    }

    /// Show checkpoint banner
    pub fn show_checkpoint_banner(&mut self, task_id: &str) {
        self.checkpoint_banner = Some(CheckpointBanner::new(task_id.to_string()));
    }

    /// Check if in plan mode
    pub fn is_plan_mode(&self) -> bool {
        self.plan_mode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panels_creation() {
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let panels = Panels::new(event_tx);
        assert_eq!(panels.focus, Focus::Input);
        assert!(!panels.plan_mode);
    }

    #[test]
    fn test_plan_mode_toggle() {
        let (event_tx, _event_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut panels = Panels::new(event_tx);

        panels.toggle_plan_build_mode();
        assert!(panels.plan_mode);

        panels.toggle_plan_build_mode();
        assert!(!panels.plan_mode);
    }
}
