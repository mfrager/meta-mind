//! The numeric self-model: Actual, Model and Ideal, and the gaps between them.
//!
//! Design §62 asks the being to measure itself. The measurement is three scalars over one
//! run and the three pairwise gaps between them:
//!
//! * **Actual** — what the run *did*: the fraction of the ten stages that completed.
//! * **Model** — what the run *says it produced*: the fraction of stages that registered
//!   an artifact. This is deliberately not the same as Actual. A stage can claim an
//!   artifact and fail, and the difference between the claim and the outcome is exactly
//!   what a self-model is for.
//! * **Ideal** — what the declared principles *demand*: the fraction of the run's
//!   invariants that held. The four are named in [`RunSelfModel::invariants`].
//!
//! The three gaps are `|Actual − Model|`, `|Actual − Ideal|` and `|Model − Ideal|`, and
//! they are what the report stores and what `ontology/shapes/self_model.ttl` requires to
//! be exactly three. A report whose number came from a narrative rather than from rows
//! would not be checkable, which is why every input here is read from the store.
//!
//! Every scalar comes from *this run's* rows and events, so the same run measured twice
//! produces the same report; nothing here consults the wall clock or a model.

use std::sync::Arc;

use async_trait::async_trait;
use mm_core::{Param, Tabular, Timestamp, Ulid, UlidFactory};
use mm_log::{codes, Level, Logger};
use mm_store_graph::GraphStore;
use mm_store_sqlite::SqliteStore;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{LoopError, Result};
use crate::rdf;
use crate::LoopStage;

/// One named gap.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Divergence {
    /// Which comparison it is: `actual_model`, `actual_ideal` or `model_ideal`.
    pub dimension: String,
    /// The absolute gap, in `[0,1]`.
    pub value: f32,
}

/// One measurement of the being against itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelfModelReport {
    /// Its ULID.
    #[serde(with = "mm_core::serde_ulid")]
    pub id: Ulid,
    /// The run it measured.
    #[serde(with = "mm_core::serde_ulid")]
    pub run_id: Ulid,
    /// `|Actual − Model|`.
    pub actual_model: f32,
    /// `|Actual − Ideal|`.
    pub actual_ideal: f32,
    /// `|Model − Ideal|`.
    pub model_ideal: f32,
    /// The three gaps, named. Exactly three, one per field above.
    pub dims: Vec<Divergence>,
}

impl SelfModelReport {
    /// The three pairwise gaps of three scalars.
    ///
    /// Pure, so the fixture with a known gap can be checked against it without a store:
    /// `bench/self_model/gap_01.json` carries `actual`, `model`, `ideal` and the gaps they
    /// must produce, and a unit test reads both.
    pub fn measure_from(id: Ulid, run_id: Ulid, actual: f32, model: f32, ideal: f32) -> Self {
        let gap = |a: f32, b: f32| (a - b).abs().clamp(0.0, 1.0);
        let actual_model = gap(actual, model);
        let actual_ideal = gap(actual, ideal);
        let model_ideal = gap(model, ideal);
        SelfModelReport {
            id,
            run_id,
            actual_model,
            actual_ideal,
            model_ideal,
            dims: vec![
                Divergence {
                    dimension: "actual_model".to_string(),
                    value: actual_model,
                },
                Divergence {
                    dimension: "actual_ideal".to_string(),
                    value: actual_ideal,
                },
                Divergence {
                    dimension: "model_ideal".to_string(),
                    value: model_ideal,
                },
            ],
        }
    }

    /// The largest gap, and the dimension it is on. What a caller reports as "the worst
    /// thing the run found about itself".
    pub fn worst(&self) -> (&str, f32) {
        self.dims
            .iter()
            .max_by(|a, b| a.value.partial_cmp(&b.value).expect("gaps are finite"))
            .map(|d| (d.dimension.as_str(), d.value))
            .unwrap_or(("none", 0.0))
    }
}

/// What the self-model does.
#[async_trait]
pub trait SelfModel {
    /// Measure one run.
    async fn measure(&self, run: Ulid) -> Result<SelfModelReport>;
}

/// The self-model over the loop's rows and the run's events.
pub struct RunSelfModel {
    store: SqliteStore,
    logger: Arc<Logger>,
    ids: Arc<UlidFactory>,
    graph: Option<Arc<GraphStore>>,
}

impl RunSelfModel {
    /// A self-model over the kernel's tabular store.
    pub fn new(store: SqliteStore, logger: Arc<Logger>, ids: Arc<UlidFactory>) -> Self {
        RunSelfModel {
            store,
            logger,
            ids,
            graph: None,
        }
    }

    /// Attach the graph store, so a report is mirrored into `/self`.
    pub fn with_graph(mut self, graph: Arc<GraphStore>) -> Self {
        self.graph = Some(graph);
        self
    }

    /// The three scalars of a run: `(actual, model, ideal_clauses_held, ideal_clauses)`.
    async fn scalars(&self, run: &Ulid) -> Result<(f32, f32, usize, usize)> {
        let run_text = mm_core::ulid_string(run);
        let rows = self
            .store
            .query_json(
                "SELECT idx, stage, outcome FROM loop_iterations WHERE run_id = ? ORDER BY idx",
                vec![Param::Text(run_text.clone())],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's stages: {e}")))?;
        let total = rows.len();
        if total == 0 {
            return Err(LoopError::validation(
                "run_id",
                format!("{run_text} has no recorded stage; there is nothing to measure"),
            ));
        }
        let ok = rows
            .iter()
            .filter(|row| row["outcome"].as_str() == Some("ok"))
            .count();

        // What the run says it produced. A stage's artifact is recorded in the event it
        // appended, so the claim is read from the log rather than from a counter: a claim
        // this module could not check against the log would be the narrative the phase
        // forbids.
        let claims = self.claims(run).await?;
        let model = if total == 0 {
            0.0
        } else {
            (claims as f32 / total as f32).clamp(0.0, 1.0)
        };
        let actual = (ok as f32 / total as f32).clamp(0.0, 1.0);

        let (held, clauses) = self.invariants(run, &rows).await?;
        Ok((actual, model, held, clauses))
    }

    /// How many of the run's stages claimed an artifact.
    async fn claims(&self, run: &Ulid) -> Result<usize> {
        let run_text = mm_core::ulid_string(run);
        let rows = self
            .store
            .query_json(
                "SELECT payload FROM events WHERE correlation = ? ORDER BY seq",
                vec![Param::Text(run_text)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run's events: {e}")))?;
        let mut claims = 0;
        for row in rows {
            let Some(text) = row["payload"].as_str() else {
                continue;
            };
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(text) else {
                continue;
            };
            let artifact = payload
                .get("artifact")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !artifact.trim().is_empty() {
                claims += 1;
            }
        }
        Ok(claims)
    }

    /// The four named principles, and how many held.
    ///
    /// The clauses are the phase's own invariants, read from rows: the run reached
    /// `completed`; no stage failed; the stages are the ten, once each, in order; and the
    /// run recorded a digest. A clause is checked against the store, never assumed.
    async fn invariants(&self, run: &Ulid, stages: &[serde_json::Value]) -> Result<(usize, usize)> {
        let run_text = mm_core::ulid_string(run);
        let status = self
            .store
            .query_json(
                "SELECT status, digest FROM loop_runs WHERE id = ?",
                vec![Param::Text(run_text)],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot read the run: {e}")))?;
        let Some(row) = status.first() else {
            return Err(LoopError::validation(
                "run_id",
                format!("no run {run} is recorded"),
            ));
        };
        let completed = row["status"].as_str() == Some("completed");
        let digest_recorded = row["digest"].as_str().is_some_and(|d| !d.is_empty());
        let none_failed = stages
            .iter()
            .all(|stage| stage["outcome"].as_str() != Some("failed"));
        let ordered = stages.len() == LoopStage::ALL.len()
            && stages.iter().enumerate().all(|(idx, stage)| {
                stage["stage"].as_str() == Some(LoopStage::at(idx).map_or("", |s| s.as_str()))
            });
        let clauses = [
            ("run_completed", completed),
            ("no_stage_failed", none_failed),
            ("stages_ordered", ordered),
            ("digest_recorded", digest_recorded),
        ];
        let held = clauses.iter().filter(|(_, held)| *held).count();
        Ok((held, clauses.len()))
    }

    /// Write a report's rows.
    pub async fn persist(&self, report: &SelfModelReport) -> Result<()> {
        let id = mm_core::ulid_string(&report.id);
        self.store
            .execute(
                "INSERT INTO self_model_reports \
                 (id, run_id, actual_model, actual_ideal, model_ideal, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
                vec![
                    Param::Text(id.clone()),
                    Param::Text(mm_core::ulid_string(&report.run_id)),
                    Param::Real(f64::from(report.actual_model)),
                    Param::Real(f64::from(report.actual_ideal)),
                    Param::Real(f64::from(report.model_ideal)),
                    Param::Text(Timestamp::now().to_rfc3339()),
                ],
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the report: {e}")))?;
        for divergence in &report.dims {
            self.store
                .execute(
                    "INSERT INTO divergence_metrics (id, report_id, dimension, value) \
                     VALUES (?, ?, ?, ?)",
                    vec![
                        Param::Text(mm_core::ulid_string(&self.ids.next())),
                        Param::Text(id.clone()),
                        Param::Text(divergence.dimension.clone()),
                        Param::Real(f64::from(divergence.value)),
                    ],
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot record the divergence: {e}")))?;
        }
        self.logger
            .audit(
                Level::Info,
                codes::SELF_MODEL_REPORT,
                crate::TARGET,
                Some(report.id),
                json!({
                    "run_id": mm_core::ulid_string(&report.run_id),
                    "actual_model": report.actual_model,
                    "actual_ideal": report.actual_ideal,
                    "model_ideal": report.model_ideal,
                }),
            )
            .await
            .map_err(|e| LoopError::Store(format!("cannot record the report: {e}")))?;
        for divergence in &report.dims {
            self.logger
                .emit(
                    mm_log::LogRecord::new(
                        Level::Info,
                        codes::SELF_MODEL_DIVERGENCE,
                        crate::TARGET,
                    )
                    .with_trace(report.id)
                    .with_field("report_id", id.clone())
                    .with_field("dimension", divergence.dimension.clone())
                    .with_field("value", divergence.value),
                )
                .await
                .map_err(|e| LoopError::Store(format!("cannot record the divergence: {e}")))?;
        }
        // The mirror count is not this function's business: the report is persisted and
        // mirrored, and how many triples that took belongs to `mirror`'s own callers.
        self.mirror(report).await.map(|_| ())
    }

    /// Mirror a report into `/self`.
    pub async fn mirror(&self, report: &SelfModelReport) -> Result<usize> {
        let Some(graph) = &self.graph else {
            return Ok(0);
        };
        let quads = rdf::self_model_quads(report);
        let turtle = rdf::turtle(&quads);
        graph
            .handle()
            .insert_turtle(rdf::SELF_GRAPH, &turtle)
            .await
            .map_err(LoopError::from)
    }
}

#[async_trait]
impl SelfModel for RunSelfModel {
    /// Measure, persist and mirror a run.
    async fn measure(&self, run: Ulid) -> Result<SelfModelReport> {
        let (actual, model, held, clauses) = self.scalars(&run).await?;
        let ideal = if clauses == 0 {
            0.0
        } else {
            (held as f32 / clauses as f32).clamp(0.0, 1.0)
        };
        let report = SelfModelReport::measure_from(self.ids.next(), run, actual, model, ideal);
        self.persist(&report).await?;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mm_core::Config;

    fn fixture_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("bench/self_model/gap_01.json")
    }

    /// The fixture's three scalars must produce the fixture's three gaps.
    ///
    /// This is the plan's "a fixture with a known gap yields the expected divergence
    /// within tolerance", and it is checked against the committed fixture rather than
    /// against numbers repeated in the test, so the two cannot drift apart.
    #[test]
    fn the_seeded_gap_fixture_measures_to_its_expected_divergences() {
        let text = std::fs::read_to_string(fixture_path()).expect("bench/self_model/gap_01.json");
        let fixture: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        let actual = fixture["actual"].as_f64().expect("actual") as f32;
        let model = fixture["model"].as_f64().expect("model") as f32;
        let ideal = fixture["ideal"].as_f64().expect("ideal") as f32;
        let tolerance = fixture["tolerance"].as_f64().expect("tolerance") as f32;

        let report = SelfModelReport::measure_from(
            Ulid::from_parts(1_700_000_000_000, 1),
            Ulid::from_parts(1_700_000_000_000, 2),
            actual,
            model,
            ideal,
        );
        let expected = &fixture["expected"];
        for (field, value) in [
            ("actual_model", report.actual_model),
            ("actual_ideal", report.actual_ideal),
            ("model_ideal", report.model_ideal),
        ] {
            let want = expected[field].as_f64().expect(field) as f32;
            assert!(
                (value - want).abs() <= tolerance,
                "{field}: measured {value}, fixture says {want}"
            );
        }
        assert_eq!(report.dims.len(), 3);
        assert_eq!(
            report
                .dims
                .iter()
                .map(|d| d.dimension.as_str())
                .collect::<Vec<_>>(),
            vec!["actual_model", "actual_ideal", "model_ideal"]
        );
        assert_eq!(report.worst().0, "model_ideal");
    }

    #[test]
    fn identical_scalars_measure_as_no_divergence() {
        let report = SelfModelReport::measure_from(
            Ulid::from_parts(1, 1),
            Ulid::from_parts(1, 2),
            0.9,
            0.9,
            0.9,
        );
        assert_eq!(report.actual_model, 0.0);
        assert_eq!(report.actual_ideal, 0.0);
        assert_eq!(report.model_ideal, 0.0);
        assert_eq!(report.worst().1, 0.0);
    }

    #[test]
    fn gaps_are_absolute_and_bounded() {
        let report = SelfModelReport::measure_from(
            Ulid::from_parts(1, 1),
            Ulid::from_parts(1, 2),
            1.0,
            0.4,
            0.6,
        );
        assert!((report.actual_model - 0.6).abs() < 1e-6);
        assert!((report.actual_ideal - 0.4).abs() < 1e-6);
        assert!((report.model_ideal - 0.2).abs() < 1e-6);
    }

    /// A real measurement over a run whose rows say it did nine of ten stages.
    #[tokio::test]
    async fn a_run_is_measured_from_its_own_rows() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::for_data_dir(dir.path().join("data"));
        std::fs::create_dir_all(&cfg.store.data_dir).unwrap();
        let store = SqliteStore::open(&cfg.store.sqlite_file).await.unwrap();
        store.migrate().await.unwrap();
        let logger =
            Arc::new(Logger::from_config(&cfg.log, Some(Arc::new(store.clone()))).unwrap());
        let ids = Arc::new(UlidFactory::new());
        let run = ids.next();
        let run_text = mm_core::ulid_string(&run);
        store
            .execute(
                "INSERT INTO loop_runs (id, goal_ulid, goal_text, novel, status, config_hash, \
                 code_version, digest, started_at) VALUES (?, ?, ?, 1, 'running', 'c', 'v', '', \
                 '2026-10-08T00:00:00Z')",
                vec![
                    Param::Text(run_text.clone()),
                    Param::Text(mm_core::ulid_string(&ids.next())),
                    Param::Text("a goal".to_string()),
                ],
            )
            .await
            .unwrap();
        for stage in LoopStage::ALL {
            let outcome = if stage == LoopStage::Shadow {
                "refused"
            } else {
                "ok"
            };
            store
                .execute(
                    "INSERT INTO loop_iterations (id, run_id, idx, stage, outcome, detail, \
                     started_at) VALUES (?, ?, ?, ?, ?, 'd', '2026-10-08T00:00:00Z')",
                    vec![
                        Param::Text(mm_core::ulid_string(&ids.next())),
                        Param::Text(run_text.clone()),
                        Param::Int(stage.idx() as i64),
                        Param::Text(stage.as_str().to_string()),
                        Param::Text(outcome.to_string()),
                    ],
                )
                .await
                .unwrap();
        }

        let model = RunSelfModel::new(store.clone(), logger.clone(), ids.clone());
        let report = model.measure(run).await.expect("a report");
        // Nine of ten stages completed, no stage claimed an artifact, one invariant (the
        // run has not reached `completed`) failed.
        assert!((report.dims[0].value - 0.9).abs() < 1e-6, "{report:?}");
        let rows = store
            .query_json(
                "SELECT count(*) AS n FROM divergence_metrics WHERE report_id = ?",
                vec![Param::Text(mm_core::ulid_string(&report.id))],
            )
            .await
            .unwrap();
        assert_eq!(rows[0]["n"].as_i64(), Some(3));
        let runs = store
            .query_json(
                "SELECT count(*) AS n FROM self_model_reports WHERE run_id = ?",
                vec![Param::Text(run_text)],
            )
            .await
            .unwrap();
        assert_eq!(runs[0]["n"].as_i64(), Some(1));
    }
}
