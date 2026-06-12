use anyhow::Result;
use forge_provider::{ModelProvider, Message, ToolCall};
use forge_context::{ContextEngine, ContextIndex};
use forge_sandbox::Sandbox;
use tracing::{debug, info, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Core event loop: observe -> think -> act
pub struct EventLoop<P: ModelProvider> {
    provider: P,
    context: ContextEngine,
    sandbox: Sandbox,
    running: bool,
    /// ContextIndex for symbol verification (v0.150.0)
    context_index: Option<Arc<Mutex<dyn ContextIndex>>>,
}

impl<P: ModelProvider> EventLoop<P> {
    pub fn new(provider: P, context: ContextEngine, sandbox: Sandbox) -> Self {
        Self {
            provider,
            context,
            sandbox,
            running: true,
            context_index: None,
        }
    }

    /// Set the ContextIndex for symbol verification (v0.150.0)
    pub fn with_context_index(mut self, context_index: Arc<Mutex<dyn ContextIndex>>) -> Self {
        self.context_index = Some(context_index);
        self
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Starting event loop");

        while self.running {
            // 1. OBSERVE: Get current state
            let observation = self.observe().await?;
            debug!("Observation: {}", observation);

            // 2. THINK: Ask model what to do
            let messages = vec![
                Message::system(self.get_system_prompt()),
                Message::user(observation),
            ];

            let response = self.provider.chat(&messages).await?;

            // 3. ACT: Execute tool calls
            if !response.tool_calls.is_empty() {
                for tool_call in response.tool_calls {
                    self.execute_tool(tool_call).await?;
                }
            } else {
                // No tools to call, conversation might be done
                info!("Model response: {}", response.content);
                self.running = false;
            }
        }

        Ok(())
    }

    async fn observe(&self) -> Result<String> {
        // For v0.100.0 MVP: simple file listing
        let files = self.sandbox.list_files().await?;
        Ok(format!("Current files:\n{}", files.join("\n")))
    }

    fn get_system_prompt(&self) -> String {
        // Load AGENTS.md if it exists, otherwise use default
        match self.context.load_agents_md() {
            Ok(content) => content,
            Err(_) => self.default_system_prompt(),
        }
    }

    fn default_system_prompt(&self) -> String {
        "You are Forge, a CLI coding agent. Help the user with software engineering tasks."
            .to_string()
    }

    async fn execute_tool(&mut self, tool_call: ToolCall) -> Result<()> {
        debug!("Executing tool: {}", tool_call.name);

        match tool_call.name.as_str() {
            "read_file" => self.tool_read_file(tool_call).await,
            "write_file" => self.tool_write_file(tool_call).await,
            "run_command" => self.tool_run_command(tool_call).await,
            "diff_edit" => self.tool_diff_edit(tool_call).await,
            _ => {
                debug!("Unknown tool: {}", tool_call.name);
                Ok(())
            }
        }
    }

    async fn tool_read_file(&self, tool_call: ToolCall) -> Result<()> {
        let path: String = tool_call.get_arg("path")?;
        let content = self.sandbox.read_file(&path).await?;
        debug!("Read file {}: {} bytes", path, content.len());
        Ok(())
    }

    async fn tool_write_file(&self, tool_call: ToolCall) -> Result<()> {
        let path: String = tool_call.get_arg("path")?;
        let content: String = tool_call.get_arg("content")?;
        self.sandbox.write_file(&path, &content).await?;
        debug!("Wrote file {}", path);
        Ok(())
    }

    async fn tool_run_command(&self, tool_call: ToolCall) -> Result<()> {
        let command: String = tool_call.get_arg("command")?;
        let output = self.sandbox.run_command(&command).await?;
        debug!("Command output: {}", output);
        Ok(())
    }

    async fn tool_diff_edit(&self, tool_call: ToolCall) -> Result<()> {
        let path: String = tool_call.get_arg("path")?;
        let old_text: String = tool_call.get_arg("old_text")?;
        let new_text: String = tool_call.get_arg("new_text")?;

        // VERIFY-SYMBOL-BEFORE-EDIT (v0.150.0 Track B)
        // If we have a ContextIndex, verify symbols exist before allowing edit
        if let Some(context_index) = &self.context_index {
            self.verify_symbols_before_edit(context_index, &old_text, &new_text).await?;
        }

        self.sandbox.diff_edit(&path, &old_text, &new_text).await?;
        debug!("Diff edit applied to {}", path);
        Ok(())
    }

    /// Verify that symbols referenced in the edit exist in the ContextIndex
    /// This prevents editing non-existent APIs (solves #3 hallucination)
    async fn verify_symbols_before_edit(
        &self,
        context_index: &Arc<Mutex<dyn ContextIndex>>,
        old_text: &str,
        new_text: &str,
    ) -> Result<()> {
        debug!("Verifying symbols before edit");

        // Extract symbol references from old_text and new_text
        let old_symbols = self.extract_symbol_references(old_text);
        let new_symbols = self.extract_symbol_references(new_text);

        // Check that all symbols in new_text exist in the index
        let index = context_index.lock().await;
        for symbol_name in &new_symbols {
            // Skip symbols that are in old_text (they're just being moved around)
            if old_symbols.contains(symbol_name) {
                continue;
            }

            // Try to resolve the symbol
            if index.resolve_symbol(symbol_name).is_none() {
                let error_msg = format!(
                    "REJECTED edit: Symbol '{}' does not exist in context index. \
                    This prevents editing non-existent APIs (#3 hallucination).",
                    symbol_name
                );
                warn!("{}", error_msg);
                return Err(anyhow::anyhow!(error_msg));
            }
        }
        drop(index);

        debug!("Symbol verification passed");
        Ok(())
    }

    /// Extract symbol references from text (very basic implementation)
    /// This looks for patterns like "FunctionName", "TypeName::method", etc.
    fn extract_symbol_references(&self, text: &str) -> Vec<String> {
        let mut symbols = Vec::new();

        // Very basic pattern matching for function/method calls
        // This is a simplified version - in production you'd use tree-sitter
        for line in text.lines() {
            // Look for function calls: function_name(
            if let Some(cap) = self.extract_function_call(line) {
                symbols.push(cap);
            }

            // Look for method calls: Type::method(
            if let Some(cap) = self.extract_method_call(line) {
                symbols.push(cap);
            }
        }

        symbols
    }

    /// Extract function call pattern: "function_name("
    fn extract_function_call(&self, line: &str) -> Option<String> {
        let line = line.trim();
        if !line.contains("(") {
            return None;
        }

        // Find the last function call before "("
        let before_paren = line.split('(').next().unwrap_or("");
        let parts: Vec<&str> = before_paren.split_whitespace().collect();

        if let Some(last_part) = parts.last() {
            let func_name = last_part.trim_matches(|c| c == '.' || c == ',');
            if !func_name.is_empty() && func_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Some(func_name.to_string());
            }
        }

        None
    }

    /// Extract method call pattern: "TypeName::method("
    fn extract_method_call(&self, line: &str) -> Option<String> {
        if !line.contains("::") {
            return None;
        }

        // Find "Type::method(" patterns
        for part in line.split("::") {
            if part.contains('(') {
                let method_part = part.split('(').next().unwrap_or("");
                let method_name = method_part.trim();
                if !method_name.is_empty() {
                    // Reconstruct full symbol name (simplified)
                    return Some(format!("::{}", method_name));
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_provider::anthropic::AnthropicProvider;
    use forge_context::MockContextIndex;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_event_loop_creation() {
        // Create a dummy provider with empty API key (will fail if actually called)
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();

        let event_loop = EventLoop::new(provider, context, sandbox);
        assert!(event_loop.running);
    }

    #[tokio::test]
    async fn test_event_loop_with_context_index() {
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();
        let context_index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(MockContextIndex::new()));

        let event_loop = EventLoop::new(provider, context, sandbox)
            .with_context_index(context_index);

        assert!(event_loop.context_index.is_some());
    }

    #[test]
    fn test_extract_function_call() {
        let event_loop = EventLoop::new(
            AnthropicProvider::new("test-key", "test-model").unwrap(),
            ContextEngine::new("/tmp/test").unwrap(),
            Sandbox::new("/tmp/test", "off").unwrap(),
        );

        // Test function call extraction
        let line = "let result = some_function(arg1, arg2)";
        let func = event_loop.extract_function_call(line);
        assert_eq!(func, Some("some_function".to_string()));

        // Test method call extraction
        let line = "let result = Type::method_name(arg1)";
        let method = event_loop.extract_method_call(line);
        assert!(method.is_some());

        // Test non-call
        let line = "let x = 42";
        let func = event_loop.extract_function_call(line);
        assert!(func.is_none());
    }

    #[tokio::test]
    async fn test_verify_symbols_before_edit_pass() {
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();

        let mut context_index = MockContextIndex::new();

        // Add a symbol to the index
        use std::path::PathBuf;
        use forge_context::{Symbol, SymbolKind};
        context_index.upsert_file(
            &PathBuf::from("lib.rs"),
            "fn existing_function() {}"
        );

        // DEBUG: Verify symbol was extracted correctly
        use forge_context::ContextIndex;
        let resolved = context_index.resolve_symbol("existing_function");
        assert!(resolved.is_some(), "Symbol 'existing_function' should exist after upsert_file");
        let symbol = resolved.unwrap();
        assert_eq!(symbol.kind, SymbolKind::Function);

        let context_index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(context_index));

        let event_loop = EventLoop::new(provider, context, sandbox)
            .with_context_index(context_index);

        // Test with existing symbol (simplified - direct function call)
        let old_text = "old code";
        let new_text = "existing_function();";

        let result = event_loop.verify_symbols_before_edit(
            &event_loop.context_index.as_ref().unwrap(),
            old_text,
            new_text,
        ).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_symbols_before_edit_reject() {
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();

        let context_index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(MockContextIndex::new()));

        let event_loop = EventLoop::new(provider, context, sandbox)
            .with_context_index(context_index);

        // Test with non-existent symbol
        let old_text = "old code";
        let new_text = "let x = non_existent_function();";

        let result = event_loop.verify_symbols_before_edit(
            &event_loop.context_index.as_ref().unwrap(),
            old_text,
            new_text,
        ).await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("REJECTED edit"));
    }
}
