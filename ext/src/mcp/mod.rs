//! Model Context Protocol (MCP) implementation
//!
//! Provides MCP client and server functionality for connecting
//! to external MCP servers and exposing Forge's own tools.

pub mod client;
pub mod handlers;
pub mod protocol;
pub mod server;
pub mod server_process;
pub mod transport;

pub use client::McpClient;
pub use handlers::handle_request;
pub use protocol::{JsonRpcRequest, JsonRpcResponse, JsonRpcError, McpTool, McpResource};
pub use server::McpServer;
pub use server_process::ServerProcess;
pub use transport::StdioTransport;
