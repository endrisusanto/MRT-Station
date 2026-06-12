use anyhow::Context;
use em_core::{AgentResponse, AppError, ErrorCode};
use em_protocol::{Envelope, read_frame, write_frame};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{info, warn};

use crate::{config::AgentConfig, state::AgentState};

#[cfg(unix)]
struct SocketCleanup(std::path::PathBuf);

#[cfg(unix)]
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct PeerIdentity {
    uid: u32,
    pid: Option<i32>,
}

#[cfg(unix)]
fn verify_unix_peer(
    stream: &tokio::net::UnixStream,
    allowed_uids: &std::collections::BTreeSet<u32>,
) -> anyhow::Result<PeerIdentity> {
    let credentials = stream.peer_cred().context("SO_PEERCRED is unavailable")?;
    let uid = credentials.uid();
    if !allowed_uids.is_empty() && !allowed_uids.contains(&uid) {
        anyhow::bail!("UID {uid} is not in EM_AGENT_ALLOWED_UIDS");
    }
    Ok(PeerIdentity {
        uid,
        pid: credentials.pid(),
    })
}

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
pub async fn run(config: &AgentConfig, state: AgentState) -> anyhow::Result<()> {
    use std::{
        fs,
        os::unix::fs::{FileTypeExt, PermissionsExt},
    };
    use tokio::net::UnixListener;

    let path = config.endpoint.as_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() {
            anyhow::bail!(
                "refusing to replace non-socket endpoint: {}",
                path.display()
            );
        }
        fs::remove_file(path)
            .with_context(|| format!("failed to remove stale endpoint: {}", path.display()))?;
    }
    let listener =
        UnixListener::bind(path).with_context(|| format!("failed to bind {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o660))?;
    let _endpoint_guard = SocketCleanup(path.to_path_buf());
    info!(endpoint = %path.display(), "agent IPC listener ready");
    loop {
        let (stream, _) = listener.accept().await?;
        let peer = match verify_unix_peer(&stream, &config.allowed_uids) {
            Ok(peer) => peer,
            Err(error) => {
                warn!(%error, "rejected unauthorized IPC peer");
                continue;
            }
        };
        info!(uid = peer.uid, pid = ?peer.pid, "accepted IPC peer");
        let state = state.clone();
        tokio::spawn(serve_connection(stream, state));
    }
}

#[cfg(windows)]
pub async fn run(config: &AgentConfig, state: AgentState) -> anyhow::Result<()> {
    use tokio::net::windows::named_pipe::ServerOptions;

    let endpoint = config.endpoint.to_string_lossy();
    let mut first = true;
    info!(endpoint = %endpoint, "agent named pipe ready");
    loop {
        let server = ServerOptions::new()
            .first_pipe_instance(first)
            .create(endpoint.as_ref())?;
        first = false;
        server.connect().await?;
        let state = state.clone();
        tokio::spawn(serve_connection(server, state));
    }
}
