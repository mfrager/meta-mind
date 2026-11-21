//! Phase 4 writes. Each one calls the subsystem the CLI calls.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mm_being::{BeingOp, GoalOrigin};
use mm_core::ulid_string;
use mm_decision::{
    hosted, CoreId, DecisionCore, DecisionQuestion, DecisionState, HostedCore, RulesCore,
};
use mm_llm::{OpenAiClient, SchemaRegistry};
use mm_memory::{Memory, MemoryEngine, MemoryKind};
use mm_selfeng::{
    changeset::{self, ChangeSetStore},
    sandbox::{audit_writes, Sandbox, WorktreeSandbox},
    BudgetKind, BudgetLedger, ChangeSetStatus, DeterministicGate, EvolutionJournal,
    PromotionDecision,
};
use mm_tools::{rollback, ActionSpec, ActionStatus, Executor, Principal, ToolName};
use serde_json::{json, Value};

use crate::state::AppState;

use super::super::exec::ToolResult;
use super::super::intro::llm_config;

pub async fn goal_add(app: &AppState, input: &Value) -> ToolResult {
    let Some(being) = &app.being else {
        return ToolResult::fail(being_error(app));
    };
    let description = text(input, "description");
    let priority = input.get("priority").and_then(Value::as_f64).unwrap_or(0.5) as f32;
    let mut guard = being.lock().await;
    let before: Vec<_> = guard.goals().keys().copied().collect();
    if let Err(err) = guard
        .apply(BeingOp::AddGoal {
            description: description.clone(),
            priority,
            origin: GoalOrigin::User,
        })
        .await
    {
        return ToolResult::fail(err.to_string());
    }
    let id = guard
        .goals()
        .keys()
        .find(|id| !before.contains(id))
        .copied();
    let id = id.map(|id| ulid_string(&id)).unwrap_or_default();
    let mut result = ToolResult::done(
        format!("goal added: {id}"),
        json!({ "id": id, "description": description, "origin": "operator" }),
    );
    result.focus = Some(json!({ "id": "goals", "kind": "tree", "title": "Goals" }));
    result
}

pub async fn memory_add(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(graph_error(app));
    };
    let kind = match MemoryKind::parse(text(input, "kind").as_str()) {
        Some(kind) if matches!(kind, MemoryKind::Semantic | MemoryKind::Episodic) => kind,
        Some(_) => return ToolResult::fail("kind must be semantic or episodic"),
        None => return ToolResult::fail("kind must be semantic or episodic"),
    };
    let content = text(input, "content");
    let mut engine = match MemoryEngine::open(
        &app.sqlite,
        graph.handle(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    )
    .await
    {
        Ok(engine) => engine,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let now = mm_core::Timestamp::now();
    let id = engine.ids().next();
    let memory = match Memory::draft(id, kind, content.clone(), now) {
        Ok(memory) => memory,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    match engine.remember(memory).await {
        Ok(admission) => {
            let summary = format!("memory added: {}", ulid_string(&id));
            ToolResult::done(
                summary,
                json!({
                    "id": ulid_string(&id),
                    "kind": kind.as_str(),
                    "content": content,
                    "admission": format!("{admission:?}"),
                }),
            )
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn memory_forget(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(graph_error(app));
    };
    let apply = input.get("apply").and_then(Value::as_bool).unwrap_or(false);
    let engine = match MemoryEngine::open(
        &app.sqlite,
        graph.handle(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    )
    .await
    {
        Ok(engine) => engine,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    match engine.forget_cycle(mm_core::Timestamp::now(), !apply).await {
        Ok(report) => ToolResult::done(
            format!(
                "forget considered {} archived {} dry_run {}",
                report.considered,
                report.archived.len(),
                report.dry_run
            ),
            json!({
                "considered": report.considered,
                "archived": report.archived.len(),
                "dry_run": report.dry_run,
                "apply": apply,
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn decide_ask(app: &AppState, input: &Value) -> ToolResult {
    let core_name = input.get("core").and_then(Value::as_str).unwrap_or("rules");
    let Some(core_id) = CoreId::parse(core_name) else {
        return ToolResult::fail(format!("unknown core {core_name}"));
    };
    let prompt = text(input, "prompt");
    if prompt.is_empty() {
        return ToolResult::fail("a prompt is required");
    }
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("yes_no");
    let id = app.ids.next();
    let question = match kind {
        "yes_no" => DecisionQuestion::YesNo {
            id,
            prompt: prompt.clone(),
        },
        "choice" => {
            let options = input
                .get("options")
                .and_then(Value::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| row.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            DecisionQuestion::Choice {
                id,
                prompt: prompt.clone(),
                options,
                allow_none: true,
            }
        }
        other => return ToolResult::fail(format!("unknown question kind {other}")),
    };
    let state = DecisionState::new(app.ids.next(), json!({}));
    let answer = match core_id {
        CoreId::Rules => {
            let core = match RulesCore::embedded() {
                Ok(core) => core,
                Err(err) => return ToolResult::fail(err.to_string()),
            };
            match core.answer(&question, &state).await {
                Ok(answer) => answer,
                Err(err) => return ToolResult::fail(err.to_string()),
            }
        }
        CoreId::Hosted => match hosted_answer(app, &question, &state).await {
            Ok(answer) => answer,
            Err(reason) => return ToolResult::fail(reason),
        },
        CoreId::Local => {
            return ToolResult::fail(
                "local cannot answer: no decision head is configured on this route",
            );
        }
    };
    let admitted = !matches!(answer.answer, mm_decision::AnswerValue::None);
    let decision_id = if input
        .get("no_record")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Value::Null
    } else {
        json!(ulid_string(&id))
    };
    let summary = if admitted {
        format!(
            "core {} admitted {}",
            answer.core.as_str(),
            answer.answer.canonical()
        )
    } else {
        format!("core {} abstained", answer.core.as_str())
    };
    let mut result = ToolResult::done(
        summary,
        json!({
            "core": answer.core.as_str(),
            "admitted": admitted,
            "answer": answer.answer.canonical(),
            "confidence": answer.confidence,
            "decision_id": decision_id,
        }),
    );
    result.focus = Some(json!({ "id": "decision", "kind": "table", "title": "Decision" }));
    result
}

async fn hosted_answer(
    app: &AppState,
    question: &DecisionQuestion,
    state: &DecisionState,
) -> Result<mm_decision::DecisionAnswer, String> {
    let cfg = llm_config(app);
    let client = OpenAiClient::new(&cfg)
        .map_err(|_| "no language-model provider is configured".to_string())?;
    let mut registry = SchemaRegistry::new();
    hosted::register(&mut registry).map_err(|err| err.to_string())?;
    let core = HostedCore::new(
        std::sync::Arc::new(client),
        std::sync::Arc::new(registry),
        256,
    );
    core.answer(question, state)
        .await
        .map_err(|err| err.to_string())
}

pub async fn firewall_eval(app: &AppState, input: &Value) -> ToolResult {
    let name = input
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("memory.add");
    let Some(spec) =
        super::super::catalog::get(name).or_else(|| super::super::catalog::get("memory.add"))
    else {
        return ToolResult::fail(format!("unknown tool {name}"));
    };
    match super::super::firewall::allow(app, spec) {
        Ok(()) => ToolResult::done(
            "firewall allowed the action",
            json!({ "verdict": "allow", "tool": name }),
        ),
        Err(reason) => {
            let mut result = ToolResult::done(
                reason.clone(),
                json!({ "verdict": "reject", "tool": name, "reason": reason }),
            );
            result.ok = false;
            result.reason = Some(reason);
            result
        }
    }
}

pub fn compare_check(input: &Value) -> ToolResult {
    let baseline = text(input, "baseline");
    let candidate = text(input, "candidate");
    let report = mm_selfeng::shadow::compare(
        std::slice::from_ref(&baseline),
        std::slice::from_ref(&candidate),
    );
    ToolResult::done(
        format!("{} divergences", report.divergences),
        json!({
            "divergences": report.divergences,
            "detail": report.detail,
        }),
    )
}

pub fn risk_analyze(input: &Value) -> ToolResult {
    let Some(rows) = input.get("outcomes").and_then(Value::as_array) else {
        return ToolResult::fail("outcomes is required");
    };
    let outcomes: Vec<mm_decision::LossOutcome> = rows
        .iter()
        .filter_map(|row| {
            Some(mm_decision::LossOutcome {
                name: row
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("outcome")
                    .into(),
                probability: row.get("probability").and_then(Value::as_f64)?,
                loss: row.get("loss").and_then(Value::as_f64)?,
            })
        })
        .collect();
    if let Err(err) = mm_decision::validate_outcomes(&outcomes) {
        return ToolResult::fail(err.to_string());
    }
    let profile = mm_decision::analyze_risk(&outcomes, mm_decision::RiskMeasure::Variance);
    ToolResult::done(
        format!("expected_loss {}", profile.expected_loss),
        json!({
            "expected_loss": profile.expected_loss,
            "max_loss": profile.max_loss,
            "variance": profile.variance,
        }),
    )
}

pub async fn changeset_new(app: &AppState, input: &Value) -> ToolResult {
    let gap = if let Some(path) = input.get("gap").and_then(Value::as_str) {
        let full = match resolve_allowed(app, Path::new(path)) {
            Ok(full) => full,
            Err(err) => return ToolResult::fail(err),
        };
        changeset::load_gap(&full).map_err(|err| err.to_string())
    } else {
        let title = text(input, "title");
        let hypothesis = text(input, "hypothesis");
        if title.is_empty() || hypothesis.is_empty() {
            return ToolResult::fail("a gap path or a title and hypothesis is required");
        }
        let dir = app.cfg.store.data_dir.join("gaps");
        if let Err(err) = std::fs::create_dir_all(&dir) {
            return ToolResult::fail(err.to_string());
        }
        let evidence = app.ids.next();
        let path = dir.join(format!("{}.json", ulid_string(&evidence)));
        let body = json!({
            "gap_id": ulid_string(&app.ids.next()),
            "kind": "capability",
            "statement": if hypothesis.is_empty() { title } else { hypothesis },
            "evidence": [ulid_string(&evidence)],
            "target_uri": "https://metamind.dev/code/module/note",
            "target_path": "modules/note",
        });
        if let Err(err) = std::fs::write(&path, body.to_string()) {
            return ToolResult::fail(err.to_string());
        }
        changeset::load_gap(&path).map_err(|err| err.to_string())
    };
    let gap = match gap {
        Ok(gap) => gap,
        Err(err) => return ToolResult::fail(err),
    };
    let note = format!("# {}\n\n{}\n", gap.statement, gap.kind);
    let patch = changeset::Patch::create(gap.target_path.join("SESSION.md"), note);
    let change_set = match changeset::from_gap(
        &gap,
        vec![patch],
        vec![mm_selfeng::TestId::new("bench/regression/seeded_bug_01")],
        vec![mm_selfeng::BenchmarkId::new("promotion_bench")],
        app.ids.as_ref(),
    ) {
        Ok(change_set) => change_set,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let store = ChangeSetStore::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    if let Err(err) = store.create(&change_set).await {
        return ToolResult::fail(err.to_string());
    }
    let id = ulid_string(&change_set.id);
    let mut result = ToolResult::done(
        format!("changeset {id}"),
        json!({
            "id": id,
            "status": ChangeSetStatus::Draft.as_str(),
            "reason": change_set.reason,
        }),
    );
    result.focus = Some(json!({ "id": "changesets", "kind": "tree", "title": "Change sets" }));
    result
}

pub async fn sandbox_run(app: &AppState, input: &Value) -> ToolResult {
    let Some(id) = parse_id(input) else {
        return ToolResult::fail("a change-set id is required");
    };
    let store = ChangeSetStore::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    let change_set = match store.get(id).await {
        Ok(Some(change_set)) => change_set,
        Ok(None) => return ToolResult::fail(format!("no change set {}", ulid_string(&id))),
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let sandbox = WorktreeSandbox::new(
        app.cfg.root.clone(),
        app.cfg.store.data_dir.join("sandbox"),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    match sandbox.prepare(&change_set).await {
        Ok(dir) => {
            let _ = store
                .set_status(change_set.id, ChangeSetStatus::Sandboxed)
                .await;
            let mut result = ToolResult::done(
                format!("sandbox {}", dir.root.display()),
                json!({ "sandbox": dir.root.display().to_string(), "id": ulid_string(&id) }),
            );
            result.focus =
                Some(json!({ "id": "changesets", "kind": "tree", "title": "Change sets" }));
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn promote_run(app: &AppState, input: &Value) -> ToolResult {
    let Some(id) = parse_id(input) else {
        return ToolResult::fail("a change-set id is required");
    };
    let store = ChangeSetStore::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    let change_set = match store.get(id).await {
        Ok(Some(change_set)) => change_set,
        Ok(None) => return ToolResult::fail(format!("no change set {}", ulid_string(&id))),
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let suite = app.cfg.root.join("bench/regression");
    let regression = match mm_mistakes::run_suite(&suite).await {
        Ok(report) => report,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let ledger = BudgetLedger::new(app.sqlite.clone(), Arc::clone(&app.logger));
    let remaining = match ledger.remaining(BudgetKind::Evolution).await {
        Ok(remaining) => remaining,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    if let Err(err) = ledger
        .debit(BudgetKind::Evolution, 1.0, change_set.id)
        .await
    {
        return ToolResult::fail(err.to_string());
    }
    let mut bundle =
        mm_selfeng::EvidenceBundle::new(change_set.id, regression).with_risk(0.0, remaining);
    let bench_file = app.cfg.root.join("bench/promotion/promotion_bench.json");
    if bench_file.exists() {
        match mm_selfeng::benchmark::load_specs(&bench_file) {
            Ok(specs) => {
                if let Some(spec) = specs.first() {
                    match mm_selfeng::benchmark::run_benchmark(
                        spec,
                        &app.cfg.root,
                        app.logger.as_ref(),
                    )
                    .await
                    {
                        Ok(result) => bundle = bundle.with_bench(result),
                        Err(err) => return ToolResult::fail(err.to_string()),
                    }
                }
            }
            Err(err) => return ToolResult::fail(err.to_string()),
        }
    }
    let gate = DeterministicGate::new();
    if gate.policy().touches_immutable(&change_set).is_some() {
        bundle = bundle.touching_immutable();
    }
    let journal = EvolutionJournal::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    match gate.promote(&change_set, &bundle, &store, &journal).await {
        Ok(outcome) => {
            let reason = outcome
                .decision
                .reason()
                .unwrap_or("the gate found nothing to reject")
                .to_string();
            let ok = matches!(outcome.decision, PromotionDecision::Promote);
            let mut result = ToolResult::done(
                reason.clone(),
                json!({
                    "id": ulid_string(&change_set.id),
                    "promotion": ulid_string(&outcome.id),
                    "decision": outcome.decision.as_str(),
                    "reason": reason,
                }),
            );
            if !ok {
                result.ok = false;
                result.reason = Some(reason);
            }
            result
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn reject_run(app: &AppState, input: &Value) -> ToolResult {
    let Some(id) = parse_id(input) else {
        return ToolResult::fail("a change-set id is required");
    };
    let reason = text(input, "reason");
    if reason.trim().len() < 8 {
        return ToolResult::fail("a rejection without a reason is not a decision: pass --reason");
    }
    let store = ChangeSetStore::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    let change_set = match store.get(id).await {
        Ok(Some(change_set)) => change_set,
        Ok(None) => return ToolResult::fail(format!("no change set {}", ulid_string(&id))),
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let gate = DeterministicGate::new();
    let decision = PromotionDecision::Reject {
        reason: reason.clone(),
    };
    let bundle = mm_selfeng::EvidenceBundle::new(
        change_set.id,
        mm_mistakes::SuiteReport {
            cases: Vec::new(),
            failed: 0,
        },
    );
    let journal = EvolutionJournal::new(
        app.sqlite.clone(),
        Arc::clone(&app.logger),
        Arc::clone(&app.ids),
    );
    match gate
        .record_decision(
            &change_set,
            &decision,
            &bundle,
            &app.sqlite,
            app.logger.as_ref(),
            app.ids.as_ref(),
        )
        .await
    {
        Ok(promotion_id) => {
            let self_version = match journal.next_version("reject", change_set.id).await {
                Ok(version) => version,
                Err(err) => return ToolResult::fail(err.to_string()),
            };
            let mut event = mm_selfeng::journal::EvolutionEvent::new(
                app.ids.as_ref(),
                change_set.reason.clone(),
                change_set.hypothesis.clone(),
                reason.clone(),
                "reject",
            )
            .with_changeset(change_set.id);
            event.self_version = self_version;
            event.evidence_json = match bundle.evidence_json() {
                Ok(json) => json,
                Err(err) => return ToolResult::fail(err.to_string()),
            };
            if let Err(err) = journal.append(&event).await {
                return ToolResult::fail(err.to_string());
            }
            let _ = store
                .set_status(change_set.id, ChangeSetStatus::Rejected)
                .await;
            ToolResult::done(
                reason.clone(),
                json!({
                    "id": ulid_string(&id),
                    "promotion": ulid_string(&promotion_id),
                    "decision": "reject",
                    "reason": reason,
                }),
            )
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn audit_production_tree(app: &AppState) -> ToolResult {
    let promotions: i64 = match sqlx::query_scalar("SELECT count(*) FROM promotions")
        .fetch_one(app.sqlite.pool())
        .await
    {
        Ok(count) => count,
        Err(err) => return ToolResult::fail(format!("cannot read promotions: {err}")),
    };
    let journal: i64 = match sqlx::query_scalar("SELECT count(*) FROM evolution_journal")
        .fetch_one(app.sqlite.pool())
        .await
    {
        Ok(count) => count,
        Err(err) => return ToolResult::fail(format!("cannot read the journal: {err}")),
    };
    let audit = audit_writes(
        &app.cfg.root,
        &app.cfg.store.data_dir.join("sandbox"),
        std::time::SystemTime::UNIX_EPOCH,
    );
    ToolResult::done(
        format!("promotions {promotions}"),
        json!({
            "promotions": promotions,
            "journal": journal,
            "audit_complete": audit.complete,
            "outside": audit.outside.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        }),
    )
}

pub fn sandbox_tool_list() -> ToolResult {
    match mm_tools::tools::default_registry() {
        Ok(registry) => {
            let names: Vec<String> = registry
                .list()
                .iter()
                .map(|spec| spec.name.to_string())
                .collect();
            ToolResult::done(
                format!("{} sandbox tools", names.len()),
                json!({ "tools": names }),
            )
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub fn sandbox_tool_describe(input: &Value) -> ToolResult {
    let name = text(input, "name");
    let registry = match mm_tools::tools::default_registry() {
        Ok(registry) => registry,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let tool = match ToolName::new(name) {
        Ok(tool) => tool,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    match registry.describe(&tool) {
        Ok(spec) => ToolResult::done(
            spec.description.clone(),
            json!({ "name": spec.name.to_string(), "description": spec.description }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn sandbox_tool_run(app: &AppState, input: &Value) -> ToolResult {
    let Some(graph) = &app.graph else {
        return ToolResult::fail(graph_error(app));
    };
    let name = text(input, "name");
    let arguments = input.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let registry = match mm_tools::tools::default_registry() {
        Ok(registry) => registry,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    if let Err(err) = registry.persist(&app.sqlite).await {
        return ToolResult::fail(err.to_string());
    }
    let engine = match mm_tools::executor::load_engine(&app.sqlite).await {
        Ok(engine) => engine,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let executor = Executor::new(
        registry,
        std::sync::Arc::new(engine),
        app.sqlite.clone(),
        graph.handle().clone(),
        Arc::clone(&app.ids),
    )
    .with_sandbox_root(app.cfg.root.join(mm_tools::SANDBOX_ROOT));
    let tool = match ToolName::new(name) {
        Ok(tool) => tool,
        Err(err) => return ToolResult::fail(err.to_string()),
    };
    let action = ActionSpec::new(tool, arguments, Principal("operator".into()));
    match executor.execute(action).await {
        Ok(result) => {
            let ok = matches!(result.status, ActionStatus::Ok | ActionStatus::Deduplicated);
            let reason = result
                .reason
                .clone()
                .unwrap_or_else(|| result.status.as_str().into());
            let mut body = ToolResult::done(
                reason.clone(),
                json!({
                    "status": result.status.as_str(),
                    "action_id": ulid_string(&result.action_id),
                    "reason": result.reason,
                    "output": result.output,
                }),
            );
            if !ok {
                body.ok = false;
                body.reason = Some(reason);
            }
            body
        }
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn action_ledger(app: &AppState, input: &Value) -> ToolResult {
    let id = input.get("id").and_then(Value::as_str);
    let rows: Result<Vec<(String, String)>, sqlx::Error> = if let Some(id) = id {
        sqlx::query_as("SELECT tool_name, status FROM tool_calls WHERE id = ?")
            .bind(id)
            .fetch_all(app.sqlite.pool())
            .await
    } else {
        sqlx::query_as("SELECT tool_name, status FROM tool_calls ORDER BY rowid DESC LIMIT 20")
            .fetch_all(app.sqlite.pool())
            .await
    };
    match rows {
        Ok(rows) if rows.is_empty() => {
            let label = id.unwrap_or("any");
            ToolResult::done(
                format!("no tool_calls row for {label}"),
                json!({ "rows": [] }),
            )
        }
        Ok(rows) => ToolResult::done(
            format!("{} ledger rows", rows.len()),
            json!({
                "rows": rows.into_iter().map(|(tool, status)| json!({"tool": tool, "status": status})).collect::<Vec<_>>()
            }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn action_rollback(app: &AppState, input: &Value) -> ToolResult {
    let Some(id) = parse_id(input) else {
        return ToolResult::fail("an action id is required");
    };
    match rollback::rollback(&app.sqlite, &id).await {
        Ok(hash) => ToolResult::done(
            format!("restored {} to {hash}", ulid_string(&id)),
            json!({ "id": ulid_string(&id), "restored_hash": hash }),
        ),
        Err(err) => ToolResult::fail(err.to_string()),
    }
}

pub async fn focus_patch(app: &AppState, input: &Value) -> ToolResult {
    let id = text(input, "id");
    if let Ok(doc) = crate::routes::focus::resolve(app, &id).await {
        if doc
            .reason
            .as_deref()
            .is_some_and(|reason| reason.to_ascii_lowercase().contains("fixture"))
        {
            return ToolResult::fail(doc.reason.unwrap_or_else(|| "fixture".into()));
        }
    }
    let patch = input.get("patch").cloned().unwrap_or_else(|| json!({}));
    let value = patch
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match id.as_str() {
        "goals" => {
            goal_add(
                app,
                &json!({ "description": value, "priority": patch.get("priority").cloned().unwrap_or(json!(0.5)) }),
            )
            .await
        }
        "memory" => {
            let kind = patch
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("semantic");
            memory_add(app, &json!({ "kind": kind, "content": value })).await
        }
        _ => ToolResult::fail(format!("focus.patch cannot map {id}")),
    }
}

fn text(input: &Value, key: &str) -> String {
    input
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn parse_id(input: &Value) -> Option<mm_core::Ulid> {
    let text = input.get("id").and_then(Value::as_str)?;
    mm_core::id::parse_ulid(text).ok()
}

fn resolve_allowed(app: &AppState, path: &Path) -> Result<PathBuf, String> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        app.cfg.root.join(path)
    };
    let repo = app
        .cfg
        .root
        .canonicalize()
        .unwrap_or_else(|_| app.cfg.root.clone());
    let data = app
        .cfg
        .store
        .data_dir
        .canonicalize()
        .unwrap_or_else(|_| app.cfg.store.data_dir.clone());
    let check = full.canonicalize().unwrap_or_else(|_| full.clone());
    if check.starts_with(&repo) || check.starts_with(&data) {
        Ok(full)
    } else {
        Err("the path is outside the repo and the data dir".into())
    }
}

fn being_error(app: &AppState) -> String {
    app.being_error
        .clone()
        .unwrap_or_else(|| "the being store did not open".into())
}

fn graph_error(app: &AppState) -> String {
    app.graph_error
        .clone()
        .unwrap_or_else(|| "the RDF store did not open".into())
}
