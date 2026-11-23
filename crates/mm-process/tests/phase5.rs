//! Phase 5 acceptance tests: templates, machine, queue, engine, waits, recovery,
//! sub-processes, plan building, expiry and version pinning.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mm_data::process::{
    ProcessError, ProcessStatus, ProcessTemplate, StepKind, StepStatus, TemplateStep,
    WaitCondition, WaitKind,
};
use mm_data::rdf::now_ms;
use mm_data::{initialize_data_layer, InitConfig, Plan, PlanStatus, PlanStore, RdfPlanStore};
use mm_llm::mock::{MockClient, ScriptedResponse};
use mm_process::config::{EngineConfig, ProcessConfig};
use mm_process::engine::{Clock, ProcessEngine, StepOutcome, StepRequest, StepRunner};
use mm_process::plan_builder::{expire_due, materialize, PlanBuilder};
use mm_process::structure::StructureKind;
use mm_process::{machine, template};
use mm_prompt::PromptStore;
use serde_json::json;

struct Harness {
    engine: Arc<ProcessEngine>,
    _data: mm_data::InitResult,
}

struct StepFake {
    seen: Mutex<Vec<StepRequest>>,
    outputs: Mutex<BTreeMap<String, String>>,
    delay_ms: AtomicU64,
    current: AtomicUsize,
    peak: AtomicUsize,
    fail: Mutex<Option<String>>,
    fail_first: std::sync::atomic::AtomicBool,
}

impl StepFake {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            outputs: Mutex::new(BTreeMap::new()),
            delay_ms: AtomicU64::new(0),
            current: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            fail: Mutex::new(None),
            fail_first: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn with_delay(self: &Arc<Self>, delay_ms: u64) -> Arc<Self> {
        self.delay_ms.store(delay_ms, Ordering::SeqCst);
        Arc::clone(self)
    }

    fn set_fail_first(&self) {
        self.fail_first.store(true, Ordering::SeqCst);
    }

    fn set_output(&self, step: &str, output: &str) {
        self.outputs
            .lock()
            .unwrap()
            .insert(step.to_owned(), output.to_owned());
    }

    fn instructions(&self, step: &str) -> Option<String> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .find(|request| request.step_id == step)
            .map(|request| request.instructions.clone())
    }
}

#[async_trait]
impl StepRunner for StepFake {
    async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, ProcessError> {
        let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(current, Ordering::SeqCst);
        let delay = self.delay_ms.load(Ordering::SeqCst);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        self.current.fetch_sub(1, Ordering::SeqCst);
        if self.fail_first.swap(false, Ordering::SeqCst) {
            return Err(ProcessError::Invalid("transient failure".into()));
        }
        if let Some(message) = self.fail.lock().unwrap().clone() {
            return Err(ProcessError::Invalid(message));
        }
        self.seen.lock().unwrap().push(request.clone());
        let output = self
            .outputs
            .lock()
            .unwrap()
            .get(&request.step_id)
            .cloned()
            .unwrap_or_else(|| format!("{} complete", request.step_id));
        Ok(StepOutcome {
            reply: output,
            tokens_in: 3,
            tokens_out: 4,
            prompt_instance_id: Some("prompt-1".into()),
        })
    }
}

struct FakeClock(Mutex<i64>);

impl FakeClock {
    fn new(at: i64) -> Arc<Self> {
        Arc::new(Self(Mutex::new(at)))
    }
}

impl Clock for FakeClock {
    fn now_ms(&self) -> i64 {
        *self.0.lock().unwrap()
    }
}

async fn build_harness(runner: Arc<dyn StepRunner>, config: ProcessConfig) -> Harness {
    let data = initialize_data_layer(InitConfig {
        in_memory: true,
        ..InitConfig::default()
    })
    .await
    .unwrap();
    let store = Arc::new(mm_data::RdfProcessStore::new(Arc::clone(&data.engine)));
    let engine = Arc::new(
        ProcessEngine::new(
            store,
            runner,
            config,
            FakeClock::new(now_ms()),
            mm_process::BackendRegistry::empty(),
            std::env::temp_dir(),
            vec![
                "metamind.plan".into(),
                "metamind.act".into(),
                "data.search".into(),
                "agent.run".into(),
            ],
        )
        .unwrap(),
    );
    Harness {
        engine,
        _data: data,
    }
}

fn config() -> ProcessConfig {
    let mut config = ProcessConfig::default();
    config.engine = EngineConfig {
        batch_size: 4,
        max_live_instances: 8,
        ..config.engine
    };
    config
}

fn step(id: &str, title: &str) -> TemplateStep {
    TemplateStep::new(id, title, format!("do {title}"))
}

fn fixture(uri: &str, steps: Vec<TemplateStep>) -> ProcessTemplate {
    ProcessTemplate::new(
        format!("urn:metamind:process/template/{uri}"),
        uri,
        steps,
        now_ms(),
    )
}

async fn wait_status(
    engine: &Arc<ProcessEngine>,
    id: &str,
    expected: ProcessStatus,
) -> ProcessStatus {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let instance = engine.load_instance(id).await.unwrap();
        if instance.status == expected {
            return instance.status;
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "timed out waiting for {expected:?}: status={:?} error={:?} events={:?}",
                instance.status, instance.error, instance.events
            );
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn linear_steps_run_in_order_and_persist_outputs() {
    let fake = StepFake::new();
    fake.set_output("a", "alpha");
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config()).await;
    let mut b = step("b", "B");
    b.depends_on = vec!["a".into()];
    b.instructions = "use {{steps.a.output}}".into();
    let template = fixture("linear", vec![step("a", "A"), b]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "session", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
    let done = harness.engine.load_instance(&instance.id).await.unwrap();
    assert_eq!(done.steps["a"].output.as_deref(), Some("alpha"));
    assert_eq!(done.steps["b"].status, StepStatus::Done);
    assert_eq!(
        fake.instructions("b").as_deref(),
        Some("use alpha"),
        "the second step sees the first step's output"
    );
    assert_eq!(done.tokens_in, 6);
    assert!(done.events.iter().any(|event| event.kind == "completed"));
}

#[tokio::test]
async fn parallel_group_runs_concurrently_and_a_failure_fails_the_instance() {
    let fake = StepFake::new().with_delay(40);
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config()).await;
    let mut a = step("a", "A");
    a.parallel_group = Some("research".into());
    let mut b = step("b", "B");
    b.parallel_group = Some("research".into());
    let template = fixture("parallel", vec![a, b]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
    assert_eq!(fake.peak.load(Ordering::SeqCst), 2);

    let failing = StepFake::new();
    *failing.fail.lock().unwrap() = Some("tool exploded".into());
    let harness = build_harness(failing as Arc<dyn StepRunner>, config()).await;
    let template = fixture("failing", vec![step("a", "A")]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Failed).await,
        ProcessStatus::Failed
    );
    let failed = harness.engine.load_instance(&instance.id).await.unwrap();
    assert!(failed.error.unwrap().contains("tool exploded"));
    assert_eq!(failed.steps["a"].status, StepStatus::Failed);
}

#[tokio::test]
async fn human_step_waits_and_resume_continues() {
    let fake = StepFake::new();
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config()).await;
    let mut approval = step("approve", "Approve");
    approval.kind = StepKind::Human;
    approval.instructions = "Approve the change".into();
    approval.wait = Some(WaitCondition {
        kind: WaitKind::Approval,
        detail: "deploy?".into(),
        wake_at: None,
    });
    let mut next = step("deploy", "Deploy");
    next.depends_on = vec!["approve".into()];
    let template = fixture("approval", vec![approval, next]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Waiting).await,
        ProcessStatus::Waiting
    );
    assert!(
        fake.seen.lock().unwrap().is_empty(),
        "the CM step must not run yet"
    );
    let waiting = harness.engine.load_instance(&instance.id).await.unwrap();
    assert_eq!(waiting.waiting.unwrap().kind, WaitKind::Approval);
    let resumed = harness
        .engine
        .resume(&instance.id, Some("approved by operator"))
        .await
        .unwrap();
    assert_eq!(resumed.status, ProcessStatus::Running);
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
    let done = harness.engine.load_instance(&instance.id).await.unwrap();
    assert_eq!(
        done.steps["approve"].output.as_deref(),
        Some("approved by operator")
    );
    assert_eq!(done.steps["deploy"].status, StepStatus::Done);
}

#[tokio::test]
async fn process_document_and_template_version_are_pinned() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("PROCESS.md"),
        "Always run the checks.",
    )
    .unwrap();
    let data = initialize_data_layer(InitConfig {
        in_memory: true,
        ..InitConfig::default()
    })
    .await
    .unwrap();
    let fake = StepFake::new();
    let store = Arc::new(mm_data::RdfProcessStore::new(Arc::clone(&data.engine)));
    let engine = Arc::new(
        ProcessEngine::new(
            store,
            Arc::clone(&fake) as Arc<dyn StepRunner>,
            config(),
            FakeClock::new(now_ms()),
            mm_process::BackendRegistry::empty(),
            workspace.path(),
            Vec::new(),
        )
        .unwrap(),
    );
    let mut template = fixture("pinned", vec![step("a", "A")]);
    template.doc_path = Some("PROCESS.md".into());
    let saved = engine.save_template(template.clone(), 0).await.unwrap();
    assert_eq!(
        saved.instructions.as_deref(),
        Some("Always run the checks.")
    );
    let instance = engine
        .create_instance(&saved.uri, "s", BTreeMap::new(), 0, false, None)
        .await
        .unwrap();
    let mut v2 = saved.clone();
    v2.version = 2;
    v2.steps[0].instructions = "v2 instructions".into();
    engine.save_template(v2, saved.revision).await.unwrap();
    engine.start_instance(&instance.id).unwrap();
    assert_eq!(
        wait_status(&engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
    assert_eq!(fake.instructions("a").as_deref(), Some("do A"));
    assert_eq!(
        fake.seen.lock().unwrap()[0].doc.as_deref(),
        Some("Always run the checks.")
    );
}

#[tokio::test]
async fn queue_is_deduped_leased_and_bounded() {
    let fake = StepFake::new().with_delay(10);
    let mut config = config();
    config.engine.max_live_instances = 4;
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config).await;
    let template = fixture("queue", vec![step("a", "A")]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for index in 0..25 {
        let instance = harness
            .engine
            .create_instance(&template.uri, "s", BTreeMap::new(), index, false, None)
            .await
            .unwrap();
        ids.push(instance.id);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        harness.engine.drain_queue("worker-a").await.unwrap();
        let all = harness.engine.session_instances("s").await.unwrap();
        if all
            .iter()
            .all(|item| item.status == ProcessStatus::Completed)
        {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            let summary: Vec<_> = all
                .iter()
                .map(|item| (item.id.clone(), item.status, item.error.clone()))
                .collect();
            panic!("queue did not drain: {summary:?}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        fake.peak.load(Ordering::SeqCst) <= 4,
        "concurrency cap was exceeded: {}",
        fake.peak.load(Ordering::SeqCst)
    );
    assert_eq!(ids.len(), 25);

    // Dedupe returns the live instance instead of starting a second one.
    let first = harness
        .engine
        .create_instance(
            &template.uri,
            "dedupe",
            BTreeMap::new(),
            0,
            false,
            Some("same-key".into()),
        )
        .await
        .unwrap();
    let second = harness
        .engine
        .create_instance(
            &template.uri,
            "dedupe",
            BTreeMap::new(),
            0,
            false,
            Some("same-key".into()),
        )
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
}

#[tokio::test]
async fn step_timeout_and_backend_stub_fail_honestly() {
    let fake = StepFake::new().with_delay(5_000);
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config()).await;
    let mut slow = step("slow", "Slow");
    slow.timeout_ms = Some(50);
    let template = fixture("timeout", vec![slow]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Failed).await,
        ProcessStatus::Failed
    );
    let failed = harness.engine.load_instance(&instance.id).await.unwrap();
    let error = failed.error.unwrap();
    assert!(error.contains("timed out") || error.contains("timeout"));

    let fake = StepFake::new();
    let harness = build_harness(fake as Arc<dyn StepRunner>, config()).await;
    let mut backend = step("deploy", "Deploy");
    backend.kind = StepKind::Backend;
    backend.service = Some("deploy".into());
    backend.instructions = "deploy it".into();
    let template = fixture("backend", vec![backend]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Failed).await,
        ProcessStatus::Failed
    );
    let failed = harness.engine.load_instance(&instance.id).await.unwrap();
    let error = failed.error.unwrap();
    assert!(error.contains("backend_unavailable"), "{error}");
}

#[tokio::test]
async fn recovery_resumes_an_interrupted_instance() {
    let fake = StepFake::new();
    let harness = build_harness(fake as Arc<dyn StepRunner>, config()).await;
    let template = fixture("recover", vec![step("a", "A")]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let mut instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, false, None)
        .await
        .unwrap();
    instance.status = ProcessStatus::Running;
    instance.steps.get_mut("a").unwrap().status = StepStatus::Running;
    instance.steps.get_mut("a").unwrap().attempts = 1;
    harness
        .engine
        .store()
        .save_instance(&instance, instance.revision)
        .await
        .unwrap();
    let recovered = harness.engine.recover().await.unwrap();
    assert_eq!(recovered, vec![instance.id.clone()]);
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
    let done = harness.engine.load_instance(&instance.id).await.unwrap();
    assert_eq!(done.steps["a"].attempts, 2);
}

#[tokio::test]
async fn sub_process_cycles_and_depth_are_refused() {
    let fake = StepFake::new();
    let mut config = config();
    config.limits.max_depth = 2;
    let harness = build_harness(fake as Arc<dyn StepRunner>, config).await;
    let mut child = step("child", "Child");
    child.kind = StepKind::SubProcess;
    child.template_uri = Some("urn:metamind:process/template/child".into());
    let template = fixture("child", vec![child]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Failed).await,
        ProcessStatus::Failed
    );
    let failed = harness.engine.load_instance(&instance.id).await.unwrap();
    assert!(failed.error.unwrap().contains("sub_process_refused"));
}

#[tokio::test]
async fn plan_builder_repairs_bad_json_and_materializes_a_plan() {
    let prompts = Arc::new(
        PromptStore::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/prompts"),
        )
        .unwrap(),
    );
    let client = Arc::new(MockClient::script(vec![
        ScriptedResponse::text(
            json!({"complexity":"standard","structure":"flat","persist":false,"reason":"short"}).to_string(),
        ),
        ScriptedResponse::text("{\"title\": \"broken\""),
        ScriptedResponse::text(
            json!({
                "title": "Ship the fix",
                "summary": "Two steps",
                "steps": [
                    {"order": 0, "title": "Reproduce", "instructions": "Reproduce the bug", "depends_on": [], "parent": null, "parallel": null, "tools": [], "check": "the bug is reproducible"},
                    {"order": 1, "title": "Fix", "instructions": "Fix the bug", "depends_on": [0], "parent": 0, "parallel": null, "tools": [], "check": null}
                ]
            })
            .to_string(),
        ),
        ScriptedResponse::text(json!({"pass": true, "notes": []}).to_string()),
    ]));
    let builder = PlanBuilder::new(
        client,
        Some(prompts),
        Some("mock-small".into()),
        mm_process::PlanConfig::default(),
        mm_process::LimitConfig::default(),
    );
    let built = builder
        .build(
            "fix the refresh bug",
            None,
            Some(StructureKind::Flat),
            Some(false),
        )
        .await
        .unwrap();
    assert_eq!(built.generation_attempts, 2, "one repair after bad JSON");
    assert_eq!(built.structure.steps.len(), 2);
    let (plan, template) = materialize(
        "session",
        "fix the refresh bug",
        &built,
        "urn:metamind:process/template/from-plan",
        &mm_process::PlanConfig::default(),
        &mm_process::LimitConfig::default(),
        &[],
    )
    .unwrap();
    assert_eq!(plan.status, PlanStatus::Draft);
    assert!(plan.expires_at.is_some());
    assert!(plan.structure.is_some());
    assert_eq!(template.steps.len(), 2);
    assert_eq!(template.steps[1].depends_on, vec!["step-1"]);
}

#[tokio::test]
async fn plans_expire_unless_they_are_durable() {
    let harness = build_harness(StepFake::new() as Arc<dyn StepRunner>, config()).await;
    let store = Arc::new(RdfPlanStore::new(Arc::clone(&harness._data.engine)));
    let mut expiring = Plan::new("expiring", "s", "short task", 0);
    expiring.expires_at = Some(5);
    let mut durable = Plan::new("durable", "s", "long task", 0);
    durable.expires_at = Some(5);
    durable.never_expires = true;
    store.save(&expiring, 0).await.unwrap();
    store.save(&durable, 0).await.unwrap();
    let expired = expire_due(store.as_ref(), 10).await.unwrap();
    assert_eq!(expired, 1);
    assert_eq!(
        store.load("expiring").await.unwrap().unwrap().status,
        PlanStatus::Expired
    );
    assert_eq!(
        store.load("durable").await.unwrap().unwrap().status,
        PlanStatus::Draft
    );
}

#[tokio::test]
async fn failed_step_can_be_retried_and_pause_resumes() {
    let fake = StepFake::new();
    fake.set_fail_first();
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config()).await;
    let template = fixture("retry", vec![step("a", "A")]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Failed).await,
        ProcessStatus::Failed
    );
    let retried = harness.engine.retry_step(&instance.id, "a").await.unwrap();
    assert_eq!(retried.status, ProcessStatus::Running);
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );

    let slow = StepFake::new().with_delay(300);
    let harness = build_harness(slow as Arc<dyn StepRunner>, config()).await;
    let template = fixture("pause", vec![step("a", "A")]);
    harness
        .engine
        .save_template(template.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&template.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    wait_status(&harness.engine, &instance.id, ProcessStatus::Running).await;
    let paused = harness.engine.pause(&instance.id).await.unwrap();
    assert_eq!(paused.status, ProcessStatus::Paused);
    harness.engine.resume(&instance.id, None).await.unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
}

#[tokio::test]
async fn a_child_process_completes_and_both_are_exportable() {
    let fake = StepFake::new();
    let harness = build_harness(Arc::clone(&fake) as Arc<dyn StepRunner>, config()).await;
    let child = fixture("child-ok", vec![step("work", "Work")]);
    harness
        .engine
        .save_template(child.clone(), 0)
        .await
        .unwrap();
    let mut parent_step = step("call", "Call child");
    parent_step.kind = StepKind::SubProcess;
    parent_step.template_uri = Some(child.uri.clone());
    let parent = fixture("parent-ok", vec![parent_step]);
    harness
        .engine
        .save_template(parent.clone(), 0)
        .await
        .unwrap();
    let instance = harness
        .engine
        .create_instance(&parent.uri, "s", BTreeMap::new(), 0, true, None)
        .await
        .unwrap();
    assert_eq!(
        wait_status(&harness.engine, &instance.id, ProcessStatus::Completed).await,
        ProcessStatus::Completed
    );
    let done = harness.engine.load_instance(&instance.id).await.unwrap();
    assert_eq!(
        done.steps["call"].output.as_deref(),
        Some("all steps completed")
    );
    let analytics = harness.engine.analytics(Some("s")).await.unwrap();
    assert_eq!(analytics.total, 2, "parent and child");
    assert_eq!(analytics.completed, 2);
    let export = harness.engine.export(&instance.id, false).await.unwrap();
    assert!(export.contains("mm.process.export.v1"));
    let rdf = harness.engine.export(&instance.id, true).await.unwrap();
    assert!(rdf.contains("ProcessInstance"));
}

#[test]
fn machine_requires_the_current_group_or_a_resume_event() {
    let mut first = step("a", "A");
    first.parallel_group = Some("g".into());
    let mut second = step("b", "B");
    second.parallel_group = Some("g".into());
    let mut third = step("c", "C");
    third.depends_on = vec!["a".into(), "b".into()];
    let template = fixture("machine", vec![first, second, third]);
    let limits = mm_process::LimitConfig::default();
    let layout = template::layout(&template, limits.max_parallel).unwrap();
    let instance = mm_data::process::ProcessInstance::new(
        "urn:metamind:process/instance/m",
        &template,
        "s",
        0,
    );
    let snapshot = machine::start(&template, &layout, &instance).unwrap();
    // mm-sm treats an unmatched event as a no-op; the state must not move.
    let unchanged = machine::dispatch(
        &template,
        &layout,
        &snapshot,
        &format!("resume:{}", machine::step_state("a")),
    )
    .unwrap();
    assert_eq!(unchanged.state, snapshot.state);
    assert_eq!(unchanged.transitions, snapshot.transitions);
    let snapshot = machine::start(&template, &layout, &instance).unwrap();
    let snapshot = machine::dispatch(&template, &layout, &snapshot, "wait").unwrap();
    assert!(machine::is_waiting(&snapshot));
    let resumed = machine::dispatch(
        &template,
        &layout,
        &snapshot,
        &format!("resume:{}", machine::step_state("a")),
    )
    .unwrap();
    assert_eq!(resumed.state.as_str(), machine::step_state("a"));
}
