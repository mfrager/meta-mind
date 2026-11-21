# Voice Prosody Adaptation

> Dynamic TTS prosody control based on user affect, conversation context, and learned preferences.

---

## 1. Problem

Flat TTS output doesn't match:
- User's emotional state (frustrated → needs slower, empathetic)
- Conversation stakes (high-stakes → deliberate pace)
- Content type (joke → playful, correction → serious)
- Learned user preferences (some users want faster, slower, more/less expressive)

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    PROSODY PIPELINE                             │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  User Speech/Text                                               │
│       │                                                         │
│       ▼                                                         │
│  ┌─────────────────┐    affect + context    ┌──────────────┐   │
│  │ Affect Detector │ ──────────────────────▶ │ Prosody      │   │
│  │ (Multi-modal)   │                         │ Controller   │   │
│  └─────────────────┘                         └──────┬───────┘   │
│                                                     │           │
│                    ┌────────────────────────────────┼───────┐   │
│                    ▼                                ▼       ▼   │
│            ┌─────────────────┐            ┌──────────────┐ ┌──────────┐
│            │ User Baseline   │            │ Context      │ │ Content  │
│            │ (learned prefs) │            │ Rules        │ │ Analysis │
│            └─────────────────┘            └──────────────┘ └──────────┘
│                                                     │           │
│                                                     ▼           │
│                                            ┌──────────────┐     │
│                                            │ Prosody      │     │
│                                            │ Profile      │     │
│                                            └──────┬───────┘     │
│                                                  │             │
│                    ┌─────────────────────────────┼─────────────┘
│                    ▼                             ▼
│            ┌─────────────────┐            ┌──────────────┐
│            │ Tier 1/2: SSML  │            │ Tier 3:      │
│            │ Generator       │            │ Realtime     │
│            │                 │            │ Instructions │
│            └────────┬────────┘            └──────────────┘
│                     │
│                     ▼
│            ┌─────────────────┐
│            │ TTS Engine      │
│            │ (Piper, 11Labs, │
│            │  OpenAI, Azure) │
│            └────────┬────────┘
│                     │
│                     ▼
│              Audio Output
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. Affect Detection (Multi-Modal)

### 3.1 Signal Sources

```rust
struct AffectDetector {
    text_analyzer: TextAffectAnalyzer,
    audio_analyzer: Option<AudioAffectAnalyzer>,
    behavioral_analyzer: BehavioralAffectAnalyzer,
    fusion: AffectFusion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum UserAffect {
    Neutral,
    Frustrated,      // slower, lower pitch, longer pauses, empathetic
    Excited,         // faster, higher pitch, shorter pauses, energetic
    Uncertain,       // more hesitant, rising intonation, questioning
    Tired,           // slower, lower energy, monotone
    Focused,         // steady, precise, minimal filler
    Amused,          // playful, varied pitch, lighter
    Anxious,         // faster, higher pitch, breathy
    Satisfied,       // warm, steady, slightly slower
}

struct AffectSignal {
    affect: UserAffect,
    confidence: f32,           // 0.0–1.0
    source: SignalSource,
    features: HashMap<String, f32>,
}

enum SignalSource {
    Text,           // Lexical, syntactic, punctuation
    Audio,          // Pitch, rate, energy, voice quality
    Behavioral,     // Interruption, latency, correction patterns
    Explicit,       // User said "I'm frustrated", "slow down"
}
```

### 3.2 Text-Based Affect Analysis

```rust
struct TextAffectAnalyzer {
    frustration_lexicon: HashSet<String>,      // "ugh", "annoying", "broken", "again"
    excitement_lexicon: HashSet<String>,       // "awesome", "great", "love", "amazing"
    uncertainty_lexicon: HashSet<String>,      // "maybe", "not sure", "think", "guess"
    anxiety_lexicon: HashSet<String>,          // "urgent", "deadline", "panic", "help"
    satisfaction_lexicon: HashSet<String>,     // "perfect", "exactly", "works", "thanks"
    
    punctuation_patterns: Vec<PunctPattern>,
    syntactic_patterns: Vec<SyntacticPattern>,
}

impl TextAffectAnalyzer {
    fn analyze(&self, text: &str, context: &ConversationContext) -> Vec<AffectSignal> {
        let mut signals = Vec::new();
        let lower = text.to_lowercase();
        
        // Lexical signals
        if self.frustation_lexicon.iter().any(|w| lower.contains(w)) {
            signals.push(AffectSignal {
                affect: UserAffect::Frustrated,
                confidence: 0.7,
                source: SignalSource::Text,
                features: [("lexicon_match".into(), 1.0)].into(),
            });
        }
        
        if self.excitement_lexicon.iter().any(|w| lower.contains(w)) {
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.6,
                source: SignalSource::Text,
                features: [("lexicon_match".into(), 1.0)].into(),
            });
        }
        
        // Punctuation
        let exclamation_count = text.matches('!').count();
        let question_count = text.matches('?').count();
        let ellipsis_count = text.matches("...").count();
        
        if exclamation_count > 2 {
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.5 + (exclamation_count as f32 * 0.1).min(0.3),
                source: SignalSource::Text,
                features: [("exclamation_count".into(), exclamation_count as f32)].into(),
            });
        }
        
        if ellipsis_count > 0 {
            signals.push(AffectSignal {
                affect: UserAffect::Uncertain,
                confidence: 0.4 + (ellipsis_count as f32 * 0.1).min(0.3),
                source: SignalSource::Text,
                features: [("ellipsis_count".into(), ellipsis_count as f32)].into(),
            });
        }
        
        // CAPS detection (shouting)
        let words: Vec<&str> = text.split_whitespace().collect();
        let caps_words = words.iter().filter(|w| w.chars().all(|c| c.is_uppercase()) && w.len() > 2).count();
        if caps_words > 0 && caps_words as f32 / words.len() as f32 > 0.3 {
            signals.push(AffectSignal {
                affect: UserAffect::Frustrated,
                confidence: 0.8,
                source: SignalSource::Text,
                features: [("caps_ratio".into(), caps_words as f32 / words.len() as f32)].into(),
            });
        }
        
        // Explicit affect statements
        if let Some(explicit) = self.detect_explicit_affect(text) {
            signals.push(AffectSignal {
                affect: explicit.0,
                confidence: 0.95,
                source: SignalSource::Explicit,
                features: HashMap::new(),
            });
        }
        
        signals
    }
    
    fn detect_explicit_affect(&self, text: &str) -> Option<(UserAffect, String)> {
        let patterns = [
            (UserAffect::Frustrated, vec!["i'm frustrated", "this is frustrating", "so annoying"]),
            (UserAffect::Excited, vec!["i'm excited", "so excited", "can't wait"]),
            (UserAffect::Tired, vec!["i'm tired", "exhausted", "burned out"]),
            (UserAffect::Anxious, vec!["i'm worried", "stressed", "panicking"]),
            (UserAffect::Satisfied, vec!["perfect", "exactly what i needed", "works perfectly"]),
        ];
        
        let lower = text.to_lowercase();
        for (affect, phrases) in patterns {
            for phrase in phrases {
                if lower.contains(phrase) {
                    return Some((affect, phrase.into()));
                }
            }
        }
        None
    }
}
```

### 3.3 Audio-Based Affect Analysis (Tier 2/3)

```rust
struct AudioAffectAnalyzer {
    // Uses openSMILE features or similar
    pitch_extractor: PitchExtractor,
    energy_extractor: EnergyExtractor,
    rate_calculator: SpeechRateCalculator,
    voice_quality: VoiceQualityAnalyzer,
}

impl AudioAffectAnalyzer {
    fn analyze(&self, audio_chunk: &[f32], sample_rate: u32) -> Vec<AffectSignal> {
        let mut signals = Vec::new();
        
        // Pitch statistics
        let pitch_stats = self.pitch_extractor.analyze(audio_chunk, sample_rate);
        if pitch_stats.mean_hz > 200.0 { // High pitch
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.5,
                source: SignalSource::Audio,
                features: [("mean_pitch_hz".into(), pitch_stats.mean_hz)].into(),
            });
        } else if pitch_stats.mean_hz < 100.0 { // Low pitch
            signals.push(AffectSignal {
                affect: UserAffect::Tired,
                confidence: 0.4,
                source: SignalSource::Audio,
                features: [("mean_pitch_hz".into(), pitch_stats.mean_hz)].into(),
            });
        }
        
        // Pitch variance (excitement/stress)
        if pitch_stats.std_dev_hz > 50.0 {
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.6,
                source: SignalSource::Audio,
                features: [("pitch_std_dev".into(), pitch_stats.std_dev_hz)].into(),
            });
        }
        
        // Speech rate
        let rate = self.rate_calculator.analyze(audio_chunk, sample_rate);
        if rate.words_per_minute > 180 {
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.6,
                source: SignalSource::Audio,
                features: [("wpm".into(), rate.words_per_minute as f32)].into(),
            });
        } else if rate.words_per_minute < 100 {
            signals.push(AffectSignal {
                affect: UserAffect::Tired,
                confidence: 0.5,
                source: SignalSource::Audio,
                features: [("wpm".into(), rate.words_per_minute as f32)].into(),
            });
        }
        
        // Energy/volume
        let energy = self.energy_extractor.analyze(audio_chunk);
        if energy.rms_db > -10.0 { // Loud
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.4,
                source: SignalSource::Audio,
                features: [("rms_db".into(), energy.rms_db)].into(),
            });
        }
        
        // Voice quality (breathiness, creak)
        let quality = self.voice_quality.analyze(audio_chunk, sample_rate);
        if quality.breathiness > 0.5 {
            signals.push(AffectSignal {
                affect: UserAffect::Anxious,
                confidence: 0.5,
                source: SignalSource::Audio,
                features: [("breathiness".into(), quality.breathiness)].into(),
            })
        }
        
        signals
    }
}
```

### 3.4 Behavioral Affect Analysis

```rust
struct BehavioralAffectAnalyzer {
    window_turns: usize,
}

impl BehavioralAffectAnalyzer {
    fn analyze(&self, session: &Session, recent_turns: &[Turn]) -> Vec<AffectSignal> {
        let mut signals = Vec::new();
        
        // Interruption frequency → frustration
        let interruptions = recent_turns.iter().filter(|t| t.interrupted_prev_agent).count();
        if interruptions > 2 {
            signals.push(AffectSignal {
                affect: UserAffect::Frustrated,
                confidence: 0.6 + (interruptions as f32 * 0.1).min(0.3),
                source: SignalSource::Behavioral,
                features: [("interruption_count".into(), interruptions as f32)].into(),
            });
        }
        
        // Fast user responses → excitement/urgency
        let avg_response_latency = recent_turns.iter()
            .filter_map(|t| t.user_response_latency_ms)
            .sum::<u64>() / recent_turns.len().max(1) as u64;
        if avg_response_latency < 1500 {
            signals.push(AffectSignal {
                affect: UserAffect::Excited,
                confidence: 0.4,
                source: SignalSource::Behavioral,
                features: [("avg_response_latency_ms".into(), avg_response_latency as f32)].into(),
            });
        } else if avg_response_latency > 30000 {
            signals.push(AffectSignal {
                affect: UserAffect::Tired,
                confidence: 0.3,
                source: SignalSource::Behavioral,
                features: [("avg_response_latency_ms".into(), avg_response_latency as f32)].into(),
            });
        }
        
        // Repeated corrections → frustration
        let corrections = recent_turns.iter()
            .filter(|t| t.has_explicit_correction())
            .count();
        if corrections > 1 {
            signals.push(AffectSignal {
                affect: UserAffect::Frustrated,
                confidence: 0.7,
                source: SignalSource::Behavioral,
                features: [("correction_count".into(), corrections as f32)].into(),
            });
        }
        
        signals
    }
}
```

### 3.5 Multi-Modal Fusion

```rust
struct AffectFusion {
    source_weights: HashMap<SignalSource, f32>,
    temporal_decay: f32,  // How fast old signals fade
}

impl AffectFusion {
    fn fuse(&self, signals: Vec<AffectSignal>, previous_affect: Option<UserAffect>) -> FusedAffect {
        // Group by affect
        let mut affect_scores: HashMap<UserAffect, f32> = HashMap::new();
        
        for signal in signals {
            let weight = self.source_weights.get(&signal.source).copied().unwrap_or(1.0);
            let score = signal.confidence * weight;
            *affect_scores.entry(signal.affect).or_insert(0.0) += score;
        }
        
        // Apply temporal smoothing
        if let Some(prev) = previous_affect {
            if let Some(prev_score) = affect_scores.get(&prev) {
                affect_scores.insert(prev, prev_score * 0.7 + 0.3); // Momentum
            }
        }
        
        // Find dominant affect
        let dominant = affect_scores.iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(a, _)| *a)
            .unwrap_or(UserAffect::Neutral);
        
        let confidence = affect_scores.get(&dominant).copied().unwrap_or(0.0) / 
                        affect_scores.values().sum::<f32>().max(1.0);
        
        FusedAffect {
            dominant,
            confidence: confidence.min(1.0),
            all_scores: affect_scores,
            timestamp: now(),
        }
    }
}

struct FusedAffect {
    dominant: UserAffect,
    confidence: f32,
    all_scores: HashMap<UserAffect, f32>,
    timestamp: Timestamp,
}
```

---

## 4. Prosody Controller

### 4.1 Prosody Profiles

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProsodyProfile {
    // Rate
    rate_multiplier: f32,           // 0.5–2.0 (1.0 = normal)
    
    // Pitch
    pitch_shift_semitones: f32,     // -12 to +12
    pitch_range_multiplier: f32,    // 0.5–2.0 (expressiveness)
    
    // Volume
    volume_db: f32,                 // -10 to +10
    
    // Pausing
    pause_scaling: f32,             // 0.5–2.0
    pause_at_commas: bool,
    pause_at_periods: bool,
    pause_at_paragraphs: bool,
    
    // Emphasis
    emphasis_style: EmphasisStyle,
    emphasis_frequency: f32,        // 0.0–1.0
    
    // Breathing
    breathing_pattern: BreathingPattern,
    breath_pause_probability: f32,
    
    // Voice quality
    voice_quality: VoiceQuality,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum EmphasisStyle {
    Neutral,       // Even emphasis
    Deliberate,    // Clear, measured emphasis on key words
    Casual,        // Natural, conversational emphasis
    Expressive,    // Dramatic, varied emphasis
    Instructional, // Clear emphasis on actions/numbers
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum BreathingPattern {
    Natural,       // Occasional breaths
    Controlled,    // Regular, measured breaths
    Minimal,       // Few breaths (for speed)
    Expressive,    // Audible breaths for effect
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum VoiceQuality {
    Normal,
    Warm,          // Slightly lower pitch, softer
    Bright,        // Higher pitch, clearer
    Serious,       // Lower, flatter, authoritative
    Gentle,        // Soft, caring
    Energetic,     // Bright, fast, varied
}
```

### 4.2 Default Profiles per Affect

```rust
impl ProsodyProfile {
    fn for_affect(affect: UserAffect) -> Self {
        match affect {
            UserAffect::Frustrated => Self {
                rate_multiplier: 0.85,
                pitch_shift_semitones: -2.0,
                pitch_range_multiplier: 0.8,
                volume_db: -2.0,
                pause_scaling: 1.3,
                pause_at_commas: true,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Deliberate,
                emphasis_frequency: 0.3,
                breathing_pattern: BreathingPattern::Controlled,
                breath_pause_probability: 0.2,
                voice_quality: VoiceQuality::Serious,
            },
            
            UserAffect::Excited => Self {
                rate_multiplier: 1.15,
                pitch_shift_semitones: +3.0,
                pitch_range_multiplier: 1.3,
                volume_db: +2.0,
                pause_scaling: 0.8,
                pause_at_commas: false,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Expressive,
                emphasis_frequency: 0.7,
                breathing_pattern: BreathingPattern::Natural,
                breath_pause_probability: 0.1,
                voice_quality: VoiceQuality::Energetic,
            },
            
            UserAffect::Uncertain => Self {
                rate_multiplier: 0.9,
                pitch_shift_semitones: +1.0,
                pitch_range_multiplier: 1.1,
                volume_db: -1.0,
                pause_scaling: 1.2,
                pause_at_commas: true,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Casual,
                emphasis_frequency: 0.4,
                breathing_pattern: BreathingPattern::Natural,
                breath_pause_probability: 0.15,
                voice_quality: VoiceQuality::Gentle,
            },
            
            UserAffect::Tired => Self {
                rate_multiplier: 0.85,
                pitch_shift_semitones: -3.0,
                pitch_range_multiplier: 0.7,
                volume_db: -3.0,
                pause_scaling: 1.4,
                pause_at_commas: true,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Neutral,
                emphasis_frequency: 0.2,
                breathing_pattern: BreathingPattern::Minimal,
                breath_pause_probability: 0.25,
                voice_quality: VoiceQuality::Normal,
            },
            
            UserAffect::Focused => Self {
                rate_multiplier: 1.0,
                pitch_shift_semitones: 0.0,
                pitch_range_multiplier: 0.9,
                volume_db: 0.0,
                pause_scaling: 1.0,
                pause_at_commas: true,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Instructional,
                emphasis_frequency: 0.5,
                breathing_pattern: BreathingPattern::Controlled,
                breath_pause_probability: 0.1,
                voice_quality: VoiceQuality::Normal,
            },
            
            UserAffect::Amused => Self {
                rate_multiplier: 1.05,
                pitch_shift_semitones: +2.0,
                pitch_range_multiplier: 1.2,
                volume_db: +1.0,
                pause_scaling: 0.9,
                pause_at_commas: false,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Casual,
                emphasis_frequency: 0.5,
                breathing_pattern: BreathingPattern::Expressive,
                breath_pause_probability: 0.1,
                voice_quality: VoiceQuality::Warm,
            },
            
            UserAffect::Anxious => Self {
                rate_multiplier: 1.2,
                pitch_shift_semitones: +4.0,
                pitch_range_multiplier: 1.4,
                volume_db: +1.0,
                pause_scaling: 0.7,
                pause_at_commas: false,
                pause_at_periods: true,
                pause_at_paragraphs: false,
                emphasis_style: EmphasisStyle::Expressive,
                emphasis_frequency: 0.6,
                breathing_pattern: BreathingPattern::Natural,
                breath_pause_probability: 0.2,
                voice_quality: VoiceQuality::Bright,
            },
            
            UserAffect::Satisfied => Self {
                rate_multiplier: 0.95,
                pitch_shift_semitones: -1.0,
                pitch_range_multiplier: 1.0,
                volume_db: 0.0,
                pause_scaling: 1.1,
                pause_at_commas: true,
                pause_at_periods: true,
                pause_at_paragraphs: true,
                emphasis_style: EmphasisStyle::Casual,
                emphasis_frequency: 0.3,
                breathing_pattern: BreathingPattern::Natural,
                breath_pause_probability: 0.1,
                voice_quality: VoiceQuality::Warm,
            },
            
            UserAffect::Neutral => Self::default(),
        }
    }
}

impl Default for ProsodyProfile {
    fn default() -> Self {
        Self {
            rate_multiplier: 1.0,
            pitch_shift_semitones: 0.0,
            pitch_range_multiplier: 1.0,
            volume_db: 0.0,
            pause_scaling: 1.0,
            pause_at_commas: true,
            pause_at_periods: true,
            pause_at_paragraphs: true,
            emphasis_style: EmphasisStyle::Neutral,
            emphasis_frequency: 0.4,
            breathing_pattern: BreathingPattern::Natural,
            breath_pause_probability: 0.15,
            voice_quality: VoiceQuality::Normal,
        }
    }
}
```

### 4.3 Context & Baseline Adaptation

```rust
struct ProsodyController {
    affect_detector: AffectDetector,
    profiles: HashMap<UserAffect, ProsodyProfile>,
    user_baselines: HashMap<UserId, UserProsodyBaseline>,
    context_rules: Vec<ContextRule>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UserProsodyBaseline {
    user_id: UserId,
    avg_rate_multiplier: f32,           // Learned from user's speech
    avg_pitch_semitones: f32,
    typical_pause_ms: u32,
    preferred_emphasis: EmphasisStyle,
    learned_from_turns: u32,
    last_updated: Timestamp,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ContextRule {
    condition: ContextCondition,
    adjustments: ProsodyAdjustment,
    priority: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ContextCondition {
    HighStakes { threshold: f32 },
    Correction,
    Joke,
    TechnicalExplanation,
    CodeReading,
    LongForm { min_sentences: usize },
    FirstTurn,
    ReturningUser,
    UserExplicitRequest { request: String },
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct ProsodyAdjustment {
    rate_delta: f32,
    pitch_delta: f32,
    pause_scale_delta: f32,
    volume_delta: f32,
    emphasis_style_override: Option<EmphasisStyle>,
    voice_quality_override: Option<VoiceQuality>,
}

impl ProsodyController {
    fn compute_prosody(&self, text: &str, ctx: &ProsodyContext) -> ProsodyProfile {
        // 1. Base profile from affect
        let mut profile = self.profiles.get(&ctx.fused_affect.dominant)
            .cloned()
            .unwrap_or_default();
        
        // 2. Adapt to user baseline
        if let Some(baseline) = self.user_baselines.get(&ctx.user_id) {
            if baseline.learned_from_turns > 10 {
                // Blend with baseline (70% baseline, 30% affect profile)
                profile.rate_multiplier = baseline.avg_rate_multiplier * 0.7 + profile.rate_multiplier * 0.3;
                profile.pitch_shift_semitones = baseline.avg_pitch_semitones * 0.7 + profile.pitch_shift_semitones * 0.3;
                profile.pause_scaling = (baseline.typical_pause_ms as f32 / 500.0) * 0.7 + profile.pause_scaling * 0.3;
                profile.emphasis_style = baseline.preferred_emphasis;
            }
        }
        
        // 3. Apply context rules
        for rule in &self.context_rules {
            if rule.condition.matches(ctx) {
                profile.apply_adjustment(&rule.adjustments);
            }
        }
        
        // 4. Content-specific adjustments
        profile.apply_content_adjustments(text);
        
        // 5. Clamp to valid ranges
        profile.clamp();
        
        profile
    }
}

impl ProsodyProfile {
    fn apply_adjustment(&mut self, adj: &ProsodyAdjustment) {
        self.rate_multiplier = (self.rate_multiplier + adj.rate_delta).clamp(0.5, 2.0);
        self.pitch_shift_semitones = (self.pitch_shift_semitones + adj.pitch_delta).clamp(-12.0, 12.0);
        self.pause_scaling = (self.pause_scaling + adj.pause_scale_delta).clamp(0.5, 2.0);
        self.volume_db = (self.volume_db + adj.volume_delta).clamp(-10.0, 10.0);
        if let Some(style) = adj.emphasis_style_override { self.emphasis_style = style; }
        if let Some(quality) = adj.voice_quality_override { self.voice_quality = quality; }
    }
    
    fn apply_content_adjustments(&mut self, text: &str) {
        // Code blocks → slower, more precise
        if text.contains("```") || text.contains("`") {
            self.rate_multiplier *= 0.9;
            self.pause_scaling *= 1.1;
            self.emphasis_style = EmphasisStyle::Instructional;
        }
        
        // Questions → rising intonation (handled by TTS via SSML)
        if text.trim_end().ends_with('?') {
            self.pitch_range_multiplier *= 1.1;
        }
        
        // Exclamations → more expressive
        if text.matches('!').count() > 1 {
            self.emphasis_style = EmphasisStyle::Expressive;
            self.pitch_range_multiplier *= 1.1;
        }
        
        // Long sentences → more pauses
        let avg_sentence_len = text.split(['.', '?', '!']).map(|s| s.len()).sum::<usize>() 
            as f32 / text.matches(['.', '?', '!']).count().max(1) as f32;
        if avg_sentence_len > 100.0 {
            self.pause_scaling *= 1.2;
        }
    }
    
    fn clamp(&mut self) {
        self.rate_multiplier = self.rate_multiplier.clamp(0.5, 2.0);
        self.pitch_shift_semitones = self.pitch_shift_semitones.clamp(-12.0, 12.0);
        self.pitch_range_multiplier = self.pitch_range_multiplier.clamp(0.5, 2.0);
        self.volume_db = self.volume_db.clamp(-10.0, 10.0);
        self.pause_scaling = self.pause_scaling.clamp(0.5, 2.0);
        self.emphasis_frequency = self.emphasis_frequency.clamp(0.0, 1.0);
        self.breath_pause_probability = self.breath_pause_probability.clamp(0.0, 1.0);
    }
}
```

---

## 5. TTS Integration

### 5.1 Tier 1/2: SSML Generation

```rust
fn generate_ssml(text: &str, profile: &ProsodyProfile) -> String {
    let mut ssml = String::new();
    ssml.push_str(r#"<speak>"#);
    
    // Global prosody
    ssml.push_str(&format!(
        r#"<prosody rate="{:.0}%" pitch="{:+.1}st" volume="{:+.1}dB">"#,
        profile.rate_multiplier * 100.0,
        profile.pitch_shift_semitones,
        profile.volume_db
    ));
    
    // Process text into segments
    let segments = segment_for_ssml(text, profile);
    
    for segment in segments {
        match segment {
            TextSegment::Text(t) => ssml.push_str(&escape_xml(t)),
            TextSegment::Pause(duration_ms) => {
                ssml.push_str(&format!(r#"<break time="{}ms"/>"#, duration_ms));
            }
            TextSegment::Emphasis(t) => {
                ssml.push_str(&format!(r#"<emphasis level="strong">{}</emphasis>"#, escape_xml(t)));
            }
            TextSegment::Breath => {
                ssml.push_str(r#"<break time="300ms"/><prosody volume="-10dB">h</prosody>"#);
            }
        }
    }
    
    ssml.push_str("</prosody></speak>");
    ssml
}

enum TextSegment {
    Text(String),
    Pause(u32),
    Emphasis(String),
    Breath,
}

fn segment_for_ssml(text: &str, profile: &ProsodyProfile) -> Vec<TextSegment> {
    let mut segments = Vec::new();
    let sentences: Vec<&str> = text.split_inclusive(['.', '?', '!']).collect();
    
    for (i, sentence) in sentences.iter().enumerate() {
        let trimmed = sentence.trim();
        if trimmed.is_empty() { continue; }
        
        // Add emphasis to key words
        let emphasized = add_emphasis_markers(trimmed, profile);
        segments.push(TextSegment::Text(emphasized));
        
        // Pauses
        if i < sentences.len() - 1 {
            let pause_ms = match trimmed.chars().last() {
                Some('.') => base_pause_ms(profile, 600),
                Some('?') => base_pause_ms(profile, 400),
                Some('!') => base_pause_ms(profile, 500),
                _ => base_pause_ms(profile, 300),
            };
            segments.push(TextSegment::Pause(pause_ms));
        }
    }
    
    segments
}

fn add_emphasis_markers(text: &str, profile: &ProsodyProfile) -> String {
    match profile.emphasis_style {
        EmphasisStyle::Deliberate => {
            // Emphasize numbers, commands, key terms
            text.replace(
                |c: char| c.is_ascii_digit(), 
                |m| format!("<emphasis level=\"strong\">{}</emphasis>", m)
            )
        }
        EmphasisStyle::Instructional => {
            // Emphasize commands, flags, paths
            let mut result = text.to_string();
            for pattern in [r"\b\w+\.\w+\b", r"\-\-\w+", r"\/\S+"] {
                result = regex_replace(&result, pattern, |m| format!("<emphasis level=\"moderate\">{}</emphasis>", m));
            }
            result
        }
        EmphasisStyle::Expressive => {
            // Emphasize emotional words
            let emotional = ["amazing", "terrible", "perfect", "disaster", "brilliant", "awful"];
            let mut result = text.to_string();
            for word in emotional {
                result = result.replace(word, &format!("<emphasis level=\"strong\">{}</emphasis>", word));
            }
            result
        }
        _ => text.to_string(),
    }
}
```

### 5.2 Tier 3: Realtime Instructions

```rust
fn realtime_instructions(profile: &ProsodyProfile) -> String {
    format!(
        r#"Speak at {:.1}x normal rate. 
Pitch: {:+.1} semitones from baseline. 
Volume: {:+.1} dB.
Pause scaling: {:.1}x normal.
Emphasis style: {}.
Voice quality: {}.
Pause at commas: {}. Pause at periods: {}.
Breathing pattern: {}. Breath pause probability: {:.0}%.
Pitch range: {:.1}x normal."#,
        profile.rate_multiplier,
        profile.pitch_shift_semitones,
        profile.volume_db,
        profile.pause_scaling,
        format_emphasis_style(profile.emphasis_style),
        format_voice_quality(profile.voice_quality),
        profile.pause_at_commas,
        profile.pause_at_periods,
        format_breathing(profile.breathing_pattern),
        profile.breath_pause_probability * 100.0,
        profile.pitch_range_multiplier
    )
}
```

---

## 6. User Baseline Learning

```rust
struct BaselineLearner {
    min_turns_for_baseline: u32,
    update_interval_turns: u32,
}

impl BaselineLearner {
    fn update_from_audio(&self, baseline: &mut UserProsodyBaseline, audio_features: &AudioFeatures) {
        if baseline.learned_from_turns < self.min_turns_for_baseline {
            // Simple average
            baseline.avg_rate_multiplier = running_average(baseline.avg_rate_multiplier, audio_features.rate_multiplier, baseline.learned_from_turns);
            baseline.avg_pitch_semitones = running_average(baseline.avg_pitch_semitones, audio_features.pitch_semitones, baseline.learned_from_turns);
            baseline.typical_pause_ms = running_average_u32(baseline.typical_pause_ms, audio_features.typical_pause_ms, baseline.learned_from_turns);
        } else {
            // Exponential moving average
            let alpha = 0.1;
            baseline.avg_rate_multiplier = baseline.avg_rate_multiplier * (1.0 - alpha) + audio_features.rate_multiplier * alpha;
            baseline.avg_pitch_semitones = baseline.avg_pitch_semitones * (1.0 - alpha) + audio_features.pitch_semitones * alpha;
            baseline.typical_pause_ms = ((baseline.typical_pause_ms as f32 * (1.0 - alpha)) + (audio_features.typical_pause_ms as f32 * alpha)) as u32;
        }
        
        baseline.learned_from_turns += 1;
        baseline.last_updated = now();
    }
    
    fn update_from_text_preferences(&self, baseline: &mut UserProsodyBaseline, prefs: &UserPreferenceModel) {
        if let Some(p) = prefs.preferences.get("verbosity") {
            if p.confidence > 0.7 {
                // Verbose users tend to prefer slower rate
                baseline.avg_rate_multiplier = baseline.avg_rate_multiplier * 0.8 + (1.0 - p.value * 0.3) * 0.2;
            }
        }
        if let Some(p) = prefs.preferences.get("explanation_style") {
            if p.confidence > 0.7 {
                baseline.preferred_emphasis = match p.value {
                    v if v > 0.5 => EmphasisStyle::Expressive,
                    v if v > 0.0 => EmphasisStyle::Casual,
                    v if v > -0.5 => EmphasisStyle::Deliberate,
                    _ => EmphasisStyle::Instructional,
                };
            }
        }
    }
}
```

---

## 7. Configuration

```toml
[voice_prosody]
enabled = true

# Affect detection
affect_fusion_weights = { Text = 1.0, Audio = 1.5, Behavioral = 0.8, Explicit = 2.0 }
temporal_decay = 0.95

# Profiles
use_affect_profiles = true
use_baseline_adaptation = true
min_turns_for_baseline = 10
baseline_update_interval = 5

# Context rules
context_rules = [
  { condition = "HighStakes", threshold = 0.7, adjustments = { rate_delta = -0.1, pause_scale_delta = 0.2 }, priority = 10 },
  { condition = "Correction", adjustments = { rate_delta = -0.15, pitch_delta = -2.0, emphasis_style = "Deliberate" }, priority = 20 },
  { condition = "Joke", adjustments = { rate_delta = 0.1, pitch_delta = 2.0, voice_quality = "Warm" }, priority = 15 },
  { condition = "TechnicalExplanation", adjustments = { rate_delta = -0.1, emphasis_style = "Instructional" }, priority = 10 },
  { condition = "CodeReading", adjustments = { rate_delta = -0.15, pause_scale_delta = 0.2 }, priority = 10 },
]

# TTS-specific
ssml_emphasis_enabled = true
realtime_instructions_enabled = true
baseline_blend_ratio = 0.7  # 70% baseline, 30% affect profile

# Tier-specific
tier1_ssml_dialect = "standard"  # or "amazon", "google", "azure"
tier2_streaming_chunk_sentences = true
tier3_instructions_format = "openai"
```

---

## 8. Metrics

| Metric | Target |
|---|---|
| Affect detection accuracy (vs user labels) | > 80% |
| Prosody naturalness (MOS) | > 4.0 |
| Baseline convergence (turns) | < 20 |
| Context rule activation accuracy | > 85% |
| User satisfaction (prosody) | > 4.2/5 |

---

## 9. Example Scenarios

| Scenario | User Affect | Context | Prosody Result |
|---|---|---|---|
| Deploy failed | Frustrated | HighStakes | Slow (0.85x), lower pitch (-2st), long pauses (1.3x), deliberate emphasis |
| "Great job!" | Satisfied | - | Normal rate, warm quality, slight pause increase |
| "Why did this break?" | Uncertain | - | Slower (0.9x), gentle quality, rising intonation on question |
| "Deploy the service" | Focused | VoiceCommand | Normal rate, instructional emphasis, controlled breathing |
| "Here's a joke..." | Amused | Joke | Faster (1.05x), warm, expressive, lighter pauses |
| Reading code | - | CodeReading | Slow (0.85x), instructional, longer pauses |

---

*Prosody adaptation makes the agent sound **present and responsive** — not like a robot reading text, but like a colleague who hears you and adjusts their voice accordingly.*