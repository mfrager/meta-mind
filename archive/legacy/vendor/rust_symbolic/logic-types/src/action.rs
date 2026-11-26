//! The high-level LLM operation vocabulary (`full_system_design.md` §24.4).
//! These are the only operations an LLM may invoke; each maps to exactly one
//! deterministic CLI/API verb in the action layer (`logic-orchestrator`).

/// A high-level LLM operation (design §24.4). The LLM proposes semantic
/// intent; it never chooses solvers or backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LlmOperation {
    ConstructModel,
    ProposeHypothesis,
    FindExplanation,
    RequestCounterexample,
    FindMissingAssumptions,
    CompareModels,
    DesignExperiment,
}

impl LlmOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            LlmOperation::ConstructModel => "construct model",
            LlmOperation::ProposeHypothesis => "propose hypothesis",
            LlmOperation::FindExplanation => "find explanation",
            LlmOperation::RequestCounterexample => "request counterexample",
            LlmOperation::FindMissingAssumptions => "find missing assumptions",
            LlmOperation::CompareModels => "compare models",
            LlmOperation::DesignExperiment => "design experiment",
        }
    }

    /// All seven high-level operations.
    pub const ALL: [LlmOperation; 7] = [
        LlmOperation::ConstructModel,
        LlmOperation::ProposeHypothesis,
        LlmOperation::FindExplanation,
        LlmOperation::RequestCounterexample,
        LlmOperation::FindMissingAssumptions,
        LlmOperation::CompareModels,
        LlmOperation::DesignExperiment,
    ];
}
