//! One conversational turn: a short model loop that may call read tools.

use std::sync::Arc;
use std::time::Instant;

use mm_llm::{
    CacheMode, LlmClient, LlmRequest, LlmService, Message, Purpose, SchemaId, ToolSchema,
};
use mm_log::{Level, LogRecord};
use serde_json::{json, Value};

use crate::dto::chat::{ChatFrame, Phase};
use crate::state::AppState;

use super::admit::{self, TurnView};
use super::catalog::{self, GroupId};
use super::exec::{self, ToolResult};
use super::intro::{self, llm_config};
use super::limits::{MAX_MODEL_ROUNDS, MAX_TOOL_STEPS, MODEL_DATA_BYTES};
use super::persona;
use super::prompt;
use super::resolve_client;

pub const NO_MODEL: &str = "I can't call a model yet.";

pub async fn reply(
    app: &AppState,
    request_id: &str,
    session_id: &str,
    query: &str,
    pane: &str,
) -> Vec<ChatFrame> {
    let _ = app.chat.retitle_from_query(session_id, query);
    let facts = intro::facts(app).await;
    let high_stakes = persona::mentions_gate(query);
    let voice = match &app.being {
        Some(being) => {
            let guard = being.lock().await;
            persona::project(guard.personality(), high_stakes)
        }
        None => persona::project(&mm_being::Personality::baseline(), high_stakes),
    };
    let _ = app
        .logger
        .emit(
            LogRecord::new(Level::Info, mm_log::codes::CONTROLLER_TURN, "mm.controller")
                .with_trace_str(request_id)
                .with_field("session_id", session_id)
                .with_field("run_id", request_id)
                .with_field("high_stakes", high_stakes),
        )
        .await;

    let mut frames = vec![
        ChatFrame::Started {
            query: query.into(),
        },
        ChatFrame::Run {
            request_id: request_id.into(),
            session_id: session_id.into(),
            query: query.into(),
        },
    ];

    let cfg = llm_config(app);
    let Some(client) = resolve_client(app, &cfg) else {
        frames.extend(closing(
            request_id,
            &format!("{NO_MODEL} {}", facts.llm_reason),
        ));
        return frames;
    };
    let Some(service) = service(app, client) else {
        frames.extend(closing(
            request_id,
            &format!("{NO_MODEL} {}", facts.llm_reason),
        ));
        return frames;
    };

    let voice_json = json!({
        "warmth": voice.warmth,
        "directness": voice.directness,
        "curiosity": voice.curiosity,
        "playfulness": voice.playfulness,
        "formality": voice.formality,
    });
    let mut groups = catalog::groups_for(query);
    let mut extra_opens = 0u32;
    let mut tool_steps = 0u32;
    let mut rounds = 0u32;
    let mut transcript = vec![Message::user(query)];
    let turn_id = format!("{request_id}-turn");

    let text = loop {
        if app.chat.is_cancelled(request_id).unwrap_or(false) {
            frames.push(ChatFrame::Failed {
                error: "cancelled".into(),
                reason: Some("cancelled".into()),
            });
            return frames;
        }
        rounds += 1;
        let summarizing = rounds > MAX_MODEL_ROUNDS || tool_steps >= MAX_TOOL_STEPS;
        let mut messages = vec![Message::system(standing_prompt(
            app,
            &facts,
            &voice,
            high_stakes,
            &groups,
            pane,
        ))];
        messages.extend(transcript.iter().cloned());
        if summarizing {
            messages.push(Message::user(
                "Summarize what the tools returned and stop.".to_string(),
            ));
        }
        let tools = if summarizing {
            Vec::new()
        } else {
            tool_schemas(&groups)
        };
        frames.push(ChatFrame::LlmRequestStarted {
            turn_id: turn_id.clone(),
            phase: if summarizing {
                Phase::Final
            } else {
                Phase::Act
            },
            subsystem: Some("controller".into()),
            turn_number: rounds,
            visible: true,
            voice: Some(voice_json.clone()),
        });
        let request = LlmRequest::new(Purpose::Converse, messages)
            .with_tools(tools)
            .with_cache(CacheMode::Use)
            .with_trace(parse_trace(request_id));
        let response = match service.call(request).await {
            Ok(response) => response,
            Err(err) => {
                frames.push(ChatFrame::LlmRequestFinished {
                    turn_id: turn_id.clone(),
                    outcome: "error".into(),
                    error: Some(err.to_string()),
                });
                break err.to_string();
            }
        };
        frames.push(ChatFrame::LlmRequestFinished {
            turn_id: turn_id.clone(),
            outcome: "response".into(),
            error: None,
        });
        if summarizing || response.tool_calls.is_empty() {
            let text = response.text.trim();
            break if text.is_empty() {
                "the model returned neither text nor a tool call".into()
            } else {
                text.to_string()
            };
        }
        if !response.text.trim().is_empty() {
            frames.push(ChatFrame::LlmResponse {
                turn_id: turn_id.clone(),
                phase: Phase::Act,
                text: response.text.clone(),
                visible: true,
                is_final: false,
            });
        }
        transcript.push(Message::assistant_calls(
            response.text.clone(),
            response.tool_calls.clone(),
        ));
        for call in &response.tool_calls {
            let input = parse_arguments(&call.arguments);
            let result = if tool_steps >= MAX_TOOL_STEPS {
                let denied = denied_result("tool step cap");
                log_denied(app, request_id, &call.name, "tool step cap").await;
                denied
            } else {
                let view = TurnView {
                    groups: &groups,
                    extra_opens,
                };
                match admit::admit(&view, &call.name, &input) {
                    Ok(_) => {
                        let started = Instant::now();
                        let result = exec::run(app, &call.name, &input).await;
                        if call.name == "catalog.open" {
                            if let Some(id) = result.data.get("group").and_then(Value::as_str) {
                                if let Some(group) = GroupId::parse(id) {
                                    if !groups.contains(&group) {
                                        groups.push(group);
                                        extra_opens += 1;
                                    }
                                }
                            }
                        }
                        extend_groups(&mut groups, &result.summary);
                        log_tool(app, request_id, &call.name, result.ok, started).await;
                        result
                    }
                    Err(reason) => {
                        log_denied(app, request_id, &call.name, &reason).await;
                        denied_result(reason)
                    }
                }
            };
            tool_steps += 1;
            let call_id = format!("{request_id}-{tool_steps}");
            frames.push(ChatFrame::ToolCall {
                id: call_id.clone(),
                turn_id: turn_id.clone(),
                phase: Phase::Act,
                name: call.name.clone(),
                input: input.clone(),
            });
            if result.ok {
                if let Some(focus) = &result.focus {
                    push_focus(app, &mut frames, focus).await;
                }
            }
            frames.push(ChatFrame::ToolResult {
                id: call_id.clone(),
                turn_id: turn_id.clone(),
                name: call.name.clone(),
                success: result.ok,
                output: result.to_value(),
                duration_ms: 0,
            });
            transcript.push(Message::tool(
                call.id.clone(),
                call.name.clone(),
                model_content(&result),
            ));
        }
    };

    frames.extend(closing(request_id, &text));
    frames
}

pub(crate) fn service(app: &AppState, client: Arc<dyn LlmClient>) -> Option<LlmService> {
    let graph = app.graph.as_ref()?;
    let cfg = llm_config(app);
    let mut service = LlmService::new(
        cfg,
        app.sqlite.clone(),
        graph.handle().clone(),
        Arc::clone(&app.logger),
        client,
    )
    .ok()?;
    service
        .register_schema(SchemaId::new("controller.intro"), intro::intro_schema())
        .ok()?;
    Some(service)
}

async fn push_focus(app: &AppState, frames: &mut Vec<ChatFrame>, focus: &Value) {
    let Some(id) = focus.get("id").and_then(Value::as_str) else {
        return;
    };
    if id == "state" {
        frames.push(ChatFrame::Focus {
            doc: json!({ "id": "state" }),
        });
        return;
    }
    if let Ok(doc) = crate::routes::focus::resolve(app, id).await {
        if let Ok(value) = serde_json::to_value(doc) {
            frames.push(ChatFrame::Focus { doc: value });
        }
    }
}

fn standing_prompt(
    _app: &AppState,
    facts: &intro::IntroFacts,
    voice: &mm_being::BehaviorParams,
    high_stakes: bool,
    groups: &[GroupId],
    pane: &str,
) -> String {
    let mut text = prompt::system_prompt(facts, voice, high_stakes);
    text.push('\n');
    text.push_str(pane);
    text.push_str("\nCatalog\n");
    for spec in catalog::table() {
        if groups.contains(&spec.group) {
            text.push_str(&format!("- {}: {}\n", spec.name, spec.summary));
        }
    }
    text.push_str(
        "\nGuide\nTo explain the system, call guide.search or guide.read before answering.\n",
    );
    text
}

fn tool_schemas(groups: &[GroupId]) -> Vec<ToolSchema> {
    catalog::table()
        .iter()
        .filter(|spec| groups.contains(&spec.group))
        .map(|spec| ToolSchema {
            name: spec.name.to_string(),
            description: spec.summary.clone(),
            parameters: spec.input_schema.clone(),
        })
        .collect()
}

fn parse_arguments(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return json!({});
    }
    serde_json::from_str(trimmed).unwrap_or(json!({}))
}

fn denied_result(reason: impl Into<String>) -> ToolResult {
    let reason = reason.into();
    ToolResult {
        ok: false,
        summary: reason.clone(),
        reason: Some(reason),
        data: json!({}),
        focus: None,
    }
}

fn model_content(result: &ToolResult) -> String {
    let data = serde_json::to_string(&result.data).unwrap_or_default();
    let data = if data.len() > MODEL_DATA_BYTES {
        &data[..MODEL_DATA_BYTES]
    } else {
        data.as_str()
    };
    format!("{}\n{data}", result.summary)
}

fn extend_groups(groups: &mut Vec<GroupId>, text: &str) {
    for group in catalog::groups_for(text) {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
}

fn parse_trace(request_id: &str) -> ulid::Ulid {
    mm_core::id::parse_ulid(request_id).unwrap_or_else(|_| ulid::Ulid::nil())
}

fn closing(request_id: &str, text: &str) -> Vec<ChatFrame> {
    vec![
        ChatFrame::FinalResponse {
            response_id: format!("{request_id}-final"),
            text: text.into(),
            blocks: None,
            suggestions: None,
        },
        ChatFrame::Completed {
            subsystems: vec!["controller".into()],
        },
    ]
}

async fn log_tool(app: &AppState, request_id: &str, name: &str, ok: bool, _started: Instant) {
    let _ = app
        .logger
        .emit(
            LogRecord::new(Level::Info, mm_log::codes::CONTROLLER_TOOL, "mm.controller")
                .with_trace_str(request_id)
                .with_field("tool", name)
                .with_field("ok", ok)
                .with_field("run_id", request_id)
                .with_field("trace_id", request_id),
        )
        .await;
}

async fn log_denied(app: &AppState, request_id: &str, name: &str, reason: &str) {
    let _ = app
        .logger
        .emit(
            LogRecord::new(
                Level::Info,
                mm_log::codes::CONTROLLER_DENIED,
                "mm.controller",
            )
            .with_trace_str(request_id)
            .with_field("tool", name)
            .with_field("reason", reason)
            .with_field("run_id", request_id),
        )
        .await;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mm_being::BeingOp;
    use mm_core::{Params, Tabular};
    use mm_llm::{MockClient, ProviderToolCall, ScriptedResponse};

    use crate::controller::intro::{self, SELF_DESCRIPTION};
    use crate::controller::limits::{MAX_MODEL_ROUNDS, MAX_TOOL_STEPS};
    use crate::controller::ControllerModel;
    use crate::dto::chat::ChatFrame;
    use crate::state::{AppState, ConsoleLock};

    async fn booted() -> (tempfile::TempDir, Arc<AppState>) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mm_core::Config::for_data_dir(dir.path());
        let lock = ConsoleLock::acquire(dir.path()).unwrap();
        let state = Arc::new(AppState::boot(cfg, lock).await.unwrap());
        (dir, state)
    }

    fn call(name: &str, arguments: &str) -> ProviderToolCall {
        ProviderToolCall {
            id: format!("call-{name}"),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    fn script(state: &AppState, responses: Vec<ScriptedResponse>) {
        let mock: Arc<dyn mm_llm::LlmClient> = Arc::new(MockClient::script(responses));
        *state.controller_model.lock().unwrap() = ControllerModel::Scripted(mock);
    }

    async fn frames_for(state: &AppState, query: &str) -> Vec<ChatFrame> {
        let (id, _) = state.chat.create_session(state.ids.next_string()).unwrap();
        let request_id = state.ids.next_string();
        super::reply(
            state,
            &request_id,
            &id,
            query,
            "The side pane is showing the state board.",
        )
        .await
    }

    fn final_text(frames: &[ChatFrame]) -> String {
        frames
            .iter()
            .filter_map(|frame| match frame {
                ChatFrame::FinalResponse { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn tool_names(frames: &[ChatFrame]) -> Vec<String> {
        frames
            .iter()
            .filter_map(|frame| match frame {
                ChatFrame::ToolResult { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn warmth_survives_a_reboot() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = mm_core::Config::for_data_dir(dir.path());
        let lock = ConsoleLock::acquire(dir.path()).unwrap();
        let state = AppState::boot(cfg, lock).await.unwrap();
        {
            let mut being = state.being.as_ref().unwrap().lock().await;
            being
                .apply(BeingOp::SetPersonality {
                    key: "warmth".into(),
                    value: 0.2,
                })
                .await
                .unwrap();
        }
        drop(state);
        let cfg = mm_core::Config::for_data_dir(dir.path());
        let lock = ConsoleLock::acquire(dir.path()).unwrap();
        let state = AppState::boot(cfg, lock).await.unwrap();
        let being = state.being.as_ref().unwrap().lock().await;
        let warmth = being.personality().get_disposition("warmth");
        assert!((warmth - 0.2).abs() < 1e-5, "{warmth}");
    }

    #[tokio::test]
    async fn no_provider_keeps_the_intro_and_writes_no_ledger_row() {
        let (_dir, state) = booted().await;
        *state.controller_model.lock().unwrap() = ControllerModel::Off;
        let before = ledger_count(&state).await;
        let (id, _) = state.chat.create_session(state.ids.next_string()).unwrap();
        let text = intro::compose(&state).await;
        assert!(text.contains("The kernel is"));
        assert!(text.contains(SELF_DESCRIPTION));
        let request_id = state.ids.next_string();
        state
            .chat
            .begin(request_id.clone(), id.clone(), String::new())
            .unwrap();
        state
            .chat
            .finish(
                &request_id,
                intro::frames(&request_id, &id, &text),
                "completed",
            )
            .unwrap();
        let reply = reply_text(&state, &id, "hello").await;
        assert!(reply.contains("I can't call a model yet."));
        assert!(
            reply.contains("no language-model provider is configured") || reply.contains("model=")
        );
        assert_eq!(ledger_count(&state).await, before);
        assert!(tool_names(
            &super::reply(
                &state,
                &state.ids.next_string(),
                &id,
                "hello",
                "The side pane is showing the state board.",
            )
            .await,
        )
        .is_empty());
    }

    #[tokio::test]
    async fn invalid_json_and_a_stray_integer_fall_back() {
        let (_dir, state) = booted().await;
        let mock: Arc<dyn mm_llm::LlmClient> = Arc::new(MockClient::constant(
            ScriptedResponse::text("this is not json"),
        ));
        *state.controller_model.lock().unwrap() = ControllerModel::Scripted(mock);
        let text = intro::compose(&state).await;
        assert!(text.contains("The kernel is"));
        assert!(!text.contains("this is not json"));

        let mock: Arc<dyn mm_llm::LlmClient> = Arc::new(MockClient::constant(
            ScriptedResponse::text(r#"{"text":"There are 99 goals."}"#),
        ));
        *state.controller_model.lock().unwrap() = ControllerModel::Scripted(mock);
        let text = intro::compose(&state).await;
        let facts = intro::facts(&state).await;
        if intro::phrasing_keeps_facts("There are 99 goals.", &facts) {
            assert!(text.contains("99"));
        } else {
            assert!(!text.contains("99"), "{text}");
            assert!(text.contains("The kernel is"));
        }
    }

    #[tokio::test]
    async fn a_clean_rephrase_is_what_the_session_stores() {
        let (_dir, state) = booted().await;
        let body = r#"{"text":"Hello. The kernel is ok."}"#;
        let mock: Arc<dyn mm_llm::LlmClient> =
            Arc::new(MockClient::constant(ScriptedResponse::text(body)));
        *state.controller_model.lock().unwrap() = ControllerModel::Scripted(mock);
        let text = intro::compose(&state).await;
        let facts = intro::facts(&state).await;
        if intro::phrasing_keeps_facts("Hello. The kernel is ok.", &facts) {
            assert_eq!(text, "Hello. The kernel is ok.");
        } else {
            assert!(text.contains("The kernel is"));
        }
    }

    #[tokio::test]
    async fn how_do_i_use_this_reads_the_start_chapter() {
        let (_dir, state) = booted().await;
        let answer = "mm-serve holds the graph while it runs. A CLI pointed at the same data directory fails on the RocksDB lock.";
        assert!(answer.chars().count() < 600);
        script(
            &state,
            vec![
                ScriptedResponse::text("")
                    .with_tool_calls(vec![call("guide.read", r#"{"id":"start"}"#)]),
                ScriptedResponse::text(answer),
            ],
        );
        let frames = frames_for(&state, "how do I use this?").await;
        assert_eq!(tool_names(&frames), vec!["guide.read".to_string()]);
        let output = frames.iter().find_map(|frame| match frame {
            ChatFrame::ToolResult { output, .. } => Some(output.clone()),
            _ => None,
        });
        let output = output.expect("tool card");
        assert!(output.to_string().contains("RocksDB lock"));
        let text = final_text(&frames);
        assert!(text.contains("RocksDB lock"));
        assert!(text.chars().count() < 600);
    }

    #[tokio::test]
    async fn an_unwired_tool_is_refused_without_running() {
        let (dir, state) = booted().await;
        script(
            &state,
            vec![
                ScriptedResponse::text("").with_tool_calls(vec![call("loop.run", "{}")]),
                ScriptedResponse::text("loop.run is not wired on this build"),
            ],
        );
        let frames = frames_for(&state, "status please").await;
        let output = frames
            .iter()
            .find_map(|frame| match frame {
                ChatFrame::ToolResult { name, output, .. } if name == "loop.run" => {
                    Some(output.clone())
                }
                _ => None,
            })
            .expect("denial card");
        assert_eq!(output["ok"], false);
        assert!(output["summary"]
            .as_str()
            .unwrap()
            .contains("loop.run is not wired on this build"));
        let log = std::fs::read_to_string(dir.path().join("logs").join("mm.jsonl")).unwrap();
        assert!(log.contains("controller.denied"));
        assert!(!log.contains("\"tool\":\"loop.run\"") || log.contains("controller.denied"));
        assert!(
            log.lines()
                .filter(|line| line.contains("controller.tool") && line.contains("loop.run"))
                .count()
                == 0
        );
    }

    #[tokio::test]
    async fn an_insert_is_refused_as_a_read() {
        let (_dir, state) = booted().await;
        script(
            &state,
            vec![
                ScriptedResponse::text("").with_tool_calls(vec![call(
                    "graph.query",
                    r#"{"query":"INSERT DATA { <http://example.test/s> <http://example.test/p> \"o\" }"}"#,
                )]),
                ScriptedResponse::text("graph queries on this tool are read-only"),
            ],
        );
        let frames = frames_for(&state, "graph").await;
        let output = frames
            .iter()
            .find_map(|frame| match frame {
                ChatFrame::ToolResult { output, .. } => Some(output.clone()),
                _ => None,
            })
            .unwrap();
        assert!(output["reason"]
            .as_str()
            .unwrap()
            .contains("graph queries on this tool are read-only"));
    }

    #[tokio::test]
    async fn opening_kernel_then_doctor_works_and_a_fourth_open_is_refused() {
        let (_dir, state) = booted().await;
        script(
            &state,
            vec![
                ScriptedResponse::text("").with_tool_calls(vec![
                    call("catalog.open", r#"{"group":"kernel"}"#),
                    call("catalog.open", r#"{"group":"being"}"#),
                    call("catalog.open", r#"{"group":"memory"}"#),
                    call("catalog.open", r#"{"group":"epistemic"}"#),
                ]),
                ScriptedResponse::text("").with_tool_calls(vec![call("doctor.run", "{}")]),
                ScriptedResponse::text("Doctor ran."),
            ],
        );
        let frames = frames_for(&state, "status please").await;
        let names = tool_names(&frames);
        assert_eq!(
            names.iter().filter(|name| *name == "catalog.open").count(),
            4
        );
        assert!(names.iter().any(|name| name == "doctor.run"));
        let denied = frames.iter().filter_map(|frame| match frame {
            ChatFrame::ToolResult {
                name,
                output,
                success,
                ..
            } if name == "catalog.open" && !success => Some(output.clone()),
            _ => None,
        });
        let denied: Vec<_> = denied.collect();
        assert_eq!(denied.len(), 1);
        assert!(denied[0]["reason"]
            .as_str()
            .unwrap()
            .contains("at most three extra groups"));
        let doctor = frames.iter().find_map(|frame| match frame {
            ChatFrame::ToolResult {
                name,
                success,
                output,
                ..
            } if name == "doctor.run" => Some((*success, output.clone())),
            _ => None,
        });
        let (ok, output) = doctor.unwrap();
        assert!(ok, "{output}");
    }

    #[tokio::test]
    async fn recall_on_an_empty_store_invents_nothing() {
        let (_dir, state) = booted().await;
        script(
            &state,
            vec![
                ScriptedResponse::text("").with_tool_calls(vec![call(
                    "memory.recall",
                    r#"{"query":"migration order"}"#,
                )]),
                ScriptedResponse::text("No memories matched."),
            ],
        );
        let frames = frames_for(&state, "recall migration order").await;
        let output = frames
            .iter()
            .find_map(|frame| match frame {
                ChatFrame::ToolResult { name, output, .. } if name == "memory.recall" => {
                    Some(output.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(output["ok"], true);
        assert_eq!(output["data"]["hits"].as_array().unwrap().len(), 0);
        assert!(!final_text(&frames).contains("migration order is"));
    }

    #[tokio::test]
    async fn a_tool_call_every_round_stops_at_six_results_then_summarizes() {
        let (dir, state) = booted().await;
        let mock = Arc::new(MockClient::constant(
            ScriptedResponse::text("still going")
                .with_tool_calls(vec![call("guide.list", "{}"), call("guide.list", "{}")]),
        ));
        let client: Arc<dyn mm_llm::LlmClient> = mock.clone();
        *state.controller_model.lock().unwrap() = ControllerModel::Scripted(client);
        let frames = frames_for(&state, "hello").await;
        assert_eq!(tool_names(&frames).len(), MAX_TOOL_STEPS as usize);
        let seen = mock.seen();
        assert!(seen.len() <= MAX_MODEL_ROUNDS as usize + 1);
        assert!(
            seen.last().unwrap().tools.is_empty(),
            "the last call summarizes"
        );
        assert_eq!(seen.iter().filter(|req| req.tools.is_empty()).count(), 1);
        let log = std::fs::read_to_string(dir.path().join("logs").join("mm.jsonl")).unwrap();
        assert!(log.contains("controller.tool"));
        for line in log
            .lines()
            .filter(|line| line.contains("controller.tool") || line.contains("controller.denied"))
        {
            let value: serde_json::Value = serde_json::from_str(line).unwrap();
            let record: mm_log::LogRecord = serde_json::from_value(value).unwrap();
            assert_eq!(record.record_id.len(), 26);
            assert!(!record.event_code.is_empty());
            assert!(record.fields.is_object());
            assert!(mm_log::codes::is_kernel_code(&record.event_code));
        }
    }

    #[tokio::test]
    async fn budget_show_names_the_four_rows() {
        let (_dir, state) = booted().await;
        script(
            &state,
            vec![
                ScriptedResponse::text("")
                    .with_tool_calls(vec![call("catalog.open", r#"{"group":"change"}"#)]),
                ScriptedResponse::text("").with_tool_calls(vec![call("budget.show", "{}")]),
                ScriptedResponse::text("Those are the four budget rows."),
            ],
        );
        let frames = frames_for(&state, "please").await;
        let output = frames
            .iter()
            .find_map(|frame| match frame {
                ChatFrame::ToolResult { name, output, .. } if name == "budget.show" => {
                    Some(output.clone())
                }
                _ => None,
            })
            .unwrap();
        let text = output.to_string();
        for name in ["meta_analysis", "improvement", "evolution", "metacognitive"] {
            assert!(text.contains(name), "{text}");
        }
        let focused = frames.iter().any(|frame| match frame {
            ChatFrame::Focus { doc } => {
                doc.get("id").and_then(serde_json::Value::as_str) == Some("budget")
            }
            _ => false,
        });
        assert!(focused, "budget.show focuses the budgets document");
    }

    #[tokio::test]
    async fn doctor_focuses_the_state_board() {
        let (_dir, state) = booted().await;
        script(
            &state,
            vec![
                ScriptedResponse::text("").with_tool_calls(vec![call("doctor.run", "{}")]),
                ScriptedResponse::text("The kernel is up."),
            ],
        );
        let frames = frames_for(&state, "doctor").await;
        let focused = frames.iter().any(|frame| match frame {
            ChatFrame::Focus { doc } => {
                doc.get("id").and_then(serde_json::Value::as_str) == Some("state")
            }
            _ => false,
        });
        assert!(focused, "doctor.run focuses state");
    }

    async fn reply_text(state: &AppState, session: &str, query: &str) -> String {
        let request_id = state.ids.next_string();
        let frames = super::reply(
            state,
            &request_id,
            session,
            query,
            "The side pane is showing the state board.",
        )
        .await;
        final_text(&frames)
    }

    async fn ledger_count(state: &AppState) -> i64 {
        let rows = state
            .sqlite
            .query_json("SELECT COUNT(*) AS n FROM llm_calls", Params::new())
            .await
            .unwrap_or_default();
        rows.first()
            .and_then(|row| row.get("n"))
            .and_then(|n| n.as_i64().or_else(|| n.as_u64().map(|n| n as i64)))
            .unwrap_or(0)
    }
}
