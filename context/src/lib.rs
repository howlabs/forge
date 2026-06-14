//! Forge Context Engine
//!
//! Provides semantic context indexing and retrieval for code understanding.

use anyhow::Result;
use fnv::FnvHasher;
use petgraph::graph::{DiGraph, NodeIndex};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing::debug;
use tree_sitter::{Node, Parser};

pub mod agents;
pub mod engine;
pub mod graph;
pub mod lang;
pub mod symbols;
pub mod vector;

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
pub trait ContextIndex: Send + Sync {
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
// MOCK CONTEXTINDEX - Development/Test implementation (Track B: v0.150.0)
// =============================================================================

/// Mock in-memory ContextIndex for Track B development/testing
/// Will be replaced by Track A's SemanticContextEngine after integration
#[derive(Debug, Clone)]
pub struct MockContextIndex {
    /// Map file path -> source content
    files: Arc<Mutex<HashMap<PathBuf, String>>>,
    /// Map symbol name -> Symbol metadata
    symbols: Arc<Mutex<HashMap<String, Symbol>>>,
    /// Map chunk id -> CodeChunk
    chunks: Arc<Mutex<HashMap<String, CodeChunk>>>,
}

impl MockContextIndex {
    pub fn new() -> Self {
        Self {
            files: Arc::new(Mutex::new(HashMap::new())),
            symbols: Arc::new(Mutex::new(HashMap::new())),
            chunks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Extract simple symbols from source (very basic extraction for mock)
    fn extract_symbols(path: &Path, src: &str) -> Vec<Symbol> {
        let mut symbols = Vec::new();
        let lines: Vec<&str> = src.lines().collect();

        for (line_no, line) in lines.iter().enumerate() {
            // Very basic pattern matching for common definitions
            if line.contains("fn ") && !line.trim_start().starts_with("//") {
                if let Some(name) = line.split("fn ").nth(1) {
                    if let Some(name) = name.split('(').next() {
                        let name = name.trim().to_string();
                        if !name.is_empty() {
                            symbols.push(Symbol {
                                name: name.clone(), // Use simple name without file prefix
                                kind: SymbolKind::Function,
                                file: path.to_path_buf(),
                                range: (line_no, 0, line_no, line.len()),
                            });
                        }
                    }
                }
            }

            if line.contains("struct ") && !line.trim_start().starts_with("//") {
                if let Some(name) = line.split("struct ").nth(1) {
                    if let Some(name) = name.split('{').next() {
                        let name = name.trim().to_string();
                        if !name.is_empty() {
                            symbols.push(Symbol {
                                name: name.clone(), // Use simple name without file prefix
                                kind: SymbolKind::Struct,
                                file: path.to_path_buf(),
                                range: (line_no, 0, line_no, line.len()),
                            });
                        }
                    }
                }
            }
        }

        symbols
    }

    /// Create chunks from source (very basic chunking for mock)
    fn create_chunks(path: &Path, src: &str) -> Vec<CodeChunk> {
        vec![CodeChunk {
            file: path.to_path_buf(),
            range: (0, 0, src.lines().count(), 0),
            text: src.to_string(),
            symbol_ids: Vec::new(),
        }]
    }
}

impl Default for MockContextIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextIndex for MockContextIndex {
    fn upsert_file(&mut self, path: &Path, src: &str) {
        debug!("MockContextIndex: upsert_file {}", path.display());

        // Remove old data for this file
        self.remove_file(path);

        // Store new content
        let mut files = self.files.lock().unwrap();
        files.insert(path.to_path_buf(), src.to_string());
        drop(files);

        // Extract and store symbols
        let symbols = Self::extract_symbols(path, src);
        let mut symbols_map = self.symbols.lock().unwrap();
        for symbol in symbols {
            debug!("  Extracted symbol: {}", symbol.name);
            symbols_map.insert(symbol.name.clone(), symbol);
        }
        drop(symbols_map);

        // Create and store chunks
        let chunks = Self::create_chunks(path, src);
        let mut chunks_map = self.chunks.lock().unwrap();
        for (i, chunk) in chunks.into_iter().enumerate() {
            let chunk_id = format!("{}:{}", path.display(), i);
            chunks_map.insert(chunk_id, chunk);
        }
    }

    fn remove_file(&mut self, path: &Path) {
        debug!("MockContextIndex: remove_file {}", path.display());

        // Remove file content
        let mut files = self.files.lock().unwrap();
        files.remove(path);
        drop(files);

        // Remove symbols from this file
        let mut symbols_map = self.symbols.lock().unwrap();
        symbols_map.retain(|_, symbol| symbol.file != path);
        drop(symbols_map);

        // Remove chunks from this file
        let mut chunks_map = self.chunks.lock().unwrap();
        chunks_map.retain(|_, chunk| chunk.file != path);
    }

    fn query(&self, q: &str, k: usize) -> Vec<CodeChunk> {
        debug!("MockContextIndex: query '{}', k={}", q, k);

        // Very basic query: return all chunks (no semantic search in mock)
        let chunks = self.chunks.lock().unwrap();
        let values: Vec<CodeChunk> = chunks.values().cloned().collect();
        values.into_iter().take(k).collect()
    }

    fn resolve_symbol(&self, name: &str) -> Option<Symbol> {
        debug!("MockContextIndex: resolve_symbol '{}'", name);

        let symbols = self.symbols.lock().unwrap();
        symbols.get(name).cloned()
    }
}

// =============================================================================
// VECTOR STORE - Semantic search with embeddings (Track A: v0.100.0)
// =============================================================================

/// Simple vector store for semantic code search
/// For MVP, we use basic token overlap + TF-IDF like scoring
/// In production, this would use actual embeddings (e.g., sentence-transformers)
pub struct VectorStore {
    /// Chunk ID -> Chunk with embedding
    chunks: HashMap<String, (CodeChunk, Vec<f32>)>,

    /// Inverted index: token -> [chunk_ids]
    token_index: HashMap<String, HashSet<String>>,

    /// For generating unique chunk IDs
    id_counter: u64,
}

impl VectorStore {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
            token_index: HashMap::new(),
            id_counter: 0,
        }
    }

    /// Add a code chunk to the store
    pub fn add_chunk(&mut self, chunk: CodeChunk) {
        let chunk_id = self.generate_chunk_id(&chunk);
        let embedding = self.compute_embedding(&chunk.text);
        let tokens = self.tokenize(&chunk.text);

        // Add inverted index entries
        for token in tokens {
            self.token_index
                .entry(token)
                .or_default()
                .insert(chunk_id.clone());
        }

        self.chunks.insert(chunk_id, (chunk, embedding));
    }

    /// Search for chunks similar to query
    pub fn search(&self, query: &str, k: usize) -> Vec<CodeChunk> {
        let query_tokens = self.tokenize(query);

        // Score chunks by token overlap
        let mut scored_chunks: Vec<_> = self
            .chunks
            .iter()
            .map(|(_chunk_id, (chunk, _embedding))| {
                let score = self.compute_similarity(query, chunk, &query_tokens);
                (score, chunk.clone())
            })
            .filter(|(score, _)| *score > 0.0)
            .collect();

        // Sort by score descending
        scored_chunks.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Return top-k
        scored_chunks
            .into_iter()
            .take(k)
            .map(|(_, chunk)| chunk)
            .collect()
    }

    /// Remove all chunks from a file
    pub fn remove_file(&mut self, path: &Path) {
        let to_remove: Vec<String> = self
            .chunks
            .iter()
            .filter(|(id, _)| self.chunks[*id].0.file == path)
            .map(|(id, _)| id.clone())
            .collect();

        for chunk_id in to_remove {
            self.remove_chunk(&chunk_id);
        }
    }

    /// Remove a chunk by ID
    fn remove_chunk(&mut self, chunk_id: &str) {
        if let Some((chunk, _)) = self.chunks.remove(chunk_id) {
            // Update inverted index
            let tokens = self.tokenize(&chunk.text);
            for token in tokens {
                if let Some(ids) = self.token_index.get_mut(&token) {
                    ids.remove(chunk_id);
                    if ids.is_empty() {
                        self.token_index.remove(&token);
                    }
                }
            }
        }
    }

    /// Compute a simple embedding using token hashing (MVP approach)
    fn compute_embedding(&self, text: &str) -> Vec<f32> {
        // For MVP, we create a simple 64-dimensional embedding
        // In production, use actual embeddings (e.g., from sentence-transformers)
        let tokens = self.tokenize(text);
        let mut embedding = vec![0.0f32; 64];

        for (i, token) in tokens.iter().enumerate() {
            let mut hasher = FnvHasher::default();
            token.hash(&mut hasher);
            let hash = hasher.finish();

            let idx = (hash % 64) as usize;
            embedding[idx] += 1.0 / (i + 1) as f32; // Weight by position
        }

        // Normalize
        let norm: f32 = embedding.iter().map(|&x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            embedding.iter_mut().for_each(|x| *x /= norm);
        }

        embedding
    }

    /// Tokenize text into words
    fn tokenize(&self, text: &str) -> Vec<String> {
        // Simple tokenization: keep identifiers with underscores
        let tokens: Vec<String> = text
            .split(|c: char| {
                c.is_whitespace()
                    || c == '('
                    || c == ')'
                    || c == '{'
                    || c == '}'
                    || c == ';'
                    || c == ':'
            })
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .flat_map(|s| {
                // Split camelCase and snake_case
                let mut parts = Vec::new();
                let mut current = String::new();

                for ch in s.chars() {
                    if ch == '_' {
                        if !current.is_empty() {
                            parts.push(current.to_lowercase());
                            current = String::new();
                        }
                    } else if ch.is_uppercase() {
                        if !current.is_empty() {
                            parts.push(current.to_lowercase());
                        }
                        current = ch.to_lowercase().to_string();
                    } else if ch.is_alphanumeric() {
                        current.push(ch);
                    }
                }

                if !current.is_empty() {
                    parts.push(current.to_lowercase());
                }

                parts
            })
            .filter(|s| s.len() > 2) // Skip very short tokens
            .collect();

        tokens
    }

    /// Compute similarity between query and chunk (simplified TF-IDF)
    fn compute_similarity(&self, _query: &str, chunk: &CodeChunk, query_tokens: &[String]) -> f32 {
        let chunk_tokens = self.tokenize(&chunk.text);
        if chunk_tokens.is_empty() || query_tokens.is_empty() {
            return 0.0;
        }

        // Count tokens
        let mut query_tf: HashMap<&String, usize> = HashMap::new();
        for token in query_tokens {
            *query_tf.entry(token).or_insert(0) += 1;
        }

        let mut chunk_tf: HashMap<&String, usize> = HashMap::new();
        for token in &chunk_tokens {
            *chunk_tf.entry(token).or_insert(0) += 1;
        }

        // Compute cosine similarity (simplified)
        let mut dot_product = 0.0f32;
        for (token, query_count) in query_tf.iter() {
            if let Some(chunk_count) = chunk_tf.get(token) {
                dot_product += (*query_count * chunk_count) as f32;
            }
        }

        let query_norm: f32 = query_tf
            .values()
            .map(|&c| c as f32)
            .map(|c| c * c)
            .sum::<f32>()
            .sqrt();
        let chunk_norm: f32 = chunk_tf
            .values()
            .map(|&c| c as f32)
            .map(|c| c * c)
            .sum::<f32>()
            .sqrt();

        if query_norm > 0.0 && chunk_norm > 0.0 {
            dot_product / (query_norm * chunk_norm)
        } else {
            0.0
        }
    }

    /// Generate unique chunk ID
    fn generate_chunk_id(&mut self, chunk: &CodeChunk) -> String {
        let id = format!(
            "{}:{}-{}",
            chunk.file.display(),
            chunk.range.0,
            chunk.range.2
        );
        id
    }

    /// Get all chunks
    pub fn get_chunks(&self) -> Vec<CodeChunk> {
        self.chunks
            .values()
            .map(|(chunk, _)| chunk.clone())
            .collect()
    }

    /// Clear all chunks
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.token_index.clear();
        self.id_counter = 0;
    }
}

impl Default for VectorStore {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// KNOWLEDGE GRAPH - Track symbol relationships using petgraph
// =============================================================================

/// Knowledge graph tracking symbol relationships
pub struct KnowledgeGraph {
    /// Symbol ID -> Symbol metadata
    symbols: HashMap<String, Symbol>,

    /// Graph structure: NodeIndex corresponds to symbol ID
    graph: DiGraph<SymbolNode, SymbolEdge>,

    /// Symbol ID -> NodeIndex mapping
    symbol_nodes: HashMap<String, NodeIndex>,

    /// For generating unique symbol IDs
    id_counter: u64,
}

#[derive(Debug, Clone)]
struct SymbolNode {
    symbol_id: String,
}

#[derive(Debug, Clone)]
pub enum SymbolEdge {
    Calls,
}

impl KnowledgeGraph {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            graph: DiGraph::new(),
            symbol_nodes: HashMap::new(),
            id_counter: 0,
        }
    }

    /// Add a symbol to the graph
    pub fn add_symbol(&mut self, symbol: Symbol) -> String {
        let symbol_id = self.generate_symbol_id(&symbol);
        let node_idx = self.graph.add_node(SymbolNode {
            symbol_id: symbol_id.clone(),
        });

        self.symbols.insert(symbol_id.clone(), symbol);
        self.symbol_nodes.insert(symbol_id.clone(), node_idx);

        symbol_id
    }

    /// Add a relationship between symbols
    pub fn add_relationship(&mut self, from: &str, to: &str, edge: SymbolEdge) -> Result<()> {
        let from_idx = self
            .symbol_nodes
            .get(from)
            .ok_or_else(|| anyhow::anyhow!("Source symbol not found: {}", from))?;
        let to_idx = self
            .symbol_nodes
            .get(to)
            .ok_or_else(|| anyhow::anyhow!("Target symbol not found: {}", to))?;

        self.graph.add_edge(*from_idx, *to_idx, edge);
        Ok(())
    }

    /// Get symbol by ID
    pub fn get_symbol(&self, symbol_id: &str) -> Option<&Symbol> {
        self.symbols.get(symbol_id)
    }

    /// Get symbol by name (returns first match)
    pub fn find_by_name(&self, name: &str) -> Vec<&Symbol> {
        self.symbols.values().filter(|s| s.name == name).collect()
    }

    /// Get all symbols that reference this symbol
    pub fn get_references(&self, symbol_id: &str) -> Vec<&Symbol> {
        if let Some(&node_idx) = self.symbol_nodes.get(symbol_id) {
            self.graph
                .neighbors_directed(node_idx, petgraph::Direction::Incoming)
                .filter_map(|neighbor| {
                    let neighbor_node = &self.graph[neighbor];
                    self.symbols.get(&neighbor_node.symbol_id)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Get all symbols that this symbol references
    pub fn get_outgoing_references(&self, symbol_id: &str) -> Vec<&Symbol> {
        if let Some(&node_idx) = self.symbol_nodes.get(symbol_id) {
            self.graph
                .neighbors_directed(node_idx, petgraph::Direction::Outgoing)
                .filter_map(|neighbor| {
                    let neighbor_node = &self.graph[neighbor];
                    self.symbols.get(&neighbor_node.symbol_id)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Remove a symbol and all its relationships
    pub fn remove_symbol(&mut self, symbol_id: &str) {
        if let Some(_node_idx) = self.symbol_nodes.remove(symbol_id) {
            self.symbols.remove(symbol_id);
            // Node will be removed from graph when we rebuild or in next iteration
        }
    }

    /// Generate unique symbol ID
    fn generate_symbol_id(&mut self, symbol: &Symbol) -> String {
        // Create a stable ID based on file + name
        let mut hasher = FnvHasher::default();
        symbol.file.hash(&mut hasher);
        symbol.name.hash(&mut hasher);

        format!("{}::{}", symbol.file.display(), symbol.name)
    }

    /// Get all symbols in a file
    pub fn get_symbols_in_file(&self, path: &Path) -> Vec<&Symbol> {
        self.symbols.values().filter(|s| s.file == path).collect()
    }

    /// Remove all symbols from a file
    pub fn remove_file(&mut self, path: &Path) {
        let to_remove: Vec<String> = self
            .symbols
            .iter()
            .filter(|(_, s)| s.file == path)
            .map(|(id, _)| id.clone())
            .collect();

        for symbol_id in to_remove {
            self.remove_symbol(&symbol_id);
        }
    }

    /// Clear all symbols (useful for file reindexing)
    pub fn clear(&mut self) {
        self.symbols.clear();
        self.graph.clear();
        self.symbol_nodes.clear();
        self.id_counter = 0;
    }
}

impl Default for KnowledgeGraph {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// TREE-SITTER PARSER - Symbol extraction from source code
// =============================================================================

/// Tree-sitter based parser for extracting symbols from Rust code
pub struct TreeSitterParser {
    parser: Parser,
}

impl TreeSitterParser {
    pub fn new() -> Result<Self> {
        let mut parser = Parser::new();
        let language = tree_sitter_rust::language();
        parser
            .set_language(language)
            .map_err(|e| anyhow::anyhow!("Failed to set tree-sitter language: {}", e))?;

        Ok(Self { parser })
    }

    /// Parse source code and extract all symbols
    pub fn extract_symbols(&mut self, path: &Path, src: &str) -> Result<Vec<Symbol>> {
        let tree = self
            .parser
            .parse(src, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse source code"))?;

        let root_node = tree.root_node();
        let mut symbols = Vec::new();
        self.extract_symbols_recursive(path, src, root_node, &mut symbols);

        Ok(symbols)
    }

    /// Recursively extract symbols from AST nodes
    fn extract_symbols_recursive(
        &self,
        path: &Path,
        src: &str,
        node: Node,
        symbols: &mut Vec<Symbol>,
    ) {
        if node.is_named() {
            let kind = node.kind();

            if let Some(symbol_kind) = self.node_kind_to_symbol_kind(kind) {
                if let Some(symbol) = self.create_symbol(path, src, node, symbol_kind) {
                    symbols.push(symbol);
                }
            }
        }

        // Recurse into children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_symbols_recursive(path, src, child, symbols);
            }
        }
    }

    /// Convert tree-sitter node kind to SymbolKind
    fn node_kind_to_symbol_kind(&self, kind: &str) -> Option<SymbolKind> {
        match kind {
            "function_item" => Some(SymbolKind::Function),
            "struct_item" => Some(SymbolKind::Struct),
            "enum_item" => Some(SymbolKind::Enum),
            "trait_item" => Some(SymbolKind::Trait),
            "impl_item" => Some(SymbolKind::Impl),
            "mod_item" => Some(SymbolKind::Module),
            "type_item" => Some(SymbolKind::TypeAlias),
            "const_item" => Some(SymbolKind::Const),
            "static_item" => Some(SymbolKind::Static),
            "macro_definition" => Some(SymbolKind::Macro),
            _ => None,
        }
    }

    /// Create a Symbol from a tree-sitter node
    fn create_symbol(
        &self,
        path: &Path,
        src: &str,
        node: Node,
        kind: SymbolKind,
    ) -> Option<Symbol> {
        // Extract name from the appropriate child
        let name_node = match kind {
            SymbolKind::Function
            | SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::TypeAlias
            | SymbolKind::Const
            | SymbolKind::Static
            | SymbolKind::Macro => node.child_by_field_name("name"),
            SymbolKind::Impl => node.child_by_field_name("trait"),
            SymbolKind::Module => node.child_by_field_name("name"),
        };

        let name = name_node
            .map(|n| {
                let start = n.start_byte();
                let end = n.end_byte();
                &src[start..end]
            })?
            .to_string();

        let range = (
            node.start_position().row + 1, // 1-indexed lines
            node.start_position().column,
            node.end_position().row + 1,
            node.end_position().column,
        );

        Some(Symbol {
            name,
            kind,
            file: path.to_path_buf(),
            range,
        })
    }

    /// Extract code chunks from source code
    pub fn extract_chunks(&mut self, path: &Path, src: &str) -> Result<Vec<CodeChunk>> {
        let tree = self
            .parser
            .parse(src, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse source code for chunks"))?;

        let root_node = tree.root_node();
        let mut chunks = Vec::new();
        self.extract_chunks_recursive(path, src, root_node, &mut chunks);

        Ok(chunks)
    }

    /// Recursively extract code chunks
    fn extract_chunks_recursive(
        &self,
        path: &Path,
        src: &str,
        node: Node,
        chunks: &mut Vec<CodeChunk>,
    ) {
        // Extract chunks for function-level items (major semantic units)
        if node.is_named() {
            let kind = node.kind();
            if matches!(
                kind,
                "function_item" | "struct_item" | "enum_item" | "trait_item" | "impl_item"
            ) {
                let start_byte = node.start_byte();
                let end_byte = node.end_byte();
                let text = src[start_byte..end_byte].to_string();

                let range = (
                    node.start_position().row + 1,
                    node.start_position().column,
                    node.end_position().row + 1,
                    node.end_position().column,
                );

                // For now, empty symbol_ids - we'll populate from knowledge graph
                chunks.push(CodeChunk {
                    file: path.to_path_buf(),
                    range,
                    text,
                    symbol_ids: Vec::new(),
                });
            }
        }

        // Recurse into children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                self.extract_chunks_recursive(path, src, child, chunks);
            }
        }
    }
}

impl Default for TreeSitterParser {
    fn default() -> Self {
        Self::new().expect("Failed to create TreeSitterParser")
    }
}

// =============================================================================
// SEMANTIC CONTEXT ENGINE - Main implementation using tree-sitter + graph + vector store
// =============================================================================

/// Semantic context engine with advanced parsing and retrieval
pub struct SemanticContextEngine {
    /// Root directory of the project
    root: PathBuf,
    /// Vector store for semantic search
    vector_store: VectorStore,
    /// Knowledge graph for symbol relationships
    knowledge_graph: KnowledgeGraph,
    /// Symbol map: name -> Symbol
    symbols: HashMap<String, Symbol>,
    /// Rust tree-sitter parser
    rust_parser: Parser,
}

impl SemanticContextEngine {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let mut rust_parser = Parser::new();
        let language = tree_sitter_rust::language();
        rust_parser
            .set_language(language)
            .map_err(|e| anyhow::anyhow!("Failed to set tree-sitter language: {}", e))?;

        Ok(Self {
            root: root.into(),
            vector_store: VectorStore::new(),
            knowledge_graph: KnowledgeGraph::new(),
            symbols: HashMap::new(),
            rust_parser,
        })
    }

    /// Index all Rust files in the project root
    pub fn index_project(&mut self) -> Result<usize> {
        let mut count = 0;
        let root = self.root.clone();
        for entry in walkdir::WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
            .filter(|entry| {
                !entry
                    .path()
                    .components()
                    .any(|component| component.as_os_str() == "target")
            })
        {
            let path = entry.path().to_path_buf();
            if let Ok(src) = std::fs::read_to_string(&path) {
                self.upsert_file(&path, &src);
                count += 1;
            }
        }
        Ok(count)
    }

    /// Parse Rust source with tree-sitter and extract symbols
    fn parse_rust_symbols(&mut self, path: &Path, src: &str) -> Vec<Symbol> {
        let tree = match self.rust_parser.parse(src, None) {
            Some(tree) => tree,
            None => return vec![],
        };

        let mut symbols = Vec::new();
        let root_node = tree.root_node();
        Self::walk_node(root_node, src, path, &mut symbols);
        symbols
    }

    fn walk_node(node: Node, src: &str, path: &Path, symbols: &mut Vec<Symbol>) {
        let kind = match node.kind() {
            "function_item" => Some(SymbolKind::Function),
            "struct_item" => Some(SymbolKind::Struct),
            "enum_item" => Some(SymbolKind::Enum),
            "trait_item" => Some(SymbolKind::Trait),
            "impl_item" => Some(SymbolKind::Impl),
            "mod_item" => Some(SymbolKind::Module),
            "type_item" | "type_alias" => Some(SymbolKind::TypeAlias),
            "const_item" => Some(SymbolKind::Const),
            "static_item" => Some(SymbolKind::Static),
            "macro_definition" => Some(SymbolKind::Macro),
            _ => None,
        };

        if let Some(symbol_kind) = kind {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = &src[name_node.byte_range()];
                let start = node.start_position();
                let end = node.end_position();
                symbols.push(Symbol {
                    name: name.to_string(),
                    kind: symbol_kind,
                    file: path.to_path_buf(),
                    range: (start.row + 1, start.column, end.row + 1, end.column),
                });
            }
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            Self::walk_node(child, src, path, symbols);
        }
    }

    /// Chunk source into function-level pieces for VectorStore
    fn chunk_by_functions(path: &Path, src: &str, symbols: &[Symbol]) -> Vec<CodeChunk> {
        if symbols.is_empty() {
            return vec![CodeChunk {
                file: path.to_path_buf(),
                range: (0, 0, src.lines().count(), 0),
                text: src.to_string(),
                symbol_ids: vec![],
            }];
        }

        let lines: Vec<&str> = src.lines().collect();
        symbols
            .iter()
            .map(|symbol| {
                let (start_line, _, end_line, _) = symbol.range;
                let start_idx = start_line.saturating_sub(1);
                let end = end_line.min(lines.len());
                let text = if start_idx < end {
                    lines[start_idx..end].join("\n")
                } else {
                    String::new()
                };
                CodeChunk {
                    file: path.to_path_buf(),
                    range: symbol.range,
                    text,
                    symbol_ids: vec![symbol.name.clone()],
                }
            })
            .collect()
    }
}

// Re-export AGENTS.md discovery types
pub use agents::{AgentsDiscovery, AgentsFile};

impl Default for SemanticContextEngine {
    fn default() -> Self {
        Self::new(".").expect("Failed to create SemanticContextEngine")
    }
}

impl ContextIndex for SemanticContextEngine {
    fn upsert_file(&mut self, path: &Path, src: &str) {
        debug!("Upserting file: {}", path.display());
        self.remove_file(path);

        let symbols = self.parse_rust_symbols(path, src);
        for symbol in &symbols {
            self.symbols.insert(symbol.name.clone(), symbol.clone());
            self.knowledge_graph.add_symbol(symbol.clone());
        }

        let chunks = Self::chunk_by_functions(path, src, &symbols);
        for chunk in chunks {
            self.vector_store.add_chunk(chunk);
        }
    }

    fn remove_file(&mut self, path: &Path) {
        debug!("Removing file: {}", path.display());
        self.symbols.retain(|_, symbol| symbol.file != path);
        self.knowledge_graph.remove_file(path);
        self.vector_store.remove_file(path);
    }

    fn query(&self, q: &str, k: usize) -> Vec<CodeChunk> {
        debug!("Querying for: {} (top {})", q, k);
        self.vector_store.search(q, k)
    }

    fn resolve_symbol(&self, name: &str) -> Option<Symbol> {
        debug!("Resolving symbol: {}", name);
        self.symbols.get(name).cloned()
    }
}

/// Context engine for semantic retrieval.
pub struct ContextEngine {
    inner: SemanticContextEngine,
    root: PathBuf,
}

impl ContextEngine {
    pub fn new(project_path: impl Into<PathBuf>) -> Result<Self> {
        let root = project_path.into();
        debug!("Creating context engine for project: {}", root.display());
        let mut engine = SemanticContextEngine::new(&root)?;
        let count = engine.index_project()?;
        tracing::info!("Indexed {} Rust files", count);
        Ok(Self {
            inner: engine,
            root,
        })
    }

    /// Load AGENTS.md for system prompt
    pub fn load_agents_md(&self) -> Result<String> {
        agents::load_agents_md(&self.root).ok_or_else(|| anyhow::anyhow!("No AGENTS.md found"))
    }

    pub fn query_context(&self, task: &str, k: usize) -> Vec<CodeChunk> {
        self.inner.query(task, k)
    }

    pub fn resolve_symbol(&self, name: &str) -> Option<Symbol> {
        self.inner.resolve_symbol(name)
    }

    /// Get project path
    pub fn project_path(&self) -> &PathBuf {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ==================== SHARED CONTRACT TESTS ====================

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

    // ==================== TREE-SITTER PARSER TESTS ====================

    #[test]
    fn test_parser_creation() {
        let parser = TreeSitterParser::new();
        assert!(parser.is_ok());
    }

    #[test]
    fn test_extract_symbols() {
        let mut parser = TreeSitterParser::new().unwrap();
        let src = r#"
            fn test_function() -> i32 {
                42
            }

            struct TestStruct {
                field: i32,
            }

            enum TestEnum {
                A,
                B,
            }
        "#;

        let path = PathBuf::from("test.rs");
        let symbols = parser.extract_symbols(&path, src).unwrap();

        assert!(symbols.len() >= 3); // At least function, struct, enum

        let has_function = symbols
            .iter()
            .any(|s| matches!(s.kind, SymbolKind::Function));
        let has_struct = symbols.iter().any(|s| matches!(s.kind, SymbolKind::Struct));
        let has_enum = symbols.iter().any(|s| matches!(s.kind, SymbolKind::Enum));

        assert!(has_function, "Should extract function");
        assert!(has_struct, "Should extract struct");
        assert!(has_enum, "Should extract enum");
    }

    #[test]
    fn test_extract_chunks() {
        let mut parser = TreeSitterParser::new().unwrap();
        let src = r#"
            fn test_function() -> i32 {
                42
            }

            fn another_function() -> String {
                "hello".to_string()
            }
        "#;

        let path = PathBuf::from("test.rs");
        let chunks = parser.extract_chunks(&path, src).unwrap();

        assert!(chunks.len() >= 2); // At least two functions
        assert!(chunks.iter().any(|c| c.text.contains("test_function")));
        assert!(chunks.iter().any(|c| c.text.contains("another_function")));
    }

    // ==================== KNOWLEDGE GRAPH TESTS ====================

    #[test]
    fn test_graph_creation() {
        let graph = KnowledgeGraph::new();
        assert!(graph.symbols.is_empty());
    }

    #[test]
    fn test_add_symbol() {
        let mut graph = KnowledgeGraph::new();
        let symbol = Symbol {
            name: "test_func".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            range: (1, 0, 5, 0),
        };

        let symbol_id = graph.add_symbol(symbol);
        assert_eq!(graph.symbols.len(), 1);
        assert!(graph.get_symbol(&symbol_id).is_some());
    }

    #[test]
    fn test_find_by_name() {
        let mut graph = KnowledgeGraph::new();
        let symbol = Symbol {
            name: "test_func".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            range: (1, 0, 5, 0),
        };

        graph.add_symbol(symbol);
        let matches = graph.find_by_name("test_func");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].name, "test_func");
    }

    #[test]
    fn test_graph_relationships() {
        let mut graph = KnowledgeGraph::new();

        let caller = Symbol {
            name: "caller".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            range: (1, 0, 5, 0),
        };

        let callee = Symbol {
            name: "callee".to_string(),
            kind: SymbolKind::Function,
            file: PathBuf::from("test.rs"),
            range: (7, 0, 10, 0),
        };

        let caller_id = graph.add_symbol(caller);
        let callee_id = graph.add_symbol(callee);

        let result = graph.add_relationship(&caller_id, &callee_id, SymbolEdge::Calls);
        assert!(result.is_ok());
    }

    // ==================== VECTOR STORE TESTS ====================

    #[test]
    fn test_vector_store_creation() {
        let store = VectorStore::new();
        assert!(store.chunks.is_empty());
    }

    #[test]
    fn test_add_chunk() {
        let mut store = VectorStore::new();
        let chunk = CodeChunk {
            file: PathBuf::from("test.rs"),
            range: (1, 0, 5, 0),
            text: "fn database_connect() -> Connection".to_string(),
            symbol_ids: vec![],
        };

        store.add_chunk(chunk);
        assert_eq!(store.chunks.len(), 1);
    }

    #[test]
    fn test_semantic_search() {
        let mut store = VectorStore::new();

        // Add chunks with different content
        store.add_chunk(CodeChunk {
            file: PathBuf::from("db.rs"),
            range: (1, 0, 5, 0),
            text: "fn database_connection() -> Connection".to_string(),
            symbol_ids: vec![],
        });

        store.add_chunk(CodeChunk {
            file: PathBuf::from("utils.rs"),
            range: (1, 0, 5, 0),
            text: "fn helper_function() -> i32".to_string(),
            symbol_ids: vec![],
        });

        store.add_chunk(CodeChunk {
            file: PathBuf::from("db.rs"),
            range: (7, 0, 12, 0),
            text: "struct DatabaseConfig { url: String }".to_string(),
            symbol_ids: vec![],
        });

        // Search for database-related content
        let results = store.search("database connection", 2);

        assert!(results.len() <= 2);
        // The database-related chunk should be in results
        assert!(results.iter().any(|r| r.text.contains("database")));
    }

    #[test]
    fn test_tokenization() {
        let store = VectorStore::new();
        let tokens = store.tokenize("fn database_connection() -> Connection");

        // After tokenization, "database_connection" should be split into parts
        assert!(tokens.contains(&"database".to_string()));
        assert!(tokens.contains(&"connection".to_string()));
    }

    // ==================== SEMANTIC ENGINE TESTS ====================

    #[test]
    fn test_semantic_engine_creation() {
        let engine = SemanticContextEngine::new(".");
        assert!(engine.is_ok());
    }

    #[test]
    fn test_upsert_file() {
        let mut engine = SemanticContextEngine::new(".").unwrap();
        let path = PathBuf::from("test.rs");
        let src = r#"
            fn test_function() -> i32 {
                42
            }
        "#;

        engine.upsert_file(&path, src);

        // Verify symbols were extracted
        let symbols = engine.knowledge_graph.get_symbols_in_file(&path);
        assert!(!symbols.is_empty());
    }

    #[test]
    fn test_semantic_engine_indexes_rust_file() {
        let mut engine = SemanticContextEngine::new(".").unwrap();
        let path = PathBuf::from("semantic.rs");
        let src = r#"
            pub struct Customer {
                name: String,
            }

            pub fn load_customer() -> Customer {
                Customer { name: String::new() }
            }
        "#;

        engine.upsert_file(&path, src);

        let function = engine.resolve_symbol("load_customer").unwrap();
        let structure = engine.resolve_symbol("Customer").unwrap();
        assert_eq!(function.kind, SymbolKind::Function);
        assert_eq!(structure.kind, SymbolKind::Struct);
    }

    #[test]
    fn test_remove_file() {
        let mut engine = SemanticContextEngine::new(".").unwrap();
        let path = PathBuf::from("test.rs");
        let src = "fn test() {}".to_string();

        engine.upsert_file(&path, &src);
        assert!(engine.resolve_symbol("test").is_some());

        engine.remove_file(&path);
        assert!(engine.resolve_symbol("test").is_none());

        // Verify indices were cleared
        assert!(engine.knowledge_graph.get_symbols_in_file(&path).is_empty());
    }

    #[test]
    fn test_query_semantic() {
        let mut engine = SemanticContextEngine::new(".").unwrap();

        let path = PathBuf::from("test.rs");
        let src = r#"
            fn database_connect() -> Connection {
                Connection::new()
            }

            fn helper_function() -> i32 {
                42
            }
        "#;

        engine.upsert_file(&path, src);

        let results = engine.query("database", 5);
        assert!(results.len() <= 5);

        // Should find database-related content
        assert!(results.iter().any(|r| r.text.contains("database")));
    }

    #[test]
    fn test_semantic_engine_query_returns_relevant_chunks() {
        let mut engine = SemanticContextEngine::new(".").unwrap();
        let path = PathBuf::from("orders.rs");
        let src = r#"
            fn calculate_invoice_total() -> i32 {
                100
            }

            fn unrelated_helper() -> i32 {
                1
            }
        "#;

        engine.upsert_file(&path, src);
        let results = engine.query("invoice total", 1);

        assert_eq!(results.len(), 1);
        assert!(results[0].text.contains("calculate_invoice_total"));
    }

    #[test]
    fn test_resolve_symbol() {
        let mut engine = SemanticContextEngine::new(".").unwrap();

        let path = PathBuf::from("test.rs");
        let src = r#"
            fn database_connect() -> Connection {
                Connection::new()
            }
        "#;

        engine.upsert_file(&path, src);

        let symbol = engine.resolve_symbol("database_connect");
        assert!(symbol.is_some());
        assert_eq!(symbol.unwrap().name, "database_connect");
    }

    #[test]
    fn test_resolve_nonexistent_symbol() {
        let engine = SemanticContextEngine::new(".").unwrap();
        let symbol = engine.resolve_symbol("nonexistent_function");
        assert!(symbol.is_none());
    }

    // ==================== INTEGRATION TEST ====================

    #[test]
    fn test_full_context_index_workflow() {
        let mut engine = SemanticContextEngine::new(".").unwrap();

        // Upsert multiple files
        let db_path = PathBuf::from("database.rs");
        let db_src = r#"
            pub struct DatabaseConfig {
                pub url: String,
            }

            pub fn connect(config: DatabaseConfig) -> Connection {
                Connection::open(&config.url)
            }
        "#;

        let utils_path = PathBuf::from("utils.rs");
        let utils_src = r#"
            pub fn parse_url(s: &str) -> String {
                s.to_string()
            }
        "#;

        engine.upsert_file(&db_path, db_src);
        engine.upsert_file(&utils_path, utils_src);

        // Query for database-related code
        let results = engine.query("database connection", 3);
        assert!(!results.is_empty());

        // Resolve symbols
        let config_symbol = engine.resolve_symbol("DatabaseConfig");
        assert!(config_symbol.is_some());
        assert!(matches!(config_symbol.unwrap().kind, SymbolKind::Struct));

        let connect_symbol = engine.resolve_symbol("connect");
        assert!(connect_symbol.is_some());

        // Remove one file
        engine.remove_file(&db_path);

        // Query should now only return results from utils.rs
        let results_after = engine.query("parse", 5);
        assert!(results_after.iter().all(|r| r.file == utils_path));
    }

    // ==================== MOCK CONTEXTINDEX TESTS (Track B) ====================

    #[test]
    fn test_mock_context_index_upsert() {
        let mut index = MockContextIndex::new();

        let path = PathBuf::from("test.rs");
        let src = r#"
fn test_function() -> usize {
    42
}

struct TestStruct {
    value: usize,
}
"#;

        index.upsert_file(&path, src);

        // Check that file was stored
        let files = index.files.lock().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files.get(&path).unwrap(), src);
    }

    #[test]
    fn test_mock_context_index_remove() {
        let mut index = MockContextIndex::new();

        let path = PathBuf::from("test.rs");
        index.upsert_file(&path, "fn test() {}");

        index.remove_file(&path);

        // Check that file was removed
        let files = index.files.lock().unwrap();
        assert_eq!(files.len(), 0);
    }

    #[test]
    fn test_mock_context_index_resolve() {
        let mut index = MockContextIndex::new();

        let path = PathBuf::from("test.rs");
        let src = "fn test_function() {}";
        index.upsert_file(&path, src);

        // Try to resolve the function (note: simple name without file prefix)
        let symbol = index.resolve_symbol("test_function");

        assert!(symbol.is_some());
        let symbol = symbol.unwrap();
        assert_eq!(symbol.kind, SymbolKind::Function);
        assert_eq!(symbol.file, path);
    }

    #[test]
    fn test_mock_context_index_query() {
        let mut index = MockContextIndex::new();

        let path = PathBuf::from("test.rs");
        index.upsert_file(&path, "fn test() {}");

        let results = index.query("test", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file, path);
    }
}
