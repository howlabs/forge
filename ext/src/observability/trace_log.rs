//! Trace log export
//!
//! Exports structured trace logs as JSON for debugging and observability.

use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Trace log for a Forge run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceLog {
    pub task_id: String,
    pub events: Vec<TraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    pub timestamp: String,
    pub event_type: String,
    pub data: serde_json::Value,
}

impl TraceLog {
    /// Create new trace log
    pub fn new(task_id: impl Into<String>) -> Self {
        Self {
            task_id: task_id.into(),
            events: Vec::new(),
        }
    }

    /// Add event to trace log
    pub fn add_event(&mut self, event_type: &str, data: serde_json::Value) {
        let event = TraceEvent {
            timestamp: chrono::Utc::now().to_rfc3339().to_string(),
            event_type: event_type.to_string(),
            data,
        };
        self.events.push(event);
    }

    /// Write trace log to file as JSON
    pub fn write_to_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    /// Get number of events
    pub fn event_count(&self) -> usize {
        self.events.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_log_creation() {
        let log = TraceLog::new("test-task");
        assert_eq!(log.task_id, "test-task");
        assert_eq!(log.event_count(), 0);
    }

    #[test]
    fn test_trace_log_add_event() {
        let mut log = TraceLog::new("test-task");
        log.add_event("step_completed", serde_json::json!({"step": "test"}));
        assert_eq!(log.event_count(), 1);
    }

    #[test]
    fn test_trace_log_serialization() {
        let mut log = TraceLog::new("test-task");
        log.add_event("test_event", serde_json::json!({"key": "value"}));

        let json = serde_json::to_string(&log).unwrap();
        assert!(json.contains("test-task"));
        assert!(json.contains("test_event"));
    }

    #[test]
    fn test_trace_log_write_to_file() {
        let mut log = TraceLog::new("test-task");
        log.add_event("test", serde_json::json!({}));

        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("trace.json");

        let result = log.write_to_file(&file_path);
        assert!(result.is_ok());

        // Verify file was created and contains valid JSON
        let content = std::fs::read_to_string(&file_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["task_id"], "test-task");
    }
}
