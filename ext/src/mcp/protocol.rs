//! MCP protocol types (JSON-RPC 2.0)
//!
//! Full Model Context Protocol specification types for
//! client-server communication between AI agents.

use serde::{Deserialize, Serialize};

// ──────────────────────────────────────────────
// JSON-RPC 2.0 base types
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcError {
    pub fn parse_error() -> Self {
        Self {
            code: -32700,
            message: "Parse error".into(),
            data: None,
        }
    }
    pub fn parse_error_with(msg: &str) -> Self {
        Self {
            code: -32700,
            message: format!("Parse error: {}", msg),
            data: None,
        }
    }
    pub fn invalid_request() -> Self {
        Self {
            code: -32600,
            message: "Invalid Request".into(),
            data: None,
        }
    }
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

/// JSON-RPC notification (request with no id)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl JsonRpcNotification {
    pub fn new(method: &str, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

// ──────────────────────────────────────────────
// MCP capability negotiation
// ──────────────────────────────────────────────

/// Server capabilities advertised during initialize
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourcesCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logging: Option<LoggingCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourcesCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingCapability {}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SamplingCapability {}

/// Client capabilities advertised during initialize
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RootsCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Initialize request params
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeParams {
    pub protocol_version: String,
    pub capabilities: ClientCapabilities,
    pub client_info: Implementation,
}

/// Initialize result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: Implementation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Implementation {
    pub name: String,
    pub version: String,
}

// ──────────────────────────────────────────────
// Tools
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallParams {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub content: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

// ──────────────────────────────────────────────
// Resources
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceTemplate {
    pub uri_template: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContents {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResourceParams {
    pub uri: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContents>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscribeParams {
    pub uri: String,
}

// ──────────────────────────────────────────────
// Prompts
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<PromptArgument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetPromptParams {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetPromptResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptMessage {
    pub role: Role,
    pub content: Content,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

// ──────────────────────────────────────────────
// Content types
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Content {
    Text { text: String },
    Image { data: String, mime_type: String },
    Resource { resource: ResourceContents },
}

// ──────────────────────────────────────────────
// Logging
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingMessageParams {
    pub level: LogLevel,
    pub logger: Option<String>,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetLevelParams {
    pub level: LogLevel,
}

// ──────────────────────────────────────────────
// Sampling (server requests LLM from client)
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageParams {
    pub messages: Vec<SamplingMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_preferences: Option<ModelPreferences>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplingMessage {
    pub role: Role,
    pub content: Content,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPreferences {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hints: Option<Vec<ModelHint>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_priority: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intelligence_priority: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelHint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMessageResult {
    pub model: String,
    pub role: Role,
    pub content: Content,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

// ──────────────────────────────────────────────
// Roots (client filesystem roots)
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Root {
    pub uri: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

// ──────────────────────────────────────────────
// Notifications
// ──────────────────────────────────────────────

pub const NOTIFICATION_CANCELLED: &str = "notifications/cancelled";
pub const NOTIFICATION_PROGRESS: &str = "notifications/progress";
pub const NOTIFICATION_RESOURCE_UPDATED: &str = "notifications/resources/updated";
pub const NOTIFICATION_RESOURCE_LIST_CHANGED: &str = "notifications/resources/list_changed";
pub const NOTIFICATION_PROMPT_LIST_CHANGED: &str = "notifications/prompts/list_changed";
pub const NOTIFICATION_TOOL_LIST_CHANGED: &str = "notifications/tools/list_changed";
pub const NOTIFICATION_LOGGING_MESSAGE: &str = "notifications/message";
pub const NOTIFICATION_ROOTS_LIST_CHANGED: &str = "notifications/roots/list_changed";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressParams {
    pub progress_token: serde_json::Value,
    pub progress: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelledParams {
    pub request_id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUpdatedParams {
    pub uri: String,
}

// ──────────────────────────────────────────────
// Ping
// ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingParams {}

// ──────────────────────────────────────────────
// JSON-RPC method constants
// ──────────────────────────────────────────────

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_INITIALIZED: &str = "notifications/initialized";
pub const METHOD_PING: &str = "ping";
pub const METHOD_TOOLS_LIST: &str = "tools/list";
pub const METHOD_TOOLS_CALL: &str = "tools/call";
pub const METHOD_RESOURCES_LIST: &str = "resources/list";
pub const METHOD_RESOURCES_TEMPLATES_LIST: &str = "resources/templates/list";
pub const METHOD_RESOURCES_READ: &str = "resources/read";
pub const METHOD_RESOURCES_SUBSCRIBE: &str = "resources/subscribe";
pub const METHOD_RESOURCES_UNSUBSCRIBE: &str = "resources/unsubscribe";
pub const METHOD_PROMPTS_LIST: &str = "prompts/list";
pub const METHOD_PROMPTS_GET: &str = "prompts/get";
pub const METHOD_LOGGING_SET_LEVEL: &str = "logging/setLevel";
pub const METHOD_SAMPLING_CREATE_MESSAGE: &str = "sampling/createMessage";
pub const METHOD_ROOTS_LIST: &str = "roots/list";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_jsonrpc_request() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.jsonrpc, "2.0");
    }

    #[test]
    fn test_serialize_jsonrpc_response() {
        let resp = JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: serde_json::json!(1),
            result: Some(serde_json::json!({"tools": []})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("tools"));
    }

    #[test]
    fn test_mcp_tool_serialization() {
        let tool = McpTool {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("test_tool"));
    }

    #[test]
    fn test_mcp_resource_serialization() {
        let resource = McpResource {
            uri: "file:///test.txt".to_string(),
            name: "test.txt".to_string(),
            description: Some("A test file".to_string()),
            mime_type: Some("text/plain".to_string()),
        };
        let json = serde_json::to_string(&resource).unwrap();
        assert!(json.contains("test.txt"));
    }

    #[test]
    fn test_jsonrpc_error_codes() {
        let e = JsonRpcError::parse_error();
        assert_eq!(e.code, -32700);
        let e = JsonRpcError::method_not_found("foo");
        assert_eq!(e.code, -32601);
        assert!(e.message.contains("foo"));
    }

    #[test]
    fn test_notification_serialization() {
        let n = JsonRpcNotification::new("notifications/tools/list_changed", None);
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.contains("tools/list_changed"));
        assert!(!json.contains("id"));
    }

    #[test]
    fn test_content_variants() {
        let text = Content::Text {
            text: "hello".into(),
        };
        let json = serde_json::to_string(&text).unwrap();
        assert!(json.contains("\"text\""));

        let img = Content::Image {
            data: "base64data".into(),
            mime_type: "image/png".into(),
        };
        let json = serde_json::to_string(&img).unwrap();
        assert!(json.contains("\"image\""));
    }

    #[test]
    fn test_server_capabilities_serialization() {
        let caps = ServerCapabilities {
            tools: Some(ToolsCapability {
                list_changed: Some(true),
            }),
            resources: Some(ResourcesCapability {
                subscribe: Some(true),
                list_changed: None,
            }),
            prompts: None,
            logging: Some(LoggingCapability {}),
            sampling: None,
        };
        let json = serde_json::to_value(&caps).unwrap();
        assert!(json.get("tools").is_some());
        assert!(json.get("resources").is_some());
        assert!(json.get("prompts").is_none());
    }

    #[test]
    fn test_initialize_result_roundtrip() {
        let result = InitializeResult {
            protocol_version: "2024-11-05".into(),
            capabilities: ServerCapabilities::default(),
            server_info: Implementation {
                name: "forge".into(),
                version: "0.100.0".into(),
            },
            instructions: Some("I am Forge".into()),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: InitializeResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.server_info.name, "forge");
    }

    #[test]
    fn test_prompt_message_serialization() {
        let msg = PromptMessage {
            role: Role::User,
            content: Content::Text {
                text: "hello".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"user\""));
    }

    #[test]
    fn test_log_level_serialization() {
        let json = serde_json::to_string(&LogLevel::Warning).unwrap();
        assert_eq!(json, "\"warning\"");
    }

    #[test]
    fn test_sampling_message_roundtrip() {
        let msg = SamplingMessage {
            role: Role::Assistant,
            content: Content::Text {
                text: "response".into(),
            },
        };
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: SamplingMessage = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed.role, Role::Assistant));
    }

    #[test]
    fn test_progress_notification() {
        let params = ProgressParams {
            progress_token: serde_json::json!("token-1"),
            progress: 50,
            total: Some(100),
        };
        let n = JsonRpcNotification::new(
            NOTIFICATION_PROGRESS,
            Some(serde_json::to_value(params).unwrap()),
        );
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.contains("progress"));
    }

    #[test]
    fn test_cancelled_params() {
        let params = CancelledParams {
            request_id: serde_json::json!(42),
            reason: Some("user cancelled".into()),
        };
        let json = serde_json::to_value(params).unwrap();
        assert_eq!(json["request_id"], 42);
    }
}
