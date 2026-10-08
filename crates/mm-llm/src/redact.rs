//! Redaction and bounded excerpts for every LLM sink.
//!
//! A prompt is caller data, and caller data can contain a token. So the substrate
//! never logs a prompt: it logs its sha256 (see [`crate::cached::prompt_hash`]) and,
//! where a human needs context, a length-bounded excerpt that has already been
//! through the kernel's redaction policy. The policy is shared with `mm-log`'s
//! sinks so a secret pattern added for the audit chain protects the LLM records in
//! the same change.

use std::path::Path;

use mm_core::MmError;
use mm_log::RedactionPolicy;

/// A bounded, control-character-free excerpt of `text`.
///
/// Deliberately truncates without an ellipsis: the result is a *field value*, and
/// a marker would be indistinguishable from content that happens to contain one.
/// Callers that need to signal truncation compare `excerpt.len()` with the source.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_chars));
    for (i, ch) in text.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        // Newlines and tabs in a log field are how a "one-line" record stops being
        // one line, so control characters become spaces rather than being dropped.
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out
}

/// The kernel's secret-pattern policy, applied to LLM-visible text.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    policy: RedactionPolicy,
}

impl Redactor {
    /// The kernel default: no extra patterns.
    pub fn new() -> Self {
        Redactor {
            policy: RedactionPolicy::kernel_default(),
        }
    }

    /// Add one pattern to the policy.
    pub fn with_rule(mut self, pattern: impl Into<String>) -> Self {
        self.policy = self.policy.with_rule(pattern);
        self
    }

    /// Extend the policy from the shared fixture file, when it exists.
    ///
    /// Existence is not required: the kernel's own default policy is the floor, and
    /// a test that runs without the fixture must not fail because of it.
    pub fn with_fixture(self, path: &Path) -> Result<Self, MmError> {
        if !path.exists() {
            return Ok(self);
        }
        Ok(Redactor {
            policy: self.policy.extend_from_file(path)?,
        })
    }

    /// Redact `text`, returning it and how many replacements were made.
    pub fn apply(&self, text: &str) -> (String, u32) {
        self.policy.apply_to_str(text)
    }

    /// Redact, then bound: an excerpt can never contain a secret the policy knows.
    pub fn excerpt(&self, text: &str, max_chars: usize) -> String {
        let (clean, _) = self.apply(text);
        excerpt(&clean, max_chars)
    }

    /// True when `key` names a value the policy treats as secret, so a caller can
    /// drop it from a structured field rather than hash it.
    pub fn is_secret_key(&self, key: &str) -> bool {
        self.policy.is_secret_key(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_excerpt_is_bounded_and_single_line() {
        assert_eq!(excerpt("hello world", 5), "hello");
        assert_eq!(excerpt("hi", 5), "hi");
        assert_eq!(excerpt("", 5), "");
        assert_eq!(excerpt("a\nb", 10), "a b");
        // Character-counted, not byte-counted, so a multi-byte cut stays valid.
        assert_eq!(excerpt("éééé", 2), "éé");
    }

    #[test]
    fn a_secret_never_survives_a_redacted_excerpt() {
        // Patterns are literals, compared ASCII-case-insensitively — the same shape
        // the audit-chain policy uses, and the same shape the fixture file carries.
        let redactor = Redactor::new().with_rule("mm-access-key-");
        let (clean, hits) = redactor.apply("token mm-access-key-abcdef end");
        assert!(hits >= 1, "{clean}");
        assert!(!clean.contains("mm-access-key-abcdef"), "{clean}");
        let bounded = redactor.excerpt("token mm-access-key-abcdef end", 40);
        assert!(!bounded.contains("mm-access-key-abcdef"), "{bounded}");
    }

    #[test]
    fn a_missing_fixture_is_not_an_error() {
        let redactor = Redactor::new()
            .with_fixture(Path::new("/nonexistent/patterns.json"))
            .unwrap();
        assert_eq!(redactor.apply("nothing secret").1, 0);
    }
}
