//! LLM configuration.
//!
//! The file holds **no secrets and no URLs** — model specs, thresholds, and the
//! semantic-cache policy only. Endpoints and keys come from the environment, so a
//! committed config can never pin a provider host, and an unconfigured provider is
//! reported rather than defaulted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::client::{DecoderKind, Purpose};
use crate::error::LlmError;

/// How hard a task is, used to pick a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Complexity {
    /// A lookup or a one-liner.
    Low,
    /// Ordinary work.
    Medium,
    /// Multi-step reasoning.
    High,
    /// The hardest work the being does.
    VeryHigh,
}

impl Complexity {
    /// Parse the stored form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Complexity::Low),
            "medium" => Some(Complexity::Medium),
            "high" => Some(Complexity::High),
            "very_high" => Some(Complexity::VeryHigh),
            _ => None,
        }
    }

    /// The stored form.
    pub fn as_str(self) -> &'static str {
        match self {
            Complexity::Low => "low",
            Complexity::Medium => "medium",
            Complexity::High => "high",
            Complexity::VeryHigh => "very_high",
        }
    }
}

/// How much a wrong answer costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stakes {
    /// Reversible and cheap.
    Low,
    /// Reversible with effort.
    Medium,
    /// Hard to undo.
    High,
}

/// How exact the answer must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    /// A rough answer is fine.
    Low,
    /// The usual bar.
    Normal,
    /// Exactness matters.
    High,
}

/// One model the substrate may select.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    /// The name the adapter understands.
    pub name: String,
    /// Prompt cost per 1k tokens, in micros.
    pub input_cost_micros_per_1k: i64,
    /// Completion cost per 1k tokens, in micros.
    pub output_cost_micros_per_1k: i64,
    /// The latency the router should assume.
    pub typical_latency_ms: u32,
    /// The hardest complexity this model is allowed to take.
    pub complexity_ceiling: Complexity,
    /// True when this spec names a deterministic test double rather than a model a
    /// provider serves. Such a spec may be pinned (the mock client and a recorded
    /// replay both need its name) but is never chosen automatically: its name is
    /// not a request a real endpoint can answer.
    #[serde(default)]
    pub mock: bool,
}

impl ModelSpec {
    /// What a call with this usage costs.
    ///
    /// Rounded up per *thousand tokens*: a partial thousand is billed as a whole
    /// thousand, which is how providers bill and therefore how a budget must
    /// estimate it. Rounding the final cost instead would under-report exactly the
    /// short requests a small budget is most sensitive to.
    pub fn cost_micros(&self, tokens_in: u32, tokens_out: u32) -> i64 {
        fn thousand(tokens: u32) -> i64 {
            (i64::from(tokens) + 999) / 1000
        }
        thousand(tokens_in) * self.input_cost_micros_per_1k
            + thousand(tokens_out) * self.output_cost_micros_per_1k
    }

    /// The most this model can cost for a call with `max_tokens` completion room.
    pub fn worst_case_cost_micros(&self, prompt_tokens: u32, max_tokens: u32) -> i64 {
        self.cost_micros(prompt_tokens, max_tokens)
    }
}

/// How structured requests ask the provider for JSON.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutput {
    /// Send the full JSON Schema in `response_format` (OpenAI-style).
    #[default]
    JsonSchema,
    /// Ask only for valid JSON and validate against the schema afterwards.
    /// This is the compatible mode for NIM-backed endpoints.
    JsonObject,
    /// Send no `response_format`; validation and repair still apply.
    None,
}

/// Substrate-wide defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LlmDefaults {
    /// The preferred decoder.
    pub decoder: DecoderKind,
    /// Purposes for which the semantic cache may be consulted.
    pub semantic_cache_purposes: Vec<Purpose>,
    /// The similarity a semantic hit must reach.
    pub semantic_cache_threshold: f64,
    /// How many repair passes a rejected structured call gets.
    pub max_repair_attempts: u32,
    /// Optional hard ceiling for one call. `0` means unbounded: a thinking model
    /// is long by design, and a wall clock would cut it mid-answer.
    pub timeout_ms: u32,
    /// How long one connect/write/read window may stall before the call is cut.
    /// This is the progress bound that replaces the wall clock.
    pub idle_timeout_ms: u32,
    /// Fallback completion ceiling when a purpose has no entry below.
    pub max_tokens: u32,
    /// Completion ceiling per purpose, keyed by `Purpose::as_str()`. The budget is
    /// a property of the work: `classify` and `agent` cannot share one number.
    pub max_tokens_by_purpose: BTreeMap<String, u32>,
    /// Default model when routing is unavailable.
    pub default_model: Option<String>,
    /// How structured requests ask the provider for JSON.
    pub structured_output: StructuredOutput,
}

impl Default for LlmDefaults {
    fn default() -> Self {
        LlmDefaults {
            decoder: DecoderKind::ProviderNative,
            semantic_cache_purposes: Vec::new(),
            semantic_cache_threshold: 0.97,
            max_repair_attempts: 1,
            timeout_ms: 0,
            idle_timeout_ms: 300_000,
            max_tokens: 2048,
            max_tokens_by_purpose: BTreeMap::new(),
            default_model: None,
            structured_output: StructuredOutput::JsonSchema,
        }
    }
}

/// The resolved configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Substrate defaults.
    #[serde(default)]
    pub defaults: LlmDefaults,
    /// The selectable models.
    #[serde(default, rename = "model")]
    pub models: Vec<ModelSpec>,
    /// Where cached response bodies live. Not part of the file.
    #[serde(skip, default = "default_cache_dir")]
    pub cache_dir: PathBuf,
}

fn default_cache_dir() -> PathBuf {
    PathBuf::from("data/llm_cache")
}

/// A numeric environment override, ignored when absent or unparseable.
fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.trim().parse().ok()
}

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            defaults: LlmDefaults::default(),
            models: Vec::new(),
            cache_dir: default_cache_dir(),
        }
    }
}

impl LlmConfig {
    /// Parse a `config/llm.toml` document.
    pub fn from_toml_str(raw: &str) -> Result<Self, LlmError> {
        let cfg: LlmConfig = toml::from_str(raw)
            .map_err(|e| LlmError::Config(format!("invalid llm config: {e}")))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Read `config/llm.toml`.
    pub fn load(path: &Path) -> Result<Self, LlmError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| LlmError::Config(format!("cannot read {}: {e}", path.display())))?;
        let mut cfg = Self::from_toml_str(&raw)?;
        cfg.apply_env_overrides();
        Ok(cfg)
    }

    /// Operator overrides for the two knobs that differ per deployment.
    ///
    /// The per-purpose table stays file-only: it is a policy about the work, not
    /// about the machine, and a second override surface would let them drift.
    fn apply_env_overrides(&mut self) {
        if let Some(max_tokens) = env_u32("MM_LLM_MAX_TOKENS") {
            self.defaults.max_tokens = max_tokens;
        }
        if let Some(timeout_ms) = env_u32("MM_LLM_TIMEOUT_MS") {
            self.defaults.timeout_ms = timeout_ms;
        }
        if let Some(idle) = env_u32("MM_LLM_IDLE_TIMEOUT_MS") {
            self.defaults.idle_timeout_ms = idle;
        }
    }

    /// Read `config/llm.toml`, or the built-in defaults when it is absent.
    ///
    /// Absence is not an error: the substrate has to be constructible in a test
    /// with no repository config at all.
    pub fn load_or_default(path: &Path) -> Result<Self, LlmError> {
        if path.is_file() {
            Self::load(path)
        } else {
            Ok(LlmConfig::default())
        }
    }

    /// The spec for a model name.
    pub fn model(&self, name: &str) -> Option<&ModelSpec> {
        self.models.iter().find(|m| m.name == name)
    }

    /// True when the semantic cache may be consulted for `purpose`.
    pub fn semantic_cache_allowed(&self, purpose: Purpose) -> bool {
        self.defaults.semantic_cache_purposes.contains(&purpose)
    }

    /// The completion ceiling for `purpose`.
    ///
    /// A purpose entry wins over the fallback. Callers that set their own budget
    /// never reach this: the sentinel check in the service keeps explicit budgets.
    pub fn max_tokens_for(&self, purpose: Purpose) -> u32 {
        self.defaults
            .max_tokens_by_purpose
            .get(purpose.as_str())
            .copied()
            .unwrap_or(self.defaults.max_tokens)
    }

    fn validate(&self) -> Result<(), LlmError> {
        if !(0.0..=1.0).contains(&self.defaults.semantic_cache_threshold) {
            return Err(LlmError::Config(format!(
                "semantic_cache_threshold must be within 0.0..=1.0, got {}",
                self.defaults.semantic_cache_threshold
            )));
        }
        let mut names: Vec<&str> = self.models.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        let unique = {
            let mut n = names.clone();
            n.dedup();
            n
        };
        if unique.len() != names.len() {
            return Err(LlmError::Config("model names must be unique".into()));
        }
        for model in &self.models {
            if model.name.trim().is_empty() {
                return Err(LlmError::Config("a model name must not be empty".into()));
            }
        }
        for key in self.defaults.max_tokens_by_purpose.keys() {
            if Purpose::parse(key).is_none() {
                return Err(LlmError::Config(format!(
                    "max_tokens_by_purpose has an unknown purpose `{key}`; known: {}",
                    Purpose::ALL
                        .iter()
                        .map(|p| p.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        Ok(())
    }
}

/// The provider endpoint and key, read from the environment.
///
/// Never serialized, never logged: [`ProviderEnv::redacted`] is the only rendering
/// anything outside the adapter may use.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderEnv {
    /// The base URL the adapter appends its path to.
    pub base_url: String,
    /// The bearer token.
    pub api_key: String,
}

impl ProviderEnv {
    /// Read `OPENAI_BASE_URL` / `OPENAI_API_KEY`, or `None` when either is unset.
    ///
    /// `None` is the honest answer: there is no fallback host to silently use.
    pub fn from_env() -> Option<Self> {
        Self::from_pairs(
            std::env::var("OPENAI_BASE_URL").ok(),
            std::env::var("OPENAI_API_KEY").ok(),
        )
    }

    /// The same decision, from explicit values (so it is testable without env).
    pub fn from_pairs(base_url: Option<String>, api_key: Option<String>) -> Option<Self> {
        let base_url = base_url.filter(|v| !v.trim().is_empty())?;
        let api_key = api_key.filter(|v| !v.trim().is_empty())?;
        Some(ProviderEnv { base_url, api_key })
    }

    /// A rendering safe to log: the host only, the key never.
    pub fn redacted(&self) -> String {
        let host = self
            .base_url
            .split("//")
            .nth(1)
            .unwrap_or(&self.base_url)
            .split('/')
            .next()
            .unwrap_or("");
        format!("host={host} key=<redacted>")
    }
}

impl std::fmt::Debug for ProviderEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.redacted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[defaults]
decoder = "llguidance"
semantic_cache_purposes = ["summarize"]
semantic_cache_threshold = 0.97
max_repair_attempts = 2
timeout_ms = 5000


[[model]]
name = "small"
input_cost_micros_per_1k = 150
output_cost_micros_per_1k = 600
typical_latency_ms = 400
complexity_ceiling = "medium"

[[model]]
name = "frontier"
input_cost_micros_per_1k = 2500
output_cost_micros_per_1k = 10000
typical_latency_ms = 2500
complexity_ceiling = "very_high"
"#;

    #[test]
    fn the_repository_config_parses() {
        // `mm-serve` degrades to `LlmConfig::default()` when this file does not
        // parse, which silently empties the model catalog, so the real file is
        // checked here rather than only the sample above.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/llm.toml");
        let cfg = LlmConfig::load(&path).expect("config/llm.toml must parse");
        assert!(!cfg.models.is_empty(), "the catalog must not be empty");
        assert_eq!(cfg.max_tokens_for(Purpose::Converse), 4096);
        assert_eq!(cfg.defaults.timeout_ms, 0);
        assert_eq!(cfg.defaults.idle_timeout_ms, 300_000);
    }

    #[test]
    fn a_purpose_budget_replaces_the_default() {
        let cfg = LlmConfig::from_toml_str(
            r#"
[defaults]
max_tokens = 2048

[defaults.max_tokens_by_purpose]
converse = 4096
classify = 512

[[model]]
name = "small"
input_cost_micros_per_1k = 150
output_cost_micros_per_1k = 600
typical_latency_ms = 400
complexity_ceiling = "medium"
"#,
        )
        .unwrap();
        assert_eq!(cfg.max_tokens_for(Purpose::Converse), 4096);
        assert_eq!(cfg.max_tokens_for(Purpose::Classify), 512);
        assert_eq!(
            cfg.max_tokens_for(Purpose::Summarize),
            2048,
            "a purpose without an entry falls back to max_tokens"
        );
    }

    #[test]
    fn an_unknown_purpose_budget_is_rejected_at_load() {
        let error = LlmConfig::from_toml_str(
            r#"
[defaults.max_tokens_by_purpose]
converes = 4096
"#,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("converes"),
            "the message names the offending key: {error}"
        );
    }

    #[test]
    fn an_absent_ceiling_means_unbounded_with_an_idle_bound() {
        let defaults = LlmDefaults::default();
        assert_eq!(defaults.timeout_ms, 0, "no wall clock by default");
        assert_eq!(defaults.idle_timeout_ms, 300_000);
    }

    #[test]
    fn the_sample_config_parses_and_orders_models_by_cost() {
        let cfg = LlmConfig::from_toml_str(SAMPLE).unwrap();
        assert_eq!(cfg.defaults.decoder, DecoderKind::Llguidance);
        assert_eq!(cfg.defaults.semantic_cache_threshold, 0.97);
        assert_eq!(cfg.defaults.max_repair_attempts, 2);
        assert_eq!(cfg.models.len(), 2);
        assert!(
            cfg.model("small").unwrap().input_cost_micros_per_1k
                < cfg.model("frontier").unwrap().input_cost_micros_per_1k
        );
        assert!(cfg.semantic_cache_allowed(Purpose::Summarize));
        assert!(
            !cfg.semantic_cache_allowed(Purpose::Plan),
            "consequential work is exact-cache only"
        );
    }

    #[test]
    fn cost_rounds_up_per_started_thousand() {
        let spec = ModelSpec {
            name: "m".into(),
            input_cost_micros_per_1k: 150,
            output_cost_micros_per_1k: 600,
            typical_latency_ms: 1,
            complexity_ceiling: Complexity::Low,
            mock: false,
        };
        assert_eq!(spec.cost_micros(0, 0), 0);
        assert_eq!(spec.cost_micros(1000, 1000), 150 + 600);
        // One token past a thousand is billed as another thousand.
        assert_eq!(spec.cost_micros(1001, 0), 300);
        assert_eq!(spec.cost_micros(1, 0), 150);
    }

    #[test]
    fn a_bad_threshold_is_rejected() {
        let raw = format!("[defaults]\nsemantic_cache_threshold = {}\n", 1.5);
        let err = LlmConfig::from_toml_str(&raw).unwrap_err();
        assert_eq!(err.kind(), "config");
    }

    #[test]
    fn duplicate_model_names_are_rejected() {
        let raw = r#"
[[model]]
name = "x"
input_cost_micros_per_1k = 1
output_cost_micros_per_1k = 1
typical_latency_ms = 1
complexity_ceiling = "low"

[[model]]
name = "x"
input_cost_micros_per_1k = 2
output_cost_micros_per_1k = 2
typical_latency_ms = 2
complexity_ceiling = "high"
"#;
        assert!(LlmConfig::from_toml_str(raw).is_err());
    }

    #[test]
    fn provider_env_needs_both_values_and_never_renders_the_key() {
        assert!(ProviderEnv::from_pairs(None, Some("k".into())).is_none());
        assert!(ProviderEnv::from_pairs(Some("u".into()), None).is_none());
        assert!(ProviderEnv::from_pairs(Some("  ".into()), Some("k".into())).is_none());
        let env = ProviderEnv::from_pairs(
            Some("http://127.0.0.1:9/v1".into()),
            Some("secret-key".into()),
        )
        .unwrap();
        let rendered = format!("{env:?}");
        assert!(!rendered.contains("secret-key"), "{rendered}");
        assert!(rendered.contains("host=127.0.0.1:9"), "{rendered}");
        assert!(rendered.contains("<redacted>"), "{rendered}");
    }

    #[test]
    fn a_missing_file_falls_back_to_defaults_but_a_bad_file_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = LlmConfig::load_or_default(&dir.path().join("absent.toml")).unwrap();
        assert_eq!(cfg, LlmConfig::default());

        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "not = = toml").unwrap();
        assert!(LlmConfig::load_or_default(&bad).is_err());
    }

    #[test]
    fn complexity_and_the_other_labels_round_trip() {
        for (s, c) in [
            ("low", Complexity::Low),
            ("medium", Complexity::Medium),
            ("high", Complexity::High),
            ("very_high", Complexity::VeryHigh),
        ] {
            assert_eq!(Complexity::parse(s), Some(c));
            assert_eq!(c.as_str(), s);
        }
        assert!(Complexity::Low < Complexity::VeryHigh);
    }
}
