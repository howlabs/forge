//! CheckpointStore implementation for v0.180.0
//!
//! File-based checkpoint storage for crash recovery

use async_trait::async_trait;
use agents::traits::CheckpointStore;
use agents::types::Checkpoint;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// File-based checkpoint storage
#[derive(Clone)]
pub struct FileCheckpointStore {
    /// Directory where checkpoints are stored
    store_path: PathBuf,
    /// In-memory cache of checkpoints
    cache: Arc<RwLock<HashMap<String, Checkpoint>>>,
}

impl FileCheckpointStore {
    /// Create a new file-based checkpoint store
    pub fn new(store_path: impl Into<PathBuf>) -> Result<Self> {
        let store_path = store_path.into();
        fs::create_dir_all(&store_path)
            .context("Failed to create checkpoint store directory")?;

        Ok(Self {
            store_path,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Synchronous wrapper for load (for CLI usage)
    pub fn load_sync(&self, task_id: &str) -> Result<Option<Checkpoint>> {
        // Direct file read without async overhead
        let path = self.checkpoint_path(task_id);

        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(&path)
            .context("Failed to read checkpoint file")?;

        let checkpoint: Checkpoint = serde_json::from_slice(&data)
            .context("Failed to deserialize checkpoint")?;

        Ok(Some(checkpoint))
    }

    /// Get checkpoint file path for a task
    fn checkpoint_path(&self, task_id: &str) -> PathBuf {
        self.store_path.join(format!("{}.checkpoint", task_id))
    }

    /// Load checkpoint from file
    fn load_from_file(&self, task_id: &str) -> Result<Option<Checkpoint>> {
        let path = self.checkpoint_path(task_id);

        if !path.exists() {
            return Ok(None);
        }

        let data = fs::read(&path)
            .context("Failed to read checkpoint file")?;

        // For MVP, store as JSON (bincode in production)
        let checkpoint: Checkpoint = serde_json::from_slice(&data)
            .context("Failed to deserialize checkpoint")?;

        Ok(Some(checkpoint))
    }

    /// Save checkpoint to file
    fn save_to_file(&self, checkpoint: &Checkpoint) -> Result<()> {
        let path = self.checkpoint_path(&checkpoint.task_id);

        // For MVP, store as JSON (bincode in production)
        let data = serde_json::to_vec_pretty(checkpoint)
            .context("Failed to serialize checkpoint")?;

        fs::write(&path, data)
            .context("Failed to write checkpoint file")?;

        Ok(())
    }
}

#[async_trait]
impl CheckpointStore for FileCheckpointStore {
    async fn save(&self, checkpoint: &Checkpoint) -> Result<()> {
        info!("Saving checkpoint for task {} at step {}", checkpoint.task_id, checkpoint.step);

        // Save to file
        self.save_to_file(checkpoint)?;

        // Update cache
        let mut cache = self.cache.write().await;
        cache.insert(checkpoint.task_id.clone(), checkpoint.clone());

        debug!("Checkpoint saved successfully");
        Ok(())
    }

    async fn load(&self, task_id: &str) -> Result<Option<Checkpoint>> {
        debug!("Loading checkpoint for task {}", task_id);

        // Try cache first
        {
            let cache = self.cache.read().await;
            if let Some(checkpoint) = cache.get(task_id) {
                return Ok(Some(checkpoint.clone()));
            }
        }

        // Load from file
        let checkpoint = self.load_from_file(task_id)?;

        // Update cache if found
        if let Some(ref checkpoint) = checkpoint {
            let mut cache = self.cache.write().await;
            cache.insert(task_id.to_string(), checkpoint.clone());
        }

        Ok(checkpoint)
    }

    async fn list_tasks(&self) -> Result<Vec<String>> {
        debug!("Listing all checkpointed tasks");

        let mut task_ids = Vec::new();

        let entries = fs::read_dir(&self.store_path)
            .context("Failed to read checkpoint store directory")?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("checkpoint") {
                if let Some(stem) = path.file_stem() {
                    if let Some(task_id) = stem.to_str() {
                        task_ids.push(task_id.to_string());
                    }
                }
            }
        }

        Ok(task_ids)
    }

    async fn delete(&self, task_id: &str) -> Result<()> {
        info!("Deleting checkpoint for task {}", task_id);

        // Remove from cache
        let mut cache = self.cache.write().await;
        cache.remove(task_id);

        // Remove file
        let path = self.checkpoint_path(task_id);
        if path.exists() {
            fs::remove_file(&path)
                .context("Failed to remove checkpoint file")?;
        }

        debug!("Checkpoint deleted successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_checkpoint_store() {
        let temp_dir = "/tmp/test_checkpoints";
        fs::remove_dir_all(temp_dir).ok();
        fs::create_dir_all(temp_dir).ok();

        let store = FileCheckpointStore::new(temp_dir).unwrap();
        let checkpoint = Checkpoint::new("test-task", 1, vec![1, 2, 3]);

        // Test save
        store.save(&checkpoint).await.unwrap();

        // Test load
        let loaded = store.load("test-task").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.task_id, "test-task");
        assert_eq!(loaded.step, 1);

        // Test list
        let tasks = store.list_tasks().await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks.contains(&"test-task".to_string()));

        // Test delete
        store.delete("test-task").await.unwrap();
        let loaded = store.load("test-task").await.unwrap();
        assert!(loaded.is_none());

        // Cleanup
        fs::remove_dir_all(temp_dir).ok();
    }
}
