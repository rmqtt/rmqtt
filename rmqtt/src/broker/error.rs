use bytestring::ByteString;
use std::net::AddrParseError;
use std::num::{ParseIntError, TryFromIntError};
use std::str::Utf8Error;

use config::ConfigError;
use ntex_mqtt::error::SendPacketError;
use ntex_mqtt::v5;
use ntex_mqtt::v5::codec::PublishAckReason;
use ntex_mqtt::TopicError;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use tokio::task::JoinError;
use tokio::time::Duration;

use super::types::Reason;

#[derive(Error, Debug)]
pub enum MqttError {
    #[error("service unavailable")]
    ServiceUnavailable,
    #[error("read/write timeout")]
    Timeout(Duration),
    #[error("send packet error, {0}")]
    SendPacketError(SendPacketError),
    #[error("send error, {0}")]
    SendError(String),
    #[error("{0}")]
    JoinError(#[from] JoinError),
    #[error("{0}")]
    StdError(#[from] Box<dyn std::error::Error + Send + Sync>),
    #[error("{0}")]
    Error(String),
    #[error("{0}")]
    IoError(#[from] std::io::Error),
    #[error("{0}")]
    WSError(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("{0}")]
    TokioTryLockError(#[from] tokio::sync::TryLockError),
    #[error("{0}")]
    Msg(String),
    #[error("{0}")]
    Anyhow(#[from] anyhow::Error),
    #[error("gprc error: `{0}`")]
    Grpc(#[from] tonic::transport::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("topic error, {0}")]
    TopicError(String),
    #[error("utf8 error, {0}")]
    Utf8Error(#[from] Utf8Error),
    #[error("too many subscriptions")]
    TooManySubscriptions,
    #[error("too many topic levels")]
    TooManyTopicLevels,
    #[error("{0}")]
    ConfigError(#[from] ConfigError),
    #[error("{0}")]
    AddrParseError(#[from] AddrParseError),
    #[error("{0}")]
    ParseIntError(#[from] ParseIntError),
    #[error("listener config is error")]
    ListenerConfigError,
    #[error("{0}")]
    Reason(Reason),
    #[error("{1}")]
    PublishAckReason(PublishAckReason, ByteString),
    #[error("{0}")]
    TryFromIntError(#[from] TryFromIntError),
    #[error("None")]
    None,
}

impl Default for MqttError {
    #[inline]
    fn default() -> Self {
        MqttError::ServiceUnavailable
    }
}

impl From<()> for MqttError {
    #[inline]
    fn from(_: ()) -> Self {
        Self::default()
    }
}

impl From<String> for MqttError {
    #[inline]
    fn from(e: String) -> Self {
        MqttError::Msg(e)
    }
}

impl From<&str> for MqttError {
    #[inline]
    fn from(e: &str) -> Self {
        MqttError::Msg(e.to_string())
    }
}

impl From<SendPacketError> for MqttError {
    #[inline]
    fn from(e: SendPacketError) -> Self {
        MqttError::SendPacketError(e)
    }
}

impl From<TopicError> for MqttError {
    #[inline]
    fn from(e: TopicError) -> Self {
        MqttError::TopicError(format!("{:?}", e))
    }
}

impl<T: Send + Sync + core::fmt::Debug> From<SendError<T>> for MqttError {
    #[inline]
    fn from(e: SendError<T>) -> Self {
        MqttError::SendError(format!("{:?}", e))
    }
}

impl From<Box<dyn std::error::Error>> for MqttError {
    #[inline]
    fn from(e: Box<dyn std::error::Error>) -> Self {
        MqttError::Error(e.to_string())
    }
}

impl std::convert::TryFrom<MqttError> for v5::PublishAck {
    type Error = MqttError;
    #[inline]
    fn try_from(e: MqttError) -> Result<Self, Self::Error> {
        Err(e)
    }
}

impl std::convert::TryFrom<MqttError> for v5::PublishResult {
    type Error = MqttError;
    #[inline]
    fn try_from(e: MqttError) -> Result<Self, Self::Error> {
        Err(e)
    }
}

impl From<MqttError> for tonic::Status {
    #[inline]
    fn from(e: MqttError) -> Self {
        tonic::Status::new(tonic::Code::Unavailable, format!("{:?}", e))
    }
}
