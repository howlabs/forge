use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::traits::ModelProvider;
use crate::types::{ChatResponse, Message, ToolCall};
use std::collections::HashMap;

const API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic Claude API provider (v0.100.0: single provider for MVP)
pub struct AnthropicProvider {
    api_key: String,
    model: String,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
    id: Option<String>,
    name: Option<String>,
    input: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: Vec<AnthropicMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage<'a> {
    role: &'a str,
    content: &'a str,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Result<Self> {
        Ok(Self {
            api_key: api_key.into(),
            model: model.into(),
            client: Client::new(),
        })
    }

    fn convert_messages(messages: &[Message]) -> (Vec<AnthropicMessage<'_>>, Option<String>) {
        let system = messages
            .iter()
            .find(|m| matches!(m.role, crate::types::MessageRole::System))
            .map(|m| m.content.clone());

        let chat_messages: Vec<_> = messages
            .iter()
            .filter(|m| !matches!(m.role, crate::types::MessageRole::System))
            .map(|m| AnthropicMessage {
                role: match m.role {
                    crate::types::MessageRole::System => "system",
                    crate::types::MessageRole::User => "user",
                    crate::types::MessageRole::Assistant => "assistant",
                },
                content: &m.content,
            })
            .collect();

        (chat_messages, system)
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        debug!("Calling Anthropic API with model {}", self.model);

        let (chat_messages, system) = Self::convert_messages(messages);

        let request_builder = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&AnthropicRequest {
                model: &self.model,
                max_tokens: 4096,
                messages: chat_messages,
                system: system.as_deref(),
            });

        let response = request_builder.send().await?;
        if !response.status().is_success() {
            let status = response.status();
            let error = response.text().await?;
            return Err(anyhow::anyhow!("API error {}: {}", status, error));
        }

        let anthropic_response: AnthropicResponse = response.json().await?;

        let mut content_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for item in anthropic_response.content {
            match item.content_type.as_str() {
                "text" => {
                    if let Some(text) = item.text {
                        content_parts.push(text);
                    }
                }
                "tool_use" => {
                    if let (Some(id), Some(name), Some(input)) = (item.id, item.name, item.input) {
                        let arguments: HashMap<String, serde_json::Value> =
                            serde_json::from_value(input).unwrap_or_else(|_| HashMap::new());

                        tool_calls.push(ToolCall {
                            id,
                            name,
                            arguments,
                        });
                    }
                }
                _ => {}
            }
        }

        Ok(ChatResponse {
            content: content_parts.join("\n"),
            tool_calls,
        })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MessageRole;

    #[test]
    fn test_message_creation() {
        let msg = Message::system("test");
        assert!(matches!(msg.role, MessageRole::System));
        assert_eq!(msg.content, "test");
    }

    #[test]
    fn test_convert_messages() {
        let messages = vec![Message::system("You are helpful"), Message::user("Hello")];

        let (chat_messages, system) = AnthropicProvider::convert_messages(&messages);

        assert_eq!(system, Some("You are helpful".to_string()));
        assert_eq!(chat_messages.len(), 1);
        assert_eq!(chat_messages[0].role, "user");
    }
}
