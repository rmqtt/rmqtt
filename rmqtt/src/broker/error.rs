use config::ConfigError;
use ntex_mqtt::error::SendPacketError;
use ntex_mqtt::v5;
use ntex_mqtt::TopicError;
use std::str::Utf8Error;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use tokio::time::Duration;

#[derive(Error, Debug)]
pub enum MqttError {
    #[error("service unavailable")]
    ServiceUnavailable,
    #[error("read/write timeout")]
    Timeout(Duration),
    #[error("send packet error, {0}")]
    SendPacketError(SendPacketError),
    #[error("{0}")]
    Error(Box<dyn std::error::Error>),
    #[error("{0}")]
    IoError(std::io::Error),
    #[error("{0}")]
    Msg(String),
    #[error("{0}")]
    Anyhow(anyhow::Error),
    #[error("{0}")]
    Json(serde_json::Error),
    #[error("topic error, {0}")]
    TopicError(String),
    #[error("utf8 error, {0}")]
    Utf8Error(Utf8Error),
    #[error("too many subscriptions")]
    TooManySubscriptions,
    #[error("{0}")]
    ConfigError(ConfigError),
}

unsafe impl std::marker::Send for MqttError {}
unsafe impl std::marker::Sync for MqttError {}

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

impl From<Utf8Error> for MqttError {
    #[inline]
    fn from(e: Utf8Error) -> Self {
        MqttError::Utf8Error(e)
    }
}

impl From<anyhow::Error> for MqttError {
    #[inline]
    fn from(e: anyhow::Error) -> Self {
        MqttError::Anyhow(e)
    }
}

impl From<serde_json::Error> for MqttError {
    #[inline]
    fn from(e: serde_json::Error) -> Self {
        MqttError::Json(e)
    }
}

impl From<std::io::Error> for MqttError {
    #[inline]
    fn from(e: std::io::Error) -> Self {
        MqttError::IoError(e)
    }
}

impl<T: Send + Sync + core::fmt::Debug> From<SendError<T>> for MqttError {
    #[inline]
    fn from(e: SendError<T>) -> Self {
        MqttError::Msg(format!("{:?}", e))
    }
}

impl From<Box<dyn std::error::Error>> for MqttError {
    #[inline]
    fn from(e: Box<dyn std::error::Error>) -> Self {
        MqttError::Error(e)
    }
}

impl std::convert::TryFrom<MqttError> for v5::PublishAck {
    type Error = MqttError;
    #[inline]
    fn try_from(err: MqttError) -> Result<Self, Self::Error> {
        Err(err)
    }
}

impl From<ConfigError> for MqttError {
    #[inline]
    fn from(e: ConfigError) -> Self {
        MqttError::ConfigError(e)
    }
}
