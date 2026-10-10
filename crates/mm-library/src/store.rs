//! The library index: the twelve Phase 7 tables, read and written through the
//! Phase 1 [`mm_core::Tabular`] boundary.
//!
//! The `/library` graph is the authority for *what an entry says*; this store is
//! the index an operator surface queries — kind, slug, version, activation score,
//! verification state, fitness. Both are written by [`crate::LibraryManager`] in
//! one operation, and the store never decides anything: the shapes, the duplicate
//! check and the orphan check live in [`crate::validate`], above it.
//!
//! Each entity gets a plain insert and a plain list. There is deliberately **no
//! update path for an entry body or a policy behaviour**: a change is a new row,
//! which is what "immutable versions" means in practice. The two states that do
//! move in place — a skill's verification and a policy's fitness — have exactly one
//! function each, so `logs verify` can correlate them.

use std::sync::Arc;

use mm_core::{Param, Params, Tabular, Ulid, UlidFactory};
use mm_log::Logger;
use mm_store_sqlite::SqliteStore;

use crate::error::{LibraryError, Result};

/// One indexed entry, as the store holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryRow {
    /// The row's ULID.
    pub id: Ulid,
    /// The head IRI.
    pub iri: String,
    /// The versioned IRI.
    pub version_iri: String,
    /// The kind's wire name.
    pub kind: String,
    /// The slug.
    pub slug: String,
    /// The version.
    pub version: i64,
    /// The title.
    pub title: String,
    /// The canonical Turtle.
    pub body_ttl: String,
    /// The content hash.
    pub content_hash: String,
    /// The stored activation blend.
    pub activation_score: f64,
    /// When the row became current.
    pub system_from: String,
}

/// One indexed skill.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillRow {
    /// The versioned IRI.
    pub iri: String,
    /// The name.
    pub name: String,
    /// The description.
    pub description: String,
    /// The artefact reference.
    pub impl_ref: String,
    /// The entrypoint.
    pub entrypoint: String,
    /// The signature.
    pub signature: String,
    /// The test reference.
    pub tests_ref: String,
    /// The verification state.
    pub verification: String,
    /// The version.
    pub version: i64,
}

/// One indexed policy version.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyVersionRow {
    /// The policy's head IRI.
    pub policy_iri: String,
    /// The version.
    pub version: i64,
    /// The parent version, when there is one.
    pub parent_version: Option<i64>,
    /// The scope's stored form.
    pub scope: String,
    /// The activation condition.
    pub activation: String,
    /// The behaviour fragment.
    pub behavior: String,
    /// The confidence.
    pub confidence: f64,
    /// The generation it was evolved in.
    pub generation: i64,
}

/// The SQLite-backed library index.
#[derive(Clone)]
pub struct LibraryStore {
    sqlite: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
}

impl std::fmt::Debug for LibraryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibraryStore")
            .field("path", &self.sqlite.path())
            .finish_non_exhaustive()
    }
}

impl LibraryStore {
    /// Build a store over an open SQLite handle.
    pub fn new(sqlite: SqliteStore, logger: Arc<Logger>, ids: Arc<UlidFactory>) -> Self {
        LibraryStore {
            sqlite,
            logger,
            ids,
        }
    }

    /// The underlying handle.
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

    async fn query(&self, sql: &str, args: Params) -> Result<Vec<serde_json::Value>> {
        Tabular::query_json(&self.sqlite, sql, args)
            .await
            .map_err(Into::into)
    }

    async fn exec(&self, sql: &str, args: Params) -> Result<u64> {
        Ok(Tabular::execute(&self.sqlite, sql, args).await?)
    }

    async fn count(&self, sql: &str) -> Result<i64> {
        let rows = self.query(sql, Vec::new()).await?;
        Ok(rows
            .first()
            .and_then(|row| row.as_object())
            .and_then(|map| map.values().next())
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0))
    }

    fn now() -> String {
        mm_core::Timestamp::now().to_rfc3339()
    }

    // ----------------------------------------------------------------- entries --

    /// The stored content hash for an entry, if one is indexed.
    pub async fn hash_of(&self, version_iri: &str) -> Result<Option<String>> {
        let rows = self
            .query(
                "SELECT content_hash FROM library_entries WHERE version_iri = ?",
                vec![Param::Text(version_iri.to_string())],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("content_hash"))
            .and_then(|value| value.as_str())
            .map(str::to_string))
    }

    /// The entry that already holds a content hash, if any.
    pub async fn entry_with_hash(&self, content_hash: &str) -> Result<Option<String>> {
        let rows = self
            .query(
                "SELECT version_iri FROM library_entries WHERE content_hash = ? \
                 ORDER BY version_iri LIMIT 1",
                vec![Param::Text(content_hash.to_string())],
            )
            .await?;
        Ok(rows
            .first()
            .and_then(|row| row.get("version_iri"))
            .and_then(|value| value.as_str())
            .map(str::to_string))
    }

    /// Index one entry. Returns `true` when the row was written.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_entry(
        &self,
        iri: &str,
        version_iri: &str,
        kind: &str,
        slug: &str,
        version: i64,
        title: &str,
        body_ttl: &str,
        content_hash: &str,
        activation_score: f64,
    ) -> Result<bool> {
        let affected = self
            .exec(
                "INSERT OR IGNORE INTO library_entries \
                 (id, iri, version_iri, kind, slug, version, title, body_ttl, content_hash, \
                  activation_score, system_from, system_to, created_ulid) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
                vec![
                    Param::Text(mm_core::ulid_string(&self.next_id())),
                    Param::Text(iri.to_string()),
                    Param::Text(version_iri.to_string()),
                    Param::Text(kind.to_string()),
                    Param::Text(slug.to_string()),
                    Param::Int(version),
                    Param::Text(title.to_string()),
                    Param::Text(body_ttl.to_string()),
                    Param::Text(content_hash.to_string()),
                    Param::Real(activation_score),
                    Param::Text(Self::now()),
                    Param::Text(mm_core::ulid_string(&self.next_id())),
                ],
            )
            .await?;
        Ok(affected > 0)
    }

    /// Every entry, optionally filtered by kind, ordered by IRI.
    pub async fn list_entries(&self, kind: Option<&str>) -> Result<Vec<EntryRow>> {
        let (sql, args): (&str, Params) = match kind {
            Some(kind) => (
                "SELECT * FROM library_entries WHERE kind = ? ORDER BY version_iri",
                vec![Param::Text(kind.to_string())],
            ),
            None => (
                "SELECT * FROM library_entries ORDER BY version_iri",
                Vec::new(),
            ),
        };
        let rows = self.query(sql, args).await?;
        rows.iter().map(entry_of).collect()
    }

    /// One entry by its versioned IRI.
    pub async fn get_entry(&self, version_iri: &str) -> Result<Option<EntryRow>> {
        let rows = self
            .query(
                "SELECT * FROM library_entries WHERE version_iri = ? LIMIT 1",
                vec![Param::Text(version_iri.to_string())],
            )
            .await?;
        rows.first().map(entry_of).transpose()
    }

    /// Every versioned IRI, for the orphan check.
    pub async fn known_iris(&self) -> Result<std::collections::BTreeSet<String>> {
        let rows = self
            .query(
                "SELECT version_iri FROM library_entries UNION \
                 SELECT iri AS version_iri FROM library_entries UNION \
                 SELECT policy_iri || '@' || version AS version_iri FROM policy_versions",
                Vec::new(),
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|row| row.get("version_iri"))
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect())
    }

    /// The entries' references, as `(version_iri, targets)`, for the orphan check.
    pub async fn references(&self) -> Result<Vec<(String, Vec<String>)>> {
        let entries = self.list_entries(None).await?;
        let mut out = Vec::new();
        for entry in entries {
            out.push((entry.version_iri.clone(), referenced_in(&entry.body_ttl)));
        }
        Ok(out)
    }

    /// How many entries are indexed.
    pub async fn entry_count(&self) -> Result<i64> {
        self.count("SELECT count(*) AS n FROM library_entries")
            .await
    }

    // ------------------------------------------------------------------ skills --

    /// Index a skill.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_skill(
        &self,
        iri: &str,
        name: &str,
        description: &str,
        impl_ref: &str,
        entrypoint: &str,
        signature: &str,
        tests_ref: &str,
        verification: &str,
        version: i64,
    ) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO skills \
             (id, iri, name, description, impl_ref, entrypoint, signature, tests_ref, \
              verification, verified_at, embedding_ref, version, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(iri.to_string()),
                Param::Text(name.to_string()),
                Param::Text(description.to_string()),
                Param::Text(impl_ref.to_string()),
                Param::Text(entrypoint.to_string()),
                Param::Text(signature.to_string()),
                Param::Text(tests_ref.to_string()),
                Param::Text(verification.to_string()),
                Param::Int(version),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Move a skill's verification state. The one in-place update a skill has.
    pub async fn set_verification(&self, iri: &str, verification: &str) -> Result<()> {
        let affected = self
            .exec(
                "UPDATE skills SET verification = ?, verified_at = ? WHERE iri = ?",
                vec![
                    Param::Text(verification.to_string()),
                    Param::Text(Self::now()),
                    Param::Text(iri.to_string()),
                ],
            )
            .await?;
        if affected == 0 {
            return Err(LibraryError::not_found(iri));
        }
        Ok(())
    }

    /// Every skill, ordered by IRI.
    pub async fn list_skills(&self) -> Result<Vec<SkillRow>> {
        let rows = self
            .query("SELECT * FROM skills ORDER BY iri", Vec::new())
            .await?;
        Ok(rows
            .iter()
            .map(|row| SkillRow {
                iri: text_of(row, "iri"),
                name: text_of(row, "name"),
                description: text_of(row, "description"),
                impl_ref: text_of(row, "impl_ref"),
                entrypoint: text_of(row, "entrypoint"),
                signature: text_of(row, "signature"),
                tests_ref: text_of(row, "tests_ref"),
                verification: text_of(row, "verification"),
                version: int_of(row, "version"),
            })
            .collect())
    }

    // --------------------------------------------------------------- workflows --

    /// Index a workflow.
    pub async fn insert_workflow(
        &self,
        iri: &str,
        name: &str,
        steps_json: &str,
        induced_from: &str,
        version: i64,
    ) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO workflows \
             (id, iri, name, steps_json, induced_from, version, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(iri.to_string()),
                Param::Text(name.to_string()),
                Param::Text(steps_json.to_string()),
                Param::Text(induced_from.to_string()),
                Param::Int(version),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    // ------------------------------------------------------------------- cases --

    /// Index a case.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_case(
        &self,
        iri: &str,
        kind: &str,
        problem_ttl: &str,
        solution_ttl: &str,
        outcome_ttl: &str,
        outcome_quality: f64,
        transferability: f64,
        evidence_quality: f64,
    ) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO cases \
             (id, iri, kind, problem_ttl, solution_ttl, outcome_ttl, outcome_quality, \
              transferability, evidence_quality, embedding_ref, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(iri.to_string()),
                Param::Text(kind.to_string()),
                Param::Text(problem_ttl.to_string()),
                Param::Text(solution_ttl.to_string()),
                Param::Text(outcome_ttl.to_string()),
                Param::Real(outcome_quality),
                Param::Real(transferability),
                Param::Real(evidence_quality),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Record one structure-mapping correspondence.
    pub async fn insert_correspondence(
        &self,
        case_iri: &str,
        source_elem: &str,
        target_elem: &str,
        relation: &str,
        score: f64,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO case_map (id, case_iri, source_elem, target_elem, relation, score, \
             created_ulid) VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(case_iri.to_string()),
                Param::Text(source_elem.to_string()),
                Param::Text(target_elem.to_string()),
                Param::Text(relation.to_string()),
                Param::Real(score),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    // ---------------------------------------------------------------- insights --

    /// Index an insight with its vote counts.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_insight(
        &self,
        iri: &str,
        text: &str,
        kind: &str,
        evidence_json: &str,
        upvotes: i64,
        downvotes: i64,
    ) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO insights \
             (id, iri, text, kind, evidence, upvotes, downvotes, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(iri.to_string()),
                Param::Text(text.to_string()),
                Param::Text(kind.to_string()),
                Param::Text(evidence_json.to_string()),
                Param::Int(upvotes),
                Param::Int(downvotes),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    /// Every insight, ordered by IRI.
    pub async fn list_insights(&self) -> Result<Vec<(String, String, String, i64, i64)>> {
        let rows = self
            .query(
                "SELECT iri, text, kind, upvotes, downvotes FROM insights ORDER BY iri",
                Vec::new(),
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| {
                (
                    text_of(row, "iri"),
                    text_of(row, "text"),
                    text_of(row, "kind"),
                    int_of(row, "upvotes"),
                    int_of(row, "downvotes"),
                )
            })
            .collect())
    }

    // ---------------------------------------------------------------- policies --

    /// Ensure the policy head row exists and return its current head version.
    pub async fn ensure_policy(&self, head_iri: &str, name: &str) -> Result<i64> {
        let rows = self
            .query(
                "SELECT head_version FROM policies WHERE iri = ?",
                vec![Param::Text(head_iri.to_string())],
            )
            .await?;
        if let Some(row) = rows.first() {
            return Ok(int_of(row, "head_version"));
        }
        self.exec(
            "INSERT INTO policies (id, iri, name, head_version, created_ulid) VALUES (?, ?, ?, 0, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(head_iri.to_string()),
                Param::Text(name.to_string()),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(0)
    }

    /// Insert an immutable policy version and advance the head.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_policy_version(
        &self,
        head_iri: &str,
        version: i64,
        parent_version: Option<i64>,
        scope: &str,
        activation: &str,
        behavior: &str,
        evidence_ttl: &str,
        confidence: f64,
        content_hash: &str,
        generation: i64,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO policy_versions \
             (id, policy_iri, version, parent_version, scope_ttl, activation_ttl, behavior_ttl, \
              evidence_ttl, confidence, content_hash, generation, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(head_iri.to_string()),
                Param::Int(version),
                parent_version.map_or(Param::Null, Param::Int),
                Param::Text(scope.to_string()),
                Param::Text(activation.to_string()),
                Param::Text(behavior.to_string()),
                Param::Text(evidence_ttl.to_string()),
                Param::Real(confidence),
                Param::Text(content_hash.to_string()),
                Param::Int(generation),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        self.exec(
            "UPDATE policies SET head_version = ? WHERE iri = ?",
            vec![Param::Int(version), Param::Text(head_iri.to_string())],
        )
        .await?;
        Ok(())
    }

    /// Every version of a policy, oldest first.
    pub async fn policy_history(&self, head_iri: &str) -> Result<Vec<PolicyVersionRow>> {
        let rows = self
            .query(
                "SELECT * FROM policy_versions WHERE policy_iri = ? ORDER BY version",
                vec![Param::Text(head_iri.to_string())],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| PolicyVersionRow {
                policy_iri: text_of(row, "policy_iri"),
                version: int_of(row, "version"),
                parent_version: row.get("parent_version").and_then(|v| v.as_i64()),
                scope: text_of(row, "scope_ttl"),
                activation: text_of(row, "activation_ttl"),
                behavior: text_of(row, "behavior_ttl"),
                confidence: row
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                generation: int_of(row, "generation"),
            })
            .collect())
    }

    /// The head version of a policy, or 0 when it has none.
    pub async fn head_version(&self, head_iri: &str) -> Result<i64> {
        let rows = self
            .query(
                "SELECT head_version FROM policies WHERE iri = ?",
                vec![Param::Text(head_iri.to_string())],
            )
            .await?;
        Ok(rows
            .first()
            .map(|row| int_of(row, "head_version"))
            .unwrap_or(0))
    }

    /// Record a fitness update for one version.
    pub async fn record_fitness(
        &self,
        head_iri: &str,
        version: i64,
        trials: i64,
        successes: i64,
        mean_utility: f64,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO policy_fitness \
             (id, policy_iri, version, trials, successes, mean_utility, updated_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(policy_iri, version) DO UPDATE SET \
               trials = excluded.trials, successes = excluded.successes, \
               mean_utility = excluded.mean_utility, updated_ulid = excluded.updated_ulid",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(head_iri.to_string()),
                Param::Int(version),
                Param::Int(trials),
                Param::Int(successes),
                Param::Real(mean_utility),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    /// The fitness of one policy version.
    pub async fn fitness_of(
        &self,
        head_iri: &str,
        version: i64,
    ) -> Result<Option<(i64, i64, f64)>> {
        let rows = self
            .query(
                "SELECT trials, successes, mean_utility FROM policy_fitness \
                 WHERE policy_iri = ? AND version = ?",
                vec![Param::Text(head_iri.to_string()), Param::Int(version)],
            )
            .await?;
        Ok(rows.first().map(|row| {
            (
                int_of(row, "trials"),
                int_of(row, "successes"),
                row.get("mean_utility")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
            )
        }))
    }

    /// Record one candidate in the population.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_population_row(
        &self,
        generation: i64,
        head_iri: &str,
        version: i64,
        parent_a: Option<i64>,
        parent_b: Option<i64>,
        mutation: Option<&str>,
        fitness: Option<f64>,
        retained: bool,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO policy_population \
             (id, generation, policy_iri, version, parent_a, parent_b, mutation, fitness, \
              retained, created_ulid) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Int(generation),
                Param::Text(head_iri.to_string()),
                Param::Int(version),
                parent_a.map_or(Param::Null, |v| Param::Text(v.to_string())),
                parent_b.map_or(Param::Null, |v| Param::Text(v.to_string())),
                mutation.map_or(Param::Null, |m| Param::Text(m.to_string())),
                fitness.map_or(Param::Null, Param::Real),
                Param::Int(i64::from(retained)),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    /// How many population rows a generation retained.
    pub async fn retained_in_generation(&self, generation: i64) -> Result<i64> {
        let rows = self
            .query(
                "SELECT count(*) AS n FROM policy_population WHERE generation = ? AND retained = 1",
                vec![Param::Int(generation)],
            )
            .await?;
        Ok(rows.first().map(|row| int_of(row, "n")).unwrap_or(0))
    }

    // ------------------------------------------------------------------ frames --

    /// Index a composed frame instance.
    pub async fn insert_frame_instance(
        &self,
        id: &Ulid,
        frame_iri: &str,
        episode: &Ulid,
        parents_json: &str,
        slots_json: &str,
        missing_json: &str,
    ) -> Result<()> {
        self.exec(
            "INSERT OR REPLACE INTO frame_instances \
             (id, frame_iri, episode_ulid, parents_json, slots_json, missing_json, created_ulid) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(id)),
                Param::Text(frame_iri.to_string()),
                Param::Text(mm_core::ulid_string(episode)),
                Param::Text(parents_json.to_string()),
                Param::Text(slots_json.to_string()),
                Param::Text(missing_json.to_string()),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }

    /// The missing slots of a recorded instance.
    pub async fn frame_instance_missing(&self, id: &Ulid) -> Result<Option<Vec<String>>> {
        let rows = self
            .query(
                "SELECT missing_json FROM frame_instances WHERE id = ?",
                vec![Param::Text(mm_core::ulid_string(id))],
            )
            .await?;
        match rows.first().and_then(|row| row.get("missing_json")) {
            Some(serde_json::Value::String(text)) => {
                Ok(Some(serde_json::from_str(text).map_err(|e| {
                    LibraryError::Codec(format!("missing_json: {e}"))
                })?))
            }
            _ => Ok(None),
        }
    }

    // ----------------------------------------------------------- applicability --

    /// Record a ranking run.
    pub async fn insert_applicability_run(
        &self,
        episode: &Ulid,
        state_hash: &str,
        ranked_json: &str,
    ) -> Result<()> {
        self.exec(
            "INSERT INTO applicability_runs \
             (id, episode_ulid, state_hash, ranked_json, created_ulid) VALUES (?, ?, ?, ?, ?)",
            vec![
                Param::Text(mm_core::ulid_string(&self.next_id())),
                Param::Text(mm_core::ulid_string(episode)),
                Param::Text(state_hash.to_string()),
                Param::Text(ranked_json.to_string()),
                Param::Text(mm_core::ulid_string(&self.next_id())),
            ],
        )
        .await?;
        Ok(())
    }
}

/// The IRIs a Turtle document references through the library's reference
/// predicates.
///
/// One pass over the parsed triples, so the orphan check sees exactly the fields
/// the shapes care about and nothing else.
pub fn referenced_in(turtle: &str) -> Vec<String> {
    const REFERENCE_PREDICATES: [&str; 6] = [
        "evidence",
        "example",
        "counterexample",
        "testedBy",
        "parentFrame",
        "parentPolicy",
    ];
    let Ok(graph) = rdf_codec::io::parse_turtle(turtle) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for triple in graph.iter() {
        let predicate = triple.predicate.as_str();
        let local = predicate.rsplit('#').next().unwrap_or(predicate);
        if !REFERENCE_PREDICATES.contains(&local) {
            continue;
        }
        let rendered = triple.object.to_string();
        out.push(
            rendered
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_string(),
        );
    }
    out.sort();
    out.dedup();
    out
}

fn text_of(row: &serde_json::Value, key: &str) -> String {
    row.get(key)
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn int_of(row: &serde_json::Value, key: &str) -> i64 {
    row.get(key).and_then(|value| value.as_i64()).unwrap_or(0)
}

fn entry_of(row: &serde_json::Value) -> Result<EntryRow> {
    let id = text_of(row, "id");
    Ok(EntryRow {
        id: mm_core::id::parse_ulid(&id)?,
        iri: text_of(row, "iri"),
        version_iri: text_of(row, "version_iri"),
        kind: text_of(row, "kind"),
        slug: text_of(row, "slug"),
        version: int_of(row, "version"),
        title: text_of(row, "title"),
        body_ttl: text_of(row, "body_ttl"),
        content_hash: text_of(row, "content_hash"),
        activation_score: row
            .get("activation_score")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0),
        system_from: text_of(row, "system_from"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_iris_are_the_reference_predicates_only() {
        let turtle = r#"
@prefix mm: <https://metamind.dev/ontology#> .
<https://metamind.dev/library/technique/t> a mm:Technique ;
    mm:evidence <https://metamind.dev/library/case/a> ;
    mm:example <https://metamind.dev/library/case/b> ;
    mm:domain "software" .
"#;
        assert_eq!(
            referenced_in(turtle),
            vec![
                "https://metamind.dev/library/case/a".to_string(),
                "https://metamind.dev/library/case/b".to_string(),
            ]
        );
    }

    #[test]
    fn a_document_that_does_not_parse_references_nothing() {
        assert!(referenced_in("this is not turtle").is_empty());
    }
}
