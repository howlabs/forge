//! ACP request handlers - real implementation

use super::protocol::*;
use super::server::AcpServer;
use anyhow::Result;

pub async fn handle_acp_request(
    server: &mut AcpServer,
    request: &AcpRequest,
) -> Result<AcpResponse> {
    match request.method.as_str() {
        METHOD_INITIALIZE => handle_initialize(server, request),
        METHOD_SHUTDOWN => handle_shutdown(request),
        METHOD_PING => handle_ping(request),
        METHOD_CHAT => handle_chat(server, request).await,
        METHOD_EDIT => handle_edit(server, request),
        METHOD_COMMAND => handle_command(server, request).await,
        METHOD_READ_FILE => handle_read_file(server, request),
        METHOD_GET_DIAGNOSTICS => handle_get_diagnostics(server, request),
        METHOD_GET_CODE_ACTIONS => handle_get_code_actions(server, request),
        METHOD_CANCEL => handle_cancel(request),
        _ => Ok(error_response(
            &request.id,
            AcpError::method_not_found(&request.method),
        )),
    }
}

fn error_response(id: &str, error: AcpError) -> AcpResponse {
    AcpResponse {
        id: id.into(),
        result: None,
        error: Some(error),
    }
}

fn success_response(id: &str, result: serde_json::Value) -> AcpResponse {
    AcpResponse {
        id: id.into(),
        result: Some(result),
        error: None,
    }
}

fn handle_initialize(server: &mut AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    if let Some(params) = &request.params {
        if let Some(capabilities) = params.get("capabilities") {
            let caps: EditorCapabilities = serde_json::from_value(capabilities.clone())?;
            server.editor_capabilities = Some(caps);
        }
        if let Some(root) = params.get("rootPath").and_then(|v| v.as_str()) {
            server.root_path = Some(root.into());
        }
    }
    server.set_initialized(true);
    Ok(success_response(
        &request.id,
        serde_json::to_value(server.agent_info().clone())?,
    ))
}

fn handle_shutdown(request: &AcpRequest) -> Result<AcpResponse> {
    Ok(success_response(&request.id, serde_json::json!({})))
}

fn handle_ping(request: &AcpRequest) -> Result<AcpResponse> {
    Ok(success_response(&request.id, serde_json::json!({})))
}

async fn handle_chat(server: &AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    let params: ChatParams = match request.params.as_ref() {
        Some(p) => serde_json::from_value(p.clone())?,
        None => {
            return Ok(error_response(
                &request.id,
                AcpError::invalid_params("Missing params"),
            ))
        }
    };

    match server.chat_with_model(&params.messages).await {
        Ok(content) => Ok(success_response(
            &request.id,
            serde_json::json!({
                "role": "assistant",
                "content": content
            }),
        )),
        Err(e) => Ok(error_response(
            &request.id,
            AcpError::internal_error(&e.to_string()),
        )),
    }
}

fn handle_edit(server: &AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    let params: EditParams = match request.params.as_ref() {
        Some(p) => serde_json::from_value(p.clone())?,
        None => {
            return Ok(error_response(
                &request.id,
                AcpError::invalid_params("Missing params"),
            ))
        }
    };

    if let (Some(old), Some(new)) = (&params.old_text, &params.new_text) {
        match server.apply_edit(&params.file_path, old, new) {
            Ok(true) => Ok(success_response(
                &request.id,
                serde_json::json!({
                    "applied": true,
                    "file_path": params.file_path
                }),
            )),
            Ok(false) => Ok(error_response(
                &request.id,
                AcpError::invalid_params("Old text not found"),
            )),
            Err(e) => Ok(error_response(
                &request.id,
                AcpError::internal_error(&e.to_string()),
            )),
        }
    } else {
        Ok(error_response(
            &request.id,
            AcpError::invalid_params("Missing oldText/newText"),
        ))
    }
}

async fn handle_command(server: &AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    let params: CommandParams = match request.params.as_ref() {
        Some(p) => serde_json::from_value(p.clone())?,
        None => {
            return Ok(error_response(
                &request.id,
                AcpError::invalid_params("Missing params"),
            ))
        }
    };

    match server.run_command(&params.command, &params.args).await {
        Ok(output) => Ok(success_response(
            &request.id,
            serde_json::json!({
                "stdout": output.stdout,
                "stderr": output.stderr,
                "exitCode": output.exit_code
            }),
        )),
        Err(e) => Ok(error_response(
            &request.id,
            AcpError::internal_error(&e.to_string()),
        )),
    }
}

fn handle_read_file(server: &AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    let params: ReadFileParams = match request.params.as_ref() {
        Some(p) => serde_json::from_value(p.clone())?,
        None => {
            return Ok(error_response(
                &request.id,
                AcpError::invalid_params("Missing params"),
            ))
        }
    };

    match server.read_file_content(&params.file_path, params.line_start, params.line_end) {
        Ok(content) => Ok(success_response(
            &request.id,
            serde_json::json!({
                "content": content
            }),
        )),
        Err(e) => Ok(error_response(
            &request.id,
            AcpError::internal_error(&e.to_string()),
        )),
    }
}

fn handle_get_diagnostics(server: &AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    let diagnostics = server.get_diagnostics();
    Ok(success_response(
        &request.id,
        serde_json::json!({
            "diagnostics": diagnostics
        }),
    ))
}

fn handle_get_code_actions(_server: &AcpServer, request: &AcpRequest) -> Result<AcpResponse> {
    Ok(success_response(
        &request.id,
        serde_json::json!({ "actions": [] }),
    ))
}

fn handle_cancel(request: &AcpRequest) -> Result<AcpResponse> {
    Ok(success_response(
        &request.id,
        serde_json::json!({ "cancelled": true }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_handle_initialize() {
        let mut server = AcpServer::new("forge", "0.100.0");
        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_INITIALIZE.into(),
            params: Some(serde_json::json!({
                "editorName": "zed",
                "editorVersion": "0.1.0",
                "capabilities": { "supportsHover": true },
                "rootPath": "."
            })),
        };
        let resp = handle_acp_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
        assert!(server.is_initialized());
    }

    #[tokio::test]
    async fn test_handle_command() {
        let mut server = AcpServer::new("forge", "0.100.0");
        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_COMMAND.into(),
            params: Some(serde_json::json!({
                "command": "echo",
                "args": ["hello"]
            })),
        };
        let resp = handle_acp_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert!(result["stdout"].as_str().unwrap().contains("hello"));
        assert_eq!(result["exitCode"], 0);
    }

    #[tokio::test]
    async fn test_handle_read_file() {
        let mut server = AcpServer::new("forge", "0.100.0").with_root_path(".");
        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_READ_FILE.into(),
            params: Some(serde_json::json!({
                "filePath": "Cargo.toml"
            })),
        };
        let resp = handle_acp_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
        let result = resp.result.unwrap();
        assert!(result["content"].as_str().unwrap().contains("[package]"));
    }

    #[tokio::test]
    async fn test_handle_get_diagnostics() {
        let mut server = AcpServer::new("forge", "0.100.0");
        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_GET_DIAGNOSTICS.into(),
            params: None,
        };
        let resp = handle_acp_request(&mut server, &req).await.unwrap();
        assert!(resp.result.is_some());
    }

    #[tokio::test]
    async fn test_handle_unknown_method() {
        let mut server = AcpServer::new("forge", "0.100.0");
        let req = AcpRequest {
            id: "1".into(),
            method: "unknown".into(),
            params: None,
        };
        let resp = handle_acp_request(&mut server, &req).await.unwrap();
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }
}
