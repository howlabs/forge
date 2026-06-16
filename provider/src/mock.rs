//! Mock / local model provider.
//!
//! A deterministic, fully offline `ModelProvider` used for smoke tests,
//! CI gates, and local development without an API key or network access.
//!
//! By default it returns a single assistant message with no tool calls, which
//! drives the event loop to completion in one step. For richer scenarios a
//! caller can supply a scripted sequence of [`ChatResponse`]s; once the script
//! is exhausted the provider returns a terminal "done" response so the event
//! loop always terminates.

use super::traits::ModelProvider;
use super::types::{ChatResponse, Message};
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Mutex;

/// Deterministic offline provider.
pub struct MockProvider {
    model: String,
    /// Remaining scripted responses (front = next). Empty means "use default".
    scripted: Mutex<std::collections::VecDeque<ChatResponse>>,
    /// Response returned once the script is exhausted (or when unscripted).
    terminal: ChatResponse,
}

impl MockProvider {
    /// Create a mock provider that immediately reports completion.
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            scripted: Mutex::new(std::collections::VecDeque::new()),
            terminal: ChatResponse {
                content: "Mock provider: task acknowledged. No changes were made.".to_string(),
                tool_calls: Vec::new(),
            },
        }
    }

    /// Create a mock provider that replays `responses` in order, then falls
    /// back to a terminal no-tool response.
    pub fn scripted(model: impl Into<String>, responses: Vec<ChatResponse>) -> Self {
        let mut provider = Self::new(model);
        provider.scripted = Mutex::new(responses.into());
        provider
    }
}

#[async_trait]
impl ModelProvider for MockProvider {
    async fn chat(&self, _messages: &[Message]) -> Result<ChatResponse> {
        if let Some(next) = self.scripted.lock().unwrap().pop_front() {
            return Ok(next);
        }
        Ok(self.terminal.clone())
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolCall;
    use std::collections::HashMap;

    #[tokio::test]
    async fn default_returns_terminal_response_without_tools() {
        let provider = MockProvider::new("mock");
        let resp = provider.chat(&[]).await.unwrap();
        assert!(resp.tool_calls.is_empty());
        assert!(resp.content.contains("Mock provider"));
        assert_eq!(provider.model(), "mock");
    }

    #[tokio::test]
    async fn scripted_replays_then_terminates() {
        let scripted = vec![ChatResponse {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "1".into(),
                name: "read_file".into(),
                arguments: HashMap::new(),
            }],
        }];
        let provider = MockProvider::scripted("local", scripted);

        let first = provider.chat(&[]).await.unwrap();
        assert_eq!(first.tool_calls.len(), 1);

        // Script exhausted -> terminal response with no tool calls.
        let second = provider.chat(&[]).await.unwrap();
        assert!(second.tool_calls.is_empty());
    }
}
