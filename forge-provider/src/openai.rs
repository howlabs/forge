//! OpenAI API provider implementation
//!
//! Provides ModelProvider trait implementation for OpenAI's GPT models.

use super::types::{ChatResponse, Message, MessageRole};
use super::traits::ModelProvider;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

/// OpenAI API provider
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    client: Client,
    base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIMessage {
    content: String,
}

impl OpenAIProvider {
    /// Create new OpenAI provider
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            client: Client::new(),
        }
    }

    fn convert_messages(messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| {
                match m.role {
                    MessageRole::User => json!({"role": "user", "content": m.content}),
                    MessageRole::Assistant => json!({"role": "assistant", "content": m.content}),
                    MessageRole::System => json!({"role": "system", "content": m.content}),
                }
            })
            .collect()
    }
}

#[async_trait]
impl ModelProvider for OpenAIProvider {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let openai_messages = Self::convert_messages(messages);

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&json!({
                "model": self.model,
                "messages": openai_messages
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("OpenAI API error: {}", error_text));
        }

        let openai_response: OpenAIResponse = response.json().await?;
        let content = openai_response
            .choices
            .first()
            .ok_or_else(|| anyhow::anyhow!("No choices in response"))?
            .message
            .content
            .clone();

        Ok(ChatResponse {
            content,
            tool_calls: vec![],
        })
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_provider_creation() {
        let provider = OpenAIProvider::new("gpt-4", "test-key");
        assert_eq!(provider.model(), "gpt-4");
    }

    #[test]
    fn test_convert_messages() {
        let messages = vec![
            Message {
                role: MessageRole::System,
                content: "You are helpful".to_string(),
            },
            Message {
                role: MessageRole::User,
                content: "Hello".to_string(),
            },
        ];

        let converted = OpenAIProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "system");
        assert_eq!(converted[1]["role"], "user");
    }
}
