use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::Utc;
use em_backend::Authenticator;
use em_core::{
    AgentEvent, AgentEventKind, AgentRequest, AgentResponse, AgentStatus, AppError,
    DiagnosticDevice, DiagnosticSnapshot, ErrorCode, EventBatch, HealthSnapshot, OperationKind,
    OperationState, OperationStatus, PROTOCOL_VERSION, Session, TokenMode, TokenOperationRequest,
};
use em_device::DeviceProvider;
use secrecy::SecretString;
use tokio::sync::{RwLock, Semaphore};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone)]
pub struct AgentState {
    provider: Arc<dyn DeviceProvider>,
    authenticator: Arc<dyn Authenticator>,
    session: Arc<RwLock<Option<StoredSession>>>,
    operations: Arc<RwLock<HashMap<Uuid, OperationRecord>>>,
    operation_slots: Arc<Semaphore>,
    events: Arc<RwLock<VecDeque<AgentEvent>>>,
    next_event_sequence: Arc<AtomicU64>,
    last_devices: Arc<RwLock<Vec<em_core::Device>>>,
    started_at: Instant,
}

struct StoredSession {
    public: Session,
    token: SecretString,
    permissions: Vec<TokenMode>,
}

#[derive(Clone)]
struct OperationRecord {
    status: OperationStatus,
    cancelled: Arc<AtomicBool>,
}

impl AgentState {
    pub fn new(provider: Arc<dyn DeviceProvider>, authenticator: Arc<dyn Authenticator>) -> Self {
        Self {
            provider,
            authenticator,
            session: Arc::new(RwLock::new(None)),
            operations: Arc::new(RwLock::new(HashMap::new())),
            operation_slots: Arc::new(Semaphore::new(4)),
            events: Arc::new(RwLock::new(VecDeque::new())),
            next_event_sequence: Arc::new(AtomicU64::new(1)),
            last_devices: Arc::new(RwLock::new(Vec::new())),
            started_at: Instant::now(),
        }
    }

    pub fn start_background_tasks(&self) {
        let state = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            loop {
                ticker.tick().await;
                match state.provider.list_devices().await {
                    Ok(devices) => {
                        let changed = device_signature(&devices)
                            != device_signature(&state.last_devices.read().await);
                        if changed {
                            *state.last_devices.write().await = devices.clone();
                            state.emit(AgentEventKind::DevicesChanged(devices)).await;
                        }
                    }
                    Err(error) => warn!(%error, "device monitor refresh failed"),
                }
                state.cleanup_operations().await;
            }
        });
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
                let devices = self.provider.list_devices().await?;
                *self.last_devices.write().await = devices.clone();
                Ok(AgentResponse::Devices(devices))
            }
            AgentRequest::Login(credentials) => {
                let authenticated = self.authenticator.login(credentials).await?;
                let public = authenticated.public.clone();
                *self.session.write().await = Some(StoredSession {
                    public: public.clone(),
                    token: authenticated.token,
                    permissions: authenticated.permissions,
                });
                info!(user = %public.user_id, "session started");
                self.emit(AgentEventKind::SessionChanged(Some(public.clone())))
                    .await;
                Ok(AgentResponse::Session(Some(public)))
            }
            AgentRequest::Logout => {
                let session = self.session.write().await.take();
                if let Some(session) = session {
                    self.authenticator.logout(&session.token).await?;
                }
                info!("session ended");
                self.emit(AgentEventKind::SessionChanged(None)).await;
                Ok(AgentResponse::Ack)
            }
            AgentRequest::GetSession => Ok(AgentResponse::Session(self.current_session().await)),
            AgentRequest::GetPermissions => {
                self.require_session().await?;
                let permissions = self
                    .session
                    .read()
                    .await
                    .as_ref()
                    .map(|session| session.permissions.clone())
                    .unwrap_or_default();
                Ok(AgentResponse::Permissions(permissions))
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
                let status = record.status.clone();
                drop(operations);
                self.emit(AgentEventKind::OperationChanged(status.clone()))
                    .await;
                Ok(AgentResponse::Operation(status))
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
            AgentRequest::PollEvents {
                after_sequence,
                limit,
            } => Ok(AgentResponse::Events(
                self.poll_events(after_sequence, limit).await,
            )),
            AgentRequest::GetDiagnostics => {
                Ok(AgentResponse::Diagnostics(self.diagnostics().await))
            }
            AgentRequest::Health => Ok(AgentResponse::Health(self.health().await)),
        }
    }

    async fn current_session(&self) -> Option<Session> {
        let mut session = self.session.write().await;
        let stored = session.as_mut()?;
        let remaining = (stored.public.expires_at - Utc::now()).num_seconds();
        if remaining <= 0 {
            *session = None;
            drop(session);
            self.emit(AgentEventKind::SessionChanged(None)).await;
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
        if session.remaining_seconds == 0 {
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
        self.emit(AgentEventKind::OperationChanged(status.clone()))
            .await;
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
        let status = if let Some(record) = self.operations.write().await.get_mut(&id) {
            record.status.state = state;
            Some(record.status.clone())
        } else {
            None
        };
        if let Some(status) = status {
            self.emit(AgentEventKind::OperationChanged(status)).await;
        }
    }

    async fn push_result(&self, id: Uuid, result: em_core::DeviceResult) {
        let status = if let Some(record) = self.operations.write().await.get_mut(&id) {
            record.status.results.push(result);
            record.status.completed = record.status.results.len();
            Some(record.status.clone())
        } else {
            None
        };
        if let Some(status) = status {
            self.emit(AgentEventKind::OperationChanged(status)).await;
        }
    }

    async fn finish_operation(&self, id: Uuid, state: OperationState) {
        let status = if let Some(record) = self.operations.write().await.get_mut(&id) {
            record.status.state = state;
            record.status.finished_at = Some(Utc::now());
            Some(record.status.clone())
        } else {
            None
        };
        if let Some(status) = status {
            self.emit(AgentEventKind::OperationChanged(status)).await;
        }
        self.cleanup_operations().await;
    }

    async fn emit(&self, kind: AgentEventKind) {
        let sequence = self.next_event_sequence.fetch_add(1, Ordering::Relaxed);
        let mut events = self.events.write().await;
        events.push_back(AgentEvent {
            sequence,
            occurred_at: Utc::now(),
            kind,
        });
        while events.len() > 512 {
            events.pop_front();
        }
    }

    async fn poll_events(&self, after_sequence: u64, limit: usize) -> EventBatch {
        let limit = limit.clamp(1, 100);
        let events: Vec<_> = self
            .events
            .read()
            .await
            .iter()
            .filter(|event| event.sequence > after_sequence)
            .take(limit)
            .cloned()
            .collect();
        let next_sequence = events
            .last()
            .map(|event| event.sequence)
            .unwrap_or(after_sequence);
        EventBatch {
            events,
            next_sequence,
        }
    }

    async fn health(&self) -> HealthSnapshot {
        let operations = self.operations.read().await;
        HealthSnapshot {
            status: "ok".into(),
            uptime_seconds: self.started_at.elapsed().as_secs(),
            connected_devices: self.last_devices.read().await.len(),
            active_operations: operations
                .values()
                .filter(|record| {
                    matches!(
                        record.status.state,
                        OperationState::Queued | OperationState::Running
                    )
                })
                .count(),
            retained_operations: operations.len(),
            session_active: self.current_session().await.is_some(),
            event_sequence: self.next_event_sequence.load(Ordering::Relaxed) - 1,
        }
    }

    async fn diagnostics(&self) -> DiagnosticSnapshot {
        let devices = self
            .last_devices
            .read()
            .await
            .iter()
            .map(|device| DiagnosticDevice {
                model: device.model.clone(),
                transport: device.transport,
                mode: device.mode,
                connected: device.connected,
            })
            .collect();
        DiagnosticSnapshot {
            generated_at: Utc::now(),
            agent_version: env!("CARGO_PKG_VERSION").into(),
            health: self.health().await,
            devices,
            retained_event_count: self.events.read().await.len(),
        }
    }

    async fn cleanup_operations(&self) {
        let cutoff = Utc::now() - chrono::Duration::hours(1);
        let mut operations = self.operations.write().await;
        operations.retain(|_, record| {
            !is_terminal(record.status.state)
                || record
                    .status
                    .finished_at
                    .is_none_or(|finished| finished >= cutoff)
        });
        if operations.len() <= 100 {
            return;
        }
        let mut terminal: Vec<_> = operations
            .iter()
            .filter(|(_, record)| is_terminal(record.status.state))
            .map(|(id, record)| (*id, record.status.finished_at))
            .collect();
        terminal.sort_by_key(|(_, finished)| *finished);
        let remove_count = operations.len().saturating_sub(100);
        for (id, _) in terminal.into_iter().take(remove_count) {
            operations.remove(&id);
        }
    }
}

fn is_terminal(state: OperationState) -> bool {
    matches!(
        state,
        OperationState::Completed | OperationState::Failed | OperationState::Cancelled
    )
}

fn device_signature(devices: &[em_core::Device]) -> Vec<(String, bool, Option<String>)> {
    let mut signature: Vec<_> = devices
        .iter()
        .map(|device| (device.id.clone(), device.connected, device.port.clone()))
        .collect();
    signature.sort_unstable();
    signature
}
