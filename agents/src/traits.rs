//! Core traits for v0.170.0 (Multi-Agent) and v0.180.0 (Long-Horizon)
//! Frozen before parallel development split

use crate::types::{Task, TaskStatus, VerifyReport, Checkpoint};
use anyhow::Result;
use async_trait::async_trait;
use std::path::Path;

/// Orchestrator - Spawn and manage subagents
#[async_trait]
pub trait Orchestrator: Send + Sync {
    /// Spawn a new subagent to execute a task
    /// Creates isolated git worktree, launches subagent event-loop
    async fn spawn(&mut self, task: Task) -> Result<()>;

    /// Wait for all running subagents to complete
    /// Returns final states of all tasks
    async fn join_all(&mut self) -> Result<Vec<Task>>;

    /// Get current task status without blocking
    fn get_task_status(&self, task_id: &str) -> Option<TaskStatus>;

    /// Cancel a running task
    async fn cancel_task(&mut self, task_id: &str) -> Result<()>;
}

/// Verifier - Build and test verification
#[async_trait]
pub trait Verifier: Send + Sync {
    /// Run build + tests in a worktree
    /// Returns verification report with pass/fail and logs
    async fn verify(&self, workdir: &Path) -> Result<VerifyReport>;

    /// Quick check if workdir looks buildable
    /// Returns early if obvious issues (missing Cargo.toml, etc.)
    async fn quick_check(&self, workdir: &Path) -> Result<bool>;
}

/// CheckpointStore - Crash-safe state persistence
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Save checkpoint for a task step
    async fn save(&self, checkpoint: &Checkpoint) -> Result<()>;

    /// Load latest checkpoint for a task
    /// Returns None if no checkpoint exists
    async fn load(&self, task_id: &str) -> Result<Option<Checkpoint>>;

    /// List all checkpointed tasks
    async fn list_tasks(&self) -> Result<Vec<String>>;

    /// Delete all checkpoints for a task
    async fn delete(&self, task_id: &str) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // Mock implementations for testing
    struct MockOrchestrator {
        tasks: Vec<Task>,
    }

    #[async_trait]
    impl Orchestrator for MockOrchestrator {
        async fn spawn(&mut self, task: Task) -> Result<()> {
            self.tasks.push(task);
            Ok(())
        }

        async fn join_all(&mut self) -> Result<Vec<Task>> {
            Ok(self.tasks.drain(..).collect())
        }

        fn get_task_status(&self, task_id: &str) -> Option<TaskStatus> {
            self.tasks.iter().find(|t| t.id == task_id).map(|t| t.status)
        }

        async fn cancel_task(&mut self, task_id: &str) -> Result<()> {
            if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
                task.status = TaskStatus::Failed;
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_orchestrator_spawn() {
        let mut orchestrator = MockOrchestrator { tasks: vec![] };
        let task = Task::new("Test task", PathBuf::from("/tmp/test"));

        orchestrator.spawn(task).await.unwrap();
        assert_eq!(orchestrator.tasks.len(), 1);
    }

    #[tokio::test]
    async fn test_orchestrator_join_all() {
        let mut orchestrator = MockOrchestrator {
            tasks: vec![
                Task::new("Task 1", PathBuf::from("/tmp/1")),
                Task::new("Task 2", PathBuf::from("/tmp/2")),
            ],
        };

        let tasks = orchestrator.join_all().await.unwrap();
        assert_eq!(tasks.len(), 2);
    }
}
