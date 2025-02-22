use rmqtt_codec::error::{
    DecodeError, EncodeError, HandshakeError, ProtocolError, SendPacketError,
};

#[derive(Debug, thiserror::Error)]
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
    #[error("service unavailable")]
    ServiceUnavailable,
}
