# `bench/promotion` — the frozen promotion harness

A candidate is promoted or rejected on evidence, and this directory is the part of the
evidence that must not move while a candidate is being judged.

* `promotion_bench.json` — the spec `mm-selfeng::benchmark` reads: the command, the
  frozen `baseline_value`, and the `noise_margin`.
* `score.sh` — the command. It prints one number per sample (`1` held, `0` did not) and
  the runner averages them.

## Why the baseline is data

The runner compares the candidate's mean with `baseline_value` recorded in the spec, not
with a fresh run of the current tree. "Better than last time" and "better than the frozen
baseline" are different claims, and only the second is stable across a candidate that
makes everything slower: a candidate that doubles the runtime of the harness would
otherwise move its own baseline and pass.

## Why `noise_margin` is `0.0`

This harness is deterministic — three samples of a pass/fail assertion, no timing, no
sampling. A margin exists for measurements that wobble; with none, any drop below the
baseline is a real drop, which is what the gate's rule 2 then reports. A harness that
measured wall-clock time would carry a non-zero margin and say why.

## Why every sample runs against a copy

The case's scripts rewrite `subject.sh` in place, so `score.sh` copies
`bench/regression/seeded_bug_01` into a private temporary directory and runs the copy. Two
runs of the harness at once — a promotion beside `mm-cli regression run`, or two test
threads in the same binary — would otherwise interleave one sample's `fix` into another
sample's `check` and score a healthy tree as broken, and the harness would stop being
deterministic exactly when the kernel was busiest. The copy is what makes the score a
property of the candidate tree rather than of what else happened to be running, and it
means the harness leaves the repository byte-identical: it never opens a file in it for
writing.

## Why the harness is frozen

The phase's risk table names *reward hacking / self-confirming evaluation*, and the
mitigation is that the harness cannot be edited by the thing it measures. A change set
that rewrote `score.sh` to print `1` would otherwise be judged by its own rewrite.
Two mechanisms keep that honest:

* `mm-cli sandbox run` runs a candidate in a git worktree, so `git diff` shows exactly
  what the candidate changed — including this directory;
* `mm-cli audit production-tree` compares the production tree against the instant the
  promotion started, and an edit here outside a promotion is a finding.

The harness itself recomputes its score from `bench/regression`, so editing this directory
is visible as a diff *and* pointless: the case it runs is the one it reads.
