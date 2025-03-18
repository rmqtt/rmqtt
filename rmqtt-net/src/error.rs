use bytestring::ByteString;
use std::num::NonZeroU16;

use rmqtt_codec::error::{DecodeError, EncodeError, HandshakeError, ProtocolError, SendPacketError};
use rmqtt_codec::v5::{DisconnectReasonCode, PublishAckReason, ToReasonCode};

#[derive(Deserialize, Serialize, Debug, Clone, thiserror::Error)]
pub enum MqttError {
    /// Handshake error
    #[error("Mqtt handshake error: {}", _0)]
    Handshake(#[from] HandshakeError),
    #[error("Mqtt protocol error: {}", _0)]
    Protocol(#[from] ProtocolError),
    /// MQTT decoding error
    #[error("Decoding error: {0:?}")]
    Decode(#[from] DecodeError),
    /// MQTT encoding error
    #[error("Encoding error: {0:?}")]
    Encode(#[from] EncodeError),
    /// Send packet error
    #[error("Mqtt send packet error: {}", _0)]
    SendPacket(#[from] SendPacketError),
    /// Read timeout
    #[error("Read timeout")]
    ReadTimeout,
    /// Write timeout
    #[error("Write timeout")]
    WriteTimeout,
    /// Flush timeout
    #[error("Flush timeout")]
    FlushTimeout,
    /// Close timeout
    #[error("Close timeout")]
    CloseTimeout,
    #[error("{1}")]
    PublishAckReason(PublishAckReason, ByteString),
    #[error("service unavailable")]
    ServiceUnavailable,
    #[error("invalid protocol")]
    InvalidProtocol,
    #[error("too many subscriptions")]
    TooManySubscriptions,
    #[error("too many topic levels")]
    TooManyTopicLevels,
    #[error("subscription limit reached, {0}")]
    SubscribeLimited(String),
    #[error("identifier rejected")]
    IdentifierRejected,
    #[error("Provided packet id is in use")]
    PacketIdInUse(NonZeroU16),
}

impl ToReasonCode for MqttError {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            MqttError::Handshake(err) => err.to_reason_code(),
            MqttError::Protocol(err) => err.to_reason_code(),
            MqttError::Decode(err) => err.to_reason_code(),
            MqttError::Encode(err) => err.to_reason_code(),
            MqttError::SendPacket(err) => err.to_reason_code(),
            MqttError::ReadTimeout
            | MqttError::WriteTimeout
            | MqttError::FlushTimeout
            | MqttError::CloseTimeout => DisconnectReasonCode::KeepAliveTimeout,
            MqttError::PublishAckReason(_, _) => DisconnectReasonCode::ImplementationSpecificError,
            MqttError::ServiceUnavailable => DisconnectReasonCode::ServerBusy,
            MqttError::InvalidProtocol => DisconnectReasonCode::ProtocolError,
            MqttError::TooManySubscriptions => DisconnectReasonCode::QuotaExceeded,
            MqttError::TooManyTopicLevels => DisconnectReasonCode::TopicNameInvalid,
            MqttError::SubscribeLimited(_) => DisconnectReasonCode::QuotaExceeded,
            MqttError::IdentifierRejected => DisconnectReasonCode::NotAuthorized,
            MqttError::PacketIdInUse(_) => DisconnectReasonCode::UnspecifiedError,
        }
    }
}
