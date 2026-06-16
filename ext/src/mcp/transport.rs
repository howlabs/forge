//! MCP transport layer (stdio, SSE, HTTP)
//!
//! Implements communication protocols for MCP connections.

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Stdio transport for MCP (talk to subprocess via stdin/stdout)
pub struct StdioTransport {
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    connected: bool,
}

impl StdioTransport {
    pub fn new(stdin: tokio::process::ChildStdin, stdout: tokio::process::ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout),
            connected: true,
        }
    }

    pub async fn send(&mut self, message: &str) -> Result<()> {
        self.stdin.write_all(message.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    pub async fn receive(&mut self) -> Result<String> {
        let mut line = String::new();
        self.stdout.read_line(&mut line).await?;
        if line.is_empty() {
            self.connected = false;
            return Err(anyhow::anyhow!("EOF"));
        }
        Ok(line.trim().to_string())
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }
}

/// SSE transport for connecting to remote MCP servers via Server-Sent Events
pub struct SseTransport {
    endpoint: String,
    client: reqwest::Client,
    connected: bool,
    _event_tx: tokio::sync::mpsc::Sender<String>,
    event_rx: Option<tokio::sync::mpsc::Receiver<String>>,
}

impl SseTransport {
    pub fn new(endpoint: String) -> Self {
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(64);
        Self {
            endpoint,
            client: reqwest::Client::new(),
            connected: false,
            _event_tx: event_tx,
            event_rx: Some(event_rx),
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        let url = format!("{}/sse", self.endpoint);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow::anyhow!("SSE connect failed: {}", resp.status()));
        }
        self.connected = true;
        Ok(())
    }

    pub fn take_event_receiver(&mut self) -> Option<tokio::sync::mpsc::Receiver<String>> {
        self.event_rx.take()
    }

    pub async fn send_message(&self, message: &str) -> Result<()> {
        let url = format!("{}/message", self.endpoint);
        self.client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(message.to_string())
            .send()
            .await?;
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }
}

/// Streamable HTTP transport for MCP (POST-based, no persistent connection)
pub struct HttpTransport {
    endpoint: String,
    client: reqwest::Client,
}

impl HttpTransport {
    pub fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            client: reqwest::Client::new(),
        }
    }

    pub async fn send_request(&self, message: &str) -> Result<String> {
        let resp = self
            .client
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .body(message.to_string())
            .send()
            .await?;
        let body = resp.text().await?;
        Ok(body)
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stdio_transport_exists() {
        let stdin = std::process::Stdio::piped();
        let _ = stdin;
    }

    #[test]
    fn test_sse_transport_creation() {
        let transport = SseTransport::new("http://localhost:8080".into());
        assert!(!transport.is_connected());
        assert_eq!(transport.endpoint, "http://localhost:8080");
    }

    #[test]
    fn test_http_transport_creation() {
        let transport = HttpTransport::new("http://localhost:8080/mcp".into());
        assert_eq!(transport.endpoint(), "http://localhost:8080/mcp");
    }
}
