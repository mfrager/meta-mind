//! Deterministic prompt strategies and frame templates.
//!
//! The prompt manager treats every prompt as reviewed data, not code. A
//! strategy names fragments in order; a fragment is plain TOML text; the
//! renderer substitutes `{{slot}}` once, in one pass. There are no includes,
//! expressions, functions, or recursive substitutions.
//!
//! Phase 0 ships the loader, strict renderer, immutable [`PromptInstance`], and
//! manifest validation. The CME executor does not consume prompts yet.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Root prompt manifest schema.
pub const MANIFEST_SCHEMA: &str = "mm.prompt.manifest.v1";
/// Fragment file schema.
pub const FRAGMENT_SCHEMA: &str = "mm.prompt.fragment.v1";
/// Strategy file schema.
pub const STRATEGY_SCHEMA: &str = "mm.prompt.strategy.v1";
/// Frame template schema.
pub const FRAME_SCHEMA: &str = "mm.frame.template.v1";
/// Frame manifest schema.
pub const FRAME_MANIFEST_SCHEMA: &str = "mm.frame.manifest.v1";

/// One reviewed text fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptFragment {
    /// File schema.
    pub schema: String,
    /// Stable id, for example `identity/character`.
    pub id: String,
    /// Reviewed version.
    pub version: u32,
    /// Plain prompt text.
    pub text: String,
}

/// One strategy, assembled from fragments in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptStrategy {
    /// File schema.
    pub schema: String,
    /// Stable id, for example `cm.turn.v1`.
    pub id: String,
    /// Reviewed version.
    pub version: u32,
    /// Optional operator-facing description.
    #[serde(default)]
    pub description: String,
    /// Application key, for example `cm.turn`.
    #[serde(default)]
    pub application: String,
    /// Default budget class for this strategy.
    #[serde(default)]
    pub budget: String,
    /// Supported variants, in selection order.
    #[serde(default)]
    pub variants: Vec<String>,
    /// Variables that must be present at render time.
    #[serde(default)]
    pub required_variables: Vec<String>,
    /// Fragment ids in render order.
    pub fragments: Vec<String>,
    /// Every slot the fragments may use.
    #[serde(default)]
    pub slots: Vec<String>,
}

/// A frame template used by `frame.utterance.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameTemplate {
    /// File schema.
    pub schema: String,
    /// Stable template id, for example `frame.intro.connect`.
    pub id: String,
    /// Reviewed version.
    pub version: u32,
    /// Frame kind (`intro`, `work`, `goodbye`, …).
    pub kind: String,
    /// Step declared by this template.
    pub step: String,
    /// Prompt strategy used to produce the utterance.
    pub strategy: String,
    /// Plain prompt text for the utterance.
    pub text: String,
    /// Slots used by `text`.
    #[serde(default)]
    pub slots: Vec<String>,
    /// Named transitions, for example `task = "frame.work"`.
    #[serde(default)]
    pub transitions: BTreeMap<String, String>,
    /// Scheduling defaults.
    #[serde(default)]
    pub utterance: UtterancePolicy,
}

/// Frame utterance timing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UtterancePolicy {
    /// Base delay before speaking.
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u64,
    /// Fractional jitter, 0.0–1.0.
    #[serde(default)]
    pub jitter: f64,
    /// Maximum unanswered attempts.
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u8,
    /// Delay multiplier after an unanswered attempt.
    #[serde(default = "default_backoff")]
    pub backoff: u32,
}

impl Default for UtterancePolicy {
    fn default() -> Self {
        Self {
            delay_ms: default_delay_ms(),
            jitter: 0.0,
            max_attempts: default_max_attempts(),
            backoff: default_backoff(),
        }
    }
}

fn default_delay_ms() -> u64 {
    4_500
}

fn default_max_attempts() -> u8 {
    2
}

fn default_backoff() -> u32 {
    3
}

/// The frame manifest pins the complete set of templates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameManifest {
    /// File schema.
    pub schema: String,
    /// Manifest version.
    pub version: u32,
    /// Root frame template.
    pub root: String,
    /// Every template id that must exist.
    pub templates: Vec<String>,
}

/// The root manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptManifest {
    /// File schema.
    pub schema: String,
    /// Tree version.
    pub version: u32,
}

/// One rendered fragment within an instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedFragment {
    /// Fragment id.
    pub id: String,
    /// Pinned fragment version.
    pub version: u32,
    /// Rendered text.
    pub text: String,
}

/// One ordered message in a rendered prompt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMessage {
    /// `system`, `transcript`, or `user`.
    pub role: String,
    /// Rendered content.
    pub content: String,
}

/// A request to select and render one strategy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRequest {
    /// Strategy id or application key.
    pub application: String,
    /// Optional `standard`, `pending`, `resume`, or `repair` variant.
    pub variant: Option<String>,
    /// Template variables.
    pub variables: BTreeMap<String, String>,
    /// Canonical `[STATE]` block, when the strategy leaves state out of its fragments.
    pub state_block: Option<String>,
    /// Compressed prior transcript.
    pub transcript: Vec<PromptMessage>,
    /// The operator's current message.
    pub user_input: Option<String>,
}

/// An immutable, hash-addressed prompt rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptInstance {
    /// Strategy id.
    pub strategy: String,
    /// Application key.
    pub application: String,
    /// Selected variant, when one was requested.
    pub variant: Option<String>,
    /// Strategy version.
    pub strategy_version: u32,
    /// Lowercase SHA-256 over the canonical instance payload.
    pub hash: String,
    /// Stable instance id (`sha256:<hash>`).
    pub instance_id: String,
    /// Rendered fragments in order.
    pub fragments: Vec<RenderedFragment>,
    /// Ordered system/state/transcript/user messages.
    pub messages: Vec<PromptMessage>,
    /// Joined text with blank lines between fragments.
    pub text: String,
    /// Input variables, sorted by key.
    pub variables: BTreeMap<String, String>,
}

impl PromptInstance {
    /// The redacted audit record carried by logs and the inspector.
    pub fn audit(&self) -> serde_json::Value {
        serde_json::json!({
            "instanceId": self.instance_id,
            "strategy": self.strategy,
            "application": self.application,
            "variant": self.variant,
            "strategyVersion": self.strategy_version,
            "hash": self.hash,
            "fragments": self.fragments.iter().map(|fragment| {
                serde_json::json!({"id": fragment.id, "version": fragment.version})
            }).collect::<Vec<_>>(),
            "messageCount": self.messages.len(),
        })
    }
}

/// Loaded prompt tree.
#[derive(Debug, Clone)]
pub struct PromptStore {
    root: PathBuf,
    manifest: PromptManifest,
    fragments: BTreeMap<String, PromptFragment>,
    strategies: BTreeMap<String, PromptStrategy>,
    frames: BTreeMap<String, FrameTemplate>,
    frame_manifest: FrameManifest,
}

impl PromptStore {
    /// Load and validate a prompt tree from `root`.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let manifest_text = std::fs::read_to_string(root.join("manifest.toml"))?;
        let manifest: PromptManifest = toml::from_str(&manifest_text)?;
        if manifest.schema != MANIFEST_SCHEMA {
            return Err(PromptError::Schema {
                path: root.join("manifest.toml"),
                expected: MANIFEST_SCHEMA,
                found: manifest.schema,
            });
        }

        let mut fragments = BTreeMap::new();
        for path in files_under(&root.join("fragments"))? {
            let value: PromptFragment = parse_toml(&path)?;
            if value.schema != FRAGMENT_SCHEMA {
                return Err(PromptError::Schema {
                    path,
                    expected: FRAGMENT_SCHEMA,
                    found: value.schema,
                });
            }
            if fragments.insert(value.id.clone(), value).is_some() {
                return Err(PromptError::DuplicateId(path));
            }
        }

        let mut strategies = BTreeMap::new();
        for path in files_under(&root.join("applications"))? {
            let value: PromptStrategy = parse_toml(&path)?;
            if value.schema != STRATEGY_SCHEMA {
                return Err(PromptError::Schema {
                    path,
                    expected: STRATEGY_SCHEMA,
                    found: value.schema,
                });
            }
            if strategies.insert(value.id.clone(), value).is_some() {
                return Err(PromptError::DuplicateId(path));
            }
        }

        // Frames were retired with the frame engine. A tree without
        // `frames/manifest.toml` is valid; when the directory exists it is still
        // validated exactly as before.
        let frame_manifest_path = root.join("frames/manifest.toml");
        let frame_manifest = if frame_manifest_path.exists() {
            let manifest: FrameManifest =
                toml::from_str(&std::fs::read_to_string(&frame_manifest_path)?)?;
            if manifest.schema != FRAME_MANIFEST_SCHEMA {
                return Err(PromptError::Schema {
                    path: frame_manifest_path,
                    expected: FRAME_MANIFEST_SCHEMA,
                    found: manifest.schema,
                });
            }
            manifest
        } else {
            FrameManifest {
                schema: FRAME_MANIFEST_SCHEMA.into(),
                version: 1,
                root: String::new(),
                templates: Vec::new(),
            }
        };

        let mut frames = BTreeMap::new();
        for path in files_under(&root.join("frames"))? {
            if path.file_name().is_some_and(|name| name == "manifest.toml") {
                continue;
            }
            let value: FrameTemplate = parse_toml(&path)?;
            if value.schema != FRAME_SCHEMA {
                return Err(PromptError::Schema {
                    path,
                    expected: FRAME_SCHEMA,
                    found: value.schema,
                });
            }
            if frames.insert(value.id.clone(), value).is_some() {
                return Err(PromptError::DuplicateId(path));
            }
        }

        let store = Self {
            root,
            manifest,
            fragments,
            strategies,
            frames,
            frame_manifest,
        };
        store.validate()?;
        Ok(store)
    }

    /// Root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Root manifest.
    pub fn manifest(&self) -> &PromptManifest {
        &self.manifest
    }

    /// A fragment by id.
    pub fn fragment(&self, id: &str) -> Option<&PromptFragment> {
        self.fragments.get(id)
    }

    /// A strategy by id.
    pub fn strategy(&self, id: &str) -> Option<&PromptStrategy> {
        self.strategies.get(id)
    }

    /// All strategies, sorted by id.
    pub fn strategies(&self) -> impl Iterator<Item = &PromptStrategy> {
        self.strategies.values()
    }

    /// A frame template by id.
    pub fn frame(&self, id: &str) -> Option<&FrameTemplate> {
        self.frames.get(id)
    }

    /// All frame templates, sorted by id.
    pub fn frames(&self) -> impl Iterator<Item = &FrameTemplate> {
        self.frames.values()
    }

    /// The frame manifest.
    pub fn frame_manifest(&self) -> &FrameManifest {
        &self.frame_manifest
    }

    /// Validate manifests, references, slots, and frame coverage.
    pub fn validate(&self) -> Result<()> {
        if self.manifest.version == 0 {
            return Err(PromptError::Invalid(
                "manifest version must be positive".into(),
            ));
        }
        for strategy in self.strategies.values() {
            if strategy.id.trim().is_empty() || strategy.version == 0 {
                return Err(PromptError::Invalid(format!(
                    "strategy {} has an empty id or zero version",
                    strategy.id
                )));
            }
            if strategy.fragments.is_empty() {
                return Err(PromptError::Invalid(format!(
                    "strategy {} has no fragments",
                    strategy.id
                )));
            }
            let mut seen_variants = BTreeSet::new();
            for variant in &strategy.variants {
                if !seen_variants.insert(variant) {
                    return Err(PromptError::Invalid(format!(
                        "strategy {} repeats variant {variant}",
                        strategy.id
                    )));
                }
            }
            let mut seen_required = BTreeSet::new();
            for variable in &strategy.required_variables {
                if !seen_required.insert(variable) {
                    return Err(PromptError::Invalid(format!(
                        "strategy {} repeats required variable {variable}",
                        strategy.id
                    )));
                }
                if !strategy.slots.iter().any(|slot| slot == variable) {
                    return Err(PromptError::Invalid(format!(
                        "strategy {} requires undeclared variable {variable}",
                        strategy.id
                    )));
                }
            }
            let mut seen_slots = BTreeSet::new();
            for slot in &strategy.slots {
                if !seen_slots.insert(slot) {
                    return Err(PromptError::Invalid(format!(
                        "strategy {} repeats slot {slot}",
                        strategy.id
                    )));
                }
            }
            let mut declared: BTreeSet<&str> = strategy.slots.iter().map(String::as_str).collect();
            for fragment_id in &strategy.fragments {
                let fragment = self.fragments.get(fragment_id).ok_or_else(|| {
                    PromptError::MissingFragment {
                        strategy: strategy.id.clone(),
                        fragment: fragment_id.clone(),
                    }
                })?;
                for slot in placeholders(&fragment.text)? {
                    if !declared.remove(slot.as_str())
                        && !strategy.slots.iter().any(|item| item == &slot)
                    {
                        return Err(PromptError::MissingSlot {
                            strategy: strategy.id.clone(),
                            fragment: fragment.id.clone(),
                            slot,
                        });
                    }
                }
            }
            // The remaining declared slots are allowed; a strategy may expose a
            // slot used by a later application-specific overlay.
        }

        for frame in self.frames.values() {
            if frame.version == 0 {
                return Err(PromptError::Invalid(format!(
                    "frame {} has zero version",
                    frame.id
                )));
            }
            if !self.strategies.contains_key(&frame.strategy) {
                return Err(PromptError::MissingStrategy {
                    owner: frame.id.clone(),
                    strategy: frame.strategy.clone(),
                });
            }
            for slot in placeholders(&frame.text)? {
                if !frame.slots.iter().any(|item| item == &slot) {
                    return Err(PromptError::MissingSlot {
                        strategy: frame.id.clone(),
                        fragment: frame.id.clone(),
                        slot,
                    });
                }
            }
            for target in frame.transitions.values() {
                if !self.frames.contains_key(target) {
                    return Err(PromptError::MissingFrame(target.clone()));
                }
            }
        }

        let declared: BTreeSet<&str> = self
            .frame_manifest
            .templates
            .iter()
            .map(String::as_str)
            .collect();
        let actual: BTreeSet<&str> = self.frames.keys().map(String::as_str).collect();
        if declared != actual {
            return Err(PromptError::Invalid(format!(
                "frame manifest lists {declared:?} but tree contains {actual:?}"
            )));
        }
        if !declared.is_empty() && !self.frames.contains_key(&self.frame_manifest.root) {
            return Err(PromptError::MissingFrame(self.frame_manifest.root.clone()));
        }
        Ok(())
    }

    /// Select a strategy by application and optional variant.
    ///
    /// The base strategy is the application's lowest-id strategy. A variant is
    /// selected when an exact `{application}.{variant}.v1` or `cm.repair.v1`
    /// strategy exists; otherwise the base strategy is used.
    pub fn strategy_for(
        &self,
        application: &str,
        variant: Option<&str>,
    ) -> Result<&PromptStrategy> {
        let exact = format!("{application}.v1");
        let base = self
            .strategies
            .get(&exact)
            .or_else(|| {
                self.strategies
                    .values()
                    .filter(|strategy| {
                        strategy.application == application
                            || strategy.id == application
                            || strategy.id.starts_with(&format!("{application}."))
                    })
                    .min_by_key(|strategy| strategy.id.as_str())
            })
            .ok_or_else(|| PromptError::UnknownStrategy(application.to_owned()))?;
        let Some(variant) = variant.filter(|value| *value != "standard") else {
            return Ok(base);
        };
        let candidates = [
            format!("{application}.{variant}.v1"),
            format!("cm.{variant}.v1"),
            variant.to_string(),
        ];
        for candidate in candidates {
            if let Some(strategy) = self.strategies.get(&candidate) {
                return Ok(strategy);
            }
        }
        Ok(base)
    }

    /// Render one strategy with the supplied variables.
    pub fn render(
        &self,
        strategy_id: &str,
        variables: &BTreeMap<String, String>,
    ) -> Result<PromptInstance> {
        let strategy = self
            .strategies
            .get(strategy_id)
            .ok_or_else(|| PromptError::UnknownStrategy(strategy_id.to_owned()))?;
        self.render_selected(strategy, None, variables, None, &[], None)
    }

    /// Select and render a strategy, including the ordered message envelope.
    pub fn render_request(&self, request: &PromptRequest) -> Result<PromptInstance> {
        let strategy = self.strategy_for(&request.application, request.variant.as_deref())?;
        self.render_selected(
            strategy,
            request.variant.clone(),
            &request.variables,
            request.state_block.as_deref(),
            &request.transcript,
            request.user_input.as_deref(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_selected(
        &self,
        strategy: &PromptStrategy,
        variant: Option<String>,
        variables: &BTreeMap<String, String>,
        state_block: Option<&str>,
        transcript: &[PromptMessage],
        user_input: Option<&str>,
    ) -> Result<PromptInstance> {
        for required in &strategy.required_variables {
            if !variables.contains_key(required) {
                return Err(PromptError::MissingVariable(required.clone()));
            }
        }
        let mut fragments = Vec::with_capacity(strategy.fragments.len());
        for fragment_id in &strategy.fragments {
            let fragment =
                self.fragments
                    .get(fragment_id)
                    .ok_or_else(|| PromptError::MissingFragment {
                        strategy: strategy.id.clone(),
                        fragment: fragment_id.clone(),
                    })?;
            fragments.push(RenderedFragment {
                id: fragment.id.clone(),
                version: fragment.version,
                text: render_text(&fragment.text, variables)?,
            });
        }
        let fragment_text = fragments
            .iter()
            .map(|fragment| fragment.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = match state_block {
            Some(block) => format!("{fragment_text}\n\n{block}"),
            None => fragment_text.clone(),
        };
        let mut messages: Vec<PromptMessage> = fragments
            .iter()
            .map(|fragment| PromptMessage {
                role: "system".into(),
                content: fragment.text.clone(),
            })
            .collect();
        if let Some(block) = state_block {
            messages.push(PromptMessage {
                role: "system".into(),
                content: block.into(),
            });
        }
        messages.extend(transcript.iter().cloned());
        if let Some(input) = user_input {
            messages.push(PromptMessage {
                role: "user".into(),
                content: input.into(),
            });
        }
        let payload = serde_json::json!({
            "application": strategy.application,
            "variant": variant,
            "strategy": strategy.id,
            "strategyVersion": strategy.version,
            "fragments": fragments.iter().map(|fragment| {
                serde_json::json!({
                    "id": fragment.id,
                    "version": fragment.version,
                    "text": fragment.text,
                })
            }).collect::<Vec<_>>(),
            "messages": messages,
            "variables": variables,
        });
        let canonical = serde_json::to_vec(&payload)?;
        let hash = hex_sha256(&canonical);
        Ok(PromptInstance {
            strategy: strategy.id.clone(),
            application: strategy.application.clone(),
            variant,
            strategy_version: strategy.version,
            instance_id: format!("sha256:{hash}"),
            hash,
            fragments,
            messages,
            text,
            variables: variables.clone(),
        })
    }
}

fn parse_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

fn files_under(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "toml")
        {
            out.push(path);
        }
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Strict single-pass substitution. A value is never re-scanned.
pub fn render_text(text: &str, variables: &BTreeMap<String, String>) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err(PromptError::UnbalancedPlaceholder(text.to_owned()));
        };
        let key = after[..close].trim();
        if key.is_empty() {
            return Err(PromptError::UnbalancedPlaceholder(text.to_owned()));
        }
        let value = variables
            .get(key)
            .ok_or_else(|| PromptError::MissingVariable(key.to_owned()))?;
        out.push_str(value);
        rest = &after[close + 2..];
    }
    if rest.contains("}}") {
        return Err(PromptError::UnbalancedPlaceholder(text.to_owned()));
    }
    out.push_str(rest);
    Ok(out)
}

fn placeholders(text: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find("{{") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err(PromptError::UnbalancedPlaceholder(text.to_owned()));
        };
        let key = after[..close].trim();
        if key.is_empty() {
            return Err(PromptError::UnbalancedPlaceholder(text.to_owned()));
        }
        out.push(key.to_owned());
        rest = &after[close + 2..];
    }
    if rest.contains("}}") {
        return Err(PromptError::UnbalancedPlaceholder(text.to_owned()));
    }
    Ok(out)
}

/// Prompt-tree failures.
#[derive(Debug, Error)]
pub enum PromptError {
    /// Filesystem failure.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// TOML parse failure.
    #[error("toml: {0}")]
    Toml(#[from] toml::de::Error),
    /// JSON canonicalization failure.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// A document declares the wrong schema.
    #[error("schema in {}: expected {expected}, found {found}", path.display())]
    Schema {
        /// Source file.
        path: PathBuf,
        /// Expected schema.
        expected: &'static str,
        /// Found schema.
        found: String,
    },
    /// Two documents share an id.
    #[error("duplicate id in {}", .0.display())]
    DuplicateId(PathBuf),
    /// A strategy names a missing fragment.
    #[error("strategy {strategy} references missing fragment {fragment}")]
    MissingFragment {
        /// Strategy id.
        strategy: String,
        /// Fragment id.
        fragment: String,
    },
    /// A strategy names a missing strategy.
    #[error("{owner} references missing strategy {strategy}")]
    MissingStrategy {
        /// Owner id.
        owner: String,
        /// Strategy id.
        strategy: String,
    },
    /// A frame names a missing frame.
    #[error("missing frame {0}")]
    MissingFrame(String),
    /// A fragment placeholder is not declared by its owner.
    #[error("{strategy}/{fragment} uses undeclared slot {slot}")]
    MissingSlot {
        /// Strategy or template id.
        strategy: String,
        /// Fragment or template id.
        fragment: String,
        /// Slot name.
        slot: String,
    },
    /// A strategy id is unknown.
    #[error("unknown strategy {0}")]
    UnknownStrategy(String),
    /// A requested variable is missing.
    #[error("missing prompt variable {0}")]
    MissingVariable(String),
    /// A template contains a malformed placeholder.
    #[error("unbalanced or empty placeholder in {0:?}")]
    UnbalancedPlaceholder(String),
    /// General invariant failure.
    #[error("invalid prompt tree: {0}")]
    Invalid(String),
}

/// Result alias for prompt operations.
pub type Result<T, E = PromptError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> PromptStore {
        PromptStore::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"))
            .expect("checked-in prompt tree loads")
    }

    #[test]
    fn every_strategy_renders_deterministically() {
        let store = tree();
        let variables = test_variables(&store);
        for strategy in store.strategies() {
            let first = store.render(&strategy.id, &variables).unwrap();
            let second = store.render(&strategy.id, &variables).unwrap();
            assert_eq!(first.hash, second.hash, "{}", strategy.id);
            assert_eq!(first.text, second.text, "{}", strategy.id);
            assert!(!first.text.is_empty(), "{}", strategy.id);
        }
    }

    #[test]
    fn placeholders_are_single_pass() {
        let mut vars = BTreeMap::new();
        vars.insert("name".into(), "{{other}}".into());
        assert_eq!(render_text("hi {{name}}", &vars).unwrap(), "hi {{other}}");
        assert!(render_text("hi {{missing}}", &vars).is_err());
        assert!(render_text("hi {{name", &vars).is_err());
    }

    #[test]
    fn a_tree_without_frames_loads() {
        let store = tree();
        let manifest = store.frame_manifest();
        assert_eq!(manifest.templates.len(), store.frames().count());
        for frame in store.frames() {
            assert!(
                store.strategy(&frame.strategy).is_some(),
                "{} missing strategy",
                frame.id
            );
        }
    }

    #[test]
    fn missing_variable_is_an_error() {
        let store = tree();
        let error = store
            .render("cm.process.v1", &BTreeMap::new())
            .expect_err("slots are required");
        assert!(matches!(error, PromptError::MissingVariable(_)));
    }

    fn test_variables(store: &PromptStore) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        for strategy in store.strategies() {
            for slot in &strategy.slots {
                values.insert(slot.clone(), format!("fixture:{slot}"));
            }
        }
        for frame in store.frames() {
            for slot in &frame.slots {
                values.insert(slot.clone(), format!("fixture:{slot}"));
            }
        }
        values
    }
}
