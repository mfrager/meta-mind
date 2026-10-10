#!/bin/sh
# The frozen promotion harness.
#
# It prints one number per sample, and `mm-selfeng::benchmark` averages them: 1 for a
# sample that held, 0 for one that did not. The samples come from the seeded regression
# case, which is the one property this harness can assert about any candidate tree
# without a compiler:
#
#   * reset the case, and the buggy subject must make `check.sh` fail (the bug is real);
#   * apply the case's fix, and `check.sh` must pass (the fix is real);
#   * reset again, so the harness leaves the tree as it found it.
#
# Three samples rather than one, because a single sample is indistinguishable from a
# lucky one and the mean of three is still exact.
#
# This file is part of the *frozen* harness: a candidate may not edit `bench/promotion`.
# The reason is the phase's reward-hacking risk — if a change set could rewrite the thing
# that measures it, the gate would be measuring the rewrite. `mm-cli sandbox run` and
# `mm-cli audit production-tree` are what make that a check rather than a convention.
#
# Every sample runs against a **private copy** of the case, because the case's scripts
# rewrite `subject.sh` in place (`dirname "$0"`). Running them in the repository mutates
# the tree the harness is measuring, and two runs of the harness at once — a promotion
# beside a `regression run`, or two test threads — would interleave one sample's `fix`
# into another sample's `check` and score a healthy tree as broken. With a copy per
# sample the score is a property of the candidate tree and not of what else was running,
# and the repository is never written at all.

set -eu

case_dir=bench/regression/seeded_bug_01

if [ ! -f "$case_dir/case.json" ]; then
  echo "the seeded regression case is missing" >&2
  exit 2
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cp -R "$case_dir" "$work/case"

sample() {
  sh "$work/case/reset.sh"
  # Before the fix the test must fail. A case that passes here is not a regression test.
  if sh "$work/case/check.sh" >/dev/null 2>&1; then
    sh "$work/case/reset.sh"
    printf '0\n'
    return 0
  fi
  sh "$work/case/fix.sh"
  if sh "$work/case/check.sh" >/dev/null 2>&1; then
    sh "$work/case/reset.sh"
    printf '1\n'
    return 0
  fi
  sh "$work/case/reset.sh"
  printf '0\n'
}

sample
sample
sample
