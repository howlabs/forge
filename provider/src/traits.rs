use async_trait::async_trait;
use anyhow::Result;

use crate::{ChatResponse, Message};

/// Abstract model provider interface
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Send a chat request and return the response
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse>;

    /// Get the model name/version
    fn model(&self) -> &str;
}
