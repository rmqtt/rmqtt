//! MQTT common module - Shared packet types, codec, and utilities

pub mod session;
pub mod error;

// Re-export commonly used types from rmqtt_codec
pub use rmqtt_codec::v3::Packet as PacketV3;
pub use rmqtt_codec::v3::Connect as ConnectPacketInner;
pub use rmqtt_codec::v3::QoS;

// Main Packet type for v3
pub type Packet = PacketV3;

// QoS type alias
pub type QoSLevel = QoS;
pub type QoSTest = QoS;

// Aliases for compatibility
pub type Connect = ConnectPacketInner;
pub type Subscribe = PacketV3;
pub type SubAck = PacketV3;
pub type UnsubAck = PacketV3;

// Protocol version
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    V3,
    V311,
    V5,
}

impl ProtocolVersion {
    pub fn level(&self) -> u8 {
        match self {
            ProtocolVersion::V3 => 3,
            ProtocolVersion::V311 => 4,
            ProtocolVersion::V5 => 5,
        }
    }
}

// Connect Return Code
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectReturnCode {
    Accepted = 0,
    UnacceptableProtocolVersion = 1,
    IdentifierRejected = 2,
    ServerUnavailable = 3,
    BadCredentials = 4,
    NotAuthorized = 5,
}

impl ConnectReturnCode {
    pub fn is_success(&self) -> bool {
        matches!(self, ConnectReturnCode::Accepted)
    }
}

impl TryFrom<u8> for ConnectReturnCode {
    type Error = error::CodecError;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(ConnectReturnCode::Accepted),
            1 => Ok(ConnectReturnCode::UnacceptableProtocolVersion),
            2 => Ok(ConnectReturnCode::IdentifierRejected),
            3 => Ok(ConnectReturnCode::ServerUnavailable),
            4 => Ok(ConnectReturnCode::BadCredentials),
            5 => Ok(ConnectReturnCode::NotAuthorized),
            _ => Err(error::CodecError::InvalidReturnCode(v)),
        }
    }
}

// SubscribeFilter
pub type SubscribeFilter = (bytestring::ByteString, QoS);

// MQTT level constants
pub const MQTT_LEVEL_311: u8 = 4;
pub const MQTT_LEVEL_5: u8 = 5;