//! Model Context Protocol (MCP) implementation
//!
//! Full MCP client and server functionality for connecting
//! to external MCP servers and exposing Forge's own tools,
//! resources, and prompts.

pub mod client;
pub mod handlers;
pub mod oauth;
pub mod protocol;
pub mod server;
pub mod server_process;
pub mod transport;

pub use client::McpClient;
pub use handlers::handle_request;
pub use oauth::{OAuth2Client, OAuth2Config, OAuth2Token};
pub use protocol::{
    ClientCapabilities, Content, CreateMessageParams, CreateMessageResult, Implementation,
    JsonRpcError, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, LogLevel,
    McpPrompt, McpResource, McpTool, Root, ResourcesCapability, Role,
    ServerCapabilities, ToolCallResult, ToolCallParams, ToolsCapability,
    NOTIFICATION_CANCELLED, NOTIFICATION_LOGGING_MESSAGE, NOTIFICATION_PROGRESS,
    NOTIFICATION_RESOURCE_LIST_CHANGED, NOTIFICATION_RESOURCE_UPDATED,
    NOTIFICATION_TOOL_LIST_CHANGED, NOTIFICATION_PROMPT_LIST_CHANGED,
    METHOD_INITIALIZE, METHOD_INITIALIZED, METHOD_PING,
    METHOD_TOOLS_LIST, METHOD_TOOLS_CALL,
    METHOD_RESOURCES_LIST, METHOD_RESOURCES_READ, METHOD_RESOURCES_TEMPLATES_LIST,
    METHOD_RESOURCES_SUBSCRIBE, METHOD_RESOURCES_UNSUBSCRIBE,
    METHOD_PROMPTS_LIST, METHOD_PROMPTS_GET,
    METHOD_LOGGING_SET_LEVEL, METHOD_SAMPLING_CREATE_MESSAGE, METHOD_ROOTS_LIST,
};
pub use server::{McpServer, SharedMcpServer, shared_server, ToolHandler};
pub use server_process::ServerProcess;
pub use transport::{StdioTransport, SseTransport, HttpTransport};
