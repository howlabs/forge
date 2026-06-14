pub mod anthropic;
pub mod gemini;
pub mod local;
pub mod openai;
pub mod traits;
pub mod types;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use local::LocalProvider;
pub use openai::OpenAIProvider;
pub use traits::ModelProvider;
pub use types::{ChatResponse, Message, ToolCall, ToolResponse};

/// ZAI uses an OpenAI-compatible chat completions API.
pub fn zai_provider(model: impl Into<String>, api_key: impl Into<String>) -> OpenAIProvider {
    OpenAIProvider::with_base_url(
        model,
        api_key,
        "https://api.z.ai/api/paas/v4/chat/completions",
    )
}
