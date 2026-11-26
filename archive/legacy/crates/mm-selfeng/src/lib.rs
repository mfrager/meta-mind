//! `mm-selfeng` — self-engineering: the machinery that turns a diagnosis into a
//! judged, reversible change.
//!
//! Design §13 / Phase 11 asks for a loop that (a) proposes a *typed* change instead
//! of a code edit, (b) exercises it where it cannot hurt anything, (c) judges it
//! against a frozen harness, and (d) either promotes it into the record or rejects it
//! with a written reason. This crate is the middle of that loop: everything between
//! "there is a gap" and "the lineage has a new version".
//!
//! The modules, in the order data flows through them:
//!
//! * [`changeset`] — the typed proposal, the gap it came from, and its persistence.
//! * [`sandbox`] — a git worktree under `data/sandbox/`, the patches applied inside
//!   it, the build and test runs, and the filesystem audit that proves nothing
//!   outside it moved.
//! * [`benchmark`], [`shadow`] — the two measurements: against the frozen baseline,
//!   and against the system's own previous behaviour.
//! * [`promotion`] — the deterministic gate: six ordered rules, the first failing one
//!   supplying the reason, and the decision written down either way.
//! * [`rollback`] — undoing a change set by its own rollback plan.
//! * [`journal`] — the append-only lineage, its deterministic `self_version`, and the
//!   `/selfeng` mirror.
//! * [`budget`] — the four budgets, enforced by arithmetic rather than clamped.
//!
//! Five invariants hold across the crate, and each is a test somewhere:
//!
//! 1. **A change set is typed and falsifiable.** It carries a reason, a hypothesis,
//!    its artifacts, and a rollback plan; a change with no way back is refused at
//!    validation, not at promotion.
//! 2. **Production is never written by the pipeline.** Every write goes through
//!    [`sandbox::SandboxDir`], which refuses a path outside the sandbox root, and
//!    [`sandbox::audit_writes`] is the check that says so after the fact.
//! 3. **The gate decides, never a model.** [`promotion::DeterministicGate`] is pure
//!    arithmetic and policy over the evidence; the reason is reproducible from the
//!    bundle.
//! 4. **A rejection carries its reason.** Both decisions are recorded, and the reason
//!    is the first rule that failed, named.
//! 5. **Budget is enforced, never clamped.** A debit that would cross a limit is
//!    refused whole and leaves the ledger byte-identical.
#![forbid(unsafe_code)]

pub mod benchmark;
pub mod budget;
pub mod changeset;
pub mod error;
pub mod journal;
pub mod promotion;
pub mod rollback;
pub mod sandbox;
pub mod shadow;

pub use benchmark::{load_specs, run_benchmark, BenchResult, BenchSpec};
pub use budget::{BudgetKind, BudgetLedger, BudgetRow, BUDGET_KINDS, DEFAULT_PERIOD};
pub use changeset::{
    from_gap, load_gap, BenchmarkId, ChangeSet, ChangeSetStatus, ChangeSetStore, Gap, MigrationId,
    Patch, PatchOperation, PolicyDelta, PromptDelta, RollbackPlan, TestId, TransformId,
};
pub use error::{Result, SelfEngError};
pub use journal::{EvolutionEvent, EvolutionJournal, SELFENG_GRAPH};
pub use promotion::{
    DeterministicGate, EvidenceBundle, PromotionDecision, PromotionGate, PromotionOutcome,
    PromotionPolicy, DEFAULT_IMMUTABLE_PATHS,
};
pub use rollback::{apply as apply_rollback, RollbackOutcome};
pub use sandbox::{
    audit_writes, is_inside, BuildResult, Sandbox, SandboxDir, TestResult, WorktreeSandbox,
    WriteAudit,
};
pub use shadow::{compare, shadow, ShadowReport};

/// The target every record from this crate carries.
pub const TARGET: &str = "mm.selfeng";
