# calibration

**URI** `https://metamind.dev/code/module/cognition/calibration`
**Phase** 11 (self-engineering)
**Capability** `mm:CalibrationSummary`

## What it is for

Every Metamind module is a nexus-style plugin: a directory with a `plugin.toml`
manifest, a stable `uri`, and the T-Box functions it adds to the monad.

This module closes the capability gap recorded in `bench/gaps/gap_01.json`:

> no module can summarize a drifting calibration bin, so a class that has become
> overconfident is only visible as a number an operator has to read.

An operator reading `brier 0.1762` cannot tell whether the system is trustworthy or
merely noisy, and cannot tell *which* part of it stopped being trustworthy. This module
answers both: it scores a labeled set, fits one temperature per class, reports the
reliability curve as bins with edges, and — the field the gap asked for — names the
classes whose error is still too large after fitting. That list is what a change set is
assembled from, which is why it is a list and not a boolean.

It is a thin adapter on purpose. The Brier score, the log loss, the expected calibration
error and the temperature fit live in `crates/mm-decision/src/calibration.rs`, where they
already have tests and a labeled bench; a second formula here would mean the kernel's own
decision path was graded by a different definition of "calibrated" than the summary a
promotion is judged by.

## T-Box functions

| Function | Handler | Input | Output |
|---|---|---|---|
| `cognition.calibration_summary` | `handlers::calibration_summary` | `points`: a JSON array of `{class, predicted, label}`; `baseline_brier` | the `CalibrationSummary`: `{n, brier, log_loss, ece, bins, drifting, baseline_brier}` |
| `cognition.calibration_drift` | `handlers::calibration_drift` | `points`; `tolerance` | `{n, tolerance, drifting_count, drifting}` |

Rust callers can use the same functions with types — `summarize`, `drifting_classes`,
`score_at`, `fitted_temperatures`, `reliability_bins`, `parse_jsonl` — without going
through JSON.

## Contract

- **The summary is computed at the fitted temperature.** Reporting the raw numbers would
  report the problem instead of the answer; the point of the module is to say what the
  confidence *would* mean once fitted, and then whether that is still too large. A class
  with too little data to fit keeps `T = 1.0` and is scored uncalibrated, which is
  `mm-decision`'s rule rather than a second one.
- **`drifting` is a sorted list of classes, and it is the module's reason to exist.**
  The default tolerance is `DEFAULT_DRIFT_TOLERANCE = 0.05` — a class whose
  post-calibration ECE exceeds five points of probability. A caller that wants a
  different bar passes one; the default is a policy number, so it is a named constant
  rather than a literal inside the fit.
- **A refusal is typed and nothing is clamped.** A prediction outside `[0,1]`, a
  non-finite probability, an empty class, a malformed line, a non-array payload, and a
  negative or non-finite tolerance are all refused: `json` when the payload is not the
  shape the function takes, `validation` when a value is understood and impossible. A
  clamped probability would let a caller's bug read as a well-calibrated answer.
- **The module touches no table.** `READS` and `WRITES` are both empty: it is pure
  arithmetic over points handed to it, so a replayed episode or a handful of
  hand-written cases can be scored without a store. What persists a run is
  `mm-metaanalysis`'s `calibration_runs` row, written by the crate that owns the ledger.
- **The number and the picture agree.** The reliability bins use the kernel's binning
  (`floor(p * 10)`, last bin right-inclusive, empty bins skipped) and the ECE comes from
  `mm-decision`, so the curve an operator reads reconstructs the error printed beside it;
  the module's own test asserts exactly that, to 1e-9.
- **The module is graded against a reference it did not produce.**
  `bench/calibration/reference.json` is computed by
  `bench/calibration/compute_reference.py`, a second implementation of the documented
  metrics. The test asserts the module reproduces it — to 1e-7 for the metrics given the
  reference's own temperatures, which is the width of the difference between the corpus's
  `f32` probabilities and the script's `f64` ones — and that the fitted temperatures land
  on the same grid point.
- The manifest is embedded with `include_str!` and parsed by a test, so a malformed
  `plugin.toml`, or a `uri` that has drifted from this directory, fails a test rather than
  a load.

## How it was produced

This module is the artifact of Phase 11's first generated-module workflow: a gap
(`bench/gaps/gap_01.json`) became a typed `ChangeSet`, Pi scaffolded the module from
`pi/skills/mm-module-scaffold/SKILL.md`, the sandbox built and tested it, and the
promotion gate admitted it. Its `uri` is the gap's `target_uri`, so the gap is closed by
a record rather than by an assertion.

## Ownership

Phase 11 owns this module and the self-engineering phase that scaffolded it. The metrics
it reports are Phase 9's (`mm-decision::calibration`), the corpus and the reference are
Phase 11's (`bench/calibration/`), and the drift list is what Phase 11's meta-analysis
reads when it decides whether a class needs a change set. Later phases may *read* this
module — the code graph registers its URI, its T-Box functions and the tests that cover
its capability — but must not change its contract.
