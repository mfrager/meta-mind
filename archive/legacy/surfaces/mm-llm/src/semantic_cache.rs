//! The semantic cache — off by default, opt-in per purpose.
//!
//! A semantic hit is an *inference*: "this prompt is close enough to that one that
//! the recorded answer serves". That is fine for a summarization and catastrophic
//! for a decision, so two things are always true here:
//!
//! 1. A purpose must be named in `semantic_cache_purposes` before its prompts are
//!    even considered, and [`SemanticCache::allowed_for`] is the only authority on
//!    that. Consequential structured work never reaches this module.
//! 2. The similarity must clear a configured threshold; nothing is served on a
//!    "closest match" basis.
//!
//! The embedding is a deterministic hashed bag of tokens, normalized. It is not a
//! learned embedding: a learned one would make the cache's behaviour depend on a
//! model version, and a cache that changes its answer when a model ships is not a
//! cache. Determinism is the property the substrate needs here, and it is the
//! property the tests check.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::client::Purpose;

/// The width of the embedding.
pub const EMBEDDING_DIM: usize = 64;

/// Embed `text`: tokenize, hash each token into a dimension, normalize.
pub fn embed(text: &str) -> Vec<f32> {
    let mut vector = vec![0f32; EMBEDDING_DIM];
    for token in tokenize(text) {
        let digest = sha256(token.as_bytes());
        let index = usize::from(u16::from_le_bytes([digest[0], digest[1]])) % EMBEDDING_DIM;
        let sign = if digest[2] & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    let norm = vector
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    if norm > 0.0 {
        for x in &mut vector {
            *x = (f64::from(*x) / norm) as f32;
        }
    }
    vector
}

fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Cosine similarity of two vectors, in `-1.0..=1.0`.
pub fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b) {
        dot += f64::from(*x) * f64::from(*y);
        na += f64::from(*x) * f64::from(*x);
        nb += f64::from(*y) * f64::from(*y);
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// One cached prompt and the body it points at.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticCacheEntry {
    /// The hash of the prompt that was actually answered.
    pub prompt_hash: String,
    /// The purpose it was answered for.
    pub purpose: Purpose,
    /// Its embedding.
    pub embedding: Vec<f32>,
    /// Where the recorded body lives.
    pub response_path: String,
}

/// A similarity-indexed cache.
#[derive(Debug, Clone)]
pub struct SemanticCache {
    threshold: f64,
    allowed: Vec<Purpose>,
    entries: Vec<SemanticCacheEntry>,
}

impl SemanticCache {
    /// A cache that serves only `allowed` purposes at `threshold` similarity.
    pub fn new(threshold: f64, allowed: Vec<Purpose>) -> Self {
        SemanticCache {
            threshold,
            allowed,
            entries: Vec::new(),
        }
    }

    /// A cache from a configuration's policy.
    pub fn from_config(cfg: &crate::config::LlmConfig) -> Self {
        SemanticCache::new(
            cfg.defaults.semantic_cache_threshold,
            cfg.defaults.semantic_cache_purposes.clone(),
        )
    }

    /// The similarity a hit must reach.
    pub fn threshold(&self) -> f64 {
        self.threshold
    }

    /// The purposes this cache may serve.
    pub fn allowed(&self) -> &[Purpose] {
        &self.allowed
    }

    /// True when `purpose` may be served from here.
    ///
    /// The one gate that matters: a purpose that is not named in the configuration
    /// never consults a similarity index at all.
    pub fn allowed_for(&self, purpose: Purpose) -> bool {
        self.allowed.contains(&purpose)
    }

    /// How many prompts are indexed.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is indexed.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Index `prompt`, returning the entry.
    pub fn insert(
        &mut self,
        purpose: Purpose,
        prompt: &str,
        prompt_hash: &str,
        response_path: &str,
    ) -> &SemanticCacheEntry {
        let index = match self
            .entries
            .iter()
            .position(|e| e.prompt_hash == prompt_hash && e.purpose == purpose)
        {
            Some(index) => {
                self.entries[index].response_path = response_path.to_string();
                index
            }
            None => {
                self.entries.push(SemanticCacheEntry {
                    prompt_hash: prompt_hash.to_string(),
                    purpose,
                    embedding: embed(prompt),
                    response_path: response_path.to_string(),
                });
                self.entries.len() - 1
            }
        };
        &self.entries[index]
    }

    /// The best entry at or above the threshold, with its similarity.
    ///
    /// `None` for a disallowed purpose, for a normless prompt, and for a best match
    /// under the threshold — three different reasons to make a real call, and none
    /// of them is "close enough".
    pub fn lookup(&self, purpose: Purpose, prompt: &str) -> Option<(&SemanticCacheEntry, f64)> {
        if !self.allowed_for(purpose) {
            return None;
        }
        let query = embed(prompt);
        let mut best: Option<(&SemanticCacheEntry, f64)> = None;
        for entry in self.entries.iter().filter(|e| e.purpose == purpose) {
            let similarity = cosine(&query, &entry.embedding);
            if best.is_none_or(|(_, s)| similarity > s) {
                best = Some((entry, similarity));
            }
        }
        match best {
            Some((entry, similarity)) if similarity >= self.threshold => Some((entry, similarity)),
            _ => None,
        }
    }

    /// Every threshold decision the cache would make for `prompts`, for the routes
    /// report: `(prompt, best similarity, hit)`.
    pub fn survey(&self, purpose: Purpose, prompts: &[String]) -> Vec<(String, f64, bool)> {
        prompts
            .iter()
            .map(|prompt| {
                let best = self
                    .entries
                    .iter()
                    .filter(|e| e.purpose == purpose)
                    .map(|e| cosine(&embed(prompt), &e.embedding))
                    .fold(f64::NEG_INFINITY, f64::max);
                let best = if best.is_finite() { best } else { 0.0 };
                (
                    prompt.clone(),
                    best,
                    self.allowed_for(purpose) && best >= self.threshold,
                )
            })
            .collect()
    }

    /// How many entries each purpose holds.
    pub fn by_purpose(&self) -> HashMap<String, usize> {
        let mut out = HashMap::new();
        for entry in &self.entries {
            *out.entry(entry.purpose.as_str().to_string()).or_insert(0) += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_is_deterministic_and_normalized() {
        let a = embed("Summarize the phase 3 gate");
        let b = embed("Summarize the phase 3 gate");
        assert_eq!(a, b);
        assert_eq!(a.len(), EMBEDDING_DIM);
        let norm: f64 = a
            .iter()
            .map(|x| f64::from(*x) * f64::from(*x))
            .sum::<f64>()
            .sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm was {norm}");
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_disallowed_purpose_never_consults_the_index() {
        let mut cache = SemanticCache::new(0.9, vec![Purpose::Summarize]);
        cache.insert(
            Purpose::Summarize,
            "Summarize the gate",
            &"b".repeat(64),
            "/tmp/body.json",
        );
        assert!(cache.allowed_for(Purpose::Summarize));
        assert!(!cache.allowed_for(Purpose::Plan));
        assert!(
            cache.lookup(Purpose::Plan, "Summarize the gate").is_none(),
            "a consequential purpose must never be served from a similarity index"
        );
        assert!(cache
            .lookup(Purpose::Summarize, "Summarize the gate")
            .is_some());
    }

    #[test]
    fn the_threshold_decides_not_the_closest_match() {
        let mut strict = SemanticCache::new(0.99, vec![Purpose::Summarize]);
        strict.insert(
            Purpose::Summarize,
            "summarize the phase three gate",
            &"c".repeat(64),
            "/tmp/body.json",
        );
        // One extra token is a real difference at 0.99 …
        assert!(strict
            .lookup(Purpose::Summarize, "summarize the phase three gate please")
            .is_none());

        // … and the same query at a looser threshold clears it, so the threshold is
        // the thing making the decision.
        let mut loose = SemanticCache::new(0.8, vec![Purpose::Summarize]);
        loose.insert(
            Purpose::Summarize,
            "summarize the phase three gate",
            &"c".repeat(64),
            "/tmp/body.json",
        );
        let (entry, similarity) = loose
            .lookup(Purpose::Summarize, "summarize the phase three gate please")
            .unwrap();
        assert!((0.8..1.0).contains(&similarity), "{similarity}");
        assert_eq!(entry.response_path, "/tmp/body.json");

        // An unrelated prompt is below any sane threshold.
        assert!(loose
            .lookup(Purpose::Summarize, "deploy the rust compiler")
            .is_none());
    }

    #[test]
    fn inserting_the_same_prompt_twice_updates_rather_than_duplicates() {
        let mut cache = SemanticCache::new(0.9, vec![Purpose::Summarize]);
        cache.insert(Purpose::Summarize, "hi", &"d".repeat(64), "/a.json");
        cache.insert(Purpose::Summarize, "hi", &"d".repeat(64), "/b.json");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.by_purpose()["summarize"], 1);
        let (entry, _) = cache.lookup(Purpose::Summarize, "hi").unwrap();
        assert_eq!(entry.response_path, "/b.json");
    }
}
