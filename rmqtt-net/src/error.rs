//! MQTT error types for protocol, I/O, and session-level failures.
//!
//! Defines [`MqttError`] with variants covering handshake, protocol, encoding/decoding,
//! timeout, and resource-limit errors, along with conversion to MQTTv5 disconnect reason codes.

use bytestring::ByteString;
use std::num::NonZeroU16;

use serde::{Deserialize, Serialize};

use rmqtt_codec::error::{DecodeError, EncodeError, HandshakeError, ProtocolError, SendPacketError};
use rmqtt_codec::v5::{DisconnectReasonCode, PublishAckReason, ToReasonCode};

use crate::Error;

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
    #[error("Is None")]
    None,
}

/// The packet exceeds the Maximum Packet Size declared by the peer ([MQTT-3.1.2-24]).
///
/// For an Application Message this condition is **not** fatal: section 3.1.2.11.4 requires the
/// Server to "discard it without sending it and then behave as if it had completed sending that
/// Application Message" ([MQTT-3.1.2-25]). Callers must therefore downgrade it instead of
/// propagating it as a connection error.
#[inline]
pub fn is_over_max_packet_size(err: &Error) -> bool {
    #[inline]
    fn over(e: &EncodeError) -> bool {
        matches!(e, EncodeError::OverMaxPacketSize { .. })
    }

    if let Some(e) = err.downcast_ref::<MqttError>() {
        return match e {
            MqttError::Encode(e) => over(e),
            MqttError::SendPacket(SendPacketError::Encode(e)) => over(e),
            _ => false,
        };
    }
    // With `send_timeout` set to zero the encoder error is not wrapped, see `stream::send`.
    err.downcast_ref::<EncodeError>().is_some_and(over)
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
            MqttError::None => DisconnectReasonCode::UnspecifiedError,
        }
    }
}
