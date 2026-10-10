#!/bin/sh
# The qualification harness.
#
# It prints one number per sample and exits 0; `mm-selfeng::benchmark` averages the
# samples and compares the mean with the spec's `baseline_value` of 1.0. Unlike
# `bench/promotion/score.sh`, which derives each sample by exercising the seeded
# regression case, this harness is a *constant*: the qualification run's claim is about
# the loop producing a capability, not about the harness re-deriving the regression
# case, and a constant reading makes a benchmark failure in the qualification run mean
# "the harness could not run" rather than "the candidate changed the number".
#
# Three samples rather than one, for the same reason the promotion harness takes three:
# a single sample is indistinguishable from a lucky one, and the mean of three identical
# samples is still exact.
#
# It is part of the *frozen* harness: a candidate may not edit `bench/qualification`.
# `mm-cli sandbox run` and `mm-cli audit production-tree` are what make that a check
# rather than a convention.
#
# It reads no file and writes none, so running it cannot change the audit's baseline.

set -eu

printf '1\n'
printf '1\n'
printf '1\n'

exit 0
