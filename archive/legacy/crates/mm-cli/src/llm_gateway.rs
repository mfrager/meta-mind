//! Shared gateway construction for the CLI.
//!
//! Live commands journal into the persistent data layer; replay and fixture
//! commands use an in-memory layer so they cannot append to the operator's real
//! trace. The kernel's ledger, graph, and logger are reused in both cases.

use std::path::Path;
use std::sync::Arc;

use mm_core::MmError;
use mm_data::{initialize_data_layer, InitConfig, InitResult};
use mm_llm::config::LlmConfig;
use mm_llm::service::resolve_cache_dir;
use mm_llm::LlmClient;
use mm_llm_agentstream::{AgentstreamLlm, GatewayBuilder, GatewayParts, ProviderProfiles};

use crate::kernel::Kernel;

/// A gateway plus the data-layer and temp-directory owners it needs.
pub struct GatewaySession {
    /// The gateway every model call goes through.
    pub gateway: Arc<AgentstreamLlm>,
    _data: InitResult,
    _temp: Option<tempfile::TempDir>,
}

impl GatewaySession {
    /// Build a gateway over an in-memory journal.
    ///
    /// Used by replay and adversarial fixtures: they make no live call and must
    /// not write into the operator's agentstream.
    pub async fn in_memory(
        kernel: &Kernel,
        llm_cfg: &LlmConfig,
        root: &Path,
        client: Option<Arc<dyn LlmClient>>,
    ) -> Result<Self, MmError> {
        let temp = tempfile::tempdir()?;
        let data = open_data(Some(temp.path().to_path_buf()), true).await?;
        let gateway = build(kernel, llm_cfg, root, &data, client)?;
        Ok(Self {
            gateway,
            _data: data,
            _temp: Some(temp),
        })
    }

    /// Build a gateway that journals into the persistent data layer.
    ///
    /// Used by commands that make live calls, so the trace lands where the server
    /// and the rest of the kernel can read it.
    pub async fn live(kernel: &Kernel, llm_cfg: &LlmConfig, root: &Path) -> Result<Self, MmError> {
        let data = open_data(Some(kernel.cfg.store.data_dir.clone()), false).await?;
        let gateway = build(kernel, llm_cfg, root, &data, None)?;
        Ok(Self {
            gateway,
            _data: data,
            _temp: None,
        })
    }
}

async fn open_data(
    base_dir: Option<std::path::PathBuf>,
    in_memory: bool,
) -> Result<InitResult, MmError> {
    initialize_data_layer(InitConfig {
        base_dir,
        in_memory,
        ..InitConfig::default()
    })
    .await
    .map_err(|error| MmError::Internal(format!("agentstream data layer: {error}")))
}

fn build(
    kernel: &Kernel,
    llm_cfg: &LlmConfig,
    root: &Path,
    data: &InitResult,
    client: Option<Arc<dyn LlmClient>>,
) -> Result<Arc<AgentstreamLlm>, MmError> {
    let mut config = llm_cfg.clone();
    config.cache_dir = resolve_cache_dir(root, &config.cache_dir);
    let mut parts = GatewayParts::new(
        config,
        Arc::clone(&data.agentstream),
        kernel.sqlite.clone(),
        data.engine.handle().clone(),
        Arc::clone(&kernel.logger),
        ProviderProfiles::standard(),
    );
    if let Some(client) = client {
        parts = parts.with_injected_client(client);
    }
    GatewayBuilder::new(parts)
        .build()
        .map(Arc::new)
        .map_err(|error| MmError::Internal(format!("language-model gateway: {error}")))
}
