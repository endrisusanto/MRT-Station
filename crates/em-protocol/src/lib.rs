use std::io;

use em_core::{AgentRequest, AgentResponse, PROTOCOL_VERSION};
use prost::Message;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

pub const MAX_FRAME_SIZE: usize = 1024 * 1024;

#[derive(Clone, PartialEq, Message)]
pub struct Envelope {
    #[prost(uint32, tag = "1")]
    pub protocol_version: u32,
    #[prost(string, tag = "2")]
    pub correlation_id: String,
    #[prost(enumeration = "MessageKind", tag = "3")]
    pub kind: i32,
    #[prost(bytes = "vec", tag = "4")]
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, prost::Enumeration)]
#[repr(i32)]
pub enum MessageKind {
    Request = 0,
    Response = 1,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("protobuf encode error: {0}")]
    Encode(#[from] prost::EncodeError),
    #[error("JSON payload error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("frame exceeds {MAX_FRAME_SIZE} bytes")]
    FrameTooLarge,
    #[error("unsupported protocol version {0}")]
    Version(u32),
    #[error("unexpected message kind")]
    Kind,
}

impl Envelope {
    pub fn request(request: &AgentRequest) -> Result<Self, ProtocolError> {
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            correlation_id: Uuid::new_v4().to_string(),
            kind: MessageKind::Request as i32,
            payload: serde_json::to_vec(request)?,
        })
    }

    pub fn response(
        correlation_id: String,
        response: &AgentResponse,
    ) -> Result<Self, ProtocolError> {
        Ok(Self {
            protocol_version: PROTOCOL_VERSION,
            correlation_id,
            kind: MessageKind::Response as i32,
            payload: serde_json::to_vec(response)?,
        })
    }

    pub fn decode_request(&self) -> Result<AgentRequest, ProtocolError> {
        self.validate(MessageKind::Request)?;
        Ok(serde_json::from_slice(&self.payload)?)
    }

    pub fn decode_response(&self) -> Result<AgentResponse, ProtocolError> {
        self.validate(MessageKind::Response)?;
        Ok(serde_json::from_slice(&self.payload)?)
    }

    fn validate(&self, expected: MessageKind) -> Result<(), ProtocolError> {
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(ProtocolError::Version(self.protocol_version));
        }
        if MessageKind::try_from(self.kind).ok() != Some(expected) {
            return Err(ProtocolError::Kind);
        }
        Ok(())
    }
}

pub async fn write_frame<W>(writer: &mut W, envelope: &Envelope) -> Result<(), ProtocolError>
where
    W: AsyncWrite + Unpin,
{
    let mut frame = Vec::with_capacity(envelope.encoded_len());
    envelope.encode(&mut frame)?;
    if frame.len() > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    writer.write_u32(frame.len() as u32).await?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

pub async fn read_frame<R>(reader: &mut R) -> Result<Envelope, ProtocolError>
where
    R: AsyncRead + Unpin,
{
    let length = reader.read_u32().await? as usize;
    if length > MAX_FRAME_SIZE {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut frame = vec![0; length];
    reader.read_exact(&mut frame).await?;
    Ok(Envelope::decode(frame.as_slice())?)
}

pub async fn request(
    endpoint: &str,
    request: &AgentRequest,
) -> Result<AgentResponse, ProtocolError> {
    platform_request(endpoint, request).await
}

async fn exchange<S>(stream: &mut S, request: &AgentRequest) -> Result<AgentResponse, ProtocolError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let envelope = Envelope::request(request)?;
    let correlation_id = envelope.correlation_id.clone();
    write_frame(stream, &envelope).await?;
    let response = read_frame(stream).await?;
    if response.correlation_id != correlation_id {
        return Err(ProtocolError::Kind);
    }
    response.decode_response()
}

#[cfg(unix)]
async fn platform_request(
    endpoint: &str,
    request: &AgentRequest,
) -> Result<AgentResponse, ProtocolError> {
    let mut stream = tokio::net::UnixStream::connect(endpoint).await?;
    exchange(&mut stream, request).await
}

#[cfg(windows)]
async fn platform_request(
    endpoint: &str,
    request: &AgentRequest,
) -> Result<AgentResponse, ProtocolError> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let mut attempts = 0;
    let mut stream = loop {
        match ClientOptions::new().open(endpoint) {
            Ok(client) => break client,
            Err(error) if error.raw_os_error() == Some(231) && attempts < 20 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(error) => return Err(error.into()),
        }
    };
    exchange(&mut stream, request).await
}

#[cfg(test)]
mod tests {
    use em_core::AgentRequest;
    use tokio::io::duplex;

    use super::*;

    #[tokio::test]
    async fn round_trips_framed_request() {
        let (mut client, mut server) = duplex(4096);
        let envelope = Envelope::request(&AgentRequest::Health).unwrap();
        let correlation = envelope.correlation_id.clone();

        let write = tokio::spawn(async move { write_frame(&mut client, &envelope).await });
        let decoded = read_frame(&mut server).await.unwrap();
        write.await.unwrap().unwrap();

        assert_eq!(decoded.correlation_id, correlation);
        assert!(matches!(
            decoded.decode_request().unwrap(),
            AgentRequest::Health
        ));
    }
}
