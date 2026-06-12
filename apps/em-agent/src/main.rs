mod config;
mod server;
mod state;

use std::sync::Arc;

use config::{AgentConfig, AgentMode};
use em_backend::{Authenticator, HttpAuthenticator, SimulatorAuthenticator};
use em_device::{
    DeviceInventory, DeviceProvider, ProductionDeviceProvider, SimulatedDeviceProvider,
};
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
    let authenticator: Arc<dyn Authenticator> = match config.mode {
        AgentMode::Simulator => Arc::new(SimulatorAuthenticator),
        AgentMode::Production => Arc::new(HttpAuthenticator::new(
            config
                .backend_url
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("EM_BACKEND_URL is required in production mode"))?,
            config.backend_timeout,
            config.backend_allow_http,
        )?),
    };
    let provider: Arc<dyn DeviceProvider> = match config.mode {
        AgentMode::Simulator => Arc::new(SimulatedDeviceProvider::default()),
        AgentMode::Production => Arc::new(ProductionDeviceProvider::new(DeviceInventory::load(
            &config.device_inventory,
        )?)),
    };
    server::run(&config, AgentState::new(provider, authenticator)).await
}
