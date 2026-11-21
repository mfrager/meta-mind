//! The three voice tier pipelines, provider-neutral and mock-testable.
//!
//! Tier 1 transcribes, responds and synthesises sequentially. Tier 2 overlaps:
//! it reacts to the recogniser's first final while the input continues, and it
//! stops on barge-in. Tier 3 delegates to a [`RealtimeClient`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{CmeError, Result};
use crate::voice::audio::{
    AsrClient, AsrEvent, AudioChunk, BargeInDetector, BargeInInfo, RealTimeEvent, RealtimeClient,
    TtsClient, Vad,
};
use crate::voice::prosody::ProsodyController;
use crate::voice::VoiceTier;

/// Generates the assistant's textual reply. Kept as a trait so the voice
/// pipeline does not depend on the turn executor and tests can script it.
#[async_trait]
pub trait VoiceResponder: Send + Sync {
    /// Respond to the user's transcript.
    async fn respond(
        &self,
        transcript: &str,
        prosody: &crate::voice::ProsodyParams,
    ) -> Result<String>;
}

/// A responder that echoes the transcript, for tests and local smoke checks.
#[derive(Debug, Clone, Copy, Default)]
pub struct EchoResponder;

#[async_trait]
impl VoiceResponder for EchoResponder {
    async fn respond(
        &self,
        transcript: &str,
        _prosody: &crate::voice::ProsodyParams,
    ) -> Result<String> {
        Ok(format!("Echo: {transcript}"))
    }
}

/// The result of one voice turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VoiceTurn {
    /// The tier that ran.
    pub tier: VoiceTier,
    /// Final transcript, joined.
    pub transcript: String,
    /// The assistant's text.
    pub reply: String,
    /// Synthesised or provider audio.
    pub audio: Vec<AudioChunk>,
    /// Partials observed, in order.
    pub partials: Vec<String>,
    /// The interruption, when the turn was barged in on.
    pub barge_in: Option<BargeInInfo>,
}

/// Runs a voice turn over pluggable audio, ASR, TTS and realtime backends.
pub struct VoicePipeline {
    asr: Box<dyn AsrClient>,
    tts: Box<dyn TtsClient>,
    vad: Box<dyn Vad>,
    barge_in: BargeInDetector,
    prosody: ProsodyController,
}

impl std::fmt::Debug for VoicePipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoicePipeline")
            .field("prosody", &self.prosody)
            .finish_non_exhaustive()
    }
}

impl VoicePipeline {
    /// Build a pipeline from its backends.
    pub fn new(asr: Box<dyn AsrClient>, tts: Box<dyn TtsClient>, vad: Box<dyn Vad>) -> Self {
        Self {
            asr,
            tts,
            vad,
            barge_in: BargeInDetector::default(),
            prosody: ProsodyController,
        }
    }

    /// The interruption detector.
    pub fn barge_in(&self) -> &BargeInDetector {
        &self.barge_in
    }

    /// Tier 1: each speech block is transcribed, the first final is answered,
    /// and the answer is synthesised once at the end.
    pub async fn run_tier1(
        &mut self,
        input: Vec<AudioChunk>,
        responder: &dyn VoiceResponder,
        affect: &mm_being::Affect,
    ) -> Result<VoiceTurn> {
        self.barge_in.reset();
        let mut partials = Vec::new();
        let mut finals = Vec::new();
        for chunk in input {
            if !self.vad.is_speech(&chunk) {
                continue;
            }
            for event in self.asr.transcribe(&chunk).await? {
                match event {
                    AsrEvent::Partial(partial) => partials.push(partial.0),
                    AsrEvent::Final(final_text) => finals.push(final_text.0),
                }
            }
        }
        let transcript = finals.join(" ");
        if transcript.trim().is_empty() {
            return Err(CmeError::Voice("no speech recognised".to_owned()));
        }
        let params = self.prosody.compute(affect, None);
        let reply = responder.respond(&transcript, &params).await?;
        let audio = self.tts.synthesize(&reply, &params).await?;
        Ok(VoiceTurn {
            tier: VoiceTier::Tier1,
            transcript,
            reply,
            audio,
            partials,
            barge_in: None,
        })
    }

    /// Tier 2: answer as soon as a final arrives and keep consuming input, so
    /// recognition of later blocks overlaps synthesis of earlier audio.
    pub async fn run_tier2(
        &mut self,
        input: Vec<AudioChunk>,
        responder: &dyn VoiceResponder,
        affect: &mm_being::Affect,
    ) -> Result<VoiceTurn> {
        self.barge_in.reset();
        let params = self.prosody.compute(affect, None);
        let mut partials = Vec::new();
        let mut finals = Vec::new();
        let mut reply = String::new();
        let mut audio = Vec::new();
        let mut interruption = None;

        for chunk in input {
            // While the previous answer plays, a loud input block interrupts it.
            // Before any audio exists there is nothing to interrupt.
            if !audio.is_empty() {
                let played_ms: u64 = audio.iter().map(AudioChunk::duration_ms).sum();
                if let Some(info) = self.barge_in.observe(played_ms, chunk.energy()) {
                    interruption = Some(info);
                    break;
                }
            }
            if !self.vad.is_speech(&chunk) {
                continue;
            }
            for event in self.asr.transcribe(&chunk).await? {
                match event {
                    AsrEvent::Partial(partial) => partials.push(partial.0),
                    AsrEvent::Final(final_text) => {
                        finals.push(final_text.0.clone());
                        let response = responder.respond(&final_text.0, &params).await?;
                        audio.extend(self.tts.synthesize(&response, &params).await?);
                        if !reply.is_empty() {
                            reply.push(' ');
                        }
                        reply.push_str(&response);
                    }
                }
            }
        }
        if finals.is_empty() {
            return Err(CmeError::Voice("no speech recognised".to_owned()));
        }
        Ok(VoiceTurn {
            tier: VoiceTier::Tier2,
            transcript: finals.join(" "),
            reply,
            audio,
            partials,
            barge_in: interruption,
        })
    }

    /// Tier 3: send blocks to a realtime provider and collect its events.
    pub async fn run_tier3(
        &mut self,
        realtime: &mut dyn RealtimeClient,
        input: Vec<AudioChunk>,
    ) -> Result<VoiceTurn> {
        realtime.connect().await?;
        let mut transcript = String::new();
        let mut audio = Vec::new();
        let mut partials = Vec::new();
        let mut done = false;
        for chunk in input {
            if done {
                break;
            }
            for event in realtime.send_audio(&chunk).await? {
                match event {
                    RealTimeEvent::Transcript(text) => {
                        if transcript.is_empty() {
                            transcript.push_str(&text);
                        } else {
                            transcript.push(' ');
                            transcript.push_str(&text);
                        }
                        partials.push(text);
                    }
                    RealTimeEvent::Audio(chunk) => audio.push(chunk),
                    RealTimeEvent::Done => done = true,
                }
            }
        }
        realtime.close().await?;
        Ok(VoiceTurn {
            tier: VoiceTier::Tier3,
            transcript,
            reply: String::new(),
            audio,
            partials,
            barge_in: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::audio::{
        AsrEvent, AudioChunk, EnergyVad, MockAsr, MockRealtimeClient, MockTts,
    };

    fn speech() -> AudioChunk {
        AudioChunk::tone(4_000, 160, 16_000)
    }

    #[tokio::test]
    async fn tier1_runs_seqentially() {
        let mut pipeline = VoicePipeline::new(
            Box::new(MockAsr::once("hello there")),
            Box::new(MockTts::default()),
            Box::new(EnergyVad::default()),
        );
        let turn = pipeline
            .run_tier1(vec![speech()], &EchoResponder, &mm_being::Affect::neutral())
            .await
            .unwrap();
        assert_eq!(turn.tier, VoiceTier::Tier1);
        assert_eq!(turn.transcript, "hello there");
        assert_eq!(turn.reply, "Echo: hello there");
        assert_eq!(turn.audio.len(), 3);
        assert!(turn.barge_in.is_none());
    }

    #[tokio::test]
    async fn tier2_overlaps_and_stops_on_barge_in() {
        let asr = MockAsr::scripted(vec![
            vec![
                AsrEvent::Partial(crate::voice::AsrPartial("hel".into())),
                AsrEvent::Final(crate::voice::AsrFinal("hello".into())),
            ],
            vec![AsrEvent::Final(crate::voice::AsrFinal("again".into()))],
        ]);
        let mut pipeline = VoicePipeline::new(
            Box::new(asr),
            Box::new(MockTts::default()),
            Box::new(EnergyVad::default()),
        );
        // Second block is loud: after the first reply starts playing it barges in.
        let turn = pipeline
            .run_tier2(
                vec![speech(), speech()],
                &EchoResponder,
                &mm_being::Affect::neutral(),
            )
            .await
            .unwrap();
        assert_eq!(turn.tier, VoiceTier::Tier2);
        assert_eq!(turn.partials, vec!["hel".to_owned()]);
        assert_eq!(turn.transcript, "hello");
        assert!(turn.reply.starts_with("Echo: hello"));
        assert!(turn.barge_in.is_some());
    }

    #[tokio::test]
    async fn tier3_delegates_to_the_realtime_client() {
        let mut pipeline = VoicePipeline::new(
            Box::new(MockAsr::default()),
            Box::new(MockTts::default()),
            Box::new(EnergyVad::default()),
        );
        let mut client = MockRealtimeClient::scripted(vec![vec![
            RealTimeEvent::Transcript("hi".into()),
            RealTimeEvent::Audio(AudioChunk::tone(1_000, 160, 16_000)),
            RealTimeEvent::Done,
        ]]);
        let turn = pipeline
            .run_tier3(&mut client, vec![speech()])
            .await
            .unwrap();
        assert_eq!(turn.tier, VoiceTier::Tier3);
        assert_eq!(turn.transcript, "hi");
        assert_eq!(turn.audio.len(), 1);
        assert!(!client.connected);
    }
}
