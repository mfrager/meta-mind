# Durable process pipeline (Phase 5)

Phase 5 adds a second execution path beside a chat turn: a **process** is a
reviewable template with ordered, dependency-aware steps that survives restarts,
waits for people and backends, and reports what happened. The feature is behind
`MM_PROCESS_V1` and is off by default. Chat turns still run through the Phase 4
three-pass runner.

## What is real

- `crates/mm-process`: template validation, PROCESS.md injection, strict
  `{{inputs.x}}` / `{{steps.id.output}}` bindings, an `mm-sm` state machine per
  template, queue ordering with leases and priority aging, the supervisor,
  waits, sub-processes, recovery, version pinning, analytics and export.
- `crates/mm-data/src/process.rs`: templates, frozen template versions,
  instances and step-execution projections in the `process` graph. Every write
  is revision-checked.
- `crates/mm-cme`: `ThreePassRunner::run_step` runs Process → Verify with no
  Interpret pass. The reviewer runs under the Verify scope; it can never write.
- `crates/mm-serve`: lazy supervisor construction after boot, a production step
  runner, additive `metamind.plan`/`metamind.act` fields, inspector routes, and
  a process section in the Plan tab.
- Plan expiry: `expire_due` marks non-durable plans `Expired`; the server sweep
  runs from the process supervisor loop.

## What is deliberately not real

- **Backend service runtime.** `BackendRegistry` is empty. A `backend` step
  returns `backend_unavailable`; it never fakes success.
- **Typed `data.*` CRUD for `process_template`/`process_instance`.** The
  documents are ordinary RDF payloads in the `process` graph and are readable by
  the inspector, `mm-cli data verify` and the process tools. There is no
  `data.create`/`data.update` mapping for process types. Use
  `metamind.act`/`metamind.plan` for writes.
- **A dedicated Process tab.** The Plan tab now shows a plan's process runs,
  expiry/durable chips and pause/resume/cancel. Templates and queue depth are in
  `GET /api/cme/processes`, not a separate tab.
- **Instance inactivity expiry and queue-depth `QUEUE_FULL`.** Plan expiry and
  live-instance caps exist; per-instance inactivity sweeps do not.
- **Output bindings from data writes.** A step's stored reply is its output;
  `{{steps.<id>.output}}` is the binding surface.

## Turn on

```bash
MM_PROCESS_V1=true MM_THREE_PASS_V2=true mm-serve
```

`MM_THREE_PASS_V2` supplies the runner the steps use. With the process flag on,
boot validates `config/process.toml`, the `cm.step.v1` strategy and the
`step_check` fragment before serving. A missing model is a boot error, not a
silent tool-less process.

## Configuration

`config/process.toml` (unknown keys rejected):

- `engine`: tick, lease, batch size, live-instance cap, step timeout, priority
  aging.
- `limits`: steps per template, nesting depth, PROCESS.md characters, instruction
  and output caps, parallel fan-out.
- `plan`: default expiry hours (24), builder step cap, decode/review retries.

A plan without `never_expires` disappears from listings after a day; the
record is retained, never deleted.

## Tools

`metamind.plan` keeps its existing actions and adds:

| Field | Meaning |
|---|---|
| `structure` | `auto`, `flat`, `tree`, `markdown` |
| `persist` / `never_expires` | Exempt from expiry |
| `run` | Compile and start a process after creating the plan |
| `limit` | List cap |
| `last` | Return only the most recently updated non-expired plan |
| `include_expired` | Include expired plans in `list` |

When the pipeline is enabled and `content` is absent, plan creation goes through
the four-pass builder: classify, generate, decode with bounded repair, and a
separate review with one regeneration. Markdown plans take the two-pass path.
The compiled template is saved and linked to the plan.

`metamind.act` adds `process_start`, `process_pause`, `process_resume`,
`process_cancel`, `process_retry_step`, with `template`, `process_id`, `data` and
`step_id`. With the flag off, these return `PROCESS_DISABLED` rather than
silently doing nothing.

## Execution model

- One supervisor per instance. The queue lease is persisted; an expired lease is
  reclaimable. `queue` orders by priority plus one point per aging window.
- Consecutive steps sharing a `parallel_group` run concurrently. Dependencies may
  only point at an earlier group; validation refuses anything else.
- A `human` step waits for `process_resume`; `user_input` waits for the user's
  next input; a backend handle waits as `external`; a future `wake_at` waits as
  `timer`. A child process that waits makes its parent wait.
- Each step has its own request and timeout. Its tool list is the template's
  scope intersected with the Process pass; control tools are rejected at
  validation.
- A failed or timed-out step fails the instance with the provider error text.
  `process_retry_step` resets that step and resumes its group.
- Restart recovery resets `running` steps to `pending`, restores the pinned
  template version and machine snapshot, and starts the instance again. `waiting`
  and `paused` survive untouched.
- Version pinning: an instance keeps the template version it started with even
  after a newer version is saved.

## Observability

- `GET /api/cme/processes?session_id=` returns `{enabled, processes, templates,
  analytics}` (camel-cased). With the flag off it returns `enabled: false`.
- `GET /api/cme/processes/{id}` returns the instance and its pinned template.
- `POST /api/cme/processes/{id}/{pause|resume|cancel|retry}` controls one run.
- Instance events are bounded (200) and monotonically sequenced.
- `POST /api/cme/processes/{id}/{retry}` takes `{step_id}`.
- `mm-cli data verify` now also checks process templates, frozen versions,
  instance template references and step-state completeness.

## Tests

- `crates/mm-process/tests/phase5.rs`: linear and parallel execution, wait/resume,
  queue (25 instances, cap 4), dedupe, leases, timeout, backend stub, recovery,
  sub-process cycle refusal, plan builder repair, expiry, version pinning and the
  machine.
- `crates/mm-cme/tests/phase5_step.rs`: a step runs Process → Verify and a
  rejected step is a typed error.
- `crates/mm-serve/tests/phase5_process.rs`: the inspector's flag-off response and
  strict `metamind.plan`/`act` schema fields.
- `crates/mm-data` unit tests cover template versions and instance projections.

## Known limitations

- The live Phase 4 dual-run and a live Phase 5 plan→process soak are not recorded;
  tests use scripted models and the in-memory graph store.
- Plan creation still uses the legacy planner when the process flag is off, and
  the builder path has no `(session, request_hash)` idempotency cache.
- The state machine is rebuilt from the template layout on recovery; a template
  whose layout changed without a version bump is rejected by validation rather
  than migrated.
