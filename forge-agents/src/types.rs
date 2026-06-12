//! Shared types for v0.170.0 (Multi-Agent) and v0.180.0 (Long-Horizon)
//! Frozen before parallel development split

use std::path::PathBuf;
use std::time::SystemTime;

/// Unit of work for subagents
#[derive(Debug, Clone, PartialEq)]
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
        }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Crash recovery state for long-horizon tasks
#[derive(Debug, Clone)]
pub struct Checkpoint {
    /// Which task this checkpoint belongs to
    pub task_id: String,
    /// Step number within the task
    pub step: u32,
    /// Serialized state (bincode or JSON)
    pub state: Vec<u8>,
    /// When this checkpoint was created
    pub timestamp: SystemTime,
}

impl Checkpoint {
    /// Create a new checkpoint
    pub fn new(task_id: impl Into<String>, step: u32, state: Vec<u8>) -> Self {
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
        let checkpoint = Checkpoint::new("task-123", 5, vec![1, 2, 3]);
        assert_eq!(checkpoint.task_id, "task-123");
        assert_eq!(checkpoint.step, 5);
        assert_eq!(checkpoint.state, vec![1, 2, 3]);
    }
}
