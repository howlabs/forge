//! ACP protocol types
//!
//! Agent Control Protocol for editor-agent communication.

use serde::{Deserialize, Serialize};

/// ACP request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpRequest {
    pub id: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// ACP response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpResponse {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AcpError>,
}

/// ACP error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl AcpError {
    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("Method not found: {}", method),
            data: None,
        }
    }
    pub fn invalid_params(msg: &str) -> Self {
        Self {
            code: -32602,
            message: msg.into(),
            data: None,
        }
    }
    pub fn internal_error(msg: &str) -> Self {
        Self {
            code: -32603,
            message: msg.into(),
            data: None,
        }
    }
}

/// ACP notification (no id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcpNotification {
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Editor capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditorCapabilities {
    #[serde(default)]
    pub supports_hover: bool,
    #[serde(default)]
    pub supports_completion: bool,
    #[serde(default)]
    pub supports_diagnostics: bool,
    #[serde(default)]
    pub supports_editing: bool,
    #[serde(default)]
    pub supports_diff: bool,
    #[serde(default)]
    pub supports_selection: bool,
    #[serde(default)]
    pub supports_file_events: bool,
}

/// Agent capabilities
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub supports_chat: bool,
    pub supports_edit: bool,
    pub supports_command: bool,
    pub supports_file_read: bool,
    pub supports_file_write: bool,
    pub supports_code_actions: bool,
    pub supports_diagnostics: bool,
}

/// Chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// File edit operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEdit {
    pub file_path: String,
    pub old_text: String,
    pub new_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Code action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeAction {
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub edit: Option<FileEdit>,
}

/// Diagnostic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
    pub severity: DiagnosticSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
    Hint,
}

/// Initialize params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub editor_name: String,
    pub editor_version: String,
    pub capabilities: EditorCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_path: Option<String>,
}

/// Initialize result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub agent_name: String,
    pub agent_version: String,
    pub capabilities: AgentCapabilities,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Chat request params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatParams {
    pub messages: Vec<ChatMessage>,
    #[serde(alias = "filePath", skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
}

/// Edit request params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditParams {
    #[serde(alias = "filePath")]
    pub file_path: String,
    pub instruction: String,
    #[serde(alias = "oldText", skip_serializing_if = "Option::is_none")]
    pub old_text: Option<String>,
    #[serde(alias = "newText", skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
}

/// Command request params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandParams {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// Read file params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFileParams {
    #[serde(alias = "filePath")]
    pub file_path: String,
    #[serde(alias = "lineStart", skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(alias = "lineEnd", skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
}

/// ACP method constants
pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_INITIALIZED: &str = "notifications/initialized";
pub const METHOD_SHUTDOWN: &str = "shutdown";
pub const METHOD_EXIT: &str = "exit";
pub const METHOD_CHAT: &str = "chat";
pub const METHOD_EDIT: &str = "edit";
pub const METHOD_COMMAND: &str = "command";
pub const METHOD_READ_FILE: &str = "readFile";
pub const METHOD_GET_DIAGNOSTICS: &str = "getDiagnostics";
pub const METHOD_GET_CODE_ACTIONS: &str = "getCodeActions";
pub const METHOD_CANCEL: &str = "cancel";
pub const METHOD_PING: &str = "ping";

pub const NOTIFICATION_DIAGNOSTICS_CHANGED: &str = "notifications/diagnosticsChanged";
pub const NOTIFICATION_FILE_CHANGED: &str = "notifications/fileChanged";
pub const NOTIFICATION_PROGRESS: &str = "notifications/progress";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acp_request_serialization() {
        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_CHAT.into(),
            params: Some(serde_json::json!({"messages": []})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("chat"));
    }

    #[test]
    fn test_acp_response_serialization() {
        let resp = AcpResponse {
            id: "1".into(),
            result: Some(serde_json::json!({"content": "hello"})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("hello"));
    }

    #[test]
    fn test_acp_error_codes() {
        let e = AcpError::method_not_found("foo");
        assert_eq!(e.code, -32601);
        let e = AcpError::invalid_params("bad");
        assert_eq!(e.code, -32602);
    }

    #[test]
    fn test_editor_capabilities_defaults() {
        let caps = EditorCapabilities::default();
        assert!(!caps.supports_hover);
        assert!(!caps.supports_completion);
    }

    #[test]
    fn test_agent_capabilities_defaults() {
        let caps = AgentCapabilities::default();
        assert!(!caps.supports_chat);
        assert!(!caps.supports_edit);
    }

    #[test]
    fn test_chat_message_serialization() {
        let msg = ChatMessage {
            role: Role::User,
            content: "hello".into(),
            file_path: Some("src/main.rs".into()),
            line: Some(10),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"user\""));
        assert!(json.contains("main.rs"));
    }

    #[test]
    fn test_file_edit_serialization() {
        let edit = FileEdit {
            file_path: "src/main.rs".into(),
            old_text: "fn main() {}".into(),
            new_text: "fn main() {\n    println!(\"hi\");\n}".into(),
            line: Some(1),
        };
        let json = serde_json::to_string(&edit).unwrap();
        assert!(json.contains("old_text"));
    }

    #[test]
    fn test_diagnostic_serialization() {
        let diag = Diagnostic {
            file_path: "src/main.rs".into(),
            line: 5,
            column: 10,
            message: "unused variable".into(),
            severity: DiagnosticSeverity::Warning,
        };
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("\"warning\""));
    }

    #[test]
    fn test_initialize_result() {
        let result = InitializeResult {
            agent_name: "forge".into(),
            agent_version: "0.100.0".into(),
            capabilities: AgentCapabilities {
                supports_chat: true,
                supports_edit: true,
                supports_command: true,
                supports_file_read: true,
                supports_file_write: true,
                ..Default::default()
            },
            instructions: Some("I am Forge".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("forge"));
    }
}
