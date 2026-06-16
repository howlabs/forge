//! MCP server implementation
//!
//! Exposes Forge's tools, resources, and prompts over MCP protocol.

use super::handlers::handle_request;
use super::protocol::*;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tool handler function type
pub type ToolHandler = Arc<
    dyn Fn(
            String,
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolCallResult>> + Send>>
        + Send
        + Sync,
>;

/// Resource provider function type
pub type ResourceProvider = Arc<
    dyn Fn(
            String,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<ResourceContents>>> + Send>,
        > + Send
        + Sync,
>;

/// Prompt handler function type
pub type PromptHandler = Arc<
    dyn Fn(
            serde_json::Value,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<GetPromptResult>> + Send>>
        + Send
        + Sync,
>;

/// MCP server - exposes Forge's capabilities over MCP protocol
pub struct McpServer {
    server_info: Implementation,
    capabilities: ServerCapabilities,
    tools: HashMap<String, McpTool>,
    tool_handlers: HashMap<String, ToolHandler>,
    resources: HashMap<String, McpResource>,
    resource_providers: HashMap<String, ResourceProvider>,
    resource_templates: Vec<ResourceTemplate>,
    prompts: HashMap<String, McpPrompt>,
    prompt_handlers: HashMap<String, PromptHandler>,
    log_level: LogLevel,
    initialized: bool,
}

impl McpServer {
    pub fn new(name: &str, version: &str) -> Self {
        Self {
            server_info: Implementation {
                name: name.into(),
                version: version.into(),
            },
            capabilities: ServerCapabilities::default(),
            tools: HashMap::new(),
            tool_handlers: HashMap::new(),
            resources: HashMap::new(),
            resource_providers: HashMap::new(),
            resource_templates: Vec::new(),
            prompts: HashMap::new(),
            prompt_handlers: HashMap::new(),
            log_level: LogLevel::Info,
            initialized: false,
        }
    }

    pub fn with_tools(mut self, enabled: bool) -> Self {
        if enabled {
            self.capabilities.tools = Some(ToolsCapability {
                list_changed: Some(true),
            });
        }
        self
    }

    pub fn with_resources(mut self, subscribe: bool, list_changed: bool) -> Self {
        self.capabilities.resources = Some(ResourcesCapability {
            subscribe: Some(subscribe),
            list_changed: Some(list_changed),
        });
        self
    }

    pub fn with_prompts(mut self, enabled: bool) -> Self {
        if enabled {
            self.capabilities.prompts = Some(PromptsCapability {
                list_changed: Some(true),
            });
        }
        self
    }

    pub fn with_logging(mut self) -> Self {
        self.capabilities.logging = Some(LoggingCapability {});
        self
    }

    pub fn with_sampling(mut self) -> Self {
        self.capabilities.sampling = Some(SamplingCapability {});
        self
    }

    pub fn register_tool(
        &mut self,
        name: String,
        description: String,
        input_schema: serde_json::Value,
        handler: ToolHandler,
    ) {
        let tool = McpTool {
            name: name.clone(),
            description,
            input_schema,
        };
        self.tools.insert(name.clone(), tool);
        self.tool_handlers.insert(name, handler);
    }

    pub fn register_tool_simple(
        &mut self,
        name: String,
        description: String,
        input_schema: serde_json::Value,
    ) {
        let tool = McpTool {
            name,
            description,
            input_schema,
        };
        self.tools.insert(tool.name.clone(), tool);
    }

    pub fn register_resource(&mut self, resource: McpResource, provider: ResourceProvider) {
        let key = resource.uri.clone();
        self.resource_providers.insert(key.clone(), provider);
        self.resources.insert(key, resource);
    }

    pub fn register_resource_template(
        &mut self,
        template: ResourceTemplate,
        provider: ResourceProvider,
    ) {
        let key = template.uri_template.clone();
        self.resource_templates.push(template);
        self.resource_providers.insert(key, provider);
    }

    pub fn register_prompt(&mut self, prompt: McpPrompt, handler: PromptHandler) {
        let key = prompt.name.clone();
        self.prompt_handlers.insert(key.clone(), handler);
        self.prompts.insert(key, prompt);
    }

    pub fn set_log_level(&mut self, level: LogLevel) {
        self.log_level = level;
    }

    pub fn get_server_info(&self) -> &Implementation {
        &self.server_info
    }

    pub fn get_capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    pub async fn handle(&mut self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        handle_request(self, request).await
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn list_tools(&self) -> Vec<McpTool> {
        self.tools.values().cloned().collect()
    }

    pub fn get_tool(&self, name: &str) -> Option<&McpTool> {
        self.tools.get(name)
    }

    pub fn has_tool_handler(&self, name: &str) -> bool {
        self.tool_handlers.contains_key(name)
    }

    pub async fn call_tool_handler(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<ToolCallResult> {
        let handler = self
            .tool_handlers
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("No handler for tool: {}", name))?;
        let name_owned = name.to_string();
        (handler)(name_owned, args).await
    }

    pub fn list_resources(&self) -> Vec<McpResource> {
        self.resources.values().cloned().collect()
    }

    pub fn get_resource(&self, uri: &str) -> Option<&McpResource> {
        self.resources.get(uri)
    }

    pub fn list_resource_templates(&self) -> Vec<ResourceTemplate> {
        self.resource_templates.clone()
    }

    pub async fn read_resource(&self, uri: &str) -> Result<Vec<ResourceContents>> {
        let provider = self
            .resource_providers
            .get(uri)
            .ok_or_else(|| anyhow::anyhow!("No provider for resource: {}", uri))?;
        let uri_owned = uri.to_string();
        (provider)(uri_owned).await
    }

    pub fn list_prompts(&self) -> Vec<McpPrompt> {
        self.prompts.values().cloned().collect()
    }

    pub fn get_prompt(&self, name: &str) -> Option<&McpPrompt> {
        self.prompts.get(name)
    }

    pub async fn get_prompt_result(
        &self,
        name: &str,
        args: serde_json::Value,
    ) -> Result<GetPromptResult> {
        let handler = self
            .prompt_handlers
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("No handler for prompt: {}", name))?;
        (handler)(args).await
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn set_initialized(&mut self, val: bool) {
        self.initialized = val;
    }

    pub fn log_level(&self) -> &LogLevel {
        &self.log_level
    }

    pub fn subscribe_resource(&mut self, uri: &str) {
        tracing::info!("Subscribed to resource: {}", uri);
    }

    pub fn unsubscribe_resource(&mut self, uri: &str) {
        tracing::info!("Unsubscribed from resource: {}", uri);
    }

    pub fn get_roots(&self) -> Vec<Root> {
        vec![Root {
            uri: "file:///.".into(),
            name: Some("workspace".into()),
        }]
    }
}

/// Shared server handle for concurrent access
pub type SharedMcpServer = Arc<RwLock<McpServer>>;

pub fn shared_server(server: McpServer) -> SharedMcpServer {
    Arc::new(RwLock::new(server))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_creation() {
        let server = McpServer::new("test", "1.0");
        assert_eq!(server.tool_count(), 0);
        assert_eq!(server.get_server_info().name, "test");
    }

    #[test]
    fn test_mcp_server_builder() {
        let server = McpServer::new("test", "1.0")
            .with_tools(true)
            .with_resources(true, true)
            .with_prompts(false)
            .with_logging();
        assert!(server.get_capabilities().tools.is_some());
        assert!(server.get_capabilities().resources.is_some());
        assert!(server.get_capabilities().prompts.is_none());
        assert!(server.get_capabilities().logging.is_some());
    }

    #[test]
    fn test_mcp_server_register_tool_simple() {
        let mut server = McpServer::new("test", "1.0");
        server.register_tool_simple(
            "test_tool".into(),
            "A test tool".into(),
            serde_json::json!({"type": "object"}),
        );
        assert_eq!(server.tool_count(), 1);
        assert!(server.get_tool("test_tool").is_some());
    }

    #[test]
    fn test_mcp_server_list_tools() {
        let mut server = McpServer::new("test", "1.0");
        server.register_tool_simple("tool1".into(), "Tool 1".into(), serde_json::json!({}));
        server.register_tool_simple("tool2".into(), "Tool 2".into(), serde_json::json!({}));
        assert_eq!(server.list_tools().len(), 2);
    }

    #[test]
    fn test_mcp_server_list_resources() {
        let mut server = McpServer::new("test", "1.0");
        server.resources.insert(
            "file:///a".into(),
            McpResource {
                uri: "file:///a".into(),
                name: "a".into(),
                description: None,
                mime_type: None,
            },
        );
        assert_eq!(server.list_resources().len(), 1);
    }

    #[test]
    fn test_mcp_server_list_prompts() {
        let mut server = McpServer::new("test", "1.0");
        server.prompts.insert(
            "greeting".into(),
            McpPrompt {
                name: "greeting".into(),
                description: None,
                arguments: vec![],
            },
        );
        assert_eq!(server.list_prompts().len(), 1);
    }

    #[test]
    fn test_mcp_server_log_level() {
        let mut server = McpServer::new("test", "1.0");
        assert!(matches!(server.log_level(), LogLevel::Info));
        server.set_log_level(LogLevel::Debug);
        assert!(matches!(server.log_level(), LogLevel::Debug));
    }

    #[tokio::test]
    async fn test_mcp_server_handle_initialize() {
        let mut server = McpServer::new("forge", "0.100.0").with_tools(true);
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(1),
            method: METHOD_INITIALIZE.into(),
            params: None,
        };
        let response = server.handle(&request).await.unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[tokio::test]
    async fn test_mcp_server_handle_tools_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.register_tool_simple("test".into(), "Test".into(), serde_json::json!({}));
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(2),
            method: METHOD_TOOLS_LIST.into(),
            params: None,
        };
        let response = server.handle(&request).await.unwrap();
        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(|t| t.as_array()).unwrap();
        assert_eq!(tools.len(), 1);
    }

    #[tokio::test]
    async fn test_mcp_server_handle_resources_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.resources.insert(
            "file:///a".into(),
            McpResource {
                uri: "file:///a".into(),
                name: "a.txt".into(),
                description: None,
                mime_type: None,
            },
        );
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(3),
            method: METHOD_RESOURCES_LIST.into(),
            params: None,
        };
        let response = server.handle(&request).await.unwrap();
        let result = response.result.unwrap();
        let resources = result.get("resources").and_then(|r| r.as_array()).unwrap();
        assert_eq!(resources.len(), 1);
    }

    #[tokio::test]
    async fn test_mcp_server_handle_prompts_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.prompts.insert(
            "greeting".into(),
            McpPrompt {
                name: "greeting".into(),
                description: Some("Say hello".into()),
                arguments: vec![],
            },
        );
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(4),
            method: METHOD_PROMPTS_LIST.into(),
            params: None,
        };
        let response = server.handle(&request).await.unwrap();
        let result = response.result.unwrap();
        let prompts = result.get("prompts").and_then(|p| p.as_array()).unwrap();
        assert_eq!(prompts.len(), 1);
    }

    #[tokio::test]
    async fn test_mcp_server_handle_unknown_method() {
        let mut server = McpServer::new("forge", "0.100.0");
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(5),
            method: "unknown_method".into(),
            params: None,
        };
        let response = server.handle(&request).await.unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }
}
