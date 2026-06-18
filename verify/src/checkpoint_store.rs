//! CheckpointStore implementation — async wrapper over forge-core persistence.

use agents::traits::CheckpointStore;
use agents::types::{Checkpoint, CheckpointState as AgentsCheckpointState, LastVerify};
use anyhow::Result;
use async_trait::async_trait;
use forge_core::{CheckpointState, LoopCheckpoint};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// Async checkpoint store with in-memory cache (implements agents trait).
#[derive(Clone)]
pub struct CachedCheckpointStore {
    inner: Arc<forge_core::FileCheckpointStore>,
    cache: Arc<RwLock<HashMap<String, Checkpoint>>>,
}

impl CachedCheckpointStore {
    pub fn new(store_path: impl Into<PathBuf>) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(forge_core::FileCheckpointStore::new(store_path)?),
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn load_sync(&self, task_id: &str) -> Result<Option<Checkpoint>> {
        self.inner
            .load(task_id)?
            .map(to_agents_checkpoint)
            .transpose()
    }
}

fn to_agents_checkpoint(cp: LoopCheckpoint) -> Result<Checkpoint> {
    Ok(Checkpoint {
        task_id: cp.task_id,
        step: cp.step,
        timestamp: cp.timestamp,
        state: AgentsCheckpointState {
            history: cp
                .state
                .history
                .into_iter()
                .map(serde_json::to_value)
                .collect::<Result<Vec<_>, _>>()?,
            worktree_refs: cp.state.worktree_refs,
            last_verify: cp.state.last_verify.map(|v| LastVerify {
                passed: v.passed,
                logs: v.logs,
            }),
        },
    })
}

fn to_loop_checkpoint(cp: &Checkpoint) -> Result<LoopCheckpoint> {
    let history = cp
        .state
        .history
        .iter()
        .map(|v| serde_json::from_value(v.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LoopCheckpoint {
        task_id: cp.task_id.clone(),
        step: cp.step,
        timestamp: cp.timestamp,
        state: CheckpointState {
            history,
            worktree_refs: cp.state.worktree_refs.clone(),
            last_verify: cp
                .state
                .last_verify
                .as_ref()
                .map(|v| forge_core::LastVerify {
                    passed: v.passed,
                    logs: v.logs.clone(),
                }),
        },
    })
}

#[async_trait]
impl CheckpointStore for CachedCheckpointStore {
    async fn save(&self, checkpoint: &Checkpoint) -> Result<()> {
        info!(
            "Saving checkpoint for task {} at step {}",
            checkpoint.task_id, checkpoint.step
        );
        self.inner.save(&to_loop_checkpoint(checkpoint)?)?;
        let mut cache = self.cache.write().await;
        cache.insert(checkpoint.task_id.clone(), checkpoint.clone());
        debug!("Checkpoint saved successfully");
        Ok(())
    }

    async fn load(&self, task_id: &str) -> Result<Option<Checkpoint>> {
        debug!("Loading checkpoint for task {}", task_id);
        {
            let cache = self.cache.read().await;
            if let Some(checkpoint) = cache.get(task_id) {
                return Ok(Some(checkpoint.clone()));
            }
        }
        let checkpoint = self
            .inner
            .load(task_id)?
            .map(to_agents_checkpoint)
            .transpose()?;
        if let Some(ref checkpoint) = checkpoint {
            let mut cache = self.cache.write().await;
            cache.insert(task_id.to_string(), checkpoint.clone());
        }
        Ok(checkpoint)
    }

    async fn list_tasks(&self) -> Result<Vec<String>> {
        self.inner.list_tasks()
    }

    async fn delete(&self, task_id: &str) -> Result<()> {
        info!("Deleting checkpoint for task {}", task_id);
        let mut cache = self.cache.write().await;
        cache.remove(task_id);
        self.inner.delete(task_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_checkpoint_store() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let store = CachedCheckpointStore::new(temp_dir.path()).unwrap();
        let checkpoint = Checkpoint::new(
            "test-task",
            1,
            AgentsCheckpointState {
                history: vec![serde_json::json!({"role":"user","content":"hi"})],
                worktree_refs: vec!["/tmp/wt".into()],
                last_verify: None,
            },
        );

        store.save(&checkpoint).await.unwrap();
        let loaded = store.load("test-task").await.unwrap().unwrap();
        assert_eq!(loaded.task_id, "test-task");
        assert_eq!(loaded.step, 1);
        assert_eq!(loaded.state.worktree_refs, vec!["/tmp/wt"]);

        let tasks = store.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);

        store.delete("test-task").await.unwrap();
        assert!(store.load("test-task").await.unwrap().is_none());
    }
}
