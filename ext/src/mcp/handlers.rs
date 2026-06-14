//! MCP request handlers
//!
//! Implements handlers for various MCP request types.

use super::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use super::server::McpServer;
use anyhow::Result;

/// Handle incoming MCP request
pub fn handle_request(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    match request.method.as_str() {
        "tools/list" => handle_list_tools(server, request),
        "tools/call" => handle_call_tool(server, request),
        "initialize" => handle_initialize(request),
        _ => Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", request.method),
            }),
        }),
    }
}

/// Handle initialize request
fn handle_initialize(request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": "forge",
                "version": "0.190.0"
            }
        })),
        error: None,
    })
}

/// Handle tools/list request
fn handle_list_tools(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let tools = server.list_tools();
    Ok(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id.clone(),
        result: Some(serde_json::json!({ "tools": tools })),
        error: None,
    })
}

/// Handle tools/call request
fn handle_call_tool(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    // For v0.190.0, return a simple response
    // Real tool execution happens in Forge's event loop
    let params = request
        .params
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No params"))?;
    let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Verify tool exists
    if server.get_tool(tool_name).is_none() {
        return Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: request.id.clone(),
            result: None,
            error: Some(JsonRpcError {
                code: -32602, // Invalid params
                message: format!("Tool not found: {}", tool_name),
            }),
        });
    }

    Ok(JsonRpcResponse {
        jsonrpc: "2.0".to_string(),
        id: request.id.clone(),
        result: Some(serde_json::json!({
            "content": [{
                "type": "text",
                "text": format!("Tool '{}' called via MCP (v0.190.0 mock implementation)", tool_name)
            }]
        })),
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpServer;

    #[test]
    fn test_handle_initialize() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            method: "initialize".to_string(),
            params: None,
        };

        let response = handle_request(&server, &request).unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        assert!(result.get("protocolVersion").is_some());
        assert!(result.get("serverInfo").is_some());
    }

    #[test]
    fn test_handle_tools_list_empty() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(2),
            method: "tools/list".to_string(),
            params: None,
        };

        let response = handle_request(&server, &request).unwrap();
        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(|t| t.as_array()).unwrap();
        assert_eq!(tools.len(), 0);
    }

    #[test]
    fn test_handle_tools_list_with_tools() {
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

        let response = handle_request(&server, &request).unwrap();
        let result = response.result.unwrap();
        let tools = result.get("tools").and_then(|t| t.as_array()).unwrap();
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn test_handle_tools_call_existing_tool() {
        let mut server = McpServer::new();
        server.register_tool(
            "my_tool".to_string(),
            "My tool".to_string(),
            serde_json::json!({"type": "object"}),
        );

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(3),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "my_tool",
                "arguments": {"param": "value"}
            })),
        };

        let response = handle_request(&server, &request).unwrap();
        assert!(response.result.is_some());
        assert!(response.error.is_none());

        let result = response.result.unwrap();
        let content = result.get("content").and_then(|c| c.as_array()).unwrap();
        assert_eq!(content.len(), 1);
    }

    #[test]
    fn test_handle_tools_call_unknown_tool() {
        let server = McpServer::new();

        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(3),
            method: "tools/call".to_string(),
            params: Some(serde_json::json!({
                "name": "unknown_tool",
                "arguments": {}
            })),
        };

        let response = handle_request(&server, &request).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_unknown_method() {
        let server = McpServer::new();
        let request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(4),
            method: "unknown".to_string(),
            params: None,
        };

        let response = handle_request(&server, &request).unwrap();
        assert!(response.error.is_some());
        assert_eq!(response.error.unwrap().code, -32601);
    }
}
