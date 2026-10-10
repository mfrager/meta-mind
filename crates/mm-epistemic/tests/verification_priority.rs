//! `verification_priority.rs` — the priority arithmetic matches the committed
//! golden file exactly (within `1e-9`), so a change to the formula is a change to
//! a checked artefact.

use mm_core::{Config, Ulid};
use mm_epistemic::{verification_priority, Assumption, Proposition, RiskLevel};

fn assumption(risk: RiskLevel, cost: f64, dependence: f64) -> Assumption {
    Assumption::new(
        Ulid::from_parts(1_700_000_000_000, 1),
        Proposition::literal("https://x/s", "https://x/p", "v").unwrap(),
        0.5,
        risk,
        cost,
        dependence,
    )
    .unwrap()
}

#[test]
fn the_priority_arithmetic_matches_the_golden_file() {
    let path = Config::repo_root().join("bench/golden/verification_priority.csv");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut rows = 0usize;
    for (line, text) in raw.lines().enumerate() {
        let text = text.trim();
        if text.is_empty() || text.starts_with('#') || text.starts_with("p_false") {
            continue;
        }
        let fields: Vec<&str> = text.split(',').collect();
        assert_eq!(fields.len(), 5, "line {}: {text}", line + 1);
        let p_false: f64 = fields[0].parse().unwrap();
        let risk = RiskLevel::parse(fields[1]).unwrap_or_else(|| panic!("bad risk {}", fields[1]));
        let cost: f64 = fields[2].parse().unwrap();
        let dependence: f64 = fields[3].parse().unwrap();
        let expected: f64 = fields[4].parse().unwrap();

        let got = verification_priority(&assumption(risk, cost, dependence), p_false);
        assert!(
            (got - expected).abs() < 1e-9,
            "line {}: expected {expected}, got {got}",
            line + 1
        );
        rows += 1;
    }
    assert!(rows >= 5, "the golden file must exercise the formula");
}
