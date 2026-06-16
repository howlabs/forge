//! Forge TUI - Fast, keyboard-driven terminal UI for Forge
//!
//! This crate provides a best-in-class TUI experience that makes Forge's
//! unique strengths visible: parallel subagents, semantic context, verify loop,
//! checkpoint/resume.
//!
//! ## Architecture
//!
//! - **Thin Client**: The TUI is a VIEW over existing forge-core/forge-agents/
//!   forge-verify traits - business logic stays in the core
//! - **Hybrid Render**: Native terminal scrollback for conversation + ratatui
//!   overlays for input/diff/agent panel
//! - **Responsive**: Never blocks the UI thread (<16ms render time)

use anyhow::Result;
use ratatui::{backend::Backend, style::Color, Terminal};

pub mod app;
pub mod command_palette;
pub mod panels;
pub mod simple_tui;

// Re-export main components
pub use app::TuiApp;
pub use simple_tui::SimpleTui;

/// Forge TUI configuration
#[derive(Debug, Clone)]
pub struct TuiConfig {
    /// Whether to show full alt-screen mode or hybrid mode
    pub fullscreen: bool,
    /// Enable/disable agent activity panel
    pub show_agent_panel: bool,
    /// Theme color scheme
    pub theme: Theme,
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            fullscreen: false, // Hybrid mode by default
            show_agent_panel: false,
            theme: Theme::default(),
        }
    }
}

/// Color theme for the TUI
#[derive(Debug, Clone)]
pub struct Theme {
    /// Primary color for UI elements
    pub primary: Color,
    /// Secondary color
    pub secondary: Color,
    /// Background color
    pub background: Color,
    /// Text color
    pub text: Color,
    /// Error color
    pub error: Color,
    /// Warning color
    pub warning: Color,
    /// Success color
    pub success: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: Color::Blue,
            secondary: Color::Cyan,
            background: Color::Reset,
            text: Color::White,
            error: Color::Red,
            warning: Color::Yellow,
            success: Color::Green,
        }
    }
}

/// Main TUI entry point
pub async fn run_tui<B: Backend>(terminal: &mut Terminal<B>, config: TuiConfig) -> Result<()> {
    let mut app = TuiApp::new(config);
    app.run(terminal).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = TuiConfig::default();
        assert!(!config.fullscreen);
        assert!(!config.show_agent_panel);
    }

    #[test]
    fn test_theme_default() {
        let theme = Theme::default();
        assert_eq!(theme.primary, Color::Blue);
        assert_eq!(theme.error, Color::Red);
    }
}
