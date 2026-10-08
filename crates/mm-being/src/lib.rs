//! `mm-being` — the persistent being substrate.
//!
//! This crate holds the small state that survives model changes, context-window
//! boundaries, and conversations: identity and its invariants, size-limited core
//! blocks, a personality constitution, appraisal-driven affect, a probabilistic
//! user model, event-sourced relationships, a BDI goal/commitment lifecycle, and
//! deterministic budgets.
//!
//! Three rules shape the whole crate (plan §2):
//!
//! 1. **Store something only when losing it would materially change the being's
//!    future behavior.** Everything expressive — tone, elaboration, the wording of a
//!    plan — is left to a model, not enumerated here.
//! 2. **One guarded mutation path.** Every write goes through
//!    [`BeingFacade::apply`], which runs the deterministic [`IdentityGuard`] before
//!    it reaches a store. The adaptive layers can propose; they cannot revise the
//!    kernel.
//! 3. **Affect biases behavior only.** An impulse can move an affect axis and
//!    nothing else: its two flags are private, unsettable, and always false.
#![forbid(unsafe_code)]

pub mod affect;
pub mod blocks;
pub mod error;
pub mod goals;
pub mod identity;
pub mod invariants;
pub mod motivation;
pub mod personality;
pub mod rdf;
pub mod relationships;
pub mod resources;
pub mod user_model;

pub use affect::{appraise, decay, Affect, AffectImpulse, Agency, AppraisalEvent, Emotion};
pub use blocks::{default_limit, BlockKind, CoreBlock, DEFAULT_LIMIT_CHARS};
pub use error::{
    BeingError, BudgetExceeded, InvariantCode, InvariantViolation, PromotionDenied, Result,
    TransitionDenied, CORE_INVARIANTS,
};
pub use goals::{
    transition_commitment, transition_goal, Commitment, CommitmentStatus, Goal, GoalOrigin,
    GoalStatus,
};
pub use identity::{Identity, SelfVersion};
pub use invariants::{deny_terminal_transition, BeingCtx, BeingOp, CoreGuard, IdentityGuard};
pub use motivation::{intrinsic_defaults, Drive, Motivation, INTRINSIC_DRIVES};
pub use personality::{BehaviorParams, ConstraintKind, ContextVector, Personality};
pub use rdf::{BeingSnapshot, BeingTriple, FromRdf, ToRdf, BEING_GRAPH};
pub use relationships::{RelKind, RelationshipEvent, RelationshipState};
pub use resources::{debit, Account, BudgetPolicy, DebitReceipt, ResourceKind, ResourceState};
pub use user_model::{Belief, EpistemicStatus, EvidenceId, PropositionId, UserModel};

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use mm_core::{Param, Params, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, LogRecord, Logger};
use mm_store_graph::GraphHandle;
use mm_store_sqlite::SqliteStore;
use serde_json::{json, Value};
use sqlx::Executor;

use identity::InvariantId;

/// The target every record from this crate carries.
const TARGET: &str = "mm.being";

/// The self-description a fresh identity starts from.
pub const DEFAULT_SELF_DESCRIPTION: &str =
    "A Metamind being: continuous, honest about what it knows, \
     and accountable for what it does.";

/// The receipt for one committed operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpReceipt {
    /// The operation's ULID; also the audit `trace_id`.
    pub op_ulid: Ulid,
    /// The op's wire name.
    pub op: &'static str,
}

/// What `being verify` found.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifyReport {
    /// The identity being verified.
    pub identity_ulid: String,
    /// Its current self-description version.
    pub version: String,
    /// How many invariants the identity binds.
    pub invariants_enforced: usize,
    /// How many forbidden ops the corpus holds.
    pub corpus_total: usize,
    /// How many were refused for the declared invariant.
    pub corpus_denied: usize,
    /// The terminal-row triggers the schema provides.
    pub terminal_triggers: i64,
    /// True when `sum(ledger.delta) == account.balance` for every account.
    pub budget_reconciled: bool,
    /// Beliefs marked `OBSERVED` with no evidence record.
    pub unbacked_observations: i64,
    /// Affect impulses whose flags were not false. Must be zero.
    pub affect_flag_violations: i64,
    /// Human-readable failures, empty when everything held.
    pub failures: Vec<String>,
}

impl VerifyReport {
    /// True when every check held.
    pub fn ok(&self) -> bool {
        self.failures.is_empty()
            && self.corpus_total > 0
            && self.corpus_denied == self.corpus_total
            && self.terminal_triggers == 2
            && self.budget_reconciled
            && self.unbacked_observations == 0
            && self.affect_flag_violations == 0
            && self.invariants_enforced == CORE_INVARIANTS.len()
    }
}

/// The one guarded path to persistent state.
pub struct BeingFacade {
    store: SqliteStore,
    graph: GraphHandle,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    identity: Identity,
    blocks: BTreeMap<BlockKind, CoreBlock>,
    personality: Personality,
    affect: Affect,
    motivations: BTreeMap<Drive, f32>,
    users: BTreeMap<Ulid, UserModel>,
    relationships: BTreeMap<Ulid, RelationshipState>,
    goals: BTreeMap<Ulid, Goal>,
    commitments: BTreeMap<Ulid, Commitment>,
    resources: ResourceState,
    guard: CoreGuard,
}

impl std::fmt::Debug for BeingFacade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BeingFacade")
            .field("identity", &mm_core::ulid_string(&self.identity.id))
            .field("goals", &self.goals.len())
            .field("commitments", &self.commitments.len())
            .finish_non_exhaustive()
    }
}

impl BeingFacade {
    /// Open the being over a store and a graph handle, creating the identity the
    /// first time.
    ///
    /// Creating the identity here rather than in a separate step is deliberate: a
    /// kernel that can run without an identity is a kernel whose state has no
    /// owner, and the plan's rule is that the persistent self exists before any
    /// episode does.
    pub async fn open(
        store: &SqliteStore,
        graph: &GraphHandle,
        logger: Arc<Logger>,
        ids: Arc<UlidFactory>,
    ) -> Result<Self> {
        let identity = match load_identity(store).await? {
            Some(identity) => identity,
            None => {
                let identity =
                    Identity::new(ids.next(), Timestamp::now(), DEFAULT_SELF_DESCRIPTION);
                persist_identity(store, &identity).await?;
                logger
                    .audit(
                        Level::Info,
                        codes::BEING_IDENTITY_INIT,
                        TARGET,
                        Some(identity.id),
                        json!({
                            "identity_ulid": mm_core::ulid_string(&identity.id),
                            "schema_version": 1,
                            "invariant_codes": identity.invariants.clone(),
                        }),
                    )
                    .await?;
                identity
            }
        };
        ensure_invariants(store, &identity, &ids).await?;

        let mut facade = BeingFacade {
            store: store.clone(),
            graph: graph.clone(),
            logger,
            ids,
            identity,
            blocks: BTreeMap::new(),
            personality: Personality::baseline(),
            affect: Affect::neutral(),
            motivations: intrinsic_defaults()
                .into_iter()
                .map(|m| (m.drive, m.strength))
                .collect(),
            users: BTreeMap::new(),
            relationships: BTreeMap::new(),
            goals: BTreeMap::new(),
            commitments: BTreeMap::new(),
            resources: ResourceState::new(),
            guard: CoreGuard::new(),
        };
        facade.load().await?;
        facade.seed_defaults().await?;
        facade.mirror().await?;
        Ok(facade)
    }

    /// The immutable core.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The personality constitution.
    pub fn personality(&self) -> &Personality {
        &self.personality
    }

    /// The affect state.
    pub fn affect(&self) -> &Affect {
        &self.affect
    }

    /// Intrinsic drive strengths.
    pub fn motivations(&self) -> &BTreeMap<Drive, f32> {
        &self.motivations
    }

    /// Core blocks.
    pub fn blocks(&self) -> &BTreeMap<BlockKind, CoreBlock> {
        &self.blocks
    }

    /// Goals, keyed by id.
    pub fn goals(&self) -> &BTreeMap<Ulid, Goal> {
        &self.goals
    }

    /// Commitments, keyed by id.
    pub fn commitments(&self) -> &BTreeMap<Ulid, Commitment> {
        &self.commitments
    }

    /// The resource accounts.
    pub fn resources(&self) -> &ResourceState {
        &self.resources
    }

    /// A user's model, if the being has one.
    pub fn user(&self, user: Ulid) -> Option<&UserModel> {
        self.users.get(&user)
    }

    /// A relationship projection, if the being has one.
    pub fn relationship(&self, user: Ulid) -> Option<&RelationshipState> {
        self.relationships.get(&user)
    }

    /// The store, for a caller that needs to query the projection directly.
    pub fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// The whole persistent self, as the `/being` mirror sees it.
    pub fn snapshot(&self) -> BeingSnapshot {
        BeingSnapshot {
            identity: Some(self.identity.clone()),
            blocks: self.blocks.values().cloned().collect(),
            personality: Some(self.personality.clone()),
            affect: Some(self.affect),
            motivations: self
                .motivations
                .iter()
                .map(|(drive, strength)| Motivation::new(*drive, *strength))
                .collect(),
            beliefs: self
                .users
                .iter()
                .flat_map(|(user, model)| {
                    model.beliefs.values().map(|belief| (*user, belief.clone()))
                })
                .collect(),
            relationships: self
                .relationships
                .iter()
                .map(|(user, state)| (*user, state.clone()))
                .collect(),
            goals: self.goals.values().cloned().collect(),
            commitments: self.commitments.values().cloned().collect(),
            resources: rdf::accounts(&self.resources),
        }
    }

    // ------------------------------------------------------------ the one path ---

    /// Propose a mutation. The guard runs first; nothing is written if it refuses.
    pub async fn apply(&mut self, op: BeingOp) -> Result<OpReceipt> {
        let ctx = BeingCtx {
            identity_id: Some(self.identity.id),
            identity_initialized: true,
        };
        if let Err(violation) = self.guard.check(&op, &ctx) {
            self.audit(
                Level::Error,
                codes::BEING_INVARIANT_VIOLATION,
                self.ids.next(),
                json!({
                    "op": violation.op,
                    "invariant_code": violation.code.as_str(),
                    "reason": violation.reason,
                }),
            )
            .await?;
            return Err(BeingError::Invariant(violation));
        }
        self.emit(
            Level::Debug,
            codes::BEING_INVARIANT_CHECK,
            json!({"op": op.name(), "result": "ok"}),
        )
        .await?;

        let op_ulid = self.ids.next();
        let op_name: &'static str = op.name();
        match op {
            BeingOp::InitIdentity { .. } => {
                // Unreachable through the guard once an identity exists; the
                // `open` path is what actually creates one.
                return Err(BeingError::Invariant(InvariantViolation {
                    code: InvariantCode::NoHistoryRewrite,
                    op: "init_identity".into(),
                    reason: "the identity already exists".into(),
                }));
            }
            BeingOp::SetBlock {
                kind,
                label,
                content,
                limit_chars,
            } => {
                self.set_block(kind, label, content, limit_chars, op_ulid)
                    .await?;
            }
            BeingOp::SetBelief {
                user_id,
                proposition,
                status,
                confidence,
                evidence,
            } => {
                self.set_belief(user_id, proposition, status, confidence, evidence, op_ulid)
                    .await?;
            }
            BeingOp::PromoteBelief {
                user_id,
                proposition,
                to,
                evidence,
            } => {
                self.promote_belief(user_id, proposition, to, evidence, op_ulid)
                    .await?;
            }
            BeingOp::SetPersonality { key, value } => {
                self.set_personality(key, value, op_ulid).await?;
            }
            BeingOp::Appraise { event } => {
                self.appraise(event, op_ulid).await?;
            }
            BeingOp::SetMotivation { drive, strength } => {
                self.set_motivation(drive, strength, op_ulid).await?;
            }
            BeingOp::AddGoal {
                description,
                priority,
                origin,
            } => {
                self.add_goal(description, priority, origin, op_ulid)
                    .await?;
            }
            BeingOp::TransitionGoal { id, to, reason, .. } => {
                self.transition_goal(id, to, reason, op_ulid).await?;
            }
            BeingOp::AddCommitment {
                goal_id,
                made_to,
                description,
            } => {
                self.add_commitment(goal_id, made_to, description, op_ulid)
                    .await?;
            }
            BeingOp::TransitionCommitment { id, to, reason, .. } => {
                self.transition_commitment(id, to, reason, op_ulid).await?;
            }
            BeingOp::DebitBudget {
                kind,
                amount,
                purpose,
            } => {
                self.debit_budget(kind, amount, purpose, op_ulid).await?;
            }
            BeingOp::RelationshipEvent { user_id, kind } => {
                self.relationship_event(user_id, kind, op_ulid).await?;
            }
            BeingOp::FabricateAutobiography { .. }
            | BeingOp::ClaimAction { .. }
            | BeingOp::RewriteHistory { .. } => {
                // The guard refuses these; reaching here would mean the guard was
                // bypassed, which is a defect rather than a caller error.
                return Err(BeingError::Config(format!(
                    "`{op_name}` is forbidden and must be refused by the guard"
                )));
            }
        }

        self.mirror().await?;
        Ok(OpReceipt {
            op_ulid,
            op: op_name,
        })
    }

    /// Rewrite `/being` from the current state.
    pub async fn mirror(&self) -> Result<usize> {
        let snapshot = self.snapshot();
        rdf::require_identity(&snapshot)?;
        let count = rdf::mirror(&self.graph, &snapshot).await?;
        self.emit(
            Level::Debug,
            codes::BEING_RDF_MIRROR,
            json!({"graph": rdf::BEING_GRAPH, "ulid": mm_core::ulid_string(&self.identity.id), "triple_count": count}),
        )
        .await?;
        Ok(count)
    }

    /// Run the invariant suite and the projection integrity checks.
    pub async fn verify(&self, corpus: &Path) -> Result<VerifyReport> {
        let mut failures = Vec::new();
        let ctx = BeingCtx {
            identity_id: Some(self.identity.id),
            identity_initialized: true,
        };

        let corpus_entries = load_corpus(corpus)?;
        let mut denied = 0usize;
        for entry in &corpus_entries {
            match self.guard.check(&entry.op, &ctx) {
                Err(violation) if violation.code.as_str() == entry.invariant => denied += 1,
                Err(violation) => failures.push(format!(
                    "{}: refused as `{}`, corpus declares `{}`",
                    entry.name,
                    violation.code.as_str(),
                    entry.invariant
                )),
                Ok(()) => failures.push(format!(
                    "{}: was NOT refused, corpus declares `{}`",
                    entry.name, entry.invariant
                )),
            }
        }
        if corpus_entries.is_empty() {
            failures.push("the adversarial corpus is empty".to_string());
        }

        let terminal_triggers = self
            .scalar_i64(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' \
                 AND name IN ('goals_terminal_is_immutable', 'commitments_terminal_is_immutable')",
            )
            .await?;
        if terminal_triggers != 2 {
            failures.push(format!(
                "{terminal_triggers} of 2 terminal-row triggers present"
            ));
        }

        let unbacked_observations = self
            .scalar_i64(
                "SELECT count(*) FROM user_beliefs WHERE epistemic_status = 'OBSERVED' \
                 AND (evidence_json = '[]' OR evidence_json IS NULL)",
            )
            .await?;
        if unbacked_observations != 0 {
            failures.push(format!(
                "{unbacked_observations} belief(s) are OBSERVED without evidence"
            ));
        }

        let affect_flag_violations = self
            .scalar_i64(
                "SELECT count(*) FROM affect_impulses \
                 WHERE affects_internal_state <> 0 OR affects_reasoning <> 0",
            )
            .await?;
        if affect_flag_violations != 0 {
            failures.push(format!(
                "{affect_flag_violations} affect impulse(s) claim to affect state or reasoning"
            ));
        }

        let budget_reconciled = self.budget_reconciles().await?;
        if !budget_reconciled {
            failures.push("the resource ledger does not reconcile with its accounts".into());
        }

        Ok(VerifyReport {
            identity_ulid: mm_core::ulid_string(&self.identity.id),
            version: self.identity.current_version.clone(),
            invariants_enforced: self.identity.invariant_codes().len(),
            corpus_total: corpus_entries.len(),
            corpus_denied: denied,
            terminal_triggers,
            budget_reconciled,
            unbacked_observations,
            affect_flag_violations,
            failures,
        })
    }

    // ------------------------------------------------------------------ ops ------

    async fn set_block(
        &mut self,
        kind: BlockKind,
        label: String,
        content: String,
        limit_chars: u32,
        op_ulid: Ulid,
    ) -> Result<()> {
        let limit = if limit_chars == 0 {
            default_limit(kind)
        } else {
            limit_chars
        };
        let block = CoreBlock::new(kind, label.clone(), content, limit, op_ulid)?;
        let old_len = self.blocks.get(&kind).map_or(0, CoreBlock::len);
        let now = Timestamp::now().to_rfc3339();
        self.exec(
            "UPDATE core_blocks SET system_to = ? \
             WHERE identity_id = ? AND kind = ? AND system_to IS NULL",
            vec![
                text(now),
                text(mm_core::ulid_string(&self.identity.id)),
                text(kind.as_str()),
            ],
        )
        .await?;
        self.exec(
            "INSERT INTO core_blocks (id, identity_id, kind, label, content, limit_chars, \
             system_from, system_to, updated_ulid) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&self.identity.id)),
                text(kind.as_str()),
                text(block.label.clone()),
                text(block.content.clone()),
                int(i64::from(block.limit_chars)),
                text(Timestamp::now().to_rfc3339()),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.blocks.insert(kind, block);
        self.audit(
            Level::Info,
            codes::BEING_BLOCK_SET,
            op_ulid,
            json!({
                "kind": kind.as_str(),
                "label": label,
                "old_len": old_len,
                "new_len": self.blocks.get(&kind).map_or(0, CoreBlock::len),
                "limit": limit,
            }),
        )
        .await
    }

    async fn set_belief(
        &mut self,
        user_id: Ulid,
        proposition: String,
        status: EpistemicStatus,
        confidence: f32,
        evidence: Vec<String>,
        op_ulid: Ulid,
    ) -> Result<()> {
        let belief =
            Belief::new(proposition.clone(), status, confidence).with_evidence(evidence.clone());
        self.persist_belief(user_id, &belief, &proposition, op_ulid)
            .await?;
        // The block is read before the map is borrowed, because `self_block`
        // borrows `self` and `entry` borrows `self.users`.
        let block = self.self_block();
        self.users
            .entry(user_id)
            .or_insert_with(|| UserModel::new(user_id, block))
            .insert(proposition.clone(), belief);
        self.audit(
            Level::Info,
            codes::BEING_BELIEF_UPDATE,
            op_ulid,
            json!({
                "proposition_hash": content_hash(&proposition),
                "old_status": Value::Null,
                "new_status": status.as_str(),
                "old_conf": Value::Null,
                "new_conf": confidence,
                "evidence_ids": evidence,
                "user_ulid": mm_core::ulid_string(&user_id),
            }),
        )
        .await
    }

    async fn promote_belief(
        &mut self,
        user_id: Ulid,
        proposition: String,
        to: EpistemicStatus,
        evidence: Vec<String>,
        op_ulid: Ulid,
    ) -> Result<()> {
        // The model is cloned out so a refusal can be audited without holding a
        // mutable borrow of `self.users` across an await.
        let block = self.self_block();
        let mut model = self
            .users
            .get(&user_id)
            .cloned()
            .unwrap_or_else(|| UserModel::new(user_id, block));
        let before = model.get(&proposition).cloned();
        let updated = match model.promote(&proposition, to, &evidence) {
            Ok(belief) => belief.clone(),
            Err(BeingError::Promotion(denied)) => {
                self.audit(
                    Level::Warn,
                    codes::BEING_BELIEF_PROMOTION_DENIED,
                    op_ulid,
                    json!({
                        "proposition_hash": content_hash(&proposition),
                        "requested_status": to.as_str(),
                        "reason": denied.reason,
                        "user_ulid": mm_core::ulid_string(&user_id),
                    }),
                )
                .await?;
                return Err(BeingError::Promotion(denied));
            }
            Err(other) => return Err(other),
        };
        self.users.insert(user_id, model);
        self.persist_belief(user_id, &updated, &proposition, op_ulid)
            .await?;
        self.audit(
            Level::Info,
            codes::BEING_BELIEF_UPDATE,
            op_ulid,
            json!({
                "proposition_hash": content_hash(&proposition),
                "old_status": before.as_ref().map(|b| b.epistemic_status.as_str()),
                "new_status": updated.epistemic_status.as_str(),
                "old_conf": before.as_ref().map(|b| b.confidence),
                "new_conf": updated.confidence,
                "evidence_ids": updated.evidence,
                "user_ulid": mm_core::ulid_string(&user_id),
            }),
        )
        .await
    }

    async fn set_personality(&mut self, key: String, value: f32, op_ulid: Ulid) -> Result<()> {
        let old = self.personality.set_disposition(&key, value);
        let new = self.personality.get_disposition(&key);
        self.exec(
            "INSERT INTO personality_dispositions (key, baseline, value, confidence, updated_ulid) \
             VALUES (?, ?, ?, 0.5, ?) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_ulid = excluded.updated_ulid",
            vec![
                text(key.clone()),
                real(f64::from(new)),
                real(f64::from(new)),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.audit(
            Level::Info,
            codes::BEING_PERSONALITY_UPDATE,
            op_ulid,
            json!({"key": key, "old": old, "new": new, "confidence": 0.5}),
        )
        .await
    }

    async fn appraise(&mut self, event: AppraisalEvent, op_ulid: Ulid) -> Result<()> {
        let impulse = appraise(&event);
        self.affect.apply(&impulse);
        self.exec(
            "INSERT INTO affect_impulses (id, emotion, intensity, decay, affects_internal_state, \
             affects_reasoning, created_ulid, expires_ulid) VALUES (?, ?, ?, ?, 0, 0, ?, NULL)",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(impulse.emotion().as_str()),
                real(f64::from(impulse.intensity())),
                real(f64::from(impulse.decay())),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.persist_affect(op_ulid).await?;
        self.audit(
            Level::Debug,
            codes::BEING_AFFECT_IMPULSE,
            op_ulid,
            json!({
                "emotion": impulse.emotion().as_str(),
                "intensity": impulse.intensity(),
                "decay": impulse.decay(),
                "affects_internal_state": impulse.affects_internal_state(),
                "affects_reasoning": impulse.affects_reasoning(),
            }),
        )
        .await
    }

    async fn set_motivation(&mut self, drive: Drive, strength: f32, op_ulid: Ulid) -> Result<()> {
        let motivation = Motivation::new(drive, strength);
        self.motivations.insert(drive, motivation.strength);
        self.exec(
            "INSERT INTO motivations (id, drive, strength, updated_ulid) VALUES (?, ?, ?, ?) \
             ON CONFLICT(drive) DO UPDATE SET strength = excluded.strength, \
             updated_ulid = excluded.updated_ulid",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(drive.as_str()),
                real(f64::from(motivation.strength)),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.emit(
            Level::Debug,
            codes::BEING_INVARIANT_CHECK,
            json!({"op": "set_motivation", "drive": drive.as_str(), "strength": motivation.strength}),
        )
        .await
    }

    async fn add_goal(
        &mut self,
        description: String,
        priority: f32,
        origin: GoalOrigin,
        op_ulid: Ulid,
    ) -> Result<()> {
        let id = self.ids.next();
        let mut goal = Goal {
            id,
            owner: self.identity.id,
            description,
            status: GoalStatus::Active,
            priority: 0.0,
            deadline: None,
            parent: None,
            evidence: Vec::new(),
            origin,
        };
        goal.set_priority(priority);
        self.exec(
            "INSERT INTO goals (id, owner_id, description, status, priority, deadline_ulid, \
             parent_id, evidence_json, origin, created_ulid) VALUES (?, ?, ?, ?, ?, NULL, NULL, '[]', ?, ?)",
            vec![
                text(mm_core::ulid_string(&goal.id)),
                text(mm_core::ulid_string(&self.identity.id)),
                text(goal.description.clone()),
                text(goal.status.as_str()),
                real(f64::from(goal.priority)),
                text(goal.origin.as_str()),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.exec(
            "INSERT INTO goal_transitions (id, goal_id, from_status, to_status, at_ulid, reason) \
             VALUES (?, ?, 'none', ?, ?, 'created')",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&goal.id)),
                text(goal.status.as_str()),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.goals.insert(goal.id, goal);
        self.audit(
            Level::Info,
            codes::BEING_GOAL_TRANSITION,
            op_ulid,
            json!({
                "goal_ulid": mm_core::ulid_string(&id),
                "old_status": "none",
                "new_status": "active",
                "origin": origin.as_str(),
            }),
        )
        .await
    }

    async fn transition_goal(
        &mut self,
        id: Ulid,
        to: GoalStatus,
        reason: String,
        op_ulid: Ulid,
    ) -> Result<()> {
        let goal = self
            .goals
            .get(&id)
            .ok_or_else(|| BeingError::NotFound(format!("goal {}", mm_core::ulid_string(&id))))?;
        let from = goal.status;
        if let Some(violation) = deny_terminal_transition(
            &BeingOp::TransitionGoal {
                id,
                from: Some(from),
                to,
                reason: reason.clone(),
            },
            from.is_terminal(),
        ) {
            self.audit(
                Level::Error,
                codes::BEING_INVARIANT_VIOLATION,
                op_ulid,
                json!({
                    "op": violation.op,
                    "invariant_code": violation.code.as_str(),
                    "reason": violation.reason,
                }),
            )
            .await?;
            return Err(BeingError::Invariant(violation));
        }
        let next = transition_goal(from, to)?;
        self.exec(
            "UPDATE goals SET status = ? WHERE id = ?",
            vec![text(next.as_str()), text(mm_core::ulid_string(&id))],
        )
        .await?;
        self.exec(
            "INSERT INTO goal_transitions (id, goal_id, from_status, to_status, at_ulid, reason) \
             VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&id)),
                text(from.as_str()),
                text(next.as_str()),
                text(mm_core::ulid_string(&op_ulid)),
                text(reason),
            ],
        )
        .await?;
        if let Some(goal) = self.goals.get_mut(&id) {
            goal.status = next;
        }
        self.audit(
            Level::Info,
            codes::BEING_GOAL_TRANSITION,
            op_ulid,
            json!({
                "goal_ulid": mm_core::ulid_string(&id),
                "old_status": from.as_str(),
                "new_status": next.as_str(),
            }),
        )
        .await
    }

    async fn add_commitment(
        &mut self,
        goal_id: Option<Ulid>,
        made_to: Ulid,
        description: String,
        op_ulid: Ulid,
    ) -> Result<()> {
        if let Some(goal) = goal_id {
            if !self.goals.contains_key(&goal) {
                return Err(BeingError::NotFound(format!(
                    "goal {}",
                    mm_core::ulid_string(&goal)
                )));
            }
        }
        let id = self.ids.next();
        let commitment = Commitment {
            id,
            goal_id,
            made_to,
            description: description.clone(),
            status: CommitmentStatus::Active,
            deadline: None,
        };
        self.exec(
            "INSERT INTO commitments (id, goal_id, made_to, description, status, deadline_ulid, \
             created_ulid, terminal_ulid) VALUES (?, ?, ?, ?, 'active', NULL, ?, NULL)",
            vec![
                text(mm_core::ulid_string(&id)),
                opt_text(goal_id.map(|g| mm_core::ulid_string(&g))),
                text(mm_core::ulid_string(&made_to)),
                text(description),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.exec(
            "INSERT INTO commitment_transitions (id, commitment_id, from_status, to_status, \
             at_ulid, reason) VALUES (?, ?, 'none', 'active', ?, 'created')",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&id)),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.commitments.insert(id, commitment);
        self.audit(
            Level::Info,
            codes::BEING_COMMITMENT_TRANSITION,
            op_ulid,
            json!({
                "commitment_ulid": mm_core::ulid_string(&id),
                "old_status": "none",
                "new_status": "active",
                "made_to": mm_core::ulid_string(&made_to),
            }),
        )
        .await
    }

    async fn transition_commitment(
        &mut self,
        id: Ulid,
        to: CommitmentStatus,
        reason: String,
        op_ulid: Ulid,
    ) -> Result<()> {
        let commitment = self.commitments.get(&id).ok_or_else(|| {
            BeingError::NotFound(format!("commitment {}", mm_core::ulid_string(&id)))
        })?;
        let from = commitment.status;
        if let Some(violation) = deny_terminal_transition(
            &BeingOp::TransitionCommitment {
                id,
                from: Some(from),
                to,
                reason: reason.clone(),
            },
            from.is_terminal(),
        ) {
            self.audit(
                Level::Error,
                codes::BEING_INVARIANT_VIOLATION,
                op_ulid,
                json!({
                    "op": violation.op,
                    "invariant_code": violation.code.as_str(),
                    "reason": violation.reason,
                }),
            )
            .await?;
            return Err(BeingError::Invariant(violation));
        }
        let next = transition_commitment(from, to)?;
        let terminal = if next.is_terminal() {
            opt_text(Some(mm_core::ulid_string(&op_ulid)))
        } else {
            Param::Null
        };
        self.exec(
            "UPDATE commitments SET status = ?, terminal_ulid = ? WHERE id = ?",
            vec![
                text(next.as_str()),
                terminal,
                text(mm_core::ulid_string(&id)),
            ],
        )
        .await?;
        self.exec(
            "INSERT INTO commitment_transitions (id, commitment_id, from_status, to_status, \
             at_ulid, reason) VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&id)),
                text(from.as_str()),
                text(next.as_str()),
                text(mm_core::ulid_string(&op_ulid)),
                text(reason),
            ],
        )
        .await?;
        if let Some(commitment) = self.commitments.get_mut(&id) {
            commitment.status = next;
        }
        self.audit(
            Level::Info,
            codes::BEING_COMMITMENT_TRANSITION,
            op_ulid,
            json!({
                "commitment_ulid": mm_core::ulid_string(&id),
                "old_status": from.as_str(),
                "new_status": next.as_str(),
            }),
        )
        .await
    }

    async fn debit_budget(
        &mut self,
        kind: ResourceKind,
        amount: f64,
        purpose: String,
        op_ulid: Ulid,
    ) -> Result<()> {
        let policy = self.load_policy(kind).await?;
        let mut conn = self
            .store
            .pool()
            .acquire()
            .await
            .map_err(|e| BeingError::Db(e.to_string()))?;
        (&mut *conn)
            .execute("BEGIN IMMEDIATE")
            .await
            .map_err(|e| BeingError::Db(e.to_string()))?;

        let outcome = apply_debit(
            &mut conn,
            &mut self.resources,
            kind,
            amount,
            &purpose,
            policy.as_ref(),
            op_ulid,
        )
        .await;

        match outcome {
            Ok(receipt) => {
                (&mut *conn)
                    .execute("COMMIT")
                    .await
                    .map_err(|e| BeingError::Db(e.to_string()))?;
                self.audit(
                    Level::Info,
                    codes::BEING_BUDGET_DEBIT,
                    op_ulid,
                    json!({
                        "kind": receipt.kind.as_str(),
                        "amount": receipt.amount,
                        "balance_after": receipt.balance_after,
                        "purpose": receipt.purpose,
                        "ref_ulid": mm_core::ulid_string(&op_ulid),
                    }),
                )
                .await
            }
            Err(e) => {
                let _ = (&mut *conn).execute("ROLLBACK").await;
                // A refused spend is a control event, not a system failure: the
                // deterministic arithmetic did exactly its job.
                let level = if matches!(e, BeingError::Budget(_)) {
                    Level::Warn
                } else {
                    Level::Error
                };
                self.audit(
                    level,
                    codes::BEING_BUDGET_EXCEEDED,
                    op_ulid,
                    json!({
                        "kind": kind.as_str(),
                        "requested": amount,
                        "balance": self.resources.balance(kind),
                        "policy": policy
                            .as_ref()
                            .map(|p| p.period.clone())
                            .unwrap_or_else(|| "default".into()),
                        "cause": e.to_string(),
                    }),
                )
                .await?;
                Err(e)
            }
        }
    }

    async fn relationship_event(
        &mut self,
        user_id: Ulid,
        kind: RelKind,
        op_ulid: Ulid,
    ) -> Result<()> {
        let event = RelationshipEvent::for_kind(kind, Some(op_ulid));
        let state = self
            .relationships
            .entry(user_id)
            .or_insert_with(RelationshipState::neutral);
        let before = state.clone();
        state.apply(&event);
        let state = self
            .relationships
            .get(&user_id)
            .cloned()
            .unwrap_or_else(|| before.clone());
        let rel_id = relationship_id(user_id);
        self.exec(
            "INSERT INTO relationships (id, user_id, familiarity, trust, reciprocity, openness, \
             cooperation, reliance, recent_quality, unresolved_json, updated_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id) DO UPDATE SET familiarity = excluded.familiarity, \
             trust = excluded.trust, reciprocity = excluded.reciprocity, \
             openness = excluded.openness, cooperation = excluded.cooperation, \
             reliance = excluded.reliance, recent_quality = excluded.recent_quality, \
             unresolved_json = excluded.unresolved_json, updated_ulid = excluded.updated_ulid",
            vec![
                text(mm_core::ulid_string(&rel_id)),
                text(mm_core::ulid_string(&user_id)),
                real(f64::from(state.familiarity)),
                real(f64::from(state.trust)),
                real(f64::from(state.reciprocity)),
                real(f64::from(state.openness)),
                real(f64::from(state.cooperation)),
                real(f64::from(state.reliance)),
                real(f64::from(state.recent_quality)),
                text(serde_json::to_string(&state.unresolved)?),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.exec(
            "INSERT INTO relationship_events (id, relationship_id, kind, delta_json, ref_ulid, \
             created_ulid) VALUES (?, ?, ?, ?, ?, ?)",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&rel_id)),
                text(kind.as_str()),
                text(serde_json::to_string(&event.delta)?),
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        self.audit(
            Level::Info,
            codes::BEING_RELATIONSHIP_UPDATE,
            op_ulid,
            json!({
                "user_ulid": mm_core::ulid_string(&user_id),
                "kind": kind.as_str(),
                "dims_changed": {
                    "trust": [f64::from(before.trust), f64::from(state.trust)],
                    "familiarity": [f64::from(before.familiarity), f64::from(state.familiarity)],
                },
                "ref_ulid": mm_core::ulid_string(&op_ulid),
            }),
        )
        .await
    }

    // --------------------------------------------------------------- internals ---

    fn self_block(&self) -> CoreBlock {
        self.blocks
            .get(&BlockKind::Self_)
            .cloned()
            .unwrap_or_else(|| CoreBlock {
                kind: BlockKind::Self_,
                label: "self".to_string(),
                content: self.identity.self_description.clone(),
                limit_chars: default_limit(BlockKind::Self_),
                updated_ulid: self.identity.id,
            })
    }

    async fn load(&mut self) -> Result<()> {
        let identity_id = mm_core::ulid_string(&self.identity.id);
        // Read the self block once: it is the fallback for a user model that was
        // stored before its block existed, and the closure below cannot borrow
        // `self` while `self.users` is mutably borrowed.
        let self_block = self.self_block();
        for row in self
            .query(
                "SELECT id, kind, label, content, limit_chars, updated_ulid FROM core_blocks \
                 WHERE identity_id = ? AND system_to IS NULL ORDER BY kind",
                vec![text(identity_id.clone())],
            )
            .await?
        {
            let (Some(kind), Some(label), Some(content)) = (
                row["kind"].as_str().and_then(BlockKind::parse),
                row["label"].as_str(),
                row["content"].as_str(),
            ) else {
                continue;
            };
            self.blocks.insert(
                kind,
                CoreBlock {
                    kind,
                    label: label.to_string(),
                    content: content.to_string(),
                    limit_chars: row["limit_chars"].as_u64().unwrap_or(2000) as u32,
                    updated_ulid: parse_ulid_field(&row, "updated_ulid")?,
                },
            );
        }
        for row in self
            .query(
                "SELECT key, value FROM personality_dispositions",
                Vec::new(),
            )
            .await?
        {
            if let (Some(key), Some(value)) = (row["key"].as_str(), row["value"].as_f64()) {
                self.personality.set_disposition(key, value as f32);
            }
        }
        if let Some(row) = self
            .query(
                "SELECT valence, arousal, engagement, warmth, caution, curiosity, energy \
                 FROM affect_state WHERE identity_id = ?",
                vec![text(identity_id.clone())],
            )
            .await?
            .first()
        {
            self.affect = Affect {
                valence: as_f32(row, "valence"),
                arousal: as_f32(row, "arousal"),
                engagement: as_f32(row, "engagement"),
                warmth: as_f32(row, "warmth"),
                caution: as_f32(row, "caution"),
                curiosity: as_f32(row, "curiosity"),
                energy: as_f32(row, "energy"),
            }
            .clamped();
        }
        for row in self
            .query("SELECT drive, strength FROM motivations", Vec::new())
            .await?
        {
            if let (Some(drive), Some(strength)) = (row["drive"].as_str(), row["strength"].as_f64())
            {
                if let Some(drive) = Drive::parse(drive) {
                    self.motivations.insert(drive, strength as f32);
                }
            }
        }
        for row in self
            .query(
                "SELECT id, user_id, proposition, epistemic_status, confidence, evidence_json, \
                 created_ulid FROM user_beliefs ORDER BY created_ulid",
                Vec::new(),
            )
            .await?
        {
            let (Some(user), Some(proposition), Some(status)) = (
                row["user_id"].as_str(),
                row["proposition"].as_str(),
                row["epistemic_status"]
                    .as_str()
                    .and_then(EpistemicStatus::parse),
            ) else {
                continue;
            };
            let user = mm_core::id::parse_ulid(user)?;
            let mut belief = Belief::new(proposition, status, as_f32(&row, "confidence"));
            belief.evidence = serde_json::from_str(row["evidence_json"].as_str().unwrap_or("[]"))
                .unwrap_or_default();
            let block = self_block.clone();
            self.users
                .entry(user)
                .or_insert_with(|| UserModel::new(user, block))
                .insert(proposition.to_string(), belief);
        }
        for row in self
            .query(
                "SELECT user_id, familiarity, trust, reciprocity, openness, cooperation, \
                 reliance, recent_quality, unresolved_json FROM relationships",
                Vec::new(),
            )
            .await?
        {
            let Some(user) = row["user_id"].as_str() else {
                continue;
            };
            let user = mm_core::id::parse_ulid(user)?;
            self.relationships.insert(
                user,
                RelationshipState {
                    familiarity: as_f32(&row, "familiarity"),
                    trust: as_f32(&row, "trust"),
                    reciprocity: as_f32(&row, "reciprocity"),
                    openness: as_f32(&row, "openness"),
                    cooperation: as_f32(&row, "cooperation"),
                    reliance: as_f32(&row, "reliance"),
                    recent_quality: as_f32(&row, "recent_quality"),
                    unresolved: serde_json::from_str(
                        row["unresolved_json"].as_str().unwrap_or("[]"),
                    )
                    .unwrap_or_default(),
                },
            );
        }
        for row in self
            .query(
                "SELECT id, owner_id, description, status, priority, origin FROM goals",
                Vec::new(),
            )
            .await?
        {
            let (Some(id), Some(status), Some(origin)) = (
                row["id"].as_str(),
                row["status"].as_str().and_then(GoalStatus::parse),
                row["origin"].as_str().and_then(GoalOrigin::parse),
            ) else {
                continue;
            };
            self.goals.insert(
                mm_core::id::parse_ulid(id)?,
                Goal {
                    id: mm_core::id::parse_ulid(id)?,
                    owner: row["owner_id"]
                        .as_str()
                        .map(mm_core::id::parse_ulid)
                        .transpose()?
                        .unwrap_or(self.identity.id),
                    description: row["description"].as_str().unwrap_or_default().to_string(),
                    status,
                    priority: as_f32(&row, "priority"),
                    deadline: None,
                    parent: None,
                    evidence: Vec::new(),
                    origin,
                },
            );
        }
        for row in self
            .query(
                "SELECT id, goal_id, made_to, description, status FROM commitments",
                Vec::new(),
            )
            .await?
        {
            let (Some(id), Some(made_to), Some(status)) = (
                row["id"].as_str(),
                row["made_to"].as_str(),
                row["status"].as_str().and_then(CommitmentStatus::parse),
            ) else {
                continue;
            };
            self.commitments.insert(
                mm_core::id::parse_ulid(id)?,
                Commitment {
                    id: mm_core::id::parse_ulid(id)?,
                    goal_id: row["goal_id"]
                        .as_str()
                        .map(mm_core::id::parse_ulid)
                        .transpose()?,
                    made_to: mm_core::id::parse_ulid(made_to)?,
                    description: row["description"].as_str().unwrap_or_default().to_string(),
                    status,
                    deadline: None,
                },
            );
        }
        for row in self
            .query(
                "SELECT kind, balance, unit FROM resource_accounts",
                Vec::new(),
            )
            .await?
        {
            if let (Some(kind), Some(balance), Some(unit)) = (
                row["kind"].as_str().and_then(ResourceKind::parse),
                row["balance"].as_f64(),
                row["unit"].as_str(),
            ) {
                let account = self.resources.open(kind, balance);
                account.unit = unit.to_string();
            }
        }
        Ok(())
    }

    /// Make sure the singleton rows exist, so a first run has a real constitution.
    async fn seed_defaults(&mut self) -> Result<()> {
        let identity_id = mm_core::ulid_string(&self.identity.id);
        for (key, value) in &self.personality.dispositions {
            let exists = self
                .scalar_i64_with(
                    "SELECT count(*) AS n FROM personality_dispositions WHERE key = ?",
                    vec![text(key.clone())],
                )
                .await
                .unwrap_or(0);
            if exists == 0 {
                self.exec(
                    "INSERT INTO personality_dispositions (key, baseline, value, confidence, \
                     updated_ulid) VALUES (?, ?, ?, 0.5, ?)",
                    vec![
                        text(key.clone()),
                        real(f64::from(*value)),
                        real(f64::from(*value)),
                        text(identity_id.clone()),
                    ],
                )
                .await?;
            }
        }
        let affect_rows = self
            .scalar_i64_with(
                "SELECT count(*) AS n FROM affect_state WHERE identity_id = ?",
                vec![text(identity_id.clone())],
            )
            .await
            .unwrap_or(0);
        if affect_rows == 0 {
            self.persist_affect(self.identity.id).await?;
        }
        for (drive, strength) in &self.motivations {
            self.exec(
                "INSERT INTO motivations (id, drive, strength, updated_ulid) VALUES (?, ?, ?, ?) \
                 ON CONFLICT(drive) DO NOTHING",
                vec![
                    text(mm_core::ulid_string(&self.ids.next())),
                    text(drive.as_str()),
                    real(f64::from(*strength)),
                    text(identity_id.clone()),
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn persist_belief(
        &self,
        user_id: Ulid,
        belief: &Belief,
        proposition: &str,
        op_ulid: Ulid,
    ) -> Result<()> {
        let user = mm_core::ulid_string(&user_id);
        self.exec(
            "UPDATE user_beliefs SET valid_until_ulid = ? \
             WHERE user_id = ? AND proposition_hash = ? AND valid_until_ulid IS NULL",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(user.clone()),
                text(belief.proposition_hash()),
            ],
        )
        .await?;
        self.exec(
            "INSERT INTO user_beliefs (id, user_id, proposition, proposition_hash, \
             epistemic_status, confidence, evidence_json, valid_from_ulid, valid_until_ulid, \
             created_ulid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                text(mm_core::ulid_string(&op_ulid)),
                text(user),
                text(proposition.to_string()),
                text(belief.proposition_hash()),
                text(belief.epistemic_status.as_str()),
                real(f64::from(belief.confidence)),
                text(serde_json::to_string(&belief.evidence)?),
                text(mm_core::ulid_string(&op_ulid)),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        Ok(())
    }

    async fn persist_affect(&self, op_ulid: Ulid) -> Result<()> {
        let affect = self.affect;
        self.exec(
            "INSERT INTO affect_state (identity_id, valence, arousal, engagement, warmth, \
             caution, curiosity, energy, updated_ulid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(identity_id) DO UPDATE SET valence = excluded.valence, \
             arousal = excluded.arousal, engagement = excluded.engagement, \
             warmth = excluded.warmth, caution = excluded.caution, \
             curiosity = excluded.curiosity, energy = excluded.energy, \
             updated_ulid = excluded.updated_ulid",
            vec![
                text(mm_core::ulid_string(&self.identity.id)),
                real(f64::from(affect.valence)),
                real(f64::from(affect.arousal)),
                real(f64::from(affect.engagement)),
                real(f64::from(affect.warmth)),
                real(f64::from(affect.caution)),
                real(f64::from(affect.curiosity)),
                real(f64::from(affect.energy)),
                text(mm_core::ulid_string(&op_ulid)),
            ],
        )
        .await?;
        Ok(())
    }

    async fn load_policy(&self, kind: ResourceKind) -> Result<Option<BudgetPolicy>> {
        let rows = self
            .query(
                "SELECT period, limit_amount, hard FROM budget_policies WHERE kind = ?",
                vec![text(kind.as_str())],
            )
            .await?;
        Ok(rows.first().map(|row| BudgetPolicy {
            period: row["period"].as_str().unwrap_or("open").to_string(),
            limit_amount: row["limit_amount"].as_f64().unwrap_or(0.0),
            hard: row["hard"].as_i64().unwrap_or(1) != 0,
        }))
    }

    /// True when every account equals the sum of its ledger deltas and every
    /// ledger row records the balance it produced.
    pub async fn budget_reconciles(&self) -> Result<bool> {
        let bad_accounts = self
            .scalar_i64(
                "SELECT count(*) FROM resource_accounts a WHERE a.balance <> \
                 COALESCE((SELECT sum(l.delta) FROM resource_ledger l WHERE l.kind = a.kind), 0.0)",
            )
            .await?;
        if bad_accounts != 0 {
            return Ok(false);
        }
        let rows = self
            .query(
                "SELECT kind, delta, balance_after FROM resource_ledger \
                 ORDER BY kind, created_ulid, id",
                Vec::new(),
            )
            .await?;
        let mut running: BTreeMap<String, f64> = BTreeMap::new();
        for row in rows {
            let (Some(kind), Some(delta), Some(after)) = (
                row["kind"].as_str(),
                row["delta"].as_f64(),
                row["balance_after"].as_f64(),
            ) else {
                return Ok(false);
            };
            let total = running.entry(kind.to_string()).or_insert(0.0);
            *total += delta;
            if (*total - after).abs() > f64::EPSILON {
                return Ok(false);
            }
        }
        Ok(true)
    }

    async fn exec(&self, sql: &str, args: Params) -> Result<u64> {
        Ok(Tabular::execute(&self.store, sql, args).await?)
    }

    async fn query(&self, sql: &str, args: Params) -> Result<Vec<Value>> {
        Ok(Tabular::query_json(&self.store, sql, args).await?)
    }

    async fn scalar_i64(&self, sql: &str) -> Result<i64> {
        self.scalar_i64_with(sql, Vec::new()).await
    }

    async fn scalar_i64_with(&self, sql: &str, args: Params) -> Result<i64> {
        Ok(self
            .query(sql, args)
            .await?
            .first()
            .and_then(|row| row.as_object())
            .and_then(|map| map.values().next())
            .and_then(Value::as_i64)
            .unwrap_or(0))
    }

    async fn emit(&self, level: Level, code: &str, fields: Value) -> Result<()> {
        let mut record = LogRecord::new(level, code, TARGET);
        if let Value::Object(map) = fields {
            for (key, value) in map {
                record = record.with_field(&key, value);
            }
        }
        Ok(self.logger.emit(record).await?)
    }

    async fn audit(&self, level: Level, code: &str, op_ulid: Ulid, fields: Value) -> Result<()> {
        Ok(self
            .logger
            .audit(level, code, TARGET, Some(op_ulid), fields)
            .await?)
    }
}

/// Apply one debit inside an already-open transaction.
///
/// The caller owns `BEGIN`/`COMMIT`, so a refusal rolls back to exactly the state
/// the account had before the attempt — a budget check that half-applied would be
/// worse than no check at all.
async fn apply_debit(
    conn: &mut sqlx::SqliteConnection,
    state: &mut ResourceState,
    kind: ResourceKind,
    amount: f64,
    purpose: &str,
    policy: Option<&BudgetPolicy>,
    op_ulid: Ulid,
) -> Result<DebitReceipt> {
    // The account row is authoritative under the write lock. A second writer must
    // not compute from a stale in-memory balance, or two concurrent debits could
    // each pass a hard budget the pair would exceed.
    let current: Option<f64> =
        sqlx::query_scalar("SELECT balance FROM resource_accounts WHERE kind = ?")
            .bind(kind.as_str())
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| BeingError::Db(e.to_string()))?;
    if let Some(balance) = current {
        state.open(kind, balance);
    }
    let receipt = debit(state, kind, amount, purpose, policy)?;
    sqlx::query(
        "INSERT INTO resource_accounts (kind, balance, unit, updated_ulid) VALUES (?, ?, ?, ?) \
         ON CONFLICT(kind) DO UPDATE SET balance = excluded.balance, unit = excluded.unit, \
         updated_ulid = excluded.updated_ulid",
    )
    .bind(kind.as_str())
    .bind(receipt.balance_after)
    .bind(kind.unit())
    .bind(mm_core::ulid_string(&op_ulid))
    .execute(&mut *conn)
    .await
    .map_err(|e| BeingError::Db(e.to_string()))?;
    sqlx::query(
        "INSERT INTO resource_ledger (id, kind, delta, balance_after, purpose, ref_ulid, \
         created_ulid) VALUES (?, ?, ?, ?, ?, NULL, ?)",
    )
    .bind(mm_core::ulid_string(&op_ulid))
    .bind(kind.as_str())
    .bind(-receipt.amount)
    .bind(receipt.balance_after)
    .bind(purpose)
    .bind(mm_core::ulid_string(&op_ulid))
    .execute(&mut *conn)
    .await
    .map_err(|e| BeingError::Db(e.to_string()))?;
    Ok(receipt)
}

fn relationship_id(user: Ulid) -> Ulid {
    // A deterministic relationship id: the same user always maps to the same row
    // key, so the projection cannot fork.
    let mut bytes = [0u8; 16];
    let hash = mm_core::content_hash(mm_core::ulid_string(&user).as_bytes());
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hash[i * 2..i * 2 + 2], 16).unwrap_or(0);
    }
    Ulid::from_bytes(bytes)
}

fn content_hash(value: &str) -> String {
    mm_core::content_hash(value.as_bytes())
}

fn text(value: impl Into<String>) -> Param {
    Param::Text(value.into())
}

fn int(value: i64) -> Param {
    Param::Int(value)
}

fn real(value: f64) -> Param {
    Param::Real(value)
}

fn opt_text(value: Option<String>) -> Param {
    Param::opt_text(value)
}

fn as_f32(row: &Value, key: &str) -> f32 {
    row[key].as_f64().unwrap_or(0.0) as f32
}

fn parse_ulid_field(row: &Value, key: &str) -> Result<Ulid> {
    let raw = row[key]
        .as_str()
        .ok_or_else(|| BeingError::Db(format!("column `{key}` is missing")))?;
    Ok(mm_core::id::parse_ulid(raw)?)
}

async fn load_identity(store: &SqliteStore) -> Result<Option<Identity>> {
    let rows = Tabular::query_json(
        store,
        "SELECT id, self_description, lineage_json, current_version FROM identity LIMIT 1",
        Vec::new(),
    )
    .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let raw_id = row["id"]
        .as_str()
        .ok_or_else(|| BeingError::Db("identity row has no id".into()))?;
    let id = mm_core::id::parse_ulid(raw_id)?;
    let created = id
        .datetime()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| Timestamp {
            seconds: d.as_secs(),
            nanos: 0,
        })
        .unwrap_or_else(|_| Timestamp::now());
    let lineage: Vec<SelfVersion> =
        serde_json::from_str(row["lineage_json"].as_str().unwrap_or("[]")).unwrap_or_default();
    // The `invariants` table is the authority for the codes; the identity row does
    // not duplicate them, because two copies of a fixed set is one copy too many.
    let mut invariants: Vec<InvariantId> = Tabular::query_json(
        store,
        "SELECT code FROM invariants WHERE identity_id = ? ORDER BY code",
        vec![text(raw_id)],
    )
    .await?
    .iter()
    .filter_map(|row| row["code"].as_str().map(str::to_string))
    .collect();
    if invariants.is_empty() {
        invariants = CORE_INVARIANTS
            .iter()
            .map(|c| c.as_str().to_string())
            .collect();
    }
    Ok(Some(Identity {
        id,
        created_at: created,
        self_description: row["self_description"]
            .as_str()
            .unwrap_or(DEFAULT_SELF_DESCRIPTION)
            .to_string(),
        lineage,
        current_version: row["current_version"].as_str().unwrap_or("v1").to_string(),
        invariants,
    }))
}

async fn persist_identity(store: &SqliteStore, identity: &Identity) -> Result<()> {
    Tabular::execute(
        store,
        "INSERT INTO identity (id, created_ulid, self_description, lineage_json, \
         current_version, schema_version) VALUES (?, ?, ?, ?, ?, 1)",
        vec![
            text(mm_core::ulid_string(&identity.id)),
            text(mm_core::ulid_string(&identity.id)),
            text(identity.self_description.clone()),
            text(serde_json::to_string(&identity.lineage)?),
            text(identity.current_version.clone()),
        ],
    )
    .await?;
    Ok(())
}

async fn ensure_invariants(
    store: &SqliteStore,
    identity: &Identity,
    ids: &UlidFactory,
) -> Result<()> {
    for code in CORE_INVARIANTS {
        Tabular::execute(
            store,
            "INSERT INTO invariants (id, identity_id, code, assertion, enforcement, created_ulid) \
             VALUES (?, ?, ?, ?, 'deterministic', ?) ON CONFLICT(code) DO NOTHING",
            vec![
                text(mm_core::ulid_string(&ids.next())),
                text(mm_core::ulid_string(&identity.id)),
                text(code.as_str()),
                text(code.assertion()),
                text(mm_core::ulid_string(&ids.next())),
            ],
        )
        .await?;
    }
    Ok(())
}

/// One entry of the adversarial corpus.
struct CorpusEntry {
    name: String,
    op: BeingOp,
    invariant: String,
}

fn load_corpus(path: &Path) -> Result<Vec<CorpusEntry>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| BeingError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut entries = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let value: Value = serde_json::from_str(line)
            .map_err(|e| BeingError::Config(format!("{}:{}: {e}", path.display(), index + 1)))?;
        let name = value["name"].as_str().unwrap_or("unnamed").to_string();
        let invariant = value["invariant"]
            .as_str()
            .ok_or_else(|| {
                BeingError::Config(format!(
                    "{}:{}: missing `invariant`",
                    path.display(),
                    index + 1
                ))
            })?
            .to_string();
        if InvariantCode::parse(&invariant).is_none() {
            return Err(BeingError::Config(format!(
                "{}:{}: `{invariant}` is not one of the four core invariants",
                path.display(),
                index + 1
            )));
        }
        let op_name = value["op"].as_str().unwrap_or_default();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let op = BeingOp::from_corpus(op_name, &params)?;
        entries.push(CorpusEntry {
            name,
            op,
            invariant,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relationship_id_is_deterministic() {
        let user = Ulid::from_parts(1_700_000_000_000, 5);
        assert_eq!(relationship_id(user), relationship_id(user));
        assert_ne!(
            relationship_id(user),
            relationship_id(Ulid::from_parts(1, 6))
        );
    }

    #[test]
    fn the_verify_report_needs_a_real_corpus() {
        let report = VerifyReport {
            identity_ulid: "x".into(),
            version: "v1".into(),
            invariants_enforced: 4,
            corpus_total: 0,
            corpus_denied: 0,
            terminal_triggers: 2,
            budget_reconciled: true,
            unbacked_observations: 0,
            affect_flag_violations: 0,
            failures: Vec::new(),
        };
        assert!(!report.ok(), "an empty corpus must not read as a pass");
    }
}
