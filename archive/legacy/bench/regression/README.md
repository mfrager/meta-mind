# `bench/regression` — the mistake compiler's suite

Every directory here is one regression case: a bug that was made, reduced to something a
machine can check, and kept. `mm-cli regression run --suite bench/regression` runs the
whole suite; `mm-cli regression run --suite bench/regression --assert-fail-before-pass-after <name>`
runs one case and asserts both halves of its contract.

## The case format

```
bench/regression/<name>/
  case.json    the contract: what the case is and which scripts play which part
  check.sh     the test. Exit 0 when the subject behaves, non-zero while the bug is present.
  fix.sh       apply the fix
  reset.sh     restore the bug
```

`case.json`:

```json
{
  "name": "seeded_bug_01",
  "mistake_ulid": "01h0000000000000000000m101",
  "description": "one sentence an operator can read",
  "fail_before": "check.sh",
  "fix": "fix.sh",
  "reset": "reset.sh",
  "expected": { "fails_before": true, "passes_after": true }
}
```

Three properties make a case worth keeping, and the runner enforces all three:

* **It fails before.** `reset.sh`, then `check.sh` must exit non-zero. A test that passes
  on the buggy code is not a regression test; it is a test of something else.
* **It passes after.** `fix.sh`, then `check.sh` must exit zero.
* **It is hermetic and resettable.** The runner leaves the case in its buggy state, so a
  second run sees the same thing as the first. Scripts read and write only inside their
  own directory and use `/bin/sh`, so the suite runs identically on a machine with no
  toolchain installed.

The runner copies the case into a private temporary directory before running it, so a
suite run never mutates the tree it was pointed at and two runs cannot race over the same
file. Anything the scripts generate — `seeded_bug_01`'s `subject.sh` — is written inside
that copy and is not source: it is git-ignored, and `reset.sh` creates it either way.

`mm-mistakes::emit` writes exactly this directory shape from a recorded mistake: the
`check.sh` it generates is the minimization of the incident, and `mistake_ulid` is the
Phase 5 mistake the case came from. A hand-written case leaves `mistake_ulid` null, which
is how `seeded_bug_01` starts.
