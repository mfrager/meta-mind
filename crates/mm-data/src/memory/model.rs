//! Identifiers and shared enums for the four memory tiers.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::rdf::now_ms;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            /// Wrap an existing identifier.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// The identifier text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(UserId);
id_type!(ProjectId);
id_type!(AppId);
id_type!(DatasetId);
id_type!(SessionId);
id_type!(TurnId);
id_type!(EventId);
id_type!(SegmentId);
id_type!(DecisionId);
id_type!(CommitmentId);
id_type!(QuestionId);
id_type!(ArtifactId);
id_type!(GoalId);
id_type!(RuleId);
id_type!(SubgoalId);
id_type!(AlternativeId);

/// A person or agent that made a decision or owns a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityId(pub String);

impl EntityId {
    /// The agent's own identity.
    #[must_use]
    pub fn agent() -> Self {
        Self("agent".to_owned())
    }

    /// A user's identity.
    #[must_use]
    pub fn user(id: &UserId) -> Self {
        Self(format!("user:{id}"))
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unix milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// The current time.
    #[must_use]
    pub fn now() -> Self {
        Self(now_ms())
    }

    /// The raw milliseconds.
    #[must_use]
    pub fn millis(self) -> i64 {
        self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How the user supplied input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InputModality {
    /// Typed text.
    #[default]
    Text,
    /// Spoken audio.
    Voice,
    /// Structured data (tool call, paste, file).
    Structured,
}

/// How the agent responded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResponseModality {
    /// Text.
    #[default]
    Text,
    /// Synthesized speech.
    Voice,
    /// Mixed text and speech.
    Mixed,
}

/// State of a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DecisionStatus {
    /// Made and accepted.
    #[default]
    Decided,
    /// Under consideration.
    Tentative,
    /// Overturned by a later decision.
    Reversed,
    /// Replaced by a newer decision.
    Superseded,
}

/// State of a commitment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CommitmentStatus {
    /// Not started.
    #[default]
    Open,
    /// Being worked on.
    InProgress,
    /// Finished.
    Done,
    /// Waiting on something.
    Blocked,
    /// Dropped.
    Cancelled,
}

/// State of an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    /// Created.
    #[default]
    Created,
    /// Edited after creation.
    Modified,
    /// Merged.
    Merged,
    /// Deployed.
    Deployed,
    /// No longer current.
    Archived,
    /// Removed.
    Deleted,
}

/// The kind of artifact a turn produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    /// Pull request.
    PullRequest,
    /// Commit.
    Commit,
    /// Source or configuration file.
    File,
    /// Configuration change.
    Config,
    /// Diagram.
    Diagram,
    /// Test.
    Test,
    /// Document.
    Document,
    /// Script.
    Script,
    /// Database migration.
    Migration,
    /// Anything else.
    #[default]
    Other,
}

/// State of a goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Being pursued.
    #[default]
    Active,
    /// Deliberately paused.
    OnHold,
    /// Reached.
    Completed,
    /// Dropped.
    Abandoned,
}

/// Priority of an open question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuestionPriority {
    /// Blocks progress.
    High,
    /// Worth answering soon.
    #[default]
    Medium,
    /// Nice to know.
    Low,
}

/// A preference dimension the planner can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceDimension {
    /// Response length.
    Verbosity,
    /// How often the agent volunteers next steps.
    Initiative,
    /// Depth of technical detail.
    TechnicalDepth,
    /// Formal versus casual tone.
    Formality,
    /// Response speed versus deliberation.
    Pace,
}

impl PreferenceDimension {
    /// Every dimension.
    pub const ALL: [Self; 5] = [
        Self::Verbosity,
        Self::Initiative,
        Self::TechnicalDepth,
        Self::Formality,
        Self::Pace,
    ];

    /// The stable wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verbosity => "verbosity",
            Self::Initiative => "initiative",
            Self::TechnicalDepth => "technical_depth",
            Self::Formality => "formality",
            Self::Pace => "pace",
        }
    }
}

/// Where a learned preference is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PreferenceState {
    /// No evidence yet.
    #[default]
    ColdStart,
    /// Gathering observations.
    Observing,
    /// The posterior is tightening.
    Converging,
    /// Stable enough to use.
    Stable,
    /// A recent change was detected.
    DriftDetected,
}

/// A coarse expertise level, 0–100.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct ExpertiseLevel(pub u8);

impl ExpertiseLevel {
    /// Cap at 100.
    #[must_use]
    pub fn capped(value: u32) -> Self {
        Self(value.min(100) as u8)
    }

    /// Add and cap.
    #[must_use]
    pub fn raised(self, by: u32) -> Self {
        Self::capped(u32::from(self.0).saturating_add(by))
    }
}

/// Lifecycle of a project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    /// Being worked on.
    #[default]
    Active,
    /// Paused.
    OnHold,
    /// Finished.
    Completed,
    /// Kept for reference.
    Archived,
}

/// A model-produced embedding.
pub type Vector = Vec<f32>;
