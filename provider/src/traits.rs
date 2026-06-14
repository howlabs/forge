use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::{ChatResponse, Message};

/// Abstract model provider interface
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Send a chat request and return the response
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse>;

    /// Get the model name/version
    fn model(&self) -> &str;
}

#[async_trait]
impl<T: ModelProvider + ?Sized> ModelProvider for Arc<T> {
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse> {
        (**self).chat(messages).await
    }

    fn model(&self) -> &str {
        (**self).model()
    }
}
