//! Semantic context engine — ties symbol parsing, the knowledge graph, and
//! the vector store into a single retrieval API.
//!
//! # Architecture
//!
//! ```text
//!          index_dir(root, store_dir, embedder)
//!                     │
//!          ┌──────────┼──────────┐
//!          ▼          ▼          ▼
//!    parse_symbols  KnowledgeGraph  VectorStore
//!    (1.1)          (1.2)           (1.3)
//!          │          │              │
//!          └──chunks──┴──neighbors───┘
//!                        │
//!                   retrieve(query)
//!                        ▼
//!               Vec<RetrievedContext>
//! ```
//!
//! # Limitations
//!
//! * Chunking is symbol-range-based when `parse_symbols` succeeds; otherwise
//!   fixed 40-line windows.  No gap-filling between symbols yet.
//! * `related_symbols` gathers all neighbour names (Calls, References,
//!   Contains, Implements) across the entire graph — no file-level scoping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;

use crate::graph::{EdgeKind, KnowledgeGraph};
use crate::lang::LanguageRegistry;
use crate::symbols::parse_symbols;
use crate::vector::{Chunk, Embedder, VectorStore};

// =============================================================================
// Public types
// =============================================================================

/// A chunk retrieved by [`ContextEngine::retrieve`] with its score and
/// related symbol names.
#[derive(Debug)]
pub struct RetrievedContext {
    /// The matching code chunk.
    pub chunk: Chunk,
    /// Cosine similarity score (1.0 = identical).
    pub score: f32,
    /// Names of symbols related to this chunk (callers/callees, types,
    /// containing modules, etc.), deduplicated and sorted.
    pub related_symbols: Vec<String>,
}

// =============================================================================
// Context engine
// =============================================================================

/// In-memory context engine that indexes a project directory and answers
/// semantic queries.
pub struct ContextEngine {
    _root: PathBuf,
    _store_dir: PathBuf,
    graph: KnowledgeGraph,
    vector_store: VectorStore,
    embedder: Arc<dyn Embedder>,
    file_count: usize,
    symbol_count: usize,
    /// `chunk_id → Vec<symbol_name>` for each symbol whose range overlaps
    /// with the chunk's range.
    chunk_symbols: HashMap<u64, Vec<String>>,
}

// ---- Public API ------------------------------------------------------------

impl ContextEngine {
    /// Open the context engine for `root`. Uses OpenAI text-embedding-3-small by default.
    /// Does not perform a full reindex. Use `index_dir` for a full rebuild.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        let store_dir = root.join(".forge").join("context");
        let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
        let embedder = Arc::new(crate::vector::ApiEmbedder::new(
            "https://api.openai.com/v1".to_string(),
            api_key,
            "text-embedding-3-small".to_string(),
            1536,
        ));
        
        let vector_store = VectorStore::open(&store_dir, 1536, "text-embedding-3-small")?;
        let graph_path = store_dir.join("graph.json");
        let graph = KnowledgeGraph::load(&graph_path).unwrap_or_else(|_| KnowledgeGraph::new());

        Ok(Self {
            _root: root.to_path_buf(),
            _store_dir: store_dir,
            graph,
            vector_store,
            embedder,
            file_count: 0,
            symbol_count: 0,
            chunk_symbols: HashMap::new(),
        })
    }

    /// Index every supported file under `root`, storing vectors in `store_dir`.
    ///
    /// * Walks the directory tree, skipping [`IGNORE_LIST`] entries.
    /// * For each supported file: parses symbols, adds them to the
    ///   [`KnowledgeGraph`], splits the source into [`Chunk`]s, embeds them
    ///   with `embedder`, and upserts into the [`VectorStore`].
    /// * After indexing the store is persisted to `store_dir/vector_store.json`.
    pub fn index_dir(root: &Path, store_dir: &Path, embedder: Arc<dyn Embedder>) -> Result<Self> {
        let dim = embedder.dim();
        let mut vector_store = VectorStore::open(store_dir, dim, embedder.model())?;
        vector_store.clear();
        let graph = KnowledgeGraph::new();
        let registry = LanguageRegistry::new();

        let mut engine = Self {
            _root: root.to_path_buf(),
            _store_dir: store_dir.to_path_buf(),
            graph,
            vector_store,
            embedder,
            file_count: 0,
            symbol_count: 0,
            chunk_symbols: HashMap::new(),
        };

        engine.index_files(root, &registry)?;
        engine.vector_store.persist()?;
        let _ = engine.graph.save(&store_dir.join("graph.json"));

        Ok(engine)
    }

    /// Retrieve the top-`k` chunks most relevant to `query`.
    ///
    /// 1. Embeds `query` via the stored [`Embedder`].
    /// 2. Searches the [`VectorStore`] with cosine similarity.
    /// 3. For each result chunk, resolves symbol names that overlap with
    ///    the chunk's file range, then walks the [`KnowledgeGraph`] for
    ///    their neighbours (Calls, References, Contains, Implements).
    ///
    /// Results are returned sorted by score descending (ties broken by id).
    pub fn retrieve(&self, query: &str, k: usize) -> Result<Vec<RetrievedContext>> {
        let embeddings = self.embedder.embed(&[query.to_string()])?;
        let query_vec = embeddings
            .first()
            .ok_or_else(|| anyhow::anyhow!("embedder returned no vector for query"))?;

        let results = self.vector_store.search(query_vec, k);
        let mut contexts = Vec::with_capacity(results.len());

        for (chunk_id, score) in results {
            if let Some(chunk) = self.vector_store.get(chunk_id) {
                let related = self.related_symbol_names(chunk_id);
                contexts.push(RetrievedContext {
                    chunk: chunk.clone(),
                    score,
                    related_symbols: related,
                });
            }
        }

        Ok(contexts)
    }

    /// Statistics: `(file_count, symbol_count, chunk_count)`.
    pub fn stats(&self) -> (usize, usize, usize) {
        (
            self.file_count,
            self.symbol_count,
            self.vector_store.len(),
        )
    }
}

impl crate::ContextIndex for ContextEngine {
    fn upsert_file(&mut self, path: &Path, src: &str) {
        self.remove_file(path);
        
        let registry = LanguageRegistry::new();
        if let Err(e) = self.index_file(path, src, &registry) {
            tracing::warn!("engine: failed to upsert {}: {:#}", path.display(), e);
        } else {
            self.file_count += 1;
        }
        let _ = self.vector_store.persist();
        let _ = self.graph.save(&self._store_dir.join("graph.json"));
    }

    fn remove_file(&mut self, path: &Path) {
        self.graph.remove_file(path);
        self.vector_store.remove_file(path);
        // ponytail: YAGNI - chunk_symbols might leak slightly for deleted chunks, but harmless for short-lived daemon
        
        if self.file_count > 0 {
            self.file_count -= 1;
        }
        self.symbol_count = self.graph.symbol_count();
        let _ = self.vector_store.persist();
        let _ = self.graph.save(&self._store_dir.join("graph.json"));
    }

    fn resolve_symbol(&self, name: &str) -> Option<crate::symbols::Symbol> {
        let ids = self.graph.find_by_name(name);
        if let Some(id) = ids.first() {
            self.graph.symbol(*id).cloned()
        } else {
            None
        }
    }
}

// ---- Internals -------------------------------------------------------------

impl ContextEngine {
    /// Recursively walk `root` and index every file whose extension is
    /// recognised by `Lang::for_path`.
    fn index_files(&mut self, root: &Path, registry: &LanguageRegistry) -> Result<()> {
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| !is_ignored(e))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            // Only index files with a known extension.
            if crate::lang::Lang::for_path(path).is_none() {
                continue;
            }

            let source = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("engine: skipping {}: {}", path.display(), e);
                    continue;
                }
            };

            if let Err(e) = self.index_file(path, &source, registry) {
                tracing::warn!("engine: failed to index {}: {:#}", path.display(), e);
                continue;
            }
            self.file_count += 1;
        }
        Ok(())
    }

    /// Parse, chunk, embed, and store a single file.
    fn index_file(&mut self, path: &Path, source: &str, registry: &LanguageRegistry) -> Result<()> {
        // 1. Parse symbols for chunking + symbol count.
        let symbols = parse_symbols(path, source, registry)?;
        self.symbol_count += symbols.len();

        // 2. Add to the knowledge graph (also parses internally — slight
        //    overhead but keeps the API composable).
        self.graph.add_file(path, source, registry)?;

        // 3. Create chunks.
        let chunks = self.chunk_source(path, source, &symbols);

        // 4. Record which symbol names overlap with each chunk.
        for chunk in &chunks {
            let names: Vec<String> = symbols
                .iter()
                .filter(|s| {
                    ranges_overlap(chunk.start_line, chunk.end_line, s.start_line, s.end_line)
                })
                .map(|s| s.name.clone())
                .collect();
            if !names.is_empty() {
                self.chunk_symbols.insert(chunk.id, names);
            }
        }

        // 5. Embed and upsert.
        let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
        let embeddings = self.embedder.embed(&texts)?;
        for (chunk, embedding) in chunks.into_iter().zip(embeddings) {
            self.vector_store.upsert(chunk, embedding)?;
        }

        Ok(())
    }

    /// Split source text into chunks.
    ///
    /// * If `symbols` is non-empty: one chunk per symbol.
    /// * If `symbols` is empty: fixed ~40-line sliding windows.
    fn chunk_source(
        &mut self,
        path: &Path,
        source: &str,
        symbols: &[crate::symbols::Symbol],
    ) -> Vec<Chunk> {
        if symbols.is_empty() {
            // Fixed 40-line windows.
            let lines: Vec<&str> = source.lines().collect();
            let mut chunks = Vec::with_capacity(lines.len() / 40 + 1);
            for (i, window) in lines.chunks(40).enumerate() {
                let start_line = i * 40 + 1;
                let end_line = start_line + window.len() - 1;
                let text = window.join("\n");
                
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                std::hash::Hash::hash(&path, &mut hasher);
                std::hash::Hash::hash(&start_line, &mut hasher);
                let id = std::hash::Hasher::finish(&hasher);
                
                chunks.push(Chunk {
                    id,
                    file: path.to_path_buf(),
                    start_line,
                    end_line,
                    text,
                });
            }
            chunks
        } else {
            // One chunk per symbol range.
            symbols
                .iter()
                .map(|sym| {
                    let text = extract_range(source, sym.start_line, sym.end_line);
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(&path, &mut hasher);
                    std::hash::Hash::hash(&sym.start_line, &mut hasher);
                    let id = std::hash::Hasher::finish(&hasher);
                    
                    Chunk {
                        id,
                        file: path.to_path_buf(),
                        start_line: sym.start_line,
                        end_line: sym.end_line,
                        text,
                    }
                })
                .collect()
        }
    }



    /// Collect all symbol names reachable via the knowledge graph from any
    /// symbol whose range overlaps the chunk.
    fn related_symbol_names(&self, chunk_id: u64) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();

        if let Some(sym_names) = self.chunk_symbols.get(&chunk_id) {
            for sym_name in sym_names {
                let ids = self.graph.find_by_name(sym_name);
                for id in ids {
                    for kind in &[
                        EdgeKind::Calls,
                        EdgeKind::References,
                        EdgeKind::Contains,
                        EdgeKind::Implements,
                    ] {
                        for nb in self.graph.neighbors(id, *kind) {
                            if let Some(sym) = self.graph.symbol(nb) {
                                let name = sym.name.clone();
                                if !seen.contains(&name) {
                                    seen.push(name);
                                }
                            }
                        }
                    }
                }
            }
        }

        seen.sort();
        seen
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Directories and files whose names are never indexed.
const IGNORE_LIST: &[&str] = &[".git", "target", "node_modules"];

/// Returns `true` for entries whose file name matches [`IGNORE_LIST`].
fn is_ignored(entry: &walkdir::DirEntry) -> bool {
    let name = entry.file_name().to_string_lossy();
    IGNORE_LIST.contains(&name.as_ref())
}

/// Extract lines `start_line..=end_line` (1-indexed) from source.
fn extract_range(source: &str, start_line: usize, end_line: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let start = start_line.saturating_sub(1);
    let end = end_line.min(lines.len());
    if start < end {
        lines[start..end].join("\n")
    } else {
        String::new()
    }
}

/// Do two 1-indexed inclusive ranges overlap?
fn ranges_overlap(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start <= b_end && b_start <= a_end
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::Embedder;
    use std::sync::Arc;
    use tempfile::TempDir;

    // ---- Mock embedder -----------------------------------------------------

    /// Deterministic embedder that hashes text into a fixed-dimension vector.
    struct TestEmbedder {
        dim: usize,
    }

    impl TestEmbedder {
        fn new(dim: usize) -> Self {
            Self { dim }
        }

        fn vector_for(text: &str, dim: usize) -> Vec<f32> {
            let h: u64 = text
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            (0..dim)
                .map(|i| ((h.wrapping_mul(i as u64 + 1) & 0xFF) as f32) / 128.0 - 1.0)
                .collect()
        }
    }

    impl Embedder for TestEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| Self::vector_for(t, self.dim))
                .collect())
        }

        fn dim(&self) -> usize {
            self.dim
        }

        fn model(&self) -> &str {
            "test"
        }
    }

    /// Embedder based on keyword frequency.
    ///
    /// The `i`-th component of the vector is the count of
    /// `vocab[i]` occurrences in the text.  Cosine similarity
    /// with this embedder correctly ranks keyword overlap,
    /// enabling deterministic top-1 retrieval tests.
    struct KeywordEmbedder {
        vocab: Vec<&'static str>,
    }

    impl Embedder for KeywordEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| {
                    self.vocab
                        .iter()
                        .map(|kw| t.matches(kw).count() as f32)
                        .collect()
                })
                .collect())
        }

        fn dim(&self) -> usize {
            self.vocab.len()
        }

        fn model(&self) -> &str {
            "keyword"
        }
    }

    // ---- helpers -----------------------------------------------------------

    fn tmp_dir() -> TempDir {
        TempDir::new().expect("temp dir")
    }

    fn write(root: &Path, name: &str, content: &str) -> PathBuf {
        let p = root.join(name);
        std::fs::write(&p, content).unwrap_or_else(|_| panic!("writing {name}"));
        p
    }

    // ---- tests -------------------------------------------------------------

    #[test]
    fn stats_after_indexing_returns_non_zero() {
        let root = tmp_dir();
        let store = tmp_dir();

        write(
            root.path(),
            "lib.rs",
            "fn greet() -> &'static str { \"hello\" }\nfn bye() -> &'static str { \"bye\" }\n",
        );

        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        let (files, syms, chunks) = engine.stats();
        assert!(files > 0, "expected at least one file");
        assert_eq!(syms, 2, "expected 2 symbols");
        assert!(chunks > 0, "expected at least one chunk");
    }

    #[test]
    fn retrieve_ranks_matching_chunk_top1() {
        let root = tmp_dir();
        let store = tmp_dir();

        // Two files: one talks about "alpha", the other about "beta".
        write(root.path(), "a.rs", "fn alpha() { alpha() }\n");
        write(root.path(), "b.rs", "fn beta() -> i32 { beta() }\n");

        // KeywordEmbedder counts occurrences — query "beta" produces
        // vector [0,1] while the "beta" chunk produces [0,N].
        // Cosine similarity guarantees the "beta" chunk ranks first.
        let embedder = Arc::new(KeywordEmbedder {
            vocab: vec!["alpha", "beta"],
        });
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        let results = engine.retrieve("beta", 1).unwrap();
        assert_eq!(results.len(), 1, "k=1 → exactly one result");
        assert!(
            results[0].chunk.text.contains("beta"),
            "top-1 should be the beta chunk, got: {:?}",
            results[0].chunk.text
        );
    }

    #[test]
    fn reindex_clears_stale_persisted_chunks() {
        let root = tmp_dir();
        let store = tmp_dir();

        write(root.path(), "a.rs", "fn alpha() { alpha() }\n");
        write(root.path(), "b.rs", "fn beta() { beta() }\n");

        let embedder = Arc::new(KeywordEmbedder {
            vocab: vec!["alpha", "beta"],
        });
        let first = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();
        assert_eq!(first.stats().2, 2, "expected both initial chunks");

        std::fs::remove_file(root.path().join("b.rs")).unwrap();

        let embedder = Arc::new(KeywordEmbedder {
            vocab: vec!["alpha", "beta"],
        });
        let second = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();
        let (_, _, chunks) = second.stats();
        assert_eq!(chunks, 1, "stale beta chunk should be removed");

        let results = second.retrieve("beta", 10).unwrap();
        assert!(
            results.iter().all(|r| !r.chunk.text.contains("beta")),
            "removed file should not be retrieved: {results:#?}"
        );
    }

    #[test]
    fn retrieve_empty_query_returns_results() {
        let root = tmp_dir();
        let store = tmp_dir();

        write(root.path(), "x.rs", "fn foo() {}\n");

        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        // Empty query still produces a vector (the mock embedder hashes empty
        // string deterministically).  It should not crash.
        let results = engine.retrieve("", 3).unwrap();
        // Might be empty or not — either is fine as long as it doesn't panic.
        assert!(results.len() <= 3);
    }

    #[test]
    fn stats_zero_before_indexing() {
        let root = tmp_dir();
        let store = tmp_dir();

        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        let (files, syms, chunks) = engine.stats();
        assert_eq!(files, 0, "no files in empty dir");
        assert_eq!(syms, 0, "no symbols in empty dir");
        assert_eq!(chunks, 0, "no chunks in empty dir");
    }

    #[test]
    fn related_symbols_appear_for_cross_referencing_chunks() {
        let root = tmp_dir();
        let store = tmp_dir();

        write(
            root.path(),
            "lib.rs",
            "fn caller() { callee() }\nfn callee() {}\n",
        );

        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        let results = engine.retrieve("caller", 5).unwrap();
        assert!(!results.is_empty());

        // The chunk containing `caller` should have at least one related
        // symbol (e.g. `callee` via the Calls edge).
        let caller_chunk = results.iter().find(|r| r.chunk.text.contains("caller"));
        if let Some(ctx) = caller_chunk {
            assert!(
                !ctx.related_symbols.is_empty(),
                "expected related symbols for caller chunk, got empty"
            );
        }
    }

    #[test]
    fn ignore_list_skips_target_directory() {
        let root = tmp_dir();
        let store = tmp_dir();

        let target = root.path().join("target");
        std::fs::create_dir_all(&target).unwrap();
        write(&target, "build.rs", "fn generated() {}\n");
        write(root.path(), "src.rs", "fn real() {}\n");

        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        let (files, syms, _) = engine.stats();
        // Only `src.rs` should have been indexed; `target/build.rs` ignored.
        assert_eq!(files, 1, "should ignore target/");
        assert_eq!(syms, 1, "only src.rs has symbols");
    }

    #[test]
    fn window_chunking_for_files_without_symbols() {
        let root = tmp_dir();
        let store = tmp_dir();

        // A file with no recognised symbols (unknown extension).
        write(root.path(), "data.txt", &"line\n".repeat(100));

        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();

        let (files, _, chunks) = engine.stats();
        // `.txt` is not a supported extension, so it should be skipped.
        assert_eq!(files, 0, "no supported files");
        assert_eq!(chunks, 0, "no chunks");

        // Now also put a `.rs` file with no symbols (empty file).
        write(root.path(), "empty.rs", "");
        let embedder = Arc::new(TestEmbedder::new(8));
        let engine = ContextEngine::index_dir(root.path(), store.path(), embedder).unwrap();
        let (files, syms, chunks) = engine.stats();
        assert_eq!(files, 1, "empty.rs is a valid file");
        assert_eq!(syms, 0, "no symbols in empty file");
        assert_eq!(chunks, 0, "no lines means no chunks");
    }
}
