//! Gemini API provider implementation
//!
//! Provides ModelProvider trait implementation for Google's Gemini models.

use super::traits::ModelProvider;
use super::types::{ChatResponse, Message, MessageRole, ToolCall};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

/// Gemini API provider
pub struct GeminiProvider {
    model: String,
    api_key: String,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Deserialize)]
struct GeminiUsageMetadata {
    #[serde(rename = "promptTokenCount")]
    prompt_token_count: u32,
    #[serde(rename = "candidatesTokenCount")]
    candidates_token_count: u32,
    #[serde(rename = "totalTokenCount")]
    total_token_count: u32,
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
    text: Option<String>,
    #[serde(rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    args: HashMap<String, serde_json::Value>,
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

    fn build_request_body(messages: &[Message]) -> serde_json::Value {
        let gemini_messages = Self::convert_messages(messages);
        let system_content = messages
            .iter()
            .find(|m| matches!(m.role, MessageRole::System))
            .map(|m| m.content.as_str());

        let mut body = json!({ "contents": gemini_messages });
        if let Some(system) = system_content {
            body["systemInstruction"] = json!({ "parts": [{"text": system}] });
        }
        body
    }

    fn parse_chat_response(response: &str) -> Result<ChatResponse> {
        let gemini_response: GeminiResponse = serde_json::from_str(response)?;
        Self::chat_response_from_gemini(gemini_response)
    }

    fn chat_response_from_gemini(gemini_response: GeminiResponse) -> Result<ChatResponse> {
        let parts = &gemini_response
            .candidates
            .first()
            .ok_or_else(|| anyhow::anyhow!("No candidates in response"))?
            .content
            .parts;

        let content = parts
            .iter()
            .filter_map(|part| part.text.as_deref())
            .collect::<Vec<_>>()
            .join("");

        let tool_calls = parts
            .iter()
            .filter_map(|part| part.function_call.as_ref())
            .map(|function_call| ToolCall {
                id: function_call.name.clone(),
                name: function_call.name.clone(),
                arguments: function_call.args.clone(),
            })
            .collect();

        let usage = gemini_response.usage_metadata.map(|u| crate::types::TokenUsage {
            prompt_tokens: u.prompt_token_count,
            completion_tokens: u.candidates_token_count,
            total_tokens: u.total_token_count,
        });

        Ok(ChatResponse {
            content,
            tool_calls,
            usage,
        })
    }
}

#[async_trait]
impl ModelProvider for GeminiProvider {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let body = Self::build_request_body(messages);

        let response = self
            .client
            .post(format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                self.model, self.api_key
            ))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Gemini API error: {}", error_text));
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

    #[test]
    fn test_build_request_body_includes_system_instruction() {
        let messages = vec![
            Message::system("Follow system guidance"),
            Message::user("Hello"),
        ];

        let body = GeminiProvider::build_request_body(&messages);

        assert_eq!(body["contents"].as_array().unwrap().len(), 1);
        assert_eq!(
            body["systemInstruction"]["parts"][0]["text"],
            "Follow system guidance"
        );
    }

    #[test]
    fn test_parse_response_with_tool_calls() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "write_file",
                            "args": {"path": "README.md", "content": "hi"}
                        }
                    }]
                }
            }]
        }"#;

        let response = GeminiProvider::parse_chat_response(json).unwrap();

        assert_eq!(response.content, "");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "write_file");
        assert_eq!(response.tool_calls[0].arguments["path"], "README.md");
    }

    #[test]
    fn test_parse_response_without_tool_calls() {
        let json = r#"{
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello from Gemini"}]
                }
            }]
        }"#;

        let response = GeminiProvider::parse_chat_response(json).unwrap();

        assert_eq!(response.content, "Hello from Gemini");
        assert!(response.tool_calls.is_empty());
    }
}
