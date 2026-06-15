use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::{ChatResponse, Message, StreamEvent};

/// Abstract model provider interface
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Send a chat request and return the response
    async fn chat(&self, messages: &[Message]) -> Result<ChatResponse>;

    /// Get the model name/version
    fn model(&self) -> &str;

    /// Check if provider supports streaming
    fn supports_streaming(&self) -> bool {
        false
    }
}

/// Streaming model provider interface
#[async_trait]
pub trait StreamingProvider: ModelProvider {
    /// Send a chat request and return a stream of events
    async fn chat_stream(
        &self,
        messages: &[Message],
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>>;
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
