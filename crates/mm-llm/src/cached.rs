//! The exact cache, and the request hash that keys it.
//!
//! The key is a hash over the *whole* request, not a counter or a timestamp: two
//! calls with the same provider, model, purpose, messages, schema, ceiling, and
//! temperature are the same question and must get the same answer, while changing
//! any one of them must miss. A hash that ignored a field would make the cache
//! return a wrong answer that looks right, so the field list is explicit here and
//! a property test in `tests/cache_replay.rs` checks every field is folded in.
//!
//! Bodies live on disk (`data/llm_cache/<hash>.json`) and the `llm_cache_index`
//! table mirrors them. The mirror is a projection: it can be rebuilt from the files
//! at any time, and `verify-cache --strict` re-hashes every row to prove the two
//! agree.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use mm_core::{content_hash, hash_fields, Param, Params, Tabular};
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};

use crate::client::{DecoderKind, LlmRequest, SchemaId};
use crate::error::LlmError;

/// sha256 over the canonical request tuple.
///
/// The provider is part of the key because two providers can answer one prompt
/// differently; `temperature` is hashed through its bit pattern so that `0.1` and
/// `0.100000001` cannot be conflated by a float formatter.
pub fn prompt_hash(provider: &str, model: &str, req: &LlmRequest) -> String {
    let mut fields: Vec<String> = vec![
        provider.to_string(),
        model.to_string(),
        req.purpose.as_str().to_string(),
        req.schema
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default(),
        req.max_tokens.to_string(),
        req.temperature.to_bits().to_string(),
    ];
    for message in &req.messages {
        fields.push(format!("{:?}", message.role));
        fields.push(message.content.clone());
    }
    let borrowed: Vec<&str> = fields.iter().map(String::as_str).collect();
    hash_fields(&borrowed)
}

/// What identifies a cache entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CacheKey {
    /// The request hash.
    pub prompt_hash: String,
    /// The model that answered.
    pub model: String,
    /// The schema, for a structured call.
    pub schema_id: Option<SchemaId>,
}

impl CacheKey {
    /// The key for `req` answered by `model` through `provider`.
    pub fn of(provider: &str, model: &str, req: &LlmRequest) -> Self {
        CacheKey {
            prompt_hash: prompt_hash(provider, model, req),
            model: model.to_string(),
            schema_id: req.schema.clone(),
        }
    }
}

/// A stored answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedResponse {
    /// The completion text.
    pub text: String,
    /// The model that produced it.
    pub model: String,
    /// The decoder that produced it.
    pub decoder: DecoderKind,
    /// Prompt tokens the original call billed.
    pub tokens_in: u32,
    /// Completion tokens the original call billed.
    pub tokens_out: u32,
    /// The original call's latency.
    pub latency_ms: u32,
    /// sha256 of `text`, so a corrupted body is detected rather than served.
    pub response_sha: String,
}

impl CachedResponse {
    /// A body, with its own hash computed.
    pub fn new(
        text: impl Into<String>,
        model: impl Into<String>,
        decoder: DecoderKind,
        tokens_in: u32,
        tokens_out: u32,
        latency_ms: u32,
    ) -> Self {
        let text = text.into();
        CachedResponse {
            response_sha: content_hash(text.as_bytes()),
            text,
            model: model.into(),
            decoder,
            tokens_in,
            tokens_out,
            latency_ms,
        }
    }

    /// True when the body still hashes to what was recorded.
    pub fn is_intact(&self) -> bool {
        self.response_sha == content_hash(self.text.as_bytes())
    }
}

/// A cache that maps a request hash to a stored answer.
pub trait ExactCache: Send + Sync {
    /// The stored answer, when there is one that is intact.
    fn get(&self, key: &CacheKey) -> Option<CachedResponse>;
    /// Store an answer.
    fn put(&self, key: CacheKey, response: CachedResponse) -> Result<(), LlmError>;
}

/// The on-disk exact cache.
#[derive(Debug, Clone)]
pub struct FileExactCache {
    dir: PathBuf,
}

impl FileExactCache {
    /// A cache rooted at `dir` (created on first write).
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        FileExactCache { dir: dir.into() }
    }

    /// The cache directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The body path for a key.
    pub fn path_for(&self, key: &CacheKey) -> PathBuf {
        self.dir.join(format!("{}.json", key.prompt_hash))
    }

    /// How many bodies are on disk.
    pub fn len(&self) -> usize {
        self.bodies().len()
    }

    /// True when nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The body file names, sorted.
    pub fn bodies(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".json") {
                out.push(name);
            }
        }
        out.sort();
        out
    }
}

impl ExactCache for FileExactCache {
    fn get(&self, key: &CacheKey) -> Option<CachedResponse> {
        let raw = std::fs::read_to_string(self.path_for(key)).ok()?;
        let stored: CachedResponse = serde_json::from_str(&raw).ok()?;
        // A body whose hash no longer matches its text is not served: a corrupted
        // cache must miss, not lie.
        stored.is_intact().then_some(stored)
    }

    fn put(&self, key: CacheKey, response: CachedResponse) -> Result<(), LlmError> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path_for(&key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&response)
            .map_err(|e| LlmError::Config(format!("cannot encode a cached response: {e}")))?;
        std::fs::write(&path, body)?;
        Ok(())
    }
}

/// Mirror one cache body into `llm_cache_index`.
///
/// Idempotent: it is called both when a body is written and when one is served, so
/// a body that exists without a row (a crash between the two writes) heals instead
/// of showing up forever as a stray file.
pub async fn put_index(
    store: &SqliteStore,
    key: &CacheKey,
    path: &Path,
    response_sha: &str,
) -> Result<(), LlmError> {
    let index: &dyn Tabular = store;
    index
        .execute(
            "INSERT OR REPLACE INTO llm_cache_index \
             (prompt_hash, model, schema_id, response_path, response_sha, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(key.prompt_hash.clone()),
                Param::Text(key.model.clone()),
                Param::opt_text(key.schema_id.as_ref().map(|s| s.as_str().to_string())),
                Param::Text(path.display().to_string()),
                Param::Text(response_sha.to_string()),
                Param::Text(mm_core::ulid_string(&crate::accounting::new_call_id())),
            ],
        )
        .await
        .map_err(|e| LlmError::Db(e.to_string()))?;
    Ok(())
}

/// What `verify-cache` found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheVerifyReport {
    /// Rows in `llm_cache_index`.
    pub rows: usize,
    /// Rows whose body resolved and re-hashed identically.
    pub verified: usize,
    /// Bodies on disk with no index row.
    pub strays: Vec<String>,
}

impl CacheVerifyReport {
    /// True when every row verified and nothing is unindexed.
    pub fn is_clean(&self) -> bool {
        self.rows == self.verified && self.strays.is_empty()
    }
}

/// Re-hash every indexed cache body, and report bodies with no index row.
pub async fn verify_index(store: &SqliteStore, dir: &Path) -> Result<CacheVerifyReport, LlmError> {
    let index: &dyn Tabular = store;
    let rows = index
        .query_json(
            "SELECT prompt_hash, response_path, response_sha FROM llm_cache_index ORDER BY prompt_hash",
            Params::new(),
        )
        .await
        .map_err(|e| LlmError::Db(e.to_string()))?;

    let cache = FileExactCache::new(dir);
    let mut report = CacheVerifyReport {
        rows: rows.len(),
        ..CacheVerifyReport::default()
    };
    let mut indexed: BTreeSet<String> = BTreeSet::new();

    for row in &rows {
        let prompt_hash = row
            .get("prompt_hash")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let path = row
            .get("response_path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let want_sha = row
            .get("response_sha")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let resolved = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            dir.join(path)
        };
        let raw = std::fs::read_to_string(&resolved).map_err(|e| {
            LlmError::CacheMiss(format!(
                "cache row {prompt_hash} does not resolve ({}): {e}",
                resolved.display()
            ))
        })?;
        let stored: CachedResponse = serde_json::from_str(&raw).map_err(|e| {
            LlmError::CacheMiss(format!(
                "cache row {prompt_hash} is not a response body: {e}"
            ))
        })?;
        if !stored.is_intact() {
            return Err(LlmError::CacheMiss(format!(
                "cache row {prompt_hash} body does not hash to its recorded response_sha"
            )));
        }
        if content_hash(stored.text.as_bytes()) != want_sha {
            return Err(LlmError::CacheMiss(format!(
                "cache row {prompt_hash} re-hashes to a different value than the index records"
            )));
        }
        indexed.insert(format!("{prompt_hash}.json"));
        report.verified += 1;
    }

    for body in cache.bodies() {
        if !indexed.contains(&body) {
            report.strays.push(body);
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Message, Purpose};

    fn request(purpose: Purpose, text: &str) -> LlmRequest {
        LlmRequest::new(purpose, vec![Message::user(text)])
    }

    #[test]
    fn the_hash_folds_in_every_field_of_the_request() {
        let base = request(Purpose::Plan, "hello");
        let h = prompt_hash("mock", "small", &base);

        // Same question, same answer.
        assert_eq!(h, prompt_hash("mock", "small", &base.clone()));

        let changed_model = prompt_hash("mock", "frontier", &base);
        assert_ne!(h, changed_model, "the model is part of the identity");

        let changed_provider = prompt_hash("other", "small", &base);
        assert_ne!(h, changed_provider, "the provider is part of the identity");

        let mut req = base.clone();
        req.purpose = Purpose::Extract;
        assert_ne!(h, prompt_hash("mock", "small", &req));

        let mut req = base.clone();
        req.messages = vec![Message::user("hello!")];
        assert_ne!(h, prompt_hash("mock", "small", &req));

        let mut req = base.clone();
        req.max_tokens = 10;
        assert_ne!(h, prompt_hash("mock", "small", &req));

        let mut req = base.clone();
        req.temperature = 0.5;
        assert_ne!(h, prompt_hash("mock", "small", &req));

        let mut req = base.clone();
        req.schema = Some(SchemaId::new("extract.v1"));
        assert_ne!(h, prompt_hash("mock", "small", &req));

        // Message order is part of the question.
        let mut req = base.clone();
        req.messages.push(Message::user("second"));
        assert_ne!(h, prompt_hash("mock", "small", &req));
    }

    #[test]
    fn a_body_that_does_not_hash_to_its_text_is_not_served() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FileExactCache::new(dir.path());
        let key = CacheKey::of("mock", "small", &request(Purpose::Plan, "hello"));
        let body = CachedResponse::new("answer", "small", DecoderKind::None, 3, 4, 5);
        assert!(body.is_intact());
        cache.put(key.clone(), body).unwrap();
        assert!(cache.get(&key).is_some());

        // Corrupt the text on disk without touching the recorded hash.
        let path = cache.path_for(&key);
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, raw.replace("\"answer\"", "\"tampered\"")).unwrap();
        assert!(
            cache.get(&key).is_none(),
            "a corrupted body must miss, never lie"
        );
    }
}
