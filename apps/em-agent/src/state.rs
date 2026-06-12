use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use chrono::{Duration, Utc};
use em_core::{
    AgentRequest, AgentResponse, AgentStatus, AppError, ErrorCode, OperationKind, OperationState,
    OperationStatus, PROTOCOL_VERSION, Session, TokenMode, TokenOperationRequest,
};
use em_device::DeviceProvider;
use secrecy::{ExposeSecret, SecretString};
use tokio::sync::{RwLock, Semaphore};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct AgentState {
    provider: Arc<dyn DeviceProvider>,
    session: Arc<RwLock<Option<StoredSession>>>,
    operations: Arc<RwLock<HashMap<Uuid, OperationRecord>>>,
    operation_slots: Arc<Semaphore>,
}

struct StoredSession {
    public: Session,
    secret: SecretString,
}

#[derive(Clone)]
struct OperationRecord {
    status: OperationStatus,
    cancelled: Arc<AtomicBool>,
}

impl AgentState {
    pub fn new(provider: Arc<dyn DeviceProvider>) -> Self {
        Self {
            provider,
            session: Arc::new(RwLock::new(None)),
            operations: Arc::new(RwLock::new(HashMap::new())),
            operation_slots: Arc::new(Semaphore::new(4)),
        }
    }

    pub async fn handle(&self, request: AgentRequest) -> AgentResponse {
        match self.try_handle(request).await {
            Ok(response) => response,
            Err(error) => AgentResponse::Error(error),
        }
    }

    async fn try_handle(&self, request: AgentRequest) -> Result<AgentResponse, AppError> {
        match request {
            AgentRequest::GetAgentStatus => Ok(AgentResponse::AgentStatus(AgentStatus {
                installed: true,
                running: true,
                version: env!("CARGO_PKG_VERSION").into(),
                protocol_version: PROTOCOL_VERSION,
                compatible: true,
                update_available: false,
            })),
            AgentRequest::ListDevices => {
                Ok(AgentResponse::Devices(self.provider.list_devices().await?))
            }
            AgentRequest::Login(credentials) => {
                let username = credentials.username.trim();
                if username.is_empty() || credentials.password.is_empty() {
                    return Err(AppError::new(
                        ErrorCode::AuthenticationFailed,
                        "Username and password are required",
                    ));
                }
                let expires_at = Utc::now() + Duration::minutes(30);
                let public = Session {
                    user_id: username.to_owned(),
                    display_name: username.to_owned(),
                    expires_at,
                    remaining_seconds: 30 * 60,
                };
                *self.session.write().await = Some(StoredSession {
                    public: public.clone(),
                    secret: SecretString::from(credentials.password),
                });
                info!(user = username, "session started");
                Ok(AgentResponse::Session(Some(public)))
            }
            AgentRequest::Logout => {
                *self.session.write().await = None;
                info!("session ended");
                Ok(AgentResponse::Ack)
            }
            AgentRequest::GetSession => Ok(AgentResponse::Session(self.current_session().await)),
            AgentRequest::GetPermissions => {
                self.require_session().await?;
                Ok(AgentResponse::Permissions(default_permissions()))
            }
            AgentRequest::StartOperation { kind, request } => {
                self.require_session().await?;
                self.validate_operation(kind, &request).await?;
                Ok(AgentResponse::Operation(
                    self.start_operation(kind, request).await,
                ))
            }
            AgentRequest::CancelOperation { operation_id } => {
                let mut operations = self.operations.write().await;
                let record = operations.get_mut(&operation_id).ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidRequest, "Operation was not found")
                })?;
                record.cancelled.store(true, Ordering::Release);
                if matches!(
                    record.status.state,
                    OperationState::Queued | OperationState::Running
                ) {
                    record.status.state = OperationState::Cancelled;
                    record.status.finished_at = Some(Utc::now());
                }
                Ok(AgentResponse::Operation(record.status.clone()))
            }
            AgentRequest::GetOperation { operation_id } => {
                let operations = self.operations.read().await;
                let status = operations
                    .get(&operation_id)
                    .map(|record| record.status.clone())
                    .ok_or_else(|| {
                        AppError::new(ErrorCode::InvalidRequest, "Operation was not found")
                    })?;
                Ok(AgentResponse::Operation(status))
            }
            AgentRequest::Health => Ok(AgentResponse::Health {
                status: "ok".into(),
            }),
        }
    }

    async fn current_session(&self) -> Option<Session> {
        let mut session = self.session.write().await;
        let stored = session.as_mut()?;
        let remaining = (stored.public.expires_at - Utc::now()).num_seconds();
        if remaining <= 0 {
            *session = None;
            return None;
        }
        stored.public.remaining_seconds = remaining as u64;
        Some(stored.public.clone())
    }

    async fn require_session(&self) -> Result<(), AppError> {
        let Some(session) = self.current_session().await else {
            return Err(AppError::new(
                ErrorCode::SessionExpired,
                "Login is required",
            ));
        };
        let guard = self.session.read().await;
        let Some(stored) = guard.as_ref() else {
            return Err(AppError::new(
                ErrorCode::SessionExpired,
                "Login is required",
            ));
        };
        if stored.secret.expose_secret().is_empty() || session.remaining_seconds == 0 {
            return Err(AppError::new(ErrorCode::SessionExpired, "Session expired"));
        }
        Ok(())
    }

    async fn validate_operation(
        &self,
        kind: OperationKind,
        request: &TokenOperationRequest,
    ) -> Result<(), AppError> {
        if request.device_ids.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "At least one device must be selected",
            ));
        }
        if kind == OperationKind::Install && request.mode_ids.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "At least one token mode must be selected",
            ));
        }
        if kind == OperationKind::Install
            && request
                .expires_at
                .is_some_and(|expiry| expiry <= Utc::now())
        {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Expiry must be in the future",
            ));
        }
        Ok(())
    }

    async fn start_operation(
        &self,
        kind: OperationKind,
        request: TokenOperationRequest,
    ) -> OperationStatus {
        let id = Uuid::new_v4();
        let status = OperationStatus {
            id,
            kind,
            state: OperationState::Queued,
            completed: 0,
            total: request.device_ids.len(),
            started_at: Utc::now(),
            finished_at: None,
            results: Vec::new(),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        self.operations.write().await.insert(
            id,
            OperationRecord {
                status: status.clone(),
                cancelled: cancelled.clone(),
            },
        );
        let state = self.clone();
        tokio::spawn(async move { state.run_operation(id, kind, request, cancelled).await });
        status
    }

    async fn run_operation(
        &self,
        id: Uuid,
        kind: OperationKind,
        request: TokenOperationRequest,
        cancelled: Arc<AtomicBool>,
    ) {
        self.set_operation_state(id, OperationState::Running).await;
        let mut tasks = tokio::task::JoinSet::new();
        for device_id in request.device_ids.clone() {
            if cancelled.load(Ordering::Acquire) {
                break;
            }
            let provider = self.provider.clone();
            let slots = self.operation_slots.clone();
            let operation_request = request.clone();
            tasks.spawn(async move {
                let permit = slots
                    .acquire_owned()
                    .await
                    .expect("semaphore is not closed");
                let result = provider.execute(kind, &device_id, &operation_request).await;
                drop(permit);
                result
            });
        }

        while let Some(result) = tasks.join_next().await {
            if cancelled.load(Ordering::Acquire) {
                tasks.abort_all();
                self.finish_operation(id, OperationState::Cancelled).await;
                return;
            }
            match result {
                Ok(device_result) => self.push_result(id, device_result).await,
                Err(error) => warn!(%error, operation_id = %id, "device task failed"),
            }
        }
        let failed = self
            .operations
            .read()
            .await
            .get(&id)
            .is_some_and(|record| record.status.results.iter().any(|result| !result.success));
        self.finish_operation(
            id,
            if failed {
                OperationState::Failed
            } else {
                OperationState::Completed
            },
        )
        .await;
    }

    async fn set_operation_state(&self, id: Uuid, state: OperationState) {
        if let Some(record) = self.operations.write().await.get_mut(&id) {
            record.status.state = state;
        }
    }

    async fn push_result(&self, id: Uuid, result: em_core::DeviceResult) {
        if let Some(record) = self.operations.write().await.get_mut(&id) {
            record.status.results.push(result);
            record.status.completed = record.status.results.len();
        }
    }

    async fn finish_operation(&self, id: Uuid, state: OperationState) {
        if let Some(record) = self.operations.write().await.get_mut(&id) {
            record.status.state = state;
            record.status.finished_at = Some(Utc::now());
        }
    }
}

fn default_permissions() -> Vec<TokenMode> {
    [
        ("MODE_ENGINEER", "Engineer", "Engineering access"),
        ("MODE_INT_EM", "Internal EM", "Internal EM access"),
        ("MODE_ACCESS_SOD", "Access SOD", "SOD access"),
        (
            "MODE_RESTRICT_JANUS",
            "Restricted Janus",
            "Restricted Janus access",
        ),
    ]
    .into_iter()
    .map(|(id, name, description)| TokenMode {
        id: id.into(),
        display_name: name.into(),
        description: description.into(),
        permitted: true,
        attributes: BTreeMap::new(),
    })
    .collect()
}
