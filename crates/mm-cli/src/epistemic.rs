//! `mm-cli epistemic` — the epistemic discipline's operator surface.
//!
//! Every mutation goes through [`mm_epistemic::EpistemicEngine`], so a CLI write
//! takes exactly the validated path a runtime write does: the guard decides
//! promotions, the barrier decides `/world`, and every change is audited. There is
//! no second door.
//!
//! Two commands are *read-only proofs* rather than state changes, and they are the
//! ones the gate leans on: `promotable` runs the deterministic table over a fixture
//! of forbidden transitions and proves none of them is allowed, and `world` runs a
//! SPARQL query over `/world` and proves every status there is `OBSERVED` or
//! `VERIFIED`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Subcommand;
use mm_core::{Config, MmError, Ulid};
use mm_epistemic::{
    can_promote, data_tests, Claim, ClaimKind, DepKind, EpistemicEngine, EpistemicError,
    EpistemicStatus, Evidence, EvidenceKind, Justification, JustificationGraph, Proposition,
    SqliteEpistemicStore, EPISTEMIC_GRAPH,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::kernel::Kernel;

/// `mm-cli epistemic`.
#[derive(Subcommand, Debug)]
pub enum EpistemicCommand {
    /// Admit candidate claims from a JSONL fixture.
    Ingest {
        /// One claim spec per line.
        #[arg(long, value_name = "FILE")]
        file: PathBuf,
    },
    /// Run the deterministic promotion table over a fixture of transitions.
    Promotable {
        /// JSONL of `{from, to, evidence}` records.
        #[arg(long, value_name = "FILE")]
        fixture: PathBuf,
    },
    /// Detect and record contradictions over a fixture directory.
    Contradictions {
        /// Directory of `*.jsonl` claim specs and a `manifest.json`.
        #[arg(long, value_name = "DIR")]
        fixture: PathBuf,
    },
    /// Report the assumption ledger, highest verification priority first.
    Assumptions {
        /// Only `priority` is supported.
        #[arg(long, default_value = "priority")]
        sort: String,
        /// How many to print.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// The caller's probability the assumption is false.
        #[arg(long, default_value_t = 0.5)]
        p_false: f64,
    },
    /// Walk the justification graph.
    Dependency {
        #[command(subcommand)]
        command: DependencyCommand,
    },
    /// Withdraw a claim's support and retract everything that rested on it.
    Invalidate {
        /// The root claim's ULID.
        ulid: String,
        /// Why the support was withdrawn.
        #[arg(long)]
        reason: String,
    },
    /// Query `/world`; every status there must be OBSERVED or VERIFIED.
    World {
        /// A SPARQL query that names the `/world` graph.
        #[arg(long)]
        query: String,
    },
    /// Run the RDFUnit-style data-quality suite.
    DataTests {
        /// The graph the suite names in its findings.
        #[arg(long, default_value = "epistemic")]
        graph: String,
    },
}

/// `mm-cli epistemic dependency`.
#[derive(Subcommand, Debug)]
pub enum DependencyCommand {
    /// The dependency-directed retraction set for a root.
    Cascade {
        /// The root claim's ULID.
        #[arg(long, value_name = "ULID")]
        root: String,
        /// The justification graph; defaults to the cascade fixture.
        #[arg(long, value_name = "FILE")]
        graph: Option<PathBuf>,
    },
    /// Why a node is believed.
    Why {
        /// The node's ULID.
        #[arg(value_name = "ULID")]
        ulid: String,
        /// The justification graph; defaults to the cascade fixture.
        #[arg(long, value_name = "FILE")]
        graph: Option<PathBuf>,
    },
}

/// `mm-cli epistemic`.
pub async fn run(cfg: Config, command: EpistemicCommand) -> Result<ExitCode, MmError> {
    match command {
        EpistemicCommand::Ingest { file } => ingest(cfg, file).await,
        EpistemicCommand::Promotable { fixture } => promotable(fixture),
        EpistemicCommand::Contradictions { fixture } => contradictions(cfg, fixture).await,
        EpistemicCommand::Assumptions {
            sort,
            limit,
            p_false,
        } => assumptions(cfg, sort, limit, p_false).await,
        EpistemicCommand::Dependency { command } => dependency(cfg, command).await,
        EpistemicCommand::Invalidate { ulid, reason } => invalidate(cfg, ulid, reason).await,
        EpistemicCommand::World { query } => world(cfg, query).await,
        EpistemicCommand::DataTests { graph } => data_tests_cmd(cfg, graph).await,
    }
}

// -------------------------------------------------------------------- specs ----

/// One claim, as a fixture writes it.
#[derive(Debug, Deserialize)]
struct ClaimSpec {
    /// The claim's ULID; derived from the proposition when omitted.
    #[serde(default)]
    id: Option<String>,
    /// The claim kind's wire name.
    kind: String,
    /// The proposition subject IRI.
    subject: String,
    /// The proposition predicate IRI.
    predicate: String,
    /// The proposition object.
    object: String,
    /// True when the object is an IRI rather than a literal.
    #[serde(default)]
    object_is_iri: bool,
    /// The status wire name; the kind's default when omitted.
    #[serde(default)]
    status: Option<String>,
    /// The confidence, in `[0,1]`.
    #[serde(default)]
    confidence: Option<f32>,
    /// Evidence kinds to attach.
    #[serde(default)]
    evidence: Vec<String>,
}

/// One forbidden transition, as the promotion fixture writes it.
#[derive(Debug, Deserialize)]
struct PromotionSpec {
    /// The status being left.
    from: String,
    /// The status wanted.
    to: String,
    /// The evidence kinds offered.
    #[serde(default)]
    evidence: Vec<String>,
    /// A note, ignored by the guard.
    #[serde(default)]
    why: Option<String>,
}

/// The contradiction fixture's declared count.
#[derive(Debug, Deserialize)]
struct Manifest {
    /// How many conflicting pairs the fixture seeds.
    seeded_pairs: usize,
}

/// The cascade fixture.
#[derive(Debug, Deserialize)]
struct CascadeGraph {
    /// The root whose withdrawal is tested.
    root: String,
    /// Every node, as a claim id.
    #[serde(default)]
    nodes: Vec<String>,
    /// The support edges.
    #[serde(default)]
    edges: Vec<EdgeSpec>,
}

/// One support edge.
#[derive(Debug, Deserialize)]
struct EdgeSpec {
    /// The supported node.
    consequent: String,
    /// The nodes it rests on.
    antecedents: Vec<String>,
    /// The support kind's wire name.
    kind: String,
    /// How much the consequent leans on it.
    #[serde(default = "default_criticality")]
    criticality: f32,
}

fn default_criticality() -> f32 {
    0.5
}

fn to_mm(e: EpistemicError) -> MmError {
    match e {
        EpistemicError::Db(m) => MmError::Store(m),
        EpistemicError::Graph(m) => MmError::Graph(m),
        EpistemicError::Codec(m) => MmError::Codec(m),
        EpistemicError::Config(m) => MmError::Config(m),
        other => MmError::Internal(format!("{} ({})", other, other.code())),
    }
}

fn parse_ulid(text: &str) -> Result<Ulid, MmError> {
    mm_core::id::parse_ulid(text)
        .map_err(|e| MmError::Config(format!("invalid ULID {text:?}: {e}")))
}

/// A deterministic claim id for a fixture that omits one.
fn fixture_id(subject: &str, predicate: &str, object: &str) -> Ulid {
    let mut hasher = Sha256::new();
    hasher.update(b"mm.epistemic.fixture\0");
    hasher.update(subject.as_bytes());
    hasher.update([0u8]);
    hasher.update(predicate.as_bytes());
    hasher.update([0u8]);
    hasher.update(object.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Ulid::from_bytes(bytes)
}

/// Build the evidence a spec's kind names carry.
fn evidence_for(spec: &ClaimSpec) -> Result<Vec<Evidence>, MmError> {
    let mut out = Vec::new();
    for (index, kind) in spec.evidence.iter().enumerate() {
        let kind = EvidenceKind::parse(kind)
            .ok_or_else(|| MmError::Config(format!("unknown evidence kind {kind:?}")))?;
        let mut hasher = Sha256::new();
        hasher.update(b"mm.epistemic.evidence.fixture\0");
        hasher.update(spec.subject.as_bytes());
        hasher.update([0u8]);
        hasher.update(index.to_le_bytes());
        let digest = hasher.finalize();
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        let id = Ulid::from_bytes(bytes);
        let content = format!("{}|{}|{}", spec.subject, spec.predicate, spec.object);
        out.push(Evidence::from_content(id, kind, None, &content, 0.95).map_err(to_mm)?);
    }
    Ok(out)
}

/// Build a claim from a fixture spec.
fn claim_of(spec: &ClaimSpec) -> Result<Claim, MmError> {
    let kind = ClaimKind::parse(&spec.kind)
        .ok_or_else(|| MmError::Config(format!("unknown claim kind {:?}", spec.kind)))?;
    let proposition = Proposition::new(
        &spec.subject,
        &spec.predicate,
        &spec.object,
        spec.object_is_iri,
    )
    .map_err(to_mm)?;
    let status = match &spec.status {
        Some(text) => EpistemicStatus::parse(text)
            .ok_or_else(|| MmError::Config(format!("unknown status {text:?}")))?,
        None => kind.default_status(),
    };
    let id = match &spec.id {
        Some(text) => parse_ulid(text)?,
        None => fixture_id(&spec.subject, &spec.predicate, &spec.object),
    };
    Claim::new(
        id,
        kind,
        proposition,
        status,
        spec.confidence.unwrap_or(0.5),
    )
    .map_err(to_mm)
}

fn read_specs(path: &Path) -> Result<Vec<ClaimSpec>, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut out = Vec::new();
    for (line, text) in raw.lines().enumerate() {
        let text = text.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let spec: ClaimSpec = serde_json::from_str(text)
            .map_err(|e| MmError::Codec(format!("{}:{}: {e}", path.display(), line + 1)))?;
        out.push(spec);
    }
    Ok(out)
}

// ------------------------------------------------------------------- engine ----

async fn open_engine(cfg: Config) -> Result<(Kernel, EpistemicEngine), MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    let handle = kernel.graph()?.handle().clone();
    let store = Arc::new(SqliteEpistemicStore::new(
        kernel.sqlite.clone(),
        Arc::clone(&kernel.logger),
        Arc::clone(&kernel.ids),
    ));
    Ok((kernel, EpistemicEngine::new(store, handle)))
}

async fn shutdown(kernel: Kernel) -> Result<(), MmError> {
    if let Some(graph) = &kernel.graph {
        graph.shutdown().await?;
    }
    kernel.sqlite.close().await;
    Ok(())
}

fn bench_path(relative: &str) -> PathBuf {
    Config::repo_root().join(relative)
}

// ------------------------------------------------------------------- ingest ----

async fn ingest(cfg: Config, file: PathBuf) -> Result<ExitCode, MmError> {
    let specs = read_specs(&file)?;
    let (kernel, engine) = open_engine(cfg).await?;
    let mut count = 0usize;
    for spec in &specs {
        let claim = claim_of(spec)?;
        let evidence = evidence_for(spec)?;
        engine.ingest(&claim, &evidence).await.map_err(to_mm)?;
        count += 1;
    }
    println!("epistemic ingest: {count} claim(s) admitted");
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- promotable ---

fn promotable(fixture: PathBuf) -> Result<ExitCode, MmError> {
    let raw = std::fs::read_to_string(&fixture)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", fixture.display())))?;
    let mut allowed = 0usize;
    let mut rejected = 0usize;
    for (line, text) in raw.lines().enumerate() {
        let text = text.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let spec: PromotionSpec = serde_json::from_str(text)
            .map_err(|e| MmError::Codec(format!("{}:{}: {e}", fixture.display(), line + 1)))?;
        let from = EpistemicStatus::parse(&spec.from)
            .ok_or_else(|| MmError::Config(format!("unknown status {:?}", spec.from)))?;
        let to = EpistemicStatus::parse(&spec.to)
            .ok_or_else(|| MmError::Config(format!("unknown status {:?}", spec.to)))?;
        let mut evidence = Vec::new();
        for (index, kind) in spec.evidence.iter().enumerate() {
            let kind = EvidenceKind::parse(kind)
                .ok_or_else(|| MmError::Config(format!("unknown evidence kind {kind:?}")))?;
            let id = Ulid::from_parts(index as u64, index as u128 + 1);
            evidence.push(Evidence::from_content(id, kind, None, "fixture", 0.9).map_err(to_mm)?);
        }
        match can_promote(from, to, &evidence) {
            Ok(()) => {
                allowed += 1;
                println!(
                    "  ALLOWED {} -> {} ({})",
                    from,
                    to,
                    spec.why.as_deref().unwrap_or("")
                );
            }
            Err(denied) => {
                rejected += 1;
                println!(
                    "  rejected {} -> {}: {}",
                    denied.from, denied.to, denied.reason
                );
            }
        }
    }
    println!("epistemic promotable: allowed={allowed} rejected={rejected}");
    if allowed == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

// ------------------------------------------------------------- contradictions --

async fn contradictions(cfg: Config, fixture: PathBuf) -> Result<ExitCode, MmError> {
    let manifest_path = fixture.join("manifest.json");
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).map_err(|e| {
            MmError::Config(format!("cannot read {}: {e}", manifest_path.display()))
        })?)
        .map_err(|e| MmError::Codec(format!("{}: {e}", manifest_path.display())))?;

    let mut files: Vec<PathBuf> = std::fs::read_dir(&fixture)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", fixture.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();

    let (kernel, engine) = open_engine(cfg).await?;
    let mut ingested = 0usize;
    for file in &files {
        for spec in read_specs(file)? {
            let claim = claim_of(&spec)?;
            let evidence = evidence_for(&spec)?;
            engine.ingest(&claim, &evidence).await.map_err(to_mm)?;
            ingested += 1;
        }
    }
    let created = engine.detect_contradictions().await.map_err(to_mm)?;
    println!(
        "epistemic contradictions: created={} seeded={} (claims ingested {ingested})",
        created.len(),
        manifest.seeded_pairs
    );
    shutdown(kernel).await?;
    if created.len() == manifest.seeded_pairs {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

// --------------------------------------------------------------- assumptions --

async fn assumptions(
    cfg: Config,
    sort: String,
    limit: usize,
    p_false: f64,
) -> Result<ExitCode, MmError> {
    if sort != "priority" {
        return Err(MmError::Config(format!(
            "unknown sort {sort:?}; only `priority` is supported"
        )));
    }
    let (kernel, engine) = open_engine(cfg).await?;
    let ledger = mm_epistemic::AssumptionLedger {
        assumptions: engine.store().list_assumptions().await.map_err(to_mm)?,
    };
    let ranked = ledger.by_priority(p_false);
    println!("epistemic assumptions: {} held", ledger.len());
    for (id, priority) in ranked.into_iter().take(limit) {
        if let Some(assumption) = ledger.get(&id) {
            println!(
                "  {} priority={priority:.6} risk={} cost={} dependence={}",
                mm_core::ulid_string(&id),
                assumption.consequence_if_false,
                assumption.verification_cost,
                assumption.decision_dependence,
            );
        }
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- dependency --

fn load_cascade_graph(path: &Path) -> Result<CascadeGraph, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    serde_json::from_str(&raw).map_err(|e| MmError::Codec(format!("{}: {e}", path.display())))
}

fn graph_from_spec(spec: &CascadeGraph) -> Result<JustificationGraph, MmError> {
    let mut edges = Vec::new();
    for edge in &spec.edges {
        let kind = DepKind::parse(&edge.kind)
            .ok_or_else(|| MmError::Config(format!("unknown dependency kind {:?}", edge.kind)))?;
        let mut antecedents = Vec::new();
        for text in &edge.antecedents {
            antecedents.push(parse_ulid(text)?);
        }
        edges.push(
            Justification::new(
                parse_ulid(&edge.consequent)?,
                antecedents,
                kind,
                edge.criticality,
            )
            .map_err(to_mm)?,
        );
    }
    JustificationGraph::from_edges(edges).map_err(to_mm)
}

async fn dependency(cfg: Config, command: DependencyCommand) -> Result<ExitCode, MmError> {
    match command {
        DependencyCommand::Cascade { root, graph } => {
            let path = graph.unwrap_or_else(|| bench_path("bench/epistemic/cascade/graph.json"));
            let spec = load_cascade_graph(&path)?;
            let graph = graph_from_spec(&spec)?;
            let requested = parse_ulid(&root)?;
            let declared = parse_ulid(&spec.root)?;
            if requested != declared {
                return Err(MmError::Config(format!(
                    "root {root} is not the fixture's declared root {}",
                    spec.root
                )));
            }
            let cascade = graph.cascade(&requested);
            println!(
                "epistemic dependency cascade: root {}",
                mm_core::ulid_string(&cascade.root)
            );
            for node in &cascade.affected {
                println!("  affected: {}", mm_core::ulid_string(node));
            }
            println!("  affected_count={}", cascade.count());
        }
        DependencyCommand::Why { ulid, graph } => {
            let path = graph.unwrap_or_else(|| bench_path("bench/epistemic/cascade/graph.json"));
            let spec = load_cascade_graph(&path)?;
            let graph = graph_from_spec(&spec)?;
            let node = parse_ulid(&ulid)?;
            let why = graph.why(&node);
            println!("epistemic dependency why: {}", mm_core::ulid_string(&node));
            for edge in &why {
                let antecedents: Vec<String> =
                    edge.antecedents.iter().map(mm_core::ulid_string).collect();
                println!(
                    "  {} <- [{}] ({})",
                    mm_core::ulid_string(&edge.consequent),
                    antecedents.join(", "),
                    edge.kind
                );
            }
            if why.is_empty() {
                println!("  no recorded support");
            }
        }
    }
    let _ = cfg;
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------- invalidate --

async fn invalidate(cfg: Config, ulid: String, reason: String) -> Result<ExitCode, MmError> {
    let root = parse_ulid(&ulid)?;
    let spec = load_cascade_graph(&bench_path("bench/epistemic/cascade/graph.json"))?;
    let graph = graph_from_spec(&spec)?;

    let (kernel, engine) = open_engine(cfg).await?;
    // Materialize the fixture's nodes and edges so the retraction runs against the
    // store, not against a computed set.
    for node in &spec.nodes {
        let id = parse_ulid(node)?;
        let claim = Claim::new(
            id,
            ClaimKind::Inference,
            Proposition::literal(
                &format!("https://metamind.dev/data/cascade/{node}"),
                "https://metamind.dev/ontology#nodeId",
                node,
            )
            .map_err(to_mm)?,
            EpistemicStatus::Inferred,
            0.6,
        )
        .map_err(to_mm)?;
        engine.ingest(&claim, &[]).await.map_err(to_mm)?;
    }
    for edge in graph.edges() {
        engine
            .store()
            .insert_justification(edge)
            .await
            .map_err(to_mm)?;
    }
    let affected = engine.invalidate(&root, &reason).await.map_err(to_mm)?;
    println!("epistemic invalidate: root {}", mm_core::ulid_string(&root));
    for node in &affected {
        println!("  affected: {}", mm_core::ulid_string(node));
    }
    println!("  affected_count={}", affected.len());
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

// --------------------------------------------------------------------- world --

async fn world(cfg: Config, query: String) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, true).await?;
    let graph = kernel.graph()?;
    let rows = graph
        .sparql("world", &query)
        .await
        .map_err(|e| MmError::Graph(e.to_string()))?;
    // The gate's own query selects only `?s`, so the status invariant is checked
    // here with a dedicated query: every status in `/world` must be one of the two
    // world-admissible values, whatever the caller asked to see.
    let check = "SELECT ?st WHERE { GRAPH <https://metamind.dev/graph/world> \
                 { ?s <https://metamind.dev/ontology#status> ?st } }";
    let status_rows = graph
        .sparql("world", check)
        .await
        .map_err(|e| MmError::Graph(e.to_string()))?;
    let mut statuses: Vec<String> = Vec::new();
    for row in &status_rows {
        if let Some(object) = row.as_object() {
            if let Some(value) = object.get("st") {
                let text = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string());
                statuses.push(text);
            }
        }
    }
    println!("epistemic world: rows={}", rows.len());
    for status in &statuses {
        println!("  status {status}");
    }
    let bad: Vec<&String> = statuses
        .iter()
        .filter(|s| s.as_str() != "OBSERVED" && s.as_str() != "VERIFIED")
        .collect();
    if !bad.is_empty() {
        println!(
            "epistemic world: {} non-admissible status(es): {:?}",
            bad.len(),
            bad
        );
    }
    graph.shutdown().await?;
    kernel.sqlite.close().await;
    if bad.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

// ----------------------------------------------------------------- data-tests --

async fn data_tests_cmd(cfg: Config, graph_name: String) -> Result<ExitCode, MmError> {
    if graph_name != EPISTEMIC_GRAPH {
        return Err(MmError::Config(format!(
            "unknown graph {graph_name:?}; only `epistemic` has a data-test suite"
        )));
    }
    let (kernel, engine) = open_engine(cfg).await?;
    let snapshot = engine.snapshot().await.map_err(to_mm)?;
    let findings = data_tests(&snapshot.claims, &snapshot.assumptions);
    let violations = findings
        .iter()
        .filter(|f| f.severity == mm_epistemic::Severity::Violation)
        .count();
    println!(
        "epistemic data-tests: graph {graph_name} findings={} violations={violations}",
        findings.len()
    );
    for finding in &findings {
        println!(
            "  [{}] {} {}: {}",
            finding.severity.as_str(),
            finding.test_id,
            finding.subject,
            finding.detail
        );
    }
    shutdown(kernel).await?;
    if violations == 0 {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}
