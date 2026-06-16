//! `forge mcp serve` - expose Forge's built-in tools over MCP stdio.
//!
//! Runs a line-delimited JSON-RPC server on stdin/stdout that any MCP client
//! (including Forge's own `McpClient`) can connect to. Tools are backed by the
//! sandbox so the same path-traversal and command policy guards apply.

use anyhow::Result;
use forge_ext::mcp::protocol::{
    Content, JsonRpcError, JsonRpcRequest, JsonRpcResponse, ToolCallResult,
};
use forge_ext::mcp::{handle_request, McpServer};
use sandbox::Sandbox;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Build an MCP server exposing sandbox-backed filesystem/command tools.
pub fn build_server(project_path: String, network: String) -> McpServer {
    let mut server = McpServer::new("forge", env!("CARGO_PKG_VERSION"))
        .with_tools(true)
        .with_logging();

    let sandbox = Arc::new(SandboxToolset {
        project_path,
        network,
    });

    let read_sandbox = sandbox.clone();
    server.register_tool(
        "read_file".into(),
        "Read the contents of a file (sandbox-scoped)".into(),
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        }),
        Arc::new(move |_name, args| {
            let toolset = read_sandbox.clone();
            Box::pin(async move { toolset.read_file(args).await })
        }),
    );

    let write_sandbox = sandbox.clone();
    server.register_tool(
        "write_file".into(),
        "Write content to a file (sandbox-scoped, creates or overwrites)".into(),
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        }),
        Arc::new(move |_name, args| {
            let toolset = write_sandbox.clone();
            Box::pin(async move { toolset.write_file(args).await })
        }),
    );

    let run_sandbox = sandbox.clone();
    server.register_tool(
        "run_command".into(),
        "Run a shell command in the sandbox (honors network policy)".into(),
        serde_json::json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        }),
        Arc::new(move |_name, args| {
            let toolset = run_sandbox.clone();
            Box::pin(async move { toolset.run_command(args).await })
        }),
    );

    server
}

/// Run the stdio serve loop until EOF on stdin.
pub async fn serve(project_path: String, network: String) -> Result<()> {
    let mut server = build_server(project_path, network);

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();

    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => handle_request(&mut server, &request).await?,
            Err(e) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: serde_json::Value::Null,
                result: None,
                error: Some(JsonRpcError::parse_error_with(&e.to_string())),
            },
        };
        let out = serde_json::to_string(&response)?;
        stdout.write_all(out.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }

    Ok(())
}

/// Sandbox-backed tool implementations shared by the registered handlers.
struct SandboxToolset {
    project_path: String,
    network: String,
}

impl SandboxToolset {
    fn sandbox(&self) -> Result<Sandbox> {
        Sandbox::new(self.project_path.clone(), self.network.clone())
    }

    async fn read_file(&self, args: serde_json::Value) -> Result<ToolCallResult> {
        let path = arg_str(&args, "path")?;
        match self.sandbox()?.read_file(&path).await {
            Ok(content) => Ok(text_result(content, false)),
            Err(e) => Ok(text_result(e.to_string(), true)),
        }
    }

    async fn write_file(&self, args: serde_json::Value) -> Result<ToolCallResult> {
        let path = arg_str(&args, "path")?;
        let content = arg_str(&args, "content")?;
        match self.sandbox()?.write_file(&path, &content).await {
            Ok(()) => Ok(text_result(
                format!("Wrote {} bytes to {}", content.len(), path),
                false,
            )),
            Err(e) => Ok(text_result(e.to_string(), true)),
        }
    }

    async fn run_command(&self, args: serde_json::Value) -> Result<ToolCallResult> {
        let command = arg_str(&args, "command")?;
        match self.sandbox()?.run_command(&command).await {
            Ok(output) => Ok(text_result(output, false)),
            Err(e) => Ok(text_result(e.to_string(), true)),
        }
    }
}

fn arg_str(args: &serde_json::Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing string argument '{}'", key))
}

fn text_result(text: String, is_error: bool) -> ToolCallResult {
    ToolCallResult {
        content: vec![Content::Text { text }],
        is_error: Some(is_error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_ext::mcp::protocol::{METHOD_INITIALIZE, METHOD_TOOLS_CALL, METHOD_TOOLS_LIST};

    fn request(id: i32, method: &str, params: Option<serde_json::Value>) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: serde_json::json!(id),
            method: method.into(),
            params,
        }
    }

    #[tokio::test]
    async fn initialize_and_list_tools() {
        let mut server = build_server(".".into(), "off".into());

        let init = handle_request(&mut server, &request(1, METHOD_INITIALIZE, None))
            .await
            .unwrap();
        assert!(init.result.is_some());
        assert!(server.is_initialized());

        let list = handle_request(&mut server, &request(2, METHOD_TOOLS_LIST, None))
            .await
            .unwrap();
        let tools = list.result.unwrap();
        let tools = tools.get("tools").unwrap().as_array().unwrap();
        assert_eq!(tools.len(), 3);
    }

    #[tokio::test]
    async fn read_file_tool_returns_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
        let mut server = build_server(dir.path().to_str().unwrap().into(), "off".into());

        let params = serde_json::json!({
            "name": "read_file",
            "arguments": { "path": "hello.txt" }
        });
        let resp = handle_request(&mut server, &request(3, METHOD_TOOLS_CALL, Some(params)))
            .await
            .unwrap();
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("world"));
    }

    #[tokio::test]
    async fn read_file_path_traversal_is_error_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let mut server = build_server(dir.path().to_str().unwrap().into(), "off".into());

        let params = serde_json::json!({
            "name": "read_file",
            "arguments": { "path": "../../etc/passwd" }
        });
        let resp = handle_request(&mut server, &request(4, METHOD_TOOLS_CALL, Some(params)))
            .await
            .unwrap();
        let result = resp.result.unwrap();
        assert_eq!(result["is_error"], serde_json::json!(true));
    }
}
