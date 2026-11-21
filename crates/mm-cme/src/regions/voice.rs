//! Voice region: capability-aware tier negotiation, streaming tiers, barge-in and
//! failover.

use mm_sm::{SmEvent, StateMachine, Transition, TransitionRecord};
use serde_json::Value;

use crate::error::{CmeError, Result};
use crate::regions::RegionSm;
use crate::voice::{VoiceCapabilities, VoiceNegotiation, VoiceTier};

/// Voice lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceState {
    /// Not in a voice session.
    Idle,
    /// Choosing a tier within the provider's capabilities.
    Negotiating,
    /// Tier 1: ASR → LLM → TTS, sequential.
    Tier1RequestResponse,
    /// Tier 2: overlapping streaming pipeline.
    Tier2Streaming,
    /// Tier 3: realtime full duplex.
    Tier3FullDuplex,
    /// The user interrupted.
    BargedIn,
    /// Cancelling the old response and starting the new one.
    Recovering,
    /// A fallback tier is active.
    Degraded,
}

impl VoiceState {
    /// The stable wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Negotiating => "negotiating",
            Self::Tier1RequestResponse => "tier1_request_response",
            Self::Tier2Streaming => "tier2_streaming",
            Self::Tier3FullDuplex => "tier3_full_duplex",
            Self::BargedIn => "barged_in",
            Self::Recovering => "recovering",
            Self::Degraded => "degraded",
        }
    }
}

/// Events the voice region reacts to.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    /// A voice session started.
    StartVoice,
    /// A tier was negotiated.
    TierNegotiated(String),
    /// The model stream started.
    LlmStreamStart,
    /// A synthesized audio chunk.
    TtsChunk,
    /// Barge-in detected.
    BargeInDetected,
    /// Recovery started.
    Recovering,
    /// Recovery finished.
    RecoveryComplete,
    /// The tier failed.
    TierFailover(String),
    /// Try negotiation again.
    Renegotiate,
    /// Session paused.
    SessionPause,
    /// Session resumed.
    SessionResume,
}

impl VoiceEvent {
    /// The wire name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::StartVoice => "start_voice",
            Self::TierNegotiated(_) => "tier_negotiated",
            Self::LlmStreamStart => "llm_stream_start",
            Self::TtsChunk => "tts_chunk",
            Self::BargeInDetected => "barge_in_detected",
            Self::Recovering => "recovering",
            Self::RecoveryComplete => "recovery_complete",
            Self::TierFailover(_) => "tier_failover",
            Self::Renegotiate => "renegotiate",
            Self::SessionPause => "session_pause",
            Self::SessionResume => "session_resume",
        }
    }

    fn into_sm(self) -> SmEvent {
        let name = self.name();
        match self {
            Self::TierNegotiated(tier) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "tier": tier }))
            }
            Self::TierFailover(tier) => {
                SmEvent::new(name).with_payload(serde_json::json!({ "tier": tier }))
            }
            _ => SmEvent::new(name),
        }
    }
}

/// The voice state-machine definition.
pub fn voice_machine() -> StateMachine {
    StateMachine::new("voice", VoiceState::Idle.as_str())
        .state(VoiceState::Negotiating.as_str())
        .state(VoiceState::Tier1RequestResponse.as_str())
        .state(VoiceState::Tier2Streaming.as_str())
        .state(VoiceState::Tier3FullDuplex.as_str())
        .state(VoiceState::BargedIn.as_str())
        .state(VoiceState::Recovering.as_str())
        .state(VoiceState::Degraded.as_str())
        .transition(Transition::new("idle", "start_voice", "negotiating"))
        .transition(
            Transition::new("negotiating", "tier_negotiated", "tier1_request_response")
                .guard(|ctx| ctx.event.payload["tier"].as_str() == Some("tier1")),
        )
        .transition(
            Transition::new("negotiating", "tier_negotiated", "tier2_streaming")
                .guard(|ctx| ctx.event.payload["tier"].as_str() == Some("tier2")),
        )
        .transition(
            Transition::new("negotiating", "tier_negotiated", "tier3_full_duplex")
                .guard(|ctx| ctx.event.payload["tier"].as_str() == Some("tier3")),
        )
        .transition(Transition::new(
            "tier2_streaming",
            "barge_in_detected",
            "barged_in",
        ))
        .transition(Transition::new(
            "tier3_full_duplex",
            "barge_in_detected",
            "barged_in",
        ))
        .transition(Transition::new("barged_in", "recovering", "recovering"))
        .transition(Transition::new(
            "recovering",
            "recovery_complete",
            "tier2_streaming",
        ))
        .transition(Transition::new(
            "tier1_request_response",
            "tier_failover",
            "degraded",
        ))
        .transition(Transition::new(
            "tier2_streaming",
            "tier_failover",
            "degraded",
        ))
        .transition(Transition::new(
            "tier3_full_duplex",
            "tier_failover",
            "degraded",
        ))
        .transition(Transition::new("degraded", "renegotiate", "negotiating"))
        .transition(Transition::any("session_pause", "idle"))
        .transition(Transition::new("idle", "session_resume", "negotiating"))
}

/// A live voice region.
#[derive(Debug)]
pub struct VoiceRegion {
    /// The region instance.
    pub sm: RegionSm,
}

impl VoiceRegion {
    /// Enter the region.
    pub fn new() -> Result<Self> {
        Ok(Self {
            sm: RegionSm::new("voice", voice_machine())?,
        })
    }

    /// The current state.
    pub fn state(&self) -> &str {
        self.sm.state()
    }

    /// Negotiate a tier within what the deployment supports, then enter it.
    ///
    /// An unsupported request is downgraded to the best supported tier and the
    /// downgrade is reported. A deployment with no voice tier stays in
    /// `negotiating` and returns an error, so a caller can fall back to text
    /// instead of entering a state it cannot serve.
    pub fn negotiate(
        &mut self,
        requested: VoiceTier,
        caps: VoiceCapabilities,
    ) -> Result<VoiceNegotiation> {
        if self.state() == VoiceState::Idle.as_str() {
            self.dispatch(VoiceEvent::StartVoice)?;
        }
        let state = self.state().to_owned();
        if state != VoiceState::Negotiating.as_str() {
            return Err(CmeError::Voice(format!(
                "cannot negotiate a voice tier from `{state}`"
            )));
        }
        let negotiation = caps
            .negotiate(requested)
            .ok_or_else(|| CmeError::Voice("the deployment supports no voice tier".to_owned()))?;
        self.dispatch(VoiceEvent::TierNegotiated(
            negotiation.tier.as_str().to_owned(),
        ))?;
        Ok(negotiation)
    }

    /// Dispatch one event.
    pub fn dispatch(&mut self, event: VoiceEvent) -> Result<Vec<TransitionRecord>> {
        Ok(self.sm.sm.dispatch(event.into_sm())?)
    }

    /// Dispatch a raw event name, used by the session driver.
    pub fn dispatch_name(&mut self, name: &str, payload: Option<Value>) -> Result<()> {
        self.sm.dispatch_name(name, payload)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiation_enters_the_best_supported_tier() {
        let mut region = VoiceRegion::new().unwrap();
        let negotiation = region
            .negotiate(VoiceTier::Tier3, VoiceCapabilities::new(true, true, false))
            .unwrap();
        assert_eq!(negotiation.tier, VoiceTier::Tier2);
        assert!(negotiation.degraded);
        assert_eq!(region.state(), VoiceState::Tier2Streaming.as_str());
    }

    #[test]
    fn a_deployment_without_voice_stays_in_negotiating() {
        let mut region = VoiceRegion::new().unwrap();
        let error = region
            .negotiate(
                VoiceTier::Tier1,
                VoiceCapabilities::new(false, false, false),
            )
            .unwrap_err();
        assert!(matches!(error, CmeError::Voice(_)));
        assert_eq!(region.state(), VoiceState::Negotiating.as_str());
    }

    #[test]
    fn negotiating_outside_negotiation_is_refused() {
        let mut region = VoiceRegion::new().unwrap();
        region
            .negotiate(VoiceTier::Tier1, VoiceCapabilities::all())
            .unwrap();
        assert!(region
            .negotiate(VoiceTier::Tier2, VoiceCapabilities::all())
            .is_err());
    }
}
