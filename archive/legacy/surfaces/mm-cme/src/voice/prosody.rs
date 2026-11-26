//! Prosody: affect and modality mapped to delivery parameters.

use serde::{Deserialize, Serialize};

use crate::planner::ResponseModality;

/// How a synthesised answer should be delivered.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ProsodyParams {
    /// Speaking rate, 1.0 neutral.
    pub rate: f32,
    /// Pitch offset, 0.0 neutral.
    pub pitch: f32,
    /// Volume, 1.0 neutral.
    pub volume: f32,
    /// Warmth, 0.0..1.0.
    pub warmth: f32,
    /// Pause multiplier, 1.0 neutral.
    pub pause_factor: f32,
    /// Detail, 0.0..1.0.
    pub detail: f32,
}

impl Default for ProsodyParams {
    fn default() -> Self {
        Self {
            rate: 1.0,
            pitch: 0.0,
            volume: 1.0,
            warmth: 0.5,
            pause_factor: 1.0,
            detail: 0.5,
        }
    }
}

impl ProsodyParams {
    /// Clamp every parameter to a synthesizer-safe range.
    pub fn clamp(mut self) -> Self {
        self.rate = self.rate.clamp(0.5, 2.0);
        self.pitch = self.pitch.clamp(-1.0, 1.0);
        self.volume = self.volume.clamp(0.0, 2.0);
        self.warmth = self.warmth.clamp(0.0, 1.0);
        self.pause_factor = self.pause_factor.clamp(0.0, 3.0);
        self.detail = self.detail.clamp(0.0, 1.0);
        self
    }
}

/// Maps affect and response modality to [`ProsodyParams`].
#[derive(Debug, Clone, Copy, Default)]
pub struct ProsodyController;

impl ProsodyController {
    /// Compute delivery parameters from the being's affect and the turn's shape.
    ///
    /// The coefficients are the design's: affect modulates in proportion to its
    /// axes, then modality slows or speeds the delivery and changes emphasis.
    pub fn compute(
        &self,
        affect: &mm_being::Affect,
        modality: Option<ResponseModality>,
    ) -> ProsodyParams {
        let mut params = ProsodyParams::default();
        params.rate += 0.15 * affect.arousal;
        params.pitch += 0.1 * affect.valence;
        params.volume += 0.1 * affect.energy;
        params.warmth += 0.2 * affect.warmth;
        params.pause_factor += 0.3 * affect.caution;

        if let Some(modality) = modality {
            match modality {
                ResponseModality::DrillDown => {
                    params.rate *= 0.9;
                    params.detail += 0.2;
                }
                ResponseModality::Summarize => {
                    params.rate *= 1.1;
                    params.detail -= 0.1;
                }
                ResponseModality::Explain => {
                    params.pause_factor *= 1.2;
                }
                ResponseModality::ZoomOut | ResponseModality::Bullets => {}
            }
        }
        params.clamp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_affect_and_explain_only_stretch_pauses() {
        let params = ProsodyController.compute(
            &mm_being::Affect::neutral(),
            Some(ResponseModality::Explain),
        );
        assert!((params.pause_factor - 1.2).abs() < 0.001);
        assert!((params.rate - 1.0).abs() < 0.001);
        assert!((params.volume - 1.0).abs() < 0.001);
    }

    #[test]
    fn affect_raises_rate_volume_and_warmth() {
        let affect = mm_being::Affect {
            arousal: 1.0,
            energy: 1.0,
            warmth: 1.0,
            caution: -1.0,
            ..mm_being::Affect::neutral()
        };
        let params = ProsodyController.compute(&affect, Some(ResponseModality::Summarize));
        assert!(params.rate > 1.2);
        assert!(params.volume > 1.0);
        assert!(params.warmth > 0.5);
        assert!(params.pause_factor < 1.0);
    }

    #[test]
    fn parameters_are_clamped() {
        let affect = mm_being::Affect {
            arousal: 1.0,
            energy: 1.0,
            warmth: 1.0,
            caution: 1.0,
            valence: 1.0,
            ..mm_being::Affect::neutral()
        };
        let params = ProsodyController.compute(&affect, Some(ResponseModality::DrillDown));
        assert!((0.5..=2.0).contains(&params.rate));
        assert!((0.0..=1.0).contains(&params.warmth));
        assert!((0.0..=1.0).contains(&params.detail));
    }
}
