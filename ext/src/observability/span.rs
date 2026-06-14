//! Span builders for structured tracing
//!
//! Provides convenience functions for creating tracing spans.

use tracing::{info_span, Span};

/// Create span for agent step
pub fn step_span(step_name: &str) -> Span {
    info_span!("step", name = step_name)
}

/// Create span for tool call
pub fn tool_span(tool_name: &str) -> Span {
    info_span!("tool_call", tool = tool_name)
}

/// Create span for provider request
pub fn provider_span(provider: &str, model: &str) -> Span {
    info_span!("provider_request", provider = provider, model = model)
}

/// Create span for hook execution
pub fn hook_span(hook_type: &str, script_path: &str) -> Span {
    info_span!(
        "hook_execution",
        hook_type = hook_type,
        script = script_path
    )
}

/// Create span for MCP operation
pub fn mcp_span(operation: &str, server_name: &str) -> Span {
    info_span!("mcp_operation", operation = operation, server = server_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_step_span() {
        let span = step_span("test_step");
        // Just verify it doesn't panic
        drop(span);
    }

    #[test]
    fn test_tool_span() {
        let span = tool_span("test_tool");
        drop(span);
    }

    #[test]
    fn test_provider_span() {
        let span = provider_span("anthropic", "claude-sonnet-4");
        drop(span);
    }

    #[test]
    fn test_hook_span() {
        let span = hook_span("TaskCreated", "/path/to/hook.sh");
        drop(span);
    }

    #[test]
    fn test_mcp_span() {
        let span = mcp_span("tools/list", "filesystem");
        drop(span);
    }
}
