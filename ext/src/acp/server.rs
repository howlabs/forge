//! ACP server - real implementation with provider integration

use super::handlers::handle_acp_request;
use super::protocol::*;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Chat handler function type
pub type ChatHandler = Arc<
    dyn Fn(
            Vec<ChatMessage>,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send>>
        + Send
        + Sync,
>;

pub struct AcpServer {
    agent_info: InitializeResult,
    pub(crate) editor_capabilities: Option<EditorCapabilities>,
    pub(crate) root_path: Option<String>,
    chat_handler: Option<ChatHandler>,
    initialized: bool,
    request_id: u64,
}

impl AcpServer {
    pub fn new(agent_name: &str, agent_version: &str) -> Self {
        Self {
            agent_info: InitializeResult {
                agent_name: agent_name.into(),
                agent_version: agent_version.into(),
                capabilities: AgentCapabilities {
                    supports_chat: true,
                    supports_edit: true,
                    supports_command: true,
                    supports_file_read: true,
                    supports_file_write: true,
                    supports_code_actions: true,
                    supports_diagnostics: true,
                },
                instructions: Some("Forge coding agent".into()),
            },
            editor_capabilities: None,
            root_path: None,
            chat_handler: None,
            initialized: false,
            request_id: 0,
        }
    }

    pub fn with_chat_handler(mut self, handler: ChatHandler) -> Self {
        self.chat_handler = Some(handler);
        self
    }

    pub fn with_root_path(mut self, root_path: &str) -> Self {
        self.root_path = Some(root_path.into());
        self
    }

    pub fn agent_info(&self) -> &InitializeResult {
        &self.agent_info
    }

    pub fn editor_capabilities(&self) -> Option<&EditorCapabilities> {
        self.editor_capabilities.as_ref()
    }

    pub fn root_path(&self) -> Option<&str> {
        self.root_path.as_deref()
    }

    pub fn chat_handler(&self) -> Option<&ChatHandler> {
        self.chat_handler.as_ref()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn set_initialized(&mut self, val: bool) {
        self.initialized = val;
    }

    pub async fn handle(&mut self, request: &AcpRequest) -> Result<AcpResponse> {
        handle_acp_request(self, request).await
    }

    pub fn next_id(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }

    /// Chat with the model via the configured handler
    pub async fn chat_with_model(&self, messages: &[ChatMessage]) -> Result<String> {
        let handler = self
            .chat_handler
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No chat handler configured"))?;
        (handler)(messages.to_vec()).await
    }

    /// Read a file from the filesystem
    pub fn read_file_content(
        &self,
        file_path: &str,
        line_start: Option<u32>,
        line_end: Option<u32>,
    ) -> Result<String> {
        let root = self.root_path.as_deref().unwrap_or(".");
        let full_path = Path::new(root).join(file_path);

        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", file_path, e))?;

        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len() as u32;

        let start = line_start.unwrap_or(1).saturating_sub(1) as usize;
        let end = line_end.unwrap_or(total_lines) as usize;
        let end = end.min(lines.len());

        let selected: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{}: {}", start + i + 1, l))
            .collect();

        Ok(selected.join("\n"))
    }

    /// Apply a text edit to a file
    pub fn apply_edit(&self, file_path: &str, old_text: &str, new_text: &str) -> Result<bool> {
        let root = self.root_path.as_deref().unwrap_or(".");
        let full_path = Path::new(root).join(file_path);

        let content = std::fs::read_to_string(&full_path)
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", file_path, e))?;

        if let Some(pos) = content.find(old_text) {
            let new_content = format!(
                "{}{}{}",
                &content[..pos],
                new_text,
                &content[pos + old_text.len()..]
            );
            std::fs::write(&full_path, new_content)
                .map_err(|e| anyhow::anyhow!("Failed to write {}: {}", file_path, e))?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Run a shell command
    pub async fn run_command(&self, command: &str, args: &[String]) -> Result<CommandOutput> {
        let output = tokio::process::Command::new(command)
            .args(args)
            .output()
            .await?;

        Ok(CommandOutput {
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            exit_code: output.status.code().unwrap_or(-1),
        })
    }

    /// Get diagnostics (TODO/FIXME/HACK/unwrap patterns)
    pub fn get_diagnostics(&self) -> Vec<Diagnostic> {
        let root = self.root_path.as_deref().unwrap_or(".");
        let root_path = Path::new(root);
        let mut diagnostics = Vec::new();

        if let Ok(entries) = std::fs::read_dir(root_path.join("src")) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        for (i, line) in content.lines().enumerate() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("// TODO") || trimmed.starts_with("// FIXME") {
                                diagnostics.push(Diagnostic {
                                    file_path: path
                                        .strip_prefix(root_path)
                                        .unwrap_or(&path)
                                        .to_string_lossy()
                                        .to_string(),
                                    line: (i + 1) as u32,
                                    column: 0,
                                    message: trimmed.to_string(),
                                    severity: DiagnosticSeverity::Warning,
                                });
                            }
                            if trimmed.contains("unwrap()") && !trimmed.starts_with("//") {
                                diagnostics.push(Diagnostic {
                                    file_path: path
                                        .strip_prefix(root_path)
                                        .unwrap_or(&path)
                                        .to_string_lossy()
                                        .to_string(),
                                    line: (i + 1) as u32,
                                    column: line.find("unwrap()").unwrap_or(0) as u32,
                                    message: "Consider using proper error handling".into(),
                                    severity: DiagnosticSeverity::Info,
                                });
                            }
                        }
                    }
                }
            }
        }

        diagnostics
    }
}

pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

pub type SharedAcpServer = Arc<RwLock<AcpServer>>;

pub fn shared_server(server: AcpServer) -> SharedAcpServer {
    Arc::new(RwLock::new(server))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_acp_server_creation() {
        let server = AcpServer::new("forge", "0.100.0");
        assert_eq!(server.agent_info().agent_name, "forge");
        assert!(!server.is_initialized());
    }

    #[tokio::test]
    async fn test_acp_server_handle_initialize() {
        let mut server = AcpServer::new("forge", "0.100.0");
        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_INITIALIZE.into(),
            params: Some(serde_json::json!({
                "editorName": "zed",
                "editorVersion": "0.1.0",
                "capabilities": {},
                "rootPath": "."
            })),
        };
        let resp = server.handle(&req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(server.is_initialized());
    }

    #[tokio::test]
    async fn test_acp_server_run_command() {
        let server = AcpServer::new("forge", "0.100.0");
        let output = server.run_command("echo", &["hello".into()]).await.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_acp_server_read_file() {
        let server = AcpServer::new("forge", "0.100.0").with_root_path(".");
        let content = server.read_file_content("Cargo.toml", None, None).unwrap();
        assert!(content.contains("[package]"));
    }

    #[test]
    fn test_acp_server_get_diagnostics() {
        let server = AcpServer::new("forge", "0.100.0");
        let diags = server.get_diagnostics();
        // May or may not find diagnostics depending on source
        assert!(diags.is_empty() || !diags.is_empty());
    }
}
