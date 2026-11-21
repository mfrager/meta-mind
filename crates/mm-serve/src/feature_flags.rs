//! Phase 7 feature flags.
//!
//! The flags are environment-driven so the same binary can serve the legacy
//! controller and the conversation engine without a rebuild:
//!
//! | variable | default | effect |
//! |---|---|---|
//! | `MM_CME_ENABLED` | `false` | `POST /api/chat` uses the CME driver |
//! | `MM_CME_PROACTIVE` | `true` | attach the built-in proactive plugins |
//! | `MM_CME_VOICE_TIER` | unset | pre-negotiate `tier1`, `tier2` or `tier3` |
//! | `MM_CME_MEMORY_COMPRESSION` | `true` | compress completed turns |
//! | `MM_CME_PREFERENCE_LEARNING` | `true` | enable preference learning |
//! | `MM_CME_IMPROVEMENT` | `false` | run the scheduled improvement loop |
//! | `MM_CME_AGENTSTREAM_LOGGING` | `true` | log CME frames to agentstream |

use mm_cme::VoiceTier;

/// Which execution path and optional CME features are active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FeatureFlags {
    /// Route `POST /api/chat` through the conversation engine.
    pub cme_enabled: bool,
    /// Attach the proactive plugin engine to new CME sessions.
    pub proactive_enabled: bool,
    /// The voice tier negotiated before the first turn, when set.
    pub voice_tier: Option<VoiceTier>,
    /// Compress completed turns through the unified memory system.
    pub memory_compression_enabled: bool,
    /// Learn preference signals from completed turns.
    pub preference_learning_enabled: bool,
    /// Run the scheduled report/dashboard improvement loop.
    pub improvement_loop_enabled: bool,
    /// Write conversation frames to the agentstream log.
    pub agentstream_logging: bool,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            cme_enabled: false,
            proactive_enabled: true,
            voice_tier: None,
            memory_compression_enabled: true,
            preference_learning_enabled: true,
            improvement_loop_enabled: false,
            agentstream_logging: true,
        }
    }
}

impl FeatureFlags {
    /// Read the flags from the process environment.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        Self {
            cme_enabled: env_bool("MM_CME_ENABLED", defaults.cme_enabled),
            proactive_enabled: env_bool("MM_CME_PROACTIVE", defaults.proactive_enabled),
            voice_tier: std::env::var("MM_CME_VOICE_TIER")
                .ok()
                .and_then(|value| VoiceTier::parse(value.trim().to_ascii_lowercase().as_str())),
            memory_compression_enabled: env_bool(
                "MM_CME_MEMORY_COMPRESSION",
                defaults.memory_compression_enabled,
            ),
            preference_learning_enabled: env_bool(
                "MM_CME_PREFERENCE_LEARNING",
                defaults.preference_learning_enabled,
            ),
            improvement_loop_enabled: env_bool(
                "MM_CME_IMPROVEMENT",
                defaults.improvement_loop_enabled,
            ),
            agentstream_logging: env_bool(
                "MM_CME_AGENTSTREAM_LOGGING",
                defaults.agentstream_logging,
            ),
        }
    }
}

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => default,
        },
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_the_legacy_path_until_opted_in() {
        let flags = FeatureFlags::default();
        assert!(!flags.cme_enabled);
        assert!(flags.proactive_enabled);
        assert!(flags.memory_compression_enabled);
        assert!(flags.agentstream_logging);
        assert!(flags.voice_tier.is_none());
    }

    #[test]
    fn voice_tiers_parse_from_wire_names() {
        assert_eq!(VoiceTier::parse("tier2"), Some(VoiceTier::Tier2));
        assert_eq!(VoiceTier::parse("nope"), None);
    }
}
