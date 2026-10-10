//! Uncertainty signals: how much the samples disagree about what a claim means.
//!
//! A model that answers the same question five times and says five different
//! things is telling you something the first answer did not: it does not know.
//! Phase 9 needs that number, because the firewall's `VERIFY_FIRST` and the
//! decision cores' abstention are both driven by it, and a firewall driven by a
//! signal that was quietly fabricated is worse than one with no signal at all.
//!
//! Three scorers, one trait, one meaning: a value in `[0,1]` where `0` is "every
//! sample says the same thing" and `1` is "the samples are maximally spread".
//!
//! * [`SelfConsistencyScorer`] — clusters samples whose normalized text is
//!   identical. No model, no judge, fully deterministic. This is the signal that
//!   always exists, and the one the gate uses.
//! * [`SemanticEntropyScorer`] — the same entropy, but clustered by *meaning*
//!   through an [`EquivalenceJudge`]. With [`LexicalJudge`] it degenerates exactly
//!   to self-consistency; with a model-backed judge it separates "seven percent"
//!   from "7%" — a spread self-consistency over-counts as disagreement.
//! * [`KernelEntropyScorer`] — a kernel density estimate over the distinct
//!   samples: the entropy of the density-weighted masses. Where the first two
//!   answer "how many meanings are there", this one answers "how concentrated is
//!   the distribution of meaning", and it is the one that does not need a discrete
//!   clustering decision.
//!
//! Determinism is a hard requirement, not a nicety. A scorer whose value changes
//! between two runs over the same samples makes the firewall non-replayable, and
//! replayability is the property every other phase leans on. So: clustering is
//! greedy in first-appearance order, no hash-ordered iteration reaches the output,
//! and the kernel is lexical rather than embedding-based — an embedding provider is
//! nondeterministic across versions, which would silently move a threshold that
//! gates an action.
//!
//! Two readings of the number are worth stating because the code does not show
//! them. First, entropy is normalized by `ln(n_samples)` rather than by
//! `ln(n_clusters)`, so the same cluster structure scores lower when it is drawn
//! from more samples — three meanings in three samples is maximum spread; the same
//! two meanings in fifty samples is not. Second, a sample list of zero or one is
//! `0.0` rather than `1.0`: with nothing to disagree with, there is no measured
//! disagreement, and a constant `1.0` would make every one-sample pass escalate.

use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Which uncertainty signal a scorer computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UncertaintyKind {
    /// Entropy over clusters of *meaning*, clustered by a judge.
    SemanticEntropy,
    /// Entropy of a kernel density estimate over the samples.
    KernelEntropy,
    /// Entropy over clusters of normalized text; no judge.
    SelfConsistency,
}

/// Every kind, in wire order.
pub const UNCERTAINTY_KINDS: [UncertaintyKind; 3] = [
    UncertaintyKind::SemanticEntropy,
    UncertaintyKind::KernelEntropy,
    UncertaintyKind::SelfConsistency,
];

impl UncertaintyKind {
    /// The stable wire name, which is also the `uncertainty.score` `kind` field.
    pub fn as_str(self) -> &'static str {
        match self {
            UncertaintyKind::SemanticEntropy => "semantic_entropy",
            UncertaintyKind::KernelEntropy => "kernel_entropy",
            UncertaintyKind::SelfConsistency => "self_consistency",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        UNCERTAINTY_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == text)
    }
}

impl std::fmt::Display for UncertaintyKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The punctuation stripped from the ends of a sample before it is compared.
///
/// Only the ends: internal punctuation carries meaning ("7%", "p95"), and folding
/// it away would merge samples that genuinely differ.
const EDGE_PUNCTUATION: &[char] = &[
    '.', ',', ';', ':', '!', '?', '"', '\'', '(', ')', '[', ']', '{', '}', '-', '\u{2013}',
    '\u{2014}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}',
];

/// The canonical form two samples are compared by when no judge is available:
/// lowercased, whitespace-collapsed, with surrounding punctuation trimmed.
///
/// This is deliberately *not* a stemmer or a synonym map. Two samples that differ
/// only in inflection stay distinct here, and the semantic scorer is where that
/// difference is meant to be resolved — by something that can actually read.
pub fn normalize_text(text: &str) -> String {
    let lowered = text.to_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| EDGE_PUNCTUATION.contains(&c))
        .trim()
        .to_string()
}

/// The normalized Shannon entropy of a cluster-mass vector, in `[0,1]`.
///
/// `masses` are unnormalized weights (cluster sizes, or density estimates);
/// the denominator is `ln(n)`, the entropy of a uniform distribution over `n`
/// samples. `0.0` when there is one sample or fewer, when no mass is positive, or
/// when only one cluster is non-empty — in each case there is no spread to
/// measure, and returning anything else would invent one.
pub fn cluster_entropy(masses: &[f32], n: usize) -> f32 {
    if n <= 1 {
        return 0.0;
    }
    let non_empty = masses.iter().filter(|mass| **mass > 0.0).count();
    if non_empty <= 1 {
        return 0.0;
    }
    let total: f32 = masses.iter().filter(|mass| **mass > 0.0).sum();
    if !total.is_finite() || total <= 0.0 {
        return 0.0;
    }
    let mut entropy = 0.0f32;
    for mass in masses {
        if *mass <= 0.0 {
            continue;
        }
        let p = mass / total;
        entropy -= p * p.ln();
    }
    let denominator = (n as f32).ln();
    if denominator <= 0.0 {
        return 0.0;
    }
    (entropy / denominator).clamp(0.0, 1.0)
}

/// Decides whether two samples mean the same thing.
///
/// A trait rather than a function pointer because the model-backed judge carries
/// state (a client, a schema registry) and because a caller must be able to see
/// which judge produced a number. Implementations must be deterministic for the
/// same input: a judge that is not makes every threshold downstream unverifiable.
pub trait EquivalenceJudge: Send + Sync {
    /// True when the two samples express the same claim.
    fn equivalent(&self, a: &str, b: &str) -> Result<bool>;
}

/// Equivalence by [`normalize_text`] equality: no model, no network.
#[derive(Clone, Copy, Debug, Default)]
pub struct LexicalJudge;

impl EquivalenceJudge for LexicalJudge {
    fn equivalent(&self, a: &str, b: &str) -> Result<bool> {
        Ok(normalize_text(a) == normalize_text(b))
    }
}

/// The one interface every uncertainty signal implements.
///
/// Object-safe, so a caller can hold a `&dyn UncertaintyScorer` chosen at run time
/// and log which one it used — the plan's UQLM-style scorer registry, in miniature.
#[async_trait]
pub trait UncertaintyScorer: Send + Sync {
    /// Which signal this scorer computes.
    fn kind(&self) -> UncertaintyKind;

    /// Score the spread of `samples` about `claim`, in `[0,1]`.
    ///
    /// `claim` is the text the samples are answers to and may be unused by a
    /// scorer that only compares samples to each other; it is in the signature
    /// because a scorer that wants it (one that measures how far the samples drift
    /// from the most likely answer) must not need a second interface.
    async fn score(&self, claim: &str, samples: &[String]) -> Result<f32>;
}

/// Cluster `samples` greedily by an equivalence predicate, in first-appearance
/// order, as `(representative, size)` pairs.
///
/// Greedy and order-preserving on purpose: a clustering that depended on the
/// samples' order of presentation would make the score order-dependent, and an
/// order-dependent score cannot be replayed.
fn cluster_by<F>(samples: &[String], mut equivalent: F) -> Result<Vec<(String, usize)>>
where
    F: FnMut(&str, &str) -> Result<bool>,
{
    let mut clusters: Vec<(String, usize)> = Vec::new();
    for sample in samples {
        let mut placed = false;
        for (representative, size) in clusters.iter_mut() {
            if equivalent(representative, sample)? {
                *size += 1;
                placed = true;
                break;
            }
        }
        if !placed {
            clusters.push((sample.clone(), 1));
        }
    }
    Ok(clusters)
}

/// The masses of a clustering, as f32, ready for [`cluster_entropy`].
fn masses_of(clusters: &[(String, usize)]) -> Vec<f32> {
    clusters.iter().map(|(_, size)| *size as f32).collect()
}

/// Entropy over clusters of normalized text, with no judge and no model.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelfConsistencyScorer;

impl SelfConsistencyScorer {
    /// A scorer over normalized-text clusters.
    pub fn new() -> Self {
        SelfConsistencyScorer
    }
}

#[async_trait]
impl UncertaintyScorer for SelfConsistencyScorer {
    fn kind(&self) -> UncertaintyKind {
        UncertaintyKind::SelfConsistency
    }

    async fn score(&self, _claim: &str, samples: &[String]) -> Result<f32> {
        if samples.len() <= 1 {
            return Ok(0.0);
        }
        let clusters = cluster_by(samples, |a, b| Ok(normalize_text(a) == normalize_text(b)))?;
        Ok(cluster_entropy(&masses_of(&clusters), samples.len()))
    }
}

/// Entropy over clusters of *meaning*, clustered through an [`EquivalenceJudge`].
///
/// With [`LexicalJudge`] this is exactly [`SelfConsistencyScorer`], which is the
/// point: the semantic scorer is the same measurement with a better notion of
/// "the same", so a caller can swap the judge without changing what the number
/// means.
#[derive(Clone, Debug)]
pub struct SemanticEntropyScorer<J: EquivalenceJudge> {
    judge: J,
}

impl<J: EquivalenceJudge> SemanticEntropyScorer<J> {
    /// A scorer over the given judge.
    pub fn new(judge: J) -> Self {
        SemanticEntropyScorer { judge }
    }

    /// The judge this scorer reads equivalence through.
    pub fn judge(&self) -> &J {
        &self.judge
    }
}

impl Default for SemanticEntropyScorer<LexicalJudge> {
    fn default() -> Self {
        SemanticEntropyScorer::new(LexicalJudge)
    }
}

#[async_trait]
impl<J: EquivalenceJudge> UncertaintyScorer for SemanticEntropyScorer<J> {
    fn kind(&self) -> UncertaintyKind {
        UncertaintyKind::SemanticEntropy
    }

    async fn score(&self, _claim: &str, samples: &[String]) -> Result<f32> {
        if samples.len() <= 1 {
            return Ok(0.0);
        }
        let clusters = cluster_by(samples, |a, b| self.judge.equivalent(a, b))?;
        Ok(cluster_entropy(&masses_of(&clusters), samples.len()))
    }
}

/// A kernel density estimate of the samples' entropy, over a lexical kernel.
///
/// The kernel is `k(a, b) = exp(-(1 - jaccard(a, b)) / bandwidth)`: `1` for
/// identical token sets, decaying as the sets diverge. Each distinct sample's
/// *mass* is its share of the samples times the average similarity it enjoys — so
/// a value repeated many times, or one that many other samples resemble, carries
/// the weight of a mode. The reported number is the normalized entropy of those
/// masses.
///
/// Two consequences worth knowing. It is a *density* estimate, so it reads a tight
/// cluster of paraphrases as agreement where `SemanticEntropyScorer` with a
/// discrete judge might read it as three meanings. And it is lexical: `bandwidth`
/// is the only knob, a small bandwidth makes the kernel nearly an indicator
/// function of the token set (approaching a facet count, and it will call a
/// one-word paraphrase a different meaning), while a large one smooths everything
/// together.
#[derive(Clone, Copy, Debug)]
pub struct KernelEntropyScorer {
    /// The kernel's smoothing width. Clamped to a small positive minimum.
    pub bandwidth: f32,
}

/// The smallest admissible bandwidth: smaller would make the kernel an indicator
/// function on token sets and the estimate degenerate to a facet count.
const MIN_BANDWIDTH: f32 = 1.0e-3;

impl KernelEntropyScorer {
    /// A scorer with the given bandwidth, clamped into range.
    pub fn new(bandwidth: f32) -> Self {
        let bandwidth = if bandwidth.is_finite() && bandwidth > MIN_BANDWIDTH {
            bandwidth
        } else {
            MIN_BANDWIDTH
        };
        KernelEntropyScorer { bandwidth }
    }
}

impl Default for KernelEntropyScorer {
    fn default() -> Self {
        KernelEntropyScorer::new(0.25)
    }
}

#[async_trait]
impl UncertaintyScorer for KernelEntropyScorer {
    fn kind(&self) -> UncertaintyKind {
        UncertaintyKind::KernelEntropy
    }

    async fn score(&self, _claim: &str, samples: &[String]) -> Result<f32> {
        let n = samples.len();
        if n <= 1 {
            return Ok(0.0);
        }
        // Distinct normalized forms, in first-appearance order, with their counts.
        let mut distinct: Vec<String> = Vec::new();
        let mut counts: Vec<usize> = Vec::new();
        for sample in samples {
            let normalized = normalize_text(sample);
            match distinct.iter().position(|value| value == &normalized) {
                Some(index) => counts[index] += 1,
                None => {
                    distinct.push(normalized);
                    counts.push(1);
                }
            }
        }
        if distinct.len() <= 1 {
            return Ok(0.0);
        }

        let weights: Vec<f32> = counts
            .iter()
            .map(|count| *count as f32 / n as f32)
            .collect();
        let token_sets: Vec<BTreeSet<String>> =
            distinct.iter().map(|value| tokens(value)).collect();

        let mut masses = Vec::with_capacity(distinct.len());
        for (index, row) in token_sets.iter().enumerate() {
            let density: f32 = token_sets
                .iter()
                .zip(weights.iter())
                .map(|(other, weight)| {
                    let similarity = token_jaccard(row, other);
                    weight * (-(1.0 - similarity) / self.bandwidth).exp()
                })
                .sum();
            masses.push(weights[index] * density);
        }
        Ok(cluster_entropy(&masses, n))
    }
}

/// The content tokens of a text: alphanumeric runs, lowercased.
fn tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_lowercase())
        .collect()
}

/// The Jaccard similarity of two token sets, in `[0,1]`. Two empty sets are
/// identical, not disjoint.
fn token_jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let union = a.union(b).count();
    if union == 0 {
        return 1.0;
    }
    a.intersection(b).count() as f32 / union as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime")
            .block_on(future)
    }

    #[test]
    fn kinds_round_trip_and_reject_unknown_names() {
        for kind in UNCERTAINTY_KINDS {
            assert_eq!(UncertaintyKind::parse(kind.as_str()), Some(kind));
            assert_eq!(
                UncertaintyKind::parse(&kind.as_str().to_uppercase()),
                Some(kind)
            );
        }
        assert_eq!(UncertaintyKind::parse("vibes"), None);
    }

    #[test]
    fn normalize_text_folds_case_and_space_but_not_meaning() {
        assert_eq!(normalize_text("  The  Server  is UP. "), "the server is up");
        assert_eq!(normalize_text("\"seven percent\""), "seven percent");
        assert_ne!(normalize_text("7"), normalize_text("seven"));
        assert_ne!(normalize_text("p95"), normalize_text("p99"));
    }

    #[test]
    fn cluster_entropy_bounds_are_exact() {
        assert_eq!(cluster_entropy(&[5.0], 5), 0.0);
        assert_eq!(cluster_entropy(&[], 0), 0.0);
        assert_eq!(cluster_entropy(&[1.0], 1), 0.0);
        assert_eq!(cluster_entropy(&[1.0, 1.0], 2), 1.0);
        assert_eq!(cluster_entropy(&[2.0, 0.0, 0.0], 2), 0.0);
        // Normalized by ln(n_samples), not ln(n_clusters): the same two-way split
        // reads lower when it is drawn from more samples.
        let split = cluster_entropy(&[3.0, 3.0], 6);
        assert!((split - (2f32.ln() / 6f32.ln())).abs() < 1e-6, "{split}");
        assert!(split > 0.0 && split < 1.0, "{split}");
        // A skewed pair is strictly between the extremes.
        let skewed = cluster_entropy(&[1.0, 3.0], 4);
        assert!(skewed > 0.0 && skewed < 1.0, "{skewed}");
    }

    #[test]
    fn identical_samples_score_zero_for_every_scorer() {
        let identical = samples(&[
            "the server is up",
            "The server is up!",
            "  the  server is up  ",
        ]);
        let self_consistency = SelfConsistencyScorer::new();
        assert_eq!(
            block_on(self_consistency.score("up?", &identical)).unwrap(),
            0.0
        );

        let lexical: SemanticEntropyScorer<LexicalJudge> = SemanticEntropyScorer::default();
        assert_eq!(block_on(lexical.score("up?", &identical)).unwrap(), 0.0);

        let kernel = KernelEntropyScorer::default();
        assert_eq!(block_on(kernel.score("up?", &identical)).unwrap(), 0.0);
    }

    #[test]
    fn distinct_samples_score_at_the_maximum() {
        let distinct = samples(&[
            "the server is up",
            "the database is corrupt",
            "the network is saturated",
        ]);
        let self_consistency = SelfConsistencyScorer::new();
        let score = block_on(self_consistency.score("state?", &distinct)).unwrap();
        assert!((score - 1.0).abs() < 1e-6, "{score}");

        let lexical: SemanticEntropyScorer<LexicalJudge> = SemanticEntropyScorer::default();
        let score = block_on(lexical.score("state?", &distinct)).unwrap();
        assert!((score - 1.0).abs() < 1e-6, "{score}");

        // The kernel estimate is density-based, so it sits just below the ceiling
        // for a spread-out sample set rather than exactly at it.
        let kernel = KernelEntropyScorer::default();
        let score = block_on(kernel.score("state?", &distinct)).unwrap();
        assert!(score > 0.5 && score <= 1.0, "{score}");
    }

    #[test]
    fn two_meanings_in_five_samples_scores_between_the_extremes() {
        let mixed = samples(&[
            "roll back the release",
            "roll back the release",
            "roll back the release",
            "fail over to the replica",
            "fail over to the replica",
        ]);
        let self_consistency = SelfConsistencyScorer::new();
        let score = block_on(self_consistency.score("mitigation?", &mixed)).unwrap();
        assert!(score > 0.0 && score < 1.0, "{score}");

        let kernel = KernelEntropyScorer::default();
        let score = block_on(kernel.score("mitigation?", &mixed)).unwrap();
        assert!(score > 0.0 && score < 1.0, "{score}");
    }

    #[test]
    fn the_self_consistency_scorer_returns_zero_for_an_empty_sample_list() {
        let scorer = SelfConsistencyScorer::new();
        assert_eq!(block_on(scorer.score("anything?", &[])).unwrap(), 0.0);
        assert_eq!(
            block_on(scorer.score("anything?", &samples(&["only one"]))).unwrap(),
            0.0
        );
    }

    #[test]
    fn every_scorer_is_deterministic_on_fixed_samples() {
        let mixed = samples(&[
            "seven percent",
            "7%",
            "seven percent",
            "the latency is fine",
        ]);
        let scorers: Vec<Box<dyn UncertaintyScorer>> = vec![
            Box::new(SelfConsistencyScorer::new()),
            Box::new(SemanticEntropyScorer::<LexicalJudge>::default()),
            Box::new(KernelEntropyScorer::default()),
        ];
        for scorer in &scorers {
            let first = block_on(scorer.score("how bad?", &mixed)).unwrap();
            let second = block_on(scorer.score("how bad?", &mixed)).unwrap();
            assert_eq!(first.to_bits(), second.to_bits(), "{}", scorer.kind());
            assert!((0.0..=1.0).contains(&first), "{}", scorer.kind());
        }
    }

    #[test]
    fn a_semantic_judge_can_separate_wording_from_meaning() {
        /// A stand-in for a model-backed judge: it treats a numeral and its word
        /// as equivalent, which `LexicalJudge` cannot.
        struct PercentJudge;
        impl EquivalenceJudge for PercentJudge {
            fn equivalent(&self, a: &str, b: &str) -> Result<bool> {
                let fold = |text: &str| normalize_text(text).replace("7%", "seven percent");
                Ok(fold(a) == fold(b))
            }
        }

        let mixed = samples(&["7%", "seven percent", "seven percent"]);
        let lexical: SemanticEntropyScorer<LexicalJudge> = SemanticEntropyScorer::default();
        let lexical_score = block_on(lexical.score("how many?", &mixed)).unwrap();
        assert!(lexical_score > 0.0, "{lexical_score}");

        let semantic = SemanticEntropyScorer::new(PercentJudge);
        let semantic_score = block_on(semantic.score("how many?", &mixed)).unwrap();
        assert_eq!(semantic_score, 0.0);
        assert!(semantic_score < lexical_score);
    }

    #[test]
    fn the_kernel_bandwidth_is_clamped_and_the_estimate_stays_bounded() {
        let paraphrases = samples(&[
            "roll back the release",
            "roll back the release now",
            "roll back the release immediately",
        ]);
        for bandwidth in [0.05_f32, 0.25, 5.0] {
            let scorer = KernelEntropyScorer::new(bandwidth);
            let score = block_on(scorer.score("mitigation?", &paraphrases)).unwrap();
            assert!((0.0..=1.0).contains(&score), "{bandwidth} -> {score}");
            let again =
                block_on(KernelEntropyScorer::new(bandwidth).score("mitigation?", &paraphrases))
                    .unwrap();
            assert_eq!(again.to_bits(), score.to_bits(), "{bandwidth}");
        }

        // A bandwidth at or below the minimum is clamped, not trusted.
        assert_eq!(KernelEntropyScorer::new(0.0).bandwidth, MIN_BANDWIDTH);
        assert_eq!(KernelEntropyScorer::new(-1.0).bandwidth, MIN_BANDWIDTH);
        assert_eq!(KernelEntropyScorer::new(f32::NAN).bandwidth, MIN_BANDWIDTH);
        assert_eq!(KernelEntropyScorer::default().bandwidth, 0.25);
    }

    #[test]
    fn token_jaccard_is_a_bounded_metric_on_token_sets() {
        let a = tokens("the server is up");
        assert_eq!(token_jaccard(&a, &a), 1.0);
        assert_eq!(token_jaccard(&a, &tokens("the server is up")), 1.0);
        assert_eq!(token_jaccard(&tokens(""), &tokens("")), 1.0);
        assert_eq!(token_jaccard(&a, &tokens("")), 0.0);
        let overlap = token_jaccard(&a, &tokens("the server is down"));
        assert!(overlap > 0.0 && overlap < 1.0, "{overlap}");
    }

    #[test]
    fn the_trait_is_object_safe() {
        let scorer: &dyn UncertaintyScorer = &SelfConsistencyScorer::new();
        assert_eq!(scorer.kind(), UncertaintyKind::SelfConsistency);
    }
}
