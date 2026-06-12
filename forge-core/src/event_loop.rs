use anyhow::Result;
use forge_provider::{ModelProvider, Message, ToolCall};
use forge_context::ContextEngine;
use forge_sandbox::Sandbox;
use tracing::{debug, info};

/// Core event loop: observe -> think -> act
pub struct EventLoop<P: ModelProvider> {
    provider: P,
    context: ContextEngine,
    sandbox: Sandbox,
    running: bool,
}

impl<P: ModelProvider> EventLoop<P> {
    pub fn new(provider: P, context: ContextEngine, sandbox: Sandbox) -> Self {
        Self {
            provider,
            context,
            sandbox,
            running: true,
        }
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
        self.sandbox.diff_edit(&path, &old_text, &new_text).await?;
        debug!("Diff edit applied to {}", path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_provider::anthropic::AnthropicProvider;

    #[tokio::test]
    async fn test_event_loop_creation() {
        // Create a dummy provider with empty API key (will fail if actually called)
        let provider = AnthropicProvider::new("test-key", "test-model").unwrap();
        let context = ContextEngine::new("/tmp/test").unwrap();
        let sandbox = Sandbox::new("/tmp/test", "off").unwrap();

        let event_loop = EventLoop::new(provider, context, sandbox);
        assert!(event_loop.running);
    }
}
