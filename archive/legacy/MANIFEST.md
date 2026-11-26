# Legacy Archive Manifest

> Phase 1 recorded the freeze point and the source/destination table. Phase 2
> performed the `git mv` and wrote the six-crate cutover. The frontend move and
> the final commit remain outstanding; frontend rows stay `planned` until Phase 4.

## Freeze record

| Field | Value |
|---|---|
| Repository | MetaMind |
| Branch at freeze | `main` |
| Freeze commit | `29614dcf1ac9a48955c21dc47d95b466a60d2823` (`Updates`) |
| Freeze tag | `legacy-freeze-2026-11-25` (annotated, points at the freeze commit) |
| Freeze date | 2026-11-25 |
| Archive owner | core maintainers |
| Replacement design | `design/streamlined_core_four_phase_plan.md` |
| Last commit where legacy built | `29614dc` — `cargo build --workspace` passed |
| Last commit where legacy tests passed | not established: `29614dc` fails four tests (listed below); the tag does not certify a green suite |
| Data root decision | new server uses `data/core/`; legacy `data/` is untouched and stays with the tagged worktree |

The tag is created before any Phase 1 artifact is committed, so it identifies the
pre-cutover tree exactly. Restore the legacy system with:

```bash
git worktree add /tmp/metamind-legacy legacy-freeze-2026-11-25
```

### Baseline at the freeze commit (2026-11-25)

Measured on `29614dc` with a warm `target/` from the same session, Linux, Node 22.

| Check | Command | Result | Duration |
|---|---|---|---|
| Workspace build | `cargo build --workspace` | passed | 69.19 s |
| Workspace tests | `cargo test --workspace --no-fail-fast` | **failed**: 2,297 passed, 4 failed, 2 ignored across 205 test binaries | 542.12 s |
| Frontend typecheck | `npm run typecheck` (`tsc -b --noEmit`) | passed | 40.48 s |
| Frontend tests | `npm run test` (`vitest run`) | passed: 27 files, 236 tests | 154.10 s |
| Frontend build | `npm run build` | passed (bundle budget check included) | 135.40 s |

The four test failures predate Phase 1; no source file was changed to produce
them. They are recorded so Phase 2 does not inherit a false "green baseline"
assumption.

| Failing test | Behavior | Assessment |
|---|---|---|
| `mm-cme --test phase4::the_committed_routing_table_loads_and_pins_valid_models` | expects `explain` to pin `gemini/gemini-2.5-flash`; `config/routing_table.toml` now pins `auto/remote` everywhere | deterministic; stale expectation after the routing-table change in `29614dc` |
| `mm-serve --test phase7::cme_logs_the_turn_and_frames_to_unified_memory` | agentstream has no `AgentMessage` event for the turn | deterministic; the store the Phase 2 cutover replaces |
| `mm-serve --test phase7::cme_exposes_the_conversation_tool_contract` | agentstream has no `ToolCall` event for `conversation.topic` | deterministic; same store |
| `mm-process --test phase5` (one of `parallel_group_runs_concurrently_and_a_failure_fails_the_instance`, `queue_is_deduped_leased_and_bounded`) | revision conflicts / timeouts in the process runner's parallel and lease paths | flaky: a different test fails on each run |

All four live in systems Phase 2 retires (`mm-serve` agentstream assertions,
`mm-process`, and a `mm-cme` test of `config/routing_table.toml`, which is
archived with the routing table).

### Golden fixtures

Captured from the current three-pass path with `mm_llm::MockClient` under
`crates/mm-cme/tests/fixtures/golden/`:

| Fixture | Shape |
|---|---|
| `fast_path.json` | Interpret approves directly; one LLM call, no Process/Verify |
| `action_approved.json` | Interpret registers no checks; Process answers; implicit check judged; Verify approves |
| `action_retry_then_approved.json` | Verify rejects attempt 1; Process retries and stages `data.create`; Verify approves and commits |
| `events/*.jsonl` | The same turns projected into the agentstream v2 envelope Phase 2 must persist |

`crates/mm-cme/tests/golden_capture.rs` regenerates and compares them; run it
with `REGENERATE_GOLDEN=1` only after an intentional contract change. The
checked-in files were compared twice against a fresh capture and matched byte
for byte.

## Status legend

- `planned` — listed, not moved yet.
- `moved` — source path no longer exists outside `archive/legacy/`.
- `kept` — remains in the live workspace.
- `replaced` — behavior now lives in a named kept module.

## Kept workspace

| Crate | Status | Notes |
|---|---|---|
| `mm-core` | kept | trimmed to `config`, `error`, `hash`, `id`, `time` |
| `mm-prompt` | kept | unchanged contract; reduced prompt tree |
| `mm-llm` | kept | transport, mock, schema, redaction; caches/routing archived |
| `mm-agentstream` | kept (new) | JSONL event store, projections, LLM gateway |
| `mm-cme` | kept | Interpret/Process/Verify pipeline, six tools, verification |
| `mm-serve` | kept | turn routes, event routes, health, SSE |

## Retired crates → `archive/legacy/crates/`

| Crate | Status | Replaced by / reason |
|---|---|---|
| `mm-agent` | moved | external agent delegation removed; no `agent.run` |
| `mm-being` | moved | persona frozen into prompt fragments/config |
| `mm-cli` | moved | only the server surface survives |
| `mm-codex` | moved | module metadata system retired |
| `mm-data` | moved | `mm-agentstream` projections + JSONL store; no RDF/SQLite |
| `mm-decision` | moved | no decision models in the turn core |
| `mm-epistemic` | moved | no epistemic subsystem in the turn core |
| `mm-eventlog` | moved | agentstream JSONL is the only event log |
| `mm-firewall` | moved | no mutating tools remain to gate |
| `mm-library` | moved | no library/data tools |
| `mm-log` | moved | `tracing` to stderr; agentstream carries turn logs |
| `mm-memory` | moved | no memory tools or memory tier |
| `mm-metaanalysis` | moved | no meta-analysis subsystem |
| `mm-metacog` | moved | no metacognition subsystem |
| `mm-mistakes` | moved | no mistakes subsystem |
| `mm-pi` | moved | external agent delegation removed |
| `mm-process` | moved | durable process pipeline retired |
| `mm-runtime` | moved | runtime supervisor retired |
| `mm-search` | moved | no full-text search index |
| `mm-selfeng` | moved | self-engineering loop retired |
| `mm-sm` | moved | the simplified pipeline is a sequential function, not SMs |
| `mm-store-graph` | moved | agentstream is JSONL, not RDF |
| `mm-store-sqlite` | moved | no SQLite ledger or migrations |
| `mm-tools` | moved | six turn tools live in `mm-cme`; all others removed |
| `mm-llm-agentstream` | moved | superseded by `mm-agentstream` |

## Retired workspace members → `archive/legacy/`

| Path | Status | Reason |
|---|---|---|
| `modules/system/kernel-bootstrap` | moved | module system retired |
| `modules/cognition/being` | moved | being subsystem retired |
| `modules/cognition/memory-recall` | moved | memory tools retired |
| `modules/cognition/library-query` | moved | library/data tools retired |
| `modules/cognition/frame-activate` | moved | frame engine retired |
| `modules/cognition/metacog` | moved | metacognition retired |
| `modules/cognition/comparison-integrity` | moved | comparison subsystem retired |
| `modules/cognition/calibration` | moved | calibration subsystem retired |
| `modules/cognition/closed-loop` | moved | closed-loop subsystem retired |
| `modules/epistemic/epistemic-ops` | moved | epistemic subsystem retired |
| `vendor/rust_symbolic/decision-ir` | moved | only used by retired crates |
| `vendor/rust_symbolic/logic-ir` | moved | only used by retired crates |
| `vendor/rust_symbolic/logic-types` | moved | only used by retired crates |
| `vendor/rust_symbolic/math-types` | moved | only used by retired crates |
| `vendor/rust_symbolic/math-core` | moved | only used by retired crates |
| `vendor/rust_symbolic/rdf-codec` | moved | only used by retired graph code |
| `vendor/rust_symbolic/rdf-shacl` | moved | only used by retired graph code |
| `tests/e2e` | moved | replaced by core unit/integration tests |
| `bench/` | moved | benchmarks target retired systems |
| `pi/` | moved | Pi agent integration retired |
| `ontology/` | moved | RDF ontology retired |
| `scripts/` | moved | legacy ops scripts; new scripts listed in plan |

## Retired `mm-serve` modules → `archive/legacy/surfaces/mm-serve/`

| Module | Status | Replaced by |
|---|---|---|
| `src/automation.rs` | moved | no background autonomy |
| `src/chat.rs` | moved | `routes/turns.rs` + agentstream projections |
| `src/cme.rs` | moved | `mm-cme::TurnPipeline` |
| `src/cme_stream.rs` | moved | agentstream SSE |
| `src/cm_tools.rs` | moved | `mm-cme::tools` (six tools) |
| `src/controller/` | moved | three-pass pipeline |
| `src/data_tools.rs` | moved | removed with data tools |
| `src/docs.rs` | moved | focus/docs UI retired |
| `src/dto/chat.rs` | moved | `dto/agentstream.rs` |
| `src/dto/focus.rs` | moved | focus UI retired |
| `src/dto/llm_activity.rs` | moved | agentstream event DTO |
| `src/feature_flags.rs` | moved | one path, no rollout flags |
| `src/interjection.rs` | moved | interjections retired |
| `src/llm_feed.rs` | moved | agentstream live bus |
| `src/metamind_tools.rs` | moved | removed with metamind tools |
| `src/parity.rs` | moved | legacy parity harness retired |
| `src/plan_monitor.rs` | moved | plans retired |
| `src/process_runner.rs` | moved | process pipeline retired |
| `src/routes/changesets.rs` | moved | docs changesets retired |
| `src/routes/chat.rs` | moved | `routes/turns.rs` |
| `src/routes/cme.rs` | moved | `routes/turns.rs` + `routes/agentstream.rs` |
| `src/routes/console.rs` | moved | `routes/sessions.rs` + `routes/agentstream.rs` |
| `src/routes/exec.rs` | moved | shell execution retired |
| `src/routes/focus.rs` | moved | focus UI retired |
| `src/routes/llm_activity.rs` | moved | agentstream routes |
| `src/routes/runs.rs` | moved | runtime runs retired |
| `src/routes/trees.rs` | moved | trees/mindmap retired |
| `src/session.rs` | moved | session list in `mm-agentstream` |
| `src/state_service.rs` | moved | state projection in `mm-agentstream` |
| `src/status.rs` | moved | status projections in agentstream |
| `src/status_service.rs` | moved | no dirty-section service |
| `src/tool_schemas.rs` | moved | six schemas in `mm-cme` |
| `src/trees.rs` | moved | trees retired |

`src/main.rs`, `src/lib.rs`, `src/state.rs`, `src/error.rs`, `src/sse.rs`,
`src/metrics.rs`, `src/dto/mod.rs`, and `src/routes/mod.rs` stay, rewritten where
noted in the plan. `src/metrics.rs` keeps minimal counters only; it is not
archived.

## Retired frontend → `archive/legacy/frontend/`

| Path | Status | Reason |
|---|---|---|
| `src/agentgui/` | planned | second chat shell; one shell survives |
| `src/catalog/` | planned | catalog UI retired |
| `src/components/blocks/` | planned | dashboard blocks retired |
| `src/components/charts/` | planned | charts retired |
| `src/components/docs/` | planned | docs UI retired |
| `src/components/files/` | planned | files UI retired |
| `src/components/inspector/` | planned | replaced by agentstream log |
| `src/components/kit/` | planned | only primitives used by kept UI remain |
| `src/components/mindmap/` | planned | mind map retired |
| `src/components/sessions/` | planned | simplified session picker replaces it |
| `src/components/state/` | planned | state board retired |
| `src/components/trees/` | planned | trees retired |
| `src/components/chat/llm-log-*` | planned | replaced by `components/agentstream/` |
| `src/hooks/use-llm-activity-stream.ts` | planned | replaced by `use-agentstream.ts` |
| `src/hooks/use-trace-stream.ts` | planned | agentstream hook |
| `src/hooks/use-status-stream.ts` | planned | no status service |
| `src/lib/api/data.ts` | planned | data UI retired |
| `src/lib/api/files.ts` | planned | files UI retired |
| `src/lib/api/focus*.ts` | planned | focus UI retired |
| `src/lib/api/inspector.ts` | planned | agentstream API |
| `src/lib/api/llm-activity.ts` | planned | agentstream API |
| `src/lib/api/state.ts` | planned | state projection in agentstream |
| `src/lib/api/trees.ts` | planned | trees retired |
| `src/lib/store/llm-log.ts` | planned | `store/agentstream.ts` |
| `src/lib/store/inspector.ts` | planned | agentstream store |
| `src/lib/store/documents.ts` | planned | docs retired |
| `src/lib/store/focus.ts` | planned | focus retired |
| `src/lib/store/pane.ts` | planned | single workspace |
| `src/lib/store/side.ts` | planned | single workspace |
| `src/lib/store/status.ts` | planned | agentstream projections |
| `src/lib/live/` | planned | stage machine for the retired controller pipeline |
| `src/lib/focus/` | planned | focus UI retired |
| `src/lib/inspector/` | planned | agentstream log replaces the inspector projections |
| `src/lib/trees/` | planned | trees UI retired |
| `src/lib/logs.ts` | planned | old log helpers; agentstream store owns log rows |
| `src/lib/transcript/reducer.ts`, `selectors.ts` | planned | ChatFrame reducer replaced by the agentstream event reducer |
| `src/lib/transcript/fixtures/` | planned | new fixtures from JSONL captures |
| `src/lib/api/chat-stream.test.ts` | planned | tested the retired chat SSE route |
| `src/lib/wire/llm_activity.ts` | planned | `wire/agentstream.ts` |
| `src/lib/wire/inspector.ts` | planned | `wire/agentstream.ts` |
| `src/lib/wire/focus.ts` | planned | focus retired |
| `src/lib/wire/fixtures/` | planned | replaced by agentstream event fixtures |
| `src/routes/` | planned | single workspace route |
| `src/catalog/`, `src/components/**` tests | planned | moved with their components |

Kept frontend: `App.tsx`, `main.tsx`, `index.css`, `test-setup.ts`,
`vite-env.d.ts`, `styles/recipes.ts`, the chat pane/composer, the agentstream
dock and prompt viewer, the minimal shell, `lib/commands.ts`, `shortcuts.ts`,
`nav.ts`, `tone.ts`, `contrast.ts`, `utils.ts`, `use-tier.ts`, `hooks/use-now.ts`,
`lib/store/{prefs,session,chat-pending,chat-refs}.ts`,
`lib/api/{agentstream,chat,client,health,sessions}.ts`, `lib/wire/{chat,index}.ts`,
and only the UI primitives they import.

## Retired config → `archive/legacy/config/`

| File | Status | Reason |
|---|---|---|
| `config/agents.toml` | moved | agent delegation retired |
| `config/automation.toml` | moved | background autonomy retired |
| `config/interjections.toml` | moved | interjections retired |
| `config/logging.toml` | moved | audit logger retired |
| `config/metamind.toml` | moved | kernel config reduced to server config |
| `config/process.toml` | moved | process pipeline retired |
| `config/routing_table.toml` | moved | one configured model per pass |
| `config/prompts/applications/*` except the four kept strategies | moved | prompt tree reduced |
| `config/prompts/fragments/*` not named in the plan | moved | prompt tree reduced |
| `config/prompts/frames/` | moved | frame engine retired |

Kept config: `config/llm.toml` (simplified), `config/three_pass.toml`
(simplified), `config/persona.toml` (new), `config/server.toml` (new), and
`config/prompts/{manifest.toml,applications, fragments}` after pruning.

## Retired tests → `archive/legacy/tests/`

| Path | Status | Reason |
|---|---|---|
| `tests/e2e/` | moved | covered retired routes and agent flows |
| `crates/*/tests` and `#[cfg(test)]` of every retired crate | moved | moved with the crate |
| `frontend/src/**/*.test.ts(x)` of retired directories | planned | moved with the component |
| `bench/` | moved | benchmarks target retired systems |

Core test scope after the move is defined in §9 of the plan. The archive is
excluded from `cargo test`, `vitest`, Playwright, and CI.

## Move log

| Date | Commit | Moved | Notes |
|---|---|---|---|
| 2026-11-25 | working tree (uncommitted) | 25 crates, `modules/`, `vendor/`, `tests/e2e`, `bench/`, `pi/`, `ontology/`, legacy `scripts/`, 7 config files, 23 prompt applications, 30 prompt fragments, `config/prompts/frames/`, the legacy `mm-serve` modules, and their tests | `git mv` only; the new code was written after the move. Kept workspace is `mm-core`, `mm-prompt`, `mm-llm`, `mm-agentstream`, `mm-cme`, `mm-serve`. |
