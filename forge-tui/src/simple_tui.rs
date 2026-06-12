//! Simple TUI implementation for real provider integration
//!
//! This provides a basic but functional TUI that works with real AI providers

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::TuiConfig;
use provider::ModelProvider;
use provider::types::{Message, MessageRole};

/// Simple TUI that works with real providers
pub struct SimpleTui {
    config: TuiConfig,
    provider: Arc<dyn ModelProvider>,
    conversation: Vec<String>,
    input: String,
    running: bool,
}

impl SimpleTui {
    /// Create new SimpleTui with provider
    pub fn new(config: TuiConfig, provider: Arc<dyn ModelProvider>) -> Self {
        Self {
            config,
            provider,
            conversation: Vec::new(),
            input: String::new(),
            running: true,
        }
    }

    /// Run the TUI
    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        execute!(
            stdout(),
            EnterAlternateScreen,
            EnableMouseCapture
        )?;

        let res = self.run_inner().await;

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;

        res
    }

    /// Inner run loop
    async fn run_inner(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        let mut tick_rate = tokio::time::interval(Duration::from_millis(16)); // ~60fps

        // Add welcome message
        self.add_system_message("Welcome to Forge TUI! Type your message and press Enter to send.");
        self.add_system_message("Press 'q' to quit, '?' for help.");

        while self.running {
            // Handle events
            tokio::select! {
                // Keyboard input
                _ = tick_rate.tick() => {
                    if event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key) = event::read()? {
                            self.handle_key_event(key).await;
                        }
                    }
                }
            }

            // Render
            terminal.draw(|f| self.render(f))?;
        }

        Ok(())
    }

    /// Handle keyboard events
    async fn handle_key_event(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.running = false;
            }
            KeyCode::Char('c') if key.modifiers.contains(event::KeyModifiers::CONTROL) => {
                self.running = false;
            }
            KeyCode::Char(c) => {
                self.input.push(c);
            }
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    self.send_message().await;
                    self.input.clear();
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char('?') => {
                self.show_help();
            }
            _ => {}
        }
    }

    /// Send message to provider
    async fn send_message(&mut self) {
        let user_message = self.input.clone();
        self.add_user_message(&user_message);

        // Create message for provider
        let messages = vec![
            Message {
                role: MessageRole::System,
                content: "You are Forge, a CLI coding agent. Help the user with software engineering tasks.".to_string(),
            },
            Message {
                role: MessageRole::User,
                content: user_message,
            },
        ];

        // Call provider
        match self.provider.chat(&messages).await {
            Ok(response) => {
                self.add_assistant_message(&response.content);
            }
            Err(e) => {
                self.add_system_message(&format!("Error: {}", e));
            }
        }
    }

    /// Add user message to conversation
    fn add_user_message(&mut self, text: &str) {
        self.conversation.push(format!("You: {}", text));
    }

    /// Add assistant message to conversation
    fn add_assistant_message(&mut self, text: &str) {
        self.conversation.push(format!("Forge: {}", text));
    }

    /// Add system message to conversation
    fn add_system_message(&mut self, text: &str) {
        self.conversation.push(format!("System: {}", text));
    }

    /// Show help
    fn show_help(&mut self) {
        self.add_system_message("Help:");
        self.add_system_message("  Type your message and press Enter to send");
        self.add_system_message("  q/Esc - Quit");
        self.add_system_message("  ? - Show this help");
    }

    /// Render the UI
    fn render(&self, f: &mut ratatui::Frame) {
        let size = f.size();

        // Main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10), // Conversation area
                Constraint::Length(3), // Input box
            ])
            .split(size);

        // Render conversation
        let conversation_lines: Vec<Line> = self.conversation
            .iter()
            .map(|msg| {
                let (color, text) = if msg.starts_with("You:") {
                    (Color::Cyan, msg.as_str())
                } else if msg.starts_with("Forge:") {
                    (Color::Green, msg.as_str())
                } else if msg.starts_with("System:") {
                    (Color::Yellow, msg.as_str())
                } else {
                    (Color::White, msg.as_str())
                };

                Line::from(vec![
                    Span::styled(text, Style::default().fg(color)),
                ])
            })
            .collect();

        // Show last N lines
        let visible_lines: Vec<Line> = conversation_lines
            .iter()
            .cloned()
            .rev()
            .take((chunks[0].height.saturating_sub(1) as usize))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let paragraph = Paragraph::new(visible_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
                    .title("Conversation"),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, chunks[0]);

        // Render input box
        let input_paragraph = Paragraph::new(self.input.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
                    .title("Input"),
            );

        f.render_widget(input_paragraph, chunks[1]);
    }
}

#[tokio::main]
async fn run_simple_tui(config: TuiConfig, provider: Arc<dyn ModelProvider>) -> Result<()> {
    let mut tui = SimpleTui::new(config, provider);
    tui.run().await
}