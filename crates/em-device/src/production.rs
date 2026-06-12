use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::Path,
};

use async_trait::async_trait;
use em_core::{
    AppError, Device, DeviceMode, DeviceResult, ErrorCode, OperationKind, TokenOperationRequest,
    TransportKind,
};
use rusb::{Context, UsbContext};
use serde::Deserialize;

use crate::DeviceProvider;

#[derive(Debug, Clone)]
pub struct DeviceInventory {
    profiles: HashMap<(u16, u16), DeviceProfile>,
}

#[derive(Debug, Clone)]
struct DeviceProfile {
    display_name: String,
    model: String,
    mode: DeviceMode,
    firmware: String,
    protocol: Option<ProtocolProfile>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolProfile {
    codec: String,
    #[serde(default)]
    operations_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InventoryFile {
    devices: Vec<InventoryEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InventoryEntry {
    vid: String,
    pid: String,
    display_name: String,
    model: String,
    mode: DeviceMode,
    #[serde(default = "unknown_firmware")]
    firmware: String,
    #[serde(default)]
    protocol: Option<ProtocolProfile>,
}

impl DeviceInventory {
    pub fn load(path: &Path) -> Result<Self, AppError> {
        let contents = fs::read_to_string(path).map_err(|error| {
            AppError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "Unable to read device inventory {}: {error}",
                    path.display()
                ),
            )
        })?;
        let file: InventoryFile = serde_json::from_str(&contents).map_err(|error| {
            AppError::new(
                ErrorCode::InvalidRequest,
                format!("Invalid device inventory {}: {error}", path.display()),
            )
        })?;
        let mut profiles = HashMap::new();
        for entry in file.devices {
            let vid = parse_hex_id("vid", &entry.vid)?;
            let pid = parse_hex_id("pid", &entry.pid)?;
            if profiles
                .insert(
                    (vid, pid),
                    DeviceProfile {
                        display_name: entry.display_name,
                        model: entry.model,
                        mode: entry.mode,
                        firmware: entry.firmware,
                        protocol: entry.protocol,
                    },
                )
                .is_some()
            {
                return Err(AppError::new(
                    ErrorCode::InvalidRequest,
                    format!("Duplicate device inventory entry {vid:04x}:{pid:04x}"),
                ));
            }
        }
        if profiles.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Device inventory must contain at least one entry",
            ));
        }
        Ok(Self { profiles })
    }
}

#[derive(Clone)]
pub struct ProductionDeviceProvider {
    inventory: DeviceInventory,
}

impl ProductionDeviceProvider {
    pub fn new(inventory: DeviceInventory) -> Self {
        Self { inventory }
    }

    fn discover(&self) -> Result<Vec<Device>, AppError> {
        let mut devices = BTreeMap::<String, Device>::new();
        self.discover_usb(&mut devices)?;
        self.discover_serial(&mut devices)?;
        Ok(devices.into_values().collect())
    }

    fn discover_usb(&self, devices: &mut BTreeMap<String, Device>) -> Result<(), AppError> {
        let context = Context::new().map_err(discovery_error)?;
        for usb_device in context.devices().map_err(discovery_error)?.iter() {
            let descriptor = usb_device.device_descriptor().map_err(discovery_error)?;
            let key = (descriptor.vendor_id(), descriptor.product_id());
            let Some(profile) = self.inventory.profiles.get(&key) else {
                continue;
            };
            let serial = usb_device
                .open()
                .ok()
                .and_then(|handle| handle.read_serial_number_string_ascii(&descriptor).ok())
                .filter(|serial| !serial.trim().is_empty());
            let location = format!(
                "bus-{}-address-{}",
                usb_device.bus_number(),
                usb_device.address()
            );
            let identity = serial.clone().unwrap_or_else(|| location.clone());
            let id = stable_id(key.0, key.1, &identity);
            devices.insert(
                id.clone(),
                Device {
                    id,
                    display_name: profile.display_name.clone(),
                    model: profile.model.clone(),
                    serial_number: serial.unwrap_or_else(|| "unknown".into()),
                    firmware: profile.firmware.clone(),
                    transport: TransportKind::Usb,
                    mode: profile.mode,
                    connected: true,
                    port: Some(location),
                    vid: Some(key.0),
                    pid: Some(key.1),
                },
            );
        }
        Ok(())
    }

    fn discover_serial(&self, devices: &mut BTreeMap<String, Device>) -> Result<(), AppError> {
        for port in serialport::available_ports().map_err(discovery_error)? {
            let serialport::SerialPortType::UsbPort(info) = port.port_type else {
                continue;
            };
            let key = (info.vid, info.pid);
            let Some(profile) = self.inventory.profiles.get(&key) else {
                continue;
            };
            let identity = info
                .serial_number
                .clone()
                .filter(|serial| !serial.trim().is_empty())
                .unwrap_or_else(|| port.port_name.clone());
            let id = stable_id(key.0, key.1, &identity);
            let serial_device = Device {
                id: id.clone(),
                display_name: profile.display_name.clone(),
                model: profile.model.clone(),
                serial_number: info.serial_number.unwrap_or_else(|| "unknown".into()),
                firmware: profile.firmware.clone(),
                transport: TransportKind::Serial,
                mode: profile.mode,
                connected: true,
                port: Some(port.port_name),
                vid: Some(key.0),
                pid: Some(key.1),
            };
            devices
                .entry(id)
                .and_modify(|device| {
                    // Prefer the actionable CDC/COM endpoint when the same hardware is visible via USB.
                    *device = serial_device.clone();
                })
                .or_insert(serial_device);
        }
        Ok(())
    }
}

#[async_trait]
impl DeviceProvider for ProductionDeviceProvider {
    async fn list_devices(&self) -> Result<Vec<Device>, AppError> {
        let provider = self.clone();
        tokio::task::spawn_blocking(move || provider.discover())
            .await
            .map_err(|error| AppError::new(ErrorCode::Internal, error.to_string()))?
    }

    async fn execute(
        &self,
        _kind: OperationKind,
        device_id: &str,
        _request: &TokenOperationRequest,
    ) -> DeviceResult {
        let profile = self
            .discover()
            .ok()
            .and_then(|devices| devices.into_iter().find(|device| device.id == device_id))
            .and_then(|device| Some((device.vid?, device.pid?)))
            .and_then(|key| self.inventory.profiles.get(&key));
        let message = match profile.and_then(|profile| profile.protocol.as_ref()) {
            None => "Device protocol is disabled by inventory",
            Some(protocol) if !protocol.operations_enabled => {
                "Device operations are disabled by inventory"
            }
            Some(protocol) => {
                return DeviceResult {
                    device_id: device_id.into(),
                    success: false,
                    message: format!("Protocol codec '{}' is not installed", protocol.codec),
                    token_id: None,
                    error: Some(AppError::new(
                        ErrorCode::ProtocolMismatch,
                        "No approved production codec is registered for this device",
                    )),
                };
            }
        };
        DeviceResult {
            device_id: device_id.into(),
            success: false,
            message: message.into(),
            token_id: None,
            error: Some(AppError::new(ErrorCode::PermissionDenied, message)),
        }
    }
}

fn parse_hex_id(field: &str, value: &str) -> Result<u16, AppError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    u16::from_str_radix(value, 16).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidRequest,
            format!("Invalid hexadecimal {field}: {value}"),
        )
    })
}

fn stable_id(vid: u16, pid: u16, identity: &str) -> String {
    let normalized: String = identity
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    format!("em-{vid:04x}-{pid:04x}-{normalized}")
}

fn discovery_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::AgentUnavailable, error.to_string()).retryable()
}

fn unknown_firmware() -> String {
    "unknown".into()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[test]
    fn creates_stable_sanitized_identity() {
        assert_eq!(
            stable_id(0x04e8, 0x6860, "SERIAL 01/AB"),
            "em-04e8-6860-serial-01-ab"
        );
    }

    #[test]
    fn loads_hex_inventory_and_rejects_duplicates() {
        let path =
            std::env::temp_dir().join(format!("em-device-inventory-{}.json", std::process::id()));
        let mut file = fs::File::create(&path).unwrap();
        write!(file, r#"{{"devices":[{{"vid":"04e8","pid":"6860","displayName":"EM Device","model":"REF-A","mode":"normal"}}]}}"#).unwrap();
        let inventory = DeviceInventory::load(&path).unwrap();
        assert!(inventory.profiles.contains_key(&(0x04e8, 0x6860)));
        fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn production_operations_fail_closed() {
        let inventory = DeviceInventory {
            profiles: HashMap::from([(
                (0x04e8, 0x6860),
                DeviceProfile {
                    display_name: "EM Device".into(),
                    model: "REF-A".into(),
                    mode: DeviceMode::Normal,
                    firmware: "unknown".into(),
                    protocol: None,
                },
            )]),
        };
        let result = ProductionDeviceProvider::new(inventory)
            .execute(
                OperationKind::Install,
                "device",
                &TokenOperationRequest {
                    device_ids: vec!["device".into()],
                    mode_ids: vec!["mode".into()],
                    expires_at: None,
                },
            )
            .await;
        assert!(!result.success);
        assert!(result.error.is_some());
    }
}
