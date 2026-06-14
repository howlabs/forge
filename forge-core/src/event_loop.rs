use anyhow::Result;
use context::{ContextEngine, ContextIndex};
use forge_ext::mcp::{McpClient, McpTool};
use provider::{Message, ModelProvider, ToolCall};
use sandbox::Sandbox;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

/// Verification result consumed by the EventLoop verify retry flow.
#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub passed: bool,
    pub logs: String,
    pub duration_ms: u64,
}

/// Build/test verifier contract used by EventLoop.
#[async_trait::async_trait]
pub trait Verifier: Send + Sync {
    async fn verify(&self, workdir: &Path) -> Result<VerifyReport>;
    async fn quick_check(&self, workdir: &Path) -> Result<bool>;
}

const TOOL_DEFINITIONS: &str = r#"
## Available Tools

You have access to the following tools. Call them using the tool_use format.

### read_file
Read the contents of a file.
Arguments: { "path": "<relative path>" }

### write_file
Write content to a file (creates or overwrites).
Arguments: { "path": "<relative path>", "content": "<file content>" }

### diff_edit
Replace a specific text in a file with new text.
Arguments: { "path": "<relative path>", "old_text": "<exact text to replace>", "new_text": "<replacement text>" }

### run_command
Run a shell command in the project directory.
Arguments: { "command": "<shell command>" }

When you have completed the task and verified it works, respond with a plain text summary (no tool calls).
"#;

/// Default maximum steps to prevent infinite loops
const DEFAULT_MAX_STEPS: usize = 200;

/// Core event loop: observe -> think -> act
pub struct EventLoop<P: ModelProvider> {
    provider: P,
    context: ContextEngine,
    sandbox: Sandbox,
    running: bool,
    task: String,
    history: Vec<Message>,
    steps: usize,
    max_steps: usize,
    /// ContextIndex for symbol verification (v0.150.0)
    context_index: Option<Arc<Mutex<dyn ContextIndex>>>,
    mcp_client: Option<Arc<Mutex<McpClient>>>,
    mcp_tools: Vec<McpTool>,
}

impl<P: ModelProvider> EventLoop<P> {
    pub fn new(provider: P, context: ContextEngine, sandbox: Sandbox, task: String) -> Self {
        Self {
            provider,
            context,
            sandbox,
            running: true,
            task,
            history: Vec::new(),
            steps: 0,
            max_steps: DEFAULT_MAX_STEPS,
            context_index: None,
            mcp_client: None,
            mcp_tools: Vec::new(),
        }
    }

    /// Set the ContextIndex for symbol verification (v0.150.0)
    pub fn with_context_index(mut self, context_index: Arc<Mutex<dyn ContextIndex>>) -> Self {
        self.context_index = Some(context_index);
        self
    }

    pub async fn with_mcp_client(mut self, command: String, args: Vec<String>) -> Result<Self> {
        let mut client = McpClient::new_stdio(command, args).await?;
        client.initialize().await?;
        let tools = client.list_tools().await?;
        info!("MCP client connected, {} tools available", tools.len());
        self.mcp_tools = tools;
        self.mcp_client = Some(Arc::new(Mutex::new(client)));
        Ok(self)
    }

    pub async fn run(&mut self) -> Result<usize> {
        info!("Starting event loop");

        if self.history.is_empty() {
            self.history.push(Message::system(self.get_system_prompt()));
            self.history.push(Message::user(self.task.clone()));
        }

        while self.running {
            if self.steps >= self.max_steps {
                warn!(
                    "Event loop hit step limit ({}), stopping to prevent infinite loop",
                    self.max_steps
                );
                self.running = false;
                break;
            }

            let response = self.provider.chat(&self.history).await?;
            self.history
                .push(Message::assistant(response.content.clone()));
            self.steps += 1;

            if !response.tool_calls.is_empty() {
                for tool_call in response.tool_calls {
                    let result = self.execute_tool_with_result(tool_call).await;
                    let tool_result_msg = match result {
                        Ok(output) => format!("Tool result: {}", output),
                        Err(e) => format!("Tool error: {}", e),
                    };
                    self.history.push(Message::user(tool_result_msg));
                }
            } else {
                info!("Task complete after {} steps", self.steps);
                self.running = false;
            }
        }

        Ok(self.steps)
    }

    pub async fn run_with_verify(
        &mut self,
        verifier: &dyn Verifier,
        workdir: &Path,
        max_retries: usize,
    ) -> Result<usize> {
        self.run().await?;

        for attempt in 0..max_retries {
            let report = verifier.verify(workdir).await?;
            if report.passed {
                info!("Verify passed after {} steps", self.steps);
                return Ok(self.steps);
            }

            warn!(
                "Verify failed (attempt {}), feeding back to agent",
                attempt + 1
            );
            self.running = true;
            self.history.push(Message::user(format!(
                "Verification failed. Fix the errors and try again:\n{}",
                report.logs
            )));
            self.run().await?;
        }

        Err(anyhow::anyhow!(
            "Verify failed after {} retries",
            max_retries
        ))
    }

    fn get_system_prompt(&self) -> String {
        // Load AGENTS.md if it exists, otherwise use default
        let base = match self.context.load_agents_md() {
            Ok(content) => content,
            Err(_) => self.default_system_prompt(),
        };

        let context_chunks = self.context.query_context(&self.task, 5);
        let context_section = if !context_chunks.is_empty() {
            let chunks_text = context_chunks
                .iter()
                .map(|chunk| format!("// {}\n{}", chunk.file.display(), chunk.text))
                .collect::<Vec<_>>()
                .join("\n\n---\n\n");
            format!(
                "\n\n## Relevant Code Context\n\n```rust\n{}\n```",
                chunks_text
            )
        } else {
            String::new()
        };

        let mcp_section = if !self.mcp_tools.is_empty() {
            let tool_list = self
                .mcp_tools
                .iter()
                .map(|tool| format!("### {} (MCP)\n{}", tool.name, tool.description))
                .collect::<Vec<_>>()
                .join("\n\n");
            format!("\n\n## External MCP Tools\n\n{}", tool_list)
        } else {
            String::new()
        };

        format!(
            "{}{}{}\n\n{}",
            base, context_section, mcp_section, TOOL_DEFINITIONS
        )
    }

    fn default_system_prompt(&self) -> String {
        "You are Forge, a CLI coding agent. Help the user with software engineering tasks."
            .to_string()
    }

    async fn execute_tool_with_result(&mut self, tool_call: ToolCall) -> Result<String> {
        debug!("Executing tool: {}", tool_call.name);

        match tool_call.name.as_str() {
            "read_file" => self.tool_read_file(tool_call).await,
            "write_file" => self.tool_write_file(tool_call).await,
            "run_command" => self.tool_run_command(tool_call).await,
            "diff_edit" => self.tool_diff_edit(tool_call).await,
            _ => {
                if let Some(client) = &self.mcp_client {
                    let tool_name = tool_call.name.clone();
                    let args = serde_json::to_value(&tool_call.arguments)?;
                    let mut client = client.lock().await;
                    let result = client.call_tool(tool_name, args).await?;
                    Ok(result.to_string())
                } else {
                    debug!("Unknown tool: {}", tool_call.name);
                    Ok(format!("Unknown tool: {}", tool_call.name))
                }
            }
        }
    }

    async fn tool_read_file(&self, tool_call: ToolCall) -> Result<String> {
        let path: String = tool_call.get_arg("path")?;
        let content = self.sandbox.read_file(&path).await?;
        debug!("Read file {}: {} bytes", path, content.len());
        Ok(format!("File contents of {}:\n{}", path, content))
    }

    async fn tool_write_file(&self, tool_call: ToolCall) -> Result<String> {
        let path: String = tool_call.get_arg("path")?;
        let content: String = tool_call.get_arg("content")?;
        self.sandbox.write_file(&path, &content).await?;
        debug!("Wrote file {}", path);
        Ok(format!(
            "Successfully wrote {} bytes to {}",
            content.len(),
            path
        ))
    }

    async fn tool_run_command(&self, tool_call: ToolCall) -> Result<String> {
        let command: String = tool_call.get_arg("command")?;
        let output = self.sandbox.run_command(&command).await?;
        debug!("Command output: {}", output);
        Ok(format!("Command output:\n{}", output))
    }

    async fn tool_diff_edit(&mut self, tool_call: ToolCall) -> Result<String> {
        let path: String = tool_call.get_arg("path")?;
        let old_text: String = tool_call.get_arg("old_text")?;
        let new_text: String = tool_call.get_arg("new_text")?;

        // VERIFY-SYMBOL-BEFORE-EDIT (v0.150.0 Track B)
        // If we have a ContextIndex, verify symbols exist before allowing edit
        if let Some(context_index) = &self.context_index {
            self.verify_symbols_before_edit(context_index, &old_text, &new_text)
                .await?;
        }

        self.sandbox.diff_edit(&path, &old_text, &new_text).await?;
        debug!("Diff edit applied to {}", path);
        Ok(format!("Successfully applied diff edit to {}", path))
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
    use anyhow::Result;
    use async_trait::async_trait;
    use context::MockContextIndex;
    use forge_ext::mcp::McpTool;
    use provider::anthropic::AnthropicProvider;
    use std::collections::{HashMap, VecDeque};
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    struct MockProvider {
        responses: StdMutex<VecDeque<provider::ChatResponse>>,
    }

    impl MockProvider {
        fn new(responses: Vec<provider::ChatResponse>) -> Self {
            Self {
                responses: StdMutex::new(responses.into()),
            }
        }
    }

    #[async_trait]
    impl ModelProvider for MockProvider {
        async fn chat(&self, _messages: &[Message]) -> Result<provider::ChatResponse> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("No mock response available"))
        }

        fn model(&self) -> &str {
            "mock"
        }
    }

    struct RetryVerifier {
        attempts: StdMutex<usize>,
    }

    #[async_trait]
    impl Verifier for RetryVerifier {
        async fn verify(&self, _workdir: &Path) -> Result<VerifyReport> {
            let mut attempts = self.attempts.lock().unwrap();
            *attempts += 1;
            Ok(VerifyReport {
                passed: *attempts > 1,
                logs: format!("attempt {}", *attempts),
                duration_ms: 1,
            })
        }

        async fn quick_check(&self, _workdir: &Path) -> Result<bool> {
            Ok(true)
        }
    }

    fn chat_response(content: &str, tool_calls: Vec<ToolCall>) -> provider::ChatResponse {
        provider::ChatResponse {
            content: content.to_string(),
            tool_calls,
        }
    }

    fn tool_call(name: &str, arguments: HashMap<String, serde_json::Value>) -> ToolCall {
        ToolCall {
            id: format!("{}_id", name),
            name: name.to_string(),
            arguments,
        }
    }

    fn arg_map(entries: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    #[tokio::test]
    async fn test_event_loop_creation() {
        // Create a dummy provider with empty API key (will fail if actually called)
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();

        let event_loop = EventLoop::new(provider, context, sandbox, "test task".to_string());
        assert!(event_loop.running);
    }

    #[tokio::test]
    async fn test_run_accumulates_history() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("input.txt"), "hello").unwrap();

        let provider = MockProvider::new(vec![
            chat_response(
                "",
                vec![tool_call(
                    "read_file",
                    arg_map(&[("path", serde_json::json!("input.txt"))]),
                )],
            ),
            chat_response("done", vec![]),
        ]);
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();
        let mut event_loop = EventLoop::new(provider, context, sandbox, "read input".to_string());

        let steps = event_loop.run().await.unwrap();

        assert_eq!(steps, 2);
        assert_eq!(event_loop.history.len(), 5);
        assert!(event_loop.history[3]
            .content
            .contains("File contents of input.txt"));
    }

    #[tokio::test]
    async fn test_tool_read_file_returns_content() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        std::fs::write(temp_dir.path().join("input.txt"), "hello").unwrap();

        let provider = MockProvider::new(vec![]);
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();
        let event_loop = EventLoop::new(provider, context, sandbox, "read input".to_string());

        let result = event_loop
            .tool_read_file(tool_call(
                "read_file",
                arg_map(&[("path", serde_json::json!("input.txt"))]),
            ))
            .await
            .unwrap();

        assert!(result.contains("File contents of input.txt"));
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_tool_write_file_returns_confirmation() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let provider = MockProvider::new(vec![]);
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();
        let event_loop = EventLoop::new(provider, context, sandbox, "write output".to_string());

        let result = event_loop
            .tool_write_file(tool_call(
                "write_file",
                arg_map(&[
                    ("path", serde_json::json!("output.txt")),
                    ("content", serde_json::json!("hello")),
                ]),
            ))
            .await
            .unwrap();

        assert!(result.contains("Successfully wrote 5 bytes to output.txt"));
        assert_eq!(
            std::fs::read_to_string(temp_dir.path().join("output.txt")).unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn test_run_with_verify_retries_on_failure() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let provider = MockProvider::new(vec![
            chat_response("initial complete", vec![]),
            chat_response("fixed", vec![]),
        ]);
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();
        let mut event_loop = EventLoop::new(provider, context, sandbox, "fix task".to_string());
        let verifier = RetryVerifier {
            attempts: StdMutex::new(0),
        };

        let steps = event_loop
            .run_with_verify(&verifier, temp_dir.path(), 2)
            .await
            .unwrap();

        assert_eq!(steps, 2);
        assert!(event_loop
            .history
            .iter()
            .any(|message| message.content.contains("Verification failed")));
    }

    #[tokio::test]
    async fn test_unknown_tool_without_mcp_returns_error_message() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let provider = MockProvider::new(vec![]);
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();
        let mut event_loop = EventLoop::new(provider, context, sandbox, "unknown".to_string());

        let result = event_loop
            .execute_tool_with_result(tool_call("external_tool", HashMap::new()))
            .await
            .unwrap();

        assert_eq!(result, "Unknown tool: external_tool");
    }

    #[test]
    fn test_mcp_tools_injected_into_system_prompt() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let provider = MockProvider::new(vec![]);
        let context = ContextEngine::new(temp_dir.path()).unwrap();
        let sandbox = Sandbox::new(temp_dir.path(), "on").unwrap();
        let mut event_loop = EventLoop::new(provider, context, sandbox, "use mcp".to_string());
        event_loop.mcp_tools = vec![McpTool {
            name: "search_docs".to_string(),
            description: "Search documentation".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }];

        let prompt = event_loop.get_system_prompt();

        assert!(prompt.contains("## External MCP Tools"));
        assert!(prompt.contains("### search_docs (MCP)"));
        assert!(prompt.contains("Search documentation"));
    }

    #[tokio::test]
    async fn test_event_loop_with_context_index() {
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();
        let context_index: Arc<Mutex<dyn ContextIndex>> =
            Arc::new(Mutex::new(MockContextIndex::new()));

        let event_loop = EventLoop::new(provider, context, sandbox, "test task".to_string())
            .with_context_index(context_index);

        assert!(event_loop.context_index.is_some());
    }

    #[test]
    fn test_extract_function_call() {
        let event_loop = EventLoop::new(
            AnthropicProvider::new("test-key", "test-model").unwrap(),
            ContextEngine::new("/tmp/test").unwrap(),
            Sandbox::new("/tmp/test", "off").unwrap(),
            "test task".to_string(),
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
        use context::SymbolKind;
        use std::path::PathBuf;
        context_index.upsert_file(&PathBuf::from("lib.rs"), "fn existing_function() {}");

        // DEBUG: Verify symbol was extracted correctly
        use context::ContextIndex;
        let resolved = context_index.resolve_symbol("existing_function");
        assert!(
            resolved.is_some(),
            "Symbol 'existing_function' should exist after upsert_file"
        );
        let symbol = resolved.unwrap();
        assert_eq!(symbol.kind, SymbolKind::Function);

        let context_index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(context_index));

        let event_loop = EventLoop::new(provider, context, sandbox, "test task".to_string())
            .with_context_index(context_index);

        // Test with existing symbol (simplified - direct function call)
        let old_text = "old code";
        let new_text = "existing_function();";

        let result = event_loop
            .verify_symbols_before_edit(
                event_loop.context_index.as_ref().unwrap(),
                old_text,
                new_text,
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_symbols_before_edit_reject() {
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();

        let context_index: Arc<Mutex<dyn ContextIndex>> =
            Arc::new(Mutex::new(MockContextIndex::new()));

        let event_loop = EventLoop::new(provider, context, sandbox, "test task".to_string())
            .with_context_index(context_index);

        // Test with non-existent symbol
        let old_text = "old code";
        let new_text = "let x = non_existent_function();";

        let result = event_loop
            .verify_symbols_before_edit(
                event_loop.context_index.as_ref().unwrap(),
                old_text,
                new_text,
            )
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("REJECTED edit"));
    }
}
