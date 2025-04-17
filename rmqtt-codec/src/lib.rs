#![deny(unsafe_code)]

//! MQTT protocol codec implementation with multi-version support and version negotiation
//!
//! ## Core Features:
//! - **Dual Protocol Support**: Full implementation of MQTT v3.1, v3.1.1 and v5.0 specifications
//! - **Automatic Version Detection**: Handshake-based protocol negotiation during connection establishment
//! - **Zero-Copy Encoding**: Efficient binary processing using `bytes::BytesMut` for network operations
//! - **Tokio Integration**: Seamless compatibility with Tokio runtime via `tokio_util::codec`
//! - **Memory Safety**: Strict enforcement of message size limits (1MB default) with configurable constraints
//!
//! ## Architecture Components:
//! - `MqttCodec`: Main dispatcher handling version-specific encoding/decoding logic
//! - `MqttPacket`: Unified representation of all protocol versions' packet types
//! - `version::ProtocolVersion`: Detection mechanism for protocol handshake
//! - Error handling with dedicated `EncodeError`/`DecodeError` types
//!

#[macro_use]
mod utils;

/// Error types for encoding/decoding operations
pub mod error;

/// Shared types and constants for MQTT protocol
pub mod types;

/// MQTT v3.1.1 protocol implementation
pub mod v3;

/// MQTT v5.0 protocol implementation
pub mod v5;

/// Protocol version detection and negotiation
pub mod version;

/// Main MQTT protocol codec implementation
///
/// Handles version negotiation and provides unified interface for:
/// - MQTT v3.1.1
/// - MQTT v5.0
/// - Protocol version detection
#[derive(Debug)]
pub enum MqttCodec {
    /// MQTT v3.1.1 codec
    V3(v3::Codec),
    /// MQTT v5.0 codec
    V5(v5::Codec),
    /// Protocol version detection codec (used during initial handshake)
    Version(version::VersionCodec),
}

/// Decoded MQTT protocol packets
///
/// Represents all possible packet types across supported protocol versions
/// plus version detection results during handshake
#[derive(Debug)]
pub enum MqttPacket {
    /// MQTT v3.1.1 protocol packet
    V3(v3::Packet),
    /// MQTT v5.0 protocol packet
    V5(v5::Packet),
    /// Protocol version detection result
    Version(version::ProtocolVersion),
}

impl tokio_util::codec::Encoder<MqttPacket> for MqttCodec {
    type Error = error::EncodeError;

    /// Encodes MQTT packets according to active protocol version
    ///
    /// # Example
    /// ```
    /// use bytes::BytesMut;
    /// use rmqtt_codec::{MqttCodec, MqttPacket, v3};
    /// use tokio_util::codec::Encoder;
    ///
    /// let mut codec = MqttCodec::V3(v3::Codec::new(1024*1024));
    /// let mut buffer = BytesMut::new();
    /// let packet = MqttPacket::V3(v3::Packet::PingRequest);
    /// codec.encode(packet, &mut buffer).unwrap();
    /// ```
    #[inline]
    fn encode(&mut self, item: MqttPacket, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        match self {
            MqttCodec::V3(codec) => match item {
                MqttPacket::V3(p) => {
                    codec.encode(p, dst)?;
                }
                _ => return Err(error::EncodeError::MalformedPacket),
            },
            MqttCodec::V5(codec) => match item {
                MqttPacket::V5(p) => {
                    codec.encode(p, dst)?;
                }
                _ => return Err(error::EncodeError::MalformedPacket),
            },
            MqttCodec::Version(_) => return Err(error::EncodeError::UnsupportedVersion),
        };
        Ok(())
    }
}

impl tokio_util::codec::Decoder for MqttCodec {
    type Item = (MqttPacket, u32);
    type Error = error::DecodeError;

    /// Decodes network bytes into MQTT packets
    ///
    /// Returns tuple containing:
    /// - Decoded packet
    /// - Number of bytes consumed from input buffer
    ///
    /// # Example
    /// ```
    /// use bytes::{BytesMut, BufMut};
    /// use rmqtt_codec::{MqttCodec, v3};
    /// use tokio_util::codec::Decoder;
    ///
    /// let mut codec = MqttCodec::V3(v3::Codec::new(1024*1024));
    /// let mut buffer = BytesMut::new();
    /// buffer.put_slice(b"\x30\x00"); // Publish packet
    /// let packet = codec.decode(&mut buffer);
    /// ```
    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let p = match self {
            MqttCodec::V3(codec) => codec.decode(src)?.map(|(p, remaining)| (MqttPacket::V3(p), remaining)),
            MqttCodec::V5(codec) => codec.decode(src)?.map(|(p, remaining)| (MqttPacket::V5(p), remaining)),
            MqttCodec::Version(codec) => codec.decode(src)?.map(|v| (MqttPacket::Version(v), 0)),
        };
        Ok(p)
    }
}
