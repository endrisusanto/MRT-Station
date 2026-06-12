use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use crc32fast::Hasher;
use em_core::{AppError, ErrorCode};
use tokio::time::timeout;

const MAGIC: [u8; 2] = *b"EM";
const HEADER_SIZE: usize = 14;
pub const MAX_DEVICE_PAYLOAD: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFrame {
    pub version: u8,
    pub flags: u8,
    pub correlation_id: u32,
    pub command_id: u16,
    pub payload: Vec<u8>,
}

impl DeviceFrame {
    pub fn encode(&self) -> Result<Vec<u8>, ProtocolCodecError> {
        if self.payload.len() > MAX_DEVICE_PAYLOAD {
            return Err(ProtocolCodecError::PayloadTooLarge);
        }
        let mut bytes = Vec::with_capacity(HEADER_SIZE + self.payload.len() + 4);
        bytes.extend_from_slice(&MAGIC);
        bytes.push(self.version);
        bytes.push(self.flags);
        bytes.extend_from_slice(&self.correlation_id.to_be_bytes());
        bytes.extend_from_slice(&self.command_id.to_be_bytes());
        bytes.extend_from_slice(&(self.payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        bytes.extend_from_slice(&crc32(&bytes).to_be_bytes());
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolCodecError> {
        if bytes.len() < HEADER_SIZE + 4 {
            return Err(ProtocolCodecError::Truncated);
        }
        if bytes[..2] != MAGIC {
            return Err(ProtocolCodecError::Magic);
        }
        let payload_length = u32::from_be_bytes(bytes[10..14].try_into().unwrap()) as usize;
        if payload_length > MAX_DEVICE_PAYLOAD {
            return Err(ProtocolCodecError::PayloadTooLarge);
        }
        let expected_length = HEADER_SIZE + payload_length + 4;
        if bytes.len() != expected_length {
            return Err(ProtocolCodecError::Length);
        }
        let checksum_offset = expected_length - 4;
        let expected_checksum =
            u32::from_be_bytes(bytes[checksum_offset..expected_length].try_into().unwrap());
        if crc32(&bytes[..checksum_offset]) != expected_checksum {
            return Err(ProtocolCodecError::Checksum);
        }
        Ok(Self {
            version: bytes[2],
            flags: bytes[3],
            correlation_id: u32::from_be_bytes(bytes[4..8].try_into().unwrap()),
            command_id: u16::from_be_bytes(bytes[8..10].try_into().unwrap()),
            payload: bytes[HEADER_SIZE..checksum_offset].to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolCodecError {
    #[error("device frame is truncated")]
    Truncated,
    #[error("device frame magic does not match")]
    Magic,
    #[error("device frame length is invalid")]
    Length,
    #[error("device frame payload exceeds the limit")]
    PayloadTooLarge,
    #[error("device frame checksum does not match")]
    Checksum,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportErrorKind {
    Timeout,
    Disconnected,
    Busy,
    Other,
}

#[derive(Debug, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct TransportError {
    pub kind: TransportErrorKind,
    pub message: String,
}

impl TransportError {
    pub fn new(kind: TransportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    fn retryable(&self) -> bool {
        matches!(
            self.kind,
            TransportErrorKind::Timeout | TransportErrorKind::Disconnected
        )
    }
}

#[async_trait]
pub trait DeviceTransport: Send + Sync {
    async fn exchange(&self, request: Vec<u8>) -> Result<Vec<u8>, TransportError>;
}

#[derive(Debug, Clone, Copy)]
pub struct ProtocolPolicy {
    pub timeout: Duration,
    pub max_attempts: u8,
    pub version: u8,
}

impl Default for ProtocolPolicy {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            max_attempts: 2,
            version: 1,
        }
    }
}

pub struct ProtocolClient {
    transport: Arc<dyn DeviceTransport>,
    policy: ProtocolPolicy,
}

impl ProtocolClient {
    pub fn new(transport: Arc<dyn DeviceTransport>, policy: ProtocolPolicy) -> Self {
        Self { transport, policy }
    }

    pub async fn execute(
        &self,
        correlation_id: u32,
        command_id: u16,
        payload: Vec<u8>,
    ) -> Result<DeviceFrame, AppError> {
        if self.policy.max_attempts == 0 {
            return Err(AppError::new(
                ErrorCode::InvalidRequest,
                "Protocol max_attempts must be at least one",
            ));
        }
        let request = DeviceFrame {
            version: self.policy.version,
            flags: 0,
            correlation_id,
            command_id,
            payload,
        }
        .encode()
        .map_err(codec_error)?;

        for attempt in 1..=self.policy.max_attempts {
            let response = timeout(
                self.policy.timeout,
                self.transport.exchange(request.clone()),
            )
            .await
            .map_err(|_| {
                TransportError::new(TransportErrorKind::Timeout, "device exchange timed out")
            });
            match response {
                Ok(Ok(bytes)) => {
                    let frame = DeviceFrame::decode(&bytes).map_err(codec_error)?;
                    if frame.version != self.policy.version
                        || frame.correlation_id != correlation_id
                        || frame.command_id != command_id
                    {
                        return Err(AppError::new(
                            ErrorCode::ProtocolMismatch,
                            "Device response does not match the request",
                        ));
                    }
                    return Ok(frame);
                }
                Ok(Err(error)) | Err(error)
                    if error.retryable() && attempt < self.policy.max_attempts => {}
                Ok(Err(error)) | Err(error) => return Err(transport_error(error)),
            }
        }
        unreachable!("attempt loop always returns on its final iteration")
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

fn codec_error(error: ProtocolCodecError) -> AppError {
    AppError::new(ErrorCode::ProtocolMismatch, error.to_string())
}

fn transport_error(error: TransportError) -> AppError {
    let code = match error.kind {
        TransportErrorKind::Timeout => ErrorCode::TransportTimeout,
        TransportErrorKind::Disconnected => ErrorCode::DeviceDisconnected,
        TransportErrorKind::Busy => ErrorCode::DeviceBusy,
        TransportErrorKind::Other => ErrorCode::Internal,
    };
    let mut error = AppError::new(code, error.message);
    error.retryable = matches!(
        code,
        ErrorCode::TransportTimeout | ErrorCode::DeviceDisconnected
    );
    error
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use super::*;

    struct ScriptedTransport {
        responses: Mutex<VecDeque<Result<Vec<u8>, TransportError>>>,
    }

    #[async_trait]
    impl DeviceTransport for ScriptedTransport {
        async fn exchange(&self, _request: Vec<u8>) -> Result<Vec<u8>, TransportError> {
            self.responses.lock().unwrap().pop_front().unwrap()
        }
    }

    #[test]
    fn golden_frame_round_trip() {
        let frame = DeviceFrame {
            version: 1,
            flags: 0,
            correlation_id: 0x01020304,
            command_id: 0x1001,
            payload: vec![0xaa, 0xbb],
        };
        let bytes = frame.encode().unwrap();
        assert_eq!(hex(&bytes), "454d010001020304100100000002aabbb7674789");
        assert_eq!(DeviceFrame::decode(&bytes).unwrap(), frame);
    }

    #[test]
    fn rejects_corrupt_checksum() {
        let mut bytes = DeviceFrame {
            version: 1,
            flags: 0,
            correlation_id: 1,
            command_id: 2,
            payload: vec![3],
        }
        .encode()
        .unwrap();
        bytes[HEADER_SIZE] ^= 0xff;
        assert_eq!(
            DeviceFrame::decode(&bytes),
            Err(ProtocolCodecError::Checksum)
        );
    }

    #[tokio::test]
    async fn retries_disconnect_and_validates_correlation() {
        let response = DeviceFrame {
            version: 1,
            flags: 1,
            correlation_id: 7,
            command_id: 9,
            payload: vec![1],
        }
        .encode()
        .unwrap();
        let transport = Arc::new(ScriptedTransport {
            responses: Mutex::new(VecDeque::from([
                Err(TransportError::new(
                    TransportErrorKind::Disconnected,
                    "reconnect",
                )),
                Ok(response),
            ])),
        });
        let client = ProtocolClient::new(
            transport,
            ProtocolPolicy {
                timeout: Duration::from_secs(1),
                max_attempts: 2,
                version: 1,
            },
        );
        assert_eq!(client.execute(7, 9, vec![]).await.unwrap().payload, vec![1]);
    }

    #[tokio::test]
    async fn rejects_mismatched_response() {
        let response = DeviceFrame {
            version: 1,
            flags: 1,
            correlation_id: 99,
            command_id: 9,
            payload: vec![],
        }
        .encode()
        .unwrap();
        let transport = Arc::new(ScriptedTransport {
            responses: Mutex::new(VecDeque::from([Ok(response)])),
        });
        let error = ProtocolClient::new(transport, ProtocolPolicy::default())
            .execute(7, 9, vec![])
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ProtocolMismatch);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
