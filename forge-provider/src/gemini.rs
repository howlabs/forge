//! Gemini API provider implementation
//!
//! Provides ModelProvider trait implementation for Google's Gemini models.

use super::types::{ChatResponse, Message, MessageRole};
use super::traits::ModelProvider;
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

/// Gemini API provider
pub struct GeminiProvider {
    model: String,
    api_key: String,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    text: String,
}

impl GeminiProvider {
    /// Create new Gemini provider
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
            .filter(|m| !matches!(m.role, MessageRole::System)) // Gemini doesn't use system role
            .map(|m| {
                json!({
                    "role": if matches!(m.role, MessageRole::User) { "user" } else { "model" },
                    "parts": [{"text": m.content}]
                })
            })
            .collect()
    }
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let gemini_messages = Self::convert_messages(messages);

        let response = self
            .client
            .post(format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                self.model, self.api_key
            ))
            .json(&json!({
                "contents": gemini_messages
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Gemini API error: {}", error_text));
        }

        let gemini_response: GeminiResponse = response.json().await?;
        let content = gemini_response
            .candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("No candidates in response"))?
            .content
            .parts
            .first()
            .ok_or_else(|| anyhow::anyhow!("No parts in content"))?
            .text
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
    fn test_gemini_provider_creation() {
        let provider = GeminiProvider::new("gemini-pro", "test-key");
        assert_eq!(provider.model(), "gemini-pro");
    }

    #[test]
    fn test_convert_messages_filters_system() {
        let messages = vec![
            Message {
                role: MessageRole::System,
                content: "System prompt".to_string(),
            },
            Message {
                role: MessageRole::User,
                content: "Hello".to_string(),
            },
        ];

        let converted = GeminiProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1); // System message filtered out
        assert_eq!(converted[0]["role"], "user");
    }

    #[test]
    fn test_convert_messages_user_model() {
        let messages = vec![
            Message {
                role: MessageRole::User,
                content: "User message".to_string(),
            },
            Message {
                role: MessageRole::Assistant,
                content: "Assistant message".to_string(),
            },
        ];

        let converted = GeminiProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[1]["role"], "model");
    }
}
