# Implicit Preference Learning

> Continuous Bayesian inference of user preferences from behavioral signals — no questionnaires needed.

---

## 1. Problem

Questionnaires capture explicit preferences but:
- Miss nuanced, evolving patterns
- Create friction (users skip)
- Become stale quickly
- Can't capture "they prefer terse responses" from behavior

---

## 2. Preference Dimensions

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PreferenceDimension {
    id: String,
    name: String,
    range: (f32, f32),           // Always (-1.0, 1.0) normalized
    description: String,
    evidence_types: Vec<EvidenceType>,
    default: f32,                // 0.0 = neutral
    sensitivity: f32,            // How quickly evidence shifts value (0.1–1.0)
    decay_half_life_days: f32,   // Evidence decay
}

const PREFERENCE_DIMENSIONS: &[PreferenceDimension] = &[
    PreferenceDimension {
        id: "verbosity",
        name: "Verbosity",
        range: (-1.0, 1.0),      // -1 = ultra-terse, +1 = verbose
        description: "How much detail the user wants in responses",
        evidence_types: vec![
            EvidenceType::Interruption,
            EvidenceType::ExplicitComplaint,
            EvidenceType::AcceptancePattern,
            EvidenceType::ResponseLength,
        ],
        default: 0.0,
        sensitivity: 0.3,
        decay_half_life_days: 30.0,
    },
    PreferenceDimension {
        id: "technical_depth",
        name: "Technical Depth",
        range: (-1.0, 1.0),      // -1 = high-level, +1 = deep technical
        description: "How much implementation detail to include",
        evidence_types: vec![
            EvidenceType::DetailRequest,
            EvidenceType::AcceptancePattern,
            EvidenceType::ExplicitRequest,
            EvidenceType::DomainVocabulary,
        ],
        default: 0.2,            // Slightly technical by default
        sensitivity: 0.4,
        decay_half_life_days: 60.0,
    },
    PreferenceDimension {
        id: "initiative",
        name: "Initiative Level",
        range: (-1.0, 1.0),      // -1 = wait for instructions, +1 = proactive
        description: "How much the agent should act without being asked",
        evidence_types: vec![
            EvidenceType::ProactiveAcceptance,
            EvidenceType::ProactiveRejection,
            EvidenceType::ExplicitRequest,
            EvidenceType::TaskDelegation,
        ],
        default: 0.1,
        sensitivity: 0.25,
        decay_half_life_days: 90.0,
    },
    PreferenceDimension {
        id: "formality",
        name: "Formality",
        range: (-1.0, 1.0),      // -1 = casual, +1 = formal
        description: "Communication tone and register",
        evidence_types: vec![
            EvidenceType::CasualLanguage,
            EvidenceType::FormalLanguage,
            EvidenceType::ToneCorrection,
            EvidenceType::EmojiUsage,
        ],
        default: -0.2,           // Slightly casual default
        sensitivity: 0.2,
        decay_half_life_days: 120.0,
    },
    PreferenceDimension {
        id: "code_verbosity",
        name: "Code Verbosity",
        range: (-1.0, 1.0),      // -1 = minimal diffs, +1 = full files with comments
        description: "How much code context to show",
        evidence_types: vec![
            EvidenceType::CodeAcceptance,
            EvidenceType::ExplicitRequest,
            EvidenceType::DiffSizeReaction,
        ],
        default: -0.3,
        sensitivity: 0.3,
        decay_half_life_days: 60.0,
    },
    PreferenceDimension {
        id: "explanation_style",
        name: "Explanation Style",
        range: (-1.0, 1.0),      // -1 = "just do it", +1 = "teach me"
        description: "Whether user wants reasoning explained or just results",
        evidence_types: vec![
            EvidenceType::WhyQuestion,
            EvidenceType::ExplanationAcceptance,
            EvidenceType::ExplanationRejection,
            EvidenceType::TeachingRequest,
        ],
        default: 0.0,
        sensitivity: 0.25,
        decay_half_life_days: 90.0,
    },
];
```

---

## 3. Evidence Signals

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
enum PreferenceSignal {
    // Verbosity
    InterruptedLongReply { chunk_index: usize, total_chunks: usize },
    AcceptedShortReply { sentences: u8 },
    ExplicitComplaint { text: String, dimension: String },
    ResponseLength { user_chars: usize, agent_chars: usize },
    
    // Technical Depth
    DetailRequest { topic: String, explicit: bool },
    AcceptedHighLevelSummary,
    ExplicitDepthRequest { level: TechnicalLevel },
    DomainVocabularyUsed { terms: Vec<String> },
    
    // Initiative
    ProactiveAcceptance { item_type: String, acted_upon: bool },
    ProactiveRejection { item_type: String, reason: Option<String> },
    ExplicitInitiativeRequest { level: InitiativeLevel },
    TaskDelegation { task: String },
    
    // Formality
    CasualLanguage { examples: Vec<String> },
    FormalLanguage { examples: Vec<String> },
    ToneCorrection { expected: String, actual: String },
    EmojiUsage { count: usize, context: String },
    
    // Code Verbosity
    CodeAcceptance { lines: usize, type: CodeType },
    ExplicitCodeRequest { style: CodeStyle },
    DiffSizeReaction { lines: usize, accepted: bool },
    
    // Explanation Style
    WhyQuestion { topic: String },
    ExplanationAcceptance { depth: ExplanationDepth },
    ExplanationRejection { reason: String },
    TeachingRequest { topic: String },
}

#[derive(Clone, Debug)]
enum TechnicalLevel { HighLevel, Moderate, Deep, Expert }
#[derive(Clone, Debug)]
enum InitiativeLevel { Passive, Reactive, Proactive, Autonomous }
#[derive(Clone, Debug)]
enum CodeType { Diff, FullFile, Snippet, Config, Diagram }
#[derive(Clone, Debug)]
enum CodeStyle { Minimal, Standard, Verbose, Annotated }
#[derive(Clone, Debug)]
enum ExplanationDepth { None, Summary, Detailed, Tutorial }
```

---

## 4. Evidence Extraction

```rust
struct EvidenceExtractor {
    // Lightweight classifiers for each signal type
    interruption_detector: InterruptionDetector,
    length_analyzer: LengthAnalyzer,
    vocabulary_analyzer: VocabularyAnalyzer,
    tone_analyzer: ToneAnalyzer,
    code_analyzer: CodeAnalyzer,
}

impl EvidenceExtractor {
    fn extract_from_turn(&self, turn: &Turn, prev_agent_turn: Option<&Turn>, context: &Context) 
        -> Vec<PreferenceEvidence> 
    {
        let mut evidence = Vec::new();
        
        // Interruption → verbosity preference
        if turn.interrupted_prev_agent {
            if let Some(prev) = prev_agent_turn {
                evidence.push(PreferenceEvidence {
                    dimension: "verbosity".into(),
                    signal: PreferenceSignal::InterruptedLongReply {
                        chunk_index: prev.current_chunk_index,
                        total_chunks: prev.total_chunks,
                    },
                    weight: 0.9,
                    turn_id: turn.id,
                    timestamp: turn.timestamp,
                });
            }
        }
        
        // User response length vs agent → verbosity
        if let Some(agent) = prev_agent_turn {
            let ratio = turn.content.chars().count() as f32 / agent.content.chars().count() as f32;
            if ratio < 0.3 && agent.content.chars().count() > 200 {
                evidence.push(PreferenceEvidence {
                    dimension: "verbosity".into(),
                    signal: PreferenceSignal::AcceptedShortReply { 
                        sentences: count_sentences(&turn.content) as u8 
                    },
                    weight: 0.4,
                    turn_id: turn.id,
                    timestamp: turn.timestamp,
                });
            }
        }
        
        // "Why?" questions → explanation style
        if self.is_why_question(&turn.content) {
            evidence.push(PreferenceEvidence {
                dimension: "explanation_style".into(),
                signal: PreferenceSignal::WhyQuestion { 
                    topic: self.extract_topic(&turn.content) 
                },
                weight: 0.7,
                turn_id: turn.id,
                timestamp: turn.timestamp,
            });
        }
        
        // "Detail on X" → technical depth
        if let Some(topic) = self.extract_detail_request(&turn.content) {
            evidence.push(PreferenceEvidence {
                dimension: "technical_depth".into(),
                signal: PreferenceSignal::DetailRequest { 
                    topic, 
                    explicit: true 
                },
                weight: 0.8,
                turn_id: turn.id,
                timestamp: turn.timestamp,
            });
        }
        
        // Proactive item feedback
        if let Some(feedback) = turn.proactive_feedback {
            match feedback {
                ProactiveFeedback::Accepted { acted_upon } => {
                    evidence.push(PreferenceEvidence {
                        dimension: "initiative".into(),
                        signal: PreferenceSignal::ProactiveAcceptance {
                            item_type: feedback.item_type,
                            acted_upon,
                        },
                        weight: if acted_upon { 0.6 } else { 0.3 },
                        turn_id: turn.id,
                        timestamp: turn.timestamp,
                    });
                }
                ProactiveFeedback::Rejected { reason } => {
                    evidence.push(PreferenceEvidence {
                        dimension: "initiative".into(),
                        signal: PreferenceSignal::ProactiveRejection {
                            item_type: feedback.item_type,
                            reason,
                        },
                        weight: 0.8,
                        turn_id: turn.id,
                        timestamp: turn.timestamp,
                    });
                }
                _ => {}
            }
        }
        
        // Tone correction
        if let Some(correction) = self.detect_tone_correction(&turn.content) {
            evidence.push(PreferenceEvidence {
                dimension: "formality".into(),
                signal: PreferenceSignal::ToneCorrection {
                    expected: correction.expected,
                    actual: correction.actual,
                },
                weight: 0.9,
                turn_id: turn.id,
                timestamp: turn.timestamp,
            });
        }
        
        evidence
    }
}
```

---

## 5. Bayesian Preference Model

```rust
struct UserPreferenceModel {
    user_id: UserId,
    preferences: HashMap<String, InferredPreference>,
    evidence_log: VecDeque<PreferenceEvidence>,  // Rolling window
    last_updated: Timestamp,
    total_evidence_count: u64,
}

struct InferredPreference {
    dimension: String,
    value: f32,                     // -1.0 to 1.0
    confidence: f32,                // 0.0 to 1.0
    evidence_count: u32,
    recent_trend: TrendDirection,
    last_evidence_at: Timestamp,
}

#[derive(Clone, Copy)]
enum TrendDirection { Stable, Increasing, Decreasing }

impl UserPreferenceModel {
    fn update(&mut self, evidence: &PreferenceEvidence) {
        let dim = &evidence.dimension;
        let dim_config = PREFERENCE_DIMENSIONS.iter().find(|d| d.id == dim).unwrap();
        
        let prior = self.preferences.get(dim).cloned()
            .unwrap_or(InferredPreference {
                dimension: dim.clone(),
                value: dim_config.default,
                confidence: 0.0,
                evidence_count: 0,
                recent_trend: TrendDirection::Stable,
                last_evidence_at: now(),
            });
        
        // Convert signal to value
        let (signal_value, signal_strength) = self.signal_to_value(&evidence.signal, dim_config);
        let weight = evidence.weight * signal_strength * dim_config.sensitivity;
        
        // Bayesian update: posterior ∝ prior × likelihood
        let prior_weight = prior.confidence * prior.evidence_count as f32;
        let new_weight = prior_weight + weight;
        
        let new_value = if new_weight > 0.0 {
            (prior.value * prior_weight + signal_value * weight) / new_weight
        } else {
            dim_config.default
        };
        
        let new_confidence = (new_weight / (prior.evidence_count as f32 + 1.0)).min(1.0);
        let new_count = prior.evidence_count + 1;
        
        // Trend detection
        let trend = if (new_value - prior.value).abs() > 0.1 {
            if new_value > prior.value { TrendDirection::Increasing } else { TrendDirection::Decreasing }
        } else {
            TrendDirection::Stable
        };
        
        self.preferences.insert(dim.clone(), InferredPreference {
            dimension: dim.clone(),
            value: new_value.clamp(-1.0, 1.0),
            confidence: new_confidence,
            evidence_count: new_count,
            recent_trend: trend,
            last_evidence_at: now(),
        });
        
        // Log evidence (with decay)
        self.evidence_log.push_back(evidence.clone());
        self.prune_old_evidence();
        
        self.last_updated = now();
        self.total_evidence_count += 1;
    }
    
    fn signal_to_value(&self, signal: &PreferenceSignal, dim: &PreferenceDimension) -> (f32, f32) {
        match signal {
            // Verbosity signals
            PreferenceSignal::InterruptedLongReply { .. } => (-0.8, 0.9),
            PreferenceSignal::AcceptedShortReply { .. } => (-0.3, 0.4),
            PreferenceSignal::ExplicitComplaint { dimension, .. } if dimension == "verbosity" => (-0.9, 0.9),
            PreferenceSignal::ResponseLength { user_chars, agent_chars } => {
                let ratio = *user_chars as f32 / *agent_chars as f32;
                if ratio < 0.2 { (-0.5, 0.3) } else if ratio > 2.0 { (0.5, 0.3) } else { (0.0, 0.1) }
            }
            
            // Technical depth
            PreferenceSignal::DetailRequest { explicit, .. } => (0.6, if *explicit { 0.8 } else { 0.4 }),
            PreferenceSignal::AcceptedHighLevelSummary => (-0.4, 0.3),
            PreferenceSignal::ExplicitDepthRequest { level } => {
                match level {
                    TechnicalLevel::HighLevel => (-0.8, 0.9),
                    TechnicalLevel::Moderate => (0.0, 0.5),
                    TechnicalLevel::Deep => (0.7, 0.8),
                    TechnicalLevel::Expert => (1.0, 0.9),
                }
            }
            PreferenceSignal::DomainVocabularyUsed { terms } => {
                let score = (terms.len() as f32 / 10.0).min(1.0);
                (score * 0.8, 0.3)
            }
            
            // Initiative
            PreferenceSignal::ProactiveAcceptance { acted_upon, .. } => {
                if *acted_upon { (0.5, 0.6) } else { (0.2, 0.3) }
            }
            PreferenceSignal::ProactiveRejection { .. } => (-0.6, 0.8),
            PreferenceSignal::ExplicitInitiativeRequest { level } => {
                match level {
                    InitiativeLevel::Passive => (-1.0, 0.9),
                    InitiativeLevel::Reactive => (-0.3, 0.7),
                    InitiativeLevel::Proactive => (0.5, 0.8),
                    InitiativeLevel::Autonomous => (0.9, 0.9),
                }
            }
            PreferenceSignal::TaskDelegation { .. } => (0.6, 0.7),
            
            // Formality
            PreferenceSignal::CasualLanguage { .. } => (-0.4, 0.3),
            PreferenceSignal::FormalLanguage { .. } => (0.4, 0.3),
            PreferenceSignal::ToneCorrection { expected, .. } => {
                if expected.contains("casual") { (-0.8, 0.9) } else { (0.8, 0.9) }
            }
            PreferenceSignal::EmojiUsage { count, .. } => {
                let score = (*count as f32 / 5.0).min(1.0);
                (-score * 0.6, 0.2)
            }
            
            // Code verbosity
            PreferenceSignal::CodeAcceptance { lines, .. } => {
                if *lines < 20 { (-0.4, 0.3) } else if *lines > 100 { (0.4, 0.3) } else { (0.0, 0.1) }
            }
            PreferenceSignal::ExplicitCodeRequest { style } => {
                match style {
                    CodeStyle::Minimal => (-0.8, 0.9),
                    CodeStyle::Standard => (0.0, 0.5),
                    CodeStyle::Verbose => (0.6, 0.8),
                    CodeStyle::Annotated => (0.8, 0.8),
                }
            }
            PreferenceSignal::DiffSizeReaction { lines, accepted } => {
                if *accepted && *lines > 50 { (0.3, 0.4) }
                else if !*accepted && *lines > 50 { (-0.5, 0.6) }
                else { (0.0, 0.1) }
            }
            
            // Explanation style
            PreferenceSignal::WhyQuestion { .. } => (0.5, 0.6),
            PreferenceSignal::ExplanationAcceptance { depth } => {
                match depth {
                    ExplanationDepth::None => (-0.8, 0.7),
                    ExplanationDepth::Summary => (0.0, 0.4),
                    ExplanationDepth::Detailed => (0.5, 0.6),
                    ExplanationDepth::Tutorial => (0.8, 0.8),
                }
            }
            PreferenceSignal::ExplanationRejection { .. } => (-0.7, 0.8),
            PreferenceSignal::TeachingRequest { .. } => (0.7, 0.8),
            
            _ => (0.0, 0.0),
        }
    }
    
    fn prune_old_evidence(&mut self) {
        let cutoff = now() - Duration::days(365); // Keep 1 year
        while let Some(front) = self.evidence_log.front() {
            if front.timestamp < cutoff {
                self.evidence_log.pop_front();
            } else { break; }
        }
    }
}
```

---

## 6. Evidence Decay & Forgetting

```rust
impl UserPreferenceModel {
    // Apply time decay to all preferences periodically
    fn apply_decay(&mut self) {
        for (dim_id, pref) in self.preferences.iter_mut() {
            let dim_config = PREFERENCE_DIMENSIONS.iter().find(|d| d.id == dim_id).unwrap();
            let days_since = (now() - pref.last_evidence_at).as_secs_f32() / 86400.0;
            
            if days_since > dim_config.decay_half_life_days {
                // Exponential decay toward default
                let half_lives = days_since / dim_config.decay_half_life_days;
                let decay_factor = 0.5_f32.powf(half_lives);
                
                pref.value = pref.value * decay_factor + dim_config.default * (1.0 - decay_factor);
                pref.confidence *= decay_factor;
            }
        }
    }
}
```

---

## 7. Application in CME

### 7.1 Turn Plan Adaptation

```rust
impl TurnPlanner {
    fn apply_preferences(&self, plan: &mut TurnPlan, prefs: &UserPreferenceModel) {
        // Verbosity → chunk structure
        if let Some(p) = prefs.preferences.get("verbosity") {
            if p.confidence > 0.6 {
                if p.value < -0.5 {
                    // Terse: fewer chunks, fewer sentences
                    plan.chunks.iter_mut().for_each(|c| {
                        c.max_sentences = 1;
                        c.min_sentences = 1;
                    });
                    plan.max_chunks = 2;
                } else if p.value > 0.5 {
                    // Verbose: more detail allowed
                    plan.chunks.iter_mut().for_each(|c| {
                        c.max_sentences = 3;
                    });
                    plan.max_chunks = 5;
                }
            }
        }
        
        // Technical depth → directive detail
        if let Some(p) = prefs.preferences.get("technical_depth") {
            if p.confidence > 0.6 {
                let detail_suffix = if p.value > 0.5 {
                    " Include technical details, command examples, and implementation notes."
                } else if p.value < -0.3 {
                    " Keep it high-level. No commands unless asked."
                } else { "" };
                
                plan.chunks.iter_mut()
                    .filter(|c| c.role == ChunkRole::Finding || c.role == ChunkRole::NextStep)
                    .for_each(|c| c.directive.push_str(detail_suffix));
            }
        }
        
        // Initiative → proactive frequency
        if let Some(p) = prefs.preferences.get("initiative") {
            if p.confidence > 0.5 {
                // Base 0.2, range 0.1–0.4
                config.proactive_frequency = 0.15 + 0.25 * (p.value + 1.0) / 2.0;
            }
        }
        
        // Formality → persona adjustment
        if let Some(p) = prefs.preferences.get("formality") {
            if p.confidence > 0.5 {
                // Inject into system prompt
                plan.metadata.formality_adjustment = p.value;
            }
        }
        
        // Explanation style → chunk roles
        if let Some(p) = prefs.preferences.get("explanation_style") {
            if p.confidence > 0.6 && p.value < -0.5 {
                // "Just do it" mode: remove explanation chunks
                plan.chunks.retain(|c| c.role != ChunkRole::Acknowledge || c.directive.contains("technical"));
                // Ensure NextStep is direct
                if let Some(next) = plan.chunks.iter_mut().find(|c| c.role == ChunkRole::NextStep) {
                    next.directive = "State the next action in one sentence.".into();
                }
            }
        }
    }
}
```

### 7.2 Real-Time Preference Display (Debug)

```rust
// For debugging: expose current inferred preferences
fn format_preferences(model: &UserPreferenceModel) -> String {
    let mut lines = vec!["Inferred Preferences:"];
    for pref in model.preferences.values() {
        let bar = "█".repeat((pref.confidence * 10.0) as usize);
        let direction = if pref.value > 0.0 { "▶" } else if pref.value < 0.0 { "◀" } else { "●" };
        lines.push(format!(
            "  {:20} {} {:>5.2} {} [{}]",
            pref.dimension, direction, pref.value.abs(), bar, pref.confidence
        ));
    }
    lines.join("\n")
}

/* Output example:
Inferred Preferences:
  verbosity            ◀ -0.72 ███████ [0.89]
  technical_depth      ▶  0.41 ████    [0.72]
  initiative           ▶  0.23 ██      [0.54]
  formality            ◀ -0.18 █       [0.41]
  code_verbosity       ◀ -0.55 █████   [0.78]
  explanation_style    ▶  0.33 ███     [0.61]
*/
```

---

## 8. Bootstrapping from Questionnaires

```rust
impl UserPreferenceModel {
    fn seed_from_questionnaire(&mut self, answers: &QuestionnaireAnswers) {
        // Map questionnaire answers to initial preferences
        if let Some(style) = answers.get("communication_style") {
            match style.as_str() {
                "terse" => self.set_prior("verbosity", -0.7, 0.8),
                "conversational" => self.set_prior("verbosity", 0.0, 0.6),
                "teaching" => {
                    self.set_prior("explanation_style", 0.8, 0.8);
                    self.set_prior("verbosity", 0.3, 0.6);
                }
                "direct" => {
                    self.set_prior("verbosity", -0.5, 0.7);
                    self.set_prior("formality", 0.3, 0.6);
                }
                _ => {}
            }
        }
        
        if let Some(role) = answers.get("role") {
            match role.as_str() {
                "engineer" | "developer" => {
                    self.set_prior("technical_depth", 0.6, 0.7);
                    self.set_prior("code_verbosity", -0.2, 0.5);
                }
                "researcher" => {
                    self.set_prior("technical_depth", 0.4, 0.6);
                    self.set_prior("explanation_style", 0.4, 0.5);
                }
                "manager" | "pm" => {
                    self.set_prior("technical_depth", -0.3, 0.6);
                    self.set_prior("verbosity", -0.2, 0.5);
                    self.set_prior("explanation_style", 0.2, 0.5);
                }
                _ => {}
            }
        }
    }
    
    fn set_prior(&mut self, dim: &str, value: f32, confidence: f32) {
        if let Some(pref) = self.preferences.get_mut(dim) {
            // Blend with existing
            let new_weight = confidence;
            let old_weight = pref.confidence;
            let total = new_weight + old_weight;
            pref.value = (pref.value * old_weight + value * new_weight) / total;
            pref.confidence = (pref.confidence * old_weight + confidence * new_weight) / total;
        } else {
            self.preferences.insert(dim.into(), InferredPreference {
                dimension: dim.into(),
                value,
                confidence,
                evidence_count: 0,
                recent_trend: TrendDirection::Stable,
                last_evidence_at: now(),
            });
        }
    }
}
```

---

## 9. Configuration

```toml
[preference_learning]
enabled = true

# Evidence collection
min_evidence_for_application = 3
max_evidence_window_days = 365
evidence_decay_enabled = true

# Inference
bayesian_update = true
minimum_confidence_to_apply = 0.5
trend_detection_threshold = 0.1

# Dimension defaults (overridable per project)
defaults = {
    verbosity = 0.0
    technical_depth = 0.2
    initiative = 0.1
    formality = -0.2
    code_verbosity = -0.3
    explanation_style = 0.0
}

# Questionnaire seeding
seed_from_questionnaire = true
questionnaire_weight = 0.7

# Debug
expose_preferences_in_debug = true
log_preference_changes = true
```

---

## 10. Metrics

| Metric | Target |
|---|---|
| Preference stability (weekly) | > 0.8 correlation |
| Questionnaire agreement | > 0.75 |
| Explicit correction rate | < 5% of interactions |
| Preference-driven adaptation latency | < 1 turn |
| Cold-start accuracy (after questionnaire) | > 0.7 |

---

*Implicit preference learning turns every interaction into a calibration signal — the agent gets better at being *your* agent without you ever filling a form.*