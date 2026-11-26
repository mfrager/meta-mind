//! The tools phase 5 turns on. Each one calls the library the CLI calls, on the
//! stores this process already opened.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_core::store::Tabular;
use mm_core::ulid_string;
use mm_epistemic::{Claim, ClaimKind, EpistemicEngine, Proposition, SqliteEpistemicStore};
use mm_library::experience::{self, Insight, Trajectory};
use mm_library::{
    declaratives_from_turtle, rank, selectable, ApplicabilityState, LibraryStore, Skill,
    Verification,
};
use mm_runtime::{
    BudgetEnvelope, ChangeSetDesignWriter, CompositeDebtScanner, LadderCollector, LoopController,
    LoopGoal, LoopServices, ManifestModuleLoader, ModuleLoader, ModuleVersion,
};
use serde_json::{json, Value};

use crate::state::AppState;

use super::super::exec::ToolResult;

pub async fn loop_goal_allowed(app: &AppState, input: &Value) -> Result<(), String> {
    let raw = text(input, "goal");
    let raw = if raw.is_empty() {
        text(input, "path")
    } else {
        raw
    };
    if raw.is_empty() {
        return Err("a goal file under bench/ or a goal id is required".into());
    }
    if let Ok(id) = mm_core::id::parse_ulid(&raw) {
        let Some(being) = &app.being else {
            return Err("the being store did not open".into());
        };
        let guard = being.lock().await;
        if guard.goals().get(&id).is_none() {
            return Err(format!("no goal {}", ulid_string(&id)));
        }
        return Ok(());
    }
    let path = Path::new(&raw);
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        app.cfg.root.join(path)
    };
    let bench = app.cfg.root.join("bench");
    if !full.starts_with(&bench) {
        return Err("the path is outside bench".into());
    }
    Ok(())
}

pub async fn epistemic_ingest(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(graph_closed(app));
    };
    let kind = ClaimKind::parse(&text(input, "kind")).unwrap_or(ClaimKind::Fact);
    let proposition = match Proposition::new(
        &text(input, "subject"),
        &text(input, "predicate"),
        &text(input, "object"),
        false,
    ) {
        Ok(proposition) => proposition,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let claim = match Claim::of_kind(app.ids.next(), kind, proposition, 0.5) {
        Ok(claim) => claim,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let store = Arc::new(SqliteEpistemicStore::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    ));
    let engine = EpistemicEngine::new(store, graph.handle().clone());
    match engine.ingest(&claim, &[]).await {
        Ok(id) => ToolResult::done(
            format!("epistemic ingest: {}", ulid_string(&id)),
            json!({ "id": ulid_string(&id), "kind": kind.as_str() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn epistemic_show(app: &AppState) -> ToolResult {
    let claims = count(app, "SELECT COUNT(*) AS n FROM claims").await;
    let assumptions = count(app, "SELECT COUNT(*) AS n FROM assumptions").await;
    ToolResult::done(
        format!("epistemic claims {claims}"),
        json!({ "claims": claims, "assumptions": assumptions }),
    )
}

pub async fn library_applicable(app: &AppState, input: &Value) -> ToolResult {
    let store = library_store(app);
    let rows = match store.list_entries(None).await {
        Ok(rows) => rows,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let mut entries = Vec::new();
    for row in &rows {
        if let Ok(mut decoded) = declaratives_from_turtle(&row.body_ttl) {
            entries.append(&mut decoded);
        }
    }
    let goal = {
        let goal = text(input, "goal");
        if goal.is_empty() {
            "the operator asked what applies".into()
        } else {
            goal
        }
    };
    let state = ApplicabilityState::new(&goal, mm_core::Timestamp::now());
    let candidates: Vec<_> = selectable(&entries).into_iter().cloned().collect();
    let ranked = rank(&candidates, &state, 5);
    ToolResult::done(
        format!("{} library entries apply", ranked.len()),
        json!({
            "ranked": ranked.iter().map(|entry| json!({
                "iri": entry.iri,
                "title": entry.title,
                "score": entry.score.total,
            })).collect::<Vec<_>>(),
        }),
    )
}

pub async fn policy_history(app: &AppState, input: &Value) -> ToolResult {
    let policy = text(input, "policy");
    if policy.is_empty() {
        return ToolResult::fail("a policy iri is required");
    }
    match library_store(app).policy_history(&policy).await {
        Ok(rows) if rows.is_empty() => ToolResult::fail(format!("no such policy: {policy}")),
        Ok(rows) => ToolResult::done(
            format!("{} policy version(s)", rows.len()),
            json!({ "versions": rows.len() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn skill_retrieve(app: &AppState, input: &Value) -> ToolResult {
    let query = text(input, "query");
    let rows = match library_store(app).list_skills().await {
        Ok(rows) => rows,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let mut skills = Vec::new();
    for row in &rows {
        let Some(verification) = Verification::parse(&row.verification) else {
            continue;
        };
        skills.push(Skill {
            iri: mm_core::NamedNode::new_unchecked(row.iri.clone()),
            name: row.name.clone(),
            description: row.description.clone(),
            impl_ref: row.impl_ref.clone(),
            entrypoint: row.entrypoint.clone(),
            signature: row.signature.clone(),
            tests_ref: row.tests_ref.clone(),
            verification,
            verified_at: None,
            embedding_ref: None,
            version: row.version.max(1) as u32,
        });
    }
    let hits = mm_library::skill::retrieve(&skills, &query, 5);
    let summary = if hits.is_empty() {
        format!("no verified skill matches {query:?}")
    } else {
        format!("{} skill hits", hits.len())
    };
    ToolResult::done(
        summary,
        json!({
            "hits": hits.iter().map(|hit| json!({
                "iri": hit.iri,
                "name": hit.name,
                "score": hit.score,
            })).collect::<Vec<_>>(),
        }),
    )
}

pub async fn case_retrieve(app: &AppState, input: &Value) -> ToolResult {
    let query = text(input, "query").to_ascii_lowercase();
    let rows = match library_store(app).list_entries(Some("case")).await {
        Ok(rows) => rows,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let hits: Vec<_> = rows
        .iter()
        .filter(|row| {
            query.is_empty()
                || row.title.to_ascii_lowercase().contains(&query)
                || row.slug.to_ascii_lowercase().contains(&query)
        })
        .take(5)
        .map(|row| json!({ "iri": row.iri, "title": row.title }))
        .collect();
    ToolResult::done(format!("{} case hits", hits.len()), json!({ "hits": hits }))
}

pub async fn frame_missing(app: &AppState, input: &Value) -> ToolResult {
    let id = text(input, "id");
    let Ok(ulid) = mm_core::id::parse_ulid(&id) else {
        return ToolResult::fail("a frame instance id is required");
    };
    match library_store(app).frame_instance_missing(&ulid).await {
        Ok(Some(slots)) if slots.is_empty() => {
            ToolResult::done("no missing slots", json!({ "slots": [] }))
        }
        Ok(Some(slots)) => ToolResult::done(
            format!("{} missing slots", slots.len()),
            json!({ "slots": slots }),
        ),
        Ok(None) => ToolResult::fail(format!(
            "no frame instance {id}; compose one with `mm-cli frame compose`"
        )),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn calibration_show(app: &AppState) -> ToolResult {
    let rows = match app
        .sqlite
        .query_json(
            "SELECT decision_class, brier, n FROM calibration ORDER BY decision_class",
            mm_core::Params::new(),
        )
        .await
    {
        Ok(rows) => rows,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    ToolResult::done(
        format!("{} calibration rows", rows.len()),
        json!({ "rows": rows }),
    )
}

pub fn conformance_run() -> ToolResult {
    let core = match mm_decision::RulesCore::embedded() {
        Ok(core) => core,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let findings = mm_decision::run_conformance(&core);
    let passed = mm_decision::findings_passed(&findings);
    let summary = mm_decision::conformance_summary(&core);
    let mut result = ToolResult::done(
        summary,
        json!({ "passed": passed, "findings": findings.len() }),
    );
    result.ok = passed;
    result
}

pub async fn calibrate_score(app: &AppState) -> ToolResult {
    calibration_show(app).await
}

pub async fn loop_run(app: &AppState, input: &Value) -> ToolResult {
    if let Err(reason) = loop_goal_allowed(app, input).await {
        return ToolResult::fail(reason);
    }
    let raw = {
        let goal = text(input, "goal");
        if goal.is_empty() {
            text(input, "path")
        } else {
            goal
        }
    };
    let goal = if let Ok(id) = mm_core::id::parse_ulid(&raw) {
        let Some(being) = &app.being else {
            return ToolResult::fail("the being store did not open");
        };
        let guard = being.lock().await;
        let Some(recorded) = guard.goals().get(&id) else {
            return ToolResult::fail(format!("no goal {}", ulid_string(&id)));
        };
        let path = app
            .cfg
            .store
            .data_dir
            .join("goals")
            .join(format!("{}.json", ulid_string(&id)));
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                return ToolResult::fail(err.to_string());
            }
        }
        let body = json!({
            "id": ulid_string(&id),
            "description": recorded.description,
            "module_name": "operator-goal",
            "target_path": "modules/operator-goal",
            "target_uri": "https://metamind.dev/code/module/operator-goal",
            "capability": "mm:OperatorGoal",
            "function": "show",
            "success_criteria": [recorded.description],
        });
        if let Err(err) = std::fs::write(&path, body.to_string()) {
            return ToolResult::fail(err.to_string());
        }
        match LoopGoal::from_file(&path) {
            Ok(goal) => goal,
            Err(err) => return ToolResult::fail(err.to_string()),
        }
    } else {
        let path = match resolve(app, Path::new(&raw)) {
            Ok(path) => path,
            Err(err) => return ToolResult::fail(err),
        };
        match LoopGoal::from_file(&path) {
            Ok(goal) => goal,
            Err(err) => return ToolResult::fail(err.to_string()),
        }
    };
    let controller = loop_controller(app);
    match controller.run_loop(goal, BudgetEnvelope::default()).await {
        Ok(report) => ToolResult::done(
            format!("loop {}", ulid_string(&report.run_id)),
            json!({
                "run": ulid_string(&report.run_id),
                "completed": report.completed(),
                "digest": report.digest,
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn loop_status(app: &AppState, input: &Value) -> ToolResult {
    let id = text(input, "id");
    let sql = if id.is_empty() {
        "SELECT id, status FROM loop_runs ORDER BY rowid DESC LIMIT 5".to_string()
    } else {
        format!(
            "SELECT id, status FROM loop_runs WHERE id = '{}'",
            id.replace('\'', "")
        )
    };
    let rows = match app.sqlite.query_json(&sql, mm_core::Params::new()).await {
        Ok(rows) => rows,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    if rows.is_empty() {
        return ToolResult::done("no loop run is recorded", json!({ "runs": [] }));
    }
    ToolResult::done(format!("{} loop runs", rows.len()), json!({ "runs": rows }))
}

pub async fn self_model_report(app: &AppState, input: &Value) -> ToolResult {
    let id = text(input, "id");
    let Ok(run) = mm_core::id::parse_ulid(&id) else {
        return ToolResult::fail("a run id is required");
    };
    match loop_controller(app).measure(&run).await {
        Ok(report) => ToolResult::done(
            format!("self-model {}", ulid_string(&report.id)),
            json!({
                "id": ulid_string(&report.id),
                "run": ulid_string(&report.run_id),
                "actual_model": report.actual_model,
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn debt_scan(app: &AppState) -> ToolResult {
    let mut scanner = CompositeDebtScanner::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
        app.cfg.root.clone(),
    );
    if let Some(graph) = &app.graph {
        scanner = scanner.with_graph(Arc::clone(graph));
    }
    match scanner.scan_and_persist().await {
        Ok(report) => ToolResult::done(
            format!("{} debt findings", report.findings.len()),
            json!({ "findings": report.findings.len(), "persisted": report.persisted }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn gc_propose(app: &AppState) -> ToolResult {
    let scan = match loop_controller(app).scan_debt(None).await {
        Ok(scan) => scan,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let collector = collector(app);
    match collector.run(&scan.findings).await {
        Ok(report) => ToolResult::done(
            format!("gc proposed {}", report.actions.len()),
            json!({
                "proposed": report.actions.len(),
                "refused": report.refused.len(),
                "dry_run": true,
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn gc_apply(app: &AppState) -> ToolResult {
    let scan = match loop_controller(app).scan_debt(None).await {
        Ok(scan) => scan,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let collector = collector(app);
    let report = match collector.run(&scan.findings).await {
        Ok(report) => report,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    for action in &report.actions {
        if let Err(err) = collector.apply(action).await {
            return ToolResult::fail(err.to_string());
        }
    }
    ToolResult::done(
        format!("gc applied {}", report.actions.len()),
        json!({ "applied": report.actions.len(), "dry_run": false }),
    )
}

pub async fn module_load(app: &AppState, input: &Value) -> ToolResult {
    if text(input, "change_set").is_empty() {
        return ToolResult::fail(crate::chat::PROMOTE_REASON);
    }
    let spec = text(input, "uri");
    let (uri, version) = match ModuleVersion::parse_spec(&spec) {
        Ok(pair) => pair,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let loader = loader(app);
    let Some(dir) = loader.find(&uri, &version) else {
        return ToolResult::fail(format!("no materialised version {uri}@{version}"));
    };
    match loader
        .hot_load(&ModuleVersion::new(uri, version, dir))
        .await
    {
        Ok(receipt) => ToolResult::done(
            format!("module {}", receipt.status.as_str()),
            json!({ "uri": receipt.uri, "version": receipt.version, "status": receipt.status.as_str() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn module_rollback(app: &AppState, input: &Value) -> ToolResult {
    let uri = text(input, "uri");
    if uri.is_empty() {
        return ToolResult::fail("a module uri is required");
    }
    match loader(app).rollback(&uri).await {
        Ok(receipt) => ToolResult::done(
            format!("module {}", receipt.status.as_str()),
            json!({ "uri": receipt.uri, "version": receipt.version, "status": receipt.status.as_str() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn design_show(app: &AppState, input: &Value) -> ToolResult {
    let doc = text(input, "doc");
    if doc.is_empty() {
        return ToolResult::fail("a document uri is required");
    }
    let mut writer = ChangeSetDesignWriter::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
        app.cfg.store.data_dir.join("artifacts"),
    );
    if let Some(graph) = &app.graph {
        writer = writer.with_graph(Arc::clone(graph));
    }
    match writer.history(&doc).await {
        Ok(revisions) => ToolResult::done(
            format!("{} design revisions", revisions.len()),
            json!({ "revisions": revisions.len() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub fn pi_run() -> ToolResult {
    if !pi_on_path() {
        return ToolResult::fail("pi is not on PATH");
    }
    ToolResult::fail("a live pi run needs a task file, and this process does not download Pi")
}

pub async fn pi_ingest(app: &AppState, input: &Value) -> ToolResult {
    let file = text(input, "file");
    if file.is_empty() {
        return ToolResult::fail("a session file is required");
    }
    let path = match resolve(app, Path::new(&file)) {
        Ok(path) => path,
        Err(err) => return ToolResult::fail(err),
    };
    match mm_pi::ingest_session(&path, &app.sqlite, app.logger.as_ref(), app.ids.as_ref()).await {
        Ok(graph) => ToolResult::done(
            format!("pi session {}", ulid_string(&graph.session_id)),
            json!({ "session": ulid_string(&graph.session_id) }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub fn codex_scan(app: &AppState) -> ToolResult {
    match mm_codex::scan(&app.cfg.root, &mm_codex::PreviousState::default(), false) {
        Ok(output) => ToolResult::done(
            format!("codex {} modules", output.report.modules.len()),
            json!({
                "modules": output.report.modules.len(),
                "graph_hash": output.report.graph_hash,
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub fn codex_verify(app: &AppState) -> ToolResult {
    let output = match mm_codex::scan(&app.cfg.root, &mm_codex::PreviousState::default(), false) {
        Ok(output) => output,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let failures = mm_codex::verify(&output.report, None);
    let mut result = ToolResult::done(
        format!("codex verify {} failures", failures.len()),
        json!({ "failures": failures.len() }),
    );
    result.ok = failures.is_empty();
    result
}

pub fn experience_compile(app: &AppState, input: &Value) -> ToolResult {
    let file = text(input, "file");
    if file.is_empty() {
        return ToolResult::fail("a trajectory file is required");
    }
    let path = match resolve(app, Path::new(&file)) {
        Ok(path) => path,
        Err(err) => return ToolResult::fail(err),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let trajectories = match Trajectory::from_jsonl(&text) {
        Ok(rows) => rows,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let compiled = experience::compile(&trajectories, &[] as &[Insight]);
    ToolResult::done(
        format!(
            "experience compiled {} techniques",
            compiled.techniques.len()
        ),
        json!({ "techniques": compiled.techniques.len() }),
    )
}

pub async fn llm_stats(app: &AppState) -> ToolResult {
    let n = count(app, "SELECT COUNT(*) AS n FROM llm_calls").await;
    ToolResult::done(format!("llm calls {n}"), json!({ "calls": n }))
}

pub async fn meta_analyze(app: &AppState, input: &Value) -> ToolResult {
    let episode = text(input, "episode");
    if episode.is_empty() {
        return ToolResult::fail("an episode file is required");
    }
    let path = match resolve(app, Path::new(&episode)) {
        Ok(path) => path,
        Err(err) => return ToolResult::fail(err),
    };
    let ctx = mm_metaanalysis::MetaContext::local(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    match mm_metaanalysis::analyze_fixture(&path, &ctx).await {
        Ok(analysis) => ToolResult::done(
            format!("meta {}", analysis.trigger.as_str()),
            json!({ "trigger": analysis.trigger.as_str() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn regression_run(app: &AppState) -> ToolResult {
    let suite = app.cfg.root.join("bench/regression");
    match mm_mistakes::run_suite(&suite).await {
        Ok(report) => ToolResult::done(
            format!(
                "regression {} failed of {}",
                report.failed,
                report.cases.len()
            ),
            json!({ "failed": report.failed, "cases": report.cases.len() }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub fn verify_run(input: &Value) -> ToolResult {
    let kind = text(input, "kind");
    if !matches!(
        kind.as_str(),
        "cargo-build" | "cargo-test" | "static-analysis" | "symbolic" | "math-residual"
    ) {
        return ToolResult::fail(
            "unknown obligation (one of: cargo-build, cargo-test, static-analysis, symbolic, math-residual)",
        );
    }
    ToolResult::fail(format!(
        "verify {kind} is not an in-process check on this route; mm-cli verify remains the runner"
    ))
}

pub fn mcp_list() -> ToolResult {
    super::mutate::sandbox_tool_list()
}

fn library_store(app: &AppState) -> LibraryStore {
    LibraryStore::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    )
}

fn loop_controller(app: &AppState) -> LoopController {
    let events = mm_eventlog::EventLog::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    let services = LoopServices::new(
        &app.cfg,
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
        events,
        app.graph.clone(),
    );
    LoopController::new(services)
}

fn collector(app: &AppState) -> LadderCollector {
    let collector = LadderCollector::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    match &app.graph {
        Some(graph) => collector.with_graph(Arc::clone(graph)),
        None => collector,
    }
}

fn loader(app: &AppState) -> ManifestModuleLoader {
    let loader = ManifestModuleLoader::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
        app.cfg.store.data_dir.join("artifacts"),
    );
    match &app.graph {
        Some(graph) => loader.with_graph(Arc::clone(graph)),
        None => loader,
    }
}

fn pi_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join("pi").is_file())
}

async fn count(app: &AppState, sql: &str) -> i64 {
    app.sqlite
        .query_json(sql, mm_core::Params::new())
        .await
        .ok()
        .and_then(|rows| rows.first().cloned())
        .and_then(|row| row.get("n").cloned())
        .and_then(|n| n.as_i64())
        .unwrap_or(0)
}

fn text(input: &Value, key: &str) -> String {
    input
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn resolve(app: &AppState, path: &Path) -> Result<PathBuf, String> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        app.cfg.root.join(path)
    };
    let root = app.cfg.root.clone();
    let data = app.cfg.store.data_dir.clone();
    if full.starts_with(&root) || full.starts_with(&data) {
        Ok(full)
    } else {
        Err("the path is outside the repo and the data dir".into())
    }
}

fn graph_closed(app: &AppState) -> String {
    app.graph_error
        .clone()
        .unwrap_or_else(|| "the RDF store did not open".into())
}
