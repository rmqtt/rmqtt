use std::{io, num::NonZeroU16};

/// Errors which can occur during mqtt connection handshake.
#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    /// Protocol error
    #[error("Mqtt protocol error: {}", _0)]
    Protocol(#[from] ProtocolError),
    /// Handshake timeout
    #[error("Handshake timeout")]
    Timeout,
    /// Peer disconnect
    #[error("Peer is disconnected, error: {:?}", _0)]
    Disconnected(Option<io::Error>),
}

/// Protocol level errors
#[derive(Debug, thiserror::Error)]
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

#[derive(Debug, thiserror::Error)]
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
    Io(io::Error),
}

impl From<io::Error> for DecodeError {
    fn from(e: io::Error) -> DecodeError {
        DecodeError::Io(e)
    }
}

#[derive(Debug, thiserror::Error)]
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
    Io(io::Error),
}

impl From<io::Error> for EncodeError {
    fn from(e: io::Error) -> EncodeError {
        EncodeError::Io(e)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SendPacketError {
    /// Encoder error
    #[error("Encoding error {:?}", _0)]
    Encode(#[from] EncodeError),
    /// Provided packet id is in use
    #[error("Provided packet id is in use")]
    PacketIdInUse(NonZeroU16),
    /// Peer disconnected
    #[error("Peer is disconnected")]
    Disconnected,
}
