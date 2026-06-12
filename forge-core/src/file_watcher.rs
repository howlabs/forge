use anyhow::Result;
use notify::{RecommendedWatcher, RecursiveMode, Watcher, Event, EventKind};
use forge_context::ContextIndex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

// =============================================================================
// FILE WATCHER - Incremental sync with debouncing (Track B v0.150.0)
// =============================================================================

/// File watcher with debouncing for incremental updates
pub struct FileWatcher {
    /// ContextIndex to update on file changes
    context_index: Arc<Mutex<dyn ContextIndex>>,
    /// Directory to watch
    watch_path: PathBuf,
    /// Debounce timeout (ms)
    debounce_timeout: Duration,
    /// Last event timestamp (for debouncing)
    last_event_time: Arc<Mutex<Option<std::time::Instant>>>,
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new(
        context_index: Arc<Mutex<dyn ContextIndex>>,
        watch_path: impl Into<PathBuf>,
        debounce_ms: u64,
    ) -> Result<Self> {
        let watch_path = watch_path.into();
        debug!("Creating file watcher for: {}", watch_path.display());

        if !watch_path.exists() {
            return Err(anyhow::anyhow!("Watch path does not exist: {}", watch_path.display()));
        }

        Ok(Self {
            context_index,
            watch_path,
            debounce_timeout: Duration::from_millis(debounce_ms),
            last_event_time: Arc::new(Mutex::new(None)),
        })
    }

    /// Start watching for file changes
    pub fn watch(&mut self) -> Result<()> {
        info!("Starting file watcher for: {}", self.watch_path.display());

        // Create notify channel
        let (tx, rx) = std::sync::mpsc::channel();

        // Create watcher with default configuration
        let mut watcher: RecommendedWatcher = Watcher::new(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    if let Err(e) = tx.send(event) {
                        warn!("Failed to send file event: {}", e);
                    }
                }
            },
            // Default config includes reasonable debouncing
            notify::Config::default(),
        )?;

        // Watch recursively
        watcher.watch(&self.watch_path, RecursiveMode::Recursive)?;

        info!("File watcher started successfully");

        // Spawn event processing task
        let context_index = self.context_index.clone();
        let last_event_time = self.last_event_time.clone();
        let debounce_timeout = self.debounce_timeout;

        tokio::spawn(async move {
            info!("File watcher event processor started");

            for event in rx {
                debug!("Received file event: {:?}", event);

                // Check debouncing
                let mut last_time = last_event_time.lock().await;
                let should_process = match *last_time {
                    Some(instant) => {
                        let elapsed = instant.elapsed();
                        elapsed >= debounce_timeout
                    }
                    None => true,
                };

                if should_process {
                    *last_time = Some(std::time::Instant::now());
                    drop(last_time);

                    // Process event
                    if let Err(e) = Self::process_event(&context_index, event).await {
                        warn!("Failed to process file event: {}", e);
                    }
                } else {
                    debug!("Event debounced (too soon after previous event)");
                    // Update last event time anyway
                    *last_time = Some(std::time::Instant::now());
                }
            }
        });

        Ok(())
    }

    /// Process a single file event
    async fn process_event(
        context_index: &Arc<Mutex<dyn ContextIndex>>,
        event: Event,
    ) -> Result<()> {
        let path = event.paths.first().ok_or_else(|| anyhow::anyhow!("No path in event"))?;

        // Skip if path is a directory
        if path.is_dir() {
            debug!("Skipping directory event: {}", path.display());
            return Ok(());
        }

        // Process based on event kind
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                debug!("File created/modified: {}", path.display());
                Self::handle_file_upsert(context_index, path).await?;
            }
            EventKind::Remove(_) => {
                debug!("File removed: {}", path.display());
                Self::handle_file_remove(context_index, path).await?;
            }
            _ => {
                debug!("Ignoring event kind: {:?}", event.kind);
            }
        }

        Ok(())
    }

    /// Handle file upsert (create or modify)
    async fn handle_file_upsert(
        context_index: &Arc<Mutex<dyn ContextIndex>>,
        path: &Path,
    ) -> Result<()> {
        // Read file content
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", path.display(), e))?;

        // Update context index
        let mut index = context_index.lock().await;
        index.upsert_file(path, &content);
        drop(index);

        debug!("Upserted file: {} ({} bytes)", path.display(), content.len());
        Ok(())
    }

    /// Handle file removal
    async fn handle_file_remove(
        context_index: &Arc<Mutex<dyn ContextIndex>>,
        path: &Path,
    ) -> Result<()> {
        // Update context index
        let mut index = context_index.lock().await;
        index.remove_file(path);
        drop(index);

        debug!("Removed file: {}", path.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_context::MockContextIndex;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_file_watcher_creation() {
        let temp_dir = TempDir::new().unwrap();
        let index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(MockContextIndex::new()));

        let watcher = FileWatcher::new(index, temp_dir.path(), 500);
        assert!(watcher.is_ok());
    }

    #[tokio::test]
    async fn test_file_watcher_invalid_path() {
        let index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(MockContextIndex::new()));

        let watcher = FileWatcher::new(index, "/nonexistent/path", 500);
        assert!(watcher.is_err());
    }

    #[tokio::test]
    async fn test_file_upsert_handler() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        let index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(MockContextIndex::new()));

        // Create a test file
        let mut file = File::create(&test_file).unwrap();
        file.write_all(b"fn test() {}").unwrap();
        drop(file);

        // Process upsert
        FileWatcher::handle_file_upsert(&index, &test_file)
            .await
            .unwrap();

        // Verify file was indexed (note: simple name without file prefix)
        let locked_index = index.lock().await;
        let symbol = locked_index.resolve_symbol("test");
        assert!(symbol.is_some());
    }

    #[tokio::test]
    async fn test_file_remove_handler() {
        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.rs");
        let index: Arc<Mutex<dyn ContextIndex>> = Arc::new(Mutex::new(MockContextIndex::new()));

        // Create and index a file
        {
            let mut locked_index = index.lock().await;
            locked_index.upsert_file(&test_file, "fn test() {}");
        }

        // Process removal
        FileWatcher::handle_file_remove(&index, &test_file)
            .await
            .unwrap();

        // Verify file was removed from index
        let locked_index = index.lock().await;
        let symbol = locked_index.resolve_symbol(&format!("{}::test", test_file.display()));
        assert!(symbol.is_none());
    }
}
