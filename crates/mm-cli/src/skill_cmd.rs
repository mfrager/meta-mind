//! `mm-cli skill` — register, verify and retrieve verified skills.
//!
//! The rule this surface exists to enforce is Voyager's: a skill is usable only
//! after its test passes, and `skill retrieve` returns verified skills and nothing
//! else. The state lives on the record (`skills.verification`), not in a flag the
//! caller passes, so a draft cannot be retrieved by mistake.
//!
//! **How verification is decided.** The plan runs a skill's test in the Phase 10
//! sandbox; that runner does not exist yet, so `skill verify` decides from the two
//! facts that are checkable today, which are exactly the facts a sandbox would
//! report back:
//!
//! * the declared artefact (`implRef`) is **present**, and when a sidecar
//!   `<implRef>.sha256` exists its first 64 hex characters equal the file's sha256
//!   — so a change to the artefact is caught, and an unpinned artefact is
//!   honestly reported as unpinned rather than silently trusted;
//! * the test spec (`testsRef`) reports `pass == cases` with `cases > 0`.
//!
//! Nothing consults a model or a network. The seam is `Skill::decide`, so when the
//! sandbox lands only this module's two facts are replaced.
//!
//! **Where the index comes from.** The `/library` entry is the authority and the
//! `skills` row is a projection of it. `library import` writes the entry; this
//! command rebuilds the missing index row from the entry's own fields (including
//! the declared `verification`) before deciding anything, so `skill verify` works
//! on a freshly imported library rather than only on one that has been registered
//! through this CLI.
//!
//! **Where the artefact is.** `/data/**` is not committed (it is the runtime data
//! directory), so the seed's `mm:implRef` — `data/library/skills/…` — names a file
//! a fresh checkout does not have. The declared path stays authoritative: it is
//! used when it exists, and otherwise the committed fixture mirror
//! `bench/library/skills/<file-name>` stands in for it. Which of the two answered
//! is printed, never inferred, so a run against the mirror is visible rather than
//! silently equal to a run against the real artefact.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use mm_core::{Config, MmError, Timestamp};
use mm_library::skill::{retrieve, Skill, SkillTestSpec, Verification, VERIFICATIONS};
use mm_library::LibraryManager;
use mm_log::{codes, Level, LogRecord};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::library::{open_manager, shutdown};

/// `mm-cli skill`.
#[derive(Subcommand, Debug)]
pub enum SkillCommand {
    /// Register a skill from a JSON manifest. It starts as a draft.
    Register {
        /// `{slug, name, description, impl_ref, entrypoint, signature, tests_ref}`.
        #[arg(long, value_name = "FILE")]
        manifest: PathBuf,
    },
    /// Decide a skill's verification from its declared artefact and test spec.
    Verify {
        /// The skill's versioned IRI.
        #[arg(value_name = "IRI")]
        iri: String,
    },
    /// Retrieve verified skills for a query.
    Retrieve {
        /// What the caller wants to do.
        #[arg(long, value_name = "TEXT")]
        query: String,
        /// How many skills to return.
        #[arg(long, default_value_t = 3)]
        top: usize,
    },
}

/// The manifest `skill register` reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillManifest {
    slug: String,
    name: String,
    description: String,
    impl_ref: String,
    entrypoint: String,
    signature: String,
    tests_ref: String,
}

/// `mm-cli skill`.
pub async fn run(cfg: Config, command: SkillCommand) -> Result<ExitCode, MmError> {
    match command {
        SkillCommand::Register { manifest } => register(cfg, manifest).await,
        SkillCommand::Verify { iri } => verify(cfg, &iri).await,
        SkillCommand::Retrieve { query, top } => retrieve_cmd(cfg, &query, top).await,
    }
}

/// Register a skill as a draft, and pin its artefact when the file is present.
async fn register(cfg: Config, manifest: PathBuf) -> Result<ExitCode, MmError> {
    let raw = std::fs::read_to_string(&manifest).map_err(|e| {
        MmError::Config(format!("cannot read manifest {}: {e}", manifest.display()))
    })?;
    let spec: SkillManifest = serde_json::from_str(&raw)
        .map_err(|e| MmError::Config(format!("manifest {}: {e}", manifest.display())))?;

    let skill = Skill::new(
        &spec.slug,
        &spec.name,
        &spec.description,
        &spec.impl_ref,
        &spec.entrypoint,
        &spec.signature,
        &spec.tests_ref,
    )?;

    let (kernel, manager) = open_manager(cfg).await?;
    let outcome = manager.register_skill(&skill).await?;
    let pinned = pin_artefact(&spec.impl_ref)?;

    println!("registered {}", outcome.version_iri);
    println!("  verification  {}", Verification::Draft);
    let (found, real) = resolve_artefact(&spec.impl_ref);
    match (pinned, real) {
        (Some(hash), _) => println!(
            "  artefact      {} pinned {}",
            found.display(),
            &hash[..16.min(hash.len())]
        ),
        (None, true) => println!("  artefact      {} absent, nothing pinned", found.display()),
        (None, false) => println!(
            "  artefact      {} (fixture mirror), nothing pinned — /data is uncommitted",
            found.display()
        ),
    }
    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

/// Decide a skill's verification, persist it, and report it.
async fn verify(cfg: Config, iri: &str) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    ensure_indexed(&manager).await?;

    let Some(mut skill) = load_skill(&manager, iri).await? else {
        return Err(MmError::Config(format!(
            "no skill is indexed under {iri}; `library import` its entry or `skill register` it first"
        )));
    };

    let (artefact, real) = resolve_artefact(&skill.impl_ref);
    let sidecar = PathBuf::from(format!("{}.sha256", artefact.display()));
    let artefact_present = artefact.is_file();
    let artefact_hash_matches = if !artefact_present {
        false
    } else if sidecar.is_file() {
        match (read_sha256(&sidecar)?, sha256_file(&artefact)?) {
            (Some(pinned), actual) => pinned.eq_ignore_ascii_case(&actual),
            (None, _) => false,
        }
    } else {
        // Present but unpinned: the artefact exists and nothing contradicts it.
        true
    };

    let (spec_path, spec) = load_spec(Path::new(&skill.tests_ref), &skill.slug())?;
    let state = skill.decide(artefact_hash_matches, &spec, Timestamp::now());
    let exit_code = i32::from(state != Verification::Verified);
    manager
        .set_skill_verification(iri, state, exit_code)
        .await?;

    println!("skill         {iri}");
    println!(
        "  artefact      {}{}",
        artefact.display(),
        if real {
            ""
        } else {
            " (fixture mirror; the declared data/ path is absent)"
        }
    );
    println!(
        "  hash          {}",
        if artefact_hash_matches {
            if sidecar.is_file() {
                "matches the pinned sidecar"
            } else {
                "present, no sidecar to pin it"
            }
        } else if !artefact_present {
            "artefact missing"
        } else {
            "does not match the pinned sidecar"
        }
    );
    println!(
        "  spec          {} ({}/{})",
        spec_path.display(),
        spec.pass,
        spec.cases
    );
    println!("  verification  {state}");

    shutdown(kernel).await?;
    if state == Verification::Verified {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

/// Retrieve verified skills for a query.
async fn retrieve_cmd(cfg: Config, query: &str, top: usize) -> Result<ExitCode, MmError> {
    let (kernel, manager) = open_manager(cfg).await?;
    ensure_indexed(&manager).await?;

    let skills: Vec<Skill> = manager
        .store()
        .list_skills()
        .await?
        .iter()
        .map(skill_from_row)
        .collect::<Result<Vec<_>, MmError>>()?;
    let verified = skills.iter().filter(|s| s.verification.is_usable()).count();
    let hits = retrieve(&skills, query, top);

    let query_hash = mm_core::hash_fields(&[query]);
    for hit in &hits {
        println!(
            "{:.3} {} {} {}",
            hit.score, hit.verification, hit.iri, hit.name
        );
    }
    if hits.is_empty() {
        println!(
            "no verified skill matches {query:?} ({verified} of {} indexed skills are verified)",
            skills.len()
        );
    }

    for hit in &hits {
        manager
            .logger()
            .emit(
                LogRecord::new(Level::Info, codes::SKILL_RETRIEVE, mm_library::TARGET)
                    .with_field("query_hash", query_hash.clone())
                    .with_field("skill_iri", hit.iri.clone())
                    .with_field("score", f64::from(hit.score))
                    .with_field("verification", hit.verification.as_str()),
            )
            .await?;
    }

    shutdown(kernel).await?;
    if hits.is_empty() {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// The skill indexed under a versioned IRI, when there is one.
async fn load_skill(manager: &LibraryManager, iri: &str) -> Result<Option<Skill>, MmError> {
    let rows = manager.store().list_skills().await?;
    match rows.iter().find(|row| row.iri == iri) {
        Some(row) => Ok(Some(skill_from_row(row)?)),
        None => Ok(None),
    }
}

/// Index every skill entry that has no `skills` row.
///
/// The entry carries the fields — `title`, `purpose`, `implRef`, `entrypoint`,
/// `signature`, `testedBy` and the declared `verification` — so the row is a
/// projection, not a second source of truth. Rebuilding it on demand is what lets
/// `library import` and `skill verify` be run in either order.
async fn ensure_indexed(manager: &LibraryManager) -> Result<(), MmError> {
    let existing: Vec<String> = manager
        .store()
        .list_skills()
        .await?
        .iter()
        .map(|row| row.iri.clone())
        .collect();
    for entry in manager.store().list_entries(Some("Skill")).await? {
        if existing.contains(&entry.version_iri) {
            continue;
        }
        let fields = |name: &str| literal_of(&entry.body_ttl, name).unwrap_or_default();
        let verification =
            Verification::parse(&fields("verification")).unwrap_or(Verification::Draft);
        // `mm:testedBy` is a reference, not a literal: the seed points it at the
        // evaluation entry that decides the skill, and a literal-only reader would
        // silently index an empty test reference.
        let tests_ref = node_of(&entry.body_ttl, "testedBy")
            .or_else(|| literal_of(&entry.body_ttl, "testedBy"))
            .unwrap_or_default();
        manager
            .store()
            .insert_skill(
                &entry.version_iri,
                &entry.title,
                &fields("purpose"),
                &fields("implRef"),
                &fields("entrypoint"),
                &fields("signature"),
                &tests_ref,
                verification.as_str(),
                entry.version,
            )
            .await?;
    }
    Ok(())
}

/// A skill as the `skills` row records it.
fn skill_from_row(row: &mm_library::SkillRow) -> Result<Skill, MmError> {
    let old = mm_core::NamedNode::new_unchecked(row.iri.clone());
    let verification = Verification::parse(&row.verification).ok_or_else(|| {
        MmError::Store(format!(
            "{}: {:?} is not one of {VERIFICATIONS:?}",
            row.iri, row.verification
        ))
    })?;
    Ok(Skill {
        iri: old,
        name: row.name.clone(),
        description: row.description.clone(),
        impl_ref: row.impl_ref.clone(),
        entrypoint: row.entrypoint.clone(),
        signature: row.signature.clone(),
        tests_ref: row.tests_ref.clone(),
        verification,
        verified_at: None,
        embedding_ref: None,
        version: row.version.max(1) as u32,
    })
}

/// The test spec for a skill, and the file it was read from.
///
/// `testsRef` is a *library* IRI in the seed corpus
/// (`…/library/evaluation/regression-suite`), not a filesystem path, so the spec
/// file is looked up in the two places this phase defines, in order:
///
/// 1. `testsRef` itself, when it is a path that exists;
/// 2. `bench/library/skills/<last-segment>.json`;
/// 3. `data/library/skills/<skill-slug>.test.json`.
///
/// A refusl names every path it tried, because "the spec is missing" is useless
/// without saying where it looked.
fn load_spec(tests_ref: &Path, slug: &str) -> Result<(PathBuf, SkillTestSpec), MmError> {
    let last = tests_ref
        .to_string_lossy()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();
    // Every candidate is resolved against the repository root when the relative
    // path does not exist from the process's working directory, so the command
    // behaves the same from the repo root and from a test's package directory.
    let candidates = [
        repo_path(&tests_ref.to_string_lossy()),
        repo_path(&format!("bench/library/skills/{last}.json")),
        repo_path(&format!("data/library/skills/{slug}.test.json")),
    ];
    for candidate in &candidates {
        if candidate.is_file() {
            let raw = std::fs::read_to_string(candidate).map_err(|e| {
                MmError::Config(format!("cannot read {}: {e}", candidate.display()))
            })?;
            let spec: SkillTestSpec = serde_json::from_str(&raw)
                .map_err(|e| MmError::Config(format!("spec {}: {e}", candidate.display())))?;
            return Ok((candidate.clone(), spec));
        }
    }
    Err(MmError::Config(format!(
        "no test spec for testsRef {tests_ref:?}; tried {}",
        candidates
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )))
}

/// The artefact a skill declares, and whether the declared path itself exists.
///
/// `/data/**` is uncommitted, so the declared path is tried first and the
/// committed fixture mirror second. The bool is the honest part: it says which of
/// the two answered, so a caller can print it instead of pretending they are the
/// same file.
fn resolve_artefact(impl_ref: &str) -> (PathBuf, bool) {
    let declared = repo_path(impl_ref);
    if declared.is_file() {
        return (declared, true);
    }
    if let Some(name) = declared.file_name() {
        let mirror = repo_path(&format!("bench/library/skills/{}", name.to_string_lossy()));
        if mirror.is_file() {
            return (mirror, false);
        }
    }
    (declared, true)
}

/// A path relative to the repository, resolved from either the current directory
/// or the repository root.
///
/// The CLI is run from the repo root by hand and from a package directory under
/// `cargo test`, and a fixture lookup must not depend on which.
fn repo_path(relative: &str) -> PathBuf {
    let direct = PathBuf::from(relative);
    if direct.exists() {
        return direct;
    }
    mm_core::Config::repo_root().join(relative)
}

/// Write `<implRef>.sha256` when the declared artefact exists and no sidecar pins
/// it yet.
///
/// Pinning at registration is what makes the later check meaningful: `verify` can
/// then tell a changed artefact from an unchanged one, and a library that was
/// imported rather than registered simply reports that the artefact is unpinned.
///
/// Only the **declared** path is pinned. The committed fixture mirror is read-only:
/// writing a sidecar beside it would put a generated file in a committed fixture
/// directory, and the mirror exists precisely because `/data` is uncommitted, so a
/// sidecar there could never travel with the artefact it describes.
fn pin_artefact(impl_ref: &str) -> Result<Option<String>, MmError> {
    let artefact = repo_path(impl_ref);
    if !artefact.is_file() {
        return Ok(None);
    }
    let sidecar = repo_path(&format!("{impl_ref}.sha256"));
    if sidecar.is_file() {
        return read_sha256(&sidecar);
    }
    let hash = sha256_file(&artefact)?;
    std::fs::write(&sidecar, format!("{hash}\n"))
        .map_err(|e| MmError::Config(format!("cannot write {}: {e}", sidecar.display())))?;
    Ok(Some(hash))
}

/// The first sha256-shaped token in a sidecar file, if it has one.
fn read_sha256(path: &Path) -> Result<Option<String>, MmError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    Ok(raw
        .split_whitespace()
        .find(|token| token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string))
}

/// The lowercase sha256 of a file.
fn sha256_file(path: &Path) -> Result<String, MmError> {
    let bytes = std::fs::read(path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// A literal value by local predicate name, in an entry body.
///
/// The body is one statement per line, so the reading is the library's own: the
/// object is the *rest* of the line, because a title contains spaces and a reader
/// that took the next token would see `"API` and find nothing.
fn literal_of(body_ntriples: &str, local: &str) -> Option<String> {
    mm_library::rdf::literal_in(&body_lines(body_ntriples), local)
}

/// A named-node reference by local predicate name, as a bare IRI.
fn node_of(body_ntriples: &str, local: &str) -> Option<String> {
    mm_library::rdf::node_in(&body_lines(body_ntriples), local).map(|node| node.into_string())
}

/// An entry body as statements.
fn body_lines(body_ntriples: &str) -> Vec<String> {
    body_ntriples
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}
