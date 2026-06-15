//! Agent Control Protocol (ACP) support
//!
//! ACP is a protocol for editor integration with coding agents.
//! Enables Zed, JetBrains, Neovim, Emacs to communicate with Forge.

pub mod handlers;
pub mod protocol;
pub mod server;
pub mod transport;

pub use protocol::*;
pub use server::AcpServer;
pub use transport::{AcpTransport, StdioAcpTransport};
