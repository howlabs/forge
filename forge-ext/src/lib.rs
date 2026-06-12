//! Extension system (v0.190.0)
//!
//! This crate will handle:
//! - MCP client/server
//! - Hooks system
//! - Skills framework
//! - Headless execution (forge exec)
//! - Multi-provider support
//! - Observability

pub mod hooks;
pub mod mcp;
pub mod skills;

#[cfg(test)]
mod tests {
    #[test]
    fn test_placeholder() {
        // Placeholder test until v0.190.0
        assert!(true);
    }
}

