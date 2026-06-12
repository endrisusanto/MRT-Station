use chrono::{DateTime, Utc};
use em_core::{
    AgentRequest, AgentResponse, AgentStatus, Device, LoginCredentialsDto, OperationKind,
    OperationStatus, Session, TokenMode, TokenOperationRequest,
};
use uuid::Uuid;

#[cfg(unix)]
const DEVELOPMENT_ENDPOINT: &str = "/tmp/em-station/agent.sock";
#[cfg(unix)]
const PRODUCTION_ENDPOINT: &str = "/run/em-station/agent.sock";
#[cfg(windows)]
const DEFAULT_ENDPOINT: &str = r"\\.\pipe\em-station-agent";

fn endpoint() -> String {
    std::env::var("EM_AGENT_ENDPOINT").unwrap_or_else(|_| default_endpoint().into())
}

#[cfg(unix)]
fn default_endpoint() -> &'static str {
    if cfg!(debug_assertions) {
        DEVELOPMENT_ENDPOINT
    } else {
        PRODUCTION_ENDPOINT
    }
}

#[cfg(windows)]
fn default_endpoint() -> &'static str {
    DEFAULT_ENDPOINT
}

async fn call(request: AgentRequest) -> Result<AgentResponse, String> {
    em_protocol::request(&endpoint(), &request)
        .await
        .map_err(|error| error.to_string())
}

fn unexpected(response: AgentResponse) -> String {
    format!("Unexpected agent response: {response:?}")
}

#[tauri::command]
async fn get_agent_status() -> Result<AgentStatus, String> {
    match call(AgentRequest::GetAgentStatus).await? {
        AgentResponse::AgentStatus(status) => Ok(status),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[tauri::command]
async fn list_devices() -> Result<Vec<Device>, String> {
    match call(AgentRequest::ListDevices).await? {
        AgentResponse::Devices(devices) => Ok(devices),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[tauri::command]
async fn get_session() -> Result<Option<Session>, String> {
    match call(AgentRequest::GetSession).await? {
        AgentResponse::Session(session) => Ok(session),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[tauri::command]
async fn login(username: String, password: String) -> Result<Session, String> {
    match call(AgentRequest::Login(LoginCredentialsDto {
        username,
        password,
    }))
    .await?
    {
        AgentResponse::Session(Some(session)) => Ok(session),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[tauri::command]
async fn logout() -> Result<(), String> {
    match call(AgentRequest::Logout).await? {
        AgentResponse::Ack => Ok(()),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[tauri::command]
async fn get_permissions() -> Result<Vec<TokenMode>, String> {
    match call(AgentRequest::GetPermissions).await? {
        AgentResponse::Permissions(modes) => Ok(modes),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[tauri::command]
async fn start_operation(
    kind: OperationKind,
    device_ids: Vec<String>,
    mode_ids: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<OperationStatus, String> {
    let request = AgentRequest::StartOperation {
        kind,
        request: TokenOperationRequest {
            device_ids,
            mode_ids,
            expires_at,
        },
    };
    operation_response(call(request).await?)
}

#[tauri::command]
async fn get_operation(operation_id: Uuid) -> Result<OperationStatus, String> {
    operation_response(call(AgentRequest::GetOperation { operation_id }).await?)
}

#[tauri::command]
async fn cancel_operation(operation_id: Uuid) -> Result<OperationStatus, String> {
    operation_response(call(AgentRequest::CancelOperation { operation_id }).await?)
}

fn operation_response(response: AgentResponse) -> Result<OperationStatus, String> {
    match response {
        AgentResponse::Operation(operation) => Ok(operation),
        AgentResponse::Error(error) => Err(error.to_string()),
        response => Err(unexpected(response)),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_agent_status,
            list_devices,
            get_session,
            login,
            logout,
            get_permissions,
            start_operation,
            get_operation,
            cancel_operation
        ])
        .run(tauri::generate_context!())
        .expect("error while running EM Station");
}
