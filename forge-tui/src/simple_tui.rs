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
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::TuiConfig;
use provider::ModelProvider;
use crate::panels::diff_viewer::{DiffHunk, HunkState};
use crate::command_palette::SlashCommand;

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

pub struct PendingApproval {
    pub tool_name: String,
    pub details: String,
    pub tx: tokio::sync::oneshot::Sender<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectPopupField {
    Provider,
    Model,
    ApiKey,
}

pub struct ConnectPopupState {
    pub active: bool,
    pub providers: Vec<String>,
    pub selected_provider_idx: usize,
    pub models: Vec<String>,
    pub selected_model_idx: usize,
    pub api_key: String,
    pub active_field: ConnectPopupField,
}

enum AgentUpdate {
    /// A live progress event from the running event loop.
    Progress(forge_core::LoopEvent),
    ApprovalRequired {
        tool_name: String,
        details: String,
        tx: tokio::sync::oneshot::Sender<bool>,
    },
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenCodeConfig {
    pub provider: Option<HashMap<String, OpenCodeProvider>>,
    pub model: Option<String>,
    #[serde(rename = "small_model")]
    pub small_model: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenCodeProvider {
    pub options: Option<OpenCodeProviderOptions>,
    pub models: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenCodeProviderOptions {
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "baseURL")]
    pub base_url: Option<String>,
}

fn strip_jsonc_comments(jsonc: &str) -> String {
    let mut clean = String::new();
    for line in jsonc.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        clean.push_str(line);
        clean.push('\n');
    }
    clean
}

fn load_opencode_config() -> Option<OpenCodeConfig> {
    let home = std::env::var("HOME").ok()?;
    let config_paths = vec![
        std::path::PathBuf::from(&home).join(".config").join("opencode").join("opencode.json"),
        std::path::PathBuf::from(&home).join(".config").join("opencode").join("opencode.jsonc"),
    ];

    for path in config_paths {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(path) {
                let clean_content = strip_jsonc_comments(&content);
                if let Ok(config) = serde_json::from_str::<OpenCodeConfig>(&clean_content) {
                    return Some(config);
                }
            }
        }
    }
    None
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
    auto_resume_task: Option<String>,
    has_exact_tokens: bool,
    pending_approval: Option<PendingApproval>,
    theme_mode: ThemeMode,
    stream_queue: Vec<char>,
    autocomplete_options: Vec<String>,
    autocomplete_index: usize,
    did_stream_chunk: bool,
    connect_popup: ConnectPopupState,
    show_agent_panel: bool,
    opencode_config: Option<OpenCodeConfig>,
}

fn resolve_api_key(provider_name: &str, explicit: Option<String>) -> String {
    if let Some(key) = explicit.filter(|k| !k.is_empty()) {
        return key;
    }
    let lower = provider_name.to_lowercase();
    if lower == "mock" || lower == "local" {
        return String::new();
    }
    
    let env_var = match lower.as_str() {
        "openai" => "OPENAI_API_KEY",
        "zai" => "ZAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        _ => "FORGE_API_KEY",
    };
    
    std::env::var(env_var).unwrap_or_default()
}

impl SimpleTui {
    /// Create new SimpleTui with provider
    pub fn new(config: TuiConfig, provider: Arc<dyn ModelProvider>) -> Self {
        let opencode_config = load_opencode_config();
        
        let mut providers = vec![
            "anthropic".to_string(),
            "gemini".to_string(),
            "mock".to_string(),
        ];
        for p in provider::PROVIDERS {
            providers.push(p.name.to_string());
        }
        
        // Add providers from opencode.json
        if let Some(config) = &opencode_config {
            if let Some(prov_map) = &config.provider {
                for name in prov_map.keys() {
                    providers.push(name.clone());
                }
            }
        }
        providers.sort();
        providers.dedup();

        let mut tui = Self {
            _config: config.clone(),
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
            auto_resume_task: None,
            has_exact_tokens: false,
            pending_approval: None,
            theme_mode: ThemeMode::Dark,
            stream_queue: Vec::new(),
            autocomplete_options: Vec::new(),
            autocomplete_index: 0,
            did_stream_chunk: false,
            connect_popup: ConnectPopupState {
                active: false,
                providers,
                selected_provider_idx: 0,
                models: Vec::new(),
                selected_model_idx: 0,
                api_key: String::new(),
                active_field: ConnectPopupField::Provider,
            },
            show_agent_panel: config.show_agent_panel,
            opencode_config,
        };

        tui.update_connect_popup_models();
        tui
    }

    /// Create new SimpleTui with provider-backed EventLoop integration.
    pub fn with_event_loop(config: TuiConfig, provider: Arc<dyn ModelProvider>) -> Self {
        Self::new(config, provider)
    }

    /// Set initial task to auto-resume on startup
    pub fn with_auto_resume(mut self, task_id: Option<String>) -> Self {
        self.auto_resume_task = task_id;
        self
    }

    fn resolve_api_key_with_config(&self, provider_name: &str, explicit: Option<String>) -> String {
        if let Some(key) = explicit.filter(|k| !k.is_empty()) {
            return key;
        }
        
        let lower = provider_name.to_lowercase();
        
        // 1. Try opencode config options.apiKey
        if let Some(config) = &self.opencode_config {
            if let Some(prov_map) = &config.provider {
                if let Some(prov_cfg) = prov_map.get(&lower) {
                    if let Some(options) = &prov_cfg.options {
                        if let Some(api_key) = &options.api_key {
                            if !api_key.is_empty() {
                                return api_key.clone();
                            }
                        }
                    }
                }
            }
        }

        // 2. Fallback to standard environment variables
        resolve_api_key(&lower, None)
    }

    fn create_tui_provider(&self, provider_name: &str, model: &str, api_key: &str) -> anyhow::Result<Arc<dyn ModelProvider>> {
        let lower = provider_name.to_lowercase();
        
        // 1. Check if the provider is defined in opencode.json config
        if let Some(cfg_prov) = self.opencode_config.as_ref()
            .and_then(|c| c.provider.as_ref())
            .and_then(|p| p.get(&lower)) 
        {
            let options_key = cfg_prov.options.as_ref().and_then(|o| o.api_key.clone());
            let options_url = cfg_prov.options.as_ref().and_then(|o| o.base_url.clone());
            
            let resolved_key = if api_key.is_empty() {
                options_key.unwrap_or_default()
            } else {
                api_key.to_string()
            };

            // Try to get base URL: either from config options, or fallback from a known static provider matching the name
            let resolved_url = options_url.or_else(|| {
                let static_name = if lower.contains("zai") {
                    "zai"
                } else if lower.contains("openai") {
                    "openai"
                } else if lower.contains("openrouter") {
                    "openrouter"
                } else {
                    &lower
                };
                provider::find_provider(static_name).map(|entry| entry.base_url.to_string())
            });

            if let Some(base_url) = resolved_url {
                let provider = provider::OpenAIProvider::with_base_url(model, resolved_key, base_url);
                return Ok(Arc::new(provider));
            }
        }

        // 2. Fallback to standard provider creation
        provider::create_provider(&lower, model, api_key)
    }

    fn update_connect_popup_models(&mut self) {
        let provider = &self.connect_popup.providers[self.connect_popup.selected_provider_idx];
        let mut models = Vec::new();

        // 1. Try to get models from opencode.json config
        if let Some(config) = &self.opencode_config {
            if let Some(prov_map) = &config.provider {
                if let Some(prov_cfg) = prov_map.get(provider) {
                    if let Some(models_map) = &prov_cfg.models {
                        for m in models_map.keys() {
                            models.push(m.clone());
                        }
                    }
                }
            }
        }

        // 2. Fall back to static MODEL_CATALOG
        if models.is_empty() {
            models = provider::MODEL_CATALOG.iter()
                .filter(|m| m.provider == provider)
                .map(|m| m.model.to_string())
                .collect();
        }
            
        if models.is_empty() {
            models.push("default".to_string());
        }

        models.sort();
        models.dedup();
        
        self.connect_popup.models = models;
        self.connect_popup.selected_model_idx = 0;
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

        // Auto-resume task if requested
        if let Some(task_id) = self.auto_resume_task.take() {
            self.add_entry(ConversationEntry::System(format!("Resuming task from checkpoint: {}", task_id)));
            self.start_agent_task(format!("Resume task {}", task_id));
        }

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
        // Handle connect popup key interception
        if self.connect_popup.active {
            match key.code {
                KeyCode::Esc => {
                    self.connect_popup.active = false;
                    self.add_entry(ConversationEntry::System("Interactive connection cancelled.".to_string()));
                }
                KeyCode::Tab => {
                    self.connect_popup.active_field = match self.connect_popup.active_field {
                        ConnectPopupField::Provider => ConnectPopupField::Model,
                        ConnectPopupField::Model => ConnectPopupField::ApiKey,
                        ConnectPopupField::ApiKey => ConnectPopupField::Provider,
                    };
                }
                KeyCode::BackTab => {
                    self.connect_popup.active_field = match self.connect_popup.active_field {
                        ConnectPopupField::Provider => ConnectPopupField::ApiKey,
                        ConnectPopupField::Model => ConnectPopupField::Provider,
                        ConnectPopupField::ApiKey => ConnectPopupField::Model,
                    };
                }
                KeyCode::Up => {
                    match self.connect_popup.active_field {
                        ConnectPopupField::Provider => {
                            if self.connect_popup.selected_provider_idx > 0 {
                                self.connect_popup.selected_provider_idx -= 1;
                                self.update_connect_popup_models();
                                let p = &self.connect_popup.providers[self.connect_popup.selected_provider_idx];
                                self.connect_popup.api_key = self.resolve_api_key_with_config(p, None);
                            }
                        }
                        ConnectPopupField::Model => {
                            if self.connect_popup.selected_model_idx > 0 {
                                self.connect_popup.selected_model_idx -= 1;
                            }
                        }
                        ConnectPopupField::ApiKey => {}
                    }
                }
                KeyCode::Down => {
                    match self.connect_popup.active_field {
                        ConnectPopupField::Provider => {
                            if self.connect_popup.selected_provider_idx + 1 < self.connect_popup.providers.len() {
                                self.connect_popup.selected_provider_idx += 1;
                                self.update_connect_popup_models();
                                let p = &self.connect_popup.providers[self.connect_popup.selected_provider_idx];
                                self.connect_popup.api_key = self.resolve_api_key_with_config(p, None);
                            }
                        }
                        ConnectPopupField::Model => {
                            if self.connect_popup.selected_model_idx + 1 < self.connect_popup.models.len() {
                                self.connect_popup.selected_model_idx += 1;
                            }
                        }
                        ConnectPopupField::ApiKey => {}
                    }
                }
                KeyCode::Char(c) => {
                    if self.connect_popup.active_field == ConnectPopupField::ApiKey {
                        self.connect_popup.api_key.push(c);
                    }
                }
                KeyCode::Backspace => {
                    if self.connect_popup.active_field == ConnectPopupField::ApiKey {
                        self.connect_popup.api_key.pop();
                    }
                }
                KeyCode::Enter => {
                    let provider = self.connect_popup.providers[self.connect_popup.selected_provider_idx].clone();
                    let model = self.connect_popup.models.get(self.connect_popup.selected_model_idx)
                        .cloned()
                        .unwrap_or_else(|| "default".to_string());
                    let key = self.connect_popup.api_key.clone();
                    
                    self.connect_popup.active = false;
                    
                    let key_resolved = if key.is_empty() {
                        self.resolve_api_key_with_config(&provider, None)
                    } else {
                        key
                    };
                    
                    if key_resolved.is_empty() && provider != "mock" && provider != "local" {
                        self.add_entry(ConversationEntry::System(format!(
                            "Error: No API key resolved for provider '{}'.", provider
                        )));
                    } else {
                        match self.create_tui_provider(&provider, &model, &key_resolved) {
                            Ok(new_provider) => {
                                self.provider = new_provider;
                                self.add_entry(ConversationEntry::System(format!(
                                    "Successfully connected to provider '{}' using model '{}'!",
                                    provider, model
                                )));
                            }
                            Err(e) => {
                                self.add_entry(ConversationEntry::System(format!(
                                    "Failed to connect to provider '{}': {}", provider, e
                                )));
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // Handle pending interactive approval first
        if let Some(pending) = self.pending_approval.take() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let _ = pending.tx.send(true);
                    self.add_entry(ConversationEntry::System(format!("Tool '{}' approved.", pending.tool_name)));
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    let _ = pending.tx.send(false);
                    self.add_entry(ConversationEntry::System(format!("Tool '{}' rejected.", pending.tool_name)));
                }
                _ => {
                    // Put it back if any other key is pressed
                    self.pending_approval = Some(pending);
                }
            }
            return;
        }

        // Global exit shortcut
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.running = false;
            return;
        }

        // Ctrl+p toggles Agent Activity Panel (Yoga layout toggle)
        if key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.show_agent_panel = !self.show_agent_panel;
            let state = if self.show_agent_panel { "shown" } else { "hidden" };
            self.add_entry(ConversationEntry::System(format!("Agent activity panel is now {}", state)));
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
                        let trimmed = self.input.trim();
                        let is_valid_complete_command = if trimmed.starts_with('/') {
                            SlashCommand::parse(trimmed).is_some()
                        } else {
                            false
                        };

                        if is_valid_complete_command {
                            self.autocomplete_options.clear();
                            self.autocomplete_index = 0;
                            if !self.input.trim().is_empty() {
                                if self.agent_running && !self.is_local_command(&self.input) {
                                    self.queue_current_input();
                                } else {
                                    self.send_message().await;
                                }
                            }
                        } else if !self.autocomplete_options.is_empty() {
                            let option = self.autocomplete_options[self.autocomplete_index].clone();
                            let prev_input = self.input.clone();
                            self.input = option.clone();
                            self.cursor = self.input.len();
                            self.update_autocomplete_options();
                            
                            if (!option.ends_with(' ') && !option.ends_with('/')) || prev_input == option {
                                self.autocomplete_options.clear();
                                self.autocomplete_index = 0;
                                if !self.input.trim().is_empty() {
                                    if self.agent_running && !self.is_local_command(&self.input) {
                                        self.queue_current_input();
                                    } else {
                                        self.send_message().await;
                                    }
                                }
                            }
                        } else if !self.input.trim().is_empty() {
                            if self.agent_running && !self.is_local_command(&self.input) {
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
                            self.update_autocomplete_options();
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
                    KeyCode::Up if !self.autocomplete_options.is_empty() => {
                        if self.autocomplete_index > 0 {
                            self.autocomplete_index -= 1;
                        } else {
                            self.autocomplete_index = self.autocomplete_options.len().saturating_sub(1);
                        }
                    }
                    KeyCode::Down if !self.autocomplete_options.is_empty() => {
                        self.autocomplete_index = (self.autocomplete_index + 1) % self.autocomplete_options.len();
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
                        self.update_autocomplete_options();
                    }
                    KeyCode::Tab => {
                        if !self.autocomplete_options.is_empty() {
                            self.input = self.autocomplete_options[self.autocomplete_index].clone();
                            self.cursor = self.input.len();
                            self.update_autocomplete_options();
                        } else {
                            self.focus = if !self.diff_hunks.is_empty() {
                                Focus::Diff
                            } else {
                                Focus::Conversation
                            };
                        }
                    }
                    KeyCode::Char(c) => {
                        self.input.insert(self.cursor, c);
                        self.cursor += c.len_utf8();
                        self.history_index = None;
                        self.update_autocomplete_options();
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
        self.autocomplete_options.clear();
        self.autocomplete_index = 0;

        let trimmed = user_message.trim();
        if trimmed.starts_with('/') {
            if let Some(cmd) = SlashCommand::parse(trimmed) {
                match cmd {
                    SlashCommand::Plan => {
                        self.plan_mode = !self.plan_mode;
                        let status = if self.plan_mode { "enabled" } else { "disabled" };
                        self.add_entry(ConversationEntry::System(format!("Plan mode {}", status)));
                    }
                    SlashCommand::Help => {
                        self.show_help();
                    }
                    SlashCommand::Theme { theme } => {
                        match theme.as_str() {
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
                    }
                    SlashCommand::Model { model } => {
                        if let Some(info) = provider::MODEL_CATALOG.iter().find(|m| m.model == model) {
                            let key = self.resolve_api_key_with_config(info.provider, None);
                            if key.is_empty() && info.provider != "mock" && info.provider != "local" {
                                self.add_entry(ConversationEntry::System(format!(
                                    "Error: API key not found for provider '{}'. Please set the environment variable.",
                                    info.provider
                                )));
                            } else {
                                match self.create_tui_provider(info.provider, info.model, &key) {
                                    Ok(new_provider) => {
                                        self.provider = new_provider;
                                        self.add_entry(ConversationEntry::System(format!(
                                            "Model switched to {} (provider: {})",
                                            info.model, info.provider
                                        )));
                                    }
                                    Err(e) => {
                                        self.add_entry(ConversationEntry::System(format!(
                                            "Error changing model: {}",
                                            e
                                        )));
                                    }
                                }
                            }
                        } else {
                            let current_model = self.provider.model();
                            let current_provider = provider::MODEL_CATALOG.iter()
                                .find(|m| m.model == current_model)
                                .map(|m| m.provider)
                                .unwrap_or("openai");
                                
                            let key = self.resolve_api_key_with_config(current_provider, None);
                            match self.create_tui_provider(current_provider, &model, &key) {
                                Ok(new_provider) => {
                                    self.provider = new_provider;
                                    self.add_entry(ConversationEntry::System(format!(
                                        "Model switched to custom model {} on provider {}",
                                        model, current_provider
                                    )));
                                }
                                Err(_) => {
                                    match provider::create_provider(current_provider, &model, &key) {
                                        Ok(new_provider) => {
                                            self.provider = new_provider;
                                            self.add_entry(ConversationEntry::System(format!(
                                                "Model switched to custom model {} on provider {}",
                                                model, current_provider
                                            )));
                                        }
                                        Err(e) => {
                                            self.add_entry(ConversationEntry::System(format!(
                                                "Error changing model: {}",
                                                e
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    SlashCommand::Connect { provider, model, api_key } => {
                        if provider.is_empty() {
                            self.connect_popup.active = true;
                            self.connect_popup.active_field = ConnectPopupField::Provider;
                            self.connect_popup.selected_provider_idx = 0;
                            self.connect_popup.selected_model_idx = 0;
                            self.update_connect_popup_models();
                            let default_provider = &self.connect_popup.providers[0];
                            self.connect_popup.api_key = self.resolve_api_key_with_config(default_provider, None);
                            self.add_entry(ConversationEntry::System(
                                "Opened interactive Connect Selector. Press [Tab] to switch fields, [Up/Down] to select, [Enter] to connect, [Esc] to close.".to_string()
                            ));
                        } else {
                            let (prov_name, model_opt) = if let Some(pos) = provider.find('/') {
                                let p = provider[..pos].to_string();
                                let m = provider[pos + 1..].to_string();
                                (p, Some(m))
                            } else {
                                (provider.clone(), model.clone())
                            };

                            let provider_lower = prov_name.to_lowercase();
                            let resolved_model = model_opt.unwrap_or_else(|| {
                                let mut default_m = None;
                                if let Some(config) = &self.opencode_config {
                                    if let Some(prov_map) = &config.provider {
                                        if let Some(prov_cfg) = prov_map.get(&provider_lower) {
                                            if let Some(models_map) = &prov_cfg.models {
                                                default_m = models_map.keys().next().cloned();
                                            }
                                        }
                                    }
                                }
                                
                                default_m.unwrap_or_else(|| {
                                    provider::default_model(&provider_lower)
                                        .map(|m| m.model.to_string())
                                        .unwrap_or_else(|| "default".to_string())
                                })
                            });
                            
                            let key = self.resolve_api_key_with_config(&provider_lower, api_key);
                            if key.is_empty() && provider_lower != "mock" && provider_lower != "local" {
                                self.add_entry(ConversationEntry::System(format!(
                                    "Error: No API key resolved for provider '{}'. Please pass it or set the environment variable.",
                                    prov_name
                                )));
                            } else {
                                match self.create_tui_provider(&provider_lower, &resolved_model, &key) {
                                    Ok(new_provider) => {
                                        self.provider = new_provider;
                                        self.add_entry(ConversationEntry::System(format!(
                                            "Successfully connected to provider '{}' using model '{}'!",
                                            prov_name, resolved_model
                                        )));
                                    }
                                    Err(e) => {
                                        self.add_entry(ConversationEntry::System(format!(
                                            "Failed to connect to provider '{}': {}",
                                            prov_name, e
                                        )));
                                    }
                                }
                            }
                        }
                    }
                    SlashCommand::Resume { task_id } => {
                        self.add_entry(ConversationEntry::System(format!("Resuming task from checkpoint: {}", task_id)));
                        self.start_agent_task(format!("Resume task {}", task_id));
                    }
                    SlashCommand::Diff { .. } => {
                        if !self.diff_hunks.is_empty() {
                            self.focus = Focus::Diff;
                        } else {
                            self.add_entry(ConversationEntry::System("No active diff to view".to_string()));
                        }
                    }
                    SlashCommand::Context { action, path } => {
                        let path_str = path.unwrap_or_else(|| "current directory".to_string());
                        self.add_entry(ConversationEntry::System(format!("Context action '{}' on '{}'", action, path_str)));
                    }
                    SlashCommand::Agents { action, agent_id } => {
                        if action == "toggle" {
                            self.show_agent_panel = !self.show_agent_panel;
                            let state = if self.show_agent_panel { "shown" } else { "hidden" };
                            self.add_entry(ConversationEntry::System(format!("Agent activity panel is now {}", state)));
                        } else {
                            let id_str = agent_id.unwrap_or_else(|| "all".to_string());
                            self.add_entry(ConversationEntry::System(format!("Agent action '{}' on '{}'", action, id_str)));
                        }
                    }
                    _ => {}
                }
            } else {
                self.add_entry(ConversationEntry::System(format!("Unknown command: {}", trimmed)));
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
        self.token_used = 3000 + (task.len() / 4) as u32;
        self.has_exact_tokens = false;

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

            struct TuiApprovalHandler {
                tx: tokio::sync::mpsc::UnboundedSender<AgentUpdate>,
            }
            #[async_trait::async_trait]
            impl forge_core::event_loop::ApprovalHandler for TuiApprovalHandler {
                async fn approve_tool(&self, name: &str, details: &str) -> bool {
                    let (resp_tx, resp_rx) = tokio::sync::oneshot::channel::<bool>();
                    let _ = self.tx.send(AgentUpdate::ApprovalRequired {
                        tool_name: name.to_string(),
                        details: details.to_string(),
                        tx: resp_tx,
                    });
                    resp_rx.await.unwrap_or(false)
                }
            }

            let approval_handler = std::sync::Arc::new(TuiApprovalHandler {
                tx: tx_clone.clone(),
            });

            let mut event_loop =
                forge_core::event_loop::EventLoop::new(provider, context, sandbox, task_clone)
                    .with_mcp_server()
                    .with_approval_handler(approval_handler)
                    .with_tool_policy(forge_core::event_loop::ToolPolicy {
                        allow_writes: false,
                        allow_commands: false,
                    });

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
        let mut streamed = false;
        if !self.stream_queue.is_empty() {
            let take_count = self.stream_queue.len().min(8);
            let chunk: String = self.stream_queue.drain(0..take_count).collect();
            if let Some(ConversationEntry::Assistant(ref mut text)) = self.conversation.last_mut() {
                text.push_str(&chunk);
            } else {
                self.conversation.push(ConversationEntry::Assistant(chunk));
            }
            self.scroll_offset = 0;
            streamed = true;
        }

        // Drain everything currently queued so fast tool/diff bursts show up
        // within one frame instead of one-per-tick.
        let mut received = false;
        loop {
            let Some(rx) = self.agent_rx.as_mut() else {
                if streamed {
                    self.update_token_count();
                }
                return;
            };
            match rx.try_recv() {
                Ok(AgentUpdate::Progress(event)) => {
                    self.handle_loop_event(event);
                    received = true;
                }
                Ok(AgentUpdate::ApprovalRequired { tool_name, details, tx }) => {
                    self.pending_approval = Some(PendingApproval {
                        tool_name,
                        details,
                        tx,
                    });
                    self.focus = Focus::Input;
                    return;
                }
                Ok(AgentUpdate::Done { steps }) => {
                    self.add_entry(ConversationEntry::System(format!(
                        "Task complete in {} steps",
                        steps
                    )));
                    self.finish_agent_task();
                    self.update_token_count();
                    return;
                }
                Ok(AgentUpdate::Error(e)) => {
                    self.add_entry(ConversationEntry::System(format!("Error: {}", e)));
                    self.finish_agent_task();
                    self.update_token_count();
                    return;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    if streamed || received {
                        self.update_token_count();
                    }
                    return;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    self.add_entry(ConversationEntry::System(
                        "Agent task stopped unexpectedly".to_string(),
                    ));
                    self.agent_running = false;
                    self.agent_rx = None;
                    self.update_token_count();
                    return;
                }
            }
        }
    }

    /// Render a live event from the running event loop into the conversation.
    fn handle_loop_event(&mut self, event: forge_core::LoopEvent) {
        use forge_core::LoopEvent;
        match event {
            LoopEvent::AssistantMessageChunk { delta } => {
                if !self.did_stream_chunk {
                    self.did_stream_chunk = true;
                    self.conversation.push(ConversationEntry::Assistant(String::new()));
                }
                self.stream_queue.extend(delta.chars());
            }
            LoopEvent::AssistantMessage { content, .. } => {
                if self.did_stream_chunk {
                    self.did_stream_chunk = false;
                } else {
                    self.conversation.push(ConversationEntry::Assistant(String::new()));
                    self.stream_queue.extend(content.chars());
                }
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
            LoopEvent::TokensUsed { total, .. } => {
                self.token_used = total;
                self.has_exact_tokens = true;
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
        self.scroll_offset = 0;
    }

    fn has_draft(&self) -> bool {
        !self.input.trim().is_empty()
    }

    fn is_local_command(&self, input: &str) -> bool {
        let trimmed = input.trim();
        if trimmed.starts_with('/') {
            if let Some(cmd) = SlashCommand::parse(trimmed) {
                matches!(
                    cmd,
                    SlashCommand::Plan
                        | SlashCommand::Help
                        | SlashCommand::Theme { .. }
                        | SlashCommand::Model { .. }
                        | SlashCommand::Diff { .. }
                        | SlashCommand::Connect { .. }
                        | SlashCommand::Agents { .. }
                )
            } else {
                false
            }
        } else {
            false
        }
    }

    fn update_token_count(&mut self) {
        if self.has_exact_tokens {
            return;
        }
        let mut conv_chars = 0;
        for entry in &self.conversation {
            match entry {
                ConversationEntry::User(t) | ConversationEntry::Assistant(t) | ConversationEntry::System(t) => {
                    conv_chars += t.len();
                }
                ConversationEntry::ToolCall { name, result } => {
                    conv_chars += name.len() + result.len();
                }
                ConversationEntry::Diff { path, old_text, new_text } => {
                    conv_chars += path.len() + old_text.len() + new_text.len();
                }
                ConversationEntry::VerifyResult { logs, .. } => {
                    conv_chars += logs.len();
                }
            }
        }
        // Base system prompt and tools definitions take ~3000 tokens
        self.token_used = 3000 + (conv_chars / 4) as u32;
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
        let left_pct = if self.show_agent_panel { 70 } else { 100 };
        let right_pct = if self.show_agent_panel { 30 } else { 0 };
        let content_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(right_pct),
            ])
            .split(main_chunks[1]);

        // Left side layout: Conversation, Diff (dynamic height), Input
        let diff_height = if self.diff_hunks.is_empty() {
            0
        } else if self.focus == Focus::Diff {
            12
        } else {
            6
        };
        let left_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(5),                 // Conversation
                Constraint::Length(diff_height),    // Diff viewer
                Constraint::Length(3),              // Input box
            ])
            .split(content_chunks[0]);

        // 1. Render Conversation history
        let mut conversation_lines: Vec<Line> = Vec::new();
        let mut in_code_block = false;

        for entry in &self.conversation {
            let (label_span, text_color) = match entry {
                ConversationEntry::User(_) => (
                    Span::styled(" 👤 YOU ", Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    Color::Cyan
                ),
                ConversationEntry::Assistant(_) => (
                    Span::styled(" 🤖 FORGE ", Style::default().bg(Color::Green).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    Color::Green
                ),
                ConversationEntry::System(_) => (
                    Span::styled(" ⚙️ SYSTEM ", Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    Color::Yellow
                ),
                ConversationEntry::ToolCall { name, .. } => (
                    Span::styled(format!(" 🛠️ TOOL: {} ", name), Style::default().bg(Color::Magenta).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    Color::Magenta
                ),
                ConversationEntry::Diff { path, .. } => (
                    Span::styled(format!(" 📁 DIFF: {} ", path), Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    Color::Yellow
                ),
                ConversationEntry::VerifyResult { passed, .. } => {
                    if *passed {
                        (
                            Span::styled(" 🧪 VERIFIED ", Style::default().bg(Color::Green).fg(Color::Black).add_modifier(Modifier::BOLD)),
                            Color::Green
                        )
                    } else {
                        (
                            Span::styled(" 🧪 FAILED ", Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD)),
                            Color::Red
                        )
                    }
                }
            };

            let text_content = match entry {
                ConversationEntry::User(text) => text.as_str(),
                ConversationEntry::Assistant(text) => text.as_str(),
                ConversationEntry::System(text) => text.as_str(),
                ConversationEntry::ToolCall { result, .. } => result.as_str(),
                ConversationEntry::Diff { old_text, new_text, .. } => "",
                ConversationEntry::VerifyResult { logs, .. } => logs.as_str(),
            };

            let mut lines = Vec::new();
            if let ConversationEntry::Diff { old_text, new_text, .. } = entry {
                lines.push(format!("- {}", old_text));
                lines.push(format!("+ {}", new_text));
            } else {
                for line in text_content.lines() {
                    lines.push(line.to_string());
                }
            }

            let mut is_first = true;
            for line in lines {
                let trimmed = line.trim();
                if trimmed.starts_with("```") {
                    in_code_block = !in_code_block;
                    if in_code_block {
                        conversation_lines.push(Line::from(vec![
                            Span::styled("    ┌── Code Block ────────────────────────┐", Style::default().fg(Color::DarkGray))
                        ]));
                    } else {
                        conversation_lines.push(Line::from(vec![
                            Span::styled("    └── End of Code ───────────────────────┘", Style::default().fg(Color::DarkGray))
                        ]));
                    }
                    continue;
                }

                if in_code_block {
                    conversation_lines.push(Line::from(vec![
                        Span::styled("    │ ", Style::default().fg(Color::DarkGray)),
                        Span::styled(line, Style::default().fg(Color::White))
                    ]));
                } else {
                    if is_first {
                        conversation_lines.push(Line::from(vec![
                            label_span.clone(),
                            Span::raw(" "),
                            Span::styled(line, Style::default().fg(text_color))
                        ]));
                        is_first = false;
                    } else {
                        conversation_lines.push(Line::from(vec![
                            Span::raw("     │ "),
                            Span::styled(line, Style::default().fg(text_color))
                        ]));
                    }
                }
            }
            conversation_lines.push(Line::from(""));
        }

        let chat_height = left_chunks[0].height.saturating_sub(2) as usize;
        let max_scroll = conversation_lines.len().saturating_sub(chat_height);
        let actual_scroll = (self.scroll_offset as usize).min(max_scroll);

        let visible_lines: Vec<Line> = conversation_lines
            .iter()
            .cloned()
            .rev()
            .skip(actual_scroll)
            .take(chat_height)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        let conv_border_style = Style::default().fg(self.theme_border(self.focus == Focus::Conversation));
        let conv_title = if self.focus == Focus::Conversation {
            " 💬 Conversation [Focus: Esc/Tab to switch] "
        } else {
            " 💬 Conversation "
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
                    HunkState::Pending => Span::styled(" ⏳ PENDING ", Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    HunkState::Approved => Span::styled(" ✓ APPROVED ", Style::default().bg(Color::Green).fg(Color::Black).add_modifier(Modifier::BOLD)),
                    HunkState::Rejected => Span::styled(" ✗ REJECTED ", Style::default().bg(Color::Red).fg(Color::White).add_modifier(Modifier::BOLD)),
                    HunkState::Modified => Span::styled(" ~ MODIFIED ", Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)),
                };

                diff_lines.push(Line::from(vec![
                    Span::styled(&hunk.file_path, Style::default().fg(self.theme_fg())),
                    Span::raw(": "),
                    Span::styled(&hunk.header, header_style),
                    Span::raw(" "),
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
                " 🔍 Diff Viewer [↑↓: select, Enter/a: approve, r: reject, Esc: back] "
            } else {
                " 🔍 Diff Viewer "
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
        if let Some(pending) = &self.pending_approval {
            let details_display = truncate_for_display(&pending.details, size.width.saturating_sub(65) as usize);
            let prompt_text = format!(
                " Agent wants to run: {} ({})? Press [y] to Approve, [n] to Reject",
                pending.tool_name,
                details_display
            );
            let prompt_paragraph = Paragraph::new(prompt_text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
                        .title("⚠️  Interactive Tool Approval Required"),
                )
                .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
            f.render_widget(prompt_paragraph, left_chunks[2]);
        } else {
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
                " ⌨️ Input [Plan Mode] (Enter queues) "
            } else if self.agent_running {
                " ⌨️ Input (busy: Enter queues, Ctrl-C quits) "
            } else if self.focus == Focus::Input {
                " ⌨️ Input [Enter: send, Tab: navigate] "
            } else {
                " ⌨️ Input "
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
            if self.focus == Focus::Input && !self.agent_running && !self.connect_popup.active {
                f.set_cursor(left_chunks[2].x + self.cursor as u16 + 1, left_chunks[2].y + 1);
            }
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
        let status_str = if self.agent_running {
            format!("WORKING ({})", self.active_agent_status)
        } else {
            "READY".to_string()
        };

        let mut status_spans = vec![
            Span::styled(" ⚡ MODE: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", mode_str), Style::default().bg(mode_color).fg(Color::Black).add_modifier(Modifier::BOLD)),
            Span::styled(" │ 🎯 FOCUS: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", focus_str), Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)),
            Span::styled(" │ 🟢 STATUS: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", status_str), Style::default().bg(status_color).fg(Color::Black).add_modifier(Modifier::BOLD)),
        ];

        if self.agent_running {
            status_spans.push(Span::styled(" │ ⏳ TIME: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
            status_spans.push(Span::styled(format!(" {}s ", self.elapsed_seconds), Style::default().bg(Color::Yellow).fg(Color::Black).add_modifier(Modifier::BOLD)));
            status_spans.push(Span::styled(" │ 🛠️ TOOLS: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)));
            status_spans.push(Span::styled(format!(" {} ", self.tool_calls_count), Style::default().bg(Color::Magenta).fg(Color::Black).add_modifier(Modifier::BOLD)));
        }

        let ratio = if self.token_budget > 0 {
            self.token_used as f32 / self.token_budget as f32
        } else {
            0.0
        };
        let token_color = if ratio < 0.5 {
            Color::Green
        } else if ratio < 0.8 {
            Color::Yellow
        } else {
            Color::Red
        };

        status_spans.extend(vec![
            Span::styled(" │ 📥 QUEUED: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" {} ", self.queued_messages.len()), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" │ 🪙 TOKENS: ", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" {}/{} ", self.token_used, self.token_budget),
                Style::default().fg(token_color).add_modifier(Modifier::BOLD),
            ),
        ]);

        let line = Line::from(status_spans);
        let status_bar = Paragraph::new(line)
            .style(Style::default().bg(Color::DarkGray).fg(Color::White));
        f.render_widget(status_bar, main_chunks[2]);

        self.render_autocomplete_popup(f, left_chunks[2]);
        self.render_connect_popup(f);
        if self.show_agent_panel {
            self.render_agent_panel(f, content_chunks[1]);
        }
    }

    fn render_agent_panel(&self, f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        let mut lines = Vec::new();

        let spinner = if self.agent_running {
            match self.elapsed_seconds % 10 {
                0 => "⠋",
                1 => "⠙",
                2 => "⠹",
                3 => "⠸",
                4 => "⠼",
                5 => "⠴",
                6 => "⠦",
                7 => "⠧",
                8 => "⠇",
                _ => "⠏",
            }
        } else {
            "○"
        };

        // Header
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", spinner), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled("AGENT ENGINE STATUS", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(" ────────────────────────────────────────", Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));

        // Active task section
        if self.agent_running {
            lines.push(Line::from(vec![
                Span::styled(" ⚡ RUNNING ", Style::default().bg(Color::Blue).fg(Color::White).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
                Span::styled("Active Steer Task", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]));
            lines.push(Line::from(""));
            
            if let Some(task) = &self.active_agent_task {
                let display_task = truncate_for_display(task, area.width.saturating_sub(12) as usize);
                lines.push(Line::from(vec![
                    Span::styled("    Task: ", Style::default().fg(Color::DarkGray)),
                    Span::styled(display_task, Style::default().fg(Color::White)),
                ]));
            }

            lines.push(Line::from(vec![
                Span::styled("  Status: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&self.active_agent_status, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ]));

            lines.push(Line::from(vec![
                Span::styled(" Elapsed: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}s", self.elapsed_seconds), Style::default().fg(Color::Gray)),
            ]));

            lines.push(Line::from(vec![
                Span::styled("   Tools: ", Style::default().fg(Color::DarkGray)),
                Span::styled(self.tool_calls_count.to_string(), Style::default().fg(Color::Magenta)),
            ]));
            
            lines.push(Line::from(""));

            // Progress bar
            let progress_width = area.width.saturating_sub(6) as usize;
            if progress_width > 0 {
                let ratio = if self.token_budget > 0 {
                    (self.token_used as f32 / self.token_budget as f32).min(1.0)
                } else {
                    0.0
                };
                let filled = (ratio * progress_width as f32) as usize;
                let empty = progress_width.saturating_sub(filled);
                
                let progress_bar_content = format!(
                    "{}{}",
                    "█".repeat(filled),
                    "░".repeat(empty)
                );
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(progress_bar_content, Style::default().fg(Color::Blue)),
                ]));
            }
        } else {
            lines.push(Line::from(vec![
                Span::styled(" ○ IDLE ", Style::default().bg(Color::DarkGray).fg(Color::White)),
                Span::raw(" "),
                Span::styled("Waiting for input", Style::default().fg(Color::DarkGray)),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  No active agent tasks running.", Style::default().fg(Color::DarkGray)),
            ]));
        }

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" ────────────────────────────────────────", Style::default().fg(Color::DarkGray)),
        ]));
        lines.push(Line::from(""));

        // Queued tasks section
        lines.push(Line::from(vec![
            Span::styled(" 📋 QUEUE ", Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("Pending Task List", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" [{}]", self.queued_messages.len()), Style::default().fg(Color::Yellow)),
        ]));
        lines.push(Line::from(""));

        if self.queued_messages.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("  No tasks in queue.", Style::default().fg(Color::DarkGray)),
            ]));
        } else {
            for (idx, task) in self.queued_messages.iter().enumerate() {
                let display_task = truncate_for_display(task, area.width.saturating_sub(8) as usize);
                lines.push(Line::from(vec![
                    Span::styled(format!("  {}. ", idx + 1), Style::default().fg(Color::Yellow)),
                    Span::styled(display_task, Style::default().fg(Color::Gray)),
                ]));
            }
        }

        // Add shortcut legend at the bottom of the panel
        let available_height = area.height.saturating_sub(lines.len() as u16 + 2);
        if available_height > 1 {
            for _ in 0..available_height.saturating_sub(1) {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(vec![
                Span::styled("  [Ctrl-P] Toggle Panel", Style::default().fg(Color::DarkGray)),
            ]));
        }

        let panel_border_style = Style::default().fg(Color::DarkGray);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(panel_border_style)
            .title(" Agent Activity ");

        let paragraph = Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, area);
    }

    fn render_connect_popup(&self, f: &mut ratatui::Frame) {
        if !self.connect_popup.active {
            return;
        }
        
        let size = f.size();
        let popup_area = {
            let width = 60.min(size.width);
            let height = 18.min(size.height);
            let x = (size.width - width) / 2;
            let y = (size.height - height) / 2;
            let rect = ratatui::layout::Rect::new(x, y, width, height);
            rect
        };
        
        f.render_widget(ratatui::widgets::Clear, popup_area);
        
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White))
            .title(" 🔌 CONNECTION SETTINGS (OpenCode Mode) ");
            
        f.render_widget(block, popup_area);
        
        let inner_area = popup_area.inner(ratatui::layout::Margin { vertical: 1, horizontal: 2 });
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8), // Providers & Models row
                Constraint::Length(3), // API Key input
                Constraint::Min(2),    // Instructions
            ])
            .split(inner_area);
            
        let row_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50), // Providers
                Constraint::Percentage(50), // Models
            ])
            .split(chunks[0]);
            
        let item_width = (row_chunks[0].width.saturating_sub(2)) as usize;

        // 1. Render Providers List
        let is_provider_active = self.connect_popup.active_field == ConnectPopupField::Provider;
        let provider_border_style = if is_provider_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let provider_title = if is_provider_active {
            " 1. Provider (Active) "
        } else {
            " 1. Provider "
        };
        let mut provider_spans = Vec::new();
        for (idx, p) in self.connect_popup.providers.iter().enumerate() {
            let is_selected = idx == self.connect_popup.selected_provider_idx;
            
            let (prefix, style) = if is_selected {
                if is_provider_active {
                    ("▶ ", Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD))
                } else {
                    ("➔ ", Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
                }
            } else {
                if is_provider_active {
                    ("  ", Style::default().fg(Color::White))
                } else {
                    ("  ", Style::default().fg(Color::DarkGray))
                }
            };
            
            let item_text = format!("{prefix}{p}");
            let padded_text = if item_text.chars().count() < item_width {
                let padding = " ".repeat(item_width - item_text.chars().count());
                format!("{item_text}{padding}")
            } else {
                item_text
            };
            
            provider_spans.push(Line::from(vec![
                Span::styled(padded_text, style)
            ]));
        }
        let provider_list = Paragraph::new(provider_spans)
            .block(Block::default().borders(Borders::ALL).border_style(provider_border_style).title(provider_title));
        f.render_widget(provider_list, row_chunks[0]);
        
        // 2. Render Models List
        let is_model_active = self.connect_popup.active_field == ConnectPopupField::Model;
        let model_border_style = if is_model_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let model_title = if is_model_active {
            " 2. Model (Active) "
        } else {
            " 2. Model "
        };
        let mut model_spans = Vec::new();
        for (idx, m) in self.connect_popup.models.iter().enumerate() {
            let is_selected = idx == self.connect_popup.selected_model_idx;
            
            let (prefix, style) = if is_selected {
                if is_model_active {
                    ("▶ ", Style::default().bg(Color::Cyan).fg(Color::Black).add_modifier(Modifier::BOLD))
                } else {
                    ("➔ ", Style::default().bg(Color::DarkGray).fg(Color::White).add_modifier(Modifier::BOLD))
                }
            } else {
                if is_model_active {
                    ("  ", Style::default().fg(Color::White))
                } else {
                    ("  ", Style::default().fg(Color::DarkGray))
                }
            };
            
            let item_text = format!("{prefix}{m}");
            let padded_text = if item_text.chars().count() < item_width {
                let padding = " ".repeat(item_width - item_text.chars().count());
                format!("{item_text}{padding}")
            } else {
                item_text
            };
            
            model_spans.push(Line::from(vec![
                Span::styled(padded_text, style)
            ]));
        }
        let model_list = Paragraph::new(model_spans)
            .block(Block::default().borders(Borders::ALL).border_style(model_border_style).title(model_title));
        f.render_widget(model_list, row_chunks[1]);
        
        // 3. Render Api Key Field
        let is_api_active = self.connect_popup.active_field == ConnectPopupField::ApiKey;
        let api_key_border_style = if is_api_active {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        
        let api_key_title = if self.connect_popup.api_key.is_empty() {
            if is_api_active {
                " 🔑 API KEY (Type API Key) "
            } else {
                " 🔑 API KEY (Empty - falls back to env var) "
            }
        } else {
            if is_api_active {
                " 🔒 API KEY (Active - press Enter to save) "
            } else {
                " 🔒 API KEY (Configured) "
            }
        };

        let api_key_display = if self.connect_popup.api_key.is_empty() {
            if is_api_active {
                "".to_string()
            } else {
                " <not set - using env var> ".to_string()
            }
        } else {
            "*".repeat(self.connect_popup.api_key.len())
        };
        
        let api_key_style = if self.connect_popup.api_key.is_empty() {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let api_key_paragraph = Paragraph::new(Line::from(vec![
            Span::styled(api_key_display, api_key_style)
        ]))
        .block(Block::default().borders(Borders::ALL).border_style(api_key_border_style).title(api_key_title));
        f.render_widget(api_key_paragraph, chunks[1]);
        
        // 4. Render Instructions
        let instructions = vec![
            Line::from(vec![
                Span::styled(" Nav: ", Style::default().fg(Color::DarkGray)),
                Span::styled("[Tab]/[Shift-Tab]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(" │ Select: ", Style::default().fg(Color::DarkGray)),
                Span::styled("[Up/Down]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled(" Connect: ", Style::default().fg(Color::DarkGray)),
                Span::styled("[Enter]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" │ Cancel: ", Style::default().fg(Color::DarkGray)),
                Span::styled("[Esc]", Style::default().fg(Color::Red)),
            ])
        ];
        let instructions_paragraph = Paragraph::new(instructions)
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(instructions_paragraph, chunks[2]);
        
        if is_api_active {
            let cursor_x = chunks[1].x + self.connect_popup.api_key.len() as u16 + 1;
            let cursor_y = chunks[1].y + 1;
            f.set_cursor(cursor_x, cursor_y);
        }
    }

    fn update_autocomplete_options(&mut self) {
        self.autocomplete_options.clear();
        if !self.input.starts_with('/') {
            self.autocomplete_index = 0;
            return;
        }

        let input_trimmed = self.input.trim_start();
        
        let path_commands = ["/context add ", "/context remove ", "/diff ", "/review "];
        let mut matched_cmd = None;
        for cmd in &path_commands {
            if input_trimmed.starts_with(cmd) {
                matched_cmd = Some(*cmd);
                break;
            }
        }

        if let Some(cmd) = matched_cmd {
            let path_part = &input_trimmed[cmd.len()..];
            let (dir_path, prefix) = if let Some(last_slash) = path_part.rfind('/') {
                (&path_part[..=last_slash], &path_part[last_slash + 1..])
            } else {
                ("", path_part)
            };

            let lookup_dir = if dir_path.is_empty() { "." } else { dir_path };
            if let Ok(entries) = std::fs::read_dir(lookup_dir) {
                let mut paths = Vec::new();
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(prefix) && !name.starts_with('.') {
                        let full_path = if dir_path.is_empty() {
                            name
                        } else {
                            format!("{}{}", dir_path, name)
                        };
                        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        let suffix = if is_dir { "/" } else { "" };
                        paths.push(format!("{}{}{}", cmd, full_path, suffix));
                    }
                }
                paths.sort();
                self.autocomplete_options = paths.into_iter().take(5).collect();
            }
        } else {
            let commands = [
                "/model ",
                "/context add ",
                "/context remove ",
                "/context list",
                "/agents list",
                "/agents kill ",
                "/agents toggle",
                "/resume ",
                "/diff ",
                "/plan",
                "/review ",
                "/init",
                "/theme dark",
                "/theme light",
                "/theme safe",
                "/connect ",
                "/help",
            ];

            let query = input_trimmed;
            let mut filtered: Vec<String> = commands
                .iter()
                .filter(|cmd| cmd.starts_with(query))
                .map(|s| s.to_string())
                .collect();
            filtered.sort();
            self.autocomplete_options = filtered;
        }

        if self.autocomplete_index >= self.autocomplete_options.len() {
            self.autocomplete_index = 0;
        }
    }

    fn render_autocomplete_popup(&self, f: &mut ratatui::Frame, area: ratatui::layout::Rect) {
        if self.autocomplete_options.is_empty() || self.focus != Focus::Input {
            return;
        }

        let height = (self.autocomplete_options.len() + 2) as u16;
        let width = 50;
        let popup_area = ratatui::layout::Rect {
            x: area.x + 1,
            y: area.y.saturating_sub(height),
            width,
            height,
        };

        let mut lines = Vec::new();
        let item_width = (width.saturating_sub(2)) as usize;
        for (i, opt) in self.autocomplete_options.iter().enumerate() {
            let is_selected = i == self.autocomplete_index;
            let style = if is_selected {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            
            let padded_opt = if opt.chars().count() < item_width {
                let padding = " ".repeat(item_width - opt.chars().count());
                format!("{opt}{padding}")
            } else {
                opt.clone()
            };
            
            lines.push(Line::from(vec![
                Span::styled(padded_opt, style)
            ]));
        }

        let block = Block::default()
            .title(" 💡 SUGGESTIONS (Tab to cycle) ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let paragraph = Paragraph::new(lines)
            .block(block)
            .style(Style::default().bg(self.theme_bg()));

        f.render_widget(ratatui::widgets::Clear, popup_area);
        f.render_widget(paragraph, popup_area);
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
                usage: None,
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

    #[tokio::test]
    async fn test_real_streaming() {
        let mut tui = test_tui();
        use forge_core::LoopEvent;
        
        tui.handle_loop_event(LoopEvent::AssistantMessageChunk {
            delta: "hello ".to_string(),
        });
        
        assert_eq!(tui.conversation.len(), 1);
        if let ConversationEntry::Assistant(ref text) = tui.conversation[0] {
            assert!(text.is_empty());
        } else {
            panic!("Expected Assistant entry");
        }
        assert!(tui.did_stream_chunk);

        tui.handle_loop_event(LoopEvent::AssistantMessageChunk {
            delta: "world".to_string(),
        });

        assert_eq!(tui.conversation.len(), 1);

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

        tui.handle_loop_event(LoopEvent::AssistantMessage {
            step: 0,
            content: "hello world".to_string(),
        });

        assert!(!tui.did_stream_chunk);
        assert_eq!(tui.conversation.len(), 1);
        if let ConversationEntry::Assistant(ref text) = tui.conversation[0] {
            assert_eq!(text, "hello world");
        } else {
            panic!("Expected Assistant entry");
        }
    }

    #[tokio::test]
    async fn test_interactive_approval_y_key() {
        let mut tui = test_tui();
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        tui.agent_rx = Some({
            let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
            let _ = event_tx.send(AgentUpdate::ApprovalRequired {
                tool_name: "write_file".to_string(),
                details: "write approved.txt".to_string(),
                tx,
            });
            event_rx
        });

        tui.poll_agent_updates().await;
        
        assert!(tui.pending_approval.is_some());
        
        tui.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()))
            .await;
            
        assert!(tui.pending_approval.is_none());
        
        let approved = rx.await.unwrap();
        assert!(approved);
    }

    #[tokio::test]
    async fn test_interactive_approval_n_key() {
        let mut tui = test_tui();
        let (tx, rx) = tokio::sync::oneshot::channel();
        
        tui.agent_rx = Some({
            let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
            let _ = event_tx.send(AgentUpdate::ApprovalRequired {
                tool_name: "write_file".to_string(),
                details: "write rejected.txt".to_string(),
                tx,
            });
            event_rx
        });

        tui.poll_agent_updates().await;
        
        assert!(tui.pending_approval.is_some());
        
        tui.handle_key_event(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::empty()))
            .await;
            
        assert!(tui.pending_approval.is_none());
        
        let approved = rx.await.unwrap();
        assert!(!approved);
    }

    #[tokio::test]
    async fn test_autocomplete_enter_and_tab() {
        let mut tui = test_tui();
        tui.input = "/pl".to_string();
        tui.cursor = tui.input.len();
        tui.update_autocomplete_options();

        assert!(!tui.autocomplete_options.is_empty());
        assert_eq!(tui.autocomplete_options[tui.autocomplete_index], "/plan");

        // Pressing Enter on complete command (no trailing space/slash) should execute it immediately
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        
        assert!(tui.plan_mode); // /plan toggles plan mode
        assert!(tui.input.is_empty());
        assert!(tui.autocomplete_options.is_empty());

        // Pressing Enter on incomplete command (trailing space) should NOT execute it
        tui.input = "/model".to_string();
        tui.cursor = tui.input.len();
        tui.update_autocomplete_options();
        
        // Find "/model " (ends with space)
        if let Some(pos) = tui.autocomplete_options.iter().position(|opt| opt == "/model ") {
            tui.autocomplete_index = pos;
        }
        
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        assert_eq!(tui.input, "/model ");
        assert_eq!(tui.cursor, "/model ".len());
        // Since it ends with space, plan mode or model changes are not processed (input stays /model )
        assert!(tui.plan_mode); // remains toggled from before
    }

    #[tokio::test]
    async fn test_agent_running_allows_local_commands() {
        let mut tui = test_tui();
        tui.agent_running = true;

        // /plan is a local command, so it should NOT be queued
        tui.input = "/plan".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        
        assert!(tui.plan_mode); // plan mode toggled immediately
        assert!(tui.queued_messages.is_empty()); // not queued

        // Normal text should still be queued
        tui.input = "normal prompt".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        assert_eq!(tui.queued_messages, vec!["normal prompt".to_string()]); // queued!

        // /connect is a local command, so it should NOT be queued
        tui.input = "/connect".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        
        assert!(tui.connect_popup.active); // popup opened immediately
        assert_eq!(tui.queued_messages.len(), 1); // no new items queued
    }

    #[tokio::test]
    async fn test_token_count_not_overwritten() {
        let mut tui = test_tui();
        tui.conversation.push(ConversationEntry::User("short message".to_string()));
        
        // Before exact token count is received, update_token_count estimates it
        tui.update_token_count();
        assert!(tui.token_used > 3000); // 3000 base + estimate
        
        // Receive exact token count
        tui.handle_loop_event(forge_core::LoopEvent::TokensUsed {
            prompt: 100,
            completion: 50,
            total: 150,
        });
        assert_eq!(tui.token_used, 150);
        assert!(tui.has_exact_tokens);

        // Submitting/updating conv should not overwrite
        tui.update_token_count();
        assert_eq!(tui.token_used, 150); // Kept!
        
        // Starting a new task should reset it
        tui.start_agent_task("new task".to_string());
        assert!(!tui.has_exact_tokens);
    }

    #[tokio::test]
    async fn test_slash_command_model_and_connect() {
        let mut tui = test_tui();
        
        // Test switching to a model in the catalog (will use mock provider info)
        tui.input = "/model mock".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
            
        assert_eq!(tui.provider.model(), "mock");
        
        // Test connect to mock provider explicitly
        tui.input = "/connect mock mock-model".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
            
        assert_eq!(tui.provider.model(), "mock-model");
    }

    #[tokio::test]
    async fn test_interactive_connect_popup_navigation() {
        let mut tui = test_tui();
        
        // Execute /connect without args to open the popup
        tui.input = "/connect".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
            
        assert!(tui.connect_popup.active);
        assert_eq!(tui.connect_popup.active_field, ConnectPopupField::Provider);
        
        // Switch field to Model
        tui.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .await;
        assert_eq!(tui.connect_popup.active_field, ConnectPopupField::Model);
        
        // Switch field to ApiKey
        tui.handle_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()))
            .await;
        assert_eq!(tui.connect_popup.active_field, ConnectPopupField::ApiKey);
        
        // Clear any configuration-loaded default API key for deterministic test execution
        tui.connect_popup.api_key.clear();
        
        // Type some keys for API key
        tui.handle_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::empty()))
            .await;
        tui.handle_key_event(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::empty()))
            .await;
        tui.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::empty()))
            .await;
        assert_eq!(tui.connect_popup.api_key, "key");
        
        // Backspace
        tui.handle_key_event(KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()))
            .await;
        assert_eq!(tui.connect_popup.api_key, "ke");
        
        // Escape cancels
        tui.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()))
            .await;
        assert!(!tui.connect_popup.active);
    }

    #[tokio::test]
    async fn test_yoga_layout_toggling_and_resizing() {
        let mut tui = test_tui();
        
        // Default: show_agent_panel is true (from config)
        assert!(tui.show_agent_panel);
        
        // Toggle via Ctrl-P keybinding
        tui.handle_key_event(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL))
            .await;
        assert!(!tui.show_agent_panel);
        
        // Toggle via slash command /agents toggle
        tui.input = "/agents toggle".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .await;
        assert!(tui.show_agent_panel);

        // Verify focus state adjustments for Diff height
        tui.diff_hunks = vec![DiffHunk {
            file_path: "src/lib.rs".to_string(),
            header: "@@ -1,3 +1,3 @@".to_string(),
            removals: vec!["old".to_string()],
            additions: vec!["new".to_string()],
            state: HunkState::Pending,
        }];
        
        // If not focused on Diff, height should be 6
        tui.focus = Focus::Input;
        let diff_height_unfocused = if tui.diff_hunks.is_empty() {
            0
        } else if tui.focus == Focus::Diff {
            12
        } else {
            6
        };
        assert_eq!(diff_height_unfocused, 6);

        // If focused on Diff, height should stretch to 12 (Yoga feature)
        tui.focus = Focus::Diff;
        let diff_height_focused = if tui.diff_hunks.is_empty() {
            0
        } else if tui.focus == Focus::Diff {
            12
        } else {
            6
        };
        assert_eq!(diff_height_focused, 12);
    }

    #[tokio::test]
    async fn test_opencode_dynamic_connect() {
        let mut tui = test_tui();
        
        // Mock opencode config
        let mut prov_map = HashMap::new();
        prov_map.insert(
            "custom-prov".to_string(),
            OpenCodeProvider {
                options: Some(OpenCodeProviderOptions {
                    api_key: Some("custom_key".to_string()),
                    base_url: Some("http://localhost:1234/v1".to_string()),
                }),
                models: Some({
                    let mut m = HashMap::new();
                    m.insert("custom-model".to_string(), serde_json::json!({}));
                    m
                }),
            },
        );
        tui.opencode_config = Some(OpenCodeConfig {
            provider: Some(prov_map),
            model: Some("custom-prov/custom-model".to_string()),
            small_model: None,
        });

        // 1. Test key resolution
        let resolved_key = tui.resolve_api_key_with_config("custom-prov", None);
        assert_eq!(resolved_key, "custom_key");

        // 2. Test create provider
        let provider = tui.create_tui_provider("custom-prov", "custom-model", "").unwrap();
        assert_eq!(provider.model(), "custom-model");

        // 3. Test slash command /connect with provider/model format
        tui.input = "/connect custom-prov/custom-model".to_string();
        tui.cursor = tui.input.len();
        tui.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())).await;
        
        assert_eq!(tui.provider.model(), "custom-model");
    }
}
