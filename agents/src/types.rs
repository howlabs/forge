//! Shared types for v0.170.0 (Multi-Agent) and v0.180.0 (Long-Horizon)
//! Frozen before parallel development split

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Unit of work for subagents
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Unique task identifier (UUID)
    pub id: String,
    /// What the subagent should do
    pub prompt: String,
    /// Git worktree path for isolated execution
    pub worktree: PathBuf,
    /// Current state
    pub status: TaskStatus,
    /// When this task was created
    pub created_at: SystemTime,
    /// API key for the provider used by this subagent
    pub api_key: String,
    /// Model name for the provider used by this subagent
    pub model: String,
    /// Provider name (anthropic, openai, zai, openrouter, ...)
    pub provider: String,
    /// Final output or error from the subagent
    pub result: Option<String>,
    /// Number of EventLoop steps completed
    pub steps: usize,
    /// Optional glob patterns limiting which files this agent may touch.
    /// `None` means unrestricted.  Checked at merge-time only (not
    /// enforced at write-time — would need sandbox runtime).
    pub scope: Option<Vec<String>>,
}

impl Task {
    /// Create a new task with a generated UUID
    pub fn new(prompt: impl Into<String>, worktree: PathBuf) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: prompt.into(),
            worktree,
            status: TaskStatus::Pending,
            created_at: SystemTime::now(),
            api_key: String::new(),
            model: "claude-opus-4-5".to_string(),
            provider: "anthropic".to_string(),
            result: None,
            steps: 0,
            scope: None,
        }
    }

    /// Set provider credentials for this task
    pub fn with_provider(mut self, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        self.api_key = api_key.into();
        self.model = model.into();
        self
    }

    /// Set the provider name
    pub fn with_provider_name(mut self, name: impl Into<String>) -> Self {
        self.provider = name.into();
        self
    }

    /// Set file scope globs
    pub fn with_scope(mut self, globs: Vec<String>) -> Self {
        self.scope = Some(globs);
        self
    }

    /// Check if task is in a terminal state (Done or Failed)
    pub fn is_terminal(&self) -> bool {
        matches!(self.status, TaskStatus::Done | TaskStatus::Failed)
    }

    /// Check if task is currently running (Running or Verifying)
    pub fn is_running(&self) -> bool {
        matches!(self.status, TaskStatus::Running | TaskStatus::Verifying)
    }
}

/// Current state of a task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    /// Created but not started
    Pending,
    /// Subagent is executing
    Running,
    /// Post-execution verification in progress
    Verifying,
    /// Completed successfully and passed verification
    Done,
    /// Failed during execution or verification
    Failed,
}

/// Verification result from build + test
#[derive(Debug, Clone)]
pub struct VerifyReport {
    /// Did verification pass?
    pub passed: bool,
    /// Verification output (build + test logs)
    pub logs: String,
    /// How long verification took (milliseconds)
    pub duration_ms: u64,
}

impl VerifyReport {
    /// Create a successful verification report
    pub fn success(logs: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            passed: true,
            logs: logs.into(),
            duration_ms,
        }
    }

    /// Create a failed verification report
    pub fn failure(logs: impl Into<String>, duration_ms: u64) -> Self {
        Self {
            passed: false,
            logs: logs.into(),
            duration_ms,
        }
    }
}

/// Crash recovery payload stored inside a checkpoint file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckpointState {
    pub history: Vec<serde_json::Value>,
    pub worktree_refs: Vec<String>,
    pub last_verify: Option<LastVerify>,
}

/// Last verify result recorded in a checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LastVerify {
    pub passed: bool,
    pub logs: String,
}

/// Crash recovery state for long-horizon tasks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    /// Which task this checkpoint belongs to
    pub task_id: String,
    /// Step number within the task
    pub step: u32,
    /// Structured event-loop state
    pub state: CheckpointState,
    /// When this checkpoint was created
    pub timestamp: SystemTime,
}

impl Checkpoint {
    /// Create a new checkpoint
    pub fn new(task_id: impl Into<String>, step: u32, state: CheckpointState) -> Self {
        Self {
            task_id: task_id.into(),
            step,
            state,
            timestamp: SystemTime::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("Fix the bug", PathBuf::from("/tmp/worktree"));
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(!task.is_terminal());
        assert!(!task.is_running());
    }

    #[test]
    fn test_task_states() {
        let mut task = Task::new("Test", PathBuf::from("/tmp/test"));
        assert!(!task.is_terminal());

        task.status = TaskStatus::Running;
        assert!(task.is_running());
        assert!(!task.is_terminal());

        task.status = TaskStatus::Verifying;
        assert!(task.is_running());

        task.status = TaskStatus::Done;
        assert!(!task.is_running());
        assert!(task.is_terminal());

        task.status = TaskStatus::Failed;
        assert!(task.is_terminal());
    }

    #[test]
    fn test_verify_report() {
        let success = VerifyReport::success("All tests passed", 100);
        assert!(success.passed);

        let failure = VerifyReport::failure("Test failed", 50);
        assert!(!failure.passed);
    }

    #[test]
    fn test_checkpoint_creation() {
        let checkpoint = Checkpoint::new(
            "task-123",
            5,
            CheckpointState {
                history: vec![serde_json::json!({"role": "user", "content": "hi"})],
                worktree_refs: vec!["/tmp/wt".into()],
                last_verify: None,
            },
        );
        assert_eq!(checkpoint.task_id, "task-123");
        assert_eq!(checkpoint.step, 5);
        assert_eq!(checkpoint.state.worktree_refs, vec!["/tmp/wt"]);
    }

    #[test]
    fn test_task_scope() {
        let task = Task::new("Fix bug", PathBuf::from("/tmp"))
            .with_scope(vec!["src/**".into(), "tests/**".into()]);
        assert_eq!(task.scope.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn test_task_serialization_roundtrip() {
        let task = Task::new("test", PathBuf::from("/tmp"))
            .with_provider("key", "model")
            .with_provider_name("openai")
            .with_scope(vec!["src/**".into()]);
        let json = serde_json::to_string(&task).unwrap();
        let loaded: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(task.id, loaded.id);
        assert_eq!(task.prompt, loaded.prompt);
        assert_eq!(task.provider, loaded.provider);
        assert_eq!(task.scope, loaded.scope);
    }
}
