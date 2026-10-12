//! Project the stored constitution for an operator turn.

use mm_being::{BehaviorParams, ContextVector, Personality};

/// `chat`, and `high_stakes` when the turn is about a gate.
pub fn context(high_stakes: bool) -> ContextVector {
    let ctx = ContextVector::new().with("chat", 1.0);
    if high_stakes {
        ctx.with("high_stakes", 1.0)
    } else {
        ctx
    }
}

pub fn project(personality: &Personality, high_stakes: bool) -> BehaviorParams {
    personality.project(&context(high_stakes))
}

/// Stem list from the plan. It only raises the voice. It does not select a tool.
pub fn mentions_gate(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "promote",
        "reject",
        "loop",
        "hot-load",
        "hot load",
        "roll back",
        "rollback",
        "forget",
        "apply",
    ]
    .iter()
    .any(|stem| lower.contains(stem))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }

    #[test]
    fn chat_projects_the_baseline_deltas() {
        let voice = project(&Personality::baseline(), false);
        near(voice.warmth, 0.70);
        near(voice.directness, 0.80);
        near(voice.curiosity, 0.70);
        near(voice.playfulness, 0.30);
        near(voice.formality, 0.40);
    }

    #[test]
    fn chat_and_high_stakes_sum_then_clamp() {
        let voice = project(&Personality::baseline(), true);
        near(voice.warmth, 0.70);
        near(voice.directness, 0.90);
        near(voice.curiosity, 0.70);
        near(voice.playfulness, 0.00);
        // 0.50 - 0.10 + 0.30. The phase note's 0.80 does not match that sum.
        near(voice.formality, 0.70);
    }

    #[test]
    fn a_stored_warmth_is_clamped_under_chat() {
        let mut personality = Personality::baseline();
        personality.set_disposition("warmth", 0.9);
        let voice = project(&personality, false);
        near(voice.warmth, 1.0);
    }

    #[test]
    fn an_empty_context_does_not_apply_chat() {
        let personality = Personality::baseline();
        let voice = personality.project(&ContextVector::empty());
        near(voice.warmth, personality.get_disposition("warmth"));
        near(voice.directness, 0.60);
    }
}
