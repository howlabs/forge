pub mod anthropic;
pub mod gemini;
pub mod mock;
pub mod openai;
pub mod traits;
pub mod types;

pub use anthropic::AnthropicProvider;
pub use gemini::GeminiProvider;
pub use mock::MockProvider;
pub use openai::OpenAIProvider;
pub use traits::{ModelProvider, StreamingProvider};
pub use types::{ChatResponse, Message, StreamEvent, TokenUsage, ToolCall, ToolResponse};

use std::sync::Arc;

/// Provider registry entry for OpenAI-compatible providers
pub struct ProviderEntry {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub base_url: &'static str,
    pub env_var: &'static str,
    pub default_model: &'static str,
}

pub const PROVIDERS: &[ProviderEntry] = &[
    ProviderEntry {
        name: "openai",
        aliases: &[],
        base_url: "https://api.openai.com/v1/chat/completions",
        env_var: "OPENAI_API_KEY",
        default_model: "gpt-4o",
    },
    ProviderEntry {
        name: "zai",
        aliases: &["z.ai", "glm"],
        base_url: "https://api.z.ai/api/paas/v4/chat/completions",
        env_var: "ZAI_API_KEY",
        default_model: "glm-5.1",
    },
    ProviderEntry {
        name: "openrouter",
        aliases: &[],
        base_url: "https://openrouter.ai/api/v1/chat/completions",
        env_var: "OPENROUTER_API_KEY",
        default_model: "anthropic/claude-sonnet-4",
    },
];

/// Look up a provider entry by name or alias
pub fn find_provider(name: &str) -> Option<&'static ProviderEntry> {
    let lower = name.to_lowercase();
    PROVIDERS
        .iter()
        .find(|p| p.name == lower || p.aliases.iter().any(|a| *a == lower))
}

/// Create an OpenAI-compatible provider from a registry entry
pub fn create_openai_compatible(
    entry: &ProviderEntry,
    model: impl Into<String>,
    api_key: impl Into<String>,
) -> OpenAIProvider {
    OpenAIProvider::with_base_url(model, api_key, entry.base_url)
}

/// Create a provider by name. Handles Anthropic, Gemini, and
/// all OpenAI-compatible providers registered in `PROVIDERS`.
pub fn create_provider(
    name: &str,
    model: &str,
    api_key: &str,
) -> anyhow::Result<Arc<dyn ModelProvider>> {
    match name.to_lowercase().as_str() {
        "anthropic" => Ok(Arc::new(AnthropicProvider::new(api_key, model)?)),
        "gemini" => Ok(Arc::new(GeminiProvider::new(model, api_key))),
        "mock" | "local" => Ok(Arc::new(MockProvider::new(model))),
        _ => {
            if let Some(entry) = find_provider(name) {
                Ok(Arc::new(create_openai_compatible(entry, model, api_key)))
            } else {
                anyhow::bail!("Unknown provider: {}", name)
            }
        }
    }
}
