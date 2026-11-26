//! `mm-serve` — the kept MetaMind server.
//!
//! One turn route, one append-only agentstream, one session list, and health.
//! The legacy controller, chat, CME, data, and LLM-activity surfaces are in
//! `archive/legacy/`.

#![forbid(unsafe_code)]

pub mod dto;
pub mod error;
pub mod metrics;
pub mod routes;
pub mod sse;
pub mod state;

pub use error::AppError;
pub use routes::router;
pub use state::AppState;
