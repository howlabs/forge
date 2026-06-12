//! Conversation panel - streaming model output with markdown rendering
//!
//! This panel displays the conversation between user and Forge, with
//! support for streaming token-by-token output, markdown rendering,
//! and syntax highlighting.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

/// Conversation panel for streaming output
pub struct ConversationPanel {
    /// Content buffer
    content: Vec<Line<'static>>,
    /// Scroll offset
    scroll_offset: usize,
    /// Auto-scroll to bottom
    auto_scroll: bool,
}

impl ConversationPanel {
    /// Create new conversation panel
    pub fn new() -> Self {
        Self {
            content: Vec::new(),
            scroll_offset: 0,
            auto_scroll: true,
        }
    }

    /// Add streaming content (token-by-token)
    pub fn add_content(&mut self, text: String) {
        // Split into lines and add to content
        let lines: Vec<&str> = text.lines().collect();
        for line in lines {
            let line = Line::from(vec![
                Span::styled(line.to_string(), Style::default().fg(Color::White)),
            ]);
            self.content.push(line);
        }

        // Auto-scroll if enabled
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    /// Add a complete message (user or assistant)
    pub fn add_message(&mut self, role: MessageRole, text: &str) {
        let role_color = match role {
            MessageRole::User => Color::Cyan,
            MessageRole::Assistant => Color::Green,
            MessageRole::System => Color::Yellow,
        };

        // Add role prefix
        let role_line = Line::from(vec![
            Span::styled(
                format!("{}: ", role.as_str()),
                Style::default().fg(role_color).add_modifier(Modifier::BOLD),
            ),
        ]);
        self.content.push(role_line);

        // Add message content (with basic markdown support)
        for line in text.lines() {
            let styled_line = Self::render_markdown_line_static(line);
            self.content.push(styled_line);
        }

        // Blank line for spacing
        self.content.push(Line::from(""));

        // Auto-scroll
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    /// Render a single line with basic markdown support
    fn render_markdown_line_static(line: &str) -> Line<'static> {
        let mut spans = Vec::new();

        // Basic markdown patterns
        if line.starts_with("```") {
            // Code block
            spans.push(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
        } else if line.starts_with("# ") {
            // Header
            spans.push(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
        } else if line.starts_with("- ") || line.starts_with("* ") {
            // List item
            spans.push(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Green),
            ));
        } else if line.contains('`') {
            // Inline code (basic support)
            let parts: Vec<&str> = line.split('`').collect();
            for (i, part) in parts.iter().enumerate() {
                let style = if i % 2 == 1 {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::White)
                };
                spans.push(Span::styled(part.to_string(), style));
            }
        } else {
            // Regular text
            spans.push(Span::styled(line.to_string(), Style::default().fg(Color::White)));
        }

        Line::from(spans)
    }

    /// Scroll to bottom
    fn scroll_to_bottom(&mut self) {
        let content_len = self.content.len();
        self.scroll_offset = if content_len > 50 {
            content_len - 50
        } else {
            0
        };
    }

    /// Scroll up
    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
        self.auto_scroll = false;
    }

    /// Scroll down
    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
        // Check if we're at bottom
        let max_offset = self.content.len().saturating_sub(50);
        if self.scroll_offset >= max_offset {
            self.auto_scroll = true;
        }
    }

    /// Clear conversation
    pub fn clear(&mut self) {
        self.content = Vec::new();
        self.scroll_offset = 0;
    }

    /// Render the panel
    pub fn render(&self, f: &mut Frame, area: Rect) {
        let visible_lines: Vec<Line> = self.content
            .iter()
            .skip(self.scroll_offset)
            .take(area.height as usize)
            .cloned()
            .collect();

        let paragraph = Paragraph::new(visible_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
                    .title("Conversation"),
            )
            .wrap(Wrap { trim: false })
            .scroll((self.scroll_offset as u16, 0));

        f.render_widget(paragraph, area);
    }
}

/// Message role in conversation
#[derive(Debug, Clone, Copy)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    fn as_str(&self) -> &str {
        match self {
            MessageRole::User => "User",
            MessageRole::Assistant => "Forge",
            MessageRole::System => "System",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_creation() {
        let panel = ConversationPanel::new();
        assert_eq!(panel.scroll_offset, 0);
        assert!(panel.auto_scroll);
    }

    #[test]
    fn test_add_message() {
        let mut panel = ConversationPanel::new();
        panel.add_message(MessageRole::User, "Hello!");
        assert_eq!(panel.content.len(), 3); // role + content + blank
    }

    #[test]
    fn test_clear() {
        let mut panel = ConversationPanel::new();
        panel.add_message(MessageRole::User, "Test");
        panel.clear();
        assert_eq!(panel.content.len(), 0);
    }
}