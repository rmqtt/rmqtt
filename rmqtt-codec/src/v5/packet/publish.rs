use std::{num::NonZeroU16, num::NonZeroU32};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;
use serde::{Deserialize, Serialize};

use crate::error::{DecodeError, EncodeError};
use crate::types::QoS;
use crate::utils::{self, timestamp_millis, write_variable_length, Decode, Encode, Property};
use crate::v5::{encode::*, property_type as pt, UserProperties};
//
// /// PUBLISH message
// #[derive(PartialEq, Eq, Clone)]
// pub struct Publish {
//     /// this might be re-delivery of an earlier attempt to send the Packet.
//     pub dup: bool,
//     pub retain: bool,
//     /// the level of assurance for delivery of an Application Message.
//     pub qos: QoS,
//     /// only present in PUBLISH Packets where the QoS level is 1 or 2.
//     pub packet_id: Option<NonZeroU16>,
//     pub topic: ByteString,
//     pub payload: Bytes,
//     pub properties: PublishProperties,
// }
//
// impl fmt::Debug for Publish {
//     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
//         f.debug_struct("Publish")
//             .field("packet_id", &self.packet_id)
//             .field("topic", &self.topic)
//             .field("dup", &self.dup)
//             .field("retain", &self.retain)
//             .field("qos", &self.qos)
//             .field("properties", &self.properties)
//             .field("payload", &"<REDACTED>")
//             .finish()
//     }
// }

pub(crate) type Publish = crate::types::Publish;

#[derive(Debug, PartialEq, Eq, Clone, Default, Serialize, Deserialize)]
pub struct PublishProperties {
    pub topic_alias: Option<NonZeroU16>,
    pub correlation_data: Option<Bytes>,
    pub message_expiry_interval: Option<NonZeroU32>,
    pub content_type: Option<ByteString>,
    pub user_properties: UserProperties,
    pub is_utf8_payload: bool,
    pub response_topic: Option<ByteString>,
    pub subscription_ids: Vec<NonZeroU32>,
}

impl Publish {
    pub(crate) fn decode(mut src: Bytes, packet_flags: u8) -> Result<Self, DecodeError> {
        let topic = ByteString::decode(&mut src)?;
        let qos = QoS::try_from((packet_flags & 0b0110) >> 1)?;
        let packet_id = if qos == QoS::AtMostOnce {
            None
        } else {
            Some(NonZeroU16::decode(&mut src)?) // packet id = 0 encountered
        };

        let properties = parse_publish_properties(&mut src)?;
        let payload = src;

        Ok(Self {
            dup: (packet_flags & 0b1000) == 0b1000,
            qos,
            retain: (packet_flags & 0b0001) == 0b0001,
            topic,
            packet_id,
            payload,
            properties: Some(properties),
            delay_interval: None,
            create_time: Some(timestamp_millis()),
        })
    }
}

impl std::convert::From<UserProperties> for PublishProperties {
    fn from(props: UserProperties) -> Self {
        PublishProperties { user_properties: props, ..Default::default() }
    }
}

fn parse_publish_properties(src: &mut Bytes) -> Result<PublishProperties, DecodeError> {
    let prop_src = &mut utils::take_properties(src)?;

    let mut message_expiry_interval = None;
    let mut topic_alias = None;
    let mut content_type = None;
    let mut correlation_data = None;
    let mut subscription_ids = Vec::new();
    let mut response_topic = None;
    let mut is_utf8_payload = None;
    let mut user_props = Vec::new();

    while prop_src.has_remaining() {
        match prop_src.get_u8() {
            pt::UTF8_PAYLOAD => is_utf8_payload.read_value(prop_src)?,
            pt::MSG_EXPIRY_INT => message_expiry_interval.read_value(prop_src)?,
            pt::CONTENT_TYPE => content_type.read_value(prop_src)?,
            pt::RESP_TOPIC => response_topic.read_value(prop_src)?,
            pt::CORR_DATA => correlation_data.read_value(prop_src)?,
            pt::SUB_ID => {
                let id = utils::decode_variable_length_cursor(prop_src)?;
                subscription_ids.push(NonZeroU32::new(id).ok_or(DecodeError::MalformedPacket)?);
            }
            pt::TOPIC_ALIAS => topic_alias.read_value(prop_src)?,
            pt::USER => user_props.push(<(ByteString, ByteString)>::decode(prop_src)?),
            _ => return Err(DecodeError::MalformedPacket),
        }
    }

    Ok(PublishProperties {
        message_expiry_interval,
        topic_alias,
        content_type,
        correlation_data,
        subscription_ids,
        response_topic,
        is_utf8_payload: is_utf8_payload.unwrap_or(false),
        user_properties: user_props,
    })
}

impl EncodeLtd for Publish {
    fn encoded_size(&self, _limit: u32) -> usize {
        let packet_id_size = if self.qos == QoS::AtMostOnce { 0 } else { 2 };
        self.topic.encoded_size()
            + packet_id_size
            + self
                .properties
                .as_ref()
                .map(|p| p.encoded_size(_limit))
                .unwrap_or_else(|| PublishProperties::default().encoded_size(_limit))
            + self.payload.len()
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        let start_len = buf.len();
        self.topic.encode(buf)?;
        if self.qos == QoS::AtMostOnce {
            if self.packet_id.is_some() {
                return Err(EncodeError::MalformedPacket); // packet id must not be set
            }
        } else {
            self.packet_id.ok_or(EncodeError::PacketIdRequired)?.encode(buf)?;
        }
        if let Some(prop) = &self.properties {
            prop.encode(buf, size - (buf.len() - start_len + self.payload.len()) as u32)?;
        } else {
            PublishProperties::default()
                .encode(buf, size - (buf.len() - start_len + self.payload.len()) as u32)?;
        }
        buf.put(self.payload.as_ref());
        Ok(())
    }
}

impl EncodeLtd for PublishProperties {
    fn encoded_size(&self, _limit: u32) -> usize {
        let prop_len = encoded_property_size(&self.topic_alias)
            + encoded_property_size(&self.correlation_data)
            + encoded_property_size(&self.message_expiry_interval)
            + encoded_property_size(&self.content_type)
            + encoded_property_size_default(&self.is_utf8_payload, false)
            + encoded_property_size(&self.response_topic)
            + self
                .subscription_ids
                .iter()
                .fold(0, |acc, id| acc + 1 + var_int_len(id.get() as usize) as usize)
            + self.user_properties.encoded_size();
        prop_len + var_int_len(prop_len) as usize
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        let prop_len = var_int_len_from_size(size);
        utils::write_variable_length(prop_len, buf);
        encode_property(&self.topic_alias, pt::TOPIC_ALIAS, buf)?;
        encode_property(&self.correlation_data, pt::CORR_DATA, buf)?;
        encode_property(&self.message_expiry_interval, pt::MSG_EXPIRY_INT, buf)?;
        encode_property(&self.content_type, pt::CONTENT_TYPE, buf)?;
        encode_property_default(&self.is_utf8_payload, false, pt::UTF8_PAYLOAD, buf)?;
        encode_property(&self.response_topic, pt::RESP_TOPIC, buf)?;
        for sub_id in self.subscription_ids.iter() {
            buf.put_u8(pt::SUB_ID);
            write_variable_length(sub_id.get(), buf);
        }
        self.user_properties.encode(buf)
    }
}
