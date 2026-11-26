//! `mm-serve` — local HTTP surface for the three-pass turn pipeline.
#![forbid(unsafe_code)]

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use mm_core::Config;
use mm_serve::router;
use mm_serve::state::AppState;

#[derive(Parser, Debug)]
#[command(
    name = "mm-serve",
    version,
    about = "Metamind turn server",
    long_about = "Local HTTP + SSE surface over the three-pass turn pipeline. Binds loopback only."
)]
struct Cli {
    /// Configuration file (defaults to `config/server.toml`).
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Use a self-contained data directory instead of the configured one.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Bind address. Only loopback is accepted.
    #[arg(long)]
    bind: Option<IpAddr>,

    /// Listen port.
    #[arg(long, env = "MM_UI_PORT")]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> ExitCode {
    if let Err(error) = mm_core::load_project_env() {
        eprintln!("mm-serve: {error} ({})", error.code());
        return ExitCode::from(1);
    }
    let cli = Cli::parse();
    if let Err(error) = serve(cli).await {
        eprintln!("mm-serve: {error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

async fn serve(cli: Cli) -> Result<(), String> {
    let mut cfg = Config::load(
        cli.config
            .clone()
            .unwrap_or_else(|| PathBuf::from(mm_core::config::DEFAULT_CONFIG_PATH)),
    )
    .map_err(|error| error.to_string())?;
    if let Some(data_dir) = cli.data_dir {
        cfg.data_dir = data_dir;
    }
    if let Some(bind) = cli.bind {
        cfg.bind = bind.to_string();
    }
    if let Some(port) = cli.port {
        cfg.port = port;
    }
    let bind: IpAddr = cfg
        .bind
        .parse()
        .map_err(|error| format!("invalid bind address {:?}: {error}", cfg.bind))?;
    require_loopback(bind)?;
    let state = Arc::new(AppState::boot(cfg).map_err(|error| error.to_string())?);
    let addr = SocketAddr::from((bind, state.cfg.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("cannot bind {addr}: {error}"))?;
    let actual = listener.local_addr().map_err(|error| error.to_string())?;
    eprintln!(
        "mm-serve listening on http://{actual} (data {})",
        state.cfg.data_dir.display()
    );
    axum::serve(listener, router(state))
        .await
        .map_err(|error| error.to_string())
}

/// The server is a local operator surface. Remote binding is a non-goal.
fn require_loopback(bind: IpAddr) -> Result<(), String> {
    if bind.is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "refusing to bind {bind}: the server listens on loopback only"
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
        let error = require_loopback("0.0.0.0".parse::<IpAddr>().unwrap()).unwrap_err();
        assert!(error.contains("loopback"), "{error}");
    }
}
