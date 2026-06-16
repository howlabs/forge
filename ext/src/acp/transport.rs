//! ACP transport layer

use super::protocol::*;
use anyhow::Result;

/// ACP transport trait
#[async_trait::async_trait]
pub trait AcpTransport: Send + Sync {
    async fn send_request(&self, request: &AcpRequest) -> Result<AcpResponse>;
    async fn send_notification(&self, notification: &AcpNotification) -> Result<()>;
    async fn receive(&self) -> Result<AcpRequest>;
}

/// Stdio-based ACP transport
pub struct StdioAcpTransport {
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    stdout: Arc<Mutex<BufReader<tokio::process::ChildStdout>>>,
}

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

impl StdioAcpTransport {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
        }
    }

    pub async fn send_raw(&self, message: &str) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(message.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn receive_raw(&self) -> Result<String> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();
        stdout.read_line(&mut line).await?;
        Ok(line.trim().to_string())
    }
}

#[async_trait::async_trait]
impl AcpTransport for StdioAcpTransport {
    async fn send_request(&self, request: &AcpRequest) -> Result<AcpResponse> {
        let json = serde_json::to_string(request)?;
        self.send_raw(&json).await?;
        let response_json = self.receive_raw().await?;
        Ok(serde_json::from_str(&response_json)?)
    }

    async fn send_notification(&self, notification: &AcpNotification) -> Result<()> {
        let json = serde_json::to_string(notification)?;
        self.send_raw(&json).await
    }

    async fn receive(&self) -> Result<AcpRequest> {
        let json = self.receive_raw().await?;
        Ok(serde_json::from_str(&json)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_acp_notification_serialization() {
        let notif = AcpNotification {
            method: NOTIFICATION_DIAGNOSTICS_CHANGED.into(),
            params: Some(serde_json::json!({"diagnostics": []})),
        };
        let json = serde_json::to_string(&notif).unwrap();
        assert!(json.contains("diagnosticsChanged"));
    }

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

    #[tokio::test]
    async fn test_stdio_transport_send_receive() {
        let proc = tokio::process::Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();

        let stdin = proc.stdin.unwrap();
        let stdout = proc.stdout.unwrap();
        let transport = StdioAcpTransport::new(stdin, stdout);

        let req = AcpRequest {
            id: "1".into(),
            method: METHOD_PING.into(),
            params: None,
        };
        transport
            .send_raw(&serde_json::to_string(&req).unwrap())
            .await
            .unwrap();
        let received = transport.receive_raw().await.unwrap();
        assert!(received.contains("ping"));
    }
}
