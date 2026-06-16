use anyhow::{Context, Result};
use serde::Serialize;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub project_path: PathBuf,
    pub bind: String,
    pub once: bool,
}

#[derive(Debug, Serialize)]
struct HealthResponse<'a> {
    status: &'a str,
    version: &'a str,
    project_path: String,
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    id: String,
    bytes: u64,
    modified_unix: u64,
}

#[derive(Debug, Serialize)]
struct IndexResponse {
    routes: &'static [&'static str],
}

const ROUTES: &[&str] = &["GET /", "GET /health", "GET /sessions", "GET /models"];

pub fn serve(config: ServeConfig) -> Result<()> {
    let listener = TcpListener::bind(&config.bind)
        .with_context(|| format!("failed to bind Forge local server at {}", config.bind))?;
    println!("Forge local server listening on http://{}", config.bind);
    println!("Routes: {}", ROUTES.join(", "));

    for stream in listener.incoming() {
        let stream = stream?;
        handle_stream(stream, &config.project_path)?;
        if config.once {
            break;
        }
    }
    Ok(())
}

fn handle_stream(mut stream: TcpStream, project_path: &Path) -> Result<()> {
    let mut buffer = [0; 2048];
    let size = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    match path {
        "/" => json_response(&mut stream, 200, &IndexResponse { routes: ROUTES }),
        "/health" => json_response(
            &mut stream,
            200,
            &HealthResponse {
                status: "ok",
                version: env!("CARGO_PKG_VERSION"),
                project_path: project_path.display().to_string(),
            },
        ),
        "/sessions" => json_response(&mut stream, 200, &list_sessions(project_path)?),
        "/models" => json_response(&mut stream, 200, &provider::MODEL_CATALOG),
        _ => text_response(&mut stream, 404, "not found"),
    }
}

fn list_sessions(project_path: &Path) -> Result<Vec<SessionInfo>> {
    let dir = project_path.join(".forge/sessions");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = entry.metadata()?;
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
            sessions.push(SessionInfo {
                id: id.to_string(),
                bytes: metadata.len(),
                modified_unix,
            });
        }
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.modified_unix));
    Ok(sessions)
}

fn json_response<T: Serialize>(stream: &mut TcpStream, status: u16, body: &T) -> Result<()> {
    let body = serde_json::to_string_pretty(body)?;
    write_response(stream, status, "application/json", &body)
}

fn text_response(stream: &mut TcpStream, status: u16, body: &str) -> Result<()> {
    write_response(stream, status, "text/plain; charset=utf-8", body)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "OK",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_sessions_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = list_sessions(dir.path()).unwrap();
        assert!(sessions.is_empty());
    }
}
