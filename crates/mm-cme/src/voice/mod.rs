//! Voice: tiers, audio I/O and prosody.
//!
//! [`VoiceTier`] is the conversation-level choice; the voice region's state
//! machine reports which one is active, and [`VoiceCapabilities`] maps what a
//! deployment actually supports onto the tier it should run. [`audio`] defines the
//! provider-neutral I/O traits and their deterministic mocks, [`pipeline`] runs the
//! three tiers and [`prosody`] maps affect and modality to delivery parameters.
//!
//! Tier 1 is sequential ASR → responder → TTS, Tier 2 overlaps the stages and can
//! speculate on partials, and Tier 3 delegates to a [`RealtimeClient`]. All three
//! are provider-neutral: the mock implementations exist so the engine is testable
//! without a microphone or network.

pub mod audio;
pub mod pipeline;
pub mod prosody;

use serde::{Deserialize, Serialize};

pub use audio::{
    AsrClient, AsrEvent, AsrFinal, AsrPartial, AudioChunk, AudioInput, AudioOutput,
    BargeInDetector, BargeInInfo, MockAsr, MockAudioInput, MockAudioOutput, MockRealtimeClient,
    MockTts, RealTimeEvent, RealtimeClient, TtsClient, Vad,
};
pub use pipeline::{EchoResponder, VoicePipeline, VoiceResponder, VoiceTurn};
pub use prosody::{ProsodyController, ProsodyParams};

/// The voice pipeline tier in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceTier {
    /// Sequential request/response.
    Tier1,
    /// Overlapping streaming pipeline.
    Tier2,
    /// Realtime full duplex.
    Tier3,
}

impl VoiceTier {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tier1 => "tier1",
            Self::Tier2 => "tier2",
            Self::Tier3 => "tier3",
        }
    }

    /// Parse a stored name.
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Tier1, Self::Tier2, Self::Tier3]
            .into_iter()
            .find(|tier| tier.as_str() == value)
    }

    /// The tier's latency ceiling in milliseconds.
    pub fn latency_budget_ms(self) -> u32 {
        match self {
            Self::Tier1 => 1_500,
            Self::Tier2 => 700,
            Self::Tier3 => 300,
        }
    }

    /// Whether the tier can pre-warm on partial input.
    pub fn speculative(self) -> bool {
        matches!(self, Self::Tier2 | Self::Tier3)
    }

    /// Whether the tier produces audio for a turn.
    pub fn streams_audio(self) -> bool {
        true
    }
}

/// What a voice deployment can actually run.
///
/// The conversation engine asks for the best experience and negotiates down to
/// what the deployment supports, rather than assuming every tier is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceCapabilities {
    /// Sequential ASR → responder → TTS.
    pub tier1: bool,
    /// Overlapping streaming pipeline.
    pub tier2: bool,
    /// Full-duplex realtime provider.
    pub tier3: bool,
}

impl Default for VoiceCapabilities {
    fn default() -> Self {
        Self::all()
    }
}

impl VoiceCapabilities {
    /// A deployment that supports every tier.
    pub const fn all() -> Self {
        Self {
            tier1: true,
            tier2: true,
            tier3: true,
        }
    }

    /// A deployment with explicit support per tier.
    pub const fn new(tier1: bool, tier2: bool, tier3: bool) -> Self {
        Self {
            tier1,
            tier2,
            tier3,
        }
    }

    /// Whether a tier can run here.
    pub const fn supports(self, tier: VoiceTier) -> bool {
        match tier {
            VoiceTier::Tier1 => self.tier1,
            VoiceTier::Tier2 => self.tier2,
            VoiceTier::Tier3 => self.tier3,
        }
    }

    /// The highest tier this deployment supports, if any.
    pub const fn best(self) -> Option<VoiceTier> {
        if self.tier3 {
            Some(VoiceTier::Tier3)
        } else if self.tier2 {
            Some(VoiceTier::Tier2)
        } else if self.tier1 {
            Some(VoiceTier::Tier1)
        } else {
            None
        }
    }

    /// Choose the tier to run for `requested`.
    ///
    /// A supported request is honoured. A request the deployment cannot run
    /// falls back to the best supported tier and reports the downgrade. When no
    /// tier is supported the result is `None`.
    pub const fn negotiate(self, requested: VoiceTier) -> Option<VoiceNegotiation> {
        if self.supports(requested) {
            return Some(VoiceNegotiation {
                tier: requested,
                degraded: false,
            });
        }
        match self.best() {
            Some(tier) => Some(VoiceNegotiation {
                tier,
                degraded: true,
            }),
            None => None,
        }
    }
}

/// The tier a deployment agreed to run, and whether it is below the request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceNegotiation {
    /// The tier to enter.
    pub tier: VoiceTier,
    /// True when the deployment could not run the requested tier.
    pub degraded: bool,
}

/// Voice configuration carried on a turn plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceTurnConfig {
    /// The negotiated tier.
    pub tier: VoiceTier,
    /// Whether to pre-warm on partial input.
    pub speculative: bool,
    /// Whether barge-in can interrupt the turn.
    pub barge_in_enabled: bool,
}

impl VoiceTurnConfig {
    /// The default configuration for a tier.
    pub fn for_tier(tier: VoiceTier) -> Self {
        Self {
            tier,
            speculative: tier.speculative(),
            barge_in_enabled: true,
        }
    }
}

impl Default for VoiceTurnConfig {
    fn default() -> Self {
        Self::for_tier(VoiceTier::Tier1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_supported_request_is_honoured() {
        let negotiation = VoiceCapabilities::all()
            .negotiate(VoiceTier::Tier2)
            .unwrap();
        assert_eq!(negotiation.tier, VoiceTier::Tier2);
        assert!(!negotiation.degraded);
    }

    #[test]
    fn an_unsupported_request_falls_back_to_the_best_supported_tier() {
        let caps = VoiceCapabilities::new(true, true, false);
        let negotiation = caps.negotiate(VoiceTier::Tier3).unwrap();
        assert_eq!(negotiation.tier, VoiceTier::Tier2);
        assert!(negotiation.degraded);
        assert_eq!(caps.best(), Some(VoiceTier::Tier2));
    }

    #[test]
    fn a_deployment_without_voice_reports_no_tier() {
        let caps = VoiceCapabilities::new(false, false, false);
        assert_eq!(caps.negotiate(VoiceTier::Tier1), None);
        assert_eq!(caps.best(), None);
    }
}
