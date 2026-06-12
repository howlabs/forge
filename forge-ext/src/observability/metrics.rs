//! Metrics collection for observability
//!
//! Tracks token usage, duration, and other metrics.

use std::collections::HashMap;

/// Collector for metrics (token usage, duration, etc.)
pub struct MetricsCollector {
    token_usage: HashMap<String, TokenUsage>,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl MetricsCollector {
    /// Create new metrics collector
    pub fn new() -> Self {
        Self {
            token_usage: HashMap::new(),
        }
    }

    /// Record token usage for a provider
    pub fn record_tokens(&mut self, provider: &str, input: u64, output: u64) {
        let usage = self.token_usage
            .entry(provider.to_string())
            .or_insert_with(TokenUsage::default);
        usage.input_tokens += input;
        usage.output_tokens += output;
    }

    /// Get token usage for a provider
    pub fn get_usage(&self, provider: &str) -> TokenUsage {
        self.token_usage
            .get(provider)
            .cloned()
            .unwrap_or_default()
    }

    /// Get total usage across all providers
    pub fn get_total_usage(&self) -> TokenUsage {
        self.token_usage.values().fold(
            TokenUsage::default(),
            |mut acc, usage| {
                acc.input_tokens += usage.input_tokens;
                acc.output_tokens += usage.output_tokens;
                acc
            },
        )
    }

    /// Get all providers that have been tracked
    pub fn get_providers(&self) -> Vec<String> {
        self.token_usage.keys().cloned().collect()
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_collector_creation() {
        let collector = MetricsCollector::new();
        assert_eq!(collector.get_providers().len(), 0);
    }

    #[test]
    fn test_record_tokens() {
        let mut collector = MetricsCollector::new();
        collector.record_tokens("anthropic", 1000, 500);
        collector.record_tokens("anthropic", 500, 250);

        let usage = collector.get_usage("anthropic");
        assert_eq!(usage.input_tokens, 1500);
        assert_eq!(usage.output_tokens, 750);
    }

    #[test]
    fn test_get_total_usage() {
        let mut collector = MetricsCollector::new();
        collector.record_tokens("anthropic", 1000, 500);
        collector.record_tokens("openai", 500, 250);

        let total = collector.get_total_usage();
        assert_eq!(total.input_tokens, 1500);
        assert_eq!(total.output_tokens, 750);
    }

    #[test]
    fn test_get_providers() {
        let mut collector = MetricsCollector::new();
        collector.record_tokens("anthropic", 1000, 500);
        collector.record_tokens("openai", 500, 250);

        let providers = collector.get_providers();
        assert_eq!(providers.len(), 2);
        assert!(providers.contains(&"anthropic".to_string()));
        assert!(providers.contains(&"openai".to_string()));
    }

    #[test]
    fn test_get_usage_nonexistent_provider() {
        let collector = MetricsCollector::new();
        let usage = collector.get_usage("nonexistent");
        assert_eq!(usage.input_tokens, 0);
        assert_eq!(usage.output_tokens, 0);
    }

    #[test]
    fn test_metrics_collector_default() {
        let collector = MetricsCollector::default();
        assert_eq!(collector.get_providers().len(), 0);
    }
}
