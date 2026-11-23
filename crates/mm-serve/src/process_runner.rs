//! The production [`StepRunner`] for durable processes.
//!
//! Each CM step gets a fresh [`CmTools`] bound to the process's session and to
//! the template step's reviewed tool list, then runs through the same
//! Process → Verify cycle as a turn (without Interpret). Building the handler per
//! step is deliberate: a step cannot inherit a previous step's tool surface or
//! conversation state.

use std::sync::Arc;

use async_trait::async_trait;
use mm_agent::AgentRuntime;
use mm_cme::{StepPassRequest, ThreePassRunner};
use mm_data::process::ProcessError;
use mm_llm::LlmClient;
use mm_prompt::PromptStore;

use crate::cm_tools::CmTools;
use crate::state_service::StateService;

/// Executes process steps through the conversation engine.
pub struct ProcessStepRunner {
    state: Arc<StateService>,
    engine: Arc<mm_data::RdfEngine>,
    data: Arc<mm_data::DataService>,
    agent: Arc<AgentRuntime>,
    prompts: Arc<PromptStore>,
    client: Arc<dyn LlmClient>,
    model: Option<String>,
    three_pass: mm_cme::ThreePassConfig,
}

impl std::fmt::Debug for ProcessStepRunner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProcessStepRunner")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl ProcessStepRunner {
    /// Build the runner from the server's shared services.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: Arc<StateService>,
        engine: Arc<mm_data::RdfEngine>,
        data: Arc<mm_data::DataService>,
        agent: Arc<AgentRuntime>,
        prompts: Arc<PromptStore>,
        client: Arc<dyn LlmClient>,
        model: Option<String>,
        three_pass: mm_cme::ThreePassConfig,
    ) -> Self {
        Self {
            state,
            engine,
            data,
            agent,
            prompts,
            client,
            model,
            three_pass,
        }
    }

    fn tools(&self, session_id: &str, step_scope: &[String]) -> Arc<dyn mm_cme::ToolHandler> {
        let mut tools = CmTools::new(Arc::clone(&self.state))
            .with_data(Arc::clone(&self.data))
            .with_metamind(
                Arc::clone(&self.engine),
                Arc::clone(&self.data),
                Arc::clone(&self.prompts),
                Some(Arc::clone(&self.client)),
                self.model.clone(),
            )
            .with_agent(Arc::clone(&self.agent))
            .with_session(session_id)
            .with_pass_scope(mm_cme::PassScope::Process);
        if !step_scope.is_empty() {
            tools = tools.with_tool_scope(step_scope.to_vec());
        }
        Arc::new(tools)
    }
}

#[async_trait]
impl mm_process::StepRunner for ProcessStepRunner {
    async fn run_step(
        &self,
        request: mm_process::StepRequest,
    ) -> Result<mm_process::StepOutcome, ProcessError> {
        let tools = self.tools(&request.session_id, &request.tool_scope);
        let runner = ThreePassRunner::new(
            Arc::clone(&self.client),
            Arc::clone(&self.prompts),
            tools,
            self.three_pass.clone(),
        )
        .map_err(|error| {
            ProcessError::Unavailable(format!("three-pass runner unavailable: {error}"))
        })?;
        let run = runner
            .run_step(StepPassRequest {
                session_id: request.session_id,
                step_id: request.step_id,
                task: request.task,
                title: request.title,
                instructions: request.instructions,
                prior: request.prior,
                doc: request.doc,
                check: request.check,
                model: self.model.clone(),
            })
            .await
            .map_err(|error| ProcessError::Invalid(format!("step failed: {error}")))?;
        Ok(mm_process::StepOutcome {
            reply: run.reply,
            tokens_in: run.tokens_in,
            tokens_out: run.tokens_out,
            prompt_instance_id: run.prompt_instance_id,
        })
    }
}
