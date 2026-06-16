//! Simple TUI implementation for real provider integration
//!
//! This provides a basic but functional TUI that works with real AI providers

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyModifiers,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use crate::TuiConfig;
use provider::ModelProvider;

#[derive(Clone)]
pub enum ConversationEntry {
    User(String),
    Assistant(String),
    System(String),
    ToolCall { name: String, result: String },
    VerifyResult { passed: bool, logs: String },
}

#[derive(Debug)]
enum AgentUpdate {
    Done { steps: usize },
    Error(String),
}

/// Simple TUI that works with real providers
pub struct SimpleTui {
    _config: TuiConfig,
    provider: Arc<dyn ModelProvider>,
    conversation: Vec<ConversationEntry>,
    input: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    queued_messages: Vec<String>,
    scroll_offset: u16,
    show_help: bool,
    running: bool,
    agent_running: bool,
    agent_rx: Option<tokio::sync::mpsc::UnboundedReceiver<AgentUpdate>>,
    token_used: u32,
    token_budget: u32,
}

impl SimpleTui {
    /// Create new SimpleTui with provider
    pub fn new(config: TuiConfig, provider: Arc<dyn ModelProvider>) -> Self {
        Self {
            _config: config,
            provider,
            conversation: Vec::new(),
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            queued_messages: Vec::new(),
            scroll_offset: 0,
            show_help: false,
            running: true,
            agent_running: false,
            agent_rx: None,
            token_used: 0,
            token_budget: 200_000,
        }
    }

    /// Create new SimpleTui with provider-backed EventLoop integration.
    pub fn with_event_loop(config: TuiConfig, provider: Arc<dyn ModelProvider>) -> Self {
        Self::new(config, provider)
    }

    /// Run the TUI
    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;

        let res = self.run_inner().await;

        // Restore terminal
        disable_raw_mode()?;
        execute!(stdout(), LeaveAlternateScreen, DisableMouseCapture)?;

        res
    }

    /// Inner run loop
    async fn run_inner(&mut self) -> Result<()> {
        let backend = CrosstermBackend::new(stdout());
        let mut terminal = Terminal::new(backend)?;

        let mut tick_rate = tokio::time::interval(Duration::from_millis(16)); // ~60fps

        // Add welcome message
        self.add_entry(ConversationEntry::System(
            "Welcome to Forge TUI! Type your message and press Enter to send.".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "Press 'q' to quit, '?' for help.".to_string(),
        ));

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
                    self.poll_agent_updates().await;
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
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.running = false;
            }
            KeyCode::Char('q') | KeyCode::Esc if !self.has_draft() => {
                self.running = false;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
                if self.show_help {
                    self.show_help();
                }
            }
            KeyCode::Enter => {
                if !self.input.trim().is_empty() {
                    if self.agent_running {
                        self.queue_current_input();
                    } else {
                        self.send_message().await;
                    }
                }
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let previous = self.previous_cursor_boundary();
                    self.input.drain(previous..self.cursor);
                    self.cursor = previous;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.input.len() {
                    self.input.remove(self.cursor);
                }
            }
            KeyCode::Left => {
                self.cursor = self.previous_cursor_boundary();
            }
            KeyCode::Right => {
                self.cursor = self.next_cursor_boundary();
            }
            KeyCode::Home => {
                self.cursor = 0;
            }
            KeyCode::End => {
                self.cursor = self.input.len();
            }
            KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.history_back();
            }
            KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.history_forward();
            }
            KeyCode::PageUp => {
                self.scroll_offset = self.scroll_offset.saturating_add(5);
            }
            KeyCode::PageDown => {
                self.scroll_offset = self.scroll_offset.saturating_sub(5);
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor = 0;
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                self.history_index = None;
            }
            _ => {}
        }
    }

    /// Send message to provider
    async fn send_message(&mut self) {
        let user_message = self.input.clone();
        self.remember_history(user_message.clone());
        self.add_entry(ConversationEntry::User(user_message.clone()));
        self.input.clear();
        self.cursor = 0;
        self.start_agent_task(user_message);
    }

    fn start_agent_task(&mut self, task: String) {
        self.agent_running = true;
        self.add_entry(ConversationEntry::System(format!(
            "Starting task: {}",
            task
        )));

        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentUpdate>();
        let provider = self.provider.clone();
        let project_path = std::env::current_dir().unwrap_or_default();
        let task_clone = task.clone();
        let tx_clone = tx.clone();

        tokio::spawn(async move {
            let project_path = project_path.to_str().unwrap_or(".");
            let context = match context::ContextEngine::new(project_path) {
                Ok(context) => context,
                Err(e) => {
                    let _ = tx_clone.send(AgentUpdate::Error(e.to_string()));
                    return;
                }
            };
            let sandbox = match sandbox::Sandbox::new(project_path, "on") {
                Ok(sandbox) => sandbox,
                Err(e) => {
                    let _ = tx_clone.send(AgentUpdate::Error(e.to_string()));
                    return;
                }
            };

            let mut event_loop =
                forge_core::event_loop::EventLoop::new(provider, context, sandbox, task_clone);

            match event_loop.run().await {
                Ok(steps) => {
                    let _ = tx_clone.send(AgentUpdate::Done { steps });
                }
                Err(e) => {
                    let _ = tx_clone.send(AgentUpdate::Error(e.to_string()));
                }
            }
        });

        self.agent_rx = Some(rx);
    }

    async fn poll_agent_updates(&mut self) {
        let Some(rx) = self.agent_rx.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok(update) => {
                match update {
                    AgentUpdate::Done { steps } => self.add_entry(ConversationEntry::System(
                        format!("Task complete in {} steps", steps),
                    )),
                    AgentUpdate::Error(e) => {
                        self.add_entry(ConversationEntry::System(format!("Error: {}", e)))
                    }
                }
                self.agent_running = false;
                self.agent_rx = None;
                if let Some(next) = self.queued_messages.first().cloned() {
                    self.queued_messages.remove(0);
                    self.add_entry(ConversationEntry::System(format!(
                        "Running queued message: {}",
                        next
                    )));
                    self.start_agent_task(next);
                }
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                self.add_entry(ConversationEntry::System(
                    "Agent task stopped unexpectedly".to_string(),
                ));
                self.agent_running = false;
                self.agent_rx = None;
            }
        }
    }

    fn add_entry(&mut self, entry: ConversationEntry) {
        self.conversation.push(entry);
    }
    fn has_draft(&self) -> bool {
        !self.input.trim().is_empty()
    }
    fn previous_cursor_boundary(&self) -> usize {
        self.input[..self.cursor]
            .char_indices()
            .last()
            .map_or(0, |(idx, _)| idx)
    }

    fn next_cursor_boundary(&self) -> usize {
        self.input[self.cursor..]
            .char_indices()
            .nth(1)
            .map_or(self.input.len(), |(idx, _)| self.cursor + idx)
    }

    fn queue_current_input(&mut self) {
        let message = self.input.trim().to_string();
        if !message.is_empty() {
            self.remember_history(message.clone());
            self.queued_messages.push(message);
            self.input.clear();
            self.cursor = 0;
            self.add_entry(ConversationEntry::System(format!(
                "Message queued ({} pending).",
                self.queued_messages.len()
            )));
        }
    }

    fn remember_history(&mut self, message: String) {
        if self.history.last() != Some(&message) {
            self.history.push(message);
        }
        self.history_index = None;
    }

    fn history_back(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = self
            .history_index
            .map_or(self.history.len().saturating_sub(1), |i| {
                i.saturating_sub(1)
            });
        self.history_index = Some(next);
        self.input = self.history[next].clone();
        self.cursor = self.input.len();
    }

    fn history_forward(&mut self) {
        let Some(i) = self.history_index else {
            return;
        };
        if i + 1 >= self.history.len() {
            self.history_index = None;
            self.input.clear();
        } else {
            let next = i + 1;
            self.history_index = Some(next);
            self.input = self.history[next].clone();
        }
        self.cursor = self.input.len();
    }

    /// Show help
    fn show_help(&mut self) {
        self.add_entry(ConversationEntry::System("Help:".to_string()));
        self.add_entry(ConversationEntry::System(
            "  Type your message and press Enter to send; while busy, Enter queues it".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  Ctrl+↑/↓ - History, PageUp/PageDown - Scroll".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  Home/End/←/→ - Edit, Ctrl+U - Clear draft".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  q/Esc - Quit when input is empty".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  ? - Show this help".to_string(),
        ));
    }

    fn status_text(&self) -> String {
        if self.agent_running {
            format!(
                " [WORKING...] | queued: {} | Tokens: {}/{}",
                self.queued_messages.len(),
                self.token_used,
                self.token_budget
            )
        } else {
            format!(
                " Ready | queued: {} | Tokens: {}/{}",
                self.queued_messages.len(),
                self.token_used,
                self.token_budget
            )
        }
    }

    /// Render the UI
    fn render(&self, f: &mut ratatui::Frame) {
        let size = f.size();

        // Main layout
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(10),   // Conversation area
                Constraint::Length(3), // Input box
                Constraint::Length(1), // Status bar
            ])
            .split(size);

        // Render conversation
        let conversation_lines: Vec<Line> = self
            .conversation
            .iter()
            .map(|entry| {
                let (color, text) = match entry {
                    ConversationEntry::User(text) => (Color::Cyan, format!("You: {}", text)),
                    ConversationEntry::Assistant(text) => {
                        (Color::Green, format!("Forge: {}", text))
                    }
                    ConversationEntry::System(text) => (Color::Yellow, format!("System: {}", text)),
                    ConversationEntry::ToolCall { name, result } => {
                        (Color::Magenta, format!("[tool: {}] {}", name, result))
                    }
                    ConversationEntry::VerifyResult { passed, logs } => {
                        let color = if *passed { Color::Green } else { Color::Red };
                        (color, format!("[verify: {}] {}", passed, logs))
                    }
                };

                Line::from(vec![Span::styled(text, Style::default().fg(color))])
            })
            .collect();

        // Show last N lines
        let visible_lines: Vec<Line> = conversation_lines
            .iter()
            .cloned()
            .rev()
            .skip(self.scroll_offset as usize)
            .take(chunks[0].height.saturating_sub(1) as usize)
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
        let input_style = if self.agent_running {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::Blue)
        };
        let input_title = if self.agent_running {
            "Input (busy: Enter queues, Ctrl-C quits)"
        } else {
            "Input (Enter sends, ? help)"
        };
        let input_paragraph = Paragraph::new(self.input.as_str()).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(input_style)
                .title(input_title),
        );

        f.render_widget(input_paragraph, chunks[1]);
        if !self.agent_running {
            f.set_cursor(chunks[1].x + self.cursor as u16 + 1, chunks[1].y + 1);
        }

        let status_bar = Paragraph::new(self.status_text())
            .style(Style::default().bg(Color::DarkGray).fg(Color::White));
        f.render_widget(status_bar, chunks[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use crossterm::event::KeyModifiers;
    use provider::{ChatResponse, Message};

    struct MockProvider;

    #[async_trait]
    impl ModelProvider for MockProvider {
        async fn chat(&self, _messages: &[Message]) -> Result<ChatResponse> {
            Ok(ChatResponse {
                content: "done".to_string(),
                tool_calls: vec![],
            })
        }

        fn model(&self) -> &str {
            "mock"
        }
    }

    fn test_tui() -> SimpleTui {
        SimpleTui::new(TuiConfig::default(), Arc::new(MockProvider))
    }

    #[test]
    fn test_conversation_entry_types() {
        let entries = [
            ConversationEntry::User("u".to_string()),
            ConversationEntry::Assistant("a".to_string()),
            ConversationEntry::System("s".to_string()),
            ConversationEntry::ToolCall {
                name: "read_file".to_string(),
                result: "ok".to_string(),
            },
            ConversationEntry::VerifyResult {
                passed: true,
                logs: "ok".to_string(),
            },
        ];

        assert_eq!(entries.len(), 5);
    }

    #[tokio::test]
    async fn test_agent_running_allows_queueing_input() {
        let mut tui = test_tui();
        tui.agent_running = true;

        tui.handle_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::empty()))
            .await;
        assert_eq!(tui.input, "x");
        assert!(tui.running);

        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        assert!(tui.input.is_empty());
        assert_eq!(tui.queued_messages, vec!["x".to_string()]);

        tui.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL))
            .await;
        assert!(!tui.running);
    }

    #[test]
    fn test_status_bar_shows_working() {
        let mut tui = test_tui();
        assert!(tui.status_text().contains("Ready"));

        tui.agent_running = true;
        assert!(tui.status_text().contains("[WORKING...]"));
    }
}
