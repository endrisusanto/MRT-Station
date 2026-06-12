use anyhow::Context;
use em_core::{AgentResponse, AppError, ErrorCode};
use em_protocol::{Envelope, read_frame, write_frame};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{info, warn};

use crate::state::AgentState;

async fn serve_connection<S>(mut stream: S, state: AgentState)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let envelope = match read_frame(&mut stream).await {
        Ok(envelope) => envelope,
        Err(error) => {
            warn!(%error, "rejected invalid IPC frame");
            return;
        }
    };
    let correlation_id = envelope.correlation_id.clone();
    let response = match envelope.decode_request() {
        Ok(request) => state.handle(request).await,
        Err(error) => AgentResponse::Error(AppError::new(
            ErrorCode::InvalidRequest,
            format!("Invalid IPC request: {error}"),
        )),
    };
    let Ok(response) = Envelope::response(correlation_id, &response) else {
        warn!("failed to encode IPC response");
        return;
    };
    if let Err(error) = write_frame(&mut stream, &response).await {
        warn!(%error, "failed to write IPC response");
    }
}

#[cfg(unix)]
pub async fn run(endpoint: &str, state: AgentState) -> anyhow::Result<()> {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};
    use tokio::net::UnixListener;

    let path = Path::new(endpoint);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if path.exists() {
        fs::remove_file(path).with_context(|| format!("failed to remove stale {endpoint}"))?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("failed to bind {endpoint}"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;
    info!(endpoint, "agent IPC listener ready");
    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(serve_connection(stream, state));
    }
}

#[cfg(windows)]
pub async fn run(endpoint: &str, state: AgentState) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let mut first = true;
    info!(endpoint, "agent named pipe ready");
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first)
            .create(endpoint)?;
        first = false;
        server.connect().await?;
        let state = state.clone();
        tokio::spawn(serve_connection(server, state));
    }
}
