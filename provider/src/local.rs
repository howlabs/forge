//! Local model provider implementation
//!
//! Provides ModelProvider trait implementation for local models via Ollama or llama.cpp.

use super::traits::ModelProvider;
use super::types::{ChatResponse, Message, MessageRole, ToolCall};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

/// Local model provider (Ollama, llama.cpp)
pub struct LocalProvider {
    backend: LocalBackend,
    model: String,
    client: Client,
}

#[derive(Debug, Clone)]
pub enum LocalBackend {
    Ollama { base_url: String },
    LlamaCpp { base_url: String },
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaFunction {
    name: String,
    arguments: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct LlamaCppResponse {
    content: String,
}

impl LocalProvider {
    /// Create Ollama-based local provider
    pub fn new_ollama(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            backend: LocalBackend::Ollama {
                base_url: base_url.into(),
            },
            model: model.into(),
            client: Client::new(),
        }
    }

    /// Create llama.cpp-based local provider
    pub fn new_llamacpp(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            backend: LocalBackend::LlamaCpp {
                base_url: base_url.into(),
            },
            model: model.into(),
            client: Client::new(),
        }
    }

    fn convert_messages(messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| {
                json!({
                    "role": match m.role {
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                        MessageRole::System => "system",
                    },
                    "content": m.content
                })
            })
            .collect()
    }

    fn parse_ollama_response(response: &str) -> Result<ChatResponse> {
        let ollama_response: OllamaResponse = serde_json::from_str(response)?;
        Ok(Self::chat_response_from_ollama(ollama_response))
    }

    fn chat_response_from_ollama(ollama_response: OllamaResponse) -> ChatResponse {
        let tool_calls = ollama_response
            .message
            .tool_calls
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.function.name.clone(),
                name: tc.function.name,
                arguments: tc.function.arguments,
            })
            .collect();

        ChatResponse {
            content: ollama_response.message.content,
            tool_calls,
        }
    }
}

#[async_trait]
impl ModelProvider for LocalProvider {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        let converted = Self::convert_messages(messages);

        let (url, body) = match &self.backend {
            LocalBackend::Ollama { base_url } => (
                format!("{}/api/chat", base_url),
                json!({
                    "model": self.model,
                    "messages": converted,
                    "stream": false
                }),
            ),
            LocalBackend::LlamaCpp { base_url } => (
                format!("{}/completion", base_url),
                json!({
                    "model": self.model,
                    "prompt": converted.last().and_then(|m| m["content"].as_str()).unwrap_or("")
                }),
            ),
        };

        let response = self.client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("Local API error: {}", error_text));
        }

        match self.backend {
            LocalBackend::Ollama { .. } => {
                let response_text = response.text().await?;
                Self::parse_ollama_response(&response_text)
            }
            LocalBackend::LlamaCpp { .. } => {
                let llama_response: LlamaCppResponse = response.json().await?;
                Ok(ChatResponse {
                    content: llama_response.content,
                    tool_calls: vec![],
                })
            }
        }
    }

    fn model(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_provider_ollama_creation() {
        let provider = LocalProvider::new_ollama("localhost:11434", "llama2");
        assert_eq!(provider.model(), "llama2");
    }

    #[test]
    fn test_local_provider_llamacpp_creation() {
        let provider = LocalProvider::new_llamacpp("localhost:8080", "codellama");
        assert_eq!(provider.model(), "codellama");
    }

    #[test]
    fn test_convert_messages() {
        let messages = vec![Message {
            role: MessageRole::User,
            content: "Hello".to_string(),
        }];

        let converted = LocalProvider::convert_messages(&messages);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[0]["content"], "Hello");
    }

    #[test]
    fn test_backend_types() {
        let ollama = LocalBackend::Ollama {
            base_url: "localhost:11434".to_string(),
        };
        let llamacpp = LocalBackend::LlamaCpp {
            base_url: "localhost:8080".to_string(),
        };

        assert!(matches!(ollama, LocalBackend::Ollama { .. }));
        assert!(matches!(llamacpp, LocalBackend::LlamaCpp { .. }));
    }

    #[test]
    fn test_parse_ollama_response_with_tool_calls() {
        let json = r#"{
            "message": {
                "content": "",
                "tool_calls": [{
                    "function": {
                        "name": "run_command",
                        "arguments": {"command": "cargo test"}
                    }
                }]
            }
        }"#;

        let response = LocalProvider::parse_ollama_response(json).unwrap();

        assert_eq!(response.content, "");
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "run_command");
        assert_eq!(response.tool_calls[0].name, "run_command");
        assert_eq!(response.tool_calls[0].arguments["command"], "cargo test");
    }

    #[test]
    fn test_parse_ollama_response_without_tool_calls() {
        let json = r#"{
            "message": {
                "content": "Hello from Ollama"
            }
        }"#;

        let response = LocalProvider::parse_ollama_response(json).unwrap();

        assert_eq!(response.content, "Hello from Ollama");
        assert!(response.tool_calls.is_empty());
    }
}
