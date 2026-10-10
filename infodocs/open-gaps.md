# Open gaps — what the twelve-phase build still leaves unbuilt

**Purpose.** This is the honest backlog for `metamind` at the end of the twelve-phase build. It lists
the work that the phase plans asked for and the tree does not have, plus the paths that exist but are
never reached. Nothing in here is a failing build: every phase gate passes, and these are gaps against
the plans' *intent* (and, for G3, against what the ontology promises), not red tests.

**Provenance.** Derived by auditing each full plan's deliverables against the tree — reading the code
behind every claim rather than trusting the plan or the phase gate. State when written:
`HEAD = 0204d73` ("Initial 12 phase build complete"), working tree clean, `codex verify` →
`39 modules conform` (exit 0). Line numbers are from that revision; re-anchor them with the commands in
["How to refresh this list"](#how-to-refresh-this-list) before relying on them.

**Scope.** Only work that still needs a decision or an implementation. Deliberate degradations that are
already recorded in the code are listed separately at the end so that nobody "fixes" them by accident.

## Summary

| # | Gap | Kind | Owner today |
|---|---|---|---|
| G1 | No producer for a distilled decision head — `--core local` is unreachable in practice | Missing implementation | Nobody: Phase 9 deferred it to Phase 11, Phase 11 never picked it up |
| G2 | Factuality: `search_augmented` unimplemented **and** nothing anywhere computes `factuality_support` | Missing implementation + missing wiring | Phase 9 shipped the library; the Phase 10 tool seam (INDEX D3) was never closed |
| G3 | `modules/tools/*` module directories absent; nine tool `module_uri` IRIs dangle | Missing artifacts + plan inconsistency | Phase 10 (plan) |
| G4 | `pi_editor.open` is still a refusing stub; Phase 11 wired `mm-pi` instead | Missing implementation or stale plan | Phase 10 → Phase 11 |
| G5 | Tier-2/3 sandboxes have no runtime (gVisor / microVM) | Missing implementation, operator-installable | Phase 10, env-gated by design |
| G6 | Hosted cores and LLM paths never executed against a live provider | Configuration/verification, not code | Operator |

---

<a id="g1"></a>

## G1 — No producer for a distilled decision head

**What exists**

- `DecisionHead` — a linear head over named features: `crates/mm-decision/src/local.rs:48`, `load`
  (`:66`) and `validate`, with the feature names shared with the question's state.
- `LocalCore` — `unavailable()` (`:166`) and `from_path` (`:183`), where a *missing* file is
  deliberately `Ok(unavailable())` ("no head has been distilled yet").
- The CLI flag: `--head <FILE>` on `decide` (`crates/mm-cli/src/decision_cmd.rs:63`) and on the
  conformance run (`:264`), consumed by `build_core` (`crates/mm-cli/src/decision.rs:259`).

**What is missing**

Nothing fits a head. With no `--head`, `CoreId::Local` is an unavailable core whose reason is the
admission itself: `"no distilled head is loaded; pass --head <file> (Phase 11 fits one)"`
(`crates/mm-cli/src/decision.rs:290`). A repository-wide search for a non-test construction of
`DecisionHead` finds none, and `distill` appears in **no** Phase 11 or Phase 12 plan (full or short
form) — so no later phase owns the training step either.

**What the plans say**

| Plan | Line | Text |
|---|---|---|
| `design/planning/full/phase_09_decision_firewall_build_plan.md` | 53 | "**Out of scope:** distilling frequent decisions into local models (Phase 11)" |
| same | 105 | table row: `LocalCore` — "distilled decision head (Phase 11)" |
| same | 428 | "`LocalCore` adapter (loads a distilled head or returns `Unavailable`)" |
| `design/planning/phase_09_decision_firewall_build_plan.md` | 30, 204, 316 | same deferral, short form |
| `design/planning/implementation_plan1.md` | 823, 1103, 1108 | "Record full traces for replay and later distillation (Phase 11)" |

**The data to fit from already exists.** `decisions` (`crates/mm-store-sqlite/migrations/0009_decision_firewall.sql:53`)
records `question_kind`, `question_json`, `answer_json`, `features_json`, `core_impl` and the outcome,
and is written on every `decide` (`crates/mm-cli/src/decision.rs:437`). That is exactly the trace the
head has to be fitted over.

**Where the implementation belongs.** A fitting module in `mm-decision` (pure, no store dependency)
plus the store read side, driven by a new CLI command — `mm-cli decision fit --class <name>
--out bench/heads/<class>.json` is the shape most consistent with `calibration fit`. Note that
`mm-decision` does **not** depend on `mm-tools`; anything the fitter needs from the world must be
injected at the CLI/loop level, the way the cores are.

**Acceptance criteria**

1. `mm-cli decision fit` writes a head that `LocalCore::from_path` loads and `validate()` accepts.
2. `mm-cli conformance --core local --head <fitted>` passes, and `decide --core local --head <fitted>`
   records a `decisions` row with `core_impl = 'local'`.
3. `decide --core local` **without** a head still refuses with the same honest reason (unchanged).
4. A test fits a head from recorded rows and asserts the answer changes for a question the trace
   supports — i.e. the head does something, not just parse.
5. A contract test pins the feature names in `features_from_state` against the head validator, so a
   head cannot silently read features that no longer exist.

**Decision needed:** this needs either an amendment to the closed Phase 11 plan or a small Phase 13
("follow-ups") document; today it is owned by nobody.

---

<a id="g2"></a>

## G2 — Factuality: `search_augmented` unimplemented, and no producer at all

This is two gaps that share a cause (the INDEX D3 seam to the Phase 10 tools was never closed).

**2a — `FactualityCheck::SearchAugmented` refuses.**

`crates/mm-decision/src/factuality.rs:408` returns
`DecisionError::Unavailable("search-augmented factuality needs the Phase 10 tool seam")`, while
`SelfConsistency` (`:403`) and `AtomicPrecision` are implemented. The plan names the missing recipe
exactly: "decompose → retrieve evidence per atom → judge each atomic fact … `Factuality::SearchAugmented`
verification operation (uses Phase 10 tools)"
(`design/planning/full/phase_09_decision_firewall_build_plan.md:32`; the same seam is recorded as
deferred integration point D3 in `design/planning/full/INDEX.md:78`).

`check_factuality(claim, evidence, check)` takes evidence **by value**, so closing this needs a
retriever seam (a trait implemented over the tool registry), not a direct dependency: `mm-decision`
must not depend on `mm-tools`.

**2b — Nothing computes `factuality_support`, and nothing writes `factuality_checks`.**

- The firewall consumes it as an input: `input.factuality_support` triggers
  `signal.low_factuality` at `crates/mm-firewall/src/scan.rs:674` (field declared at
  `crates/mm-firewall/src/report.rs:247`).
- The only non-`null` producer in the repository is a unit test (`crates/mm-firewall/src/scan.rs:972`);
  the shipped episode template hand-writes `"factuality_support": null` (`bench/ep.json:37`).
- `check_factuality` has **no caller outside its own test module** (`crates/mm-decision/src/factuality.rs:568`,
  `:584`, `:600` are all tests). So even the two implemented checks are library-only.
- The table to persist results exists and is unused: `factuality_checks`
  (`crates/mm-store-sqlite/migrations/0009_decision_firewall.sql:132`, index `:142`), listed as an
  expected table in `crates/mm-store-sqlite/src/pool.rs:320` and asserted by
  `crates/mm-store-sqlite/tests/migrations.rs:97` — with **no `INSERT` anywhere in the workspace**.

**Acceptance criteria**

1. `SearchAugmented` is implemented behind an injected retriever seam; with no retriever configured it
   still refuses with the honest `Unavailable` (the current message is the model for this).
2. An operator path runs a check and persists the row: a `factuality_checks` row with
   `check_kind = 'search_augmented'` (and for the other two kinds as well), mirrored into the
   `/decision` graph per the Phase 9 build sequence.
3. An end-to-end test takes one claim through the CLI: claim → support score → firewall run, and
   asserts `signal.low_factuality` appears for a low score.
4. `graph validate --graph decision` stays at zero violations once real rows exist.

---

<a id="g3"></a>

## G3 — `modules/tools/*` is absent, and nine tool `module_uri` IRIs dangle

**What the plans require.** Both Phase 10 plans and the index name nexus modules for the tool layer:

- `design/planning/phase_10_tools_execution_build_plan.md:73–75` — `modules/tools/fs-read/`,
  `modules/tools/process-exec/`, `modules/tools/verify-build/` (each with `plugin.toml` + src + tests +
  manual), with a manifest sketch at `:238–262` (`name`, stable `uri`, `version`,
  `metadata { category, owned_by_phase, capability }`, `tbox.functions`, `monad.operations`, `build`).
- `design/planning/full/phase_10_tools_execution_build_plan.md:125` — `modules/tools/{fs-read,process-exec,verify-build}/`.
- `design/planning/PHASE_INDEX.md:22` — the same three directories are listed as Phase 10's modules.

**What the tree has instead.** No `modules/tools/` directory at all. The tool specs instead carry a
module IRI string, and there are **nine** distinct ones, not three:

| IRI | Declared at |
|---|---|
| `…/module/tools/fs-read` | `crates/mm-tools/src/tools/fs.rs:81` (helper `:38`) |
| `…/module/tools/fs-write` | `crates/mm-tools/src/tools/fs.rs:158` |
| `…/module/tools/fs-list` | `crates/mm-tools/src/tools/fs.rs:228` |
| `…/module/tools/process-exec` | `crates/mm-tools/src/tools/process.rs:82` |
| `…/module/tools/graph-query` | `crates/mm-tools/src/tools/graph.rs:88` |
| `…/module/tools/tabular-query` | `crates/mm-tools/src/tools/tabular.rs:132` |
| `…/module/tools/http-fetch` | `crates/mm-tools/src/tools/http.rs:139` |
| `…/module/tools/verify-build` | `crates/mm-tools/src/tools/verify_build.rs:67` |
| `…/module/tools/pi-editor` | `crates/mm-tools/src/tools/pi_editor.rs:65` |

(Test fixtures in `spec.rs`, `registry.rs`, `rdf.rs`, `executor.rs` and `mcp.rs` use the same namespace
for synthetic tools such as `fs-write` / `remote-read`.)

Two details matter when you search for these by text. The `fs.*` IRIs are built through the format
helper at `crates/mm-tools/src/tools/fs.rs:38–40`, so `fs-list` never appears as a literal string — it
only exists as `tools/{name}`. A literal search therefore returns the other eight shipped IRIs plus the
fixture name `remote-read` (`crates/mm-tools/src/mcp.rs:511`), which is a test that ingests a *remote*
tool and is not shipped.

**Why it matters.** None of those nine IRIs resolves to a module: `modules/registry.json` lists 37
modules and `codex.lock` 39 `[[module]]` blocks, and neither contains a single `tools/*` entry. The
ontology says this link is the point — `mm:Tool` "can be traced to the code that implements it" via
`mm:moduleUri` (`ontology/tools.ttl:40–43`, `:104–107`) — so at present the traceability claim for the
whole tool layer is empty. Nothing catches it: `ToolSpec::validate` only checks that the string *starts
with* `https://metamind.dev/code/module/` (`crates/mm-tools/src/spec.rs:366`, `:383–390`; the negative
test at `:541` covers a foreign IRI, not a dangling one), and the SHACL shape requires `mm:moduleUri` to
be a string, not to resolve (`ontology/shapes/tools.ttl:91–97`). `codex verify` reads capabilities from
`[package.metadata.metamind]` / `plugin.toml`, never from `ToolSpec.module_uri`, so it cannot see the
dangling references either.

**Two acceptable resolutions — pick one and record it in the plan:**

- **A. Build the modules.** Create one workspace-member module per implemented tool area under
  `modules/tools/`, following the layout of an existing module
  (`modules/cognition/metacog/{plugin.toml, Cargo.toml, src/lib.rs, tests/behavior.rs, manual/module.md}`,
  plus the generated `metadata.ttl`), add them to `[workspace] members`, register them in
  `modules/registry.json`, and refresh `codex.lock`. Acceptance: the module IRIs resolve;
  `codex verify` is green with the new module count; each module names at least one capability with its
  tests; `mm-cli tool list --json` output is unchanged.
- **B. Correct the plans and add the missing rule.** Amend `PHASE_INDEX.md:22` and both Phase 10 plans,
  re-point every `ToolSpec.module_uri` at the module that really implements it
  (`https://metamind.dev/code/module/crates/mm-tools`), and add a `codex verify` rule that a tool
  spec's `module_uri` must resolve to a module record — with a test that a dangling IRI fails. This is
  the smaller change and it removes the possibility of the same rot recurring.

Either way, the plan-versus-tree mismatch is the deliverable: today a reader of `PHASE_INDEX.md` finds
three module directories that do not exist.

---

<a id="g4"></a>

## G4 — `pi_editor.open` is still a refusing stub

**What exists.** `PiEditorTool` (`crates/mm-tools/src/tools/pi_editor.rs`) registers with an accurate
profile — tier `MicroVm`, `Reversibility::Irreversible`, permissions `fs:read:data/sandbox/**` and
`process:execute:pi` — and refuses every call; the refusals are themselves tested (`:136`, `:154`). The
module doc states the intent plainly: "Phase 10 ships the spec and the permission profile; the RPC
client arrives in Phase 11" (`:3–7`), matching the plans
(`design/planning/phase_10_tools_execution_build_plan.md:21`, `:69`, `:295`;
`design/planning/full/phase_10_tools_execution_build_plan.md:44`, `:374`).

**What happened instead.** Phase 11 built a Pi client and drove authoring from the CLI rather than
through the tool: `PiClient::spawn` (`crates/mm-pi/src/rpc.rs:135`, struct at `:105`) is used by
`mm-cli selfeng` (`crates/mm-cli/src/selfeng_cmd.rs:1277–1281`, session ingestion at `:1381`, gated by
`pi_binary_available()` at `:1484`) and by the loop (`crates/mm-runtime/src/loop_controller.rs:1170`).
No Phase 11 or 12 plan mentions `pi_editor`, so the deferral was never honoured or withdrawn.

**Close it one of two ways.**

- Implement the tool against the `mm-pi` client (it must not open a second, divergent path to Pi), and
  accept that a `MicroVm`-tier call still refuses until G5 is resolved; or
- Amend the Phase 10 plan and the tool's module doc to record that authoring is delivered by `mm-pi` +
  `selfeng`, and decide explicitly whether `pi_editor.open` should then be removed from `tool list`
  (the stub exists so an operator can see what is installed and why a call fails — keep that property
  whichever way it goes).

---

<a id="g5"></a>

## G5 — The tier-2 and tier-3 sandboxes have no runtime

**What exists.** The tier enum (`crates/mm-tools/src/sandbox/mod.rs:60`), the `SandboxRuntime` trait and
its production default `AbsentRuntime` (`crates/mm-tools/src/sandbox/gvisor.rs:80`, `:93`, `probe` at
`:100`), the microVM lifecycle with the same probe seam (`crates/mm-tools/src/sandbox/microvm.rs:43`),
and the feature flags. `crates/mm-tools/Cargo.toml` records why they are off: "neither a gVisor runtime
nor a Firecracker/E2B endpoint is installed on a developer machine; the lifecycle state machine and its
tests are compiled either way, and only the runtime probe is gated."

**What is missing.** The probes and execution for a real runtime. Every tool at these tiers —
`pi_editor.open` today, anything the escalation ladder promotes later — gets
`SandboxError::TierUnavailable` with the reason.

**Decision.** Either implement a probe + exec path per runtime, or document the operator install
(feature flag plus how the runtime is reached) so the refusal is an actionable instruction rather than
a dead end. Acceptance in both cases: a documented, tested path from "runtime present" to a completed
tier-2/3 action, and no change to the tier-1 behaviour.

---

<a id="g6"></a>

## G6 — Hosted cores and LLM paths are never exercised end-to-end

**What exists.** `OpenAiClient` (`crates/mm-llm/src/openai.rs`) behind the substrate, the hosted
decision core (`crates/mm-decision/src/hosted.rs`), and the CLI wiring that degrades honestly:
`build_core`'s hosted arm (`crates/mm-cli/src/decision.rs:293`) returns
`BuiltCore::unavailable(id, reason)` when the provider cannot be constructed, and the firewall's
documented behaviour is to fall back to rules + escalate (`design/planning/phase_09_decision_firewall_build_plan.md:441`,
restated at `design/planning/full/phase_09_decision_firewall_build_plan.md:545`).

**What is missing.** No run against a live provider in this environment: `OPENAI_BASE_URL` and
`OPENAI_API_KEY` are unset, so the hosted decision core (`mm-cli decide --core hosted`) and the metacog
scan's LLM ops are exercised only through mocks and the refusal path. `mm-runtime` itself has no LLM
dependency — the loop reaches models through the Pi authoring path (`mm-pi`, which needs the `pi`
binary) rather than by calling a provider. This is not code to write — it is a verification the plan's
gates ask for and this machine cannot perform.

**To close:** configure the two environment variables, then run `mm-cli decide --core hosted`,
`mm-cli conformance --core local --core hosted` and the loop's qualification scenario against the real
endpoint, and record what the redaction/accounting checks observed on real traffic.

---

## Deliberate degradations — recorded on purpose, not gaps

Do not "fix" these without a decision; each is documented at its site and each refuses rather than
fabricates.

| Behaviour | Where | Why it is intentional |
|---|---|---|
| `FormalCheck::Unavailable` — contradiction detection stays structural | `crates/mm-epistemic/src/contradiction.rs:14–20` (module doc), `:84` (enum), `:122–124` (`NoFormalBackend`) | No ASP (`clingo`) or SMT (`Z3`) backend is vendored; the `FormalBackend` seam exists for a later phase. Implementing a solver is real work if wanted, but the current behaviour is a recorded deviation, not an oversight. |
| Tool `module_uri` prefix-only validation | `crates/mm-tools/src/spec.rs:383–390` | Intentional as written — but it is the reason G3 went unnoticed; if G3 is closed with option B this rule becomes the fix. |
| Tier-2/3 refusal, missing head, missing provider → `Unavailable` | see G1, G5, G6 | The firewall "must degrade to rules + escalate, never fabricate" (INDEX D3); an `Unavailable` is escalatable, a plausible invented number is not. |
| `pi_editor.open` refusing on tier **and** irreversibility | `crates/mm-tools/src/tools/pi_editor.rs:9–18` | The profile is the decision; the refusal is the honest report of an unimplemented client. |
| GitHub Actions defined but never executed here | `.github/workflows/ci.yml` | CI cannot run in this environment; every step was run locally instead (see the CI file's own comment). |

---

## How to refresh this list

Run these from the repository root; they reproduce every claim above.

```bash
# G1 — no producer for a head (expect: only the load/validate/use sites, no fitter)
rg -n "DecisionHead" crates --glob '!target'
rg -n -i "distill" design/planning/full/phase_11_self_engineering_build_plan.md \
                     design/planning/full/phase_12_autonomy_build_plan.md   # expect: no hits

# G2 — the refusal, and the absence of any caller or writer
# (scoped to crates/ so these commands do not match this file's own text)
rg -n "Phase 10 tool seam" crates/mm-decision/src/factuality.rs
rg -n "check_factuality\(" crates                      # expect: the definition + its tests only
rg -n "INSERT INTO factuality_checks" crates -g '*.rs' # expect: no hits

# G3 — dangling tool module IRIs vs. the module records that exist
# (expect 9 lines: fs-list only exists through the format helper, and remote-read is a fixture)
rg -o "code/module/tools/[a-z-]+" crates --no-filename | sort -u
grep -c '^\[\[module\]\]' codex.lock ; grep -c '"name":' modules/registry.json
grep -n "tools/" codex.lock modules/registry.json      # expect: no hits

# G4/G5 — the stub and the absent runtimes
rg -n "TierUnavailable|AbsentRuntime" crates/mm-tools/src --glob '!target' | head
sed -n '/\[features\]/,/^\[dependencies\]/p' crates/mm-tools/Cargo.toml

# The gates these gaps do NOT break (must stay green)
cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
mm-cli codex verify && mm-cli logs verify
```

**Keeping this file honest.** A gap leaves this list only when its acceptance criteria pass as written
above and the relevant plan is amended (or a follow-up plan records the decision). If a gap is instead
*withdrawn* — the requirement turns out to be wrong — change the plan first and cite it here, so the
tree and the plans never disagree silently again.
