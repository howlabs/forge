//! On-disk vector store for code chunk embeddings.
//!
//! Stores [`Chunk`]s with their embedding vectors, persists to a single JSON
//! file, and supports brute-force cosine-similarity search (top-k).
//!
//! ## Limitations
//!
//! * Search is O(n·d) brute-force over all stored vectors.  ANN (HNSW, IVF,
//!   etc.) is future work.  For the expected corpus size (<10⁵ chunks) this
//!   is acceptable.
//! * Embeddings are L2-normalised at upsert time so cosine similarity
//!   reduces to a dot product.
//! * Persistence uses JSON (not bincode) for debuggability.  If the file
//!   grows beyond ~100 MiB, switch to a binary format.
//!
//! # Example
//!
//! ```ignore
//! use context::vector::{VectorStore, Chunk};
//! use std::path::Path;
//!
//! let mut store = VectorStore::open(Path::new("/tmp/my_store"), 384).unwrap();
//! let chunk = Chunk { id: 1, file: "lib.rs".into(), start_line: 1, end_line: 5, text: "fn foo() {}".into() };
//! store.upsert(chunk, vec![0.1; 384]).unwrap();
//! store.persist().unwrap();
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// =============================================================================
// Public types
// =============================================================================

/// A single contiguous piece of source code with metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    /// Unique identifier (caller-assigned; the store does not auto-generate).
    pub id: u64,
    /// File this chunk was extracted from.
    pub file: PathBuf,
    /// 1-indexed first line of the chunk.
    pub start_line: usize,
    /// 1-indexed last line of the chunk (inclusive).
    pub end_line: usize,
    /// The source text of the chunk.
    pub text: String,
}

/// Embedding engine: turns text into floating-point vectors.
///
/// Implementations must be deterministic for the same input.  The trait is
/// object-safe so callers can `Box` it if needed.
pub trait Embedder {
    /// Produce one embedding vector per input text.
    ///
    /// All returned vectors must have length equal to [`Self::dim`].
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Dimensionality of the embedding vectors returned by [`Self::embed`].
    fn dim(&self) -> usize;
}

// =============================================================================
// Persistent store
// =============================================================================

/// On-disk vector store backed by a single JSON file.
///
/// All operations are performed in memory; call [`persist`](Self::persist) to
/// flush to disk.
pub struct VectorStore {
    dir: PathBuf,
    dim: usize,
    chunks: HashMap<u64, Chunk>,
    embeddings: HashMap<u64, Vec<f32>>,
    dirty: bool,
}

// ---- serialisation schema (private) ----------------------------------------

#[derive(Serialize, Deserialize)]
struct StoreEntry {
    chunk: Chunk,
    embedding: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct StoreData {
    dim: usize,
    entries: Vec<StoreEntry>,
}

// ----------------------------------------------------------------------------
// Public API
// ----------------------------------------------------------------------------

impl VectorStore {
    /// Open (or create) a vector store rooted at `dir`.
    ///
    /// If `dir/vector_store.json` exists it is loaded.  Otherwise an empty
    /// store is created.  `dim` must match the dimensionality of the
    /// embeddings that will be inserted.
    pub fn open(dir: &Path, dim: usize) -> Result<Self> {
        let path = dir.join("vector_store.json");
        let (chunks, embeddings) = if path.is_file() {
            let data =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let store: StoreData = serde_json::from_str(&data)
                .with_context(|| format!("parsing {}", path.display()))?;
            if store.dim != dim {
                anyhow::bail!(
                    "dimension mismatch: store has dim={} but caller specified dim={}",
                    store.dim,
                    dim
                );
            }
            let mut chunks = HashMap::with_capacity(store.entries.len());
            let mut embeddings = HashMap::with_capacity(store.entries.len());
            for entry in store.entries {
                let id = entry.chunk.id;
                chunks.insert(id, entry.chunk);
                embeddings.insert(id, entry.embedding);
            }
            (chunks, embeddings)
        } else {
            (HashMap::new(), HashMap::new())
        };

        Ok(Self {
            dir: dir.to_path_buf(),
            dim,
            chunks,
            embeddings,
            dirty: false,
        })
    }

    /// Insert or update a chunk with its embedding vector.
    ///
    /// The embedding is L2-normalised before storage so that cosine
    /// similarity reduces to a dot product at search time.
    ///
    /// # Errors
    ///
    /// Returns an error if `embedding.len() != self.dim`.
    pub fn upsert(&mut self, chunk: Chunk, embedding: Vec<f32>) -> Result<()> {
        if embedding.len() != self.dim {
            anyhow::bail!(
                "embedding dimension mismatch: got {}, expected {}",
                embedding.len(),
                self.dim
            );
        }
        let normalized = l2_normalize(&embedding);
        let id = chunk.id;
        self.chunks.insert(id, chunk);
        self.embeddings.insert(id, normalized);
        self.dirty = true;
        Ok(())
    }

    /// Remove every stored chunk and embedding.
    pub fn clear(&mut self) {
        self.chunks.clear();
        self.embeddings.clear();
        self.dirty = true;
    }

    /// Search for the `k` chunks most similar to `query` by cosine similarity.
    ///
    /// Returns `(chunk_id, score)` pairs sorted descending by score.
    /// `query` is L2-normalised internally so callers may pass any vector.
    ///
    /// If `query.len() != self.dim` (or `k == 0`) the result is an empty
    /// `Vec` — the method never panics on malformed input.
    pub fn search(&self, query: &[f32], k: usize) -> Vec<(u64, f32)> {
        if k == 0 || self.chunks.is_empty() || query.len() != self.dim {
            return Vec::new();
        }

        let q = l2_normalize(query);

        // Score every stored vector.
        let mut scores: Vec<(u64, f32)> = self
            .embeddings
            .iter()
            .map(|(&id, emb)| {
                let sim = dot(&q, emb);
                (id, sim)
            })
            .collect();

        // Full sort descending and take top-k.
        // For the expected corpus size (<10⁵ chunks) the O(n log n) sort is
        // dwarfed by the O(n·d) scoring loop above.
        scores.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scores.truncate(k);
        scores
    }

    /// Retrieve a chunk by its id.
    pub fn get(&self, id: u64) -> Option<&Chunk> {
        self.chunks.get(&id)
    }

    /// Flush the store to disk as `vector_store.json` under the root
    /// directory passed to [`open`](Self::open).
    ///
    /// Creates the directory if it does not exist.
    pub fn persist(&self) -> Result<()> {
        fs::create_dir_all(&self.dir)
            .with_context(|| format!("creating {}", self.dir.display()))?;

        let entries: Vec<StoreEntry> = self
            .chunks
            .values()
            .map(|chunk| {
                let embedding = self.embeddings.get(&chunk.id).cloned().unwrap_or_default();
                StoreEntry {
                    chunk: chunk.clone(),
                    embedding,
                }
            })
            .collect();

        let store = StoreData {
            dim: self.dim,
            entries,
        };

        let json = serde_json::to_string_pretty(&store).context("serialising vector store")?;
        let path = self.dir.join("vector_store.json");
        fs::write(&path, &json).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    // -- introspection helpers (not in the spec, but useful) -----------------

    /// Number of stored chunks.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Returns `true` if the store is empty.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The configured embedding dimensionality.
    pub fn dim(&self) -> usize {
        self.dim
    }
}

// =============================================================================
// Internal helpers
// =============================================================================

/// L2-normalise a vector in-place and return it.
fn l2_normalize(v: &[f32]) -> Vec<f32> {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        return v.to_vec(); // all-zero vector stays as-is
    }
    v.iter().map(|x| x / norm).collect()
}

/// Dot product of two equal-length vectors.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // ---- MockEmbedder ------------------------------------------------------

    /// Deterministic embedder for tests.
    ///
    /// Each text is hashed (by string length as a cheap proxy) to produce a
    /// pseudo-random vector of the configured dimension.  Same text → same
    /// vector.
    struct MockEmbedder {
        dim: usize,
    }

    impl MockEmbedder {
        fn new(dim: usize) -> Self {
            Self { dim }
        }

        fn vector_for(text: &str, dim: usize) -> Vec<f32> {
            // Deterministic: use a simple hash of the string.
            let h: u64 = text
                .bytes()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
            (0..dim)
                .map(|i| ((h.wrapping_mul(i as u64 + 1) & 0xFF) as f32) / 128.0 - 1.0)
                .collect()
        }
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            let out: Vec<Vec<f32>> = texts
                .iter()
                .map(|t| Self::vector_for(t, self.dim))
                .collect();
            Ok(out)
        }

        fn dim(&self) -> usize {
            self.dim
        }
    }

    // ---- helpers -----------------------------------------------------------

    fn tmp_dir() -> TempDir {
        TempDir::new().expect("temp dir")
    }

    fn chunk(id: u64, file: &str, start: usize, end: usize, text: &str) -> Chunk {
        Chunk {
            id,
            file: PathBuf::from(file),
            start_line: start,
            end_line: end,
            text: text.to_string(),
        }
    }

    // ---- tests -------------------------------------------------------------

    #[test]
    fn open_creates_empty_store() {
        let dir = tmp_dir();
        let store = VectorStore::open(dir.path(), 4).unwrap();
        assert!(store.is_empty());
        assert_eq!(store.dim(), 4);
    }

    #[test]
    fn upsert_and_get() {
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 3).unwrap();
        let c = chunk(1, "a.rs", 1, 5, "fn a() {}");
        store.upsert(c.clone(), vec![1.0, 0.0, 0.0]).unwrap();
        assert_eq!(store.get(1), Some(&c));
    }

    #[test]
    fn dimension_mismatch_on_upsert_returns_err() {
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 3).unwrap();
        let c = chunk(1, "x.rs", 1, 1, "x");
        let result = store.upsert(c, vec![0.1, 0.2]); // dim 2 ≠ 3
        assert!(result.is_err(), "expected Err on dimension mismatch");
    }

    #[test]
    fn search_ranks_closest_chunk_first() {
        let embedder = MockEmbedder::new(8);
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 8).unwrap();

        // Three chunks with different texts → different vectors.
        let texts = [
            "fn alpha() { beta() }",
            "fn beta() { gamma() }",
            "fn gamma() { delta() }",
        ];
        for (i, text) in texts.iter().enumerate() {
            let id = i as u64 + 1;
            let c = chunk(id, "lib.rs", i * 3 + 1, i * 3 + 3, text);
            let emb = embedder
                .embed(&[text.to_string()])
                .unwrap()
                .into_iter()
                .next()
                .unwrap();
            store.upsert(c, emb).unwrap();
        }

        // Query with the same text as chunk #2 → chunk #2 should rank first.
        let query = embedder
            .embed(&[texts[1].to_string()])
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let results = store.search(&query, 3);

        assert!(!results.is_empty(), "expected at least one result");
        assert_eq!(
            results[0].0, 2,
            "chunk #2 should rank first for its own text, got {:?}",
            results
        );
        // Score should be ~1.0 (identical vectors).
        assert!(
            (results[0].1 - 1.0).abs() < 1e-4,
            "cosine of identical vectors should be 1.0, got {}",
            results[0].1
        );
    }

    #[test]
    fn persist_round_trip_preserves_chunks() {
        let dir = tmp_dir();
        let dim = 4;
        let c = chunk(42, "main.rs", 1, 10, "fn main() {}");
        let emb = vec![1.0, 0.0, 0.0, 0.0];

        // Write.
        {
            let mut store = VectorStore::open(dir.path(), dim).unwrap();
            store.upsert(c.clone(), emb).unwrap();
            store.persist().unwrap();
        }

        // Re-open and verify.
        {
            let store = VectorStore::open(dir.path(), dim).unwrap();
            assert_eq!(store.len(), 1);
            assert_eq!(store.get(42), Some(&c));
        }
    }

    #[test]
    fn persist_file_exists_after_persist() {
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 2).unwrap();
        store
            .upsert(chunk(1, "f.rs", 1, 1, "x"), vec![1.0, 0.0])
            .unwrap();
        store.persist().unwrap();
        assert!(dir.path().join("vector_store.json").is_file());
    }

    #[test]
    fn open_existing_store_with_wrong_dimension_errors() {
        let dir = tmp_dir();
        let dim = 4;
        {
            let mut store = VectorStore::open(dir.path(), dim).unwrap();
            store
                .upsert(chunk(1, "f.rs", 1, 1, "x"), vec![1.0, 0.0, 0.0, 0.0])
                .unwrap();
            store.persist().unwrap();
        }
        // Re-open with wrong dim.
        let result = VectorStore::open(dir.path(), 8);
        assert!(result.is_err(), "expected Err on dim mismatch at open");
    }

    #[test]
    fn search_empty_store_returns_empty() {
        let dir = tmp_dir();
        let store = VectorStore::open(dir.path(), 3).unwrap();
        let results = store.search(&[1.0, 0.0, 0.0], 5);
        assert!(results.is_empty());
    }

    #[test]
    fn search_k_zero_returns_empty() {
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 3).unwrap();
        store
            .upsert(chunk(1, "f.rs", 1, 1, "x"), vec![1.0, 0.0, 0.0])
            .unwrap();
        let results = store.search(&[1.0, 0.0, 0.0], 0);
        assert!(results.is_empty());
    }

    #[test]
    fn upsert_replaces_existing_chunk() {
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 2).unwrap();
        store
            .upsert(chunk(1, "a.rs", 1, 1, "first"), vec![1.0, 0.0])
            .unwrap();
        store
            .upsert(chunk(1, "b.rs", 5, 5, "second"), vec![0.0, 1.0])
            .unwrap();
        // Same id → replaced.
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(1).unwrap().file, PathBuf::from("b.rs"));
        assert_eq!(store.get(1).unwrap().text, "second");
    }

    #[test]
    fn search_results_are_sorted_descending() {
        let embedder = MockEmbedder::new(4);
        let dir = tmp_dir();
        let mut store = VectorStore::open(dir.path(), 4).unwrap();

        let texts = ["apple", "banana", "cherry", "date"];
        for (i, text) in texts.iter().enumerate() {
            let id = i as u64 + 1;
            store
                .upsert(
                    chunk(id, "f.rs", id as usize, id as usize, text),
                    embedder
                        .embed(&[text.to_string()])
                        .unwrap()
                        .into_iter()
                        .next()
                        .unwrap(),
                )
                .unwrap();
        }

        let query = embedder
            .embed(&["banana".to_string()])
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        let results = store.search(&query, 4);

        assert_eq!(results.len(), 4);
        for w in results.windows(2) {
            assert!(w[0].1 >= w[1].1, "scores must be descending: {:?}", results);
        }
        // The closest should be banana itself.
        assert_eq!(results[0].0, 2);
    }

    #[test]
    fn search_wrong_dimension_returns_empty() {
        let dir = tmp_dir();
        let store = VectorStore::open(dir.path(), 3).unwrap();
        let results = store.search(&[1.0, 0.0], 1); // dim 2 ≠ 3
        assert!(results.is_empty(), "expected empty for dim mismatch");
    }

    #[test]
    fn mock_embedder_is_deterministic() {
        let e = MockEmbedder::new(16);
        let t = "hello world".to_string();
        let v1 = e.embed(std::slice::from_ref(&t)).unwrap();
        let v2 = e.embed(std::slice::from_ref(&t)).unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn mock_embedder_dim_matches() {
        let e = MockEmbedder::new(7);
        assert_eq!(e.dim(), 7);
        let v = e.embed(&["x".to_string()]).unwrap();
        assert_eq!(v[0].len(), 7);
    }

    #[test]
    fn equal_scores_break_ties_by_id() {
        let dir = tmp_dir();
        let mut s = VectorStore::open(dir.path(), 2).unwrap();
        // Two chunks with identical embedding → identical score.
        s.upsert(chunk(5, "a.rs", 1, 1, "a"), vec![1.0, 0.0])
            .unwrap();
        s.upsert(chunk(2, "b.rs", 2, 2, "b"), vec![1.0, 0.0])
            .unwrap();
        let r = s.search(&[1.0, 0.0], 2);
        let ids: Vec<u64> = r.iter().map(|x| x.0).collect();
        assert_eq!(ids, vec![2, 5], "ties broken by ascending id");
    }
}
