//! MCP client for connecting to external MCP servers
//!
//! Provides functionality to connect to MCP servers, list their tools,
//! and call those tools from within the agent loop.

use super::protocol::{JsonRpcRequest, JsonRpcResponse, McpTool};
use super::server_process::ServerProcess;
use anyhow::Result;

/// MCP client - connects to external MCP servers
pub struct McpClient {
    process: ServerProcess,
    initialized: bool,
    _capabilities: Option<serde_json::Value>,
}

impl McpClient {
    fn initialize_request() -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            method: "initialize".to_string(),
            params: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "clientInfo": {
                    "name": "forge",
                    "version": "0.190.0"
                }
            })),
        }
    }

    fn parse_tools_response(resp: JsonRpcResponse) -> Result<Vec<McpTool>> {
        let result = resp.result.ok_or_else(|| anyhow::anyhow!("No result"))?;
        let tools: Vec<McpTool> = serde_json::from_value(
            result
                .get("tools")
                .ok_or_else(|| anyhow::anyhow!("No tools"))?
                .clone(),
        )?;
        Ok(tools)
    }

    /// Create new stdio-based MCP client (spawn subprocess)
    pub async fn new_stdio(command: String, args: Vec<String>) -> Result<Self> {
        let process = ServerProcess::spawn(command, args).await?;
        Ok(Self {
            process,
            initialized: false,
            _capabilities: None,
        })
    }

    /// Initialize MCP session (send initialize request)
    pub async fn initialize(&mut self) -> Result<()> {
        let init_req = Self::initialize_request();

        let _response = self.process.send_and_recv(&init_req).await?;
        self.initialized = true;

        // Send initialized notification
        let notif = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(null),
            method: "notifications/initialized".to_string(),
            params: None,
        };
        self.process.send(&notif).await?;

        Ok(())
    }

    /// List available tools from MCP server
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(2),
            method: "tools/list".to_string(),
            params: None,
        };

        let resp = self.process.send_and_recv(&req).await?;
        Self::parse_tools_response(resp)
    }

    /// Call a tool on the MCP server
    pub async fn call_tool(
        &mut self,
        name: String,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(3),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        };

        let resp = self.process.send_and_recv(&req).await?;
        resp.result
            .ok_or_else(|| anyhow::anyhow!("Tool call failed"))
    }

    /// Check if client is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Check if server process is still running
    pub async fn is_connected(&self) -> bool {
        self.process.is_running().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_request_format() {
        let req = McpClient::initialize_request();
        let json = serde_json::to_value(req).unwrap();

        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], serde_json::json!(1));
        assert_eq!(json["method"], "initialize");
        assert_eq!(json["params"]["clientInfo"]["name"], "forge");
        assert_eq!(
            json["params"]["capabilities"]["tools"],
            serde_json::json!({})
        );
    }

    #[test]
    fn test_list_tools_parses_response() {
        let resp: JsonRpcResponse = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "tools": [
                    {
                        "name": "search_docs",
                        "description": "Search docs",
                        "input_schema": {"type": "object"}
                    }
                ]
            }
        }))
        .unwrap();

        let tools = McpClient::parse_tools_response(resp).unwrap();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "search_docs");
        assert_eq!(tools[0].description, "Search docs");
        assert_eq!(tools[0].input_schema, serde_json::json!({"type": "object"}));
    }
}
