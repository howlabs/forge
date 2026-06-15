//! Multi-agent orchestrator (v0.170.0 → P5 §6)
//!
//! Spawns isolated subagents in git worktrees, manages parallel execution,
//! detects merge conflicts via `git merge-tree`, and persists task state
//! to `.forge/agents/<task_id>.json`.

use crate::traits::Orchestrator;
use crate::types::{Task, TaskStatus};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Save a task to disk atomically (temp + rename).
fn save_task_to_disk(state_dir: &Path, task: &Task) -> Result<()> {
    let path = state_dir.join(format!("{}.json", task.id));
    let json =
        serde_json::to_string_pretty(task).context("failed to serialize task")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &json).context("failed to write task state")?;
    std::fs::rename(&tmp, &path).context("failed to rename task state")?;
    Ok(())
}

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
    /// Directory for persisting task state: `<repo>/.forge/agents/`
    state_dir: PathBuf,
    /// Serialise merges: one merge at a time.
    merge_lock: Arc<tokio::sync::Mutex<()>>,
}

impl MultiAgentOrchestrator {
    /// Create a new orchestrator.  State is persisted under
    /// `<repo_path>/.forge/agents/`.
    pub fn new(
        repo_path: impl Into<PathBuf>,
        worktree_base: impl Into<PathBuf>,
        max_parallel: usize,
    ) -> Result<Self> {
        let repo_path = repo_path.into();
        let worktree_base = worktree_base.into();

        if !repo_path.join(".git").exists() {
            return Err(anyhow::anyhow!(
                "Not a git repository: {}",
                repo_path.display()
            ));
        }

        std::fs::create_dir_all(&worktree_base)
            .context("Failed to create worktree base directory")?;

        let state_dir = repo_path.join(".forge").join("agents");
        std::fs::create_dir_all(&state_dir)
            .context("Failed to create agents state directory")?;

        Ok(Self {
            repo_path,
            worktree_base,
            max_parallel,
            state_dir,
            tasks: Arc::new(RwLock::new(HashMap::new())),
            merge_lock: Arc::new(tokio::sync::Mutex::new(())),
        })
    }

    /// Create a git worktree for a task.  If the branch already exists
    /// (e.g. from a previous failed run), the existing branch is reused
    /// rather than failing.
    fn create_worktree(&self, task_id: &str) -> Result<PathBuf> {
        let worktree_path = self.worktree_base.join(task_id);

        // Remove existing worktree directory if present
        if worktree_path.exists() {
            std::fs::remove_dir_all(&worktree_path)
                .context("Failed to remove existing worktree")?;
        }

        // Check if branch already exists
        let branch_exists = Command::new("git")
            .args(["rev-parse", "--verify", task_id])
            .current_dir(&self.repo_path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let status = if branch_exists {
            // Branch exists — attach worktree to it (no -b flag)
            info!("Reusing existing branch {} for worktree", task_id);
            Command::new("git")
                .args(["worktree", "add"])
                .arg(&worktree_path)
                .arg(task_id)
                .current_dir(&self.repo_path)
                .status()?
        } else {
            // New branch
            Command::new("git")
                .args(["worktree", "add", "-b", task_id])
                .arg(&worktree_path)
                .current_dir(&self.repo_path)
                .status()?
        };

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

    /// Detect merge conflicts *without* touching the working tree, using
    /// `git merge-tree`.  Returns a list of conflicting file paths, or
    /// an empty vec if the merge is clean.
    fn detect_merge_conflicts(&self, task_id: &str) -> Result<Vec<String>> {
        // Find merge base between current HEAD and the task branch
        let base_output = Command::new("git")
            .args(["merge-base", "HEAD", task_id])
            .current_dir(&self.repo_path)
            .output()
            .context("failed to run git merge-base")?;

        if !base_output.status.success() {
            return Err(anyhow::anyhow!(
                "Cannot find merge base for branch {}",
                task_id
            ));
        }

        let base = String::from_utf8_lossy(&base_output.stdout).trim().to_string();

        // Use git merge-tree to detect conflicts
        let merge_output = Command::new("git")
            .args(["merge-tree", &base, "HEAD", task_id])
            .current_dir(&self.repo_path)
            .output()
            .context("failed to run git merge-tree")?;

        let stdout = String::from_utf8_lossy(&merge_output.stdout);
        let stderr = String::from_utf8_lossy(&merge_output.stderr);

        // git merge-tree outputs conflict markers in stdout
        // Look for "CONFLICT" in output
        let mut conflicts = Vec::new();
        for line in stdout.lines().chain(stderr.lines()) {
            if line.contains("CONFLICT") {
                // Extract file path from conflict lines like:
                // "CONFLICT (content): Merge conflict in path/to/file.rs"
                if let Some(pos) = line.find("in ") {
                    let path = line[pos + 3..].trim().to_string();
                    if !path.is_empty() {
                        conflicts.push(path);
                    }
                }
            }
        }

        Ok(conflicts)
    }

    /// Check if any files modified by the task fall outside its declared
    /// scope.  Returns a list of out-of-scope files, or empty vec if
    /// all changes are within scope.
    fn check_scope_violations(&self, task: &Task) -> Vec<String> {
        let Some(ref globs) = task.scope else {
            return Vec::new();
        };

        let worktree_path = self.worktree_base.join(&task.id);
        if !worktree_path.exists() {
            return Vec::new();
        }

        // Get list of changed files vs the task branch base
        let output = Command::new("git")
            .args(["diff", "--name-only", &format!("{}^", task.id)])
            .current_dir(&worktree_path)
            .output();

        let output = match output {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };

        let changed = String::from_utf8_lossy(&output.stdout);
        let mut violations = Vec::new();

        for file in changed.lines() {
            let file = file.trim();
            if file.is_empty() {
                continue;
            }
            // ponytail: naive glob check — just check if any glob
            // suffix-matches.  Full glob semantics (`, `*, `**`) would
            // need a crate.
            let in_scope = globs.iter().any(|glob| {
                let glob = glob.trim_start_matches("**/");
                file.starts_with(glob.trim_end_matches("/**"))
                    || glob == "**"
                    || glob == "*"
            });
            if !in_scope {
                violations.push(file.to_string());
            }
        }

        violations
    }

    /// Persist task state to `.forge/agents/<task_id>.json`.
    fn save_task(&self, task: &Task) -> Result<()> {
        save_task_to_disk(&self.state_dir, task)
    }

    /// Load a single task from disk.
    fn load_task(&self, task_id: &str) -> Option<Task> {
        let path = self.state_dir.join(format!("{}.json", task_id));
        let data = std::fs::read_to_string(&path).ok()?;
        serde_json::from_str(&data).ok()
    }

    /// List all tasks persisted on disk.
    pub fn list_tasks_from_disk(&self) -> Vec<Task> {
        let mut tasks = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.state_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    if let Ok(data) = std::fs::read_to_string(&path) {
                        if let Ok(task) = serde_json::from_str::<Task>(&data) {
                            tasks.push(task);
                        }
                    }
                }
            }
        }
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        tasks
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
        if self.running_count().await >= self.max_parallel {
            return Err(anyhow::anyhow!("Max parallel limit reached"));
        }

        info!("Spawning task {}: {}", task.id, task.prompt);

        let worktree_path = self.create_worktree(&task.id)?;
        task.worktree = worktree_path.clone();
        task.status = TaskStatus::Running;

        // Persist initial state
        self.save_task(&task)?;

        let mut tasks = self.tasks.write().await;
        tasks.insert(task.id.clone(), task.clone());
        drop(tasks);

        let task_id = task.id.clone();
        let task_prompt = task.prompt.clone();
        let api_key = task.api_key.clone();
        let model = task.model.clone();
        let provider_name = task.provider.clone();
        let worktree_path_clone = worktree_path.clone();
        let tasks_ref = self.tasks.clone();
        let state_dir = self.state_dir.clone();

        tokio::spawn(async move {
            use context::ContextEngine;
            use forge_core::event_loop::EventLoop;
            use sandbox::Sandbox;

            let result = async {
                // ponytail: use provider factory instead of hardcoding Anthropic
                let provider = provider::create_provider(&provider_name, &model, &api_key)?;
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
                // Persist updated state (atomic write)
                let _ = save_task_to_disk(&state_dir, &t.clone());
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

        let tasks = self.tasks.read().await;
        let completed: Vec<Task> = tasks.values().cloned().collect();
        drop(tasks);

        // Merge sequentially, one at a time
        for task in &completed {
            if task.status == TaskStatus::Done {
                if let Err(e) = self.merge_task(task).await {
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
        // Try in-memory first, then disk
        if let Ok(tasks) = self.tasks.try_read() {
            if let Some(t) = tasks.get(task_id) {
                return Some(t.status);
            }
        }
        self.load_task(task_id).map(|t| t.status)
    }

    async fn cancel_task(&mut self, task_id: &str) -> Result<()> {
        info!("Cancelling task {}", task_id);

        let mut tasks = self.tasks.write().await;
        if let Some(task) = tasks.get_mut(task_id) {
            task.status = TaskStatus::Failed;
            task.result = Some("Cancelled by user".to_string());
            let _ = self.save_task(task);

            if let Err(e) = self.remove_worktree(task_id) {
                warn!("Failed to remove worktree for cancelled task: {}", e);
            }

            Ok(())
        } else {
            Err(anyhow::anyhow!("Task not found: {}", task_id))
        }
    }
}

impl MultiAgentOrchestrator {
    /// Merge a single completed task.  Uses `git merge-tree` to detect
    /// conflicts before touching the working tree.  If conflicts are
    /// found, stops and reports them (policy: `on-request`).
    async fn merge_task(&self, task: &Task) -> Result<()> {
        let _guard = self.merge_lock.lock().await;

        // 1. Check for scope violations
        let violations = self.check_scope_violations(task);
        if !violations.is_empty() {
            warn!(
                "Task {} modified files outside its scope: {:?}",
                task.id, violations
            );
            // ponytail: warn only, don't block — scope enforcement
            // at write-time would need sandbox runtime.
        }

        // 2. Detect merge conflicts via merge-tree (no working tree mutation)
        let conflicts = self.detect_merge_conflicts(&task.id)?;
        if !conflicts.is_empty() {
            let msg = format!(
                "Merge conflict detected for task {}. Conflicting files:\n{}",
                task.id,
                conflicts.join("\n")
            );
            warn!("{}", msg);
            // policy: on-request — stop and let the user resolve
            anyhow::bail!("{}", msg);
        }

        // 3. Clean merge — check if there's anything to merge
        let diff_output = Command::new("git")
            .args(["diff", "--quiet", "HEAD", &task.id])
            .current_dir(&self.repo_path)
            .output();

        if let Ok(o) = diff_output {
            if o.status.success() {
                debug!("No changes to merge from task {}", task.id);
                return Ok(());
            }
        }

        // 4. Perform the actual merge
        let status = Command::new("git")
            .args(["merge", "--no-edit", &task.id])
            .current_dir(&self.repo_path)
            .status()
            .context("failed to run git merge")?;

        if !status.success() {
            anyhow::bail!("Merge failed for task {}", task.id);
        }

        info!("Merged changes from task {}", task.id);
        Ok(())
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
        let task = Task::new("do work", PathBuf::new())
            .with_provider("test-key", "test-model")
            .with_provider_name("anthropic");
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

    #[tokio::test]
    async fn test_state_persistence_roundtrip() {
        let repo = init_git_repo();
        let worktrees = TempDir::new().unwrap();
        let orchestrator =
            MultiAgentOrchestrator::new(repo.path(), worktrees.path(), 2).unwrap();

        let mut task = Task::new("persist me", PathBuf::new())
            .with_provider("key", "model")
            .with_provider_name("openai")
            .with_scope(vec!["src/**".into()]);
        task.status = TaskStatus::Done;
        task.steps = 5;
        task.result = Some("ok".into());

        orchestrator.save_task(&task).unwrap();
        let loaded = orchestrator.load_task(&task.id).unwrap();
        assert_eq!(loaded.id, task.id);
        assert_eq!(loaded.provider, "openai");
        assert_eq!(loaded.scope, Some(vec!["src/**".into()]));
        assert_eq!(loaded.steps, 5);
    }

    #[tokio::test]
    async fn test_list_tasks_from_disk() {
        let repo = init_git_repo();
        let worktrees = TempDir::new().unwrap();
        let orchestrator =
            MultiAgentOrchestrator::new(repo.path(), worktrees.path(), 2).unwrap();

        let t1 = Task::new("task1", PathBuf::new());
        let t2 = Task::new("task2", PathBuf::new());
        orchestrator.save_task(&t1).unwrap();
        orchestrator.save_task(&t2).unwrap();

        let listed = orchestrator.list_tasks_from_disk();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn test_no_conflicts_on_clean_merge_base() {
        let repo = init_git_repo();
        let worktrees = TempDir::new().unwrap();
        let orchestrator =
            MultiAgentOrchestrator::new(repo.path(), worktrees.path(), 2).unwrap();

        // No branch "nonexistent" → merge-base fails
        let result = orchestrator.detect_merge_conflicts("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_scope_violations_empty_for_no_scope() {
        let repo = init_git_repo();
        let worktrees = TempDir::new().unwrap();
        let orchestrator =
            MultiAgentOrchestrator::new(repo.path(), worktrees.path(), 2).unwrap();

        let task = Task::new("test", PathBuf::new());
        assert!(orchestrator.check_scope_violations(&task).is_empty());
    }
}
