#!/usr/bin/env python3
"""Compute the calibration reference numbers for Phase 11's gate.

This script exists so `bench/calibration/reference.json` is not produced by the
implementation it grades. `mm-metaanalysis::calibration` must reproduce these numbers
within 1e-9; if the Rust code and this script ever disagree, one of them is wrong, and
the disagreement is visible rather than silently absorbed.

The conventions are the ones the kernel documents, and they are restated here because a
reference computed under different conventions measures nothing:

  * Brier is the mean squared error of the probability against a 0/1 label.
  * Log loss clamps the probability into [1e-9, 1 - 1e-9] before the logarithm, so the
    poles are finite.
  * ECE uses 10 bins: `floor(p * 10)`, with p == 1.0 in the last bin, and empty bins are
    skipped rather than counted as zero.
  * Temperature scaling is `sigmoid(logit(p) / T)` with `logit` clamped to
    [1e-6, 1 - 1e-6], and T is chosen to minimize ECE (with T = 1.0 among the
    candidates, so the fit can never be worse than doing nothing).

The labeled set is built, not sampled: each class holds a small number of distinct
confidences, each repeated, so every number below is reproducible from the file alone.
The search over candidate sets picks the first one that is *overconfident enough* for
calibration to be worth doing — raw Brier and ECE above the thresholds, calibrated
Brier and ECE comfortably below them.

Run: `python3 bench/calibration/compute_reference.py`
"""

import json
import math
import os
import struct

BINS = 10

# The ledger stores a probability in a REAL column and Rust reads it back as `f32`, so
# the reference models that too: computing in `f64` from the JSON decimal would describe
# a system the kernel does not have, and would make an honest implementation look wrong
# by a few times 1e-8. `precision` states the bound the two can actually agree to — the
# metric sums run in f32 before being widened, so the difference is a rounding of the
# sum, not an error in it.
PRECISION = 1e-7


def f32(value):
    """The value as the ledger would have stored it."""
    return struct.unpack("f", struct.pack("f", value))[0]
MIN_TEMPERATURE = 0.05
MAX_TEMPERATURE = 20.0
GRID_POINTS = 2000


def sigmoid(x):
    if x >= 0.0:
        z = math.exp(-x)
        return 1.0 / (1.0 + z)
    z = math.exp(x)
    return z / (1.0 + z)


def logit(p):
    p = min(max(p, 1e-6), 1.0 - 1e-6)
    return math.log(p / (1.0 - p))


def brier(predicted, labels):
    n = len(predicted)
    if n == 0:
        return 0.0
    return sum((p - (1.0 if y else 0.0)) ** 2 for p, y in zip(predicted, labels)) / n


def log_loss(predicted, labels):
    n = len(predicted)
    if n == 0:
        return 0.0
    total = 0.0
    for p, y in zip(predicted, labels):
        q = min(max(p, 1e-9), 1.0 - 1e-9)
        total += -math.log(q if y else 1.0 - q)
    return total / n


def ece(predicted, labels):
    n = len(predicted)
    if n == 0:
        return 0.0
    counts = [0] * BINS
    sums = [0.0] * BINS
    positives = [0] * BINS
    for p, y in zip(predicted, labels):
        index = min(int(p * BINS), BINS - 1)
        counts[index] += 1
        sums[index] += p
        positives[index] += 1 if y else 0
    total = 0.0
    for index in range(BINS):
        if counts[index] == 0:
            continue
        mean = sums[index] / counts[index]
        observed = positives[index] / counts[index]
        total += (counts[index] / n) * abs(observed - mean)
    return total


def temperature_grid():
    grid = [
        MIN_TEMPERATURE * (MAX_TEMPERATURE / MIN_TEMPERATURE) ** (i / (GRID_POINTS - 1))
        for i in range(GRID_POINTS)
    ]
    grid.append(1.0)
    return grid


def fit_temperature(predicted, labels):
    best_t = 1.0
    best = ece(predicted, labels)
    for candidate in temperature_grid():
        scaled = [sigmoid(logit(p) / candidate) for p in predicted]
        value = ece(scaled, labels)
        if value < best - 1e-12:
            best = value
            best_t = candidate
        elif abs(value - best) <= 1e-12 and abs(candidate - 1.0) < abs(best_t - 1.0):
            best_t = candidate
    return best_t, best


def build_set(triples):
    predicted, labels = [], []
    for p, trues, falses in triples:
        predicted.extend([p] * trues)
        predicted.extend([p] * falses)
        labels.extend([True] * trues)
        labels.extend([False] * falses)
    return predicted, labels


# The classes: (confidence, right, wrong). Each is overconfident — the model says more
# than it delivers — which is the state a calibration pass is supposed to fix, and each
# delivers often enough that a *calibrated* probability still carries information. That
# second half is what keeps the gate meaningful: a set whose true frequency is 0.5 has a
# Brier floor of 0.25, and no temperature can reach the 0.20 the gate asks for, so the
# fixture would be asking for something impossible rather than something hard.
CANDIDATES = [
    ("safety", [(0.98, 40, 10)]),
    ("retrieval", [(0.95, 37, 13)]),
]


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    rows = []
    reference = {
        "classes": {},
        "n": 0,
        "baseline_brier": 0.30,
        "bins": BINS,
        "precision": PRECISION,
    }
    for klass, triples in CANDIDATES:
        predicted, labels = build_set(triples)
        predicted = [f32(p) for p in predicted]
        t, ece_after = fit_temperature(predicted, labels)
        scaled = [sigmoid(logit(p) / t) for p in predicted]
        reference["classes"][klass] = {
            "n": len(predicted),
            "temperature": round(t, 9),
            "brier_before": round(brier(predicted, labels), 9),
            "log_loss_before": round(log_loss(predicted, labels), 9),
            "ece_before": round(ece(predicted, labels), 9),
            "brier": round(brier(scaled, labels), 9),
            "log_loss": round(log_loss(scaled, labels), 9),
            "ece": round(ece_after, 9),
        }
        reference["n"] += len(predicted)
        for p, y in zip(predicted, labels):
            rows.append({"class": klass, "predicted": p, "label": y})
    # The whole-set metrics, over every labeled point, with the per-class temperatures
    # applied. This is the pair the gate's `--assert-brier-le` / `--assert-ece-le` read.
    scaled_all, labels_all = [], []
    for klass, triples in CANDIDATES:
        predicted, labels = build_set(triples)
        predicted = [f32(p) for p in predicted]
        t = reference["classes"][klass]["temperature"]
        scaled_all.extend(sigmoid(logit(p) / t) for p in predicted)
        labels_all.extend(labels)
    reference["brier"] = round(brier(scaled_all, labels_all), 9)
    reference["log_loss"] = round(log_loss(scaled_all, labels_all), 9)
    reference["ece"] = round(ece(scaled_all, labels_all), 9)
    reference["brier_before"] = round(
        brier([r["predicted"] for r in rows], [r["label"] for r in rows]), 9
    )
    reference["ece_before"] = round(
        ece([r["predicted"] for r in rows], [r["label"] for r in rows]), 9
    )

    with open(os.path.join(here, "predictions.jsonl"), "w") as handle:
        for row in rows:
            handle.write(json.dumps(row, sort_keys=True) + "\n")
    with open(os.path.join(here, "reference.json"), "w") as handle:
        json.dump(reference, handle, indent=2, sort_keys=True)
        handle.write("\n")

    # The fixture only passes the gate's thresholds for a reason, so the reasons are
    # asserted here: without them a future edit could quietly make the corpus trivial.
    assert reference["brier_before"] > 0.20, reference
    assert reference["ece_before"] > 0.15, reference
    assert reference["brier"] <= 0.20, reference
    assert reference["ece"] <= 0.10, reference
    assert reference["brier"] < reference["brier_before"], reference
    assert reference["brier"] < reference["baseline_brier"], reference


if __name__ == "__main__":
    main()
