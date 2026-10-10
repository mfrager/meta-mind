//! `mm-cli frame` — compose conceptual frames and inspect what they left open.
//!
//! Frames are read from the committed seed corpus rather than reconstructed from
//! the SQLite index: a frame's *roles*, *slots* and *scripts* are what composition
//! merges, and those live in the Turtle document (`ontology/seed/library_seed.ttl`)
//! that `mm-cli library import` writes to `/library`. Reading the same file the
//! library was built from is the one way to be sure the composition uses the
//! frames that were validated — a second decoder over the index would be a second
//! place for the slots to drift.
//!
//! Composition itself is [`FrameInstance::compose`], which merges left to right and
//! **never overwrites a known slot**: a later frame may only fill what an earlier
//! one left open. Slots nobody filled are reported, not guessed at, which is what
//! makes `frame missing` a meaningful command.

use std::collections::BTreeMap;
use std::process::ExitCode;

use clap::Subcommand;
use mm_core::{Config, MmError, NamedNode};
use mm_library::manager::TARGET;
use mm_library::{Frame, FrameInstance, SlotValue};
use mm_log::{codes, Level, LogRecord};

use crate::library::{open_manager, shutdown};

/// `mm-cli frame`.
#[derive(Subcommand, Debug)]
pub enum FrameCommand {
    /// Compose frames into an instance for one episode.
    Compose {
        /// Comma-separated frame slugs, in merge order.
        #[arg(long, value_name = "A,B,C")]
        frames: String,
        /// The episode the instance belongs to.
        #[arg(long, value_name = "ULID")]
        episode: String,
    },
    /// The slots a recorded instance left unknown.
    Missing {
        /// The instance's ULID.
        #[arg(value_name = "ULID")]
        ulid: String,
    },
}

/// `mm-cli frame`.
pub async fn run(cfg: Config, command: FrameCommand) -> Result<ExitCode, MmError> {
    match command {
        FrameCommand::Compose { frames, episode } => compose(cfg, &frames, &episode).await,
        FrameCommand::Missing { ulid } => missing(cfg, &ulid).await,
    }
}

/// Compose the named frames for an episode and record the instance.
async fn compose(cfg: Config, frames: &str, episode: &str) -> Result<ExitCode, MmError> {
    let slugs = parse_slugs(frames)?;
    let episode = mm_core::id::parse_ulid(episode)?;
    let available = seed_frames()?;

    let mut chosen: Vec<Frame> = Vec::with_capacity(slugs.len());
    for slug in &slugs {
        match available.get(slug) {
            Some(frame) => chosen.push(frame.clone()),
            None => {
                return Err(MmError::Config(format!(
                    "unknown frame {slug:?}; the seed declares {}",
                    available.keys().cloned().collect::<Vec<_>>().join(", ")
                )))
            }
        }
    }

    let (kernel, manager) = open_manager(cfg).await?;
    let id = manager.store().next_id();
    let instance = FrameInstance::compose(id, episode, &chosen, BTreeMap::new())?;

    let parents: Vec<String> = instance
        .parents
        .iter()
        .map(|node| node.as_str().to_string())
        .collect();
    let slots: BTreeMap<String, String> = instance
        .slots
        .iter()
        .map(|(slot, value)| (slot.clone(), render(value)))
        .collect();
    manager
        .store()
        .insert_frame_instance(
            &instance.ulid,
            instance
                .parents
                .first()
                .map(NamedNode::as_str)
                .unwrap_or_default(),
            &instance.episode,
            &serde_json::to_string(&parents).map_err(internal)?,
            &serde_json::to_string(&slots).map_err(internal)?,
            &serde_json::to_string(&instance.missing).map_err(internal)?,
        )
        .await?;

    let known = instance
        .slots
        .iter()
        .filter(|(_, value)| !value.is_unknown())
        .count();
    manager
        .logger()
        .emit(
            LogRecord::new(Level::Info, codes::LIBRARY_FRAME_COMPOSE, TARGET)
                .with_field("instance_ulid", mm_core::ulid_string(&instance.ulid))
                .with_field("frame_iri", parents.first().cloned().unwrap_or_default())
                .with_field("parents", parents.clone())
                .with_field("slots_known", known)
                .with_field("slots_missing", instance.missing.clone()),
        )
        .await?;

    println!(
        "instance {} ({} frame(s), episode {})",
        mm_core::ulid_string(&instance.ulid),
        chosen.len(),
        mm_core::ulid_string(&episode)
    );
    // A slot is printed once: the known ones with their value, the gaps in the
    // `missing` list below, which is the list a caller reads to see what the
    // composition could not fill.
    for (slot, value) in &instance.slots {
        if value.is_unknown() {
            continue;
        }
        println!("  {} = {}", slot, render(value));
    }
    for slot in &instance.missing {
        println!("  {slot} = unknown");
    }
    println!(
        "known {} of {}, missing {}",
        known,
        instance.slots.len(),
        instance.missing.len()
    );

    shutdown(kernel).await?;
    Ok(ExitCode::SUCCESS)
}

/// Print the slots a recorded instance left unknown.
async fn missing(cfg: Config, id: &str) -> Result<ExitCode, MmError> {
    let ulid = mm_core::id::parse_ulid(id)?;
    let (kernel, manager) = open_manager(cfg).await?;
    let found = manager.store().frame_instance_missing(&ulid).await?;
    shutdown(kernel).await?;

    match found {
        Some(slots) => {
            for slot in &slots {
                println!("{slot}");
            }
            if slots.is_empty() {
                println!("no missing slots");
            }
            Ok(ExitCode::SUCCESS)
        }
        None => Err(MmError::Config(format!(
            "no frame instance {id}; compose one with `mm-cli frame compose`"
        ))),
    }
}

/// A comma-separated frame list, refusing an empty element rather than skipping it.
fn parse_slugs(text: &str) -> Result<Vec<String>, MmError> {
    let mut out = Vec::new();
    for slug in text.split(',') {
        let slug = slug.trim();
        if slug.is_empty() {
            return Err(MmError::Config(format!(
                "frame list {text:?} has an empty entry"
            )));
        }
        out.push(slug.to_string());
    }
    if out.is_empty() {
        return Err(MmError::Config("frame list is empty".into()));
    }
    Ok(out)
}

/// How a slot value is rendered and stored.
///
/// The stored form is tagged (`known:`, `inferred:`, `unknown`) so a reader can
/// tell an inference from an observation without a second column.
fn render(value: &SlotValue) -> String {
    match value {
        SlotValue::Known(text) => format!("known:{text}"),
        SlotValue::Inferred(text) => format!("inferred:{text}"),
        SlotValue::Unknown => "unknown".to_string(),
    }
}

fn internal(e: serde_json::Error) -> MmError {
    MmError::Internal(format!("frame instance json: {e}"))
}

/// The frames the committed seed declares, by slug.
fn seed_frames() -> Result<BTreeMap<String, Frame>, MmError> {
    let path = Config::repo_root()
        .join("ontology")
        .join("seed")
        .join("library_seed.ttl");
    let turtle = std::fs::read_to_string(&path)
        .map_err(|e| MmError::Config(format!("cannot read {}: {e}", path.display())))?;
    let canonical = mm_store_graph::canonical_ntriples_of_turtle(&turtle)?;

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in canonical.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let subject = line
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string();
        groups.entry(subject).or_default().push(line.to_string());
    }

    let mut frames = BTreeMap::new();
    for (subject, lines) in &groups {
        if !types_include(lines, "Frame") {
            continue;
        }
        let subject = subject.trim_start_matches('<').trim_end_matches('>');
        let slug = subject.rsplit('/').next().unwrap_or(subject).to_string();
        let title = literals_of(lines, "title")
            .into_iter()
            .next()
            .unwrap_or_default();
        let purpose = literals_of(lines, "purpose")
            .into_iter()
            .next()
            .unwrap_or_default();
        let mut frame = Frame::new(&slug, &title, &purpose)?;
        for domain in literals_of(lines, "domain") {
            frame = frame.with_domain(&domain);
        }
        for role in literals_of(lines, "role") {
            frame = frame.with_role(&role, &role);
        }
        for slot in literals_of(lines, "frameSlot") {
            frame = frame.with_slot(&slot);
        }
        for action in literals_of(lines, "typicalAction") {
            frame = frame.with_action(&action);
        }
        for affordance in literals_of(lines, "affordance") {
            frame = frame.with_action(&affordance);
        }
        for mode in literals_of(lines, "failureMode") {
            frame = frame.with_failure_mode(&mode);
        }
        for step in literals_of(lines, "scriptStep") {
            frame = frame.with_action(&step);
        }
        if let Some(parent) = iri_of(lines, "parentFrame") {
            frame = frame.with_parent(NamedNode::new_unchecked(parent));
        }
        frames.insert(slug, frame);
    }

    if frames.is_empty() {
        return Err(MmError::Config(format!(
            "the seed at {} declares no frames",
            path.display()
        )));
    }
    Ok(frames)
}

/// True when the subject's type list names `mm:{class}`.
fn types_include(lines: &[String], class: &str) -> bool {
    let target = format!("https://metamind.dev/ontology#{class}");
    lines
        .iter()
        .any(|line| line.contains("22-rdf-syntax-ns#type") && line.contains(&format!("<{target}>")))
}

/// Every literal value of a predicate, in line order.
fn literals_of(lines: &[String], local: &str) -> Vec<String> {
    mm_library::rdf::literals_in(lines, local)
}

/// The first IRI value of a predicate, if there is one.
fn iri_of(lines: &[String], local: &str) -> Option<String> {
    mm_library::rdf::node_in(lines, local).map(|node| node.into_string())
}
