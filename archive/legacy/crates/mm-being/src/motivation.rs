//! Intrinsic motivation: a few drives the being acts to satisfy.
//!
//! Motivation is kept deliberately crude. The point is not to model desire in
//! detail but to give the planning layers a *reason to act when nobody asked*, so
//! the drives are a closed set of five with a strength each, stored as rows.
//! Anything richer belongs in the cognitive library (Phase 7), where a technique
//! can name the drive it serves; here there is only the signal itself.

use serde::{Deserialize, Serialize};

/// A drive the being acts to satisfy without being asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Drive {
    /// Getting better at what it does.
    Competence,
    /// Learning what it does not know.
    Curiosity,
    /// Keeping its own state and beliefs consistent.
    Coherence,
    /// Maintaining the relationships it depends on.
    Relatedness,
    /// Acting on its own judgment rather than by instruction alone.
    Autonomy,
}

/// Every intrinsic drive, in a stable order.
pub const INTRINSIC_DRIVES: [Drive; 5] = [
    Drive::Competence,
    Drive::Curiosity,
    Drive::Coherence,
    Drive::Relatedness,
    Drive::Autonomy,
];

impl Drive {
    /// The stable wire name, recorded in `motivations.drive`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Drive::Competence => "competence",
            Drive::Curiosity => "curiosity",
            Drive::Coherence => "coherence",
            Drive::Relatedness => "relatedness",
            Drive::Autonomy => "autonomy",
        }
    }

    /// Parse a wire name.
    pub fn parse(s: &str) -> Option<Self> {
        INTRINSIC_DRIVES.into_iter().find(|d| d.as_str() == s)
    }

    /// The strength a drive starts at, in `[0, 1]`.
    ///
    /// Curiosity leads, because an idle agent that explores is more useful than
    /// one that optimises what it already knows; autonomy trails, because a being
    /// that over-weights its own judgment is harder to correct.
    pub fn default_strength(self) -> f32 {
        match self {
            Drive::Competence => 0.60,
            Drive::Curiosity => 0.70,
            Drive::Coherence => 0.50,
            Drive::Relatedness => 0.55,
            Drive::Autonomy => 0.45,
        }
    }
}

impl std::fmt::Display for Drive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One drive and how strongly it is felt.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Motivation {
    /// Which drive.
    pub drive: Drive,
    /// How strongly, in `[0, 1]`.
    pub strength: f32,
}

impl Motivation {
    /// Build a motivation, clamping the strength into `[0, 1]`.
    pub fn new(drive: Drive, strength: f32) -> Self {
        Motivation {
            drive,
            strength: strength.clamp(0.0, 1.0),
        }
    }

    /// Whether this drive is currently driving anything: a strength of zero means
    /// the being is not acting on it, which is different from a low strength.
    pub fn is_active(&self) -> bool {
        self.strength > 0.0
    }
}

/// The motivations a new being starts with.
pub fn intrinsic_defaults() -> Vec<Motivation> {
    INTRINSIC_DRIVES
        .into_iter()
        .map(|drive| Motivation::new(drive, drive.default_strength()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_cover_every_drive_once() {
        let defaults = intrinsic_defaults();
        assert_eq!(defaults.len(), INTRINSIC_DRIVES.len());
        let mut seen: Vec<&str> = defaults.iter().map(|m| m.drive.as_str()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), INTRINSIC_DRIVES.len(), "a drive was duplicated");
    }

    #[test]
    fn strengths_are_clamped_into_range() {
        assert!((0.0..=1.0).contains(&Motivation::new(Drive::Curiosity, 5.0).strength));
        assert!((0.0..=1.0).contains(&Motivation::new(Drive::Curiosity, -5.0).strength));
        assert!((Motivation::new(Drive::Curiosity, 5.0).strength - 1.0).abs() < 1e-6);
        assert!(Motivation::new(Drive::Curiosity, -5.0).strength.abs() < 1e-6);
    }

    #[test]
    fn every_default_strength_is_in_range_and_non_zero() {
        for drive in INTRINSIC_DRIVES {
            let strength = drive.default_strength();
            assert!((0.0..=1.0).contains(&strength), "{drive} = {strength}");
            assert!(strength > 0.0, "{drive} would never act");
        }
    }

    #[test]
    fn a_zero_strength_drive_is_not_active() {
        assert!(!Motivation::new(Drive::Autonomy, 0.0).is_active());
        assert!(Motivation::new(Drive::Autonomy, 0.01).is_active());
    }

    #[test]
    fn drive_names_round_trip() {
        for drive in INTRINSIC_DRIVES {
            assert_eq!(Drive::parse(drive.as_str()), Some(drive));
            assert_eq!(drive.to_string(), drive.as_str());
        }
        assert_eq!(Drive::parse("power"), None);
    }
}
