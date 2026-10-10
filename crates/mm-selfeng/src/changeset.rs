//! The typed change set: what a candidate change *is*, before anything runs it.
//!
//! The phase's central design decision is that self-modification is a **candidate
//! judged by evidence**, not an edit. So the unit of change is not a diff: it is a
//! record that carries the reason it exists, the hypothesis it tests, every artifact
//! it touches — code, schema migrations, data migrations, memory transforms, policies,
//! prompts, tests, benchmarks — and the plan that undoes it.
//!
//! Three decisions are worth stating because they are not visible in the shape of the
//! code:
//!
//! * **A rollback plan is mandatory and is not a second diff.** It is a list of steps
//!   plus the `state_hash` the restore is checked against. Restoring a *diff* would
//!   make the restorer trust the same content it is undoing; restoring to a recorded
//!   hash is checkable. The bytes come from the snapshot Phase 10 already takes for a
//!   reversible action, so this crate does not grow a second snapshot store.
//! * **`payload_json` holds the whole record, and `rollback_json` repeats the plan.**
//!   That is deliberate denormalization: "does this candidate have a rollback" has to
//!   be answerable in SQL, because rule 5 of the promotion gate asks exactly that, and
//!   a plan that can only be read by parsing every payload is a plan nobody checks.
//! * **Typing is real.** `schema_migrations`, `data_migrations`, `memory_transforms`,
//!   `policies`, `prompts`, `tests` and `benchmarks` are separate fields with their own
//!   id types rather than one `Vec<String>` named `artifacts`. A migration and a prompt
//!   are reviewed by different people and roll back differently; a design that cannot
//!   tell them apart cannot route them.
//!
//! A gap is where a change set usually starts: a statement about what the system
//! cannot do, with the evidence that showed it. A gap with no evidence is an opinion,
//! so [`Gap::validate`] refuses one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_core::{Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{Result, SelfEngError};

macro_rules! string_id {
    ($name:ident, $what:literal) => {
        #[doc = concat!("The identifier of a ", $what, ".")]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// A new identifier.
            pub fn new(value: impl Into<String>) -> Self {
                $name(value.into())
            }

            /// The identifier as text.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// True when it is empty. An empty identifier names nothing, so a change
            /// set carrying one is refused.
            pub fn is_empty(&self) -> bool {
                self.0.trim().is_empty()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(MigrationId, "a schema or data migration");
string_id!(TransformId, "a memory transform");
string_id!(TestId, "a regression test");
string_id!(BenchmarkId, "a benchmark");

/// What a patch does to its path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOperation {
    /// The path did not exist and is created.
    Create,
    /// The path existed and is replaced.
    Modify,
    /// The path is removed.
    Delete,
}

impl PatchOperation {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            PatchOperation::Create => "create",
            PatchOperation::Modify => "modify",
            PatchOperation::Delete => "delete",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        [
            PatchOperation::Create,
            PatchOperation::Modify,
            PatchOperation::Delete,
        ]
        .into_iter()
        .find(|operation| operation.as_str() == text.trim().to_ascii_lowercase())
    }

    /// The rollback step that undoes one patch of this kind.
    fn rollback_step(self, path: &Path) -> String {
        match self {
            PatchOperation::Create => format!("delete {}", path.display()),
            PatchOperation::Modify => format!(
                "restore {} from the snapshot taken before the sandbox ran",
                path.display()
            ),
            PatchOperation::Delete => format!(
                "restore {} from the snapshot taken before the sandbox ran",
                path.display()
            ),
        }
    }
}

/// One artifact the change touches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Patch {
    /// The path, relative to the repository root.
    pub path: PathBuf,
    /// What happens to it.
    pub operation: PatchOperation,
    /// The content to write, for a create or a modify.
    pub content: String,
}

impl Patch {
    /// A patch that creates a file.
    pub fn create(path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Patch {
            path: path.into(),
            operation: PatchOperation::Create,
            content: content.into(),
        }
    }

    /// A patch that replaces a file.
    pub fn modify(path: impl Into<PathBuf>, content: impl Into<String>) -> Self {
        Patch {
            path: path.into(),
            operation: PatchOperation::Modify,
            content: content.into(),
        }
    }

    /// A patch that removes a file.
    pub fn delete(path: impl Into<PathBuf>) -> Self {
        Patch {
            path: path.into(),
            operation: PatchOperation::Delete,
            content: String::new(),
        }
    }

    /// The path as it is rendered in a log record or a rollback step.
    pub fn path_str(&self) -> String {
        self.path.display().to_string()
    }
}

/// One policy field a change moves.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDelta {
    /// The policy's identifier.
    pub policy: String,
    /// The field that moves.
    pub field: String,
    /// Its value before.
    pub from: String,
    /// Its value after.
    pub to: String,
}

/// One prompt a change edits. Prompts are policy too — they steer behaviour — so they
/// are typed separately from code, because a prompt change is reviewed by reading it
/// and rolls back by restoring text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptDelta {
    /// The prompt's path or identifier.
    pub target: String,
    /// What changes.
    pub change: String,
}

/// How to undo a change set, and the state hash the undo is checked against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackPlan {
    /// The steps, in the order they must run.
    pub steps: Vec<String>,
    /// The digest of the state the plan restores to.
    pub state_hash: String,
}

impl RollbackPlan {
    /// The plan that undoes these patches, in reverse order.
    ///
    /// Reverse order is not cosmetic: creating `a/` and then writing `a/b` undoes by
    /// removing `a/b` first, and a plan that ran its steps forward would delete a
    /// directory that is still full.
    pub fn for_patches(patches: &[Patch]) -> RollbackPlan {
        let mut steps: Vec<String> = patches
            .iter()
            .rev()
            .map(|patch| patch.operation.rollback_step(&patch.path))
            .collect();
        if steps.is_empty() {
            steps.push("nothing to undo: the change set carries no patches".to_string());
        }
        RollbackPlan {
            steps,
            state_hash: patches_state_hash(patches),
        }
    }
}

/// The digest of the patch set, which is what a rollback is checked against.
pub fn patches_state_hash(patches: &[Patch]) -> String {
    let mut parts: Vec<String> = patches
        .iter()
        .map(|patch| format!("{}:{}", patch.operation.as_str(), patch.path_str()))
        .collect();
    parts.sort();
    let mut fields: Vec<&str> = vec!["selfeng.rollback.v1"];
    for part in &parts {
        fields.push(part.as_str());
    }
    mm_core::hash_fields(&fields)
}

/// Where a change set is in the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeSetStatus {
    /// Assembled, not yet run anywhere.
    Draft,
    /// A sandbox exists for it.
    Sandboxed,
    /// It built inside the sandbox.
    Built,
    /// Its tests ran inside the sandbox.
    Tested,
    /// Its benchmark and shadow runs completed.
    Benchmarked,
    /// Its shadow comparison completed.
    Shadowed,
    /// The gate promoted it.
    Promoted,
    /// The gate rejected it.
    Rejected,
}

impl ChangeSetStatus {
    /// Every status, in pipeline order.
    pub const ALL: [ChangeSetStatus; 8] = [
        ChangeSetStatus::Draft,
        ChangeSetStatus::Sandboxed,
        ChangeSetStatus::Built,
        ChangeSetStatus::Tested,
        ChangeSetStatus::Benchmarked,
        ChangeSetStatus::Shadowed,
        ChangeSetStatus::Promoted,
        ChangeSetStatus::Rejected,
    ];

    /// The stable wire name, which is also the `change_sets.status` value.
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeSetStatus::Draft => "draft",
            ChangeSetStatus::Sandboxed => "sandboxed",
            ChangeSetStatus::Built => "built",
            ChangeSetStatus::Tested => "tested",
            ChangeSetStatus::Benchmarked => "benchmarked",
            ChangeSetStatus::Shadowed => "shadowed",
            ChangeSetStatus::Promoted => "promoted",
            ChangeSetStatus::Rejected => "rejected",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let lower = text.trim().to_ascii_lowercase();
        ChangeSetStatus::ALL
            .into_iter()
            .find(|status| status.as_str() == lower)
    }

    /// True when nothing further happens to this change set.
    pub fn is_terminal(self) -> bool {
        matches!(self, ChangeSetStatus::Promoted | ChangeSetStatus::Rejected)
    }
}

/// A typed, falsifiable change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeSet {
    /// Its ULID.
    pub id: Ulid,
    /// Why the change exists.
    pub reason: String,
    /// The statement it tests, which the gate judges evidence for.
    pub hypothesis: String,
    /// Code artifacts.
    pub code: Vec<Patch>,
    /// Schema migrations it needs.
    pub schema_migrations: Vec<MigrationId>,
    /// Data migrations it needs.
    pub data_migrations: Vec<MigrationId>,
    /// Memory rewrites it needs.
    pub memory_transforms: Vec<TransformId>,
    /// Policy movements.
    pub policies: Vec<PolicyDelta>,
    /// Prompt edits.
    pub prompts: Vec<PromptDelta>,
    /// The tests that must pass.
    pub tests: Vec<TestId>,
    /// The benchmarks it is judged by.
    pub benchmarks: Vec<BenchmarkId>,
    /// How to undo it.
    pub rollback: RollbackPlan,
}

impl ChangeSet {
    /// The number of artifacts of every kind.
    pub fn artifact_count(&self) -> usize {
        self.code.len()
            + self.schema_migrations.len()
            + self.data_migrations.len()
            + self.memory_transforms.len()
            + self.policies.len()
            + self.prompts.len()
            + self.tests.len()
            + self.benchmarks.len()
    }

    /// Refuse a change that is not a usable candidate.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_nil() {
            return Err(SelfEngError::validation("id", "is the nil ULID"));
        }
        if self.reason.trim().is_empty() {
            return Err(SelfEngError::validation("reason", "must not be empty"));
        }
        if self.hypothesis.trim().is_empty() {
            return Err(SelfEngError::validation("hypothesis", "must not be empty"));
        }
        if self.artifact_count() == 0 {
            return Err(SelfEngError::validation(
                "artifacts",
                "a change set with nothing in it is not a candidate",
            ));
        }
        if self.rollback.steps.is_empty() {
            return Err(SelfEngError::validation(
                "rollback.steps",
                "a change with no way back is not a candidate",
            ));
        }
        if self.rollback.state_hash.trim().is_empty() {
            return Err(SelfEngError::validation(
                "rollback.state_hash",
                "the restore must be checkable against a hash",
            ));
        }
        for patch in &self.code {
            validate_relative_path(&patch.path)?;
        }
        for delta in &self.policies {
            if delta.policy.trim().is_empty() || delta.field.trim().is_empty() {
                return Err(SelfEngError::validation(
                    "policies",
                    "a policy delta must name the policy and the field it moves",
                ));
            }
        }
        Ok(())
    }

    /// The canonical JSON the payload column stores.
    pub fn payload_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(SelfEngError::from)
    }

    /// The rollback plan's own JSON, for the column that answers "has it one".
    pub fn rollback_json(&self) -> Result<String> {
        serde_json::to_string(&self.rollback).map_err(SelfEngError::from)
    }

    /// The paths the change writes, in patch order.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.code.iter().map(|patch| patch.path.clone()).collect()
    }
}

/// Refuse a path that is not a repository-relative file path.
///
/// Absolute paths and `..` are refused here as well as in the sandbox: this is the
/// first gate a bad path meets, and a change set that cannot pass it should never
/// reach a filesystem write. The sandbox checks again because a path that is valid in
/// one checkout can still resolve outside the worktree, and the second check is the
/// one that has a real root to compare against.
pub fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(SelfEngError::validation("patch.path", "must not be empty"));
    }
    if path.is_absolute() {
        return Err(SelfEngError::validation(
            "patch.path",
            format!(
                "{} is absolute; a patch path is repository-relative",
                path.display()
            ),
        ));
    }
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err(SelfEngError::validation(
                    "patch.path",
                    format!("{} contains `..`", path.display()),
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(SelfEngError::validation(
                    "patch.path",
                    format!("{} is not repository-relative", path.display()),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// A statement about what the system cannot do, with the evidence that showed it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    /// Its ULID.
    pub gap_id: Ulid,
    /// The kind of gap: `capability`, `data`, `policy` or `prompt`.
    pub kind: String,
    /// What is missing, in words.
    pub statement: String,
    /// The records that showed it.
    pub evidence: Vec<Ulid>,
    /// The module IRI the change targets.
    pub target_uri: String,
    /// The path the change targets.
    pub target_path: PathBuf,
}

impl Gap {
    /// Refuse a gap that states nothing or stands on nothing.
    pub fn validate(&self) -> Result<()> {
        if self.statement.trim().is_empty() {
            return Err(SelfEngError::validation("statement", "must not be empty"));
        }
        if self.evidence.is_empty() {
            return Err(SelfEngError::validation(
                "evidence",
                "a gap with no evidence is an opinion",
            ));
        }
        if self.target_uri.trim().is_empty() {
            return Err(SelfEngError::validation("target_uri", "must not be empty"));
        }
        validate_relative_path(&self.target_path)?;
        Ok(())
    }
}

/// Read a gap fixture.
///
/// Unknown keys are ignored on purpose: the committed fixtures carry `_comment` and
/// `expected` keys for the gate's benefit, and a reader that refused them would make
/// the fixtures unable to explain themselves.
pub fn load_gap(path: &Path) -> Result<Gap> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| SelfEngError::Changeset(format!("cannot read {}: {e}", path.display())))?;
    let gap: Gap = serde_json::from_str(&text)
        .map_err(|e| SelfEngError::Changeset(format!("{} is not a gap: {e}", path.display())))?;
    gap.validate()?;
    Ok(gap)
}

/// Assemble a change set from a gap and the artifacts that close it.
///
/// The hypothesis is derived rather than taken from the caller: a change set that can be
/// assembled with an empty hypothesis would be one the gate's rule 5 then refuses, and
/// the derivation makes the statement it tests a function of the evidence that produced
/// it — which is what makes it falsifiable rather than decorative.
pub fn from_gap(
    gap: &Gap,
    patches: Vec<Patch>,
    tests: Vec<TestId>,
    benchmarks: Vec<BenchmarkId>,
    ids: &UlidFactory,
) -> Result<ChangeSet> {
    gap.validate()?;
    let rollback = RollbackPlan::for_patches(&patches);
    let change_set = ChangeSet {
        id: ids.next(),
        reason: gap.statement.clone(),
        hypothesis: format!(
            "closing this {} gap (evidence {}) changes what the system can do, and the \
             frozen benchmark will read the change",
            gap.kind,
            gap.evidence
                .iter()
                .map(mm_core::ulid_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        code: patches,
        schema_migrations: Vec::new(),
        data_migrations: Vec::new(),
        memory_transforms: Vec::new(),
        policies: Vec::new(),
        prompts: Vec::new(),
        tests,
        benchmarks,
        rollback,
    };
    change_set.validate()?;
    Ok(change_set)
}

/// The `change_sets` table.
pub struct ChangeSetStore {
    store: mm_store_sqlite::SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl ChangeSetStore {
    /// A store over the kernel's tabular store.
    pub fn new(
        store: mm_store_sqlite::SqliteStore,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Self {
        ChangeSetStore { store, logger, ids }
    }

    /// The identifier factory the store shares with the caller.
    pub fn ids(&self) -> &UlidFactory {
        &self.ids
    }

    /// Record a new change set, as a draft.
    pub async fn create(&self, change_set: &ChangeSet) -> Result<()> {
        change_set.validate()?;
        let now = Timestamp::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO change_sets \
             (id, reason, hypothesis, payload_json, rollback_json, status, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(mm_core::ulid_string(&change_set.id))
        .bind(&change_set.reason)
        .bind(&change_set.hypothesis)
        .bind(change_set.payload_json()?)
        .bind(change_set.rollback_json()?)
        .bind(ChangeSetStatus::Draft.as_str())
        .bind(&now)
        .execute(self.store.pool())
        .await
        .map_err(|e| SelfEngError::Store(format!("cannot record the change set: {e}")))?;
        self.logger
            .audit(
                Level::Info,
                codes::CHANGESET_CREATE,
                crate::TARGET,
                Some(change_set.id),
                json!({
                    "change_set_id": mm_core::ulid_string(&change_set.id),
                    "status": ChangeSetStatus::Draft.as_str(),
                    "reason": change_set.reason,
                    "hypothesis": change_set.hypothesis,
                    "artifacts": change_set.artifact_count(),
                }),
            )
            .await?;
        Ok(())
    }

    /// Move a change set to another status.
    pub async fn set_status(&self, id: Ulid, status: ChangeSetStatus) -> Result<()> {
        let affected = sqlx::query("UPDATE change_sets SET status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(mm_core::ulid_string(&id))
            .execute(self.store.pool())
            .await
            .map_err(|e| SelfEngError::Store(format!("cannot stage the change set: {e}")))?;
        if affected.rows_affected() == 0 {
            return Err(SelfEngError::Changeset(format!(
                "no change set {} is recorded",
                mm_core::ulid_string(&id)
            )));
        }
        self.logger
            .audit(
                Level::Info,
                codes::CHANGESET_STAGE,
                crate::TARGET,
                Some(id),
                json!({
                    "change_set_id": mm_core::ulid_string(&id),
                    "status": status.as_str(),
                }),
            )
            .await?;
        Ok(())
    }

    /// Read a change set back.
    pub async fn get(&self, id: Ulid) -> Result<Option<ChangeSet>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT payload_json FROM change_sets WHERE id = ?")
                .bind(mm_core::ulid_string(&id))
                .fetch_optional(self.store.pool())
                .await
                .map_err(|e| SelfEngError::Store(format!("cannot read the change set: {e}")))?;
        match row {
            None => Ok(None),
            Some((payload,)) => {
                let change_set: ChangeSet = serde_json::from_str(&payload).map_err(|e| {
                    SelfEngError::Store(format!(
                        "stored change set {} is not a ChangeSet: {e}",
                        mm_core::ulid_string(&id)
                    ))
                })?;
                Ok(Some(change_set))
            }
        }
    }

    /// Read its status.
    pub async fn status(&self, id: Ulid) -> Result<Option<ChangeSetStatus>> {
        let row: Option<(String,)> = sqlx::query_as("SELECT status FROM change_sets WHERE id = ?")
            .bind(mm_core::ulid_string(&id))
            .fetch_optional(self.store.pool())
            .await
            .map_err(|e| SelfEngError::Store(format!("cannot read the status: {e}")))?;
        match row {
            None => Ok(None),
            Some((status,)) => ChangeSetStatus::parse(&status).map(Some).ok_or_else(|| {
                SelfEngError::Store(format!("unknown change-set status {status:?}"))
            }),
        }
    }

    /// Every recorded change set, oldest first.
    pub async fn list(&self) -> Result<Vec<(ChangeSet, ChangeSetStatus)>> {
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT payload_json, status FROM change_sets ORDER BY created_at, id")
                .fetch_all(self.store.pool())
                .await
                .map_err(|e| SelfEngError::Store(format!("cannot list the change sets: {e}")))?;
        rows.into_iter()
            .map(|(payload, status)| {
                let change_set: ChangeSet = serde_json::from_str(&payload).map_err(|e| {
                    SelfEngError::Store(format!("a stored change set is not a ChangeSet: {e}"))
                })?;
                let status = ChangeSetStatus::parse(&status)
                    .ok_or_else(|| SelfEngError::Store(format!("unknown status {status:?}")))?;
                Ok((change_set, status))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;
    use mm_store_sqlite::SqliteStore;

    fn ids() -> UlidFactory {
        UlidFactory::new()
    }

    fn gap() -> Gap {
        Gap {
            gap_id: Ulid::from_parts(1_700_000_000_000, 1),
            kind: "capability".to_string(),
            statement: "no module summarizes a drifting calibration bin".to_string(),
            evidence: vec![Ulid::from_parts(1_700_000_000_000, 2)],
            target_uri: "https://metamind.dev/code/module/cognition/calibration".to_string(),
            target_path: PathBuf::from("modules/cognition/calibration"),
        }
    }

    fn change_set() -> ChangeSet {
        from_gap(
            &gap(),
            vec![Patch::create(
                "modules/cognition/calibration/plugin.toml",
                "name = \"calibration\"\n",
            )],
            vec![TestId::new("bench/regression/seeded_bug_01")],
            vec![BenchmarkId::new("promotion_bench")],
            &ids(),
        )
        .unwrap()
    }

    #[test]
    fn a_gap_with_no_evidence_is_refused() {
        let mut bad = gap();
        bad.evidence.clear();
        assert_eq!(bad.validate().unwrap_err().code(), "validation");
        let mut silent = gap();
        silent.statement = "   ".to_string();
        assert!(silent.validate().is_err());
    }

    #[test]
    fn a_change_set_missing_a_rollback_is_refused() {
        let mut bad = change_set();
        bad.rollback.steps.clear();
        let error = bad.validate().unwrap_err();
        assert!(error.to_string().contains("way back"), "{error}");

        let mut unhashed = change_set();
        unhashed.rollback.state_hash = String::new();
        assert!(unhashed.validate().is_err());
    }

    #[test]
    fn a_change_set_with_no_artifacts_is_refused() {
        let mut empty = change_set();
        empty.code.clear();
        empty.tests.clear();
        empty.benchmarks.clear();
        let error = empty.validate().unwrap_err();
        assert!(error.to_string().contains("nothing in it"), "{error}");
    }

    #[test]
    fn an_escaping_patch_path_is_refused_before_anything_runs() {
        for bad in ["/etc/passwd", "../outside.txt", "modules/../../outside.txt"] {
            let mut change = change_set();
            change.code = vec![Patch::modify(bad, "x")];
            assert!(
                change.validate().is_err(),
                "{bad} must be refused at validation"
            );
            assert!(validate_relative_path(Path::new(bad)).is_err());
        }
        assert!(
            validate_relative_path(Path::new("modules/cognition/calibration/src/lib.rs")).is_ok()
        );
    }

    #[test]
    fn the_json_round_trip_is_stable() {
        let change = change_set();
        let payload = change.payload_json().unwrap();
        let back: ChangeSet = serde_json::from_str(&payload).unwrap();
        assert_eq!(back, change);
        assert_eq!(back.payload_json().unwrap(), payload, "stable rendering");
        let rollback: RollbackPlan =
            serde_json::from_str(&change.rollback_json().unwrap()).unwrap();
        assert_eq!(rollback, change.rollback);
    }

    #[test]
    fn the_rollback_plan_reverses_the_patch_order() {
        let patches = vec![
            Patch::create("a/one.txt", "1"),
            Patch::create("a/two.txt", "2"),
        ];
        let plan = RollbackPlan::for_patches(&patches);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0], "delete a/two.txt");
        assert_eq!(plan.steps[1], "delete a/one.txt");
        assert_eq!(plan.state_hash, patches_state_hash(&patches));
        // Order-independent hash: the same set of patches hashes the same either way.
        let reversed = vec![patches[1].clone(), patches[0].clone()];
        assert_eq!(patches_state_hash(&patches), patches_state_hash(&reversed));
    }

    #[test]
    fn a_plan_with_no_patches_says_so_rather_than_being_empty() {
        let plan = RollbackPlan::for_patches(&[]);
        assert_eq!(plan.steps.len(), 1);
        assert!(plan.steps[0].contains("nothing to undo"));
    }

    #[test]
    fn statuses_round_trip_and_only_two_are_terminal() {
        for status in ChangeSetStatus::ALL {
            assert_eq!(ChangeSetStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(ChangeSetStatus::parse("Nonesuch"), None);
        assert_eq!(
            ChangeSetStatus::ALL
                .iter()
                .filter(|status| status.is_terminal())
                .count(),
            2
        );
    }

    #[test]
    fn patch_operations_round_trip() {
        for operation in [
            PatchOperation::Create,
            PatchOperation::Modify,
            PatchOperation::Delete,
        ] {
            assert_eq!(PatchOperation::parse(operation.as_str()), Some(operation));
        }
        assert_eq!(PatchOperation::parse("rename"), None);
        assert_eq!(Patch::delete("x").content, "");
    }

    #[tokio::test]
    async fn the_store_records_reads_back_and_stages_a_change_set() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open(&dir.path().join("mm.db")).await.unwrap();
        store.migrate().await.unwrap();
        let cfg = Config::for_data_dir(dir.path());
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let table = ChangeSetStore::new(store.clone(), logger, ids);

        let change = change_set();
        table.create(&change).await.unwrap();
        assert_eq!(
            table.status(change.id).await.unwrap(),
            Some(ChangeSetStatus::Draft)
        );
        let back = table.get(change.id).await.unwrap().unwrap();
        assert_eq!(back, change, "the record survives the round trip");
        table
            .set_status(change.id, ChangeSetStatus::Sandboxed)
            .await
            .unwrap();
        assert_eq!(
            table.status(change.id).await.unwrap(),
            Some(ChangeSetStatus::Sandboxed)
        );
        let listed = table.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].1, ChangeSetStatus::Sandboxed);
        assert!(table
            .get(Ulid::from_parts(1_700_000_000_000, 999))
            .await
            .unwrap()
            .is_none());
        // An unknown id is a refusal, not a silent no-op.
        assert!(table
            .set_status(
                Ulid::from_parts(1_700_000_000_000, 998),
                ChangeSetStatus::Built
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn the_gap_fixture_in_the_repository_loads() {
        let path = mm_core::Config::repo_root()
            .join("bench")
            .join("gaps")
            .join("gap_01.json");
        let gap = load_gap(&path).unwrap();
        assert_eq!(gap.kind, "capability");
        assert_eq!(gap.evidence.len(), 1);
        assert!(gap
            .target_path
            .to_string_lossy()
            .contains("modules/cognition/calibration"));
        let change = from_gap(
            &gap,
            vec![Patch::create("modules/cognition/calibration/case.txt", "x")],
            vec![TestId::new("bench/regression/seeded_bug_01")],
            vec![BenchmarkId::new("promotion_bench")],
            &ids(),
        )
        .unwrap();
        change.validate().unwrap();
        assert!(change.reason.contains("calibration"));
    }
}
