use anyhow::Result;
use std::path::{Path, PathBuf};
use tracing::debug;

// =============================================================================
// SHARED CONTRACT - Frozen before splitting into parallel tracks
// Both v0.100.0 (Context Engine) and v0.130.0 (Sync) depend on this API
// =============================================================================

/// Symbol extracted from code (function, struct, enum, etc.)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Symbol name (e.g., "ContextIndex::upsert_file")
    pub name: String,
    /// Symbol kind
    pub kind: SymbolKind,
    /// File containing this symbol
    pub file: PathBuf,
    /// Source range: (start_line, start_col, end_line, end_col)
    pub range: (usize, usize, usize, usize),
}

/// Kinds of symbols we can extract
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    TypeAlias,
    Const,
    Static,
    Macro,
}

/// Chunk of code with semantic metadata
#[derive(Debug, Clone)]
pub struct CodeChunk {
    /// File containing this chunk
    pub file: PathBuf,
    /// Source range within file
    pub range: (usize, usize, usize, usize),
    /// Actual source text
    pub text: String,
    /// IDs of symbols referenced in this chunk
    pub symbol_ids: Vec<String>,
}

/// Context index interface - both tracks implement this
pub trait ContextIndex {
    /// Add or update a file in the index
    fn upsert_file(&mut self, path: &Path, src: &str);

    /// Remove a file from the index
    fn remove_file(&mut self, path: &Path);

    /// Semantic search: return top-k chunks matching query
    fn query(&self, q: &str, k: usize) -> Vec<CodeChunk>;

    /// Resolve a symbol by name -> returns symbol metadata if found
    fn resolve_symbol(&self, name: &str) -> Option<Symbol>;
}

// =============================================================================
// LEGACY MVP CODE - Load AGENTS.md (v0.98.0)
// =============================================================================

/// Context engine for semantic retrieval (v0.100.0: minimal AGENTS.md loading)
pub struct ContextEngine {
    project_path: PathBuf,
}

impl ContextEngine {
    pub fn new(project_path: impl Into<PathBuf>) -> Result<Self> {
        let path = project_path.into();
        debug!("Creating context engine for project: {}", path.display());
        Ok(Self { project_path: path })
    }

    /// Load AGENTS.md for system prompt
    pub fn load_agents_md(&self) -> Result<String> {
        let agents_path = self.project_path.join("AGENTS.md");
        debug!("Loading AGENTS.md from: {}", agents_path.display());

        std::fs::read_to_string(&agents_path)
            .map_err(|e| anyhow::anyhow!("Failed to load AGENTS.md: {}", e))
    }

    /// Get project path
    pub fn project_path(&self) -> &PathBuf {
        &self.project_path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shared_contract_types() {
        // Verify Symbol can be created
        let symbol = Symbol {
            name: "test_function".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            range: (0, 0, 5, 0),
        };
        assert_eq!(symbol.name, "test_function");
        assert_eq!(symbol.kind, SymbolKind::Function);
    }

    #[test]
    fn test_code_chunk_creation() {
        let chunk = CodeChunk {
            file: PathBuf::from("test.rs"),
            range: (0, 0, 5, 0),
            text: "fn test() {}".to_string(),
            symbol_ids: vec!["test_function".to_string()],
        };
        assert_eq!(chunk.symbol_ids.len(), 1);
    }

    #[test]
    fn test_context_creation() {
        let ctx = ContextEngine::new("/tmp/test").unwrap();
        assert_eq!(*ctx.project_path(), PathBuf::from("/tmp/test"));
    }
}
