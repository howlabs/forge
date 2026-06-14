//! Multi-agent orchestrator (v0.170.0)
//!
//! Spawns isolated subagents in git worktrees, manages parallel execution

use crate::traits::Orchestrator;
use crate::types::{Task, TaskStatus};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Multi-agent orchestrator with git worktree isolation
pub struct MultiAgentOrchestrator {
    /// Git repository path (for creating worktrees)
    repo_path: PathBuf,
    /// Running tasks indexed by task_id
    tasks: Arc<RwLock<HashMap<String, Task>>>,
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
            return Err(anyhow::anyhow!(
                "Not a git repository: {}",
                repo_path.display()
            ));
        }

        // Create worktree base directory if needed
        std::fs::create_dir_all(&worktree_base)
            .context("Failed to create worktree base directory")?;

        Ok(Self {
            repo_path,
            worktree_base,
            max_parallel,
            tasks: Arc::new(RwLock::new(HashMap::new())),
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
            return Err(anyhow::anyhow!(
                "Failed to create worktree for task {}",
                task_id
            ));
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

        let task_id = task.id.clone();
        let task_prompt = task.prompt.clone();
        let api_key = task.api_key.clone();
        let model = task.model.clone();
        let worktree_path_clone = worktree_path.clone();
        let tasks_ref = self.tasks.clone();

        tokio::spawn(async move {
            use context::ContextEngine;
            use forge_core::event_loop::EventLoop;
            use provider::anthropic::AnthropicProvider;
            use sandbox::Sandbox;

            let result = async {
                let provider = AnthropicProvider::new(&api_key, &model)?;
                let context = ContextEngine::new(&worktree_path_clone)?;
                let sandbox = Sandbox::new(&worktree_path_clone, "on")?;
                let mut event_loop = EventLoop::new(provider, context, sandbox, task_prompt);
                let steps = event_loop.run().await?;
                anyhow::Ok(steps)
            }
            .await;

            let mut tasks = tasks_ref.write().await;
            if let Some(t) = tasks.get_mut(&task_id) {
                match result {
                    Ok(steps) => {
                        t.status = TaskStatus::Done;
                        t.steps = steps;
                        t.result = Some(format!("Completed in {} steps", steps));
                    }
                    Err(error) => {
                        t.status = TaskStatus::Failed;
                        t.result = Some(error.to_string());
                    }
                }
            }
        });

        Ok(())
    }

    async fn join_all(&mut self) -> Result<Vec<Task>> {
        info!("Waiting for all tasks to complete");

        loop {
            if self.running_count().await == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

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
                if let Err(e) = self.remove_worktree(&task.id) {
                    warn!("Failed to remove worktree {}: {}", task.id, e);
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
    use tempfile::TempDir;

    fn init_git_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::process::Command::new("git")
            .args([
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=Test User",
                "commit",
                "--allow-empty",
                "-m",
                "init",
            ])
            .current_dir(dir.path())
            .status()
            .unwrap();
        dir
    }

    #[tokio::test]
    async fn test_orchestrator_creation() {
        let orchestrator = MultiAgentOrchestrator::new("/tmp/test_repo", "/tmp/test_worktrees", 2);

        // Will fail because /tmp/test_repo is not a git repo
        assert!(orchestrator.is_err());
    }

    #[tokio::test]
    async fn test_task_spawn_simulation() {
        let repo = init_git_repo();
        let worktrees = TempDir::new().unwrap();
        let orchestrator = MultiAgentOrchestrator::new(repo.path(), worktrees.path(), 2).unwrap();
        let worktree_path = orchestrator.create_worktree("test-task").unwrap();

        assert!(worktree_path.exists());
        assert!(worktree_path.join(".git").exists());
    }

    #[tokio::test]
    async fn test_spawn_records_running_task_in_worktree() {
        let repo = init_git_repo();
        let worktrees = TempDir::new().unwrap();
        let mut orchestrator =
            MultiAgentOrchestrator::new(repo.path(), worktrees.path(), 2).unwrap();
        let task = Task::new("do work", PathBuf::new()).with_provider("test-key", "test-model");
        let task_id = task.id.clone();

        orchestrator.spawn(task).await.unwrap();

        let tasks = orchestrator.tasks.read().await;
        let stored = tasks.get(&task_id).unwrap();
        assert!(stored.worktree.exists());
        assert!(matches!(
            stored.status,
            TaskStatus::Running | TaskStatus::Failed
        ));
    }
}
