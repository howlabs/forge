//! Forge Context Engine
//!
//! Provides semantic context indexing and retrieval for code understanding.

use std::path::Path;

pub mod agents;
pub mod engine;
pub mod graph;
pub mod lang;
pub mod symbols;
pub mod vector;

// ponytail: deleted 1400 lines of legacy dual implementations. The only shared contract is this trait.
// query() and resolve_symbol() were unused, removed per YAGNI.

/// Context index interface for file watching
pub trait ContextIndex: Send + Sync {
    /// Add or update a file in the index
    fn upsert_file(&mut self, path: &Path, src: &str);

    /// Remove a file from the index
    fn remove_file(&mut self, path: &Path);
}
