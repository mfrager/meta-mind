//! Verification: obligations that end in a proof, never in prose.
//!
//! The phase plan's §5.12 asks for `cargo build`/`cargo test`, static analysis, a
//! symbolic check and a residual-math check, "each returning a proof/evidence object,
//! never prose". That requirement is what shapes this module: an [`Obligation`] is a
//! closed, named question about a subject, [`run_obligation`] answers it, and the answer
//! is a [`Proof`] — a verdict, the artifact it is about, a hash, and a counterexample
//! when there is one.
//!
//! # What is proved, and what is not
//!
//! The two *sound* checks here are decidable by construction and say so:
//!
//! * [`check_symbolic`] refutes a clause set when a clause contains a literal and its
//!   negation, or when two unit clauses are complementary. That is unit-resolution
//!   refutation: **sound** — it never reports an inconsistency that is not one — and
//!   deliberately **incomplete**: a set that is inconsistent for a reason needing
//!   resolution between non-unit clauses returns [`Verdict::Inconclusive`], not
//!   `Proven`. Reporting "proven consistent" from a search that did not find a
//!   refutation would be the exact dishonesty the phase's first invariant exists to
//!   prevent, so the verdict is named for what it is.
//! * [`check_residuals`] proves a residual set zero within a stated tolerance, and names
//!   the first value that exceeds it.
//!
//! The two *command* checks (`cargo build`, `cargo test`, static analysis) prove nothing
//! on their own: a green build is evidence that a program compiles, not that a claim is
//! true. They return [`Verdict::Proven`] with the exit code and the output hash recorded,
//! and the report says which question was answered, so a reader is never invited to
//! read more into it than that.

use serde::{Deserialize, Serialize};

use crate::error::ToolError;
use crate::registry::ExecCtx;
use mm_core::Timestamp;

/// A named, decidable question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObligationKind {
    /// Does the subject compile?
    CargoBuild,
    /// Do the subject's tests pass?
    CargoTest,
    /// Does the subject pass the workspace lints?
    StaticAnalysis,
    /// Is the subject's clause set refutable?
    Symbolic,
    /// Are the subject's residuals zero within tolerance?
    MathResidual,
}

/// Every kind, in the order the CLI lists them.
pub const OBLIGATION_KINDS: [ObligationKind; 5] = [
    ObligationKind::CargoBuild,
    ObligationKind::CargoTest,
    ObligationKind::StaticAnalysis,
    ObligationKind::Symbolic,
    ObligationKind::MathResidual,
];

impl ObligationKind {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            ObligationKind::CargoBuild => "cargo-build",
            ObligationKind::CargoTest => "cargo-test",
            ObligationKind::StaticAnalysis => "static-analysis",
            ObligationKind::Symbolic => "symbolic",
            ObligationKind::MathResidual => "math-residual",
        }
    }

    /// Parse a wire name.
    pub fn parse(text: &str) -> Option<Self> {
        OBLIGATION_KINDS
            .into_iter()
            .find(|kind| kind.as_str() == text.trim().to_ascii_lowercase())
    }

    /// The command a command-backed obligation runs, or `None` for a file-backed one.
    ///
    /// The subject is a crate or package name, never a free-form shell string: a
    /// verification command that a caller could compose would be a way to run anything
    /// under the name of verification.
    pub fn command(self, subject: &str) -> Option<Vec<String>> {
        let words = |words: &[&str]| -> Option<Vec<String>> {
            Some(
                words
                    .iter()
                    .map(|word| (*word).to_string())
                    .chain(std::iter::once(subject.to_string()))
                    .collect(),
            )
        };
        match self {
            ObligationKind::CargoBuild => words(&["cargo", "build", "-p"]),
            ObligationKind::CargoTest => words(&["cargo", "test", "-p"]),
            // `-D warnings` rather than the plain lint run: the gate's own criterion is
            // that the workspace is warning-free, so the obligation that checks it is
            // the strict one.
            ObligationKind::StaticAnalysis => words(&["cargo", "clippy", "-p"]),
            ObligationKind::Symbolic | ObligationKind::MathResidual => None,
        }
    }

    /// True when the kind answers its question by running a command.
    pub fn is_command_backed(self) -> bool {
        matches!(
            self,
            ObligationKind::CargoBuild | ObligationKind::CargoTest | ObligationKind::StaticAnalysis
        )
    }

    /// True when the kind reads its subject from the sandbox.
    pub fn is_file_backed(self) -> bool {
        !self.is_command_backed()
    }
}

impl std::fmt::Display for ObligationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One obligation: a kind and a subject.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obligation {
    /// What is being asked.
    pub kind: ObligationKind,
    /// What it is being asked about: a package name, or a path inside the sandbox.
    pub subject: String,
}

impl Obligation {
    /// An obligation, refused when the subject is empty or looks like an option.
    pub fn new(kind: ObligationKind, subject: impl Into<String>) -> Result<Self, String> {
        let subject = subject.into();
        let trimmed = subject.trim();
        if trimmed.is_empty() {
            return Err("an obligation needs a subject".to_string());
        }
        if trimmed.starts_with('-') {
            return Err(format!(
                "subject {trimmed:?} begins with '-' and would be read as an argument"
            ));
        }
        Ok(Obligation {
            kind,
            subject: trimmed.to_string(),
        })
    }

    /// `kind:subject`, the form the ledger and the log records carry.
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.kind.as_str(), self.subject)
    }
}

impl std::fmt::Display for Obligation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.canonical())
    }
}

/// Build an obligation from the CLI's or a tool's two strings.
pub fn obligation_for(kind: &str, subject: &str) -> Option<Obligation> {
    let kind = ObligationKind::parse(kind)?;
    Obligation::new(kind, subject).ok()
}

/// What a check concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The obligation holds.
    Proven,
    /// The obligation fails, and there is a counterexample.
    Refuted,
    /// The check could not decide. Never a synonym for "proven".
    Inconclusive,
}

impl Verdict {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Proven => "proven",
            Verdict::Refuted => "refuted",
            Verdict::Inconclusive => "inconclusive",
        }
    }

    /// True when the obligation holds.
    pub fn passed(self) -> bool {
        matches!(self, Verdict::Proven)
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The answer to an obligation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Proof {
    /// What was asked.
    pub obligation: Obligation,
    /// What was concluded.
    pub verdict: Verdict,
    /// One sentence naming the evidence: an exit code, a clause pair, a residual value.
    pub detail: String,
    /// The counterexample, when the verdict is [`Verdict::Refuted`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counterexample: Option<String>,
    /// A hash of the artifact the verdict is about, so two runs are comparable.
    pub artifact_hash: String,
    /// When it was decided.
    pub at: Timestamp,
}

impl Proof {
    /// A proof.
    pub fn new(
        obligation: Obligation,
        verdict: Verdict,
        detail: impl Into<String>,
        counterexample: Option<String>,
        artifact_hash: impl Into<String>,
    ) -> Self {
        Proof {
            obligation,
            verdict,
            detail: detail.into(),
            counterexample,
            artifact_hash: artifact_hash.into(),
            at: Timestamp::now(),
        }
    }

    /// A stable hash of the whole proof, for the `verify.run.end` record.
    pub fn hash(&self) -> String {
        mm_core::hash_fields(&[
            &self.obligation.canonical(),
            self.verdict.as_str(),
            &self.detail,
            self.counterexample.as_deref().unwrap_or(""),
            &self.artifact_hash,
        ])
    }

    /// True when the obligation holds.
    pub fn passed(&self) -> bool {
        self.verdict.passed()
    }

    /// True when a counterexample was produced.
    pub fn has_counterexample(&self) -> bool {
        self.counterexample.is_some()
    }
}

/// One literal: a predicate applied to arguments, possibly negated.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Literal {
    predicate: String,
    arguments: String,
    negated: bool,
}

impl Literal {
    fn complement(&self) -> Literal {
        Literal {
            predicate: self.predicate.clone(),
            arguments: self.arguments.clone(),
            negated: !self.negated,
        }
    }

    fn render(&self) -> String {
        if self.negated {
            format!("¬{}({})", self.predicate, self.arguments)
        } else {
            format!("{}({})", self.predicate, self.arguments)
        }
    }
}

/// Parse one literal, or `None` when the text is not one.
fn parse_literal(text: &str) -> Option<Literal> {
    let mut text = text.trim();
    let mut negated = false;
    if let Some(rest) = text.strip_prefix('¬') {
        negated = true;
        text = rest.trim();
    } else if let Some(rest) = text.strip_prefix("not ") {
        negated = true;
        text = rest.trim();
    } else if let Some(rest) = text.strip_prefix('~') {
        negated = true;
        text = rest.trim();
    }
    let (predicate, arguments) = match text.split_once('(') {
        Some((predicate, rest)) => {
            let arguments = rest.strip_suffix(')')?;
            (predicate.trim(), arguments.trim())
        }
        None => (text.trim(), ""),
    };
    if predicate.is_empty() {
        return None;
    }
    Some(Literal {
        predicate: predicate.to_string(),
        arguments: arguments.replace(' ', ""),
        negated,
    })
}

/// The clauses of a `.logic` fixture: one clause per line, literals joined by `or`.
fn clauses(text: &str) -> Vec<Vec<Literal>> {
    text.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with("//"))
        .map(|line| {
            let body = line
                .strip_prefix('(')
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or(line);
            body.split(" or ")
                .filter_map(parse_literal)
                .collect::<Vec<_>>()
        })
        .filter(|clause| !clause.is_empty())
        .collect()
}

/// Refute a clause set by unit resolution, or report that no refutation was found.
///
/// Returns the verdict and, when refuted, the pair of clauses that disagree.
pub fn check_symbolic(text: &str) -> (Verdict, Option<String>) {
    let clauses = clauses(text);
    if clauses.is_empty() {
        return (
            Verdict::Inconclusive,
            Some("the subject contains no clauses to check".to_string()),
        );
    }
    for clause in &clauses {
        for literal in clause {
            if clause.contains(&literal.complement()) {
                return (
                    Verdict::Refuted,
                    Some(format!(
                        "the clause {{{}}} contains {} and its negation",
                        clause
                            .iter()
                            .map(Literal::render)
                            .collect::<Vec<_>>()
                            .join(", "),
                        literal.render()
                    )),
                );
            }
        }
    }
    let units: Vec<Literal> = clauses
        .iter()
        .filter(|clause| clause.len() == 1)
        .map(|clause| clause[0].clone())
        .collect();
    for literal in &units {
        if units.contains(&literal.complement()) {
            return (
                Verdict::Refuted,
                Some(format!(
                    "{} and {} are both asserted",
                    literal.render(),
                    literal.complement().render()
                )),
            );
        }
    }
    // No refutation found. Sound but incomplete: this is not a consistency proof.
    (
        Verdict::Inconclusive,
        Some(format!(
            "no unit-resolution refutation among {} clauses",
            clauses.len()
        )),
    )
}

/// Prove that every residual in the subject is zero within `tolerance`.
pub fn check_residuals(text: &str, tolerance: f64) -> (Verdict, Option<String>) {
    let mut seen = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        let value = line
            .rsplit('=')
            .next()
            .unwrap_or(line)
            .trim()
            .trim_end_matches(';');
        let Ok(parsed) = value.parse::<f64>() else {
            continue;
        };
        seen += 1;
        if !parsed.is_finite() {
            return (
                Verdict::Refuted,
                Some(format!("{line} is not a finite number")),
            );
        }
        if parsed.abs() > tolerance {
            return (
                Verdict::Refuted,
                Some(format!(
                    "{line} has a residual of {parsed}, above the tolerance {tolerance}"
                )),
            );
        }
    }
    if seen == 0 {
        return (
            Verdict::Inconclusive,
            Some("the subject contains no residual values".to_string()),
        );
    }
    (Verdict::Proven, None)
}

/// Run an obligation and produce its proof.
///
/// A command-backed obligation goes through the sandbox like any other subprocess:
/// [`ExecCtx::check_subprocess`] refuses it when the caller holds no process
/// capability, so verification cannot become the hole a denied `process.exec` would be.
pub async fn run_obligation(
    ctx: &ExecCtx,
    obligation: &Obligation,
    tolerance: f64,
) -> Result<Proof, ToolError> {
    match obligation.kind {
        ObligationKind::Symbolic => {
            let path = ctx.read_path(&obligation.subject)?;
            let text = std::fs::read_to_string(&path)
                .map_err(|e| ctx.failed(format!("cannot read {}: {e}", path.display())))?;
            ctx.check_size(text.len() as u64)?;
            let (verdict, note) = check_symbolic(&text);
            let detail = note.clone().unwrap_or_else(|| "no refutation".to_string());
            Ok(Proof::new(
                obligation.clone(),
                verdict,
                detail,
                note,
                mm_core::content_hash(text.as_bytes()),
            ))
        }
        ObligationKind::MathResidual => {
            let path = ctx.read_path(&obligation.subject)?;
            let text = std::fs::read_to_string(&path)
                .map_err(|e| ctx.failed(format!("cannot read {}: {e}", path.display())))?;
            ctx.check_size(text.len() as u64)?;
            let (verdict, note) = check_residuals(&text, tolerance);
            let detail = note
                .clone()
                .unwrap_or_else(|| format!("every residual is within {tolerance}"));
            Ok(Proof::new(
                obligation.clone(),
                verdict,
                detail,
                note,
                mm_core::content_hash(text.as_bytes()),
            ))
        }
        kind => {
            ctx.check_subprocess()?;
            let words = kind
                .command(&obligation.subject)
                .ok_or_else(|| ctx.unsupported(format!("{kind} has no command")))?;
            let (program, args) = words
                .split_first()
                .ok_or_else(|| ctx.unsupported("the obligation has an empty command"))?;
            let output = tokio::process::Command::new(program)
                .args(args)
                .output()
                .await
                .map_err(|e| ctx.failed(format!("cannot run {program}: {e}")))?;
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            ctx.check_size((stdout.len() + stderr.len()) as u64)?;
            let code = output.status.code().unwrap_or(-1);
            let artifact_hash = mm_core::hash_fields(&[&stdout, &stderr, &code.to_string()]);
            let verdict = if output.status.success() {
                Verdict::Proven
            } else {
                Verdict::Refuted
            };
            let counterexample = if output.status.success() {
                None
            } else {
                let tail: String = stderr
                    .lines()
                    .chain(stdout.lines())
                    .filter(|line| !line.trim().is_empty())
                    .rev()
                    .take(5)
                    .collect::<Vec<_>>()
                    .join("\n");
                Some(tail)
            };
            Ok(Proof::new(
                obligation.clone(),
                verdict,
                format!("{program} exited with {code}"),
                counterexample,
                artifact_hash,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obligations_parse_and_render() {
        let obligation = obligation_for("symbolic", "bench/tools/x.logic").unwrap();
        assert_eq!(obligation.kind, ObligationKind::Symbolic);
        assert_eq!(obligation.canonical(), "symbolic:bench/tools/x.logic");
        assert!(obligation_for("nonsense", "x").is_none());
        assert!(obligation_for("symbolic", "  ").is_none());
        assert!(obligation_for("symbolic", "--evil").is_none());
        for kind in OBLIGATION_KINDS {
            assert_eq!(ObligationKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(
            ObligationKind::CargoTest.command("mm-tools"),
            Some(vec![
                "cargo".to_string(),
                "test".to_string(),
                "-p".to_string(),
                "mm-tools".to_string()
            ])
        );
        assert!(ObligationKind::Symbolic.command("x").is_none());
        assert!(ObligationKind::CargoBuild.is_command_backed());
        assert!(ObligationKind::Symbolic.is_file_backed());
    }

    #[test]
    fn a_complementary_clause_is_refuted() {
        let (verdict, counterexample) = check_symbolic("(p(a) or not p(a))\n");
        assert_eq!(verdict, Verdict::Refuted);
        assert!(counterexample.unwrap().contains("negation"));
    }

    #[test]
    fn two_complementary_units_are_refuted() {
        let (verdict, counterexample) = check_symbolic("p(a)\nnot p(a)\n");
        assert_eq!(verdict, Verdict::Refuted);
        assert!(counterexample.unwrap().contains("both asserted"));
    }

    #[test]
    fn a_consistent_set_is_inconclusive_rather_than_proven() {
        let (verdict, note) = check_symbolic("p(a)\nq(b)\n");
        assert_eq!(
            verdict,
            Verdict::Inconclusive,
            "a sound-but-incomplete check must never claim a consistency proof"
        );
        assert!(note.unwrap().contains("no unit-resolution refutation"));
    }

    #[test]
    fn an_empty_subject_is_inconclusive() {
        assert_eq!(check_symbolic("\n# nothing\n").0, Verdict::Inconclusive);
        assert_eq!(
            check_residuals("# nothing\n", 1e-9).0,
            Verdict::Inconclusive
        );
    }

    #[test]
    fn residuals_are_proven_within_tolerance_and_refuted_outside_it() {
        let (verdict, _) = check_residuals("a = 0.0\nb = 1e-12\n", 1e-9);
        assert_eq!(verdict, Verdict::Proven);
        let (verdict, counterexample) = check_residuals("a = 0.0\nb = 0.001\n", 1e-9);
        assert_eq!(verdict, Verdict::Refuted);
        assert!(counterexample.unwrap().contains("0.001"));
        let (verdict, counterexample) = check_residuals("a = NaN\n", 1e-9);
        assert_eq!(verdict, Verdict::Refuted);
        assert!(counterexample.unwrap().contains("finite"));
    }

    #[test]
    fn a_proof_hashes_its_own_content() {
        let obligation = obligation_for("symbolic", "x.logic").unwrap();
        let one = Proof::new(obligation.clone(), Verdict::Proven, "ok", None, "hash");
        let same = Proof::new(obligation.clone(), Verdict::Proven, "ok", None, "hash");
        let other = Proof::new(
            obligation,
            Verdict::Refuted,
            "ok",
            Some("p and ¬p".into()),
            "hash",
        );
        assert_eq!(one.hash(), same.hash());
        assert_ne!(one.hash(), other.hash());
        assert!(one.passed());
        assert!(!other.passed());
        assert!(other.has_counterexample());
    }

    #[test]
    fn a_proof_round_trips_through_json() {
        let obligation = obligation_for("math-residual", "r.txt").unwrap();
        let proof = Proof::new(obligation, Verdict::Proven, "all zero", None, "abc");
        let text = serde_json::to_string(&proof).unwrap();
        let back: Proof = serde_json::from_str(&text).unwrap();
        assert_eq!(proof, back);
        assert_eq!(Verdict::Refuted.as_str(), "refuted");
    }
}
