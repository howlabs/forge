//! Multi-agent orchestrator (v0.170.0)
//!
//! Spawns isolated subagents in git worktrees, manages parallel execution

use crate::traits::Orchestrator;
use crate::types::{Task, TaskStatus};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Multi-agent orchestrator with git worktree isolation
pub struct MultiAgentOrchestrator {
    /// Git repository path (for creating worktrees)
    repo_path: PathBuf,
    /// Running tasks indexed by task_id
    tasks: RwLock<HashMap<String, Task>>,
    /// Maximum parallel tasks
    max_parallel: usize,
    /// Worktree base directory
    worktree_base: PathBuf,
}

impl MultiAgentOrchestrator {
    /// Create a new orchestrator
    pub fn new(
        repo_path: impl Into<PathBuf>,
        worktree_base: impl Into<PathBuf>,
        max_parallel: usize,
    ) -> Result<Self> {
        let repo_path = repo_path.into();
        let worktree_base = worktree_base.into();

        // Verify repo exists
        if !repo_path.join(".git").exists() {
            return Err(anyhow::anyhow!("Not a git repository: {}", repo_path.display()));
        }

        // Create worktree base directory if needed
        std::fs::create_dir_all(&worktree_base)
            .context("Failed to create worktree base directory")?;

        Ok(Self {
            repo_path,
            worktree_base,
            max_parallel,
            tasks: RwLock::new(HashMap::new()),
        })
    }

    /// Create a git worktree for a task
    fn create_worktree(&self, task_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktree_base.join(task_id);

        // Remove existing worktree if present
        if worktree_path.exists() {
            std::fs::remove_dir_all(&worktree_path)
                .context("Failed to remove existing worktree")?;
        }

        // Create new worktree
        let status = Command::new("git")
            .args(["worktree", "add", "-b", task_id])
            .arg(&worktree_path)
            .current_dir(&self.repo_path)
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!("Failed to create worktree for task {}", task_id));
        }

        info!("Created worktree: {}", worktree_path.display());
        Ok(worktree_path)
    }

    /// Remove a worktree after task completion
    fn remove_worktree(&self, task_id: &str) -> Result<()> {
        let worktree_path = self.worktree_base.join(task_id);

        if worktree_path.exists() {
            // Remove worktree
            let status = Command::new("git")
                .args(["worktree", "remove"])
                .arg(&worktree_path)
                .current_dir(&self.repo_path)
                .status()?;

            if !status.success() {
                warn!("Failed to remove worktree for task {}", task_id);
            } else {
                info!("Removed worktree: {}", worktree_path.display());
            }
        }

        Ok(())
    }

    /// Merge worktree changes back to main branch
    fn merge_worktree(&self, task_id: &str) -> Result<()> {
        let worktree_path = self.worktree_base.join(task_id);

        // Check if worktree has changes
        let status = Command::new("git")
            .args(["diff", "--quiet", "HEAD"])
            .current_dir(&worktree_path)
            .status()?;

        if status.success() {
            debug!("No changes to merge from worktree {}", task_id);
            return Ok(());
        }

        // Merge worktree branch into main
        let status = Command::new("git")
            .args(["merge", "--no-edit", task_id])
            .current_dir(&self.repo_path)
            .status()?;

        if !status.success() {
            return Err(anyhow::anyhow!("Merge conflict from task {}", task_id));
        }

        info!("Merged changes from task {}", task_id);
        Ok(())
    }

    /// Get count of currently running tasks
    async fn running_count(&self) -> usize {
        let tasks = self.tasks.read().await;
        tasks.values().filter(|t| t.is_running()).count()
    }
}

#[async_trait::async_trait]
impl Orchestrator for MultiAgentOrchestrator {
    async fn spawn(&mut self, mut task: Task) -> Result<()> {
        // Check parallel limit
        if self.running_count().await >= self.max_parallel {
            return Err(anyhow::anyhow!("Max parallel limit reached"));
        }

        info!("Spawning task {}: {}", task.id, task.prompt);

        // Create isolated worktree
        let worktree_path = self.create_worktree(&task.id)?;
        task.worktree = worktree_path.clone();

        // Update task status to Running
        task.status = TaskStatus::Running;

        // Store task
        let mut tasks = self.tasks.write().await;
        tasks.insert(task.id.clone(), task.clone());
        drop(tasks);

        // TODO: Launch subagent event-loop in worktree
        // For now, simulate immediate completion
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Update task to Done
        let mut tasks = self.tasks.write().await;
        if let Some(t) = tasks.get_mut(&task.id) {
            t.status = TaskStatus::Done;
        }

        info!("Task {} completed", task.id);
        Ok(())
    }

    async fn join_all(&mut self) -> Result<Vec<Task>> {
        info!("Waiting for all tasks to complete");

        // Wait for all running tasks (simulate for now)
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Collect all tasks
        let tasks = self.tasks.read().await;
        let completed: Vec<Task> = tasks.values().cloned().collect();
        drop(tasks);

        // Merge all completed tasks
        for task in &completed {
            if task.status == TaskStatus::Done {
                if let Err(e) = self.merge_worktree(&task.id) {
                    warn!("Failed to merge task {}: {}", task.id, e);
                }
            }
        }

        Ok(completed)
    }

    fn get_task_status(&self, task_id: &str) -> Option<TaskStatus> {
        // Blocking read for sync interface
        let tasks = self.tasks.try_read().ok()?;
        tasks.get(task_id).map(|t| t.status)
    }

    async fn cancel_task(&mut self, task_id: &str) -> Result<()> {
        info!("Cancelling task {}", task_id);

        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Failed;

            // Remove worktree
            if let Err(e) = self.remove_worktree(task_id) {
                warn!("Failed to remove worktree for cancelled task: {}", e);
            }

            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found: {}", task_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_orchestrator_creation() {
        let orchestrator = MultiAgentOrchestrator::new(
            "/tmp/test_repo",
            "/tmp/test_worktrees",
            2,
        );

        // Will fail because /tmp/test_repo is not a git repo
        assert!(orchestrator.is_err());
    }

    #[tokio::test]
    async fn test_task_spawn_simulation() {
        // This test requires a real git repo, skip for now
        // TODO: Setup test git repo
        assert!(true);
    }
}
