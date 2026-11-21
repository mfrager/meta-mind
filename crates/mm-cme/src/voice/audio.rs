//! Provider-neutral audio I/O.
//!
//! The engine depends on these traits, never on a microphone, WebRTC stack or a
//! cloud vendor. The mocks in this module are deterministic and exist so the tier
//! pipelines can be tested end to end without hardware or network.

use std::collections::VecDeque;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// One block of 16-bit mono PCM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioChunk {
    /// Samples.
    pub pcm: Vec<i16>,
    /// Samples per second.
    pub sample_rate: u32,
}

impl AudioChunk {
    /// A chunk from samples.
    pub fn new(pcm: Vec<i16>, sample_rate: u32) -> Self {
        Self { pcm, sample_rate }
    }

    /// A silent chunk of `samples` samples.
    pub fn silence(samples: usize, sample_rate: u32) -> Self {
        Self {
            pcm: vec![0; samples],
            sample_rate,
        }
    }

    /// A chunk of constant amplitude, useful for VAD and barge-in tests.
    pub fn tone(amplitude: i16, samples: usize, sample_rate: u32) -> Self {
        Self {
            pcm: vec![amplitude; samples],
            sample_rate,
        }
    }

    /// The duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.pcm.len() as u64 * 1_000) / u64::from(self.sample_rate)
    }

    /// Root-mean-square amplitude, normalised to 0.0..=1.0.
    pub fn energy(&self) -> f32 {
        if self.pcm.is_empty() {
            return 0.0;
        }
        let sum: f64 = self
            .pcm
            .iter()
            .map(|sample| {
                let value = f64::from(*sample) / f64::from(i16::MAX);
                value * value
            })
            .sum();
        (sum / self.pcm.len() as f64).sqrt() as f32
    }
}

/// A partial transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrPartial(pub String);

/// A final transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsrFinal(pub String);

/// One ASR result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AsrEvent {
    /// An unstable partial.
    Partial(AsrPartial),
    /// A committed transcript.
    Final(AsrFinal),
}

/// A detected interruption.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BargeInInfo {
    /// Offset from the start of the response, in milliseconds.
    pub at_ms: u64,
    /// Speech energy at detection.
    pub energy: f32,
}

/// A source of audio.
#[async_trait]
pub trait AudioInput: Send {
    /// The next block, or `None` at end of stream.
    async fn next_chunk(&mut self) -> Result<Option<AudioChunk>>;
}

/// A sink for audio.
#[async_trait]
pub trait AudioOutput: Send {
    /// Write a block.
    async fn write(&mut self, chunk: &AudioChunk) -> Result<()>;

    /// Finish and flush.
    async fn finish(&mut self) -> Result<()>;
}

/// Voice activity detection.
pub trait Vad: Send + Sync {
    /// Whether the block contains speech.
    fn is_speech(&self, chunk: &AudioChunk) -> bool;
}

/// A speech recogniser.
#[async_trait]
pub trait AsrClient: Send {
    /// Transcribe one block, yielding partials and possibly a final.
    async fn transcribe(&mut self, chunk: &AudioChunk) -> Result<Vec<AsrEvent>>;
}

/// A speech synthesiser.
#[async_trait]
pub trait TtsClient: Send {
    /// Synthesise text into audio blocks.
    async fn synthesize(&mut self, text: &str, prosody: &ProsodyParams) -> Result<Vec<AudioChunk>>;
}

/// One event from a realtime provider.
#[derive(Debug, Clone, PartialEq)]
pub enum RealTimeEvent {
    /// A transcript update.
    Transcript(String),
    /// Output audio.
    Audio(AudioChunk),
    /// The response finished.
    Done,
}

/// A full-duplex realtime provider (OpenAI Realtime, Gemini Live, …).
#[async_trait]
pub trait RealtimeClient: Send {
    /// Open the session.
    async fn connect(&mut self) -> Result<()>;

    /// Send input audio and receive what the provider emits.
    async fn send_audio(&mut self, chunk: &AudioChunk) -> Result<Vec<RealTimeEvent>>;

    /// Close the session.
    async fn close(&mut self) -> Result<()>;
}

/// Simple energy-threshold interruption detector with a refractory window.
#[derive(Debug, Clone)]
pub struct BargeInDetector {
    /// Minimum energy that counts as speech.
    pub threshold: f32,
    /// Ignore detection for this long after the response starts.
    pub grace_ms: u64,
    elapsed_ms: u64,
    fired: bool,
}

impl Default for BargeInDetector {
    fn default() -> Self {
        Self {
            threshold: 0.05,
            grace_ms: 0,
            elapsed_ms: 0,
            fired: false,
        }
    }
}

impl BargeInDetector {
    /// A detector with an energy threshold.
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold: threshold.clamp(0.0, 1.0),
            ..Self::default()
        }
    }

    /// Observe output playback duration and input energy. Returns the
    /// interruption once; subsequent calls stay quiet.
    pub fn observe(&mut self, playback_ms: u64, input_energy: f32) -> Option<BargeInInfo> {
        self.elapsed_ms = self.elapsed_ms.saturating_add(playback_ms);
        if self.fired || self.elapsed_ms < self.grace_ms || input_energy < self.threshold {
            return None;
        }
        self.fired = true;
        Some(BargeInInfo {
            at_ms: self.elapsed_ms,
            energy: input_energy,
        })
    }

    /// Whether an interruption was reported.
    pub fn fired(&self) -> bool {
        self.fired
    }

    /// Clear for the next response.
    pub fn reset(&mut self) {
        self.elapsed_ms = 0;
        self.fired = false;
    }
}

/// A scripted audio source.
#[derive(Debug, Default)]
pub struct MockAudioInput {
    chunks: VecDeque<AudioChunk>,
}

impl MockAudioInput {
    /// A source over the given blocks.
    pub fn new(chunks: Vec<AudioChunk>) -> Self {
        Self {
            chunks: chunks.into(),
        }
    }
}

#[async_trait]
impl AudioInput for MockAudioInput {
    async fn next_chunk(&mut self) -> Result<Option<AudioChunk>> {
        Ok(self.chunks.pop_front())
    }
}

/// An in-memory audio sink.
#[derive(Debug, Default)]
pub struct MockAudioOutput {
    /// Blocks written so far.
    pub written: Vec<AudioChunk>,
    /// Whether `finish` was called.
    pub finished: bool,
}

#[async_trait]
impl AudioOutput for MockAudioOutput {
    async fn write(&mut self, chunk: &AudioChunk) -> Result<()> {
        self.written.push(chunk.clone());
        Ok(())
    }

    async fn finish(&mut self) -> Result<()> {
        self.finished = true;
        Ok(())
    }
}

/// A VAD that treats energy above a threshold as speech.
#[derive(Debug, Clone)]
pub struct EnergyVad {
    /// Minimum energy.
    pub threshold: f32,
}

impl Default for EnergyVad {
    fn default() -> Self {
        Self { threshold: 0.02 }
    }
}

impl Vad for EnergyVad {
    fn is_speech(&self, chunk: &AudioChunk) -> bool {
        chunk.energy() >= self.threshold
    }
}

/// A scripted recogniser: one event list per input block.
#[derive(Debug, Default)]
pub struct MockAsr {
    script: VecDeque<Vec<AsrEvent>>,
}

impl MockAsr {
    /// A recogniser that emits `final_text` once, on the first block.
    pub fn once(final_text: impl Into<String>) -> Self {
        Self {
            script: VecDeque::from([vec![AsrEvent::Final(AsrFinal(final_text.into()))]]),
        }
    }

    /// A recogniser that emits a partial then a final per block.
    pub fn partial_then_final(partial: impl Into<String>, final_text: impl Into<String>) -> Self {
        let partial = partial.into();
        let final_text = final_text.into();
        Self {
            script: VecDeque::from([vec![
                AsrEvent::Partial(AsrPartial(partial)),
                AsrEvent::Final(AsrFinal(final_text)),
            ]]),
        }
    }

    /// A recogniser over an explicit script.
    pub fn scripted(script: Vec<Vec<AsrEvent>>) -> Self {
        Self {
            script: script.into(),
        }
    }
}

#[async_trait]
impl AsrClient for MockAsr {
    async fn transcribe(&mut self, _chunk: &AudioChunk) -> Result<Vec<AsrEvent>> {
        Ok(self.script.pop_front().unwrap_or_default())
    }
}

/// A deterministic synthesiser: one tone block per word.
#[derive(Debug, Clone)]
pub struct MockTts {
    /// Samples per output block.
    pub block_samples: usize,
    /// Samples per second.
    pub sample_rate: u32,
}

impl Default for MockTts {
    fn default() -> Self {
        Self {
            block_samples: 160,
            sample_rate: 16_000,
        }
    }
}

#[async_trait]
impl TtsClient for MockTts {
    async fn synthesize(
        &mut self,
        text: &str,
        _prosody: &ProsodyParams,
    ) -> Result<Vec<AudioChunk>> {
        let blocks = text.split_whitespace().count().max(1);
        Ok((0..blocks)
            .map(|_| AudioChunk::tone(2_000, self.block_samples, self.sample_rate))
            .collect())
    }
}

/// A scripted full-duplex provider.
#[derive(Debug, Default)]
pub struct MockRealtimeClient {
    /// Events returned per sent block.
    pub script: VecDeque<Vec<RealTimeEvent>>,
    /// Whether `connect` was called.
    pub connected: bool,
}

impl MockRealtimeClient {
    /// A client that answers every block with a transcript and audio, then done.
    pub fn scripted(events: Vec<Vec<RealTimeEvent>>) -> Self {
        Self {
            script: events.into(),
            connected: false,
        }
    }
}

#[async_trait]
impl RealtimeClient for MockRealtimeClient {
    async fn connect(&mut self) -> Result<()> {
        self.connected = true;
        Ok(())
    }

    async fn send_audio(&mut self, _chunk: &AudioChunk) -> Result<Vec<RealTimeEvent>> {
        Ok(self
            .script
            .pop_front()
            .unwrap_or_else(|| vec![RealTimeEvent::Done]))
    }

    async fn close(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
}

// The pipeline's prosody type lives in `super::prosody`; importing it here keeps
// the trait signature readable without a cycle at module level.
use super::prosody::ProsodyParams;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_and_duration_are_measured() {
        let chunk = AudioChunk::tone(16_000, 160, 16_000);
        assert!((chunk.energy() - 0.488).abs() < 0.01);
        assert_eq!(chunk.duration_ms(), 10);
    }

    #[test]
    fn barge_in_fires_once_after_the_grace_window() {
        let mut detector = BargeInDetector::new(0.05);
        detector.grace_ms = 100;
        assert!(detector.observe(50, 0.5).is_none());
        assert!(detector.observe(50, 0.01).is_none());
        let info = detector.observe(50, 0.5).expect("fired");
        assert_eq!(info.at_ms, 150);
        assert!(detector.observe(50, 0.5).is_none());
        assert!(detector.fired());
        detector.reset();
        assert!(!detector.fired());
    }
}
