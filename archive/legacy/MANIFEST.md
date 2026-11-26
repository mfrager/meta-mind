# Legacy Archive Manifest

> Populated mechanically in Phase 2. Phase 1 records the freeze point and fills
> the source/destination table; Phase 2 performs `git mv` and records the
> resulting commit. Status is `planned` until the move lands.

## Freeze record

| Field | Value |
|---|---|
| Repository | MetaMind |
| Branch at freeze | `main` |
| Freeze commit | _recorded in Phase 1_ |
| Freeze tag | `legacy-freeze-2026-11-25` |
| Archive owner | core maintainers |
| Replacement design | `design/streamlined_core_four_phase_plan.md` |
| Last commit where legacy built | _same as freeze commit_ |
| Last commit where legacy tests passed | _same as freeze commit_ |

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
| `mm-agent` | planned | external agent delegation removed; no `agent.run` |
| `mm-being` | planned | persona frozen into prompt fragments/config |
| `mm-cli` | planned | only the server surface survives |
| `mm-codex` | planned | module metadata system retired |
| `mm-data` | planned | `mm-agentstream` projections + JSONL store; no RDF/SQLite |
| `mm-decision` | planned | no decision models in the turn core |
| `mm-epistemic` | planned | no epistemic subsystem in the turn core |
| `mm-eventlog` | planned | agentstream JSONL is the only event log |
| `mm-firewall` | planned | no mutating tools remain to gate |
| `mm-library` | planned | no library/data tools |
| `mm-log` | planned | `tracing` to stderr; agentstream carries turn logs |
| `mm-memory` | planned | no memory tools or memory tier |
| `mm-metaanalysis` | planned | no meta-analysis subsystem |
| `mm-metacog` | planned | no metacognition subsystem |
| `mm-mistakes` | planned | no mistakes subsystem |
| `mm-pi` | planned | external agent delegation removed |
| `mm-process` | planned | durable process pipeline retired |
| `mm-runtime` | planned | runtime supervisor retired |
| `mm-search` | planned | no full-text search index |
| `mm-selfeng` | planned | self-engineering loop retired |
| `mm-sm` | planned | the simplified pipeline is a sequential function, not SMs |
| `mm-store-graph` | planned | agentstream is JSONL, not RDF |
| `mm-store-sqlite` | planned | no SQLite ledger or migrations |
| `mm-tools` | planned | six turn tools live in `mm-cme`; all others removed |
| `mm-llm-agentstream` | planned | superseded by `mm-agentstream` |

## Retired workspace members → `archive/legacy/`

| Path | Status | Reason |
|---|---|---|
| `modules/system/kernel-bootstrap` | planned | module system retired |
| `modules/cognition/being` | planned | being subsystem retired |
| `modules/cognition/memory-recall` | planned | memory tools retired |
| `modules/cognition/library-query` | planned | library/data tools retired |
| `modules/cognition/frame-activate` | planned | frame engine retired |
| `modules/cognition/metacog` | planned | metacognition retired |
| `modules/cognition/comparison-integrity` | planned | comparison subsystem retired |
| `modules/cognition/calibration` | planned | calibration subsystem retired |
| `modules/cognition/closed-loop` | planned | closed-loop subsystem retired |
| `modules/epistemic/epistemic-ops` | planned | epistemic subsystem retired |
| `vendor/rust_symbolic/decision-ir` | planned | only used by retired crates |
| `vendor/rust_symbolic/logic-ir` | planned | only used by retired crates |
| `vendor/rust_symbolic/logic-types` | planned | only used by retired crates |
| `vendor/rust_symbolic/math-types` | planned | only used by retired crates |
| `vendor/rust_symbolic/math-core` | planned | only used by retired crates |
| `vendor/rust_symbolic/rdf-codec` | planned | only used by retired graph code |
| `vendor/rust_symbolic/rdf-shacl` | planned | only used by retired graph code |
| `tests/e2e` | planned | replaced by core unit/integration tests |
| `bench/` | planned | benchmarks target retired systems |
| `pi/` | planned | Pi agent integration retired |
| `ontology/` | planned | RDF ontology retired |
| `scripts/` | planned | legacy ops scripts; new scripts listed in plan |

## Retired `mm-serve` modules → `archive/legacy/surfaces/mm-serve/`

| Module | Status | Replaced by |
|---|---|---|
| `src/automation.rs` | planned | no background autonomy |
| `src/chat.rs` | planned | `routes/turns.rs` + agentstream projections |
| `src/cme.rs` | planned | `mm-cme::TurnPipeline` |
| `src/cme_stream.rs` | planned | agentstream SSE |
| `src/cm_tools.rs` | planned | `mm-cme::tools` (six tools) |
| `src/controller/` | planned | three-pass pipeline |
| `src/data_tools.rs` | planned | removed with data tools |
| `src/docs.rs` | planned | focus/docs UI retired |
| `src/dto/chat.rs` | planned | `dto/agentstream.rs` |
| `src/dto/focus.rs` | planned | focus UI retired |
| `src/dto/llm_activity.rs` | planned | agentstream event DTO |
| `src/feature_flags.rs` | planned | one path, no rollout flags |
| `src/interjection.rs` | planned | interjections retired |
| `src/llm_feed.rs` | planned | agentstream live bus |
| `src/metamind_tools.rs` | planned | removed with metamind tools |
| `src/parity.rs` | planned | legacy parity harness retired |
| `src/plan_monitor.rs` | planned | plans retired |
| `src/process_runner.rs` | planned | process pipeline retired |
| `src/routes/changesets.rs` | planned | docs changesets retired |
| `src/routes/chat.rs` | planned | `routes/turns.rs` |
| `src/routes/cme.rs` | planned | `routes/turns.rs` + `routes/agentstream.rs` |
| `src/routes/console.rs` | planned | `routes/sessions.rs` + `routes/agentstream.rs` |
| `src/routes/exec.rs` | planned | shell execution retired |
| `src/routes/focus.rs` | planned | focus UI retired |
| `src/routes/llm_activity.rs` | planned | agentstream routes |
| `src/routes/runs.rs` | planned | runtime runs retired |
| `src/routes/trees.rs` | planned | trees/mindmap retired |
| `src/session.rs` | planned | session list in `mm-agentstream` |
| `src/state_service.rs` | planned | state projection in `mm-agentstream` |
| `src/status.rs` | planned | status projections in agentstream |
| `src/status_service.rs` | planned | no dirty-section service |
| `src/tool_schemas.rs` | planned | six schemas in `mm-cme` |
| `src/trees.rs` | planned | trees retired |
| `src/metrics.rs` | planned | minimal counters only |

`src/main.rs`, `src/lib.rs`, `src/state.rs`, `src/error.rs`, `src/sse.rs`, and
`src/metrics.rs` stay, rewritten where noted in the plan.

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
| `src/lib/transcript/fixtures/` | planned | new fixtures from JSONL captures |
| `src/lib/wire/llm_activity.ts` | planned | `wire/agentstream.ts` |
| `src/lib/wire/inspector.ts` | planned | `wire/agentstream.ts` |
| `src/lib/wire/focus.ts` | planned | focus retired |
| `src/routes/` | planned | single workspace route |
| `src/catalog/`, `src/components/**` tests | planned | moved with their components |

Kept frontend: `App.tsx`, `main.tsx`, `index.css`, the chat pane/composer,
the agentstream dock and prompt viewer, the minimal shell, the API/store/wire
files named in the plan, and only the UI primitives they import.

## Retired config → `archive/legacy/config/`

| File | Status | Reason |
|---|---|---|
| `config/agents.toml` | planned | agent delegation retired |
| `config/automation.toml` | planned | background autonomy retired |
| `config/interjections.toml` | planned | interjections retired |
| `config/logging.toml` | planned | audit logger retired |
| `config/metamind.toml` | planned | kernel config reduced to server config |
| `config/process.toml` | planned | process pipeline retired |
| `config/routing_table.toml` | planned | one configured model per pass |
| `config/prompts/applications/*` except the four kept strategies | planned | prompt tree reduced |
| `config/prompts/fragments/*` not named in the plan | planned | prompt tree reduced |
| `config/prompts/frames/` | planned | frame engine retired |

Kept config: `config/llm.toml` (simplified), `config/three_pass.toml`
(simplified), `config/persona.toml` (new), `config/server.toml` (new), and
`config/prompts/{manifest.toml,applications, fragments}` after pruning.

## Retired tests → `archive/legacy/tests/`

| Path | Status | Reason |
|---|---|---|
| `tests/e2e/` | planned | covered retired routes and agent flows |
| `crates/*/tests` and `#[cfg(test)]` of every retired crate | planned | moved with the crate |
| `frontend/src/**/*.test.ts(x)` of retired directories | planned | moved with the component |
| `bench/` | planned | benchmarks target retired systems |

Core test scope after the move is defined in §9 of the plan. The archive is
excluded from `cargo test`, `vitest`, Playwright, and CI.

## Move log

| Date | Commit | Moved | Notes |
|---|---|---|---|
| _Phase 2_ | _pending_ | _pending_ | _git mv only; no copy_ |
