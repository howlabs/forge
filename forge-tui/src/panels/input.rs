//! Input box - multiline input with history and command palette
//!
//! Provides keyboard-driven input with:
//! - Multiline editing
//! - Command history
//! - Slash commands (/model /context /agents /resume /diff /plan /review /init /help)
//! - Message queue for steering

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Input box for user messages
pub struct InputBox {
    /// Current input lines
    lines: Vec<String>,
    /// Cursor position (line, column)
    cursor: (usize, usize),
    /// Command history
    history: Vec<String>,
    /// History navigation index
    history_index: usize,
    /// Message queue for steering
    message_queue: Vec<String>,
    /// Plan mode (no edits allowed)
    plan_mode: bool,
    /// Command palette active
    command_palette: bool,
}

impl InputBox {
    /// Create new input box
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor: (0, 0),
            history: Vec::new(),
            history_index: 0,
            message_queue: Vec::new(),
            plan_mode: false,
            command_palette: false,
        }
    }

    /// Handle keyboard event
    pub fn handle_key_event(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c) => {
                self.insert_char(c);
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    // Shift+Enter = new line
                    self.new_line();
                } else {
                    // Enter = send message (or queue if agent busy)
                    self.send_or_queue_message();
                }
            }
            KeyCode::Backspace => {
                self.delete_char();
            }
            KeyCode::Delete => {
                self.delete_forward();
            }
            KeyCode::Left => {
                self.move_cursor_left();
            }
            KeyCode::Right => {
                self.move_cursor_right();
            }
            KeyCode::Up => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Up = history back
                    self.history_back();
                } else {
                    self.move_cursor_up();
                }
            }
            KeyCode::Down => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    // Ctrl+Down = history forward
                    self.history_forward();
                } else {
                    self.move_cursor_down();
                }
            }
            KeyCode::Char('/') => {
                // Start command palette if at beginning of line
                if self.get_current_line().trim().is_empty() {
                    self.command_palette = true;
                    self.insert_char('/');
                } else {
                    self.insert_char('/');
                }
            }
            KeyCode::Esc => {
                // Cancel command palette
                self.command_palette = false;
            }
            _ => {}
        }
    }

    /// Insert character at cursor position
    fn insert_char(&mut self, c: char) {
        let (line, col) = self.cursor;
        if line < self.lines.len() {
            self.lines[line].insert(col, c);
            self.move_cursor_right();
        }
    }

    /// Delete character before cursor
    fn delete_char(&mut self) {
        let (line, col) = self.cursor;
        if col > 0 {
            self.lines[line].remove(col - 1);
            self.move_cursor_left();
        } else if line > 0 {
            // Merge with previous line
            let current_line_content = self.lines[line].clone();
            let prev_line_len = self.lines[line - 1].len();
            self.lines[line - 1].push_str(&current_line_content);
            self.lines.remove(line);
            self.cursor = (line - 1, prev_line_len);
        }
    }

    /// Delete character after cursor
    fn delete_forward(&mut self) {
        let (line, col) = self.cursor;
        if col < self.lines[line].len() {
            self.lines[line].remove(col);
        } else if line < self.lines.len() - 1 {
            // Merge with next line
            let next_line_content = self.lines[line + 1].clone();
            self.lines[line].push_str(&next_line_content);
            self.lines.remove(line + 1);
        }
    }

    /// Move cursor left
    fn move_cursor_left(&mut self) {
        let (line, col) = self.cursor;
        if col > 0 {
            self.cursor = (line, col - 1);
        } else if line > 0 {
            let prev_line_len = self.lines[line - 1].len();
            self.cursor = (line - 1, prev_line_len);
        }
    }

    /// Move cursor right
    fn move_cursor_right(&mut self) {
        let (line, col) = self.cursor;
        if col < self.lines[line].len() {
            self.cursor = (line, col + 1);
        } else if line < self.lines.len() - 1 {
            self.cursor = (line + 1, 0);
        }
    }

    /// Move cursor up
    fn move_cursor_up(&mut self) {
        let (line, col) = self.cursor;
        if line > 0 {
            let prev_line_len = self.lines[line - 1].len();
            self.cursor = (line - 1, col.min(prev_line_len));
        }
    }

    /// Move cursor down
    fn move_cursor_down(&mut self) {
        let (line, col) = self.cursor;
        if line < self.lines.len() - 1 {
            let next_line_len = self.lines[line + 1].len();
            self.cursor = (line + 1, col.min(next_line_len));
        }
    }

    /// Insert new line
    fn new_line(&mut self) {
        let (line, col) = self.cursor;
        let current_line = &mut self.lines[line];
        let new_line = current_line.split_off(col);
        self.lines.insert(line + 1, new_line);
        self.cursor = (line + 1, 0);
    }

    /// Get current line
    fn get_current_line(&self) -> &str {
        let (line, _) = self.cursor;
        &self.lines[line]
    }

    /// Check if input has content
    pub fn has_content(&self) -> bool {
        self.lines.iter().any(|line| !line.trim().is_empty())
    }

    /// Get full input text
    pub fn get_text(&self) -> String {
        self.lines.join("\n")
    }

    /// Clear input
    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.cursor = (0, 0);
    }

    /// Send message or queue for later
    fn send_or_queue_message(&mut self) {
        let text = self.get_text();
        if !text.trim().is_empty() {
            // Check if this is a slash command
            if text.starts_with('/') {
                self.handle_slash_command(&text);
            } else {
                // Queue message (will be sent when agent is ready)
                self.message_queue.push(text.clone());
                self.history.push(text);
                self.history_index = self.history.len();
                self.clear();
            }
        }
    }

    /// Handle slash commands
    fn handle_slash_command(&mut self, command: &str) {
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return;
        }

        match parts[0] {
            "/model" => {
                // Change model: /model claude-3-5-sonnet
                if parts.len() > 1 {
                    // TODO: Send model change event
                    self.clear();
                }
            }
            "/context" => {
                // Manage context: /context add <path> or /context list
                // TODO: Handle context commands
                self.clear();
            }
            "/agents" => {
                // List or manage agents: /agents list
                // TODO: Handle agent commands
                self.clear();
            }
            "/resume" => {
                // Resume from checkpoint: /resume <task_id>
                if parts.len() > 1 {
                    // TODO: Send resume event
                    self.clear();
                }
            }
            "/diff" => {
                // Show diff viewer
                // TODO: Switch to diff panel
                self.clear();
            }
            "/plan" => {
                // Enter plan mode
                // TODO: Send plan mode event
                self.clear();
            }
            "/review" => {
                // Request code review
                // TODO: Send review request
                self.clear();
            }
            "/init" => {
                // Initialize project
                // TODO: Send init event
                self.clear();
            }
            "/help" => {
                // Show help
                // TODO: Show help panel
                self.clear();
            }
            _ => {
                // Unknown command - show in conversation
                // TODO: Send error message
                self.clear();
            }
        }
    }

    /// Queue message (for steering)
    pub fn queue_message(&mut self) {
        let text = self.get_text();
        if !text.trim().is_empty() {
            self.message_queue.push(text);
            self.clear();
        }
    }

    /// Get next queued message
    pub fn get_queued_message(&mut self) -> Option<String> {
        if self.message_queue.is_empty() {
            None
        } else {
            Some(self.message_queue.remove(0))
        }
    }

    /// History navigation
    fn history_back(&mut self) {
        if self.history_index > 0 {
            self.history_index -= 1;
            if let Some(text) = self.history.get(self.history_index) {
                self.lines = vec![text.clone()];
                self.cursor = (0, self.lines[0].len());
            }
        }
    }

    fn history_forward(&mut self) {
        if self.history_index < self.history.len() {
            self.history_index += 1;
            if self.history_index < self.history.len() {
                if let Some(text) = self.history.get(self.history_index) {
                    self.lines = vec![text.clone()];
                    self.cursor = (0, self.lines[0].len());
                }
            } else {
                self.clear();
            }
        }
    }

    /// Set plan mode
    pub fn set_plan_mode(&mut self, plan_mode: bool) {
        self.plan_mode = plan_mode;
    }

    /// Render the input box
    pub fn render(&self, f: &mut Frame, area: Rect, plan_mode: bool) {
        let input_text = self.get_text();

        let title = if plan_mode {
            "Input [Plan Mode]"
        } else {
            "Input"
        };

        let paragraph = Paragraph::new(input_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(if plan_mode {
                        Style::default().fg(Color::Yellow)
                    } else {
                        Style::default().fg(Color::Blue)
                    })
                    .title(title),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, area);

        // Show cursor
        let (_line, _col) = self.cursor;
        let _cursor_x = area.x + _col as u16 + 1; // +1 for border
        let _cursor_y = area.y + _line as u16 + 1; // +1 for border

        // Note: In real implementation, you'd use f.set_cursor here
        // but for now we'll skip cursor positioning
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_creation() {
        let input = InputBox::new();
        assert_eq!(input.lines.len(), 1);
        assert!(!input.has_content());
    }

    #[test]
    fn test_insert_char() {
        let mut input = InputBox::new();
        input.insert_char('a');
        input.insert_char('b');
        assert_eq!(input.get_text(), "ab");
    }

    #[test]
    fn test_new_line() {
        let mut input = InputBox::new();
        input.insert_char('a');
        input.insert_char('b');
        input.new_line();
        input.insert_char('c');
        assert_eq!(input.get_text(), "ab\nc");
    }

    #[test]
    fn test_clear() {
        let mut input = InputBox::new();
        input.insert_char('a');
        input.clear();
        assert!(!input.has_content());
    }
}