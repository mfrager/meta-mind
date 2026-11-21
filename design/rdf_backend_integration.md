# MetaMind RDF Backend Integration — Cubecast Design Plan

**Status**: Design proposal for integrating `cubecast` (from `~/Build/cubecast`) as the unified RDF/OWL/SKOS backend
**Scope**: Replace `mm-store-graph` Oxigraph usage with cubecast engine; unify data directory layout; add agentstream logs, user/project/app homes, datasets
**Target Crates**: `mm-store-graph` (refactor), new `mm-data` crate, `mm-core` (config updates), `mm-serve` (init integration)

---

## Executive Summary

This design integrates **cubecast** — an embeddable RDF/OWL/SKOS ontology engine with Oxigraph+RocksDB storage, multi-graph registry, semantic registry, projects, documents, and inference — as MetaMind's single RDF backend.

**Key changes:**
1. **Single FS root** — One base directory (`$MM_DATA_DIR` or `~/.metamind`) containing all stores
2. **Cubecast engine** — Replaces direct Oxigraph usage; provides schema bootstrap, multi-graph, SPARQL, inference
3. **Agentstream logs** — RDF-backed event store in `agentstream/` subdir
4. **User/Project/App homes** — Hierarchical directory structure with RDF metadata
5. **Datasets root** — Versioned, cataloged datasets with provenance

---

## I. Directory Layout (Single FS Root)

```
$MM_DATA_DIR/                           # e.g. ~/.metamind or /var/lib/metamind
├── config.toml                         # Cubecast + MetaMind unified config
├── cubecast/                           # Cubecast engine root
│   ├── main/                           # Primary persistent store (RocksDB)
│   │   └── rocksdb/                    # Oxigraph RocksDB files
│   ├── graphs/                         # Multi-graph registry (when enabled)
│   │   ├── registry.rdf.xml            # RDF/XML graph registry
│   │   └── stores/                     # Satellite stores per graph
│   │       ├── being/
│   │       ├── memory/
│   │       ├── epistemic/
│   │       ├── library/
│   │       ├── decision/
│   │       ├── tools/
│   │       └── selfeng/
│   ├── registry/                       # Semantic registry artifacts
│   │   ├── sources/                    # Content-addressed source files
│   │   └── distributions/              # Published distributions
│   ├── projects/                       # Project metadata (cubecast project layer)
│   │   └── <project-id>/
│   │       ├── manifest.ttl            # Project RDF manifest
│   │       ├── works/                  # Work artifacts
│   │       └── exports/
│   ├── extractions/                    # MinerU document extractions
│   └── cache/                          # Embedding cache, query cache
│
├── agentstream/                        # Agent conversation/event logs (RDF)
│   ├── sessions/                       # Per-session Named Graphs
│   │   └── <session-id>.ttl            # Session as RDF graph
│   ├── events/                         # Event sourcing log (append-only)
│   │   └── events-<date>.nt            # N-Triples event log
│   ├── projections/                    # Materialized views
│   │   ├── conversations.ttl           # Conversation index
│   │   ├── turns.ttl                   # Turn index
│   │   └── tool_calls.ttl              # Tool call index
│   └── config.ttl                      # Agentstream schema config
│
├── users/                              # User home directories
│   ├── <user-id>/                      # ULID-based user directory
│   │   ├── home/                       # User's personal files
│   │   ├── config.ttl                  # User preferences (RDF)
│   │   ├── profile.ttl                 # User profile (FOAF/SKOS)
│   │   ├── projects/                   # Symlinks to project homes
│   │   └── apps/                       # Symlinks to app homes
│   └── index.ttl                       # User index (SPARQL queryable)
│
├── projects/                           # Project home directories
│   ├── <project-id>/                   # ULID-based project directory
│   │   ├── home/                       # Project working files
│   │   ├── config.ttl                  # Project config (RDF)
│   │   ├── manifest.ttl                # Project manifest (DOAP-like)
│   │   ├── members/                    # Member references (symlinks to users)
│   │   ├── datasets/                   # Symlinks to datasets
│   │   ├── agents/                     # Agent configs for this project
│   │   └── logs/                       # Project-specific logs
│   └── index.ttl                       # Project index
│
├── apps/                               # App home directories
│   ├── <app-id>/                       # ULID-based app directory
│   │   ├── home/                       # App working files
│   │   ├── config.ttl                  # App config (RDF)
│   │   ├── manifest.ttl                # App manifest
│   │   ├── data/                       # App-specific data
│   │   └── plugins/                    # App plugins (WASM)
│   └── index.ttl                       # App index
│
├── datasets/                           # Datasets root
│   ├── <dataset-id>/                   # ULID-based dataset directory
│   │   ├── manifest.ttl                # Dataset manifest (DCAT/DCAT-AP)
│   │   ├── data/                       # Actual data files
│   │   │   ├── raw/                    # Raw inputs
│   │   │   ├── processed/              # Processed/derived
│   │   │   └── exports/                # Export formats
│   │   ├── versions/                   # Versioned snapshots
│   │   │   └── v<semver>/
│   │   ├── provenance/                 # PROV-O provenance graph
│   │   ├── catalog/                    # Data catalog entries
│   │   └── metadata/                   # Quality, schema, stats
│   ├── catalog.ttl                     # Master dataset catalog (DCAT)
│   └── index.ttl                       # Dataset index
│
├── sqlite/                             # SQLite databases (non-RDF)
│   ├── metamind.db                     # Main SQLite (chat, sessions, etc.)
│   ├── audit.db                        # Audit log
│   └── ulid.watermark                  # ULID watermark
│
├── logs/                               # Text/JSONL logs (non-RDF)
│   ├── mm.jsonl                        # Main JSONL stream
│   ├── audit.jsonl                     # Audit JSONL
│   └── agentstream/                    # Agentstream text logs
│
└── temp/                               # Temporary files
    └── uploads/
```

---

## II. Cubecast Engine Integration

### II.1 New Crate: `mm-data` (RDF Data Layer)

```
crates/
├── mm-data/                            # NEW: Unified RDF data layer
│   ├── src/
│   │   ├── engine.rs                   # Cubecast engine wrapper
│   │   ├── config.rs                   # Unified config (cubecast + metamind)
│   │   ├── init.rs                     # Initialization & migration
│   │   ├── agentstream.rs              # Agentstream RDF store
│   │   ├── users.rs                    # User home management
│   │   ├── projects.rs                 # Project home management
│   │   ├── apps.rs                     # App home management
│   │   ├── datasets.rs                 # Dataset management
│   │   ├── graphs.rs                   # Graph aliases & routing
│   │   ├── ontology.rs                 # Ontology loading (mm.ttl, shapes)
│   │   └── lib.rs
│   └── Cargo.toml                      # Depends on cubecast (application feature)
│
├── mm-store-graph/                     # REFACTOR: Thin wrapper over mm-data
│   └── src/
│       ├── actor.rs                    # Keep: GraphHandle, mailbox
│       ├── graphs.rs                   # Keep: canonicalization, term JSON
│       ├── shacl.rs                    # Keep: SHACL validation
│       └── lib.rs                      # Re-export from mm-data
│
├── mm-core/                            # UPDATE: Config for unified root
└── mm-serve/                           # UPDATE: Init on boot
```

### II.2 Cubecast Engine Wrapper

```rust
// mm-data/src/engine.rs
use cubecast::{Cubecast, core::config::Config as CubecastConfig};
use std::path::PathBuf;
use std::sync::Arc;

pub struct RdfEngine {
    pub cubecast: Cubecast,
    pub config: EngineConfig,
    pub base_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub base_dir: PathBuf,
    pub enable_multi_graph: bool,
    pub enable_registry: bool,
    pub enable_projects: bool,
    pub enable_extractions: bool,
    pub enable_inference: bool,
    pub rocksdb_max_open_files: Option<u32>,
}

impl RdfEngine {
    /// Open or create the engine at the unified data root.
    pub async fn open(config: EngineConfig) -> Result<Self, DataError> {
        let base_dir = config.base_dir.clone();
        std::fs::create_dir_all(&base_dir)?;

        // Build cubecast config from our unified config
        let cubecast_config = build_cubecast_config(&config)?;

        // Open engine (creates stores, bootstraps schema)
        let cubecast = Cubecast::from_config(&cubecast_config)?;

        // Load MetaMind ontology files
        load_metamind_ontology(&cubecast).await?;

        Ok(Self { cubecast, config, base_dir })
    }

    /// Get the main RDF store for direct SPARQL access.
    pub fn store(&self) -> Arc<dyn cubecast::core::rdf::RdfStore> {
        self.cubecast.store()
    }

    /// Get the application service for command execution.
    pub fn service(&self) -> &cubecast::application::ApplicationService {
        self.cubecast.service()
    }

    /// Execute a cubecast command (RDF, OWL, SKOS, Project, etc.)
    pub async fn execute(&self, cmd: cubecast::core::command::Command)
        -> Result<cubecast::core::command::CommandResult, DataError>
    {
        self.cubecast.execute(cmd).await.map_err(Into::into)
    }
}

fn build_cubecast_config(cfg: &EngineConfig) -> Result<CubecastConfig, DataError> {
    let mut cc = CubecastConfig::default();

    // Storage: persistent RocksDB at base_dir/cubecast/main
    cc.storage.backend = cubecast::core::config::StorageBackend::Persistent;
    cc.storage.path = Some(cfg.base_dir.join("cubecast/main"));

    // Multi-graph registry
    cc.graphs.enabled = cfg.enable_multi_graph;
    if cfg.enable_multi_graph {
        cc.graphs.registry = Some(cfg.base_dir.join("cubecast/graphs/registry.rdf.xml"));
        cc.graphs.root = cfg.base_dir.join("cubecast/graphs/stores");
    }

    // Semantic registry
    cc.registry.artifact_dir = cfg.base_dir.join("cubecast/registry");

    // Projects
    cc.project.base_iri = "urn:metamind:project:".to_string();

    // Extractions (MinerU)
    cc.extraction.enabled = cfg.enable_extractions;
    cc.extraction.output_dir = cfg.base_dir.join("cubecast/extractions");

    // Limits
    cc.limits.max_blocking_tasks = 8;
    cc.limits.max_page_size = 1000;

    cc.validate()?;
    Ok(cc)
}
```

### II.3 Ontology Bootstrap

```rust
// mm-data/src/ontology.rs
async fn load_metamind_ontology(engine: &Cubecast) -> Result<(), DataError> {
    let ontology_files = [
        "ontology/mm.ttl",
        "ontology/code.ttl",
        "ontology/llm.ttl",
        "ontology/library.ttl",
        "ontology/metacog.ttl",
        "ontology/tools.ttl",
        "ontology/selfeng.ttl",
        "ontology/self_model.ttl",
    ];

    for file in ontology_files {
        let path = Config::repo_root().join(file);
        if path.exists() {
            let turtle = std::fs::read_to_string(&path)?;
            engine.execute(Command::Rdf(RdfCommand::Update {
                sparql: format!("LOAD <file://{}> INTO GRAPH <https://metamind.dev/graph/ontology>", path.display()),
            })).await?;
        }
    }

    // Load shape files into respective graphs
    load_shapes(engine).await?;
    Ok(())
}
```

---

## III. Initialization Process

### III.1 Init Sequence (on first boot / mm-serve start)

```
┌─────────────────────────────────────────────────────────────────┐
│                    MM-DATA INIT SEQUENCE                        │
├─────────────────────────────────────────────────────────────────┤
│  1. Resolve base directory                                       │
│     - $MM_DATA_DIR env var                                       │
│     - ~/.metamind (default)                                      │
│     - Config file override                                       │
│                                                                  │
│  2. Create directory structure (idempotent)                     │
│     - All subdirs under base_dir                                 │
│     - Set permissions (0700 for user dirs)                      │
│                                                                  │
│  3. Write unified config.toml                                    │
│     - Merge cubecast + metamind settings                         │
│     - Generate if missing                                        │
│                                                                  │
│  4. Initialize Cubecast engine                                   │
│     - Open/create RocksDB store                                  │
│     - Bootstrap OWL/SKOS schema                                  │
│     - Load MetaMind ontology files                               │
│     - Register graphs (being, memory, epistemic, ...)           │
│                                                                  │
│  5. Initialize Agentstream store                                 │
│     - Create agentstream/ subdir                                 │
│     - Load agentstream ontology (session, event, turn classes)  │
│     - Create named graph per session on demand                  │
│                                                                  │
│  6. Initialize User/Project/App indexes                          │
│     - Create users/index.ttl, projects/index.ttl, apps/index.ttl │
│     - Create datasets/catalog.ttl (DCAT)                         │
│                                                                  │
│  7. Run migrations (if schema version changed)                  │
│                                                                  │
│  8. Health check: SPARQL ASK on each graph                      │
│                                                                  │
│  9. Return initialized RdfEngine + handles                      │
└─────────────────────────────────────────────────────────────────┘
```

### III.2 Init API

```rust
// mm-data/src/init.rs
pub struct InitResult {
    pub engine: RdfEngine,
    pub agentstream: AgentstreamStore,
    pub users: UserRegistry,
    pub projects: ProjectRegistry,
    pub apps: AppRegistry,
    pub datasets: DatasetCatalog,
    pub sqlite: SqlitePool,
}

pub async fn initialize_data_layer(config: InitConfig) -> Result<InitResult, DataError> {
    // 1. Resolve & create base directory
    let base_dir = resolve_base_dir(&config)?;
    create_directory_structure(&base_dir)?;

    // 2. Load or create unified config
    let unified_config = load_or_create_config(&base_dir, &config)?;

    // 3. Initialize Cubecast engine
    let engine = RdfEngine::open(unified_config.engine.clone()).await?;

    // 4. Initialize subsystems
    let agentstream = AgentstreamStore::new(&engine, &base_dir).await?;
    let users = UserRegistry::new(&engine, &base_dir).await?;
    let projects = ProjectRegistry::new(&engine, &base_dir).await?;
    let apps = AppRegistry::new(&engine, &base_dir).await?;
    let datasets = DatasetCatalog::new(&engine, &base_dir).await?;

    // 5. Initialize SQLite (non-RDF)
    let sqlite = SqlitePool::open(&base_dir.join("sqlite")).await?;

    // 6. Run migrations
    run_migrations(&engine, &sqlite).await?;

    Ok(InitResult { engine, agentstream, users, projects, apps, datasets, sqlite })
}
```

---

## IV. Agentstream Logs (RDF Event Store) — **Part of Unified Memory Tier 1**

> **Note**: Agentstream is now the **Episodic tier (Tier 1)** of the [Unified Memory System](unified_memory_system.md). It stores raw turns, tool calls, voice chunks, and events as the input to the compression pipeline. See `unified_memory_system.md` §II.1 for the full episodic schema and §III.1 for the compression pipeline.

### IV.1 Ontology (agentstream.ttl)

```turtle
@prefix as: <https://metamind.dev/agentstream#> .
@prefix mm: <https://metamind.dev/ontology#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

as:Session a owl:Class ;
    rdfs:label "Agent Conversation Session" ;
    rdfs:subClassOf prov:Entity .

as:Turn a owl:Class ;
    rdfs:label "Conversation Turn" ;
    rdfs:subClassOf prov:Activity .

as:Event a owl:Class ;
    rdfs:label "Agentstream Event" ;
    rdfs:subClassOf prov:Activity .

# Properties
as:hasSession a owl:ObjectProperty ;
    rdfs:domain as:Turn ;
    rdfs:range as:Session .

as:hasTurn a owl:ObjectProperty ;
    rdfs:domain as:Event ;
    rdfs:range as:Turn .

as:eventType a owl:DatatypeProperty ;
    rdfs:range xsd:string .  # "user_message", "assistant_message", "tool_call", "tool_result", "proactive", "barge_in"

as:eventPayload a owl:DatatypeProperty ;
    rdfs:range xsd:string .  # JSON serialized

as:timestamp a owl:DatatypeProperty ;
    rdfs:range xsd:dateTimeStamp .

as:agent a owl:ObjectProperty ;
    rdfs:range mm:Being .

as:modality a owl:DatatypeProperty ;
    rdfs:range xsd:string .  # "drill_down", "explain", "zoom_out", "summarize", "bullets"

as:detailLevel a owl:DatatypeProperty ;
    rdfs:range xsd:integer .
```

### IV.2 Storage Pattern

```
agentstream/
├── sessions/                          # Named graphs: <https://metamind.dev/agentstream/session/{id}>
│   └── {session-id}.ttl               # Full session RDF (periodic snapshots)
├── events/                            # Append-only N-Triples event log
│   └── events-2026-11-21.nt           # One file per day
├── projections/                       # Materialized SPARQL views
│   ├── conversations.ttl              # SELECT ?session ?start ?end ?turnCount
│   ├── turns.ttl                      # SELECT ?turn ?session ?number ?modality
│   └── tool_calls.ttl                 # SELECT ?call ?turn ?tool ?success
└── config.ttl                         # Schema + prefixes
```

### IV.3 Agentstream API

```rust
// mm-data/src/agentstream.rs
pub struct AgentstreamStore {
    engine: RdfEngine,
    base_dir: PathBuf,
    current_session: Option<SessionId>,
    event_writer: AsyncFileWriter,
}

impl AgentstreamStore {
    pub async fn start_session(&mut self, session_id: SessionId, metadata: SessionMetadata) -> Result<()> {
        // Create named graph for session
        let graph_iri = session_graph_iri(&session_id);
        self.engine.execute(Command::Rdf(RdfCommand::Update {
            sparql: format!("CREATE GRAPH <{}>", graph_iri),
        })).await?;

        // Write session metadata
        self.write_session_metadata(&session_id, metadata).await?;

        // Open daily event file
        self.event_writer = AsyncFileWriter::open(
            self.base_dir.join("agentstream/events").join(format!("events-{}.nt", today()))
        ).await?;

        self.current_session = Some(session_id);
        Ok(())
    }

    pub async fn log_event(&mut self, event: AgentEvent) -> Result<()> {
        let triples = event_to_triples(&event, self.current_session.as_ref())?;
        let ntriples = triples_to_ntriples(&triples);

        // Append to event log (durable, append-only)
        self.event_writer.write(&ntriples).await?;

        // Also write to session named graph (for queries)
        if let Some(session) = &self.current_session {
            self.engine.execute(Command::Rdf(RdfCommand::Update {
                sparql: format!("INSERT DATA {{ GRAPH <{}> {{ {} }} }}", session_graph_iri(session), ntriples),
            })).await?;
        }
        Ok(())
    }

    pub async fn query_sessions(&self, filter: SessionFilter) -> Result<Vec<SessionSummary>> {
        let sparql = build_session_query(filter);
        self.engine.store().query(sparql).await
    }

    pub async fn get_session_graph(&self, session_id: &SessionId) -> Result<String> {
        let graph_iri = session_graph_iri(session_id);
        self.engine.store().query(format!("CONSTRUCT {{ ?s ?p ?o }} FROM <{}> WHERE {{ ?s ?p ?o }}", graph_iri)).await
    }
}
```

---

## V. User / Project / App Home Directories — **Unified Memory Tiers 3 & 4**

> **Note**: User homes (Tier 4), Project homes (Tier 3), and App homes are now part of the [Unified Memory System](unified_memory_system.md). User homes store UserMemory (global preferences, expertise, cross-project learning). Project homes store ProjectMemory (aggregated decisions, commitments, conventions). App homes store app-specific config/state. See `unified_memory_system.md` §II.3, §II.4 for schemas and §VI for RDF storage mapping.

### V.1 RDF Metadata Schema

**User Profile (users/\<id\>/config.ttl):**
```turtle
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix skos: <http://www.w3.org/2004/02/skos/core#> .
@prefix mm: <https://metamind.dev/ontology#> .

<urn:metamind:user:{user-id}> a foaf:Person, mm:User ;
    foaf:name "Display Name" ;
    foaf:mbox "user@example.com" ;
    mm:homeDir "/home/user" ;
    mm:preferences [
        mm:verbosity "concise" ;
        mm:technicalDepth "high" ;
        mm:formality "casual" ;
        mm:pace "fast"
    ] ;
    mm:memberOf <urn:metamind:project:{proj-id}> ;
    mm:usesApp <urn:metamind:app:{app-id}> .
```

**Project Manifest (projects/\<id\>/manifest.ttl):**
```turtle
@prefix doap: <http://usefulinc.com/ns/doap#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix mm: <https://metamind.dev/ontology#> .

<urn:metamind:project:{project-id}> a doap:Project, mm:Project ;
    doap:name "Project Name" ;
    doap:description "Project description" ;
    dct:created "2026-11-21T00:00:00Z"^^xsd:dateTime ;
    mm:homeDir "/home/user/projects/proj" ;
    mm:member <urn:metamind:user:{user-id}> ;
    mm:hasDataset <urn:metamind:dataset:{ds-id}> ;
    mm:hasAgent <urn:metamind:agent:{agent-id}> .
```

**App Manifest (apps/\<id\>/manifest.ttl):**
```turtle
@prefix doap: <http://usefulinc.com/ns/doap#> .
@prefix mm: <https://metamind.dev/ontology#> .

<urn:metamind:app:{app-id}> a doap:Project, mm:App ;
    doap:name "App Name" ;
    doap:version "1.0.0" ;
    mm:homeDir "/home/user/apps/app" ;
    mm:config <urn:metamind:app:{app-id}/config> ;
    mm:plugin <urn:metamind:plugin:{plugin-id}> .
```

### V.2 Registry APIs

```rust
// mm-data/src/users.rs
pub struct UserRegistry {
    engine: RdfEngine,
    base_dir: PathBuf,
}

impl UserRegistry {
    pub async fn create_user(&self, spec: UserSpec) -> Result<UserId> {
        let id = UserId::new();
        let user_dir = self.base_dir.join("users").join(id.to_string());
        std::fs::create_dir_all(&user_dir.join("home"))?;

        // Write config.ttl
        let config_ttl = build_user_config_ttl(&id, &spec);
        std::fs::write(user_dir.join("config.ttl"), config_ttl)?;

        // Write profile.ttl (FOAF)
        let profile_ttl = build_user_profile_ttl(&id, &spec);
        std::fs::write(user_dir.join("profile.ttl"), profile_ttl)?;

        // Load into RDF store (user graph)
        self.engine.execute(Command::Rdf(RdfCommand::Update {
            sparql: format!("LOAD <file://{}> INTO GRAPH <{}>", 
                user_dir.join("config.ttl").display(),
                user_graph_iri(&id)
            ),
        })).await?;

        // Update index
        self.update_index().await?;
        Ok(id)
    }

    pub async fn get_user_home(&self, user_id: &UserId) -> Result<PathBuf> {
        Ok(self.base_dir.join("users").join(user_id.to_string()).join("home"))
    }

    pub async fn link_project(&self, user_id: &UserId, project_id: &ProjectId) -> Result<()> {
        let user_dir = self.base_dir.join("users").join(user_id.to_string());
        std::os::unix::fs::symlink(
            self.base_dir.join("projects").join(project_id.to_string()),
            user_dir.join("projects").join(project_id.to_string())
        )?;
        // Update RDF membership
        self.add_membership(user_id, project_id).await
    }
}
```

---

## VI. Datasets Root

> **Note**: Datasets are managed by the `mm-data::DatasetCatalog` and are **separate from the Unified Memory hierarchy** (which covers conversation memory: episodic → segments → project → user). Datasets use DCAT/PROV-O for versioned data artifacts with provenance. They can be *referenced* from ProjectMemory (`key_artifacts`, `datasets` fields) but have independent lifecycle and storage.

### VI.1 DCAT Catalog (datasets/catalog.ttl)

```turtle
@prefix dcat: <http://www.w3.org/ns/dcat#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix mm: <https://metamind.dev/ontology#> .

<urn:metamind:datasets> a dcat:Catalog ;
    dct:title "MetaMind Datasets Catalog" ;
    dct:description "Versioned datasets with provenance" ;
    dcat:dataset <urn:metamind:dataset:{id1}>,
                 <urn:metamind:dataset:{id2}> .

<urn:metamind:dataset:{id}> a dcat:Dataset, mm:Dataset ;
    dct:title "Dataset Title" ;
    dct:description "Description" ;
    dct:created "2026-11-21T00:00:00Z"^^xsd:dateTime ;
    dcat:distribution [
        a dcat:Distribution ;
        dct:format "text/turtle" ;
        dcat:downloadURL <file:///path/to/dataset/data/processed/dataset.ttl> ;
        dcat:byteSize 1024000 ;
        dcat:checksum "sha256:..." ;
    ] ;
    mm:version "1.2.0" ;
    mm:schema <urn:metamind:schema:{schema-id}> ;
    mm:provenance <urn:metamind:provenance:{prov-id}> ;
    mm:qualityMetrics [
        mm:completeness 0.95 ;
        mm:accuracy 0.98 ;
        mm:freshnessDays 7
    ] .
```

### VI.2 Dataset API

```rust
// mm-data/src/datasets.rs
pub struct DatasetCatalog {
    engine: RdfEngine,
    base_dir: PathBuf,
}

impl DatasetCatalog {
    pub async fn create_dataset(&self, spec: DatasetSpec) -> Result<DatasetId> {
        let id = DatasetId::new();
        let ds_dir = self.base_dir.join("datasets").join(id.to_string());
        std::fs::create_dir_all(&ds_dir.join("data/raw"))?;
        std::fs::create_dir_all(&ds_dir.join("data/processed"))?;
        std::fs::create_dir_all(&ds_dir.join("data/exports"))?;
        std::fs::create_dir_all(&ds_dir.join("versions"))?;
        std::fs::create_dir_all(&ds_dir.join("provenance"))?;
        std::fs::create_dir_all(&ds_dir.join("catalog"))?;
        std::fs::create_dir_all(&ds_dir.join("metadata"))?;

        // Write manifest
        let manifest = build_dataset_manifest(&id, &spec);
        std::fs::write(ds_dir.join("manifest.ttl"), manifest)?;

        // Load into catalog graph
        self.engine.execute(Command::Rdf(RdfCommand::Update {
            sparql: format!("LOAD <file://{}> INTO GRAPH <{}>", 
                ds_dir.join("manifest.ttl").display(),
                DATASET_CATALOG_GRAPH
            ),
        })).await?;

        Ok(id)
    }

    pub async fn add_version(&self, dataset_id: &DatasetId, version: &str, files: &[PathBuf]) -> Result<()> {
        let version_dir = self.base_dir.join("datasets").join(dataset_id.to_string()).join("versions").join(version);
        std::fs::create_dir_all(&version_dir)?;

        // Copy files, compute checksums
        for file in files {
            let dest = version_dir.join(file.file_name().unwrap());
            std::fs::copy(file, &dest)?;
            let checksum = sha256_file(&dest)?;
            // Record in provenance
        }

        // Update manifest with new version
        self.update_dataset_version(dataset_id, version).await
    }

    pub async fn query_catalog(&self, filter: DatasetFilter) -> Result<Vec<DatasetSummary>> {
        let sparql = build_catalog_query(filter);
        self.engine.store().query(sparql).await
    }
}
```

---

## VII. Unified Configuration (config.toml)

```toml
# $MM_DATA_DIR/config.toml
# Single source of truth for all subsystems

[system]
base_dir = "/home/user/.metamind"          # Resolved at runtime
kernel_name = "metamind"
kernel_codename = "mm"

# Cubecast Engine Configuration
[cubecast]
storage_backend = "persistent"             # memory | persistent
storage_path = "cubecast/main"             # Relative to base_dir
rocksdb_max_open_files = 512

graphs_enabled = true
graphs_registry = "cubecast/graphs/registry.rdf.xml"
graphs_root = "cubecast/graphs/stores"
graphs_sync = "on_startup"                 # on_startup | manual
graphs_update_policy = "strict"            # strict | split

registry_enabled = true
registry_artifact_dir = "cubecast/registry"
registry_max_source_bytes = 52428800       # 50 MB
registry_allow_remote = false
registry_remote_hosts = []

project_enabled = true
project_base_iri = "urn:metamind:project:"

extraction_enabled = false
extraction_output_dir = "cubecast/extractions"
extraction_backend = "pipeline"
extraction_tier = "default"

limits_max_blocking_tasks = 8
limits_max_page_size = 1000

# Agentstream Configuration
[agentstream]
enabled = true
sessions_dir = "agentstream/sessions"
events_dir = "agentstream/events"
projections_dir = "agentstream/projections"
snapshot_interval_turns = 10
event_log_rotation = "daily"               # daily | hourly | size:100MB

# User/Project/App Directories
[directories]
users_root = "users"
projects_root = "projects"
apps_root = "apps"
datasets_root = "datasets"

# SQLite (non-RDF)
[sqlite]
main_db = "sqlite/metamind.db"
audit_db = "sqlite/audit.db"
ulid_watermark = "sqlite/ulid.watermark"

# Logging (text/JSONL)
[logging]
level = "info"
json = true
console = false
dir = "logs"
jsonl_file = "mm.jsonl"
audit_file = "audit.jsonl"

# Ontology (loaded from repo, not data dir)
[ontology]
repo_root = "/home/user/Build/metamind"    # Set at build/runtime
files = [
    "ontology/mm.ttl",
    "ontology/code.ttl",
    "ontology/llm.ttl",
    "ontology/library.ttl",
    "ontology/metacog.ttl",
    "ontology/tools.ttl",
    "ontology/selfeng.ttl",
    "ontology/self_model.ttl"
]
shape_files = {
    "code" = "ontology/shapes/mmc-shapes.ttl",
    "being" = "ontology/shapes/being.shacl.ttl",
    "memory" = "ontology/shapes/memory.ttl",
    "epistemic" = "ontology/shapes/epistemic_shapes.ttl",
    "library" = "ontology/shapes/library.shacl.ttl",
    "decision" = "ontology/shapes/decision.ttl",
    "tools" = "ontology/shapes/tools.ttl",
    "selfeng" = "ontology/shapes/selfeng.ttl",
    "self" = "ontology/shapes/self_model.ttl"
}
```

---

## VIII. Migration from Current mm-store-graph

### VIII.1 Current State
- `mm-store-graph` uses Oxigraph directly via `GraphStore` actor
- Single RocksDB at `data/graph`
- Graphs: `/being`, `/memory`, `/epistemic`, `/world`, `/library`, `/code`, `/decision`, `/tools`, `/selfeng`, `/self`, `/provenance`
- SHACL validation per graph
- No semantic registry, no projects, no documents

### VIII.2 Migration Steps

| Phase | Action | Rollback |
|-------|--------|----------|
| 1 | Add `cubecast` as dependency to `mm-store-graph` (feature `application`) | Cargo revert |
| 2 | Create `mm-data` crate with `RdfEngine` wrapper | Delete crate |
| 3 | Implement `EngineConfig` → `CubecastConfig` mapping | Config compat |
| 4 | Add `initialize_data_layer()` called from `mm-serve::AppState::boot` | Feature flag |
| 5 | Migrate graph handles: `GraphHandle` → `RdfEngine.store()` | Dual-write period |
| 6 | Enable multi-graph registry (maps current named graphs) | Keep single store |
| 7 | Migrate agentstream logs to RDF event store | Keep JSONL |
| 8 | Add user/project/app/dataset registries | New only |
| 9 | Remove direct Oxigraph usage from `mm-store-graph` | Full cutover |

### VIII.3 Compatibility Layer

```rust
// mm-store-graph/src/actor.rs (updated)
pub struct GraphHandle {
    // Old: direct Oxigraph Store handle
    // New: Arc<RdfEngine> + graph IRI
    engine: Arc<RdfEngine>,
    graph_iri: String,
}

impl GraphHandle {
    pub async fn query(&self, sparql: &str) -> Result<QueryResults> {
        let full_sparql = format!("FROM <{}> {}", self.graph_iri, sparql);
        self.engine.store().query(full_sparql).await
    }

    pub async fn update(&self, sparql: &str) -> Result<()> {
        let full_sparql = format!("WITH <{}> {}", self.graph_iri, sparql);
        self.engine.execute(Command::Rdf(RdfCommand::Update { sparql: full_sparql })).await
    }
}
```

---

## IX. Integration with mm-serve

### IX.1 Updated AppState

```rust
// mm-serve/src/state.rs (updated)
use mm_data::{InitResult, initialize_data_layer, InitConfig};

pub struct AppState {
    // ... existing fields ...

    /// Unified RDF data layer (NEW)
    pub data: InitResult,

    /// Cubecast engine for direct access
    pub rdf_engine: Arc<RdfEngine>,

    /// Agentstream for conversation logging
    pub agentstream: Arc<AgentstreamStore>,
}

impl AppState {
    pub async fn boot(cfg: Config, lock: ConsoleLock) -> Result<Self, MmError> {
        // ... existing SQLite, eventlog, logger init ...

        // NEW: Initialize unified data layer
        let init_config = InitConfig {
            base_dir: cfg.store.data_dir.clone(),
            enable_multi_graph: true,
            enable_registry: true,
            enable_projects: true,
            enable_extractions: false,
            enable_inference: true,
        };
        let data = initialize_data_layer(init_config).await?;

        // ... existing graph, being, focus, trees, chat init ...

        Ok(Self {
            // ... existing fields ...
            rdf_engine: Arc::new(data.engine),
            agentstream: Arc::new(data.agentstream),
            data,
        })
    }
}
```

### IX.2 Controller Integration (Agentstream Logging)

```rust
// mm-serve/src/controller/turn.rs (updated)
use mm_data::agentstream::{AgentEvent, EventType};

async fn reply(...) -> TurnOutcome {
    // ... existing logic ...

    // Log each frame to agentstream
    let mut agentstream = state.agentstream.lock().await;
    
    for frame in &frames {
        let event = AgentEvent {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            event_type: frame_to_event_type(frame),
            payload: serde_json::to_string(frame)?,
            timestamp: Utc::now(),
            modality: extract_modality(frame),
            detail_level: extract_detail_level(frame),
        };
        agentstream.log_event(event).await?;
    }
}
```

---

## X. Testing Strategy

### X.1 Unit Tests (mm-data)
- `EngineConfig` → `CubecastConfig` mapping
- Directory structure creation (idempotent)
- User/Project/App/Dataset CRUD
- Agentstream event serialization
- SPARQL query building

### X.2 Integration Tests
- Full init sequence from empty dir
- Session resume after restart (agentstream persistence)
- Multi-graph queries across being/memory/epistemic
- Dataset versioning + provenance queries
- User-project membership queries

### X.3 Migration Tests
- Load existing `data/graph` into cubecast
- Verify all 11 named graphs accessible
- SHACL validation passes
- Chat sessions resume correctly

### X.4 Property Tests
- Agentstream event log append-only property
- Dataset version immutability
- User home directory isolation
- Config round-trip (TOML ↔ struct)

---

## XI. Rollout Plan

| Week | Milestone |
|------|-----------|
| 1 | `mm-data` crate skeleton, `EngineConfig`, directory layout |
| 2 | Cubecast engine wrapper, ontology bootstrap |
| 3 | Agentstream store + event logging |
| 4 | User/Project/App registries + RDF indexes |
| 5 | Dataset catalog + DCAT integration |
| 6 | `mm-store-graph` compatibility layer |
| 7 | `mm-serve` integration, init sequence |
| 8 | Migration script for existing `data/graph` |
| 9 | Synthetic testing + load testing |
| 10 | Canary deploy, monitoring, rollback plan |

---

## XII. Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| **RocksDB locking** | Single writer (current) vs cubecast multi-graph | Keep single writer; cubecast GraphBackend handles |
| **In-memory vs persistent default** | Memory for tests, persistent for prod | `Config::for_data_dir` → memory; env var → persistent |
| **Agentstream graph per session vs single graph** | Named graph per session (query isolation) | Named graph per session |
| **Dataset versioning** | Git-like (content-addressed) vs semantic | Content-addressed + semantic version in manifest |
| **User auth** | External (OIDC) vs local RDF | Local RDF for metadata; auth external |
| **Backup strategy** | RocksDB checkpoint + RDF dump | Daily checkpoint + weekly full Turtle dump |

---

## XIII. Appendix: Cubecast Crate Mapping

| Cubecast Module | MetaMind Use Case |
|-----------------|-------------------|
| `cubecast::core` | IDs, namespaces, commands, errors |
| `cubecast::rdf` | Oxigraph store, SPARQL, transactions, backups |
| `cubecast::datacatalog` | Dataset catalog, external refs, resource resolution |
| `cubecast::graph` | Multi-graph registry (current 11 graphs) |
| `cubecast::owl` | OWL class/property management (mm.ttl) |
| `cubecast::skos` | SKOS taxonomies (preferences, modalities) |
| `cubecast::classify` | Instance classification (agent types) |
| `cubecast::embedding` | Vector similarity (semantic search) |
| `cubecast::inference` | RDFS/OWL reasoning (entailments) |
| `cubecast::import` | External ontology import |
| `cubecast::registry` | Semantic registry (published ontologies) |
| `cubecast::project` | Project/work management |
| `cubecast::document` | MinerU extraction → RDF |
| `cubecast::application` | Unified command dispatch (HTTP/CLI/MCP) |

---

*This design plan is the single source of truth for the RDF backend integration. Review, approve, then begin Phase 1 (mm-data crate skeleton).*