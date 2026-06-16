//! MCP client for connecting to external MCP servers
//!
//! Supports all MCP capabilities: tools, resources, prompts, logging, sampling, roots.

use super::protocol::*;
use super::server_process::ServerProcess;
use anyhow::Result;

/// MCP client - connects to external MCP servers
pub struct McpClient {
    process: ServerProcess,
    initialized: bool,
    server_capabilities: Option<ServerCapabilities>,
    server_info: Option<Implementation>,
    request_id: u64,
    roots: Vec<Root>,
}

impl McpClient {
    pub async fn new_stdio(command: String, args: Vec<String>) -> Result<Self> {
        let process = ServerProcess::spawn(command, args).await?;
        Ok(Self::from_process(process))
    }

    fn from_process(process: ServerProcess) -> Self {
        Self {
            process,
            initialized: false,
            server_capabilities: None,
            server_info: None,
            request_id: 0,
            roots: vec![Root {
                uri: "file:///.".into(),
                name: Some("workspace".into()),
            }],
        }
    }

    /// Create a client for testing (does not spawn a real process)
    #[cfg(test)]
    pub(crate) async fn new_stdio_sync(_command: String, _args: Vec<String>) -> Self {
        Self::mock().await
    }

    /// Create a mock client for testing (no real process)
    #[cfg(test)]
    pub(crate) async fn mock() -> Self {
        Self {
            process: ServerProcess::mock().await,
            initialized: false,
            server_capabilities: None,
            server_info: None,
            request_id: 0,
            roots: vec![Root {
                uri: "file:///.".into(),
                name: Some("workspace".into()),
            }],
        }
    }

    fn next_id(&mut self) -> u64 {
        self.request_id += 1;
        self.request_id
    }

    fn make_request(&mut self, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(self.next_id()),
            method: method.into(),
            params,
        }
    }

    fn make_notification(
        &self,
        method: &str,
        params: Option<serde_json::Value>,
    ) -> JsonRpcNotification {
        JsonRpcNotification::new(method, params)
    }

    pub async fn initialize(&mut self) -> Result<()> {
        let req = self.make_request(
            METHOD_INITIALIZE,
            Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "roots": { "listChanged": true },
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "forge",
                    "version": "0.100.0"
                }
            })),
        );

        let resp = self.process.send_and_recv(&req).await?;
        if let Some(err) = resp.error {
            return Err(anyhow::anyhow!("Initialize failed: {}", err.message));
        }
        if let Some(result) = resp.result {
            self.server_capabilities = Some(serde_json::from_value(
                result.get("capabilities").cloned().unwrap_or_default(),
            )?);
            self.server_info = Some(serde_json::from_value(
                result.get("server_info").cloned().unwrap_or_default(),
            )?);
        }

        let notif = self.make_notification(METHOD_INITIALIZED, None);
        self.process.send_notification(&notif).await?;
        self.initialized = true;
        Ok(())
    }

    pub async fn ping(&mut self) -> Result<()> {
        let req = self.make_request(METHOD_PING, None);
        let resp = self.process.send_and_recv(&req).await?;
        if resp.error.is_some() {
            return Err(anyhow::anyhow!("Ping failed"));
        }
        Ok(())
    }

    // ── Tools ──

    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let req = self.make_request(METHOD_TOOLS_LIST, None);
        let resp = self.process.send_and_recv(&req).await?;
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        Ok(serde_json::from_value(
            result.get("tools").cloned().unwrap_or_default(),
        )?)
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<ToolCallResult> {
        let req = self.make_request(
            METHOD_TOOLS_CALL,
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        );
        let resp = self.process.send_and_recv(&req).await?;
        if let Some(err) = resp.error {
            return Err(anyhow::anyhow!("Tool call failed: {}", err.message));
        }
        Ok(serde_json::from_value(resp.result.unwrap_or_default())?)
    }

    // ── Resources ──

    pub async fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        let req = self.make_request(METHOD_RESOURCES_LIST, None);
        let resp = self.process.send_and_recv(&req).await?;
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        Ok(serde_json::from_value(
            result.get("resources").cloned().unwrap_or_default(),
        )?)
    }

    pub async fn list_resource_templates(&mut self) -> Result<Vec<ResourceTemplate>> {
        let req = self.make_request(METHOD_RESOURCES_TEMPLATES_LIST, None);
        let resp = self.process.send_and_recv(&req).await?;
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        Ok(serde_json::from_value(
            result.get("resourceTemplates").cloned().unwrap_or_default(),
        )?)
    }

    pub async fn read_resource(&mut self, uri: &str) -> Result<Vec<ResourceContents>> {
        let req = self.make_request(
            METHOD_RESOURCES_READ,
            Some(serde_json::json!({ "uri": uri })),
        );
        let resp = self.process.send_and_recv(&req).await?;
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        Ok(serde_json::from_value(
            result.get("contents").cloned().unwrap_or_default(),
        )?)
    }

    pub async fn subscribe_resource(&mut self, uri: &str) -> Result<()> {
        let req = self.make_request(
            METHOD_RESOURCES_SUBSCRIBE,
            Some(serde_json::json!({ "uri": uri })),
        );
        self.process.send_and_recv(&req).await?;
        Ok(())
    }

    pub async fn unsubscribe_resource(&mut self, uri: &str) -> Result<()> {
        let req = self.make_request(
            METHOD_RESOURCES_UNSUBSCRIBE,
            Some(serde_json::json!({ "uri": uri })),
        );
        self.process.send_and_recv(&req).await?;
        Ok(())
    }

    // ── Prompts ──

    pub async fn list_prompts(&mut self) -> Result<Vec<McpPrompt>> {
        let req = self.make_request(METHOD_PROMPTS_LIST, None);
        let resp = self.process.send_and_recv(&req).await?;
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        Ok(serde_json::from_value(
            result.get("prompts").cloned().unwrap_or_default(),
        )?)
    }

    pub async fn get_prompt(
        &mut self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<GetPromptResult> {
        let req = self.make_request(
            METHOD_PROMPTS_GET,
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        );
        let resp = self.process.send_and_recv(&req).await?;
        if let Some(err) = resp.error {
            return Err(anyhow::anyhow!("Get prompt failed: {}", err.message));
        }
        Ok(serde_json::from_value(resp.result.unwrap_or_default())?)
    }

    // ── Logging ──

    pub async fn set_log_level(&mut self, level: LogLevel) -> Result<()> {
        let req = self.make_request(
            METHOD_LOGGING_SET_LEVEL,
            Some(serde_json::json!({
                "level": level
            })),
        );
        self.process.send_and_recv(&req).await?;
        Ok(())
    }

    // ── Sampling ──

    pub async fn create_message(
        &mut self,
        params: CreateMessageParams,
    ) -> Result<CreateMessageResult> {
        let req = self.make_request(
            METHOD_SAMPLING_CREATE_MESSAGE,
            Some(serde_json::to_value(params)?),
        );
        let resp = self.process.send_and_recv(&req).await?;
        if let Some(err) = resp.error {
            return Err(anyhow::anyhow!("Create message failed: {}", err.message));
        }
        Ok(serde_json::from_value(resp.result.unwrap_or_default())?)
    }

    // ── Roots ──

    pub async fn list_roots(&mut self) -> Result<Vec<Root>> {
        let req = self.make_request(METHOD_ROOTS_LIST, None);
        let resp = self.process.send_and_recv(&req).await?;
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        Ok(serde_json::from_value(
            result.get("roots").cloned().unwrap_or_default(),
        )?)
    }

    pub fn set_roots(&mut self, roots: Vec<Root>) {
        self.roots = roots;
    }

    pub fn get_roots(&self) -> &[Root] {
        &self.roots
    }

    // ── Notifications ──

    pub async fn notify_resources_list_changed(&self) -> Result<()> {
        let notif = self.make_notification(NOTIFICATION_RESOURCE_LIST_CHANGED, None);
        self.process.send_notification(&notif).await
    }

    pub async fn notify_cancelled(
        &self,
        request_id: serde_json::Value,
        reason: Option<String>,
    ) -> Result<()> {
        let notif = self.make_notification(
            NOTIFICATION_CANCELLED,
            Some(serde_json::json!({
                "requestId": request_id,
                "reason": reason
            })),
        );
        self.process.send_notification(&notif).await
    }

    // ── Status ──

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.server_capabilities.as_ref()
    }

    pub fn server_info(&self) -> Option<&Implementation> {
        self.server_info.as_ref()
    }

    pub async fn is_connected(&self) -> bool {
        self.process.is_running().await
    }

    pub fn supports_tools(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.tools.as_ref())
            .is_some()
    }

    pub fn supports_resources(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.resources.as_ref())
            .is_some()
    }

    pub fn supports_prompts(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.prompts.as_ref())
            .is_some()
    }

    pub fn supports_logging(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.logging.as_ref())
            .is_some()
    }

    pub fn supports_sampling(&self) -> bool {
        self.server_capabilities
            .as_ref()
            .and_then(|c| c.sampling.as_ref())
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_initialize_request_format() {
        let mut client = McpClient::new_stdio_sync("echo".into(), vec![]).await;
        let req = client.make_request(
            METHOD_INITIALIZE,
            Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "roots": { "listChanged": true }, "sampling": {} },
                "clientInfo": { "name": "forge", "version": "0.100.0" }
            })),
        );
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, METHOD_INITIALIZE);
        assert_eq!(req.id, serde_json::json!(1));
    }

    #[tokio::test]
    async fn test_client_roots() {
        let mut client = McpClient::new_stdio_sync("echo".into(), vec![]).await;
        let roots = vec![Root {
            uri: "file:///project".into(),
            name: Some("project".into()),
        }];
        client.set_roots(roots);
        assert_eq!(client.get_roots().len(), 1);
    }

    #[tokio::test]
    async fn test_supports_checks() {
        let mut client = McpClient::new_stdio_sync("echo".into(), vec![]).await;
        client.server_capabilities = Some(ServerCapabilities {
            tools: Some(ToolsCapability::default()),
            resources: None,
            prompts: Some(PromptsCapability::default()),
            logging: None,
            sampling: None,
        });
        assert!(client.supports_tools());
        assert!(!client.supports_resources());
        assert!(client.supports_prompts());
        assert!(!client.supports_logging());
        assert!(!client.supports_sampling());
    }

    #[tokio::test]
    async fn test_next_id_increments() {
        let mut client = McpClient::new_stdio_sync("echo".into(), vec![]).await;
        assert_eq!(client.next_id(), 1);
        assert_eq!(client.next_id(), 2);
        assert_eq!(client.next_id(), 3);
    }
}
