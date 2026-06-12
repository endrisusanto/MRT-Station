use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::AppError;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub installed: bool,
    pub running: bool,
    pub version: String,
    pub protocol_version: u32,
    pub compatible: bool,
    pub update_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    Usb,
    Serial,
    Wifi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceMode {
    Normal,
    Download,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub display_name: String,
    pub model: String,
    pub serial_number: String,
    pub firmware: String,
    pub transport: TransportKind,
    pub mode: DeviceMode,
    pub connected: bool,
    pub port: Option<String>,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub user_id: String,
    pub display_name: String,
    pub expires_at: DateTime<Utc>,
    pub remaining_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenMode {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub permitted: bool,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    TokenInfo,
    Install,
    Remove,
    Recover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenOperationRequest {
    pub device_ids: Vec<String>,
    #[serde(default)]
    pub mode_ids: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceResult {
    pub device_id: String,
    pub success: bool,
    pub message: String,
    pub token_id: Option<String>,
    pub error: Option<AppError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationStatus {
    pub id: Uuid,
    pub kind: OperationKind,
    pub state: OperationState,
    pub completed: usize,
    pub total: usize,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub results: Vec<DeviceResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params", rename_all = "snake_case")]
pub enum AgentRequest {
    GetAgentStatus,
    ListDevices,
    Login(LoginCredentialsDto),
    Logout,
    GetSession,
    GetPermissions,
    StartOperation {
        kind: OperationKind,
        request: TokenOperationRequest,
    },
    CancelOperation {
        operation_id: Uuid,
    },
    GetOperation {
        operation_id: Uuid,
    },
    Health,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginCredentialsDto {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentResponse {
    AgentStatus(AgentStatus),
    Devices(Vec<Device>),
    Session(Option<Session>),
    Permissions(Vec<TokenMode>),
    Operation(OperationStatus),
    Ack,
    Health { status: String },
    Error(AppError),
}
