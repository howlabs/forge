//! Main TUI application struct and event loop
//!
//! The TuiApp is the central coordinator that manages:
//! - UI rendering and layout
//! - Input handling and keyboard events
//! - Communication with the core Forge system
//! - State management for all panels

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{backend::Backend, Terminal};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::{panels::Panels, TuiConfig};
use ratatui::layout::{Constraint, Direction, Layout};

// Import provider types for real streaming
pub use provider::types::{Message, MessageRole};
pub use provider::ModelProvider;

// Import Forge types for TUI integration
use context::ContextEngine;
use sandbox::Sandbox;

/// Main TUI application
pub struct TuiApp {
    /// Application configuration
    config: TuiConfig,
    /// UI panels (conversation, input, diff, agent activity, etc.)
    panels: Panels,
    /// Running state
    running: bool,
    /// Event receiver
    event_rx: mpsc::UnboundedReceiver<AppEvent>,
    /// Optional model provider for real streaming
    _provider: Option<Arc<dyn ModelProvider>>,
    /// Optional context engine
    _context: Option<Arc<context::ContextEngine>>,
    /// Optional sandbox
    _sandbox: Option<Arc<sandbox::Sandbox>>,
}

/// Events that can update the TUI state
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// User keyboard input
    KeyEvent(KeyEvent),
    /// Streaming content from model
    StreamContent(String),
    /// Agent status update
    AgentStatusUpdate { id: String, status: String },
    /// Token usage update
    TokenUpdate { used: u32, budget: u32 },
    /// Verification state change
    VerifyStateChanged(String),
    /// Checkpoint available
    CheckpointAvailable(String),
}

impl TuiApp {
    /// Create a new TUI application
    pub fn new(config: TuiConfig) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            config,
            panels: Panels::new(event_tx),
            running: true,
            event_rx,
            _provider: None,
            _context: None,
            _sandbox: None,
        }
    }

    /// Create a new TUI application with real provider integration
    pub fn with_provider(
        config: TuiConfig,
        provider: Arc<dyn ModelProvider>,
        context: Arc<ContextEngine>,
        sandbox: Arc<Sandbox>,
    ) -> Self {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        Self {
            config,
            panels: Panels::new(event_tx),
            running: true,
            event_rx,
            _provider: Some(provider),
            _context: Some(context),
            _sandbox: Some(sandbox),
        }
    }

    /// Run the TUI main loop
    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;

        let res = self.run_inner(terminal).await;

        // Restore terminal
        disable_raw_mode()?;

        res
    }

    /// Inner run loop
    async fn run_inner<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        let mut tick_rate = tokio::time::interval(Duration::from_millis(16)); // ~60fps

        while self.running {
            // Handle events
            tokio::select! {
                // Event from TUI
                Some(event) = self.event_rx.recv() => {
                    self.handle_app_event(event);
                }

                // Keyboard input
                _ = tick_rate.tick() => {
                    if event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key) = event::read()? {
                            self.handle_key_event(key);
                        }
                    }
                }
            }

            // Render
            terminal.draw(|f| self.render(f))?;
        }

        Ok(())
    }

    /// Handle application events
    fn handle_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::KeyEvent(key) => self.handle_key_event(key),
            AppEvent::StreamContent(content) => {
                self.panels.conversation.add_content(content);
            }
            AppEvent::AgentStatusUpdate { id, status } => {
                self.panels.agent_panel.update_status(id, status);
            }
            AppEvent::TokenUpdate { used, budget } => {
                self.panels.status_bar.update_tokens(used, budget);
            }
            AppEvent::VerifyStateChanged(state) => {
                self.panels.status_bar.update_verify_state(state);
            }
            AppEvent::CheckpointAvailable(task_id) => {
                self.panels.show_checkpoint_banner(&task_id);
            }
        }
    }

    /// Handle keyboard events
    fn handle_key_event(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.running = false;
            }
            KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                self.running = false;
            }
            // Tab to toggle Plan/Build mode
            KeyCode::Tab => {
                self.panels.toggle_plan_build_mode();
            }
            // Enter-Enter to queue message (soft steer)
            KeyCode::Enter => {
                if self.panels.input.has_content() {
                    self.panels.input.queue_message();
                }
            }
            _ => {
                // Delegate to focused panel
                self.panels.handle_key_event(key);
            }
        }
    }

    /// Render the UI
    fn render(&self, f: &mut ratatui::Frame) {
        // Layout calculations based on terminal size
        let size = f.size();

        // If not in fullscreen mode, leave room for native scrollback
        let render_area = if self.config.fullscreen {
            size
        } else {
            // Reserve bottom portion for native terminal scrollback
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(80), // TUI area
                    Constraint::Percentage(20), // Native scrollback area
                ])
                .split(size);

            chunks[0]
        };

        // Render panels
        self.panels.render(f, render_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_creation() {
        let config = TuiConfig::default();
        let app = TuiApp::new(config);
        assert!(app.running);
    }

    #[test]
    fn test_app_event_handling() {
        let config = TuiConfig::default();
        let mut app = TuiApp::new(config);

        // Test quit event
        app.handle_app_event(AppEvent::KeyEvent(KeyEvent::new(
            KeyCode::Char('q'),
            event::KeyModifiers::empty(),
        )));
        assert!(!app.running);
    }
}
