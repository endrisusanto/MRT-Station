mod server;
mod state;

use std::sync::Arc;

use em_device::SimulatedDeviceProvider;
use state::AgentState;
use tracing_subscriber::EnvFilter;

#[cfg(unix)]
const DEFAULT_ENDPOINT: &str = "/tmp/em-station/agent.sock";
#[cfg(windows)]
const DEFAULT_ENDPOINT: &str = r"\\.\pipe\em-station-agent";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .json()
        .init();
    let endpoint = std::env::var("EM_AGENT_ENDPOINT").unwrap_or_else(|_| DEFAULT_ENDPOINT.into());
    let provider = Arc::new(SimulatedDeviceProvider::default());
    server::run(&endpoint, AgentState::new(provider)).await
}
