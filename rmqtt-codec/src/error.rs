use std::io;

use bytestring::ByteString;
use serde::{Deserialize, Serialize};

use crate::v5::{DisconnectReasonCode, ToReasonCode};

/// Errors which can occur during mqtt connection handshake.
#[derive(Deserialize, Serialize, Debug, Clone, thiserror::Error)]
pub enum HandshakeError {
    /// Protocol error
    #[error("Mqtt protocol error: {}", _0)]
    Protocol(#[from] ProtocolError),
    /// Handshake timeout
    #[error("Handshake timeout")]
    Timeout,
}

impl ToReasonCode for HandshakeError {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            HandshakeError::Protocol(err) => err.to_reason_code(),
            HandshakeError::Timeout => DisconnectReasonCode::MaximumConnectTime,
            // HandshakeError::Disconnected(_) => DisconnectReasonCode::ServerBusy,
        }
    }
}

/// Protocol level errors
#[derive(Deserialize, Serialize, Debug, Clone, thiserror::Error)]
pub enum ProtocolError {
    /// MQTT decoding error
    #[error("Decoding error: {0:?}")]
    Decode(#[from] DecodeError),
    /// MQTT encoding error
    #[error("Encoding error: {0:?}")]
    Encode(#[from] EncodeError),
    /// Keep alive timeout
    #[error("Keep Alive timeout")]
    KeepAliveTimeout,
}

impl ToReasonCode for ProtocolError {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            ProtocolError::Decode(err) => err.to_reason_code(),
            ProtocolError::Encode(err) => err.to_reason_code(),
            ProtocolError::KeepAliveTimeout => DisconnectReasonCode::KeepAliveTimeout,
        }
    }
}

#[derive(Debug, Clone, thiserror::Error, Deserialize, Serialize)]
pub enum DecodeError {
    #[error("Invalid protocol")]
    InvalidProtocol,
    #[error("Invalid length")]
    InvalidLength,
    #[error("Malformed packet")]
    MalformedPacket,
    #[error("Unsupported protocol level")]
    UnsupportedProtocolLevel,
    #[error("Connect frame's reserved flag is set")]
    ConnectReservedFlagSet,
    #[error("ConnectAck frame's reserved flag is set")]
    ConnAckReservedFlagSet,
    #[error("Invalid client id")]
    InvalidClientId,
    #[error("Unsupported packet type")]
    UnsupportedPacketType,
    // MQTT v3 only
    #[error("Packet id is required")]
    PacketIdRequired,
    #[error("Max size exceeded")]
    MaxSizeExceeded,
    #[error("utf8 error")]
    Utf8Error,
    #[error("io error, {:?}", _0)]
    Io(ByteString),
}

impl From<io::Error> for DecodeError {
    fn from(e: io::Error) -> DecodeError {
        DecodeError::Io(e.to_string().into())
    }
}

impl ToReasonCode for DecodeError {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            DecodeError::InvalidProtocol => DisconnectReasonCode::ProtocolError,
            DecodeError::InvalidLength => DisconnectReasonCode::MalformedPacket,
            DecodeError::MalformedPacket => DisconnectReasonCode::MalformedPacket,
            DecodeError::UnsupportedProtocolLevel => DisconnectReasonCode::ImplementationSpecificError,
            DecodeError::ConnectReservedFlagSet => DisconnectReasonCode::ProtocolError,
            DecodeError::ConnAckReservedFlagSet => DisconnectReasonCode::ProtocolError,
            DecodeError::InvalidClientId => DisconnectReasonCode::NotAuthorized,
            DecodeError::UnsupportedPacketType => DisconnectReasonCode::ImplementationSpecificError,
            DecodeError::PacketIdRequired => DisconnectReasonCode::ProtocolError,
            DecodeError::MaxSizeExceeded => DisconnectReasonCode::PacketTooLarge,
            DecodeError::Utf8Error => DisconnectReasonCode::PayloadFormatInvalid,
            DecodeError::Io(_) => DisconnectReasonCode::UnspecifiedError,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, thiserror::Error)]
pub enum EncodeError {
    #[error("Packet is bigger than peer's Maximum Packet Size")]
    OverMaxPacketSize,
    #[error("Invalid length")]
    InvalidLength,
    #[error("Malformed packet")]
    MalformedPacket,
    #[error("Packet id is required")]
    PacketIdRequired,
    #[error("Unsupported version")]
    UnsupportedVersion,
    #[error("io error, {:?}", _0)]
    Io(ByteString),
}

impl From<io::Error> for EncodeError {
    fn from(e: io::Error) -> EncodeError {
        EncodeError::Io(e.to_string().into())
    }
}

impl ToReasonCode for EncodeError {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            EncodeError::OverMaxPacketSize => DisconnectReasonCode::PacketTooLarge,
            EncodeError::InvalidLength => DisconnectReasonCode::MalformedPacket,
            EncodeError::MalformedPacket => DisconnectReasonCode::MalformedPacket,
            EncodeError::PacketIdRequired => DisconnectReasonCode::ProtocolError,
            EncodeError::UnsupportedVersion => DisconnectReasonCode::ImplementationSpecificError,
            EncodeError::Io(_) => DisconnectReasonCode::UnspecifiedError,
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, thiserror::Error)]
pub enum SendPacketError {
    /// Encoder error
    #[error("Encoding error {:?}", _0)]
    Encode(#[from] EncodeError),
    // /// Provided packet id is in use
    // #[error("Provided packet id is in use")]
    // PacketIdInUse(NonZeroU16),

    // /// Peer disconnected
    // #[error("Peer is disconnected")]
    // Disconnected,
}

impl ToReasonCode for SendPacketError {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            SendPacketError::Encode(e) => e.to_reason_code(),
        }
    }
}
