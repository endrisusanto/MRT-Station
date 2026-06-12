mod config;
mod server;
mod state;

use std::sync::Arc;

use anyhow::bail;
use config::{AgentConfig, AgentMode};
use em_device::SimulatedDeviceProvider;
use state::AgentState;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();
    let config = AgentConfig::from_env()?;
    let provider = match config.mode {
        AgentMode::Simulator => Arc::new(SimulatedDeviceProvider::default()),
        AgentMode::Production => bail!(
            "production backend and device adapters are not configured; set EM_AGENT_MODE=simulator only for authorized development"
        ),
    };
    server::run(&config, AgentState::new(provider)).await
}
