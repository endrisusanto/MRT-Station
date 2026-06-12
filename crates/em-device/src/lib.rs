mod production;
pub mod protocol;

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::Utc;
use em_core::{
    AppError, Device, DeviceMode, DeviceResult, ErrorCode, OperationKind, TokenOperationRequest,
    TransportKind,
};
use tokio::sync::RwLock;

pub use production::{DeviceInventory, ProductionDeviceProvider};

#[async_trait]
pub trait DeviceProvider: Send + Sync {
    async fn list_devices(&self) -> Result<Vec<Device>, AppError>;
    async fn execute(
        &self,
        kind: OperationKind,
        device_id: &str,
        request: &TokenOperationRequest,
    ) -> DeviceResult;
}

#[derive(Clone)]
pub struct SimulatedDeviceProvider {
    devices: Arc<RwLock<HashMap<String, Device>>>,
    operation_delay: Duration,
}

impl Default for SimulatedDeviceProvider {
    fn default() -> Self {
        let devices = [
            Device {
                id: "em-usb-001".into(),
                display_name: "EM Reference Device A".into(),
                model: "REF-A".into(),
                serial_number: "SIM000001".into(),
                firmware: "1.0.0-sim".into(),
                transport: TransportKind::Usb,
                mode: DeviceMode::Normal,
                connected: true,
                port: Some("USB-SIM-1".into()),
                vid: Some(0x04e8),
                pid: Some(0x6860),
            },
            Device {
                id: "em-serial-002".into(),
                display_name: "EM Reference Device B".into(),
                model: "REF-B".into(),
                serial_number: "SIM000002".into(),
                firmware: "1.1.0-sim".into(),
                transport: TransportKind::Serial,
                mode: DeviceMode::Normal,
                connected: true,
                port: Some(
                    if cfg!(windows) {
                        "COM7"
                    } else {
                        "/dev/ttyACM0"
                    }
                    .into(),
                ),
                vid: Some(0x04e8),
                pid: Some(0x685d),
            },
        ]
        .into_iter()
        .map(|device| (device.id.clone(), device))
        .collect();

        Self {
            devices: Arc::new(RwLock::new(devices)),
            operation_delay: Duration::from_millis(450),
        }
    }
}

#[async_trait]
impl DeviceProvider for SimulatedDeviceProvider {
    async fn list_devices(&self) -> Result<Vec<Device>, AppError> {
        let mut devices: Vec<_> = self.devices.read().await.values().cloned().collect();
        devices.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(devices)
    }

    async fn execute(
        &self,
        kind: OperationKind,
        device_id: &str,
        request: &TokenOperationRequest,
    ) -> DeviceResult {
        tokio::time::sleep(self.operation_delay).await;
        let Some(device) = self.devices.read().await.get(device_id).cloned() else {
            return DeviceResult {
                device_id: device_id.into(),
                success: false,
                message: "Device was not found".into(),
                token_id: None,
                error: Some(AppError::new(
                    ErrorCode::DeviceNotFound,
                    "Device was not found",
                )),
            };
        };

        if !device.connected {
            return DeviceResult {
                device_id: device_id.into(),
                success: false,
                message: "Device disconnected".into(),
                token_id: None,
                error: Some(
                    AppError::new(ErrorCode::DeviceDisconnected, "Device disconnected").retryable(),
                ),
            };
        }

        let message = match kind {
            OperationKind::TokenInfo => "Token information read",
            OperationKind::Install => "Token installed",
            OperationKind::Remove => "Token removed",
            OperationKind::Recover => "ESI recovery completed",
        };
        DeviceResult {
            device_id: device_id.into(),
            success: true,
            message: format!("{message} on {}", device.display_name),
            token_id: matches!(kind, OperationKind::TokenInfo | OperationKind::Install)
                .then(|| format!("SIM-{}-{}", Utc::now().timestamp(), request.mode_ids.len())),
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulator_reports_known_devices() {
        let provider = SimulatedDeviceProvider::default();
        let devices = provider.list_devices().await.unwrap();
        assert_eq!(devices.len(), 2);
    }
}
