//! Where claims live: the eight Phase 6 tables, read and written through the
//! Phase 1 [`Tabular`] boundary.
//!
//! The store is deliberately thin. It owns the *encoding* — a proposition becomes
//! three text columns through [`Proposition::object_text`], a timestamp becomes an
//! RFC3339 string — and nothing else. Status transitions, promotion decisions, and
//! the `/world` separation live above it, in [`crate::engine`] and
//! [`crate::validate`], because a store that also decided would make the decision
//! unauditable.
//!
//! Every row carries `system_from`/`system_to`: the system-time interval over
//! which the row is the current view. Closing a row (`system_to`) rather than
//! deleting it is what makes `replay` able to reconstruct what the being knew at a
//! past instant.

use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Params, Tabular, Ulid, UlidFactory};
use mm_log::{Level, Logger};
use mm_store_sqlite::SqliteStore;
use serde_json::Value;

use crate::assumption::Assumption;
use crate::claim::{Claim, ClaimKind, Evidence, EvidenceKind, Observation};
use crate::contradiction::{Contradiction, ContradictionStatus};
use crate::error::{EpistemicError, Result};
use crate::justification::{DepKind, Justification};
use crate::prediction::{Prediction, PredictionOutcome};
use crate::proposition::{iri_node, term_from_text, Proposition, RiskLevel};
use crate::status::EpistemicStatus;
use crate::TARGET;

/// The operations the epistemic engine needs from persistence.
#[async_trait]
pub trait EpistemicStore: Send + Sync {
    /// Store a claim.
    async fn put_claim(&self, claim: &Claim) -> Result<()>;
    /// Fetch a claim.
    async fn get_claim(&self, id: &Ulid) -> Result<Option<Claim>>;
    /// Every claim.
    async fn list_claims(&self) -> Result<Vec<Claim>>;
    /// The evidence recorded against a claim.
    async fn evidence_for(&self, claim: &Ulid) -> Result<Vec<Evidence>>;
    /// Record a status transition.
    async fn put_transition(&self, transition: &Transition) -> Result<()>;
}

/// One recorded status change.
///
/// The row is the *audit* of a change: what the subject was, what it became, why,
/// and which PROV activity recorded it. It is written for every accepted change
/// and for no refused one, which is what `logs verify`'s epistemic correlation
/// check counts against.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Transition {
    /// The transition's own ULID.
    pub id: Ulid,
    /// The claim that changed.
    pub subject: Ulid,
    /// The status it held, when it held one.
    pub from_status: Option<EpistemicStatus>,
    /// The status it now holds.
    pub to_status: Option<EpistemicStatus>,
    /// Why it changed.
    pub reason: String,
    /// The evidence that justified it, when one did.
    pub evidence: Option<Ulid>,
    /// The PROV activity that recorded it.
    pub prov_activity: Option<Ulid>,
}

/// The SQLite-backed epistemic store.
#[derive(Clone)]
pub struct SqliteEpistemicStore {
    sqlite: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl std::fmt::Debug for SqliteEpistemicStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteEpistemicStore")
            .field("path", &self.sqlite.path())
            .finish_non_exhaustive()
    }
}

impl SqliteEpistemicStore {
    /// Build a store over an open SQLite handle.
    pub fn new(sqlite: SqliteStore, logger: Arc<Logger>, ids: Arc<UlidFactory>) -> Self {
        SqliteEpistemicStore {
            sqlite,
            logger,
            ids,
        }
    }

    /// The underlying SQLite handle.
    pub fn sqlite(&self) -> &SqliteStore {
        &self.sqlite
    }

    /// The logger this store writes to.
    pub fn logger(&self) -> &Arc<Logger> {
        &self.logger
    }

    /// Mint a fresh ULID.
    pub fn next_id(&self) -> Ulid {
        self.ids.next()
    }

    async fn query(&self, sql: &str, args: Params) -> Result<Vec<Value>> {
        Tabular::query_json(&self.sqlite, sql, args)
            .await
            .map_err(Into::into)
    }

    async fn exec(&self, sql: &str, args: Params) -> Result<u64> {
        Ok(Tabular::execute(&self.sqlite, sql, args).await?)
    }

    async fn scalar(&self, sql: &str, args: Params) -> Result<i64> {
        let rows = self.query(sql, args).await?;
        Ok(rows
            .first()
            .and_then(Value::as_object)
            .and_then(|map| map.values().next())
            .and_then(Value::as_i64)
            .unwrap_or(0))
    }

    /// The instant a new row becomes current, as the DDL stores it.
    fn now() -> String {
        mm_core::Timestamp::now().to_rfc3339()
    }

    // ------------------------------------------------------------------ claims --

    /// Store a claim and its evidence rows.
    pub async fn insert_claim(&self, claim: &Claim, evidence: &[Evidence]) -> Result<()> {
        claim.validate()?;
        self.exec(
            "INSERT INTO claims (id, kind, subject, predicate, object, status, confidence, \
             valid_from, valid_until, source_ulid, system_from, system_to, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&claim.id)),
                Param::Text(claim.kind.as_str().to_string()),
                Param::Text(claim.proposition.subject_text().to_string()),
                Param::Text(claim.proposition.predicate_text().to_string()),
                Param::Text(claim.proposition.object_text()),
                Param::Text(claim.status.as_str().to_string()),
                Param::Real(f64::from(claim.confidence)),
                Param::opt_text(claim.valid_from.map(|t| t.to_rfc3339())),
                Param::opt_text(claim.valid_until.map(|t| t.to_rfc3339())),
                Param::Text(Self::now()),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        for item in evidence {
            self.insert_evidence(&claim.id, item).await?;
        }
        Ok(())
    }

    /// Replace a claim's status in place, closing the old system interval.
    pub async fn update_claim_status(&self, id: &Ulid, status: EpistemicStatus) -> Result<()> {
        let affected = self
            .exec(
                "UPDATE claims SET status = ?, system_from = ? WHERE id = ? AND system_to IS NULL",
                vec![
                    Param::Text(status.as_str().to_string()),
                    Param::Text(Self::now()),
                    Param::Text(mm_core::ulid_string(id)),
                ],
            )
            .await?;
        if affected == 0 {
            return Err(EpistemicError::NotFound(mm_core::ulid_string(id)));
        }
        Ok(())
    }

    /// Fetch one claim.
    pub async fn fetch_claim(&self, id: &Ulid) -> Result<Option<Claim>> {
        let rows = self
            .query(
                "SELECT * FROM claims WHERE id = ? ORDER BY system_from DESC LIMIT 1",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let mut claim = claim_of(row)?;
        claim.evidence = self
            .evidence_for(id)
            .await?
            .into_iter()
            .map(|e| e.id)
            .collect();
        Ok(Some(claim))
    }

    /// Every claim, ordered by ULID.
    pub async fn list_all_claims(&self) -> Result<Vec<Claim>> {
        let rows = self
            .query("SELECT * FROM claims ORDER BY id", Vec::new())
            .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut claim = claim_of(row)?;
            claim.evidence = self
                .evidence_for(&claim.id)
                .await?
                .into_iter()
                .map(|e| e.id)
                .collect();
            out.push(claim);
        }
        Ok(out)
    }

    /// The evidence recorded for a claim.
    pub async fn evidence_for(&self, claim: &Ulid) -> Result<Vec<Evidence>> {
        let rows = self
            .query(
                "SELECT * FROM evidence WHERE claim_id = ? ORDER BY id",
                vec![Param::Text(mm_core::ulid_string(claim))],
            )
            .await?;
        rows.iter().map(evidence_of).collect()
    }

    /// Record one piece of evidence.
    pub async fn insert_evidence(&self, claim: &Ulid, evidence: &Evidence) -> Result<()> {
        evidence.validate()?;
        self.exec(
            "INSERT INTO evidence (id, claim_id, kind, source_uri, span_start, span_end, \
             content_hash, reliability, system_from, system_to, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&evidence.id)),
                Param::Text(mm_core::ulid_string(claim)),
                Param::Text(evidence.kind.as_str().to_string()),
                Param::opt_text(evidence.source_uri.as_ref().map(|n| n.as_str().to_string())),
                evidence
                    .span
                    .map_or(Param::Null, |(start, _)| Param::Int(i64::from(start))),
                evidence
                    .span
                    .map_or(Param::Null, |(_, end)| Param::Int(i64::from(end))),
                Param::Text(evidence.content_hash_hex()),
                Param::Real(f64::from(evidence.reliability)),
                Param::Text(Self::now()),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------ observations --

    /// Record an observation.
    pub async fn insert_observation(&self, observation: &Observation) -> Result<()> {
        self.exec(
            "INSERT INTO observations (id, claim_id, authoritative, observed_at, source_ulid, \
             system_from, system_to, created_ulid) VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&observation.id)),
                Param::Text(mm_core::ulid_string(&observation.claim_id)),
                Param::Int(i64::from(observation.authoritative)),
                Param::Text(observation.observed_at.to_rfc3339()),
                Param::Text(mm_core::ulid_string(&observation.source)),
                Param::Text(Self::now()),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Every observation for a claim.
    pub async fn observations_for(&self, claim: &Ulid) -> Result<Vec<Observation>> {
        let rows = self
            .query(
                "SELECT * FROM observations WHERE claim_id = ? ORDER BY id",
                vec![Param::Text(mm_core::ulid_string(claim))],
            )
            .await?;
        rows.iter().map(observation_of).collect()
    }

    // ------------------------------------------------------------- assumptions --

    /// Store an assumption.
    pub async fn insert_assumption(&self, assumption: &Assumption) -> Result<()> {
        assumption.validate()?;
        self.exec(
            "INSERT INTO assumptions (id, proposition_uri, status, confidence, \
             consequence_if_false, verification_cost, decision_dependence, valid_from, \
             valid_until, system_from, system_to, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&assumption.id)),
                Param::Text(assumption.proposition.object_text()),
                Param::Text(assumption.status.as_str().to_string()),
                Param::Real(f64::from(assumption.confidence)),
                Param::Text(assumption.consequence_if_false.as_str().to_string()),
                Param::Real(assumption.verification_cost),
                Param::Real(assumption.decision_dependence),
                Param::opt_text(assumption.valid_from.map(|t| t.to_rfc3339())),
                Param::opt_text(assumption.valid_until.map(|t| t.to_rfc3339())),
                Param::Text(Self::now()),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Every assumption.
    pub async fn list_assumptions(&self) -> Result<Vec<Assumption>> {
        let rows = self
            .query("SELECT * FROM assumptions ORDER BY id", Vec::new())
            .await?;
        rows.iter().map(assumption_of).collect()
    }

    // ------------------------------------------------------------- predictions --

    /// Store a prediction.
    pub async fn insert_prediction(&self, prediction: &Prediction) -> Result<()> {
        prediction.validate()?;
        let (status, value, resolved_at) = match &prediction.outcome {
            Some(outcome) => (
                Param::Text(outcome.status.as_str().to_string()),
                Param::opt_text(outcome.value.clone()),
                Param::Text(outcome.resolved_at.to_rfc3339()),
            ),
            None => (Param::Null, Param::Null, Param::Null),
        };
        self.exec(
            "INSERT INTO predictions (id, proposition_uri, probability, horizon_secs, \
             conditions_json, created_ulid, system_from, system_to, outcome_status, outcome_value, \
             resolved_at) VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&prediction.id)),
                Param::Text(prediction.proposition.object_text()),
                Param::Real(prediction.probability),
                Param::Int(prediction.horizon_secs as i64),
                Param::Text(serde_json::to_string(&prediction.conditions)?),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
                Param::Text(Self::now()),
                status,
                value,
                resolved_at,
            ],
        )
        .await?;
        Ok(())
    }

    /// Every prediction, for the mirror.
    pub async fn list_predictions(&self) -> Result<Vec<Prediction>> {
        let rows = self
            .query("SELECT * FROM predictions ORDER BY id", Vec::new())
            .await?;
        rows.iter().map(prediction_of).collect()
    }

    // ----------------------------------------------------------- contradictions --

    /// Store a contradiction, refusing a duplicate of the same ordered pair.
    pub async fn insert_contradiction(&self, contradiction: &Contradiction) -> Result<bool> {
        let existing = self
            .scalar(
                "SELECT count(*) FROM contradictions WHERE claim_a = ? AND claim_b = ? \
                 AND status != 'dismissed'",
                vec![
                    Param::Text(mm_core::ulid_string(&contradiction.claim_a)),
                    Param::Text(mm_core::ulid_string(&contradiction.claim_b)),
                ],
            )
            .await?;
        if existing > 0 {
            return Ok(false);
        }
        self.exec(
            "INSERT INTO contradictions (id, claim_a, claim_b, reason, evidence_a, evidence_b, \
             status, system_from, system_to, created_ulid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&contradiction.id)),
                Param::Text(mm_core::ulid_string(&contradiction.claim_a)),
                Param::Text(mm_core::ulid_string(&contradiction.claim_b)),
                Param::Text(contradiction.reason.clone()),
                Param::opt_text(
                    contradiction.evidence_a.map(|e| mm_core::ulid_string(&e)),
                ),
                Param::opt_text(
                    contradiction.evidence_b.map(|e| mm_core::ulid_string(&e)),
                ),
                Param::Text(contradiction.status.as_str().to_string()),
                Param::Text(Self::now()),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        Ok(true)
    }

    /// Every contradiction.
    pub async fn list_contradictions(&self) -> Result<Vec<Contradiction>> {
        let rows = self
            .query("SELECT * FROM contradictions ORDER BY id", Vec::new())
            .await?;
        rows.iter().map(contradiction_of).collect()
    }

    // ----------------------------------------------------------- dependencies --

    /// Store a justification edge.
    pub async fn insert_justification(&self, justification: &Justification) -> Result<()> {
        for antecedent in &justification.antecedents {
            self.exec(
                "INSERT INTO dependencies (id, consequent, antecedent, kind, criticality, \
                 system_from, system_to, created_ulid) VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&self.ids.next())),
                    Param::Text(mm_core::ulid_string(&justification.consequent)),
                    Param::Text(mm_core::ulid_string(antecedent)),
                    Param::Text(justification.kind.as_str().to_string()),
                    Param::Real(f64::from(justification.criticality)),
                    Param::Text(Self::now()),
                    Param::Text(mm_core::ulid_string(&self.ids.next())),
                ],
            )
            .await?;
        }
        Ok(())
    }

    /// Every justification edge.
    pub async fn list_justifications(&self) -> Result<Vec<Justification>> {
        let rows = self
            .query(
                "SELECT consequent, antecedent, kind, criticality FROM dependencies \
                 ORDER BY consequent, antecedent",
                Vec::new(),
            )
            .await?;
        let mut out = Vec::new();
        for row in &rows {
            let (Some(consequent), Some(antecedent), Some(kind)) = (
                row["consequent"].as_str(),
                row["antecedent"].as_str(),
                row["kind"].as_str().and_then(DepKind::parse),
            ) else {
                continue;
            };
            out.push(Justification::new(
                parse_id(consequent)?,
                vec![parse_id(antecedent)?],
                kind,
                row["criticality"].as_f64().unwrap_or(1.0) as f32,
            )?);
        }
        Ok(out)
    }

    // ------------------------------------------------------------ transitions --

    /// Record a status transition.
    pub async fn insert_transition(&self, transition: &Transition) -> Result<()> {
        self.exec(
            "INSERT INTO epistemic_transitions (id, subject, from_status, to_status, reason, \
             evidence_ulid, prov_activity, system_from, system_to, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&transition.id)),
                Param::Text(mm_core::ulid_string(&transition.subject)),
                Param::opt_text(transition.from_status.map(|s| s.as_str().to_string())),
                Param::opt_text(transition.to_status.map(|s| s.as_str().to_string())),
                Param::Text(transition.reason.clone()),
                Param::opt_text(transition.evidence.map(|e| mm_core::ulid_string(&e))),
                Param::opt_text(transition.prov_activity.map(|a| mm_core::ulid_string(&a))),
                Param::Text(Self::now()),
                Param::Text(mm_core::ulid_string(&self.ids.next())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Every recorded transition.
    pub async fn list_transitions(&self) -> Result<Vec<Transition>> {
        let rows = self
            .query(
                "SELECT * FROM epistemic_transitions ORDER BY id",
                Vec::new(),
            )
            .await?;
        rows.iter().map(transition_of).collect()
    }

    /// How many rows a table holds, for reconciliation.
    pub async fn count_of(&self, table: &str) -> Result<usize> {
        let count = self
            .scalar(&format!("SELECT count(*) FROM {table}"), Vec::new())
            .await?;
        Ok(count.max(0) as usize)
    }

    /// Emit an audit record.
    pub async fn audit(&self, code: &'static str, trace: Ulid, fields: Value) -> Result<()> {
        self.logger
            .audit(Level::Info, code, TARGET, Some(trace), fields)
            .await?;
        Ok(())
    }

    /// Emit a non-audit record.
    pub async fn emit(&self, code: &'static str, trace: Ulid, fields: Value) -> Result<()> {
        let mut record = mm_log::LogRecord::new(Level::Debug, code, TARGET)
            .with_field("trace_id", mm_core::ulid_string(&trace));
        if let Some(map) = fields.as_object() {
            for (key, value) in map {
                record = record.with_field(key.as_str(), value.clone());
            }
        }
        self.logger.emit(record).await?;
        Ok(())
    }
}

#[async_trait]
impl EpistemicStore for SqliteEpistemicStore {
    async fn put_claim(&self, claim: &Claim) -> Result<()> {
        self.insert_claim(claim, &[]).await
    }

    async fn get_claim(&self, id: &Ulid) -> Result<Option<Claim>> {
        self.fetch_claim(id).await
    }

    async fn list_claims(&self) -> Result<Vec<Claim>> {
        self.list_all_claims().await
    }

    async fn evidence_for(&self, claim: &Ulid) -> Result<Vec<Evidence>> {
        SqliteEpistemicStore::evidence_for(self, claim).await
    }

    async fn put_transition(&self, transition: &Transition) -> Result<()> {
        self.insert_transition(transition).await
    }
}

// --------------------------------------------------------------------- helpers --

/// Parse a ULID out of a column.
pub fn parse_id(text: &str) -> Result<Ulid> {
    mm_core::id::parse_ulid(text)
        .map_err(|e| EpistemicError::Codec(format!("invalid ULID {text:?}: {e}")))
}

fn claim_of(row: &Value) -> Result<Claim> {
    let id = parse_id(row["id"].as_str().unwrap_or_default())?;
    let kind = row["kind"]
        .as_str()
        .and_then(ClaimKind::parse)
        .ok_or_else(|| EpistemicError::Codec(format!("unknown claim kind {}", row["kind"])))?;
    let status = row["status"]
        .as_str()
        .and_then(EpistemicStatus::parse)
        .ok_or_else(|| EpistemicError::Codec(format!("unknown status {}", row["status"])))?;
    let object = term_from_text(row["object"].as_str().unwrap_or_default())?;
    let proposition = Proposition {
        subject: iri_node(row["subject"].as_str().unwrap_or_default())?,
        predicate: iri_node(row["predicate"].as_str().unwrap_or_default())?,
        object,
    };
    Ok(Claim {
        id,
        kind,
        proposition,
        status,
        confidence: row["confidence"].as_f64().unwrap_or(0.0) as f32,
        evidence: Vec::new(),
        valid_from: timestamp_of(row["valid_from"].as_str()),
        valid_until: timestamp_of(row["valid_until"].as_str()),
    })
}

fn evidence_of(row: &Value) -> Result<Evidence> {
    let id = parse_id(row["id"].as_str().unwrap_or_default())?;
    let kind = row["kind"]
        .as_str()
        .and_then(EvidenceKind::parse)
        .ok_or_else(|| EpistemicError::Codec(format!("unknown evidence kind {}", row["kind"])))?;
    let hash = crate::claim::unhex(row["content_hash"].as_str().unwrap_or_default())?;
    let source_uri = row["source_uri"].as_str().map(iri_node).transpose()?;
    let mut evidence = Evidence::new(
        id,
        kind,
        source_uri,
        hash,
        row["reliability"].as_f64().unwrap_or(0.0) as f32,
    )?;
    if let (Some(start), Some(end)) = (row["span_start"].as_i64(), row["span_end"].as_i64()) {
        evidence = evidence.with_span(start as u32, end as u32)?;
    }
    Ok(evidence)
}

fn observation_of(row: &Value) -> Result<Observation> {
    let id = parse_id(row["id"].as_str().unwrap_or_default())?;
    let claim_id = parse_id(row["claim_id"].as_str().unwrap_or_default())?;
    let source = parse_id(row["source_ulid"].as_str().unwrap_or_default())?;
    let observed_at =
        timestamp_of(row["observed_at"].as_str()).unwrap_or(mm_core::Timestamp::EPOCH);
    Observation::new(
        id,
        claim_id,
        row["authoritative"].as_i64().unwrap_or(0) != 0,
        observed_at,
        source,
    )
}

fn assumption_of(row: &Value) -> Result<Assumption> {
    let id = parse_id(row["id"].as_str().unwrap_or_default())?;
    let status = row["status"]
        .as_str()
        .and_then(EpistemicStatus::parse)
        .ok_or_else(|| EpistemicError::Codec(format!("unknown status {}", row["status"])))?;
    let risk = row["consequence_if_false"]
        .as_str()
        .and_then(RiskLevel::parse)
        .ok_or_else(|| {
            EpistemicError::Codec(format!("unknown risk {}", row["consequence_if_false"]))
        })?;
    // The proposition is stored by its object text only; the subject and predicate
    // of an assumption are the assumption record itself, which the mirror renders.
    let object = row["proposition_uri"].as_str().unwrap_or_default();
    let proposition = Proposition::literal(
        "https://metamind.dev/ontology#Assumption",
        "https://metamind.dev/ontology#assumes",
        object,
    )?;
    let mut assumption = Assumption::new(
        id,
        proposition,
        row["confidence"].as_f64().unwrap_or(0.0) as f32,
        risk,
        row["verification_cost"].as_f64().unwrap_or(0.0),
        row["decision_dependence"].as_f64().unwrap_or(0.0),
    )?;
    assumption.status = status;
    assumption.valid_from = timestamp_of(row["valid_from"].as_str());
    assumption.valid_until = timestamp_of(row["valid_until"].as_str());
    Ok(assumption)
}

fn prediction_of(row: &Value) -> Result<Prediction> {
    let id = parse_id(row["id"].as_str().unwrap_or_default())?;
    let object = row["proposition_uri"].as_str().unwrap_or_default();
    let proposition = Proposition::literal(
        "https://metamind.dev/ontology#Prediction",
        "https://metamind.dev/ontology#predicts",
        object,
    )?;
    let mut prediction = Prediction::new(
        id,
        proposition,
        row["probability"].as_f64().unwrap_or(0.0),
        row["horizon_secs"].as_i64().unwrap_or(0).max(0) as u64,
    )?;
    prediction.conditions =
        serde_json::from_str(row["conditions_json"].as_str().unwrap_or("[]")).unwrap_or_default();
    if let Some(status) = row["outcome_status"]
        .as_str()
        .and_then(crate::outcome::OutcomeStatus::parse)
    {
        let resolved_at =
            timestamp_of(row["resolved_at"].as_str()).unwrap_or(mm_core::Timestamp::EPOCH);
        prediction.outcome = Some(PredictionOutcome {
            status,
            value: row["outcome_value"].as_str().map(str::to_string),
            resolved_at,
            error: None,
        });
    }
    Ok(prediction)
}

fn contradiction_of(row: &Value) -> Result<Contradiction> {
    let id = parse_id(row["id"].as_str().unwrap_or_default())?;
    let claim_a = parse_id(row["claim_a"].as_str().unwrap_or_default())?;
    let claim_b = parse_id(row["claim_b"].as_str().unwrap_or_default())?;
    let status = row["status"]
        .as_str()
        .and_then(ContradictionStatus::parse)
        .ok_or_else(|| {
            EpistemicError::Codec(format!("unknown contradiction status {}", row["status"]))
        })?;
    Ok(Contradiction {
        id,
        claim_a,
        claim_b,
        reason: row["reason"].as_str().unwrap_or_default().to_string(),
        evidence_a: row["evidence_a"].as_str().map(parse_id).transpose()?,
        evidence_b: row["evidence_b"].as_str().map(parse_id).transpose()?,
        status,
    })
}

fn transition_of(row: &Value) -> Result<Transition> {
    Ok(Transition {
        id: parse_id(row["id"].as_str().unwrap_or_default())?,
        subject: parse_id(row["subject"].as_str().unwrap_or_default())?,
        from_status: row["from_status"].as_str().and_then(EpistemicStatus::parse),
        to_status: row["to_status"].as_str().and_then(EpistemicStatus::parse),
        reason: row["reason"].as_str().unwrap_or_default().to_string(),
        evidence: row["evidence_ulid"].as_str().map(parse_id).transpose()?,
        prov_activity: row["prov_activity"].as_str().map(parse_id).transpose()?,
    })
}

/// Parse an RFC3339 column back into a timestamp.
pub fn timestamp_of(text: Option<&str>) -> Option<mm_core::Timestamp> {
    text.and_then(|t| mm_core::Timestamp::from_rfc3339(t).ok())
}
