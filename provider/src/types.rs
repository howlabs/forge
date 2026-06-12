use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
        }
    }
}

/// Tool call requested by the model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: HashMap<String, serde_json::Value>,
}

impl ToolCall {
    pub fn get_arg<T: serde::de::DeserializeOwned>(&self, key: &str) -> anyhow::Result<T> {
        let value = self
            .arguments
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("Missing argument: {}", key))?;
        serde_json::from_value(value.clone())
            .map_err(|e| anyhow::anyhow!("Failed to parse argument {}: {}", key, e))
    }
}

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResponse {
    pub tool_call_id: String,
    pub content: String,
}

/// Response from a chat request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}
