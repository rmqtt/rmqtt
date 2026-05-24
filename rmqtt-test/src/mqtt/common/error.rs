//! Codec error type

#[derive(Debug, Clone, thiserror::Error)]
pub enum CodecError {
    #[error("invalid QoS level: {0}")]
    InvalidQoS(u8),
    #[error("invalid return code: {0}")]
    InvalidReturnCode(u8),
    #[error("malformed packet: {0}")]
    MalformedPacket(&'static str),
    #[error("invalid length")]
    InvalidLength,
    #[error("invalid protocol")]
    InvalidProtocol,
    #[error("packet ID required for QoS > 0")]
    PacketIdRequired,
    #[error("unsupported packet type: {0:#x}")]
    UnsupportedPacketType(u8),
    #[error("IO error: {0}")]
    Io(String),
    #[error("UTF-8 error")]
    Utf8Error,
    #[error("max size exceeded")]
    MaxSizeExceeded,
}

impl From<std::io::Error> for CodecError {
    fn from(e: std::io::Error) -> Self {
        CodecError::Io(e.to_string())
    }
}
