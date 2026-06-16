use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelCapability {
    pub tools: bool,
    pub streaming: bool,
    pub vision: bool,
    pub reasoning: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelInfo {
    pub provider: &'static str,
    pub model: &'static str,
    pub context_tokens: u32,
    pub default: bool,
    pub capability: ModelCapability,
    pub recommended_for: &'static [&'static str],
}

const TOOL_STREAM: ModelCapability = ModelCapability {
    tools: true,
    streaming: true,
    vision: false,
    reasoning: false,
};

const TOOL_STREAM_REASONING: ModelCapability = ModelCapability {
    tools: true,
    streaming: true,
    vision: false,
    reasoning: true,
};

pub const MODEL_CATALOG: &[ModelInfo] = &[
    ModelInfo {
        provider: "zai",
        model: "glm-5.1",
        context_tokens: 128_000,
        default: true,
        capability: TOOL_STREAM_REASONING,
        recommended_for: &["default", "code", "agentic-edits"],
    },
    ModelInfo {
        provider: "openai",
        model: "gpt-4o",
        context_tokens: 128_000,
        default: true,
        capability: TOOL_STREAM,
        recommended_for: &["code", "general"],
    },
    ModelInfo {
        provider: "anthropic",
        model: "claude-3-5-sonnet",
        context_tokens: 200_000,
        default: true,
        capability: TOOL_STREAM,
        recommended_for: &["code", "long-context"],
    },
    ModelInfo {
        provider: "gemini",
        model: "gemini-1.5-pro",
        context_tokens: 1_000_000,
        default: true,
        capability: TOOL_STREAM,
        recommended_for: &["long-context", "analysis"],
    },
    ModelInfo {
        provider: "openrouter",
        model: "anthropic/claude-sonnet-4",
        context_tokens: 200_000,
        default: true,
        capability: TOOL_STREAM,
        recommended_for: &["router", "code"],
    },
    ModelInfo {
        provider: "mock",
        model: "mock",
        context_tokens: 8_000,
        default: true,
        capability: ModelCapability {
            tools: false,
            streaming: false,
            vision: false,
            reasoning: false,
        },
        recommended_for: &["tests", "offline-smoke"],
    },
];

pub fn list_models(provider: Option<&str>) -> Vec<&'static ModelInfo> {
    let provider = provider.map(str::to_lowercase);
    MODEL_CATALOG
        .iter()
        .filter(|model| provider.as_ref().is_none_or(|p| model.provider == p))
        .collect()
}

pub fn default_model(provider: &str) -> Option<&'static ModelInfo> {
    let provider = provider.to_lowercase();
    MODEL_CATALOG
        .iter()
        .find(|model| model.provider == provider && model.default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_by_provider() {
        let models = list_models(Some("mock"));
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model, "mock");
    }
}
