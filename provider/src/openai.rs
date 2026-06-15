//! OpenAI API provider implementation
//!
//! Provides ModelProvider trait implementation for OpenAI's GPT models.

use super::traits::ModelProvider;
use super::types::{ChatResponse, Message, MessageRole, ToolCall};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

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
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAIToolCall>,
}

#[derive(Debug, Deserialize)]
struct OpenAIToolCall {
    id: String,
    function: OpenAIFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAIFunction {
    name: String,
    arguments: String,
}

impl OpenAIProvider {
    /// Create new OpenAI provider
    pub fn new(model: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            client: Client::new(),
            base_url: None,
        }
    }

    pub fn with_base_url(
        model: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            model: model.into(),
            api_key: api_key.into(),
            client: Client::new(),
            base_url: Some(base_url.into()),
        }
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    fn convert_messages(messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| match m.role {
                MessageRole::User => json!({"role": "user", "content": m.content}),
                MessageRole::Assistant => json!({"role": "assistant", "content": m.content}),
                MessageRole::System => json!({"role": "system", "content": m.content}),
            })
            .collect()
    }

    fn parse_chat_response(response: &str) -> Result<ChatResponse> {
        let openai_response: OpenAIResponse = serde_json::from_str(response)?;
        Self::chat_response_from_openai(openai_response)
    }

    fn chat_response_from_openai(openai_response: OpenAIResponse) -> Result<ChatResponse> {
        let message = &openai_response
            .choices
            .first()
            .ok_or_else(|| anyhow::anyhow!("No choices in response"))?
            .message;

        let tool_calls = message
            .tool_calls
            .iter()
            .map(|tc| {
                let arguments: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&tc.function.arguments).unwrap_or_default();
                ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments,
                }
            })
            .collect();

        Ok(ChatResponse {
            content: message.content.clone().unwrap_or_default(),
            tool_calls,
        })
    }
}

#[async_trait]
impl ModelProvider for OpenAIProvider {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let openai_messages = Self::convert_messages(messages);

        let url = self
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com/v1/chat/completions");

        let response = self
            .client
            .post(url)
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

        let response_text = response.text().await?;
        Self::parse_chat_response(&response_text)
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

    #[test]
    fn test_with_base_url() {
        let provider = OpenAIProvider::with_base_url(
            "glm-5.1",
            "test-key",
            "https://api.z.ai/api/paas/v4/chat/completions",
        );

        assert_eq!(provider.model(), "glm-5.1");
        assert_eq!(
            provider.base_url(),
            Some("https://api.z.ai/api/paas/v4/chat/completions")
        );
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let json = r#"{
            "choices": [{
                "message": {
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "function": {
                            "name": "read_file",
                            "arguments": "{\"path\":\"src/main.rs\"}"
                        }
                    }]
                }
            }]
        }"#;

        let response = OpenAIProvider::parse_chat_response(json).unwrap();

        assert_eq!(response.content, "");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_123");
        assert_eq!(response.tool_calls[0].name, "read_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "src/main.rs");
    }

    #[test]
    fn test_parse_response_without_tool_calls() {
        let json = r#"{
            "choices": [{
                "message": {
                    "content": "Hello from OpenAI"
                }
            }]
        }"#;

        let response = OpenAIProvider::parse_chat_response(json).unwrap();

        assert_eq!(response.content, "Hello from OpenAI");
        assert!(response.tool_calls.is_empty());
    }

    // reason: this test exercises a live LLM provider endpoint (Z.AI / GLM) and
    // therefore requires network access plus a valid ZAI_API_KEY. It is gated
    // behind the `integration` cargo feature and ignored by default so that
    // `cargo test --workspace` stays fully offline. Run it explicitly with:
    //     cargo test -p provider --features integration test_zai_real_api -- --ignored
    #[tokio::test]
    #[cfg_attr(not(feature = "integration"), ignore)]
    async fn test_zai_real_api() {
        let api_key = std::env::var("ZAI_API_KEY")
            .expect("Set ZAI_API_KEY environment variable to run this test");

        let provider = OpenAIProvider::with_base_url(
            "glm-4.5-air", // Cheaper model for testing
            api_key,
            "https://api.z.ai/api/paas/v4/chat/completions",
        );

        let messages = vec![Message::user("Say hello in one sentence")];
        let response = provider.chat(&messages).await;

        assert!(response.is_ok());
        let response = response.unwrap();
        assert!(!response.content.is_empty());
        println!("Z.AI Response: {}", response.content);
    }
}
