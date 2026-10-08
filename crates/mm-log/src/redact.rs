//! Redaction.
//!
//! A record must never carry a secret, including on an error path. The policy is
//! deliberately simple and deterministic: known secret shapes are replaced
//! anywhere they appear, and any field whose *name* looks like a credential has
//! its whole value replaced regardless of the value's shape.

use std::path::Path;

use mm_core::MmError;
use serde::{Deserialize, Serialize};

/// What a replaced value becomes.
pub const REDACTED: &str = "[redacted]";

/// One literal pattern and its replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// The literal to look for (compared ASCII-case-insensitively).
    pub pattern: String,
    /// What to substitute.
    pub replacement: String,
}

/// The redaction policy applied to every record before it reaches a sink.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RedactionPolicy {
    /// Secret shapes to scrub from any string.
    pub rules: Vec<Rule>,
    /// Field names whose values are always scrubbed.
    pub secret_keys: Vec<String>,
}

/// The field-name fragments that always mark a credential.
const DEFAULT_SECRET_KEYS: [&str; 10] = [
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "apikey",
    "authorization",
    "credential",
    "private_key",
    "access_key",
];

/// Secret shapes Metamind itself emits or might receive.
const DEFAULT_PATTERNS: [&str; 8] = [
    "sk-",
    "ghp_",
    "gho_",
    "AKIA",
    "Bearer ",
    "BEGIN PRIVATE KEY",
    "BEGIN RSA PRIVATE KEY",
    "xoxb-",
];

impl Default for RedactionPolicy {
    fn default() -> Self {
        RedactionPolicy::kernel_default()
    }
}

impl RedactionPolicy {
    /// A policy that scrubs nothing. Only for a caller that has already proven
    /// its records are secret-free.
    pub fn empty() -> Self {
        RedactionPolicy {
            rules: Vec::new(),
            secret_keys: Vec::new(),
        }
    }

    /// The kernel policy: known token prefixes plus credential-looking keys.
    pub fn kernel_default() -> Self {
        RedactionPolicy {
            rules: DEFAULT_PATTERNS
                .iter()
                .map(|p| Rule {
                    pattern: (*p).to_string(),
                    replacement: REDACTED.to_string(),
                })
                .collect(),
            secret_keys: DEFAULT_SECRET_KEYS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    /// Add a literal pattern.
    pub fn with_rule(mut self, pattern: impl Into<String>) -> Self {
        self.rules.push(Rule {
            pattern: pattern.into(),
            replacement: REDACTED.to_string(),
        });
        self
    }

    /// Load additional patterns from a JSON file shaped like
    /// `tests/fixtures/secrets/patterns.json`.
    pub fn extend_from_file(mut self, path: &Path) -> Result<Self, MmError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
        let loaded: RedactionPolicy = serde_json::from_str(&raw)
            .map_err(|e| MmError::Config(format!("invalid {}: {e}", path.display())))?;
        for rule in loaded.rules {
            if !self.rules.iter().any(|r| r.pattern == rule.pattern) {
                self.rules.push(rule);
            }
        }
        for key in loaded.secret_keys {
            if !self.secret_keys.contains(&key) {
                self.secret_keys.push(key);
            }
        }
        Ok(self)
    }

    /// Whether a field name marks a credential.
    ///
    /// A substring test, deliberately: it fails *closed*, so an unfamiliar field
    /// name that merely looks like a credential is scrubbed rather than trusted.
    /// Callers therefore choose field names that do not look like credentials — see
    /// `mm-llm`'s `llm.response` record, which reports its counts under `usage` for
    /// exactly this reason.
    pub fn is_secret_key(&self, key: &str) -> bool {
        let lowered = key.to_ascii_lowercase();
        self.secret_keys
            .iter()
            .any(|k| lowered.contains(k.as_str()))
    }

    /// Scrub one string, returning the scrubbed text and how many substitutions ran.
    pub fn apply_to_str(&self, text: &str) -> (String, u32) {
        let mut current = text.to_string();
        let mut total = 0;
        for rule in &self.rules {
            let (next, n) =
                replace_ascii_case_insensitive(&current, &rule.pattern, &rule.replacement);
            current = next;
            total += n;
        }
        (current, total)
    }

    /// Recursively scrub a JSON value in place.
    pub fn apply_to_value(&self, value: &mut serde_json::Value) -> u32 {
        match value {
            serde_json::Value::String(s) => {
                let (next, n) = self.apply_to_str(s);
                *s = next;
                n
            }
            serde_json::Value::Array(items) => {
                items.iter_mut().map(|v| self.apply_to_value(v)).sum()
            }
            serde_json::Value::Object(map) => {
                let mut n = 0;
                for (key, val) in map.iter_mut() {
                    if self.is_secret_key(key) && !val.is_null() {
                        *val = serde_json::Value::String(REDACTED.to_string());
                        n += 1;
                    } else {
                        n += self.apply_to_value(val);
                    }
                }
                n
            }
            _ => 0,
        }
    }
}

/// Replace every ASCII-case-insensitive occurrence of `needle`.
///
/// Byte-wise comparison is index-safe here: pattern bytes are all ASCII, and no
/// ASCII byte can occur inside a multi-byte UTF-8 sequence, so a match can never
/// begin mid-character.
fn replace_ascii_case_insensitive(hay: &str, needle: &str, replacement: &str) -> (String, u32) {
    if needle.is_empty() {
        return (hay.to_string(), 0);
    }
    let hay_bytes = hay.as_bytes();
    let needle_bytes = needle.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(hay_bytes.len());
    let mut i = 0usize;
    let mut hits = 0u32;
    while i < hay_bytes.len() {
        let end = i + needle_bytes.len();
        if end <= hay_bytes.len() && hay_bytes[i..end].eq_ignore_ascii_case(needle_bytes) {
            out.extend_from_slice(replacement.as_bytes());
            i = end;
            hits += 1;
        } else {
            out.push(hay_bytes[i]);
            i += 1;
        }
    }
    match String::from_utf8(out) {
        Ok(s) => (s, hits),
        Err(_) => (hay.to_string(), 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_token_shapes_are_scrubbed_from_text() {
        let p = RedactionPolicy::kernel_default();
        let (s, n) = p.apply_to_str("using sk-abc123 and ghp_DEADBEEF now");
        assert!(!s.contains("sk-abc123"));
        assert!(!s.contains("ghp_DEADBEEF"));
        assert_eq!(n, 2);
    }

    #[test]
    fn matching_is_case_insensitive_and_unrelated_text_survives() {
        let p = RedactionPolicy::kernel_default();
        let (s, n) = p.apply_to_str("authorization: bEaReR abc.def");
        assert_eq!(n, 1);
        assert!(s.contains(REDACTED));
        assert!(s.contains("authorization:"));
        assert!(s.contains("abc.def"));
    }

    #[test]
    fn credential_keyed_fields_are_scrubbed_whatever_their_shape() {
        let p = RedactionPolicy::kernel_default();
        let mut v = serde_json::json!({
            "model": "gpt",
            "api_key": "anything at all",
            "nested": {"access_token": 12, "keep": "fine"}
        });
        let n = p.apply_to_value(&mut v);
        assert_eq!(n, 2);
        assert_eq!(v["api_key"], serde_json::json!(REDACTED));
        assert_eq!(v["nested"]["access_token"], serde_json::json!(REDACTED));
        assert_eq!(v["nested"]["keep"], serde_json::json!("fine"));
        assert_eq!(v["model"], serde_json::json!("gpt"));
    }

    #[test]
    fn multibyte_text_is_not_corrupted() {
        let p = RedactionPolicy::kernel_default();
        let (s, _) = p.apply_to_str("héllo — wörld 日本語 sk-xx ünïcode");
        assert!(s.starts_with("héllo — wörld 日本語"));
        assert!(s.ends_with("ünïcode"));
        assert!(!s.contains("sk-xx"));
    }

    #[test]
    fn empty_policy_changes_nothing() {
        let p = RedactionPolicy::empty();
        let (s, n) = p.apply_to_str("sk-abc");
        assert_eq!(s, "sk-abc");
        assert_eq!(n, 0);
    }
}
