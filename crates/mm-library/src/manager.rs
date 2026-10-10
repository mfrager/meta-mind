//! The commit gate and the one door into `/library`.
//!
//! Every write to the library goes through [`LibraryManager`], which composes the
//! pieces below it in the order the plan fixes:
//!
//! 1. the entry validates itself ([`LibraryEntry::validate`]) — a field-level
//!    refusal names the field;
//! 2. its Turtle is **SHACL-checked before anything is written** ([`crate::validate`])
//!    — "SHACL-or-nothing" is invariant 1, and a violation leaves no trace in
//!    either store;
//! 3. the content hash is checked against the index — the same body twice would
//!    double its weight in every rank;
//! 4. every reference resolves, or is external — a dangling `mm:evidence` is a
//!    missing argument, not a formatting nit;
//! 5. only then is the canonical Turtle inserted into the `/library` graph and the
//!    index row written, with one `library.entry.upsert` audit record.
//!
//! A refusal at steps 1–4 returns a typed [`LibraryError`], emits
//! `library.entry.rejected`, and writes nothing. That ordering is the point of the
//! module, and it is what the gate's "a violation writes nothing" check exercises.

use std::collections::{BTreeMap, BTreeSet};

use mm_core::{ActivationCondition, NamedNode, Ulid};
use mm_log::{codes, Level, LogRecord, Logger};
use mm_store_graph::GraphHandle;

use crate::entry::{iri, Declarative, EntryKind, LibraryEntry, POLICY_BASE};
use crate::error::{LibraryError, Result, ShapeViolation};
use crate::policy::{FitnessRecord, Policy, PolicyBehavior, PolicyDelta, Scope};
use crate::rdf::{content_hash, LIBRARY_GRAPH};
use crate::skill::{Skill, Verification};
use crate::store::{EntryRow, LibraryStore};
use crate::validate::{self, Duplicate, OrphanReport};

/// The logger target every record from the manager carries.
pub const TARGET: &str = "mm.library";

/// What a successful commit produced.
#[derive(Debug, Clone, PartialEq)]
pub struct CommitOutcome {
    /// The versioned IRI the entry is stored under.
    pub version_iri: String,
    /// The canonical content hash.
    pub content_hash: String,
    /// The activation score the index holds.
    pub activation_score: f32,
}

/// Every problem the library currently has.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ValidationReport {
    /// How many entries were inspected.
    pub entries: usize,
    /// Shape violations, entry by entry.
    pub violations: Vec<(String, ShapeViolation)>,
    /// Entries sharing a body with another.
    pub duplicates: Vec<Duplicate>,
    /// References that do not resolve.
    pub orphans: Vec<OrphanReport>,
}

impl ValidationReport {
    /// True when the library is clean.
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty() && self.duplicates.is_empty() && self.orphans.is_empty()
    }

    /// The one-line summary the CLI prints.
    pub fn summary(&self) -> String {
        format!(
            "{} entries, {} shape violation(s), {} duplicate(s), {} orphaned entry(ies)",
            self.entries,
            self.violations.len(),
            self.duplicates.len(),
            self.orphans.len()
        )
    }
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportReport {
    /// Entries written, in IRI order.
    pub accepted: Vec<String>,
    /// Entries refused, with the reason.
    pub rejected: Vec<(String, String)>,
}

/// The library's write path.
#[derive(Clone)]
pub struct LibraryManager {
    store: LibraryStore,
    graph: Option<GraphHandle>,
}

impl std::fmt::Debug for LibraryManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryManager")
            .field("store", &self.store)
            .field("with_graph", &self.graph.is_some())
            .finish()
    }
}

impl LibraryManager {
    /// Build a manager over an index and, when the caller wants graph writes, a
    /// handle to the `/library` graph.
    pub fn new(store: LibraryStore, graph: Option<GraphHandle>) -> Self {
        LibraryManager { store, graph }
    }

    /// The index.
    pub fn store(&self) -> &LibraryStore {
        &self.store
    }

    /// The logger.
    pub fn logger(&self) -> &std::sync::Arc<Logger> {
        self.store.logger()
    }

    /// Attach a graph handle.
    pub fn with_graph(mut self, graph: GraphHandle) -> Self {
        self.graph = Some(graph);
        self
    }

    /// The graph handle, or a refusal naming what is missing.
    pub fn graph(&self) -> Result<&GraphHandle> {
        self.graph
            .as_ref()
            .ok_or_else(|| LibraryError::Graph("no /library graph handle is attached".into()))
    }

    // ---------------------------------------------------------------- commits --

    /// Commit one entry.
    ///
    /// `activation` is the ACT-R-style blend the index records, because it is a
    /// function of *use* and therefore not a field of the entry being written: a
    /// freshly written entry has no uses yet.
    pub async fn commit(&self, entry: &dyn LibraryEntry, activation: f32) -> Result<CommitOutcome> {
        let version_iri = entry.version_iri().into_string();
        entry.validate()?;
        let turtle = entry.to_turtle()?;

        if let Err(violations) = shape_check(&turtle, &version_iri) {
            self.reject(
                &version_iri,
                &format!("{} shape violation(s)", violations.len()),
            )
            .await?;
            return Err(LibraryError::Shape {
                entry: version_iri,
                violations,
            });
        }

        let hash = content_hash(&turtle)?;
        if let Some(existing) = self.store.entry_with_hash(&hash).await? {
            if existing != version_iri {
                self.emit_dup(&version_iri, &existing, &hash).await?;
                return Err(LibraryError::Duplicate {
                    iri: version_iri,
                    existing,
                    content_hash: hash,
                });
            }
        }

        let missing = self
            .missing_references(entry.referenced_iris(), &version_iri)
            .await?;
        if !missing.is_empty() {
            self.emit_orphans(&version_iri, &missing).await?;
            return Err(LibraryError::Orphan {
                entry: version_iri,
                missing,
            });
        }

        self.write(
            &version_iri,
            &entry.head_iri().into_string(),
            entry.kind(),
            entry.title(),
            entry.version(),
            &turtle,
            &hash,
            activation,
        )
        .await?;

        // The index says what was written; the graph says what it says.
        self.graph()?
            .insert_turtle(LIBRARY_GRAPH, &turtle)
            .await
            .map_err(|e| LibraryError::Graph(e.to_string()))?;

        self.logger()
            .emit(
                LogRecord::new(Level::Info, codes::LIBRARY_ENTRY_UPSERT, TARGET)
                    .with_field("entry_iri", version_iri.clone())
                    .with_field("kind", entry.kind().as_str())
                    .with_field("version", entry.version())
                    .with_field("content_hash", hash.clone())
                    .with_field("activation_score", f64::from(activation))
                    .with_field("result", "accepted"),
            )
            .await?;
        self.logger()
            .emit(
                LogRecord::new(Level::Debug, codes::LIBRARY_ENTRY_VALIDATED, TARGET)
                    .with_field("entry_iri", version_iri.clone())
                    .with_field("shapes", "library.shacl.ttl")
                    .with_field("violation_count", 0),
            )
            .await?;

        Ok(CommitOutcome {
            version_iri,
            content_hash: hash,
            activation_score: activation,
        })
    }

    /// Commit a declarative entry, using the activation score it carries.
    pub async fn commit_declarative(&self, entry: &Declarative) -> Result<CommitOutcome> {
        self.commit(entry, entry.activation_score).await
    }

    /// Index and graph-write an already-validated document, subject by subject.
    ///
    /// Used by `library import`: the document is validated once as a whole against
    /// the shapes, then each subject is indexed with **its own** content hash, so
    /// the duplicate check and `library list --kind` see real entries rather than
    /// one blob. The per-entry body is the subject's canonical N-Triples, which the
    /// shapes and the reference scanner both read.
    pub async fn import_document(&self, turtle: &str) -> Result<ImportReport> {
        validate::validate_turtle_with_shape_bytes(turtle, "library_seed")?;
        let groups = crate::rdf::group_lines(turtle)?;
        let mut report = ImportReport::default();
        let mut seen: Vec<(String, String)> = Vec::new();

        for (subject, lines) in &groups {
            match self.index_subject(subject, lines, &mut seen).await {
                Ok(iri) => report.accepted.push(iri),
                Err(e) => report.rejected.push((subject.clone(), e.to_string())),
            }
        }

        self.graph()?
            .insert_turtle(LIBRARY_GRAPH, turtle)
            .await
            .map_err(|e| LibraryError::Graph(e.to_string()))?;
        Ok(report)
    }

    /// Index one subject of a parsed document.
    async fn index_subject(
        &self,
        subject: &str,
        lines: &[String],
        seen: &mut Vec<(String, String)>,
    ) -> Result<String> {
        let body = format!("{}\n", lines.join("\n"));
        let hash = mm_core::content_hash(lines.join("\n").as_bytes());
        // The grouped subject is rendered as Turtle (`<iri>`); the entry's identity
        // is the bare IRI, because that is what the index stores and what a
        // reference in another entry compares against.
        let iri_text = subject.trim_start_matches('<').trim_end_matches('>');

        let kind = crate::rdf::kind_of_lines(lines).ok_or_else(|| {
            LibraryError::validation("kind", format!("{subject} has no library kind"))
        })?;
        let version = crate::rdf::literal_in(lines, "version")
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(1);
        let version_iri = if iri_text.contains('@') {
            iri_text.to_string()
        } else {
            let slug = iri_text.rsplit('/').next().unwrap_or(iri_text);
            iri::versioned(kind, slug, version).into_string()
        };
        let head = iri::head_of(&version_iri).unwrap_or_else(|| version_iri.clone());
        let title = crate::rdf::literal_in(lines, "title").unwrap_or_else(|| head.clone());
        let activation = crate::rdf::literal_in(lines, "activationScore")
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0) as f32;

        if let Some((other, _)) = seen.iter().find(|(_, h)| *h == hash) {
            return Err(LibraryError::Duplicate {
                iri: version_iri,
                existing: other.clone(),
                content_hash: hash,
            });
        }
        if let Some(existing) = self.store.entry_with_hash(&hash).await? {
            if existing != version_iri {
                self.emit_dup(&version_iri, &existing, &hash).await?;
                return Err(LibraryError::Duplicate {
                    iri: version_iri,
                    existing,
                    content_hash: hash,
                });
            }
        }
        seen.push((version_iri.clone(), hash.clone()));

        self.write(
            &version_iri,
            &head,
            kind,
            &title,
            version,
            &body,
            &hash,
            activation,
        )
        .await?;

        // A policy is two rows, not one: the index row above and its immutable
        // version below. Indexing only the first would leave `policy history`
        // empty for a seeded policy, and would make the first evolved version
        // claim to be version 1 all over again.
        if kind == EntryKind::Policy {
            let name = iri::slug_of(&head).unwrap_or_default();
            let version_number = i64::from(version);
            self.store.ensure_policy(&head, &name).await?;
            self.store
                .insert_policy_version(
                    &head,
                    version_number,
                    version_number
                        .checked_sub(1)
                        .filter(|previous| *previous > 0),
                    &crate::rdf::literal_in(lines, "scope").unwrap_or_else(|| "global".into()),
                    &crate::rdf::literal_in(lines, "activationCondition").unwrap_or_default(),
                    &crate::rdf::literal_in(lines, "behavior").unwrap_or_default(),
                    &body,
                    crate::rdf::literal_in(lines, "confidence")
                        .and_then(|value| value.parse::<f64>().ok())
                        .unwrap_or(0.5),
                    &hash,
                    0,
                )
                .await?;
        }
        Ok(version_iri)
    }

    /// Write the index row.
    #[allow(clippy::too_many_arguments)]
    async fn write(
        &self,
        version_iri: &str,
        head_iri: &str,
        kind: EntryKind,
        title: &str,
        version: u32,
        body_ttl: &str,
        hash: &str,
        activation: f32,
    ) -> Result<()> {
        let slug = iri::slug_of(version_iri).unwrap_or_default();
        let written = self
            .store
            .insert_entry(
                head_iri,
                version_iri,
                kind.as_str(),
                &slug,
                i64::from(version),
                title,
                body_ttl,
                hash,
                f64::from(activation),
            )
            .await?;
        if !written {
            return Err(LibraryError::Duplicate {
                iri: version_iri.to_string(),
                existing: version_iri.to_string(),
                content_hash: hash.to_string(),
            });
        }
        Ok(())
    }

    /// The references that do not resolve, excluding the entry's own IRIs.
    async fn missing_references(
        &self,
        referenced: Vec<NamedNode>,
        self_iri: &str,
    ) -> Result<Vec<String>> {
        if referenced.is_empty() {
            return Ok(Vec::new());
        }
        let known = self.store.known_iris().await?;
        let head = iri::head_of(self_iri).unwrap_or_default();
        let mut missing: Vec<String> = referenced
            .into_iter()
            .map(|node| node.into_string())
            .filter(|target| target != self_iri && *target != head)
            .filter(|target| !validate::is_external(target))
            .filter(|target| !known.contains(target))
            .collect();
        missing.sort();
        missing.dedup();
        Ok(missing)
    }

    // ------------------------------------------------------------------ checks --

    /// Validate every indexed entry: shapes, duplicates, orphans.
    pub async fn validate_all(&self) -> Result<ValidationReport> {
        let entries = self.store.list_entries(None).await?;
        let mut report = ValidationReport {
            entries: entries.len(),
            ..ValidationReport::default()
        };
        let mut hashes: Vec<(String, String)> = Vec::new();
        for entry in &entries {
            if let Ok(found) =
                validate::violations(&validate::shapes()?, &entry.body_ttl, &entry.version_iri)
            {
                for violation in found {
                    report
                        .violations
                        .push((entry.version_iri.clone(), violation));
                }
            }
            hashes.push((entry.version_iri.clone(), entry.content_hash.clone()));
        }
        report.duplicates = validate::duplicates(&hashes);
        report.orphans = self.orphans().await?;
        Ok(report)
    }

    /// The dangling references across the index.
    pub async fn orphans(&self) -> Result<Vec<OrphanReport>> {
        let known = self.store.known_iris().await?;
        let references = self.store.references().await?;
        Ok(validate::orphans(&references, &known))
    }

    // ------------------------------------------------------------------ skills --

    /// Register a skill, as a draft.
    pub async fn register_skill(&self, skill: &Skill) -> Result<CommitOutcome> {
        let outcome = self.commit(skill, 0.5).await?;
        let iri = outcome.version_iri.clone();
        self.store
            .insert_skill(
                &iri,
                &skill.name,
                &skill.description,
                &skill.impl_ref,
                &skill.entrypoint,
                &skill.signature,
                &skill.tests_ref,
                skill.verification.as_str(),
                i64::from(skill.version),
            )
            .await?;
        self.logger()
            .emit(
                LogRecord::new(Level::Info, codes::SKILL_REGISTER, TARGET)
                    .with_field("skill_iri", iri)
                    .with_field("version", skill.version)
                    .with_field("verification", skill.verification.as_str()),
            )
            .await?;
        Ok(outcome)
    }

    /// Decide a skill's verification state and log the decision.
    pub async fn set_skill_verification(
        &self,
        skill_iri: &str,
        verification: Verification,
        exit_code: i32,
    ) -> Result<()> {
        self.store
            .set_verification(skill_iri, verification.as_str())
            .await?;
        self.logger()
            .emit(
                LogRecord::new(Level::Info, codes::SKILL_VERIFY, TARGET)
                    .with_field("skill_iri", skill_iri.to_string())
                    .with_field("verification", verification.as_str())
                    .with_field("exit_code", i64::from(exit_code)),
            )
            .await?;
        Ok(())
    }

    // ----------------------------------------------------------------- policies --

    /// A new immutable policy version.
    ///
    /// The parent is the policy's current head, read back from the index, and the
    /// new version carries it as its `parent` — [`Policy::next`] enforces the chain,
    /// so a version that skipped a number cannot be built. Nothing in the store is
    /// updated in place: the old row stays byte-identical.
    pub async fn new_policy_version(
        &self,
        name: &str,
        delta: PolicyDelta,
        evidence: &[NamedNode],
    ) -> Result<Policy> {
        let head = iri::head_of(&iri::policy(name, 0).into_string())
            .unwrap_or_else(|| format!("{POLICY_BASE}{name}"));
        let history = self.store.policy_history(&head).await?;

        let policy = match history.last() {
            None => {
                let mut fresh = Policy::new(
                    name,
                    delta.scope.clone(),
                    delta.activation.clone(),
                    delta.behavior.clone(),
                    delta.confidence,
                    name,
                    "A behavioural policy, evolved from evidence.",
                )?;
                fresh.evidence = evidence.to_vec();
                fresh
            }
            Some(last) => {
                let version = last.version.max(1) as u32;
                let parent = Policy {
                    iri: iri::policy(name, version),
                    version,
                    scope: Scope::parse(&last.scope).unwrap_or(Scope::Global),
                    activation: ActivationCondition::new(last.activation.clone())?,
                    behavior: PolicyBehavior::new(&last.behavior)?,
                    evidence: evidence.to_vec(),
                    fitness: FitnessRecord::default(),
                    confidence: last.confidence as f32,
                    parent: last
                        .parent_version
                        .map(|v| iri::policy(name, v.max(1) as u32)),
                    generation: last.generation.max(0) as u32,
                    title: name.to_string(),
                    purpose: "A behavioural policy, evolved from evidence.".to_string(),
                };
                parent.next(delta, 0)?
            }
        };

        let turtle = policy.to_turtle()?;
        validate::validate_turtle_with_shape_bytes(&turtle, policy.iri.as_str())?;
        let hash = content_hash(&turtle)?;
        self.graph()?
            .insert_turtle(LIBRARY_GRAPH, &turtle)
            .await
            .map_err(|e| LibraryError::Graph(e.to_string()))?;
        self.store.ensure_policy(&head, name).await?;
        self.store
            .insert_policy_version(
                &head,
                i64::from(policy.version),
                policy.version.checked_sub(1).map(i64::from),
                &policy.scope.as_str(),
                policy.activation.as_str(),
                policy.behavior.as_str(),
                &turtle,
                f64::from(policy.confidence),
                &hash,
                i64::from(policy.generation),
            )
            .await?;
        self.logger()
            .emit(
                LogRecord::new(Level::Info, codes::POLICY_VERSION_CREATE, TARGET)
                    .with_field("policy_iri", policy.iri.to_string())
                    .with_field("version", policy.version)
                    .with_field("parent_version", policy.version.saturating_sub(1))
                    .with_field("scope", policy.scope.as_str())
                    .with_field("confidence", f64::from(policy.confidence)),
            )
            .await?;
        Ok(policy)
    }

    /// Record one trial of a policy version, folding an outcome into its fitness.
    ///
    /// The Phase 6 [`mm_epistemic::Outcome`] is the only outcome type in the system
    /// (index decision D2), so a success is `Outcome::succeeded()` and a score is
    /// `Outcome::score()`. The update folds the running mean rather than recomputing
    /// it from stored trials, because trials are a stream and a stream cannot be
    /// re-read after the fact.
    pub async fn record_fitness(
        &self,
        policy_iri: &str,
        version: u32,
        outcome: &mm_epistemic::Outcome,
    ) -> Result<FitnessRecord> {
        outcome.validate().map_err(|e| LibraryError::Validation {
            field: "outcome",
            detail: e.to_string(),
        })?;
        let head = iri::head_of(policy_iri).unwrap_or_else(|| policy_iri.to_string());
        let (trials, successes, mean) = self
            .store
            .fitness_of(&head, i64::from(version))
            .await?
            .unwrap_or((0, 0, 0.0));
        let trials = trials + 1;
        let successes = successes + i64::from(outcome.succeeded());
        let mean = (mean * (trials - 1) as f64 + outcome.score()) / trials as f64;
        self.store
            .record_fitness(&head, i64::from(version), trials, successes, mean)
            .await?;
        self.logger()
            .emit(
                LogRecord::new(Level::Info, codes::POLICY_FITNESS_UPDATE, TARGET)
                    .with_field("policy_iri", head)
                    .with_field("version", version)
                    .with_field("trials", trials)
                    .with_field("successes", successes)
                    .with_field("mean_utility", mean),
            )
            .await?;
        Ok(FitnessRecord {
            trials: trials as u32,
            successes: successes as u32,
            mean_utility: mean as f32,
        })
    }

    // ---------------------------------------------------------------- listings --

    /// The entry kinds present in the index, with counts.
    pub async fn kind_counts(&self) -> Result<Vec<(String, i64)>> {
        let entries = self.store.list_entries(None).await?;
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        for entry in entries {
            *counts.entry(entry.kind).or_insert(0) += 1;
        }
        Ok(counts.into_iter().collect())
    }

    /// Every indexed entry.
    pub async fn entries(&self) -> Result<Vec<EntryRow>> {
        self.store.list_entries(None).await
    }

    // ----------------------------------------------------------------- logging --

    async fn reject(&self, entry_iri: &str, reason: &str) -> Result<()> {
        self.logger()
            .emit(
                LogRecord::new(Level::Warn, codes::LIBRARY_ENTRY_REJECTED, TARGET)
                    .with_field("entry_iri", entry_iri.to_string())
                    .with_field("reason", reason.to_string())
                    .with_field("result", "rejected"),
            )
            .await?;
        Ok(())
    }

    async fn emit_dup(&self, entry_iri: &str, existing: &str, hash: &str) -> Result<()> {
        self.logger()
            .emit(
                LogRecord::new(Level::Warn, codes::LIBRARY_DUP_DETECTED, TARGET)
                    .with_field("entry_iri", entry_iri.to_string())
                    .with_field("existing_iri", existing.to_string())
                    .with_field("content_hash", hash.to_string()),
            )
            .await?;
        Ok(())
    }

    async fn emit_orphans(&self, entry_iri: &str, missing: &[String]) -> Result<()> {
        self.logger()
            .emit(
                LogRecord::new(Level::Warn, codes::LIBRARY_ORPHAN_DETECTED, TARGET)
                    .with_field("entry_iri", entry_iri.to_string())
                    .with_field("missing", missing.to_vec()),
            )
            .await?;
        Ok(())
    }
}

/// The ULID IRI a frame instance is stored under.
pub fn instance_iri(id: &Ulid) -> NamedNode {
    iri::instance(id)
}

/// Re-exported so a caller of [`LibraryManager::orphans`] can name the set type.
pub type KnownIris = BTreeSet<String>;

/// The shapes check over a single document, flattening the refusal to its list.
fn shape_check(turtle: &str, label: &str) -> std::result::Result<(), Vec<ShapeViolation>> {
    match validate::validate_turtle_with_shape_bytes(turtle, label) {
        Ok(()) => Ok(()),
        Err(LibraryError::Shape { violations, .. }) => Err(violations),
        Err(_) => Err(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use crate::entry::EntryKind;
    use crate::rdf::{group_lines, kind_of_lines, literal_in};

    #[test]
    fn statements_group_by_subject_in_subject_order() {
        let turtle = r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/case/a> a mm:Case ; mm:title "A" ; mm:version "1"^^<http://www.w3.org/2001/XMLSchema#integer> .
<https://metamind.dev/library/case/b> a mm:Case ; mm:title "B" .
"#;
        let groups = group_lines(turtle).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "<https://metamind.dev/library/case/a>");
        assert_eq!(groups[0].1.len(), 3);
    }

    #[test]
    fn kind_and_literals_are_read_from_the_type_and_predicate() {
        let turtle = r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/technique/t> a mm:LibraryEntry, mm:Technique ;
    mm:title "Decompose" ;
    mm:version "2"^^<http://www.w3.org/2001/XMLSchema#integer> .
"#;
        let groups = group_lines(turtle).unwrap();
        let lines = &groups[0].1;
        assert_eq!(kind_of_lines(lines), Some(EntryKind::Technique));
        assert_eq!(literal_in(lines, "title").as_deref(), Some("Decompose"));
        assert_eq!(literal_in(lines, "version").as_deref(), Some("2"));
    }
}
