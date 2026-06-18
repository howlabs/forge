//! Crash-safe checkpoint persistence for long-horizon event-loop tasks.

use anyhow::{Context, Result};
use provider::Message;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Serializable event-loop snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckpointState {
    pub history: Vec<Message>,
    pub worktree_refs: Vec<String>,
    pub last_verify: Option<LastVerify>,
}

impl CheckpointState {
    pub fn new(history: Vec<Message>) -> Self {
        Self {
            history,
            worktree_refs: Vec::new(),
            last_verify: None,
        }
    }

    pub fn with_worktree(mut self, path: impl Into<String>) -> Self {
        self.worktree_refs.push(path.into());
        self
    }
}

/// Last verify run recorded in a checkpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastVerify {
    pub passed: bool,
    pub logs: String,
}

/// On-disk checkpoint envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopCheckpoint {
    pub task_id: String,
    pub step: u32,
    pub timestamp: SystemTime,
    pub state: CheckpointState,
}

impl LoopCheckpoint {
    pub fn new(task_id: impl Into<String>, step: u32, state: CheckpointState) -> Self {
        Self {
            task_id: task_id.into(),
            step,
            timestamp: SystemTime::now(),
            state,
        }
    }
}

/// File-based checkpoint store under `.forge/checkpoints/`.
#[derive(Debug, Clone)]
pub struct FileCheckpointStore {
    store_path: PathBuf,
}

impl FileCheckpointStore {
    pub fn new(store_path: impl Into<PathBuf>) -> Result<Self> {
        let store_path = store_path.into();
        fs::create_dir_all(&store_path).context("Failed to create checkpoint store directory")?;
        Ok(Self { store_path })
    }

    pub fn default_for_cwd() -> Result<Self> {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Self::new(cwd.join(".forge/checkpoints"))
    }

    pub fn checkpoint_path(&self, task_id: &str) -> PathBuf {
        self.store_path.join(format!("{task_id}.checkpoint"))
    }

    pub fn save(&self, checkpoint: &LoopCheckpoint) -> Result<()> {
        let path = self.checkpoint_path(&checkpoint.task_id);
        let data =
            serde_json::to_vec_pretty(checkpoint).context("Failed to serialize checkpoint")?;
        fs::write(path, data).context("Failed to write checkpoint file")?;
        Ok(())
    }

    /// Sync alias used by CLI helpers.
    pub fn load_sync(&self, task_id: &str) -> Result<Option<LoopCheckpoint>> {
        self.load(task_id)
    }

    pub fn load(&self, task_id: &str) -> Result<Option<LoopCheckpoint>> {
        let path = self.checkpoint_path(task_id);
        if !path.exists() {
            return Ok(None);
        }
        let data = fs::read(&path).context("Failed to read checkpoint file")?;
        if let Ok(checkpoint) = serde_json::from_slice::<LoopCheckpoint>(&data) {
            return Ok(Some(checkpoint));
        }
        // ponytail: legacy format stored history as raw JSON bytes in `state`
        #[derive(Deserialize)]
        struct LegacyCheckpoint {
            task_id: String,
            step: u32,
            state: Vec<u8>,
            timestamp: SystemTime,
        }
        let legacy: LegacyCheckpoint =
            serde_json::from_slice(&data).context("Failed to deserialize checkpoint")?;
        let history: Vec<Message> = serde_json::from_slice(&legacy.state)
            .context("Failed to deserialize legacy history")?;
        Ok(Some(LoopCheckpoint {
            task_id: legacy.task_id,
            step: legacy.step,
            timestamp: legacy.timestamp,
            state: CheckpointState::new(history),
        }))
    }

    pub fn list_tasks(&self) -> Result<Vec<String>> {
        let mut task_ids = Vec::new();
        if !self.store_path.exists() {
            return Ok(task_ids);
        }
        for entry in fs::read_dir(&self.store_path).context("Failed to read checkpoint store")? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("checkpoint") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    task_ids.push(stem.to_string());
                }
            }
        }
        Ok(task_ids)
    }

    pub fn delete(&self, task_id: &str) -> Result<()> {
        let path = self.checkpoint_path(task_id);
        if path.exists() {
            fs::remove_file(path).context("Failed to remove checkpoint file")?;
        }
        Ok(())
    }

    /// Restore worktree cwd when the checkpoint recorded a different directory.
    pub fn restore_worktree(checkpoint: &LoopCheckpoint) -> Result<()> {
        let Some(worktree) = checkpoint.state.worktree_refs.last() else {
            return Ok(());
        };
        let target = Path::new(worktree);
        if target.exists() {
            std::env::set_current_dir(target)
                .with_context(|| format!("Failed to chdir to worktree {}", worktree))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = FileCheckpointStore::new(dir.path()).unwrap();
        let state = CheckpointState::new(vec![
            Message::user("fix bug"),
            Message::assistant("reading file"),
        ])
        .with_worktree("/tmp/worktree");
        let cp = LoopCheckpoint::new("task-1", 2, state);
        store.save(&cp).unwrap();
        let loaded = store.load("task-1").unwrap().unwrap();
        assert_eq!(loaded.task_id, cp.task_id);
        assert_eq!(loaded.step, cp.step);
        assert_eq!(loaded.state.history.len(), cp.state.history.len());
        assert_eq!(loaded.state.worktree_refs, cp.state.worktree_refs);
    }

    #[test]
    fn legacy_checkpoint_loads() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = FileCheckpointStore::new(dir.path()).unwrap();
        let history = serde_json::to_vec(&vec![Message::user("hello")]).unwrap();
        let legacy = serde_json::json!({
            "task_id": "legacy-task",
            "step": 1,
            "state": history,
            "timestamp": SystemTime::now(),
        });
        fs::write(
            store.checkpoint_path("legacy-task"),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();
        let loaded = store.load("legacy-task").unwrap().unwrap();
        assert_eq!(loaded.task_id, "legacy-task");
        assert_eq!(loaded.step, 1);
        assert_eq!(loaded.state.history.len(), 1);
    }
}
