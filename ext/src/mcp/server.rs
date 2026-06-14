//! MCP server implementation
//!
//! Exposes Forge's own tools over MCP protocol so other agents can drive it.

use super::handlers::handle_request;
use super::protocol::{JsonRpcRequest, JsonRpcResponse, McpTool};
use anyhow::Result;
use std::collections::HashMap;

/// MCP server - exposes Forge's tools over MCP protocol
#[derive(Default)]
pub struct McpServer {
    tools: HashMap<String, McpTool>,
}

impl McpServer {
    /// Create new MCP server
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool that other agents can call
    pub fn register_tool(
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

    /// Handle incoming JSON-RPC request
    pub fn handle(&self, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
        handle_request(self, request)
    }

    /// Get number of registered tools
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// List all registered tools
    pub fn list_tools(&self) -> Vec<McpTool> {
        self.tools.values().cloned().collect()
    }

    /// Get a specific tool by name
    pub fn get_tool(&self, name: &str) -> Option<McpTool> {
        self.tools.get(name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_server_creation() {
        let server = McpServer::new();
        assert_eq!(server.tool_count(), 0);
    }

    #[test]
    fn test_mcp_server_register_tool() {
        let mut server = McpServer::new();
        server.register_tool(
            "test_tool".to_string(),
            "A test tool".to_string(),
            serde_json::json!({"type": "object"}),
        );
        assert_eq!(server.tool_count(), 1);
        assert!(server.get_tool("test_tool").is_some());
    }

    #[test]
    fn test_mcp_server_list_tools() {
        let mut server = McpServer::new();
        server.register_tool(
            "tool1".to_string(),
            "Tool 1".to_string(),
            serde_json::json!({"type": "object"}),
        );
        server.register_tool(
            "tool2".to_string(),
            "Tool 2".to_string(),
            serde_json::json!({"type": "object"}),
        );

        let tools = server.list_tools();
        assert_eq!(tools.len(), 2);
    }

    #[test]
    fn test_mcp_server_handle_initialize() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            method: "initialize".to_string(),
            params: None,
        };

        let response = server.handle(&request).unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());
    }

    #[test]
    fn test_mcp_server_handle_tools_list() {
        let mut server = McpServer::new();
        server.register_tool(
            "test".to_string(),
            "Test tool".to_string(),
            serde_json::json!({"type": "object"}),
        );

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(2),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = server.handle(&request).unwrap();
        assert!(response.result.is_some());

        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(|t| t.as_array());
        assert!(tools.is_some());
        assert_eq!(tools.unwrap().len(), 1);
    }

    #[test]
    fn test_mcp_server_handle_unknown_method() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(3),
            method: "unknown_method".to_string(),
            params: None,
        };

        let response = server.handle(&request).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601); // Method not found
    }
}
