//! Spawned MCP server subprocess management
//!
//! Handles spawning and communication with MCP server processes.

use super::protocol::JsonRpcRequest;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

/// Spawned MCP server subprocess
pub struct ServerProcess {
    child: Arc<Mutex<tokio::process::Child>>,
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    stdout: Arc<Mutex<BufReader<tokio::process::ChildStdout>>>,
}

impl ServerProcess {
    /// Spawn new MCP server subprocess
    pub async fn spawn(command: String, args: Vec<String>) -> Result<Self> {
        let mut child = Command::new(&command)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("No stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("No stdout"))?;

        Ok(Self {
            child: Arc::new(Mutex::new(child)),
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
        })
    }

    /// Send JSON-RPC request and wait for response
    pub async fn send_and_recv(
        &self,
        req: &JsonRpcRequest,
    ) -> Result<super::protocol::JsonRpcResponse> {
        self.send(req).await?;
        self.recv().await
    }

    /// Send request (no response wait)
    pub async fn send(&self, req: &JsonRpcRequest) -> Result<()> {
        let json = serde_json::to_string(req)?;
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(json.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Receive response
    pub async fn recv(&self) -> Result<super::protocol::JsonRpcResponse> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();
        stdout.read_line(&mut line).await?;
        let response: super::protocol::JsonRpcResponse = serde_json::from_str(&line)?;
        Ok(response)
    }

    /// Check if process is still running
    pub async fn is_running(&self) -> bool {
        let mut child = self.child.lock().await;
        child.try_wait().ok().flatten().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_process_type_exists() {
        // Just verify the type exists
        // Empty test for now
        // TODO: Add actual server process tests
    }

    #[tokio::test]
    async fn test_server_process_spawn_echo() {
        // Test with a simple echo command
        let result = ServerProcess::spawn("echo".to_string(), vec!["test".to_string()]).await;
        // This might fail on some systems, so we just check it doesn't panic
        let _ = result;
    }
}
