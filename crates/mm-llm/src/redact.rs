//! Redaction and bounded excerpts for LLM-visible text.
//!
//! The kernel `mm-log` policy retired with the audit logger. This module keeps a
//! small literal-rule redactor with conservative defaults for key-shaped strings,
//! so a prompt excerpt cannot carry an obvious credential.

use std::path::Path;

use mm_core::MmError;

/// A bounded, control-character-free excerpt of `text`.
pub fn excerpt(text: &str, max_chars: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max_chars));
    for (i, ch) in text.chars().enumerate() {
        if i >= max_chars {
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    out
}

/// Literal, case-insensitive secret patterns.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    rules: Vec<String>,
}

impl Redactor {
    /// A redactor with conservative defaults.
    pub fn new() -> Self {
        Self {
            rules: vec![
                "sk-".into(),
                "mm-access-key-".into(),
                "authorization: bearer".into(),
            ],
        }
    }

    /// Add one literal pattern.
    pub fn with_rule(mut self, pattern: impl Into<String>) -> Self {
        let pattern = pattern.into();
        if !pattern.is_empty() {
            self.rules.push(pattern);
        }
        self
    }

    /// Add patterns from a newline-separated fixture file, when it exists.
    pub fn with_fixture(mut self, path: &Path) -> Result<Self, MmError> {
        if !path.exists() {
            return Ok(self);
        }
        let text = std::fs::read_to_string(path)
            .map_err(|error| MmError::Config(format!("{}: {error}", path.display())))?;
        for line in text.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                self = self.with_rule(line);
            }
        }
        Ok(self)
    }

    /// Redact `text`, returning it and how many rules matched.
    pub fn apply(&self, text: &str) -> (String, u32) {
        let mut out = text.to_owned();
        let mut hits = 0;
        for rule in &self.rules {
            let lower = out.to_ascii_lowercase();
            let needle = rule.to_ascii_lowercase();
            let mut start = 0;
            let mut found = Vec::new();
            while let Some(index) = lower[start..].find(&needle) {
                found.push(start + index);
                start += index + needle.len();
            }
            for index in found.iter().rev() {
                let end = (index + rule.len()).min(out.len());
                out.replace_range(*index..end, "[redacted]");
                hits += 1;
            }
        }
        (out, hits)
    }

    /// Redact, then bound.
    pub fn excerpt(&self, text: &str, max_chars: usize) -> String {
        let (clean, _) = self.apply(text);
        excerpt(&clean, max_chars)
    }

    /// True when `key` names a value the redactor treats as secret.
    pub fn is_secret_key(&self, key: &str) -> bool {
        let lower = key.to_ascii_lowercase();
        ["key", "token", "secret", "password", "authorization"]
            .iter()
            .any(|needle| lower.contains(needle))
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
        assert_eq!(excerpt("éééé", 2), "éé");
    }

    #[test]
    fn a_secret_never_survives_a_redacted_excerpt() {
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
            .with_fixture(Path::new("/nonexistent/patterns.txt"))
            .unwrap();
        assert_eq!(redactor.apply("nothing secret").1, 0);
    }
}
