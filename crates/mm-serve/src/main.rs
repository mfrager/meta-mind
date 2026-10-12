//! `mm-serve` — local HTTP surface for the MetaMind Web Console.
//!
//! Binds loopback only (plan `01` §9), acquires `data/console.lock`, boots the
//! kernel the same way `mm-cli` does, and serves `GET /api/health`.
#![forbid(unsafe_code)]

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use mm_serve::controller::{load_controller_script, ControllerModel};
use mm_serve::routes::router;
use mm_serve::state::{AppState, ConsoleLock};

#[derive(Parser, Debug)]
#[command(
    name = "mm-serve",
    version,
    about = "Metamind Web Console server",
    long_about = "Local HTTP + SSE surface over the Metamind kernel. Binds loopback only."
)]
struct Cli {
    /// Configuration file (defaults to `config/metamind.toml`, or `$MM_CONFIG`).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use a self-contained data directory instead of the configured one.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Bind address. Only loopback is accepted.
    #[arg(long, default_value = "127.0.0.1")]
    bind: IpAddr,

    /// Listen port (`MM_UI_PORT` when the flag is omitted).
    #[arg(long, env = "MM_UI_PORT", default_value_t = 8899)]
    port: u16,

    /// Path to a MockClient script. Refuses to start when `OPENAI_API_KEY` is set.
    /// Playwright is the caller. `MM_CONTROLLER_SCRIPT` is the same path.
    #[arg(long, value_name = "PATH")]
    controller_script: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(err) = mm_core::load_project_env() {
        eprintln!("mm-serve: {err} ({})", err.code());
        return ExitCode::from(1);
    }
    let cli = Cli::parse();
    if let Err(err) = serve(cli).await {
        eprintln!("mm-serve: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

async fn serve(cli: Cli) -> Result<(), String> {
    require_loopback(cli.bind)?;
    let cfg = AppState::resolve_config(cli.config, cli.data_dir).map_err(|e| e.to_string())?;
    let lock = ConsoleLock::acquire(&cfg.store.data_dir).map_err(|e| e.to_string())?;
    let state = Arc::new(AppState::boot(cfg, lock).await.map_err(|e| e.to_string())?);
    install_controller_script(&state, cli.controller_script)?;
    let addr = SocketAddr::from((cli.bind, cli.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("cannot bind {addr}: {e}"))?;
    let actual = listener.local_addr().map_err(|e| e.to_string())?;
    eprintln!(
        "mm-serve listening on http://{actual} (data {})",
        state.cfg.store.data_dir.display()
    );
    axum::serve(listener, router(state))
        .await
        .map_err(|e| e.to_string())
}

fn install_controller_script(state: &AppState, flag: Option<PathBuf>) -> Result<(), String> {
    let path = flag.or_else(|| {
        std::env::var("MM_CONTROLLER_SCRIPT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
    });
    let Some(path) = path else {
        return Ok(());
    };
    let key = std::env::var("OPENAI_API_KEY").unwrap_or_default();
    if !key.trim().is_empty() {
        return Err(
            "refusing to start: OPENAI_API_KEY is set, so a controller script cannot replace the live provider"
                .into(),
        );
    }
    let client = load_controller_script(&path)?;
    *state
        .controller_model
        .lock()
        .map_err(|err| err.to_string())? = ControllerModel::Scripted(client);
    Ok(())
}

/// The console is a local operator surface. Remote binding is a non-goal.
fn require_loopback(bind: IpAddr) -> Result<(), String> {
    if bind.is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "refusing to bind {bind}: the console listens on loopback only"
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use super::require_loopback;

    #[test]
    fn only_loopback_binds_are_accepted() {
        assert!(require_loopback("127.0.0.1".parse::<IpAddr>().unwrap()).is_ok());
        assert!(require_loopback("::1".parse::<IpAddr>().unwrap()).is_ok());
        let err = require_loopback("0.0.0.0".parse::<IpAddr>().unwrap()).unwrap_err();
        assert!(err.contains("loopback"), "{err}");
    }
}
