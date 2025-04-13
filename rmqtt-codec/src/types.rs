use std::fmt;
use std::num::NonZeroU16;

use bytes::Bytes;
use bytestring::ByteString;
use serde::{Deserialize, Serialize};

use crate::v5::PublishProperties;

/// MQTT protocol name for version 3.1.1
pub(crate) const MQTT: &[u8] = b"MQTT";
/// Legacy MQTT protocol name for version 3.1
pub(crate) const MQISDP: &[u8] = b"MQIsdp";
/// Protocol level for MQTT 3.1
pub const MQTT_LEVEL_31: u8 = 3;
/// Protocol level for MQTT 3.1.1
pub const MQTT_LEVEL_311: u8 = 4;
/// Protocol level for MQTT 5.0
pub const MQTT_LEVEL_5: u8 = 5;
/// Bit shift position for Will QoS in Connect flags
pub(crate) const WILL_QOS_SHIFT: u8 = 3;

/// Maximum allowed packet size (268,435,455 bytes)
pub(crate) const MAX_PACKET_SIZE: u32 = 0xF_FF_FF_FF;

/// Represents MQTT protocol version information
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Protocol(pub u8);

impl Protocol {
    /// Gets the protocol name string
    #[inline]
    pub fn name(self) -> &'static str {
        match self {
            Protocol(MQTT_LEVEL_311) => "MQTT",
            Protocol(MQTT_LEVEL_31) => "MQIsdp",
            Protocol(MQTT_LEVEL_5) => "MQTT",
            Protocol(_) => "MQTT",
        }
    }

    /// Gets the protocol level number
    #[inline]
    pub fn level(self) -> u8 {
        self.0
    }
}

impl Default for Protocol {
    /// Default protocol version (3.1.1)
    fn default() -> Self {
        Protocol(MQTT_LEVEL_311)
    }
}

prim_enum! {
    /// Quality of Service levels for message delivery
    ///
    /// Determines the guarantee of message delivery between client and broker
    #[derive(serde::Serialize, serde::Deserialize, PartialOrd, Ord, Hash)]
    pub enum QoS {
        /// At most once delivery (Fire and Forget)
        ///
        /// # Example
        /// Used for non-critical sensor data where occasional loss is acceptable
        AtMostOnce = 0,

        /// At least once delivery (Acknowledged Delivery)
        ///
        /// # Example
        /// Used for important notifications that must be received but can tolerate duplicates
        AtLeastOnce = 1,

        /// Exactly once delivery (Assured Delivery)
        ///
        /// # Example
        /// Used for critical commands where duplication could cause harmful effects
        ExactlyOnce = 2
    }
}

impl QoS {
    /// Gets the numeric value of the QoS level
    #[inline]
    pub fn value(&self) -> u8 {
        match self {
            QoS::AtMostOnce => 0,
            QoS::AtLeastOnce => 1,
            QoS::ExactlyOnce => 2,
        }
    }

    /// Returns the lower of two QoS levels
    ///
    /// # Example
    /// ```
    /// use rmqtt_codec::types::QoS;
    ///
    /// let lower = QoS::ExactlyOnce.less_value(QoS::AtLeastOnce);
    /// assert_eq!(lower, QoS::AtLeastOnce);
    /// ```
    #[inline]
    pub fn less_value(&self, qos: QoS) -> QoS {
        if self.value() < qos.value() {
            *self
        } else {
            qos
        }
    }
}

impl From<QoS> for u8 {
    fn from(v: QoS) -> Self {
        v.value()
    }
}

bitflags::bitflags! {
    /// Connection flags for MQTT CONNECT packet
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ConnectFlags: u8 {
        /// Username flag (bit 7)
        const USERNAME    = 0b1000_0000;
        /// Password flag (bit 6)
        const PASSWORD    = 0b0100_0000;
        /// Will retain flag (bit 5)
        const WILL_RETAIN = 0b0010_0000;
        /// Will QoS mask (bits 4-3)
        const WILL_QOS    = 0b0001_1000;
        /// Will flag (bit 2)
        const WILL        = 0b0000_0100;
        /// Clean session flag (bit 1)
        const CLEAN_START = 0b0000_0010;
    }
}

bitflags::bitflags! {
    /// Connection acknowledgment flags for MQTT CONNACK packet
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ConnectAckFlags: u8 {
        /// Session present flag (bit 0)
        const SESSION_PRESENT = 0b0000_0001;
    }
}

/// Packet type identifiers and masks
pub(super) mod packet_type {
    /// CONNECT packet type (0x10)
    pub(crate) const CONNECT: u8 = 0b0001_0000;
    /// CONNACK packet type (0x20)
    pub(crate) const CONNACK: u8 = 0b0010_0000;
    /// PUBLISH packet type range start (0x30)
    pub(crate) const PUBLISH_START: u8 = 0b0011_0000;
    /// PUBLISH packet type range end (0x3F)
    pub(crate) const PUBLISH_END: u8 = 0b0011_1111;
    /// PUBACK packet type (0x40)
    pub(crate) const PUBACK: u8 = 0b0100_0000;
    /// PUBREC packet type (0x50)
    pub(crate) const PUBREC: u8 = 0b0101_0000;
    /// PUBREL packet type (0x62)
    pub(crate) const PUBREL: u8 = 0b0110_0010;
    /// PUBCOMP packet type (0x70)
    pub(crate) const PUBCOMP: u8 = 0b0111_0000;
    /// SUBSCRIBE packet type (0x82)
    pub(crate) const SUBSCRIBE: u8 = 0b1000_0010;
    /// SUBACK packet type (0x90)
    pub(crate) const SUBACK: u8 = 0b1001_0000;
    /// UNSUBSCRIBE packet type (0xA2)
    pub(crate) const UNSUBSCRIBE: u8 = 0b1010_0010;
    /// UNSUBACK packet type (0xB0)
    pub(crate) const UNSUBACK: u8 = 0b1011_0000;
    /// PINGREQ packet type (0xC0)
    pub(crate) const PINGREQ: u8 = 0b1100_0000;
    /// PINGRESP packet type (0xD0)
    pub(crate) const PINGRESP: u8 = 0b1101_0000;
    /// DISCONNECT packet type (0xE0)
    pub(crate) const DISCONNECT: u8 = 0b1110_0000;
    /// AUTH packet type (0xF0) - MQTT 5.0 only
    pub(crate) const AUTH: u8 = 0b1111_0000;
}

/// Represents the fixed header of an MQTT packet
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct FixedHeader {
    /// First byte containing packet type and flags
    pub(crate) first_byte: u8,
    /// Remaining length of the packet (variable header + payload)
    pub(crate) remaining_length: u32,
}

/// MQTT PUBLISH packet structure
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Publish {
    /// Duplicate delivery flag
    pub dup: bool,
    /// Retain message flag
    pub retain: bool,
    /// Quality of Service level
    pub qos: QoS,
    /// Topic name to publish to
    pub topic: ByteString,
    /// Packet identifier (required for QoS 1 and 2)
    pub packet_id: Option<NonZeroU16>,
    /// Message payload
    pub payload: Bytes,

    /// MQTT 5.0 properties (None for MQTT 3.1.1)
    pub properties: Option<PublishProperties>,
    /// Delayed publish interval in seconds
    pub delay_interval: Option<u32>,
    /// Message creation timestamp
    pub create_time: Option<i64>,
}

impl fmt::Debug for Publish {
    /// Security-conscious debug implementation that redacts payload
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Publish")
            .field("packet_id", &self.packet_id)
            .field("topic", &self.topic)
            .field("dup", &self.dup)
            .field("retain", &self.retain)
            .field("qos", &self.qos)
            .field("payload", &"<REDACTED>")
            .field("properties", &self.properties)
            .field("delay_interval", &self.delay_interval)
            .field("create_time", &self.create_time)
            .finish()
    }
}
