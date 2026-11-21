# `mm-data` Integration Guide

`mm-data` owns the data root: the RDF store, agentstream, user/project/app homes,
datasets, SQLite, and the four-tier unified memory. `mm-serve` will boot it with
[`initialize_data_layer`]; `mm-cme` will call [`MemorySystem`] for turn logging,
compression, retrieval and continuity.

```rust
use mm_data::{initialize_data_layer, InitConfig};

let data = initialize_data_layer(InitConfig {
    base_dir: Some("/var/lib/metamind".into()),
    ..InitConfig::default()
})
.await?;

let context = data.memory_system
    .start_session(&user_id, Some(&project_id), None)
    .await?;
```

## Backend: why this wraps `mm-store-graph`

The design names `cubecast` as the engine. Cubecast pins a fork of Oxigraph whose
`oxrocksdb-sys` declares `links = "rocksdb"`; the kernel's `mm-store-graph` links
crates.io Oxigraph, which declares the same native library. Cargo allows only one
package with a given `links` value, so `cubecast` and the current kernel cannot be
in the same binary. Migrating the kernel to the fork is not a dependency bump: the
fork's 0.6-dev RDF API returns owned terms and reworked iterators, and adapting the
vendored `rdf-codec`/`rdf-shacl` crates alone produces roughly a hundred compile
errors before `mm-store-graph` is touched.

`RdfEngine` therefore wraps [`mm_store_graph::GraphStore`] — the kernel's existing
persistent Oxigraph store with its single writer thread and scoped SPARQL. Every
graph, class and API in this crate is engine-independent. The swap to cubecast is
Phase 7's `mm-store-graph → mm-data` refactor; it replaces `engine.rs` and leaves
the rest of the layer intact.

## Layout

`initialize_data_layer` runs the nine design steps, each idempotent:

1. Resolve the root: `InitConfig::base_dir`, else `MM_DATA_DIR`, else `~/.metamind`.
2. Create the layout (`cubecast/`, `agentstream/`, `users/`, `projects/`, `apps/`,
   `datasets/`, `sqlite/`, `logs/`, `temp/uploads/`).
3. Load `config.toml`, or write it from the supplied defaults.
4. Open the engine (`cubecast/main` when persistent).
5. Load `ontology/*.ttl` and `ontology/shapes/*.ttl` into the ontology graph.
6. Open agentstream.
7. Open the four registries and write their index files.
8. Open SQLite and run the data-layer migrations.
9. Probe the store and build the `MemorySystem`.

An existing `config.toml` is authoritative: reopening a root never silently
changes its engine. `InitConfig::in_memory` forces an in-memory store for tests.

## Records and revisions

Every record is a subject with four triples: `rdf:type`, `mm:name` (when useful),
`mm:revision` and `mm:payload` (the JSON record). Updates append a new payload at a
higher revision rather than deleting the old one; readers take the highest
revision. This keeps the store append-only, which is what agentstream promises,
and lets the Rust schema evolve without an RDF migration. Typed projections can be
added beside the payload when a query needs them.

## Agentstream

`AgentstreamStore` gives every session a named graph
(`https://metamind.dev/agentstream/session/{id}`) and writes each event both as an
entity in that graph and as an N-Triples line in
`agentstream/events/events-YYYY-MM-DD.nt`. `snapshot_session` writes the Turtle
mirror under `agentstream/sessions/`; `write_projections` refreshes
`conversations.ttl`, `turns.ttl` and `tool_calls.ttl`.

## Registries

- `UserRegistry` creates `users/{id}/{home,projects,apps}`, `config.ttl`,
  `profile.ttl`, and a FOAF/SKOS user entity. `link_project`/`link_app` create the
  home symlinks.
- `ProjectRegistry` creates `projects/{id}/{home,members,datasets,agents,logs}` and
  a DOAP manifest, and maintains membership and dataset links.
- `AppRegistry` creates `apps/{id}/{home,data,plugins}`, a manifest and plugin
  registrations.
- `DatasetCatalog` creates the DCAT layout and adds versions by copying files into
  `versions/{version}/`, recording size and `sha256:<hex>` per file.

## Unified memory

| Tier | Module | Storage | Written by |
|---|---|---|---|
| 1 Episodic | `memory::episodic` | Session graph, `mem:EpisodicTurn` | `log_turn` |
| 2 Segments | `memory::compressed` | Segment graph + segment index | `compress_session` |
| 3 Project | `memory::project` | `memory/projects/{id}` | `aggregate_project` |
| 4 User | `memory::user` | `memory/users/{id}` | `sync_user` |

### Compression

`UnifiedCompressor` chunks the uncompressed tail (token budget with overlap),
extracts structure, writes a narrative, embeds it and records topics, then marks
the source turns compressed. The default pipeline is offline and deterministic:

- `HeuristicExtractor` reads explicit phrasing (`decided`, `will`, `?`, paths,
  `PR #n`, `Goal:`). It is conservative on purpose.
- `TemplateNarrator` writes a turn range, request, response, decision,
  commitment and open question.
- `HashingEmbedder` is a 384-dimensional hashed bag of words, not a semantic model.

`MemoryExtractor`, `NarrativeGenerator` and `Embedder` are the seams for Phase 5's
LLM and real embedding model. The segment records the model names that produced
it, so a re-embedding pass can find old vectors.

### Aggregation and transfer

`ProjectAggregator` merges segments idempotently: decisions by topic (or
topic+choice), commitments by id or description, questions by text, artifacts by
identifier, goals by id. It maintains the topic index, regenerates the project
narrative and picks a continuation in priority order: blocked commitment, active
goal's next step, high-priority question, recent artifact.

`CrossProjectLearner` raises expertise from the project stack, lifts conventions
above 0.8 confidence into tool preferences, refreshes the project summary and
regenerates the global narrative. It respects
`privacy_policies.allow_cross_project_learning`.

### Continuity and sync

`SessionContinuityManager` assembles project, user and previous-session context,
then generates the greeting in the design's order: concrete continuation, recent
project, returning user, introduction.

`MultiDeviceSync` keeps in-process presence. `register_channel` returns a
`tokio::mpsc` receiver; `broadcast_turn` sends to every other channel;
`handoff_modality` pushes a `ModalityChanged` message with preserved context.

## Configuration

`config.toml` mirrors `MemoryConfig`: episodic retention and compression triggers,
segment token budget/overlap/quality thresholds, project aggregation and
continuation, user cross-project learning, compression model names, vector
dimension, and sync presence TTL. Defaults are the design's values; the default
compression models name the offline implementations (`heuristic`, `template`,
`hashing-384`).

## What is not here yet

- A model-backed extractor/embedder (Phase 5).
- Deletion and retention enforcement: records are append-only and no TTL sweep
  runs. `export_user_data` exists; destructive deletion needs an engine-level
  delete path and the privacy policy from Phase 6.
- Multi-graph satellite stores and the cubecast semantic registry (Phase 7).
- Websocket transport for presence; only the in-process channels exist.

## Verification

`crates/mm-data/tests/data_layer.rs` covers the full bootstrap, a real SPARQL
query, all four registries, dataset version checksums, the compression → search →
aggregation → transfer → continuity path, multi-device broadcast/handoff, the
revision helper, and reopening a persistent root.
