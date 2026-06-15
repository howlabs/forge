//! MCP request handlers
//!
//! Handles all MCP protocol methods: tools, resources, prompts, logging, sampling, roots.

use super::protocol::*;
use super::server::McpServer;
use anyhow::Result;

/// Handle incoming MCP request (async to support resource/prompt handlers)
pub async fn handle_request(server: &mut McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    match request.method.as_str() {
        METHOD_INITIALIZE => handle_initialize(server, request),
        METHOD_INITIALIZED => handle_initialized(server, request),
        METHOD_PING => handle_ping(request),
        METHOD_TOOLS_LIST => handle_list_tools(server, request),
        METHOD_TOOLS_CALL => handle_call_tool(server, request).await,
        METHOD_RESOURCES_LIST => handle_list_resources(server, request),
        METHOD_RESOURCES_TEMPLATES_LIST => handle_list_resource_templates(server, request),
        METHOD_RESOURCES_READ => handle_read_resource(server, request).await,
        METHOD_RESOURCES_SUBSCRIBE => handle_subscribe(server, request),
        METHOD_RESOURCES_UNSUBSCRIBE => handle_unsubscribe(server, request),
        METHOD_PROMPTS_LIST => handle_list_prompts(server, request),
        METHOD_PROMPTS_GET => handle_get_prompt(server, request).await,
        METHOD_LOGGING_SET_LEVEL => handle_set_level(server, request),
        METHOD_ROOTS_LIST => handle_list_roots(server, request),
        _ => Ok(error_response(&request.id, JsonRpcError::method_not_found(&request.method))),
    }
}

fn error_response(id: &serde_json::Value, error: JsonRpcError) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".into(), id: id.clone(), result: None, error: Some(error) }
}

fn success_response(id: &serde_json::Value, result: serde_json::Value) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".into(), id: id.clone(), result: Some(result), error: None }
}

fn handle_initialize(server: &mut McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let result = InitializeResult {
        protocol_version: "2024-11-05".into(),
        capabilities: server.get_capabilities().clone(),
        server_info: server.get_server_info().clone(),
        instructions: Some("Forge MCP server".into()),
    };
    server.set_initialized(true);
    Ok(success_response(&request.id, serde_json::to_value(result)?))
}

fn handle_initialized(_server: &mut McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(success_response(&request.id, serde_json::json!({})))
}

fn handle_ping(request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(success_response(&request.id, serde_json::json!({})))
}

fn handle_list_tools(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(success_response(&request.id, serde_json::json!({ "tools": server.list_tools() })))
}

async fn handle_call_tool(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let params = match request.params.as_ref() {
        Some(p) => p,
        None => return Ok(error_response(&request.id, JsonRpcError::invalid_params("Missing params"))),
    };
    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return Ok(error_response(&request.id, JsonRpcError::invalid_params("Missing tool name"))),
    };
    let arguments = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));

    if !server.has_tool_handler(tool_name) {
        if server.get_tool(tool_name).is_some() {
            let result = ToolCallResult {
                content: vec![Content::Text { text: format!("Tool '{}' registered but no handler attached", tool_name) }],
                is_error: Some(false),
            };
            return Ok(success_response(&request.id, serde_json::to_value(result)?));
        }
        return Ok(error_response(&request.id, JsonRpcError::invalid_params(&format!("Tool not found: {}", tool_name))));
    }

    match server.call_tool_handler(tool_name, arguments).await {
        Ok(result) => Ok(success_response(&request.id, serde_json::to_value(result)?)),
        Err(e) => {
            let result = ToolCallResult {
                content: vec![Content::Text { text: format!("Error: {}", e) }],
                is_error: Some(true),
            };
            Ok(success_response(&request.id, serde_json::to_value(result)?))
        }
    }
}

fn handle_list_resources(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(success_response(&request.id, serde_json::json!({ "resources": server.list_resources() })))
}

fn handle_list_resource_templates(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(success_response(&request.id, serde_json::json!({ "resourceTemplates": server.list_resource_templates() })))
}

async fn handle_read_resource(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let params = match request.params.as_ref() {
        Some(p) => p,
        None => return Ok(error_response(&request.id, JsonRpcError::invalid_params("Missing params"))),
    };
    let uri = match params.get("uri").and_then(|v| v.as_str()) {
        Some(u) => u,
        None => return Ok(error_response(&request.id, JsonRpcError::invalid_params("Missing uri"))),
    };

    match server.read_resource(uri).await {
        Ok(contents) => Ok(success_response(&request.id, serde_json::json!({ "contents": contents }))),
        Err(e) => Ok(error_response(&request.id, JsonRpcError::internal_error(&e.to_string()))),
    }
}

fn handle_subscribe(server: &mut McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let uri = request.params.as_ref()
        .and_then(|p| p.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    server.subscribe_resource(uri);
    Ok(success_response(&request.id, serde_json::json!({})))
}

fn handle_unsubscribe(server: &mut McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let uri = request.params.as_ref()
        .and_then(|p| p.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    server.unsubscribe_resource(uri);
    Ok(success_response(&request.id, serde_json::json!({})))
}

fn handle_list_roots(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let roots = server.get_roots().iter().map(|r| Root {
        uri: r.uri.clone(),
        name: r.name.clone(),
    }).collect::<Vec<_>>();
    Ok(success_response(&request.id, serde_json::json!({ "roots": roots })))
}

fn handle_list_prompts(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    Ok(success_response(&request.id, serde_json::json!({ "prompts": server.list_prompts() })))
}

async fn handle_get_prompt(server: &McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let params = match request.params.as_ref() {
        Some(p) => p,
        None => return Ok(error_response(&request.id, JsonRpcError::invalid_params("Missing params"))),
    };
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return Ok(error_response(&request.id, JsonRpcError::invalid_params("Missing prompt name"))),
    };
    let arguments = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));

    match server.get_prompt_result(name, arguments).await {
        Ok(result) => Ok(success_response(&request.id, serde_json::to_value(result)?)),
        Err(e) => Ok(error_response(&request.id, JsonRpcError::invalid_params(&e.to_string()))),
    }
}

fn handle_set_level(server: &mut McpServer, request: &JsonRpcRequest) -> Result<JsonRpcResponse> {
    let level = request.params.as_ref()
        .and_then(|p| p.get("level"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or(LogLevel::Info);
    server.set_log_level(level);
    Ok(success_response(&request.id, serde_json::json!({})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::McpServer;
    use std::sync::Arc;

    fn make_request(id: i32, method: &str) -> JsonRpcRequest {
        JsonRpcRequest { jsonrpc: "2.0".into(), id: serde_json::json!(id), method: method.into(), params: None }
    }

    #[tokio::test]
    async fn test_handle_initialize() {
        let mut server = McpServer::new("forge", "0.100.0");
        let req = make_request(1, METHOD_INITIALIZE);
        let resp = handle_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(server.is_initialized());
    }

    #[tokio::test]
    async fn test_handle_ping() {
        let mut server = McpServer::new("forge", "0.100.0");
        let req = make_request(1, METHOD_PING);
        let resp = handle_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_handle_tools_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.register_tool_simple("test".into(), "Test".into(), serde_json::json!({}));
        let req = make_request(1, METHOD_TOOLS_LIST);
        let resp = handle_request(&mut server, &req).await.unwrap();
        let result = resp.result.unwrap();
        let tools = result.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_tools_call_unknown() {
        let mut server = McpServer::new("forge", "0.100.0");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(), id: serde_json::json!(1), method: METHOD_TOOLS_CALL.into(),
            params: Some(serde_json::json!({"name": "nonexistent", "arguments": {}})),
        };
        let resp = handle_request(&mut server, &req).await.unwrap();
        assert!(resp.error.is_some());
    }

    #[tokio::test]
    async fn test_handle_tools_call_no_handler() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.register_tool_simple("test".into(), "Test".into(), serde_json::json!({}));
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(), id: serde_json::json!(1), method: METHOD_TOOLS_CALL.into(),
            params: Some(serde_json::json!({"name": "test", "arguments": {}})),
        };
        let resp = handle_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[tokio::test]
    async fn test_handle_resources_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.register_resource(McpResource {
            uri: "file:///a".into(), name: "a.txt".into(),
            description: None, mime_type: None,
        }, Arc::new(|_| Box::pin(async { Ok(vec![]) })));
        let req = make_request(1, METHOD_RESOURCES_LIST);
        let resp = handle_request(&mut server, &req).await.unwrap();
        let result = resp.result.unwrap();
        let resources = result.get("resources").unwrap().as_array().unwrap();
        assert_eq!(resources.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_prompts_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        server.register_prompt(McpPrompt {
            name: "greeting".into(), description: None, arguments: vec![],
        }, Arc::new(|_| Box::pin(async { Ok(GetPromptResult { description: None, messages: vec![] }) })));
        let req = make_request(1, METHOD_PROMPTS_LIST);
        let resp = handle_request(&mut server, &req).await.unwrap();
        let result = resp.result.unwrap();
        let prompts = result.get("prompts").unwrap().as_array().unwrap();
        assert_eq!(prompts.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_set_level() {
        let mut server = McpServer::new("forge", "0.100.0");
        let req = JsonRpcRequest {
            jsonrpc: "2.0".into(), id: serde_json::json!(1), method: METHOD_LOGGING_SET_LEVEL.into(),
            params: Some(serde_json::json!({"level": "debug"})),
        };
        let resp = handle_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(matches!(server.log_level(), LogLevel::Debug));
    }

    #[tokio::test]
    async fn test_handle_roots_list() {
        let mut server = McpServer::new("forge", "0.100.0");
        let req = make_request(1, METHOD_ROOTS_LIST);
        let resp = handle_request(&mut server, &req).await.unwrap();
        let result = resp.result.unwrap();
        let roots = result.get("roots").unwrap().as_array().unwrap();
        assert_eq!(roots.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let mut server = McpServer::new("forge", "0.100.0");
        let req = make_request(1, "unknown");
        let resp = handle_request(&mut server, &req).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}
