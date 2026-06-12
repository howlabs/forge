//! Hooks system types
//!
//! Defines lifecycle events that trigger hook scripts.

use serde::{Deserialize, Serialize};

/// Lifecycle event that triggers hooks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum HookEvent {
    TaskCreated(TaskCreatedEvent),
    TaskCompleted(TaskCompletedEvent),
    PreEdit(PreEditEvent),
    PostVerify(PostVerifyEvent),
}

/// Event fired when a new task is created
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCreatedEvent {
    pub task_id: String,
    pub description: String,
}

/// Event fired when a task completes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompletedEvent {
    pub task_id: String,
    pub status: String, // "success" | "failed"
    pub duration_ms: u64,
}

/// Event fired before making an edit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreEditEvent {
    pub task_id: String,
    pub file_path: String,
    pub edit_description: String,
}

/// Event fired after verification runs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostVerifyEvent {
    pub task_id: String,
    pub verify_result: String, // "pass" | "fail"
    pub test_output: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_created_event() {
        let event = TaskCreatedEvent {
            task_id: "test-123".to_string(),
            description: "Test task".to_string(),
        };
        assert_eq!(event.task_id, "test-123");
        assert_eq!(event.description, "Test task");
    }

    #[test]
    fn test_task_completed_event() {
        let event = TaskCompletedEvent {
            task_id: "test-456".to_string(),
            status: "success".to_string(),
            duration_ms: 1500,
        };
        assert_eq!(event.status, "success");
        assert_eq!(event.duration_ms, 1500);
    }

    #[test]
    fn test_pre_edit_event() {
        let event = PreEditEvent {
            task_id: "test-789".to_string(),
            file_path: "/path/to/file.rs".to_string(),
            edit_description: "Fix bug".to_string(),
        };
        assert_eq!(event.file_path, "/path/to/file.rs");
    }

    #[test]
    fn test_post_verify_event() {
        let event = PostVerifyEvent {
            task_id: "test-abc".to_string(),
            verify_result: "pass".to_string(),
            test_output: "All tests passed".to_string(),
        };
        assert_eq!(event.verify_result, "pass");
    }

    #[test]
    fn test_hook_event_serialization() {
        let event = HookEvent::TaskCreated(TaskCreatedEvent {
            task_id: "abc".to_string(),
            description: "Do something".to_string(),
        });

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("TaskCreated"));
        assert!(json.contains("abc"));
    }

    #[test]
    fn test_hook_event_deserialization() {
        let json = r#"{"type":"TaskCreated","task_id":"xyz","description":"Test"}"#;
        let event: HookEvent = serde_json::from_str(json).unwrap();

        match event {
            HookEvent::TaskCreated(e) => {
                assert_eq!(e.task_id, "xyz");
                assert_eq!(e.description, "Test");
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_all_hook_events_serializable() {
        let events = vec![
            HookEvent::TaskCreated(TaskCreatedEvent {
                task_id: "1".to_string(),
                description: "Test".to_string(),
            }),
            HookEvent::TaskCompleted(TaskCompletedEvent {
                task_id: "2".to_string(),
                status: "success".to_string(),
                duration_ms: 1000,
            }),
            HookEvent::PreEdit(PreEditEvent {
                task_id: "3".to_string(),
                file_path: "/test.rs".to_string(),
                edit_description: "Edit".to_string(),
            }),
            HookEvent::PostVerify(PostVerifyEvent {
                task_id: "4".to_string(),
                verify_result: "pass".to_string(),
                test_output: "OK".to_string(),
            }),
        ];

        for event in events {
            let json = serde_json::to_string(&event).unwrap();
            let _deserialized: HookEvent = serde_json::from_str(&json).unwrap();
        }
    }
}
