use std::fmt;
use std::num::NonZeroU16;

use bytes::Bytes;
use bytestring::ByteString;

use crate::v5::PublishProperties;

pub(crate) const MQTT: &[u8] = b"MQTT";
pub(crate) const MQISDP: &[u8] = b"MQIsdp";
pub const MQTT_LEVEL_31: u8 = 3;
pub const MQTT_LEVEL_311: u8 = 4;
pub const MQTT_LEVEL_5: u8 = 5;
pub(crate) const WILL_QOS_SHIFT: u8 = 3;

/// Max possible packet size
pub(crate) const MAX_PACKET_SIZE: u32 = 0xF_FF_FF_FF;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct Protocol(pub u8);

impl Protocol {
    #[inline]
    pub fn name(self) -> &'static str {
        match self {
            Protocol(MQTT_LEVEL_311) => "MQTT",
            Protocol(MQTT_LEVEL_31) => "MQIsdp",
            Protocol(MQTT_LEVEL_5) => "MQTT",
            Protocol(_) => "MQTT",
        }
    }

    #[inline]
    pub fn level(self) -> u8 {
        self.0
    }
}

impl Default for Protocol {
    fn default() -> Self {
        Protocol(MQTT_LEVEL_311)
    }
}

prim_enum! {
    /// Quality of Service
    #[derive(serde::Serialize, serde::Deserialize, PartialOrd, Ord, Hash)]
    pub enum QoS {
        /// At most once delivery
        ///
        /// The message is delivered according to the capabilities of the underlying network.
        /// No response is sent by the receiver and no retry is performed by the sender.
        /// The message arrives at the receiver either once or not at all.
        AtMostOnce = 0,
        /// At least once delivery
        ///
        /// This quality of service ensures that the message arrives at the receiver at least once.
        /// A QoS 1 PUBLISH Packet has a Packet Identifier in its variable header
        /// and is acknowledged by a PUBACK Packet.
        AtLeastOnce = 1,
        /// Exactly once delivery
        ///
        /// This is the highest quality of service,
        /// for use when neither loss nor duplication of messages are acceptable.
        /// There is an increased overhead associated with this quality of service.
        ExactlyOnce = 2
    }
}

impl QoS {
    #[inline]
    pub fn value(&self) -> u8 {
        match self {
            QoS::AtMostOnce => 0,
            QoS::AtLeastOnce => 1,
            QoS::ExactlyOnce => 2,
        }
    }

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
        match v {
            QoS::AtMostOnce => 0,
            QoS::AtLeastOnce => 1,
            QoS::ExactlyOnce => 2,
        }
    }
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ConnectFlags: u8 {
        const USERNAME    = 0b1000_0000;
        const PASSWORD    = 0b0100_0000;
        const WILL_RETAIN = 0b0010_0000;
        const WILL_QOS    = 0b0001_1000;
        const WILL        = 0b0000_0100;
        const CLEAN_START = 0b0000_0010;
    }
}

bitflags::bitflags! {
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ConnectAckFlags: u8 {
        const SESSION_PRESENT = 0b0000_0001;
    }
}

pub(super) mod packet_type {
    pub(crate) const CONNECT: u8 = 0b0001_0000;
    pub(crate) const CONNACK: u8 = 0b0010_0000;
    pub(crate) const PUBLISH_START: u8 = 0b0011_0000;
    pub(crate) const PUBLISH_END: u8 = 0b0011_1111;
    pub(crate) const PUBACK: u8 = 0b0100_0000;
    pub(crate) const PUBREC: u8 = 0b0101_0000;
    pub(crate) const PUBREL: u8 = 0b0110_0010;
    pub(crate) const PUBCOMP: u8 = 0b0111_0000;
    pub(crate) const SUBSCRIBE: u8 = 0b1000_0010;
    pub(crate) const SUBACK: u8 = 0b1001_0000;
    pub(crate) const UNSUBSCRIBE: u8 = 0b1010_0010;
    pub(crate) const UNSUBACK: u8 = 0b1011_0000;
    pub(crate) const PINGREQ: u8 = 0b1100_0000;
    pub(crate) const PINGRESP: u8 = 0b1101_0000;
    pub(crate) const DISCONNECT: u8 = 0b1110_0000;
    pub(crate) const AUTH: u8 = 0b1111_0000;
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct FixedHeader {
    /// Fixed Header byte
    pub(crate) first_byte: u8,
    /// the number of bytes remaining within the current packet,
    /// including data in the variable header and the payload.
    pub(crate) remaining_length: u32,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Publish {
    /// this might be re-delivery of an earlier attempt to send the Packet.
    pub dup: bool,
    pub retain: bool,
    /// the level of assurance for delivery of an Application Message.
    pub qos: QoS,
    /// the information channel to which payload data is published.
    pub topic: ByteString,
    /// only present in PUBLISH Packets where the QoS level is 1 or 2.
    pub packet_id: Option<NonZeroU16>,
    /// the Application Message that is being published.
    pub payload: Bytes,

    pub properties: Option<PublishProperties>,
    /// delay publish interval, unit: seconds
    pub delay_interval: Option<u32>,
    pub create_time: Option<TimestampMillis>,
}

impl fmt::Debug for Publish {
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

// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum Disconnect {
//     V3,
//     V5(v5::Disconnect),
// }
//
// #[allow(dead_code)]
// impl Disconnect {
//     #[inline]
//     pub fn reason_code(&self) -> Option<v5::DisconnectReasonCode> {
//         match self {
//             Disconnect::V3 => None,
//             Disconnect::V5(d) => Some(d.reason_code),
//         }
//     }
//
//     #[inline]
//     pub fn reason(&self) -> Option<&ByteString> {
//         match self {
//             Disconnect::V3 => None,
//             Disconnect::V5(d) => d.reason_string.as_ref(),
//         }
//     }
// }
//
// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum PublishAck {
//     V3 { packet_id: NonZeroU16 },
//     V5(v5::PublishAck),
// }
//
// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum PublishAck2 {
//     V3 { packet_id: NonZeroU16 },
//     V5(v5::PublishAck2),
// }
//
// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum SubscribeAck {
//     V3 {
//         packet_id: NonZeroU16,
//         status: Vec<v3::SubscribeReturnCode>,
//     },
//     V5(v5::SubscribeAck),
// }
//
// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum UnsubscribeAck {
//     V3 { packet_id: NonZeroU16 },
//     V5(v5::UnsubscribeAck),
// }
//
// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum Connect {
//     V3(v3::Connect),
//     V5(v5::Connect),
// }
//
// #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
// pub enum ConnectAck {
//     V3(v3::ConnectAck),
//     V5(v5::ConnectAck),
// }

// pub type PacketId = NonZeroU16;
pub type TimestampMillis = i64;
