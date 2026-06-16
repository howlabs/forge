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
    style::{Color, Style, Modifier},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use std::io::stdout;
use std::sync::Arc;
use std::time::Duration;

use crate::TuiConfig;
use provider::ModelProvider;
use crate::panels::diff_viewer::{DiffHunk, HunkState};

#[derive(Clone)]
pub enum ConversationEntry {
    User(String),
    Assistant(String),
    System(String),
    ToolCall {
        name: String,
        result: String,
    },
    Diff {
        path: String,
        old_text: String,
        new_text: String,
    },
    VerifyResult {
        passed: bool,
        logs: String,
    },
}

/// Trim long tool output / diffs so a single event cannot flood the panel.
fn truncate_for_display(text: &str, max: usize) -> String {
    let trimmed = text.trim_end();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let kept: String = trimmed.chars().take(max).collect();
    format!("{kept}… (truncated)")
}

#[derive(Debug)]
enum AgentUpdate {
    /// A live progress event from the running event loop.
    Progress(forge_core::LoopEvent),
    Done {
        steps: usize,
    },
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Input,
    Diff,
    Conversation,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Dark,
    Light,
    Safe,
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

    // Redesigned fields
    focus: Focus,
    diff_hunks: Vec<DiffHunk>,
    selected_hunk: usize,
    active_agent_task: Option<String>,
    active_agent_status: String,
    tool_calls_count: u32,
    elapsed_seconds: u32,
    start_time: Option<std::time::Instant>,
    plan_mode: bool,
    checkpoint_available: Option<String>,
    theme_mode: ThemeMode,
    stream_queue: Vec<char>,
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
            focus: Focus::Input,
            diff_hunks: Vec::new(),
            selected_hunk: 0,
            active_agent_task: None,
            active_agent_status: "Idle".to_string(),
            tool_calls_count: 0,
            elapsed_seconds: 0,
            start_time: None,
            plan_mode: false,
            checkpoint_available: None,
            theme_mode: ThemeMode::Dark,
            stream_queue: Vec::new(),
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
            "Press Shift+Tab to toggle Plan Mode, Tab to cycle pane focus, 'q' to quit.".to_string(),
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
        // Global exit shortcut
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.running = false;
            return;
        }

        // Shift+Tab (BackTab) toggles Plan Mode globally (Kiro CLI style)
        if key.code == KeyCode::BackTab {
            self.plan_mode = !self.plan_mode;
            let status = if self.plan_mode { "enabled" } else { "disabled" };
            self.add_entry(ConversationEntry::System(format!("Plan mode {}", status)));
            return;
        }

        // Global resume shortcut
        if key.code == KeyCode::Char('R') && self.checkpoint_available.is_some() {
            if let Some(task_id) = self.checkpoint_available.clone() {
                self.checkpoint_available = None;
                self.add_entry(ConversationEntry::System(format!("Resuming task from checkpoint: {}", task_id)));
                self.start_agent_task(format!("Resume task {}", task_id));
            }
            return;
        }

        match self.focus {
            Focus::Input => {
                match key.code {
                    KeyCode::Esc => {
                        if !self.has_draft() {
                            self.running = false;
                        }
                    }
                    KeyCode::Char('q') => {
                        if !self.has_draft() {
                            self.running = false;
                        }
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
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        self.input.clear();
                        self.cursor = 0;
                    }
                    KeyCode::Tab => {
                        self.focus = if !self.diff_hunks.is_empty() {
                            Focus::Diff
                        } else {
                            Focus::Conversation
                        };
                    }
                    KeyCode::Char(c) => {
                        self.input.insert(self.cursor, c);
                        self.cursor += c.len_utf8();
                        self.history_index = None;
                    }
                    _ => {}
                }
            }
            Focus::Diff => {
                match key.code {
                    KeyCode::Esc => {
                        self.focus = Focus::Input;
                    }
                    KeyCode::Up => {
                        if self.selected_hunk > 0 {
                            self.selected_hunk -= 1;
                        }
                    }
                    KeyCode::Down => {
                        if self.selected_hunk < self.diff_hunks.len().saturating_sub(1) {
                            self.selected_hunk += 1;
                        }
                    }
                    KeyCode::Char('a') | KeyCode::Enter => {
                        if self.selected_hunk < self.diff_hunks.len() {
                            self.diff_hunks[self.selected_hunk].state = HunkState::Approved;
                        }
                    }
                    KeyCode::Char('r') => {
                        if self.selected_hunk < self.diff_hunks.len() {
                            self.diff_hunks[self.selected_hunk].state = HunkState::Rejected;
                        }
                    }
                    KeyCode::Char('A') => {
                        for hunk in &mut self.diff_hunks {
                            hunk.state = HunkState::Approved;
                        }
                    }
                    KeyCode::Char('R') => {
                        for hunk in &mut self.diff_hunks {
                            hunk.state = HunkState::Rejected;
                        }
                    }
                    KeyCode::Tab => {
                        self.focus = Focus::Conversation;
                    }
                    _ => {}
                }
            }
            Focus::Conversation => {
                match key.code {
                    KeyCode::Esc => {
                        self.focus = Focus::Input;
                    }
                    KeyCode::Up | KeyCode::PageUp => {
                        self.scroll_offset = self.scroll_offset.saturating_add(2);
                    }
                    KeyCode::Down | KeyCode::PageDown => {
                        self.scroll_offset = self.scroll_offset.saturating_sub(2);
                    }
                    KeyCode::Tab => {
                        self.focus = Focus::Input;
                    }
                    _ => {}
                }
            }
        }
    }

    /// Send message to provider
    async fn send_message(&mut self) {
        let user_message = self.input.clone();
        self.remember_history(user_message.clone());
        self.input.clear();
        self.cursor = 0;

        let trimmed = user_message.trim();
        if trimmed == "/plan" {
            self.plan_mode = !self.plan_mode;
            let status = if self.plan_mode { "enabled" } else { "disabled" };
            self.add_entry(ConversationEntry::System(format!("Plan mode {}", status)));
            return;
        }

        if trimmed == "/help" || trimmed == "/?" {
            self.show_help();
            return;
        }

        if trimmed.starts_with("/theme") {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() > 1 {
                match parts[1] {
                    "dark" => {
                        self.theme_mode = ThemeMode::Dark;
                        self.add_entry(ConversationEntry::System("Theme changed to dark".to_string()));
                    }
                    "light" => {
                        self.theme_mode = ThemeMode::Light;
                        self.add_entry(ConversationEntry::System("Theme changed to light".to_string()));
                    }
                    "safe" => {
                        self.theme_mode = ThemeMode::Safe;
                        self.add_entry(ConversationEntry::System("Theme changed to safe (high compatibility)".to_string()));
                    }
                    _ => {
                        self.add_entry(ConversationEntry::System("Unknown theme. Available: dark, light, safe".to_string()));
                    }
                }
            } else {
                self.add_entry(ConversationEntry::System("Usage: /theme <dark|light|safe>".to_string()));
            }
            return;
        }

        self.add_entry(ConversationEntry::User(user_message.clone()));
        self.start_agent_task(user_message);
    }

    fn start_agent_task(&mut self, task: String) {
        self.agent_running = true;
        self.active_agent_task = Some(task.clone());
        self.active_agent_status = "Running".to_string();
        self.tool_calls_count = 0;
        self.start_time = Some(std::time::Instant::now());
        self.elapsed_seconds = 0;

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
                forge_core::event_loop::EventLoop::new(provider, context, sandbox, task_clone)
                    .with_mcp_server();

            // Forward live loop events into the TUI update channel so the
            // conversation panel can show tool calls, output, and diffs as
            // they happen rather than only a final Done/Error.
            let (loop_tx, mut loop_rx) =
                tokio::sync::mpsc::unbounded_channel::<forge_core::LoopEvent>();
            event_loop = event_loop.with_observer(loop_tx);

            let forward_tx = tx_clone.clone();
            let forwarder = tokio::spawn(async move {
                while let Some(event) = loop_rx.recv().await {
                    if forward_tx.send(AgentUpdate::Progress(event)).is_err() {
                        break;
                    }
                }
            });

            match event_loop.run().await {
                Ok(steps) => {
                    let _ = tx_clone.send(AgentUpdate::Done { steps });
                }
                Err(e) => {
                    let _ = tx_clone.send(AgentUpdate::Error(e.to_string()));
                }
            }
            // Drop the loop sender (held by event_loop) by ending scope, then
            // let the forwarder drain any remaining events.
            drop(event_loop);
            let _ = forwarder.await;
        });

        self.agent_rx = Some(rx);
    }

    async fn poll_agent_updates(&mut self) {
        if let Some(start) = self.start_time {
            self.elapsed_seconds = start.elapsed().as_secs() as u32;
        }

        // Drain character chunks from stream_queue to simulate streaming
        if !self.stream_queue.is_empty() {
            let take_count = self.stream_queue.len().min(8);
            let chunk: String = self.stream_queue.drain(0..take_count).collect();
            if let Some(ConversationEntry::Assistant(ref mut text)) = self.conversation.last_mut() {
                text.push_str(&chunk);
            } else {
                self.conversation.push(ConversationEntry::Assistant(chunk));
            }
        }

        // Drain everything currently queued so fast tool/diff bursts show up
        // within one frame instead of one-per-tick.
        loop {
            let Some(rx) = self.agent_rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(AgentUpdate::Progress(event)) => self.handle_loop_event(event),
                Ok(AgentUpdate::Done { steps }) => {
                    self.add_entry(ConversationEntry::System(format!(
                        "Task complete in {} steps",
                        steps
                    )));
                    self.finish_agent_task();
                    return;
                }
                Ok(AgentUpdate::Error(e)) => {
                    self.add_entry(ConversationEntry::System(format!("Error: {}", e)));
                    self.finish_agent_task();
                    return;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return,
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.add_entry(ConversationEntry::System(
                        "Agent task stopped unexpectedly".to_string(),
                    ));
                    self.agent_running = false;
                    self.agent_rx = None;
                    return;
                }
            }
        }
    }

    /// Render a live event from the running event loop into the conversation.
    fn handle_loop_event(&mut self, event: forge_core::LoopEvent) {
        use forge_core::LoopEvent;
        match event {
            LoopEvent::AssistantMessage { content, .. } => {
                self.conversation.push(ConversationEntry::Assistant(String::new()));
                self.stream_queue.extend(content.chars());
            }
            LoopEvent::ToolStarted { name } => {
                self.tool_calls_count += 1;
                self.active_agent_status = format!("Running tool: {}", name);
                self.add_entry(ConversationEntry::ToolCall {
                    name,
                    result: "running...".to_string(),
                });
            }
            LoopEvent::ToolCompleted {
                name,
                result,
                is_error,
            } => {
                self.active_agent_status = "Running".to_string();
                let result_display = truncate_for_display(&result, 800);
                let prefix = if is_error { "error: " } else { "" };
                self.add_entry(ConversationEntry::ToolCall {
                    name,
                    result: format!("{prefix}{result_display}"),
                });
            }
            LoopEvent::DiffApplied {
                path,
                old_text,
                new_text,
            } => {
                let hunks = self.compute_diff(&path, &old_text, &new_text);
                self.diff_hunks.extend(hunks);
                self.focus = Focus::Diff;

                self.add_entry(ConversationEntry::Diff {
                    path,
                    old_text: truncate_for_display(&old_text, 400),
                    new_text: truncate_for_display(&new_text, 400),
                });
            }
            LoopEvent::VerifyResult { passed, logs } => {
                self.active_agent_status = if passed { "Done (verified)".to_string() } else { "Failed verification".to_string() };
                self.add_entry(ConversationEntry::VerifyResult {
                    passed,
                    logs: truncate_for_display(&logs, 800),
                });
            }
        }
    }

    /// Finish the current agent task and start the next queued message, if any.
    fn finish_agent_task(&mut self) {
        self.agent_running = false;
        self.active_agent_status = "Idle".to_string();
        self.start_time = None;
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
            "  Tab - Cycle active pane focus (Input, Diff, Chat)".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  Shift+Tab - Toggle Plan Mode globally".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  /theme <dark|light|safe> - Customize TUI colors".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  Ctrl+↑/↓ - History, PageUp/PageDown - Scroll Conversation".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  Home/End/←/→ - Edit input, Ctrl+U - Clear draft".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  q/Esc - Quit when input is empty".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  /plan - Toggle Plan/Build mode".to_string(),
        ));
        self.add_entry(ConversationEntry::System(
            "  ? - Show this help".to_string(),
        ));
    }

    #[allow(dead_code)]
    fn status_text(&self) -> String {
        let mode_str = if self.plan_mode { "PLAN" } else { "BUILD" };
        let focus_str = match self.focus {
            Focus::Input => "INPUT",
            Focus::Diff => "DIFF",
            Focus::Conversation => "CHAT",
        };
        if self.agent_running {
            format!(
                " Mode: {} | Focus: {} | [WORKING...] | queued: {} | Tokens: {}/{}",
                mode_str,
                focus_str,
                self.queued_messages.len(),
                self.token_used,
                self.token_budget
            )
        } else {
            format!(
                " Mode: {} | Focus: {} | Ready | queued: {} | Tokens: {}/{}",
                mode_str,
                focus_str,
                self.queued_messages.len(),
                self.token_used,
                self.token_budget
            )
        }
    }

    fn theme_bg(&self) -> Color {
        match self.theme_mode {
            ThemeMode::Dark => Color::Reset,
            ThemeMode::Light => Color::White,
            ThemeMode::Safe => Color::Reset,
        }
    }

    fn theme_fg(&self) -> Color {
        match self.theme_mode {
            ThemeMode::Dark => Color::White,
            ThemeMode::Light => Color::Black,
            ThemeMode::Safe => Color::Reset,
        }
    }

    fn theme_border(&self, active: bool) -> Color {
        if active {
            match self.theme_mode {
                ThemeMode::Dark => Color::Cyan,
                ThemeMode::Light => Color::Blue,
                ThemeMode::Safe => Color::White,
            }
        } else {
            match self.theme_mode {
                ThemeMode::Dark => Color::DarkGray,
                ThemeMode::Light => Color::Gray,
                ThemeMode::Safe => Color::Reset,
            }
        }
    }

    fn compute_diff(&self, file_path: &str, old: &str, new: &str) -> Vec<DiffHunk> {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = new.lines().collect();

        let mut hunks = Vec::new();
        let mut current_hunk = DiffHunk {
            file_path: file_path.to_string(),
            header: String::new(),
            removals: Vec::new(),
            additions: Vec::new(),
            state: HunkState::Pending,
        };

        let max_lines = old_lines.len().max(new_lines.len());
        let mut in_hunk = false;

        for i in 0..max_lines {
            let old_line = old_lines.get(i).copied().unwrap_or("");
            let new_line = new_lines.get(i).copied().unwrap_or("");

            if old_line != new_line {
                if !in_hunk {
                    in_hunk = true;
                    current_hunk = DiffHunk {
                        file_path: file_path.to_string(),
                        header: format!("@@ -{},+{} @@", i + 1, i + 1),
                        removals: Vec::new(),
                        additions: Vec::new(),
                        state: HunkState::Pending,
                    };
                }

                if !old_line.is_empty() {
                    current_hunk.removals.push(old_line.to_string());
                }
                if !new_line.is_empty() {
                    current_hunk.additions.push(new_line.to_string());
                }
            } else if in_hunk {
                in_hunk = false;
                hunks.push(current_hunk.clone());
            }
        }

        if in_hunk {
            hunks.push(current_hunk);
        }

        hunks
    }

    /// Render the UI
    fn render(&self, f: &mut ratatui::Frame) {
        let size = f.size();
        let default_style = Style::default().bg(self.theme_bg()).fg(self.theme_fg());

        // Main layout with banner (hidden/visible), content, status
        let banner_height = if self.checkpoint_available.is_some() { 3 } else { 0 };
        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(banner_height), // Checkpoint banner
                Constraint::Min(0),                // Main content
                Constraint::Length(1),             // Status bar
            ])
            .split(size);

        // Render Checkpoint banner if available
        if let Some(task_id) = &self.checkpoint_available {
            let banner_text = format!(
                " ⚠️  CHECKPOINT DETECTED: Task '{}' crashed. Press 'R' to resume! ",
                task_id
            );
            let banner_paragraph = Paragraph::new(banner_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title("Checkpoint Recovery"),
                )
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
            f.render_widget(banner_paragraph, main_chunks[0]);
        }

        // Main content area: Left (70%) and Right (30% for Agent Activity)
        let show_panel = self._config.show_agent_panel;
        let content_chunks = if show_panel {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(70), // Left side
                    Constraint::Percentage(30), // Right side
                ])
                .split(main_chunks[1])
        } else {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Percentage(100),
                    Constraint::Percentage(0),
                ])
                .split(main_chunks[1])
        };

        // Left side layout: Conversation, Diff (dynamic height), Input
        let diff_height = if self.diff_hunks.is_empty() { 0 } else { 8 };
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),                 // Conversation
                Constraint::Length(diff_height),    // Diff viewer
                Constraint::Length(3),              // Input box
            ])
            .split(content_chunks[0]);

        // 1. Render Conversation history
        let conversation_lines: Vec<Line> = self
            .conversation
            .iter()
            .map(|entry| {
                let (color, text) = match entry {
                    ConversationEntry::User(text) => (Color::Cyan, format!("You: {}", text)),
                    ConversationEntry::Assistant(text) => (Color::Green, format!("Forge: {}", text)),
                    ConversationEntry::System(text) => (Color::Yellow, format!("System: {}", text)),
                    ConversationEntry::ToolCall { name, result } => {
                        (Color::Magenta, format!("[tool: {}] {}", name, result))
                    }
                    ConversationEntry::Diff { path, old_text, new_text } => {
                        (Color::Yellow, format!("[diff: {}]\n- {}\n+ {}", path, old_text, new_text))
                    }
                    ConversationEntry::VerifyResult { passed, logs } => {
                        let prefix = if *passed { "[✓ Passed]" } else { "[✗ Failed]" };
                        let color = if *passed { Color::Green } else { Color::Red };
                        (color, format!("{} {}", prefix, logs))
                    }
                };
                Line::from(vec![Span::styled(text, Style::default().fg(color))])
            })
            .collect();

        let visible_lines: Vec<Line> = conversation_lines
            .iter()
            .cloned()
            .rev()
            .skip(self.scroll_offset as usize)
            .take(left_chunks[0].height.saturating_sub(2) as usize)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let conv_border_style = Style::default().fg(self.theme_border(self.focus == Focus::Conversation));
        let conv_title = if self.focus == Focus::Conversation {
            "Conversation [Focus: Esc/Tab to switch]"
        } else {
            "Conversation"
        };
        let conv_paragraph = Paragraph::new(visible_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(conv_border_style)
                    .title(conv_title),
            )
            .style(default_style)
            .wrap(Wrap { trim: false });
        f.render_widget(conv_paragraph, left_chunks[0]);

        // 2. Render Diff Viewer (if not empty)
        if !self.diff_hunks.is_empty() {
            let mut diff_lines = Vec::new();
            for (i, hunk) in self.diff_hunks.iter().enumerate() {
                let is_selected = i == self.selected_hunk && self.focus == Focus::Diff;
                let header_style = if is_selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };

                let state_span = match hunk.state {
                    HunkState::Pending => Span::styled(" [?]", Style::default().fg(Color::Yellow)),
                    HunkState::Approved => Span::styled(" [✓]", Style::default().fg(Color::Green)),
                    HunkState::Rejected => Span::styled(" [✗]", Style::default().fg(Color::Red)),
                    HunkState::Modified => Span::styled(" [~]", Style::default().fg(Color::Cyan)),
                };

                diff_lines.push(Line::from(vec![
                    Span::styled(&hunk.file_path, Style::default().fg(self.theme_fg())),
                    Span::raw(": "),
                    Span::styled(&hunk.header, header_style),
                    state_span,
                ]));

                for removal in hunk.removals.iter().take(2) {
                    diff_lines.push(Line::from(vec![
                        Span::styled("  - ", Style::default().fg(Color::Red)),
                        Span::styled(removal, Style::default().fg(Color::Red)),
                    ]));
                }
                if hunk.removals.len() > 2 {
                    diff_lines.push(Line::from(vec![Span::styled(
                        format!("  ... ({} more removals)", hunk.removals.len() - 2),
                        Style::default().fg(Color::DarkGray),
                    )]));
                }

                for addition in hunk.additions.iter().take(2) {
                    diff_lines.push(Line::from(vec![
                        Span::styled("  + ", Style::default().fg(Color::Green)),
                        Span::styled(addition, Style::default().fg(Color::Green)),
                    ]));
                }
                if hunk.additions.len() > 2 {
                    diff_lines.push(Line::from(vec![Span::styled(
                        format!("  ... ({} more additions)", hunk.additions.len() - 2),
                        Style::default().fg(Color::DarkGray),
                    )]));
                }
            }

            let diff_border_style = Style::default().fg(self.theme_border(self.focus == Focus::Diff));
            let diff_title = if self.focus == Focus::Diff {
                "Diff Viewer [↑↓: select, Enter/a: approve, r: reject, Esc: back]"
            } else {
                "Diff Viewer"
            };
            let diff_paragraph = Paragraph::new(diff_lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(diff_border_style)
                        .title(diff_title),
                )
                .style(default_style)
                .wrap(Wrap { trim: false });
            f.render_widget(diff_paragraph, left_chunks[1]);
        }

        // 3. Render Input Box
        let input_border_style = if self.focus == Focus::Input {
            if self.plan_mode {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(self.theme_border(true))
            }
        } else {
            Style::default().fg(self.theme_border(false))
        };
        let input_title = if self.plan_mode {
            "Input [Plan Mode] (Enter queues)"
        } else if self.agent_running {
            "Input (busy: Enter queues, Ctrl-C quits)"
        } else if self.focus == Focus::Input {
            "Input [Enter: send, Tab: navigate]"
        } else {
            "Input"
        };
        let input_paragraph = Paragraph::new(self.input.as_str())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(input_border_style)
                    .title(input_title),
            )
            .style(default_style);
        f.render_widget(input_paragraph, left_chunks[2]);
        if self.focus == Focus::Input && !self.agent_running {
            f.set_cursor(left_chunks[2].x + self.cursor as u16 + 1, left_chunks[2].y + 1);
        }

        // 4. Render Agent Activity Panel (if enabled)
        if show_panel {
            let mut agent_lines = Vec::new();
            agent_lines.push(Line::from(vec![Span::styled(
                "Parallel Agents (1 active)",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            )]));
            agent_lines.push(Line::from(""));

            if let Some(task) = &self.active_agent_task {
                let status_icon = if self.agent_running { "●" } else { "○" };
                let status_color = if self.agent_running { Color::Blue } else { Color::Green };
                agent_lines.push(Line::from(vec![
                    Span::styled(status_icon, Style::default().fg(status_color)),
                    Span::raw(" "),
                    Span::styled("main", Style::default().fg(self.theme_fg())),
                    Span::raw(": "),
                    Span::styled(task, Style::default().fg(Color::Gray)),
                ]));

                if self.agent_running {
                    let progress_bar = "████████░░░░░░░░░░░░";
                    agent_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(progress_bar, Style::default().fg(status_color)),
                        Span::styled(" 40%", Style::default().fg(status_color)),
                    ]));
                }

                agent_lines.push(Line::from(""));
                agent_lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("Tool calls: {} | Elapsed: {}s", self.tool_calls_count, self.elapsed_seconds),
                        Style::default().fg(Color::Gray),
                    ),
                ]));
                agent_lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("Status: {}", self.active_agent_status),
                        Style::default().fg(Color::Yellow),
                    ),
                ]));
            } else {
                agent_lines.push(Line::from(vec![Span::styled(
                    "No active agent task",
                    Style::default().fg(Color::Gray),
                )]));
            }

            let agent_paragraph = Paragraph::new(agent_lines)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(self.theme_border(false)))
                        .title("Agent Activity"),
                )
                .style(default_style)
                .wrap(Wrap { trim: false });
            f.render_widget(agent_paragraph, content_chunks[1]);
        }

        // 5. Render Status Bar
        let mode_str = if self.plan_mode { "PLAN" } else { "BUILD" };
        let mode_color = if self.plan_mode { Color::Yellow } else { Color::Green };
        let focus_str = match self.focus {
            Focus::Input => "INPUT",
            Focus::Diff => "DIFF",
            Focus::Conversation => "CHAT",
        };
        let status_color = if self.agent_running { Color::Yellow } else { Color::Green };
        let status_str = if self.agent_running { "WORKING" } else { "READY" };

        let line = Line::from(vec![
            Span::styled(" Mode: ", Style::default().fg(self.theme_fg())),
            Span::styled(mode_str, Style::default().fg(mode_color).add_modifier(Modifier::BOLD)),
            Span::raw(" │ Focus: "),
            Span::styled(focus_str, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::raw(" │ Status: "),
            Span::styled(status_str, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            Span::raw(" │ Queued: "),
            Span::styled(self.queued_messages.len().to_string(), Style::default().fg(self.theme_fg())),
            Span::raw(" │ Tokens: "),
            Span::styled(
                format!("{}/{}", self.token_used, self.token_budget),
                Style::default().fg(Color::Green),
            ),
        ]);
        let status_bar = Paragraph::new(line)
            .style(Style::default().bg(Color::DarkGray).fg(Color::White));
        f.render_widget(status_bar, main_chunks[2]);
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

    #[test]
    fn test_handle_loop_event_renders_tool_and_diff() {
        use forge_core::LoopEvent;
        let mut tui = test_tui();

        tui.handle_loop_event(LoopEvent::ToolStarted {
            name: "run_command".to_string(),
        });
        tui.handle_loop_event(LoopEvent::ToolCompleted {
            name: "run_command".to_string(),
            result: "ok".to_string(),
            is_error: false,
        });
        tui.handle_loop_event(LoopEvent::DiffApplied {
            path: "src/lib.rs".to_string(),
            old_text: "a".to_string(),
            new_text: "b".to_string(),
        });
        tui.handle_loop_event(LoopEvent::VerifyResult {
            passed: true,
            logs: "all green".to_string(),
        });

        // started + completed + diff + verify = 4 entries
        assert_eq!(tui.conversation.len(), 4);
        assert!(matches!(
            tui.conversation[2],
            ConversationEntry::Diff { .. }
        ));
        assert!(matches!(
            tui.conversation[3],
            ConversationEntry::VerifyResult { passed: true, .. }
        ));
    }

    #[test]
    fn test_truncate_for_display() {
        assert_eq!(truncate_for_display("short", 100), "short");
        let long = "x".repeat(50);
        let out = truncate_for_display(&long, 10);
        assert!(out.contains("truncated"));
        assert!(out.chars().count() < long.len());
    }

    #[tokio::test]
    async fn test_streaming_simulation() {
        let mut tui = test_tui();
        use forge_core::LoopEvent;
        tui.handle_loop_event(LoopEvent::AssistantMessage {
            step: 0,
            content: "hello world".to_string(),
        });
        
        assert_eq!(tui.conversation.len(), 1);
        if let ConversationEntry::Assistant(ref text) = tui.conversation[0] {
            assert!(text.is_empty());
        } else {
            panic!("Expected Assistant entry");
        }
        
        tui.poll_agent_updates().await;
        if let ConversationEntry::Assistant(ref text) = tui.conversation[0] {
            assert_eq!(text, "hello wo");
        } else {
            panic!("Expected Assistant entry");
        }

        tui.poll_agent_updates().await;
        if let ConversationEntry::Assistant(ref text) = tui.conversation[0] {
            assert_eq!(text, "hello world");
        } else {
            panic!("Expected Assistant entry");
        }
    }
}
