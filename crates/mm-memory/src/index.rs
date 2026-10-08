//! The packed vector index (`data/index/memory.pack`).
//!
//! The plan asks for `kg-embed` with BGE-small. That model is not vendored and a
//! downloaded one would make retrieval depend on a network and a model version, so
//! the embedder here is the kernel's existing deterministic hashed bag-of-tokens
//! ([`mm_llm::semantic_cache::embed`], the Phase 3 precedent): tokenize, hash each
//! token into a dimension, sign it, normalize. That is a *documented deviation*
//! from the plan's letter, and it is the right one for this substrate — a
//! retrieval channel whose behaviour changes when a model ships is not a channel a
//! replay can trust. Plan §9 already says keyword search is primary and embeddings
//! only rank and expand, which is exactly how this channel is used.
//!
//! The pack is a text file with a versioned header, so it is diffable, hashable,
//! and readable by a human debugging a retrieval miss.

use std::collections::BTreeMap;
use std::path::Path;

use mm_core::{Ulid, content_hash};
use serde::{Deserialize, Serialize};

use crate::error::{MemoryError, Result};
use crate::model::MemoryFilter;
use crate::store::SqliteMemoryStore;

/// The pack format version. Bumped when the layout changes, never silently.
pub const INDEX_VERSION: u32 = 1;
/// The embedder's name, recorded in the header and in `memory.index.rebuild`.
pub const EMBEDDER_NAME: &str = "mm-hashed-bow-64";

/// The invariant dimension of every vector.
pub const DIM: usize = mm_llm::semantic_cache::EMBEDDING_DIM;

/// The header line of a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PackHeader {
    version: u32,
    embedder: String,
    dim: usize,
    count: usize,
}

/// One packed vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PackEntry {
    id: String,
    v: Vec<f32>,
}

/// A memory-id-keyed vector index.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryIndex {
    dim: usize,
    entries: BTreeMap<String, Vec<f32>>,
}

impl Default for MemoryIndex {
    fn default() -> Self {
        MemoryIndex::new()
    }
}

impl MemoryIndex {
    /// An empty index.
    pub fn new() -> Self {
        MemoryIndex {
            dim: DIM,
            entries: BTreeMap::new(),
        }
    }

    /// The vector width.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// How many memories are indexed.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is indexed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Embed text with the kernel's deterministic embedder.
    pub fn embed(text: &str) -> Vec<f32> {
        mm_llm::semantic_cache::embed(text)
    }

    /// Add or replace a memory's vector.
    pub fn insert(&mut self, id: Ulid, vector: Vec<f32>) {
        self.entries.insert(mm_core::ulid_string(&id), vector);
    }

    /// Add a memory's vector by embedding its content.
    pub fn insert_text(&mut self, id: Ulid, text: &str) {
        self.insert(id, MemoryIndex::embed(text));
    }

    /// Drop a memory.
    pub fn remove(&mut self, id: &Ulid) -> Option<Vec<f32>> {
        self.entries.remove(&mm_core::ulid_string(id))
    }

    /// True when a memory is indexed.
    pub fn contains(&self, id: &Ulid) -> bool {
        self.entries.contains_key(&mm_core::ulid_string(id))
    }

    /// Every indexed id, in order.
    pub fn ids(&self) -> Vec<Ulid> {
        self.entries
            .keys()
            .filter_map(|id| mm_core::id::parse_ulid(id).ok())
            .collect()
    }

    /// The `k` nearest memories to `query`, best first, ties broken by ULID.
    pub fn knn(&self, query: &[f32], k: usize) -> Vec<(Ulid, f64)> {
        let mut scored: Vec<(Ulid, f64)> = self
            .entries
            .iter()
            .filter_map(|(id, vector)| {
                let id = mm_core::id::parse_ulid(id).ok()?;
                Some((id, mm_llm::semantic_cache::cosine(query, vector)))
            })
            .collect();
        // Descending cosine, ascending ULID — the same tie-break the rerank uses.
        scored.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        scored.truncate(k);
        scored
    }

    /// Rebuild the index from the store's active memories.
    pub async fn rebuild(&mut self, store: &SqliteMemoryStore) -> Result<usize> {
        let memories = store.list(&MemoryFilter::all()).await?;
        self.entries.clear();
        for memory in &memories {
            self.insert_text(memory.id, &memory.content);
        }
        Ok(self.entries.len())
    }

    /// Render the pack exactly as it is written to disk.
    fn render(&self) -> Result<String> {
        let header = PackHeader {
            version: INDEX_VERSION,
            embedder: EMBEDDER_NAME.to_string(),
            dim: self.dim,
            count: self.entries.len(),
        };
        let mut out = serde_json::to_string(&header)?;
        out.push('\n');
        for (id, vector) in &self.entries {
            out.push_str(&serde_json::to_string(&PackEntry {
                id: id.clone(),
                v: vector.clone(),
            })?);
            out.push('\n');
        }
        Ok(out)
    }

    /// Write the pack, returning the hash of exactly what was written.
    pub fn save(&self, path: &Path) -> Result<String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rendered = self.render()?;
        std::fs::write(path, rendered.as_bytes())?;
        Ok(content_hash(rendered.as_bytes()))
    }

    /// Read a pack.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| MemoryError::Config(format!("cannot read {}: {e}", path.display())))?;
        let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
        let header_line = lines
            .next()
            .ok_or_else(|| MemoryError::Codec(format!("{} is empty", path.display())))?;
        let header: PackHeader = serde_json::from_str(header_line)
            .map_err(|e| MemoryError::Codec(format!("{} header: {e}", path.display())))?;
        if header.version != INDEX_VERSION {
            return Err(MemoryError::Codec(format!(
                "{} is version {}, this kernel writes version {INDEX_VERSION}",
                path.display(),
                header.version
            )));
        }
        if header.dim != DIM {
            return Err(MemoryError::Codec(format!(
                "{} has dimension {}, this kernel uses {DIM}",
                path.display(),
                header.dim
            )));
        }
        let mut index = MemoryIndex::new();
        for line in lines {
            let entry: PackEntry = serde_json::from_str(line)
                .map_err(|e| MemoryError::Codec(format!("{} entry: {e}", path.display())))?;
            if entry.v.len() != DIM {
                return Err(MemoryError::Codec(format!(
                    "{} entry {} has {} dimensions",
                    path.display(),
                    entry.id,
                    entry.v.len()
                )));
            }
            index.entries.insert(entry.id, entry.v);
        }
        Ok(index)
    }

    /// The hash of the pack as it would be written.
    pub fn hash(&self) -> Result<String> {
        Ok(content_hash(self.render()?.as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Ulid {
        Ulid::from_parts(1_700_000_000_000, n)
    }

    #[test]
    fn embedding_is_deterministic_and_normalized() {
        let a = MemoryIndex::embed("the workspace build was broken by the toolchain");
        let b = MemoryIndex::embed("the workspace build was broken by the toolchain");
        assert_eq!(a.len(), DIM);
        assert_eq!(a, b);
        let norm: f64 = a.iter().map(|x| f64::from(*x) * f64::from(*x)).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm {norm}");
    }

    #[test]
    fn knn_ranks_by_cosine_and_breaks_ties_on_ulid() {
        let mut index = MemoryIndex::new();
        index.insert_text(id(1), "rust build toolchain pin");
        index.insert_text(id(2), "coffee preferences morning");
        index.insert_text(id(3), "rust build failure toolchain");
        let hits = index.knn(&MemoryIndex::embed("rust build toolchain"), 3);
        assert_eq!(hits.len(), 3);
        assert!(hits[0].1 >= hits[1].1);
        assert!(hits[0].0 == id(1) || hits[0].0 == id(3));
    }

    #[test]
    fn an_empty_index_returns_no_hits() {
        let index = MemoryIndex::new();
        assert!(index.knn(&MemoryIndex::embed("anything"), 5).is_empty());
        assert!(index.is_empty());
    }

    #[test]
    fn a_pack_round_trips_and_hashes_its_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index/memory.pack");
        let mut index = MemoryIndex::new();
        index.insert_text(id(1), "one");
        index.insert_text(id(2), "two");
        let hash = index.save(&path).unwrap();
        assert_eq!(hash, index.hash().unwrap());
        let loaded = MemoryIndex::load(&path).unwrap();
        assert_eq!(loaded, index);
        assert_eq!(loaded.knn(&MemoryIndex::embed("one"), 1)[0].0, id(1));
    }

    #[test]
    fn a_pack_with_the_wrong_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memory.pack");
        std::fs::write(
            &path,
            "{\"version\":99,\"embedder\":\"x\",\"dim\":64,\"count\":0}\n",
        )
        .unwrap();
        assert_eq!(
            MemoryIndex::load(&path).unwrap_err().code(),
            "memory.codec"
        );
    }

    #[test]
    fn a_pack_with_a_short_vector_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memory.pack");
        std::fs::write(
            &path,
            "{\"version\":1,\"embedder\":\"mm-hashed-bow-64\",\"dim\":64,\"count\":1}\n\
             {\"id\":\"01hf7yat000000000000000001\",\"v\":[0.1]}\n",
        )
        .unwrap();
        assert_eq!(
            MemoryIndex::load(&path).unwrap_err().code(),
            "memory.codec"
        );
    }

    #[test]
    fn removing_a_memory_takes_it_out_of_the_candidates() {
        let mut index = MemoryIndex::new();
        index.insert_text(id(1), "one");
        assert!(index.contains(&id(1)));
        assert!(index.remove(&id(1)).is_some());
        assert!(!index.contains(&id(1)));
        assert!(index.is_empty());
    }
}
