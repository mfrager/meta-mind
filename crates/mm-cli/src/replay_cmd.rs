//! `mm-cli replay` — rebuild state from the log and report the hash it reached.

use std::process::ExitCode;

use mm_core::{MmError, Timestamp};
use mm_eventlog::{CountingApplier, DEFAULT_CHECKPOINT};
use mm_store_sqlite::AsOf;

use crate::kernel::Kernel;

/// Replay committed events into a counting projection.
pub async fn run(
    cfg: mm_core::Config,
    from: i64,
    until: Option<i64>,
    as_of: Option<&str>,
) -> Result<ExitCode, MmError> {
    let kernel = Kernel::open(cfg, false).await?;

    let point = match (until, as_of) {
        (Some(seq), None) => AsOf::system(u64::try_from(seq).map_err(|_| {
            MmError::Config(format!(
                "--until must be a non-negative sequence, got {seq}"
            ))
        })?),
        (None, Some(text)) => {
            let parsed = if let Ok(seconds) = text.parse::<u64>() {
                Timestamp::from_epoch_seconds(seconds)
            } else {
                Timestamp::from_rfc3339(text)?
            };
            AsOf::valid(parsed)
        }
        (Some(seq), Some(text)) => {
            let parsed = if let Ok(seconds) = text.parse::<u64>() {
                Timestamp::from_epoch_seconds(seconds)
            } else {
                Timestamp::from_rfc3339(text)?
            };
            AsOf::at(
                u64::try_from(seq).map_err(|_| {
                    MmError::Config(format!(
                        "--until must be a non-negative sequence, got {seq}"
                    ))
                })?,
                parsed,
            )
        }
        (None, None) => AsOf::now(),
    };

    if from > 1 {
        // `--from` selects where to resume reading; the fold must still start from
        // the log's beginning to produce a hash that means anything.
        println!(
            "note: --from {from} requested; a replay hash always folds the whole committed prefix"
        );
    }

    let mut applier = CountingApplier::new();
    let snapshot = kernel.events.replay(point, &mut applier).await?;
    let checkpoint = kernel.events.checkpoint_of(DEFAULT_CHECKPOINT).await?;

    println!("replay from 1 to {}", snapshot.last_seq);
    println!("  events applied      {}", snapshot.applied);
    println!("  aborted provisional {}", snapshot.aborted.len());
    println!("  state_hash          {}", snapshot.log_hash);
    println!("  applier_hash        {}", snapshot.applier_hash);
    match checkpoint {
        Some((seq, hash)) if seq == snapshot.last_seq => {
            let matched = hash == snapshot.log_hash;
            println!(
                "  checkpoint          seq {seq} {}",
                if matched { "match" } else { "MISMATCH" }
            );
            if !matched {
                println!("    checkpoint hash   {hash}");
                return Ok(ExitCode::from(1));
            }
        }
        Some((seq, hash)) => {
            println!(
                "  checkpoint          seq {seq} (not comparable at seq {})",
                snapshot.last_seq
            );
            println!("    checkpoint hash   {hash}");
        }
        None => println!("  checkpoint          none recorded"),
    }
    Ok(ExitCode::SUCCESS)
}
