//! OpenAI API provider implementation
//!
//! Provides ModelProvider and StreamingProvider trait implementations.

use super::traits::{ModelProvider, StreamingProvider};
use super::types::{ChatResponse, Message, MessageRole, StreamEvent, ToolCall};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use tokio::sync::mpsc;

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

    fn supports_streaming(&self) -> bool {
        true
    }
}

#[async_trait]
impl StreamingProvider for OpenAIProvider {
    async fn chat_stream(&self, messages: &[Message]) -> Result<mpsc::Receiver<StreamEvent>> {
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
                "messages": openai_messages,
                "stream": true
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("OpenAI streaming error: {}", error_text));
        }

        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            let mut buffer = String::new();
            let mut bytes = response.bytes_stream();

            use futures_util::StreamExt;
            while let Some(chunk) = bytes.next().await {
                match chunk {
                    Ok(data) => {
                        buffer.push_str(&String::from_utf8_lossy(&data));
                        while let Some(line_end) = buffer.find('\n') {
                            let line = buffer[..line_end].trim().to_string();
                            buffer = buffer[line_end + 1..].to_string();

                            if line.is_empty() || line == "data: [DONE]" {
                                if line == "data: [DONE]" {
                                    let _ = tx.send(StreamEvent::Done { usage: None }).await;
                                }
                                continue;
                            }

                            if let Some(data) = line.strip_prefix("data: ") {
                                if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(choices) =
                                        chunk.get("choices").and_then(|c| c.as_array())
                                    {
                                        if let Some(choice) = choices.first() {
                                            if let Some(delta) = choice.get("delta") {
                                                if let Some(content) =
                                                    delta.get("content").and_then(|c| c.as_str())
                                                {
                                                    let _ = tx
                                                        .send(StreamEvent::Delta {
                                                            content: content.to_string(),
                                                        })
                                                        .await;
                                                }
                                                if let Some(tool_calls) = delta
                                                    .get("tool_calls")
                                                    .and_then(|tc| tc.as_array())
                                                {
                                                    for tc in tool_calls {
                                                        if let Some(id) =
                                                            tc.get("id").and_then(|i| i.as_str())
                                                        {
                                                            if let Some(func) = tc.get("function") {
                                                                let name = func
                                                                    .get("name")
                                                                    .and_then(|n| n.as_str())
                                                                    .unwrap_or("");
                                                                let _ = tx.send(StreamEvent::ToolCallStart { id: id.to_string(), name: name.to_string() }).await;
                                                            }
                                                        }
                                                        if let Some(func) = tc.get("function") {
                                                            if let Some(args) = func
                                                                .get("arguments")
                                                                .and_then(|a| a.as_str())
                                                            {
                                                                let _ = tx.send(StreamEvent::ToolCallArgument { id: tc.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string(), argument: args.to_string() }).await;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx
                            .send(StreamEvent::Error {
                                message: e.to_string(),
                            })
                            .await;
                        break;
                    }
                }
            }
        });

        Ok(rx)
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
