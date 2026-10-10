# Metamind — user guide

> **Audience.** Anyone who wants to build this repository and actually use it: what the system is, how to
> run it, a guided tutorial that works, the command reference, and the practical notes that save an hour
> of confusion. Every command and every output below was run against `HEAD = 0204d73` while writing this
> document.
>
> **Companion documents.** [`components-and-status.md`](./components-and-status.md) — what each component
> can and cannot do, and what is tested. [`open-gaps.md`](./open-gaps.md) — what is not implemented yet.

---

## 1. What Metamind is

Metamind is a **cognitive kernel that can modify itself under a gate**. It is a single Rust workspace plus
a local datastore, driven by one CLI. It keeps a persistent being (identity, goals, beliefs, affect,
relationships), a long-term memory, an epistemic ledger that separates observation from assumption, a
library of techniques and policies, a metacognitive controller, a bounded decision core, a sanity firewall,
a sandboxed tool layer, and a self-engineering loop that can write, test, and promote new capabilities —
with **no human code edits in the loop**.

The invariant that shapes everything you will see:

> The LLM proposes; the control plane structures; the decision core makes bounded judgments; symbolic
> layers prove; deterministic code enforces; the environment decides truth; memory preserves;
> meta-analysis improves. **Production is written only by promotion.**

Two practical consequences:

1. **Nothing invents an answer.** A core with no model, a factuality check with no retriever, a sandbox
   with no runtime — each returns `Unavailable` *with a reason*, and callers escalate. You will see this
   constantly; it is the system working, not failing.
2. **Nothing writes production except a promotion.** Changes go change set → sandbox → benchmark → gate →
   promotion → hot-load, and every promotion carries a written reason.

### What you can do today

| You want to… | Use |
|---|---|
| Boot the kernel and prove it is healthy | `doctor`, `logs verify`, `replay` |
| See who the being is, and add a goal | `being show`, `being goal add`, `being verify` |
| Store and retrieve memories | `memory add`, `memory recall`, `memory eval` |
| Record a claim, or a mistake and its rule | `epistemic ingest`, `memory mistake add` |
| Browse the technique/policy/library corpus | `library import`, `library applicable`, `policy history` |
| Deliberate over an episode | `episode run`, `episode verify` |
| Ask one bounded question | `decide` (three cores) |
| Check an action before taking it | `firewall eval`, `compare check`, `risk analyze` |
| Run a tool safely, then undo it | `tool run`, `action ledger`, `action rollback` |
| Prove a build/test/lint obligation | `verify run` |
| Turn a bug into a permanent regression test | `regression run` |
| Take a change from idea to production | `changeset new`, `sandbox run`, `promote`, `audit production-tree` |
| Run the whole autonomous loop | `loop run`, `loop status`, `loop replay`, `self-model report` |
| Find architectural debt, and collect it | `debt scan`, `gc --apply` |
| Hot-load a promoted capability (no restart) | `module load`, `module rollback` |

---

## 2. Requirements and build

**System dependencies** (from CI, `.github/workflows/ci.yml`): a C toolchain, `pkg-config`, `clang`,
`libsqlite3-dev` and `librocksdb-dev`. RocksDB is needed because the RDF store is built with
`rocksdb-pkg-config`; SQLite comes through `sqlx`.

```bash
sudo apt-get update
sudo apt-get install -y --no-install-recommends build-essential pkg-config clang libsqlite3-dev librocksdb-dev
```

**Toolchain:** pinned by `rust-toolchain.toml` — just `rustup show` and Cargo will pick it up.

```bash
cd ~/Build/metamind
cargo build --workspace                  # ~2–5 min cold, seconds warm
cargo test --workspace                   # 1747 tests, 152 binaries, 0 failures
```

Optional but recommended before you trust a checkout:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The CLI is `cargo run -q -p mm-cli -- <args>`; after a build you can also call `./target/debug/mm-cli`
directly. This guide writes `mm-cli` for brevity and shows `--data-dir` in every scratch example.

**Pi (the code agent)** is optional for everything except live authoring: `pi` on `PATH` (this machine has
`pi 0.85.1`), plus a provider name for live runs. The offline replay path needs neither.

---

## 3. Concepts you need

| Concept | What it means in practice |
|---|---|
| **Kernel** | The thing `doctor` boots: config → SQLite migrations → RocksDB graph → ontology → event log. Everything else is a client of it. |
| **Data dir** | Where state lives: `metamind.db`, `graph/` (RocksDB), `logs/mm.jsonl`, `ulid.watermark`. Passed with `--data-dir`. |
| **Event log** | The authoritative history. State is a fold over committed events, so `replay` reproduces a state hash. There is no mutable "state table" you can quietly edit. |
| **Named graphs** | 11 RDF graphs (`/being`, `/memory`, `/epistemic`, `/world`, `/library`, `/code`, `/decision`, `/tools`, `/selfeng`, `/provenance`, `/self`). `/world` is restricted to OBSERVED/VERIFIED claims. |
| **ULIDs** | Lowercase Crockford identifiers, e.g. `01m4hvpt019ppr5ygt5b7695jz`. Everything is addressed by one; IRIs are `https://metamind.dev/data/{ulid}`. |
| **Promotion** | The only door to production. `promote` runs the gate and records a decision with a written reason; `audit production-tree` proves nothing else changed. |
| **Change set** | A typed, reversible patch list built from a gap: what changes, its hypothesis, and its rollback. |
| **Sandbox** | A git worktree the candidate is applied to and audited, with `--assert-isolated` proving nothing outside it moved. |
| **Epistemic status** | Claims move through statuses monotonically and only with evidence; assumptions carry a verification priority. |
| **Budgets** | Hard caps, not targets: `meta_analysis`, `improvement`, `evolution`, `metacognitive`, plus `wall_ms`. A stage that would cross its bucket is denied whole. |
| **Firewall** | The pre-action check: deterministic prohibitions first, bounded judgments second, `VERIFY_FIRST`/`HUMAN_REVIEW` when uncertain. |
| **Conformance** | One suite that every decision core must pass, so `rules`, `local` and `hosted` are comparable. |
| **Tools** | The only way out of the process: 9 registered tools, default-deny permissions, tiered sandboxes, exactly-once execution, an append-only ledger. |
| **Nexus module** | A capability crate with a `plugin.toml`, a stable `uri`, a capability name, T-Box functions, tests, and a manual. |
| **Hot-load** | The runtime switches its active module-version pointer without restarting the process. |

---

## 4. Configuration

### 4.1 The config file

`config/metamind.toml` (every path is relative to the repository root, so it works from any cwd):

```toml
[kernel]
name = "metamind"
codename = "mm"

[store]
data_dir = "data"
sqlite_file = "data/metamind.db"
graph_dir = "data/graph"

[log]
level = "info"
dir = "data/logs"
console = true
jsonl = true
```

Select it explicitly with `--config <PATH>` (global) or `$MM_CONFIG`. Note that a *relative* `data_dir`
is resolved against the repository root, and `--data-dir` overrides it.

### 4.2 Environment variables

| Variable | Effect |
|---|---|
| `MM_CONFIG` | Path to the config file, instead of `config/metamind.toml`. |
| `OPENAI_BASE_URL`, `OPENAI_API_KEY` | **Both** are required for any hosted model path: the hosted decision core, the metacog scan, live Pi authoring. With either missing, those paths report `Unavailable` and say which variables are missing. |
| `MM_DATA_DIR` | Documented as a data-dir override, and implemented in `mm-core` (`config.rs:379`) — but **no caller applies it**, so it has no effect on the CLI. Use `--data-dir` instead. |
| `MM_CLI_BIN` | Test-harness override for which CLI binary the e2e tests drive. Not needed for normal use. |
| `MM_OXIGRAPH_VERSION`, `MM_CODEX_LOG_SOURCE`, `MM_SUBJECT_SCRIPT`, `MM_INCIDENT`, `MM_LOG` | Internal/test knobs used by specific fixtures. |

**Recommended habit:** use `--data-dir $(mktemp -d)` for anything exploratory, so the repository's `data/`
stays a known-good state:

```bash
DATA=$(mktemp -d)
mm-cli --data-dir "$DATA" doctor
```

### 4.3 Where state lives

```
$DATA/
├── metamind.db      # SQLite: 110 tables, WAL
├── graph/           # Oxigraph/RocksDB: the named graphs
├── logs/mm.jsonl    # every structured record
└── ulid.watermark    # monotonic ULID safety net
```

The *tool* sandbox is separate and always the repository's `data/sandbox` — not your `--data-dir`. That is
deliberate (the tools are sandboxed against the working tree) but surprising the first time you see
`fs.write` land in the repo.

---

## 5. First run: a five-minute tour

Everything below is copy-pasteable. `DATA` is your scratch data dir.

```bash
cd ~/Build/metamind
cargo build --workspace
DATA=$(mktemp -d)
mm-cli --data-dir "$DATA" doctor
```

Real output of that last line:

```
mm-cli 0.1.0 starting kernel self-check
  codename            mm
  data dir            /tmp/tmp.qLrIDBY61E
  sqlite              3.46.0 (journal_mode=wal)
  database            /tmp/tmp.qLrIDBY61E/metamind.db
  migrations applied  [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
  oxigraph            0.5.11
  graph dir           /tmp/tmp.qLrIDBY61E/graph
  graph invariants    ok
  ontology triples    1450 (1450 loaded this run)
  total quads         1450
  named graphs        (none yet)
  events              head seq 2, committed 2, provisional 0, aborted 0
  audit chain         2 record(s): 2 records, chain intact
kernel ready
```

Read that as a checklist: twelve migrations, a writable graph store, the ontology loaded, the event log
open, the audit chain intact.

### 5.1 The being

```bash
mm-cli --data-dir "$DATA" being show
mm-cli --data-dir "$DATA" being goal add --description "ship a low-latency memory recall path" --priority 0.8
mm-cli --data-dir "$DATA" being verify
```

`being show` prints identity, version, description, the four invariants, core blocks, affect (seven
dimensions), goal/commitment counts and resource accounts. `being verify` runs the invariant suite, the
adversarial corpus and the SHACL shapes:

```
being verify
  corpus         .../bench/being/invariants.jsonl
  invariants     4 enforced
  adversarial    22 denied of 22
  terminal rows  2 of 2 trigger(s)
  budget         reconciled
  observations   0 unbacked
  affect         0 flag violation(s)
  shacl /being   0 violation(s) against .../ontology/shapes/being.shacl.ttl
```

### 5.2 Memory

```bash
mm-cli --data-dir "$DATA" memory add \
  --content 'the migration order is 0001 through 0012 and never repeats' --kind semantic
mm-cli --data-dir "$DATA" memory recall 'migration order' --k 3
```

```
memory recall (1 hit(s))
   1. 0.256562  01m4hvpt4mewnd5y97cr0jexac  semantic
      lexical=0.000 vector=0.217 graph=0.000 recency=0.100 importance=0.025
      the migration order is 0001 through 0012 and never repeats
```

The five score terms are the retrieval model, shown rather than hidden. `memory verify` reconciles SQLite
against `/memory`; `memory forget` is a dry run unless you ask otherwise; `memory eval` grades against the
committed gold set and exits non-zero below its thresholds.

### 5.3 A bounded question

```bash
mm-cli --data-dir "$DATA" decide \
  --question '{"kind":"yes_no","prompt":"is the migration idempotent?"}' --json
```

```json
{
  "admitted": false,
  "answer": "bool:false",
  "available": true,
  "calibrated": false,
  "confidence": 0.20000000298023224,
  "core": "rules",
  "decision_id": "01m4hvptba6b5b3h4j90bk9psb",
  "features": { "rules.declared_confidence": 0.20000000298023224, "rules.fired": 0.0 }
}
```

`admitted: false` is the conformal-abstention rule: a class with no fitted threshold admits nothing. Fit
one with `calibration conformal`, then re-run with `--calibrate`.

Now see what the other two cores say:

```bash
mm-cli --data-dir "$DATA" conformance
```

```
conformance: rules 6 passed, 0 failed
conformance: local not run (unavailable)
conformance: hosted not run (unavailable)
  unavailable local: no distilled head is loaded; pass --head <file> (Phase 11 fits one)
  unavailable hosted: llm provider is not configured: OPENAI_BASE_URL and OPENAI_API_KEY must both be set
```

This is the honest-degradation rule in action: unavailable cores are **not** counted as passes.

### 5.4 Tools: run, deny, undo

```bash
mm-cli --data-dir "$DATA" tool list
mm-cli --data-dir "$DATA" tool describe fs.read
mm-cli --data-dir "$DATA" tool run fs.write --arg path=data/sandbox/hello.txt --arg text=hello \
  --idempotency-key k1 --json
mm-cli --data-dir "$DATA" tool run fs.read  --arg path=data/sandbox/hello.txt --json
mm-cli --data-dir "$DATA" tool run process.exec --arg cmd='cargo test'      # denied, with a reason
mm-cli --data-dir "$DATA" action ledger --verify

# byte-identical undo: write twice to the same path, then roll the newest action back
AID=$(mm-cli --data-dir "$DATA" tool run fs.write --arg path=data/sandbox/hello.txt \
        --arg text=again --json | jq -r .action_id)
mm-cli --data-dir "$DATA" action rollback "$AID" --json
```

Rollback restores a snapshot, so it needs a *previous* version of the file: the second write above can be
rolled back, while the write that *created* the file has no snapshot and refuses with `no snapshot … is
recorded`. That refusal is the correct answer, not a bug.

The write returns the action id, the output hash, the observation id and the evidence id; repeating it
with the same idempotency key returns `deduplicated: true`. The denial is the interesting one:

```
status          denied
action_id       01m4hvqh9tz6fn4ewy9851pxm5
sandbox tier    WasmCaps
reason          no grant covers process.exec:process:execute:cargo
```

Authorization is a deterministic table, never a model judgment. Ask the same engine directly with
`policy check --principal <id> --action fs.write --resource data/sandbox/x`.

### 5.5 Self-engineering: change set → sandbox → promote

```bash
CS=$(mm-cli --data-dir "$DATA" changeset new --from-gap bench/gaps/gap_01.json --json | jq -r .changeset)
mm-cli --data-dir "$DATA" sandbox run "$CS" --apply --assert-isolated --json
mm-cli --data-dir "$DATA" promote "$CS" --assert-reason-present --json
mm-cli --data-dir "$DATA" budget show
mm-cli --data-dir "$DATA" audit production-tree --json
```

```json
{ "changeset": "01m4hvqgdch5b7kn7spvt6m67k",
  "decision": "promote",
  "promotion": "01m4hvqgv8t995wnha304n60s6",
  "reason": "the gate found nothing to reject",
  "self_version": "self-1-f614308166df",
  "status": "promoted" }
```

`audit production-tree` then reports `promotions: 1`, `journal: 1`, `outside: []` — proof that the only
thing written was the promotion. A `reject` is equally recorded: it *requires* `--reason`, because a
rejection without one is not a decision.

### 5.6 The flagship run: the whole loop

```bash
RUN=$(mm-cli --data-dir "$DATA" loop run \
        --goal-file bench/qualification/goal_novel.json \
        --budget-file bench/qualification/budget.toml --novel --json | jq -r .run_id)

mm-cli --data-dir "$DATA" loop status    --run "$RUN"
mm-cli --data-dir "$DATA" loop artifacts --run "$RUN"
mm-cli --data-dir "$DATA" loop replay    --run "$RUN"
mm-cli --data-dir "$DATA" self-model report --run "$RUN"
mm-cli --data-dir "$DATA" debt scan
mm-cli --data-dir "$DATA" gc --json
mm-cli --data-dir "$DATA" module load \
  --uri https://metamind.dev/code/module/cognition/goal-attainment@0.1.0 --json
mm-cli --data-dir "$DATA" logs verify
```

The goal is *novel* (no module or fixture already provides goal-attainment progress), so the loop has to
build the capability. `loop status` walks the ten stages:

```
  0  experience       ok  episode 01h0000000000000000000e110 trigger=repeated_failure attempts=3
  1  event_log        ok  committed the run's inputs as event 01m4hvqk72zbt2r4rh5g9sr1g2
  2  meta_analysis    ok  analysis 01m4hvqk764mb4mty9e7z0zf91 trigger=repeated_failure
  3  capability_gap   ok  capability gap on modules/cognition/goal-attainment
  4  change_set       ok  change set 01m4hvqk7bg07srp2nhhb59zrz with 7 artifact(s)
  5  self_engineering ok  materialised 5 file(s) under .../sandbox/loop/<run> and ingested <pi session>
  6  test_benchmark   ok  qualification_bench: 1 case(s), 0 failed; candidate=1.0 baseline=1.0 within_noise=true
  7  shadow           ok
  8  promotion_gate   ok  promote: the gate found nothing to reject (self_version self-2-a0da1d2bbb77)
  9  new_version      ok  hot-loaded .../goal-attainment@0.1.0 (1 function); design revision 1
```

`loop replay` recomputes the digest and finds it identical:

```
recorded          9d8b545fd7fac29215f772b852fcd18966999fa481beaa0eb7a94dea9c7cb153
recomputed        9d8b545fd7fac29215f772b852fcd18966999fa481beaa0eb7a94dea9c7cb153
identical         true
```

`self-model report` gives the numbers (`actual_model 0.2`, `actual_ideal 0.0`, `model_ideal 0.2`),
`debt scan` finds six findings, and `gc` proposes five interventions while refusing the one subject that
is part of the immutable developmental ledger:

```json
{"applied":0,"dry_run":true,"findings":6,"proposed":5,"protected":1,"protected_untouched":true,
 "refusals":[{"action":"data","subject_uri":"https://metamind.dev/data/01h0000000000000000000db02",
 "reason":"gc refused …: the subject is part of the immutable developmental ledger"}],"refused":1}
```

Finally `module load` puts the promoted version live in the running process, and `logs verify` proves the
whole session is still internally consistent (ten checks, all `[ok]`).

This run takes well under a second because the Pi stage replays the recorded session named by the goal
(`crates/mm-pi/fixtures/recorded_session_qualification_novel.jsonl`) — deterministic, no provider, no
network.

---

## 6. Guided tours by capability

### 6.1 Code metadata (`codex`)

```bash
mm-cli codex scan                       # rebuild /code, regenerate modules/registry.json
mm-cli codex verify                     # every rule; 39 modules conform
mm-cli codex graph | head               # the module dependency graph
mm-cli codex meta --query ModulesPerPhase
mm-cli codex lock --check               # is codex.lock current?
```

After a deliberate change to a module, bump its version and run `mm-cli codex lock`, then commit
`codex.lock` with the change. A forgotten version bump is caught by `codex verify`.

### 6.2 The LLM substrate (`llm`)

```bash
mm-cli llm stats          # aggregate the call ledger; every row complete
mm-cli llm routes         # which model the router would choose for a labelled request
mm-cli llm grammars       # compile every fixture schema with every decoder backend
mm-cli llm schema-reject  # every malformed fixture is rejected, for the right reason
mm-cli llm verify-cache   # re-hash each cached body against its index row
```

These all run offline. A real call needs the provider variables; without them the substrate reports
`Unavailable` rather than guessing.

### 6.3 Epistemic discipline

```bash
mm-cli epistemic data-tests        # RDFUnit-style data-quality suite over /epistemic
mm-cli epistemic assumptions       # the assumption ledger, highest verification priority first
mm-cli epistemic promotable        # the deterministic promotion table over a fixture
mm-cli epistemic world --query 'SELECT ?s ?o WHERE { GRAPH <https://metamind.dev/graph/world> { ?s a ?o } } LIMIT 3'
```

Two things to know about `epistemic world`: `--query` is required (omitting it is an argument error, not
an empty result), and the query must be scoped to the world graph with `GRAPH <…/graph/world> { … }` or
`FROM <…/graph/world>` — an unscoped query is refused rather than silently answered from another graph.

### 6.4 Library, policies, frames, skills

```bash
mm-cli library import ontology/seed/library_seed.ttl   # 21 entries accepted, 0 violations
mm-cli library validate
mm-cli library applicable --state bench/library/state_uncertain_strategy.json \
  --gold bench/library/gold_applicability.jsonl
mm-cli library list --kind policy
mm-cli case retrieve --problem bench/library/gold_cases.jsonl --top 2
mm-cli frame compose --frames debugging,software --episode 01h00000000000000000000090
mm-cli experience compile --trajectories bench/library/trajectories/success.jsonl --out "$DATA/drafts.jsonl"
mm-cli policy history https://metamind.dev/policy/prefer_simpler_solution@1
```

Observed on the shipped fixtures: `library applicable` ranks five techniques against the gold table;
`case retrieve` maps three problems to precedents (`outage-rollback` at 0.306, `schema-migration-regression`
at 0.182); `frame compose` reports `known 0 of 6, missing 6`, which is what a fresh kernel should say;
`experience compile` writes draft candidates (2 techniques, 2 policies, 2 lessons) and promotes nothing.

**Registering a skill is a three-step flow — register, verify, retrieve — and the seed already contains
one:**

```bash
# in a fresh data dir, before importing the seed
mm-cli skill register --manifest bench/library/skills/manifest.json
mm-cli skill verify 'https://metamind.dev/library/skill/api-capability-check@1'
mm-cli skill retrieve --query 'confirm an external api behaves as documented'
```

```
registered https://metamind.dev/library/skill/api-capability-check@1
  verification  draft

skill         https://metamind.dev/library/skill/api-capability-check@1
  artefact      data/library/skills/api-capability-check.wasm
  hash          matches the pinned sidecar
  spec          bench/library/skills/api-capability-check.test.json (4/4)
  verification  verified

1.000 verified https://metamind.dev/library/skill/api-capability-check@1 API capability check
```

A skill starts as a `draft` and `retrieve` ignores drafts, so the middle step is what makes it
findable. After `library import ontology/seed/library_seed.ttl` the same `register` refuses with a
duplicate error, because the seed ships `library/skill/api-capability-check@1`
(`ontology/seed/library_seed.ttl:198`) — correct deduplication with an unhelpful error class (see the
troubleshooting table). Importing the seed twice behaves differently: it *rejects* the duplicates and
reports `0 accepted, 21 rejected`, which is the intended idempotence.

### 6.5 Deliberation and judgment

```bash
# the gold table covers the whole corpus, so run all of it into one artifact dir
mm-cli episode run --input bench/episodes --out "$DATA/artifacts"
mm-cli episode verify --gold bench/episodes/gold --artifacts "$DATA/artifacts"
mm-cli episode value --gold-table bench/episodes/operation_value.csv
mm-cli episode budget-audit --thresholds bench/episodes/thresholds.json --artifacts "$DATA/artifacts"
mm-cli episode program --episode 01h00000000000000000000015 --artifacts "$DATA/artifacts"
mm-cli firewall eval --episode bench/ep.json --no-record
mm-cli compare check --fixtures bench/comparison/fixtures.jsonl
mm-cli risk analyze --outcomes bench/risk/reference_values.json
mm-cli calibration fit --labeled bench/calibration/labeled.jsonl
mm-cli calibrate --bench bench/calibration/labeled.jsonl --assert-improves-baseline
```

Observed on the shipped fixtures: 9 episodes run and `episode verify` finds `0 difference(s), gold
matches exactly`; `episode value` matches all 10 rows of the operation-value table; `episode budget-audit`
reports 0 of 9 over budget; 13 comparison fixtures with 0 mismatches; 4 risk cases with 0 field
mismatches at tolerance 1e-6; calibration fits `T = 2.125` on 400 points and drops ECE from 0.087 to
0.012 in both classes; conformal fitting reaches 0.910 held-out coverage against a 0.90 ± 0.03 target
across 5 classes.

If you run only *part* of the corpus, `episode verify` fails on purpose (`the gold holds 9 programs; the
artifacts hold 3`). Run the whole directory, or point `--gold` at a matching subset.

### 6.6 Verification obligations

```bash
mm-cli verify run cargo-build     --subject mm-core --json
mm-cli verify run cargo-test      --subject mm-core
mm-cli verify run static-analysis --subject mm-core
```

The subject is a **package name** (or something `cargo` accepts as one). Hand it a file path and the
obligation is honestly *refuted* rather than fudged:

```
"kind": "static_analysis", "subject": "crates/mm-core/src/lib.rs",
"verdict": "refuted", "detail": "cargo exited 101",
"counterexample": "exit code 101: error: package ID specification `crates/mm-core/src/lib.rs` looks like a file path, …"
```

```
{ "obligation": { "kind": "cargo_build", "subject": "mm-core" },
  "verdict": "proven", "detail": "cargo exited 0", "artifact_hash": "e3b0c442…" }
```

### 6.7 Self-observation, mistakes, and budgets

```bash
mm-cli meta analyze --episode bench/episodes/seeded_failure_01.json --assert-lessons-ge 1
mm-cli meta queue ; mm-cli meta lessons
mm-cli regression run --suite bench/regression
mm-cli budget show
```

`meta analyze` on the seeded failure extracts one lesson (`lesson_conf 0.7`) with a five-class diagnosis;
`regression run` proves the seeded bug *fails before and passes after*.

### 6.8 Pi authoring

```bash
mm-cli pi run --task bench/pi/module_scaffold_01.json --offline
```

```
mode            recorded
session_file    .../crates/mm-pi/fixtures/recorded_session_module_scaffold_01.jsonl
events          8
edits           3
tool_calls      2
triples         30
turtle_hash     a3523f40e51263993f9560ba027619d39581c618cf3fce8dc74ab684109b3ec8
```

Drop `--offline` and pass `--provider <name> [--model provider/id] [--binary <path>]` to drive the real
`pi` binary; that requires a configured provider.

### 6.9 Diagnostics: logs, replay, audit

```bash
mm-cli logs tail --help ; mm-cli logs trace --help
mm-cli logs verify
mm-cli replay --from 0
mm-cli graph validate --graph being
mm-cli audit production-tree --assert-unmodified-outside-promotion
```

`logs verify` is the single most useful command when something looks wrong: it re-checks the audit chain,
audit completeness, event sequencing, record schema, redaction, replay determinism, and the correlation
between logs and the being/memory/epistemic tables.

---

## 7. Command reference

42 subcommands. Run any of them with `--help` for flags; all accept the global `--config` and
`--data-dir`, and most accept `--json`.

| Command | What it does |
|---|---|
| `doctor` | Open every store, load the ontology, prove the kernel is writable |
| `replay` | Rebuild state from the event log and report the state hash |
| `graph validate` | SHACL-validate a named graph; non-zero on any violation |
| `logs verify` / `logs tail` / `logs trace` | Verify the streams; print the last N; print one trace |
| `codex scan` / `verify` / `graph` / `diff` / `meta` / `lock` | Code metadata: rebuild, check, render, drift, query, lock |
| `llm stats` / `replay` / `verify-cache` / `schema-reject` / `routes` / `grammars` | The LLM substrate, offline |
| `being show` / `verify` / `invariants` / `block` / `belief` / `goal` / `commitment` / `relationship` / `affect` / `budget` | The persistent self |
| `memory add` / `get` / `recall` / `consolidate` / `forget` / `stats` / `eval` / `verify` / `mistake` / `procedure` / `index` | The memory organ |
| `epistemic ingest` / `promotable` / `contradictions` / `assumptions` / `dependency` / `invalidate` / `world` / `data-tests` | Claims, promotion, contradictions, justification |
| `library import` / `validate` / `orphans` / `list` / `applicable` / `extract` | The cognitive library |
| `skill register` / `verify` / `retrieve` | The verified skill library |
| `case retrieve` | Structure-mapped case retrieval |
| `experience compile` | Trajectories/insights → draft candidates |
| `policy history` / `fitness` / `check` / `evolve` | The policy genome and authorization questions |
| `frame compose` / `missing` | Conceptual frames and unknown slots |
| `episode run` / `replay` / `verify` / `value` / `budget-audit` / `program` | The metacognitive controller |
| `decide` | Answer one bounded question (`--core rules\|local\|hosted`) |
| `firewall eval` | Evaluate one episode or a corpus, with precision/recall thresholds |
| `compare check` | Comparison integrity over a pair or a fixtures corpus |
| `risk analyze` | Eight risk measures graded against the reference table |
| `calibration fit` / `conformal` | Temperature calibration; split-conformal abstention thresholds |
| `conformance` | The one conformance suite across the named cores |
| `tool list` / `describe` / `run` | The tool registry and the executor |
| `action ledger` / `show` / `rollback` | The hash-chained action ledger and byte-identical undo |
| `verify run` | Discharge one obligation and print its proof |
| `mcp list` / `call` / `serve` | The MCP bridge (stdio JSON-RPC) |
| `calibrate` | Score staked probabilities against outcomes, with assertion flags |
| `meta analyze` / `queue` / `lessons` | Self-observation: diagnosis and lessons |
| `regression run` | The suite the mistake compiler writes |
| `changeset new` / `show` | Typed change sets |
| `sandbox run` | Prepare a worktree, apply, build, test, audit isolation |
| `promote` / `reject` | Judge a change set; record the decision and the lineage |
| `budget show` | The four self-engineering budgets and what is left |
| `pi run` / `ingest` | The Pi code agent, live or recorded |
| `audit production-tree` | Prove production was not written outside a promotion |
| `loop run` / `status` / `artifacts` / `replay` / `resume` | The ten-stage closed loop |
| `self-model report` | Actual vs Model vs Ideal, numerically |
| `debt scan` | Architectural debt across code, memory and policies |
| `gc` | Reversible cognitive garbage collection (`--apply` to act) |
| `module load` / `rollback` | Hot-load a promoted version; restore the previous one |
| `design revise` / `history` | Design revisions, only ever through a promoted change set |

---

## 8. Practical notes

**Always scratch-dir first.** `--data-dir $(mktemp -d)` gives you a clean kernel in under a second and
keeps `data/` as a reference state. Commands that do not need a kernel (`llm grammars`, `episode verify`)
still accept it harmlessly.

**Two sandboxes, don't confuse them.** The CLI's `--data-dir` is *your* state. The tool sandbox is always
the repository's `data/sandbox`, so `tool run fs.write --arg path=data/sandbox/x` writes into the repo.
Reading outside that root is refused with `sandbox.escape`.

**Determinism is checkable, so check it.** After anything interesting: `logs verify` (ten checks),
`loop replay` (digest equality), `episode replay`, `replay --from 0` (state hash). If two runs of the same
inputs produce different digests, one of them is a bug worth filing.

**Idempotency keys are free.** Every `tool run` derives a key from the call if you omit `--idempotency-key`,
so a verbatim retry is exactly-once. Pass an explicit key when the *intent* is the same but the arguments
differ slightly (for example a timestamp in the payload).

**Refusals are normal, and they tell you what to fix.** `denied ... no grant covers X` → add a permission
grant or a policy row. `Unavailable ... OPENAI_BASE_URL and OPENAI_API_KEY must both be set` → configure
the provider. `no distilled head is loaded` → that path is not implemented yet (G1). `TierUnavailable` →
no tier-2/3 runtime is installed (G5).

**Keep `codex.lock` honest.** After a version bump: `mm-cli codex lock` and commit the diff. `codex verify`
and `codex lock --check` are the two commands to run before pushing.

**Budgets are hard caps.** A loop run gets its envelope from `--budget-file`; the four buckets are
`meta_analysis`, `improvement`, `evolution`, `metacognitive`, plus `wall_ms`. A stage that would cross its
bucket is denied whole and the denial is recorded — raise the file rather than expecting a partial stage.

**Use the assertion flags in scripts.** `--assert-brier-le`, `--assert-improves-baseline`,
`--assert-lessons-ge`, `--assert-reason-present`, `--assert-isolated`,
`--assert-unmodified-outside-promotion`, `--assert-fail-before-pass-after`, `--assert-completed` turn
reports into gates.

**Read the raw state when you need to.** SQLite is a normal file
(`sqlite3 $DATA/metamind.db 'select * from loop_runs'`), and the logs are JSONL
(`tail -f $DATA/logs/mm.jsonl | jq -c '{event_code, fields}'`). Treat both read-only: the event log, not
the tables, is authoritative.

**JSON ULID case.** Everything the CLI prints in text is lowercase, and almost all JSON is too — but
`tool run --json` and `action ledger --json` emit **uppercase** ULIDs (`01M4HVQH…`). Normalise with
`tr 'A-Z' 'a-z'` if you compare ids across commands.

**Performance notes.** `doctor` on an empty dir: well under a second. The full test suite: 1747 tests
across 152 binaries (about 78 s of aggregate binary time, parallel). The qualification loop run: ~0.3 s
with a recorded Pi session. Cold `cargo build --workspace`: minutes, dominated by RocksDB/Oxigraph.

**Resetting.** Delete the data dir (or `rm -rf data/` in the repo) and run `doctor` again. Nothing outside
the data dir is state; the repo itself is code plus fixtures.

---

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `error: the following required arguments were not provided: --query <QUERY>` | `epistemic world` needs a SPARQL query | pass `--query 'SELECT …'` |
| `reason: sandbox.escape: … does not resolve inside the sandbox root` | A tool path outside `data/sandbox` | use a path under `data/sandbox/` |
| `reason: no grant covers process.exec:process:execute:cargo` | Default-deny authorization | add a grant/policy row, or use the tool that is granted |
| `no snapshot <ulid> is recorded` on `action rollback` | The action created the file, so there was nothing to snapshot | roll back a write that *modified* an existing file (see §5.4) |
| `internal error: … (duplicate) (mm.internal)` on `skill register` | The entry already exists (the seed ships one) | register in a fresh data dir, or drop `library import` first; treat the exit code as a refusal |
| `episode verify` reports `the artifacts hold N` and a problem count | You verified a subset against the full gold table | run `--input bench/episodes` for the whole corpus |
| `no verified skill matches …` | The skill is still a `draft` | run `skill verify <iri>` first |
| `verdict: refuted` with `cargo exited 101` | The obligation's subject was not something cargo accepts (e.g. a file path) | use a package name |
| `query is not scoped to <…/graph/world>` | An unscoped SPARQL query against a named graph | add `GRAPH <…> { … }` or `FROM <…>` |
| `unavailable hosted: llm provider is not configured: OPENAI_BASE_URL and OPENAI_API_KEY must both be set` | No provider configured | export both variables |
| `unavailable local: no distilled head is loaded` | The head fitter does not exist yet | see [`open-gaps.md` G1](./open-gaps.md#g1) |
| `sandbox.tier_unavailable` | Tier-2/3 runtime absent | see [`open-gaps.md` G5](./open-gaps.md#g5) |
| `codex.lock is stale` + a list of `~ uris` | A module changed without a version bump | bump the version, `codex lock`, commit |
| `logs verify` fails on `audit chain` | Something appended outside the logger | inspect `audit_log`; do not edit it — the chain is hash-linked |
| `replay` reports a different state hash | Non-determinism in a write path | bisect with `--until <seq>` |
| `MM_DATA_DIR` seems ignored | It is (see §4.2) | use `--data-dir` |
| `pi: command not found` on a live run | `pi` not on `PATH` | install Pi, or use `--offline` / `--binary` |

---

## 10. What does not work yet

Five known gaps, documented in detail with evidence in [`open-gaps.md`](./open-gaps.md):

1. **`--core local` is unreachable** — nothing fits a distilled decision head.
2. **Factuality is library-only** — `search_augmented` is unimplemented, and nothing computes
   `factuality_support` or writes `factuality_checks` rows, so that firewall signal is inert.
3. **The tool layer's module provenance dangles** — nine `module/tools/*` IRIs name modules that do not
   exist, and no rule catches it.
4. **`pi_editor.open` is a stub** — authoring actually happens through `mm-pi` and `mm selfeng`.
5. **Sandbox tiers 2/3 have no runtime**, and **no model-facing path has been exercised against a live
   provider** in this environment.

Everything else in this guide works, and each of these refuses honestly rather than pretending.
