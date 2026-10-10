# `bench/memory` — Phase 5 retrieval, consolidation, and adversarial fixtures

These fixtures are what `mm-cli memory eval`, `mm-cli memory consolidate`, and the
`mm-memory` integration tests read. They are committed, deterministic, and
credential-free; nothing here is generated at test time.

## `episodes.jsonl`

The episode stream. `mm-cli memory consolidate --window 30d --input
bench/memory/episodes.jsonl` ingests it, and `memory eval` ingests it into a
throwaway store before grading.

```json
{"ref":"ep-001","kind":"episodic","topic":"rust-build","content":"...","confidence":0.9,
 "importance":0.7,"entities":["rust","workspace-build"],"category":"multi_session",
 "valid_from":"2026-09-28T09:00:00Z"}
```

| Field | Meaning |
|---|---|
| `ref` | Stable external reference. **Required.** Derives the memory ULID. |
| `kind` | Memory class; `episodic` for every line here. |
| `topic` | The consolidation key: episodes sharing a topic fold into one semantic memory. |
| `content` | The free text. Every line is lexically distinctive. |
| `confidence`, `importance` | `[0,1]` floats. |
| `entities` | Entity names; shared within a topic, and `rust` bridges two topics so the graph channel has an edge to walk. |
| `category` | `multi_session`, `temporal`, `knowledge_update`, or `abstention`. |
| `valid_from` | RFC3339 UTC with `Z`. |

Twelve episodes, four topics, three each. **Rerunning the same file is a no-op**:
ingestion derives the memory ULID from `ref`, so the second run finds the row
already present and skips it. The derivation is `sha256("mm.memory.ref" ‖ ref)`
for the random 128 bits, with the timestamp part taken from `valid_from` — see
`mm_memory::model::ref_ulid`.

The four categories come from LongMemEval and LoCoMo: **multi_session** (recall
across sessions), **temporal** (when something held), **knowledge_update** (a fact
that changed — `ep-007` states the old toolchain policy and `ep-008` replaces it,
and both traces are kept), and **abstention** (what the being cannot answer).

## `gold.jsonl`

```json
{"query":"what fixed the workspace build","category":"multi_session",
 "relevant":["ep-001","ep-002","ep-003"]}
```

`relevant` lists `ref` values; the eval maps each back to its memory ULID with
`ref_ulid`, so the file survives a rebuilt store. Six queries: one per topic, one
extra knowledge-update query, and one abstention query whose `relevant` is empty.

**Metric definition the fixtures are sized for.** Each relevant list holds two or
three refs, so the eval must score

```
precision@k = |top_k ∩ relevant| / min(k, |relevant|)
recall@k    = |top_k ∩ relevant| / |relevant|
```

i.e. precision *over the retrieved relevant pool*. Under the bare
`|top_k ∩ relevant| / k` reading, a query with three relevant episodes could never
exceed 0.60 and no gold set of this shape could clear the plan's 0.80 floor. A
query with an empty `relevant` asserts that the corpus holds no answer and must be
skipped by both metrics rather than dividing by zero.

## `thresholds.toml`

The retrieval floors (`precision_at_k`, `recall_at_k`), the pinned hybrid weights
(`0.35`/`0.30`/`0.20`/`0.10`/`0.05`), the retention floor and half-life
(7 days), and the consolidation policy. `memory eval` fails when precision or
recall falls below the floors, so a regression is a non-zero exit rather than a
number someone has to notice.

## `adversarial/`

Each file is a list of memory write attempts:

```json
{"case":"identical_content_written_twice","expect_flag":"duplicate","kind":"semantic",
 "content":"...","confidence":0.8,"importance":0.5,"entities":["deploy-window"],
 "provenance":"new","valid_from":"2026-10-01T00:00:00Z"}
```

`expect_flag` is the flag `MemoryEngine::remember` must report; `provenance` is
`"new"` (a fresh ULID) or `"nil"` (the nil ULID, which must be **refused**).
`valid_from` is optional here and only present where the case needs a specific
world time to be reproducible.

| File | Flag | What must happen |
|---|---|---|
| `duplicates.jsonl` | `duplicate` | Not stored twice; the existing memory is returned. |
| `near_duplicates.jsonl` | `near_duplicate` | Stored **and** linked with `mm:similarTo`; cosine ≥ 0.92. |
| `stale.jsonl` | `stale` | Admitted, but not treated as the current policy. |
| `contradictory.jsonl` | `contradiction` | Admitted and linked with `mm:contradicts`; never silently resolved. |
| `orphan_entity.jsonl` | `orphan_entity` | Admitted and flagged; the entity has no edges anywhere else. |
| `missing_provenance.jsonl` | `missing_provenance` | **Refused** by validation with `field: "provenance"`. |

Nothing in this directory may be silently accepted: a fixture that stopped being
flagged is a gate failure, not a fixture that got better.
