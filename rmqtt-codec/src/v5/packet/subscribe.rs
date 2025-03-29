use std::num::{NonZeroU16, NonZeroU32};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;

use super::ack_props;
use crate::error::{DecodeError, EncodeError};
use crate::types::QoS;
use crate::utils::{self, write_variable_length, Decode, Encode};
use crate::v5::{encode::*, property_type as pt, UserProperties, UserProperty};

/// Represents SUBSCRIBE packet
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Subscribe {
    /// Packet Identifier
    pub packet_id: NonZeroU16,
    /// Subscription Identifier
    pub id: Option<NonZeroU32>,
    pub user_properties: UserProperties,
    /// the list of Topic Filters and QoS to which the Client wants to subscribe.
    pub topic_filters: Vec<(ByteString, SubscriptionOptions)>,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct SubscriptionOptions {
    pub qos: QoS,
    pub no_local: bool,
    pub retain_as_published: bool,
    pub retain_handling: RetainHandling,
}

impl Default for SubscriptionOptions {
    fn default() -> Self {
        Self {
            qos: QoS::AtMostOnce,
            no_local: false,
            retain_as_published: false,
            retain_handling: RetainHandling::AtSubscribe,
        }
    }
}

prim_enum! {
    pub enum RetainHandling {
        AtSubscribe = 0,
        AtSubscribeNew = 1,
        NoAtSubscribe = 2
    }
}

impl From<RetainHandling> for u8 {
    fn from(v: RetainHandling) -> Self {
        match v {
            RetainHandling::AtSubscribe => 0,
            RetainHandling::AtSubscribeNew => 1,
            RetainHandling::NoAtSubscribe => 2,
        }
    }
}

/// Represents SUBACK packet
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub properties: UserProperties,
    pub reason_string: Option<ByteString>,
    /// corresponds to a Topic Filter in the SUBSCRIBE Packet being acknowledged.
    pub status: Vec<SubscribeAckReason>,
}

/// Represents UNSUBSCRIBE packet
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Unsubscribe {
    /// Packet Identifier
    pub packet_id: NonZeroU16,
    pub user_properties: UserProperties,
    /// the list of Topic Filters that the Client wishes to unsubscribe from.
    pub topic_filters: Vec<ByteString>,
}

/// Represents UNSUBACK packet
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct UnsubscribeAck {
    /// Packet Identifier
    pub packet_id: NonZeroU16,
    pub properties: UserProperties,
    pub reason_string: Option<ByteString>,
    pub status: Vec<UnsubscribeAckReason>,
}

prim_enum! {
    /// SUBACK reason codes
    #[derive(Deserialize, Serialize)]
    pub enum SubscribeAckReason {
        GrantedQos0 = 0,
        GrantedQos1 = 1,
        GrantedQos2 = 2,
        UnspecifiedError = 128,
        ImplementationSpecificError = 131,
        NotAuthorized = 135,
        TopicFilterInvalid = 143,
        PacketIdentifierInUse = 145,
        QuotaExceeded = 151,
        SharedSubscriptionNotSupported = 158,
        SubscriptionIdentifiersNotSupported = 161,
        WildcardSubscriptionsNotSupported = 162
    }
}

impl From<SubscribeAckReason> for u8 {
    fn from(v: SubscribeAckReason) -> Self {
        match v {
            SubscribeAckReason::GrantedQos0 => 0,
            SubscribeAckReason::GrantedQos1 => 1,
            SubscribeAckReason::GrantedQos2 => 2,
            SubscribeAckReason::UnspecifiedError => 128,
            SubscribeAckReason::ImplementationSpecificError => 131,
            SubscribeAckReason::NotAuthorized => 135,
            SubscribeAckReason::TopicFilterInvalid => 143,
            SubscribeAckReason::PacketIdentifierInUse => 145,
            SubscribeAckReason::QuotaExceeded => 151,
            SubscribeAckReason::SharedSubscriptionNotSupported => 158,
            SubscribeAckReason::SubscriptionIdentifiersNotSupported => 161,
            SubscribeAckReason::WildcardSubscriptionsNotSupported => 162,
        }
    }
}

prim_enum! {
    /// UNSUBACK reason codes
    #[derive(Deserialize, Serialize)]
    pub enum UnsubscribeAckReason {
        Success = 0,
        NoSubscriptionExisted = 17,
        UnspecifiedError = 128,
        ImplementationSpecificError = 131,
        NotAuthorized = 135,
        TopicFilterInvalid = 143,
        PacketIdentifierInUse = 145
    }
}

impl From<UnsubscribeAckReason> for u8 {
    fn from(v: UnsubscribeAckReason) -> Self {
        match v {
            UnsubscribeAckReason::Success => 0,
            UnsubscribeAckReason::NoSubscriptionExisted => 17,
            UnsubscribeAckReason::UnspecifiedError => 128,
            UnsubscribeAckReason::ImplementationSpecificError => 131,
            UnsubscribeAckReason::NotAuthorized => 135,
            UnsubscribeAckReason::TopicFilterInvalid => 143,
            UnsubscribeAckReason::PacketIdentifierInUse => 145,
        }
    }
}

impl Subscribe {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let packet_id = NonZeroU16::decode(src)?;
        let prop_src = &mut utils::take_properties(src)?;
        let mut sub_id = None;
        let mut user_properties = Vec::new();
        while prop_src.has_remaining() {
            let prop_id = prop_src.get_u8();
            match prop_id {
                pt::SUB_ID => {
                    ensure!(sub_id.is_none(), DecodeError::MalformedPacket); // can't appear twice
                    let val = utils::decode_variable_length_cursor(prop_src)?;
                    sub_id = Some(NonZeroU32::new(val).ok_or(DecodeError::MalformedPacket)?);
                }
                pt::USER => user_properties.push(UserProperty::decode(prop_src)?),
                _ => return Err(DecodeError::MalformedPacket),
            }
        }

        let mut topic_filters = Vec::new();
        while src.has_remaining() {
            let topic = ByteString::decode(src)?;
            let opts = SubscriptionOptions::decode(src)?;
            topic_filters.push((topic, opts));
        }

        Ok(Self { packet_id, id: sub_id, user_properties, topic_filters })
    }
}

impl SubscribeAck {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let packet_id = NonZeroU16::decode(src)?;
        let (properties, reason_string) = ack_props::decode(src)?;
        let mut status = Vec::with_capacity(src.remaining());
        for code in src.as_ref().iter().copied() {
            status.push(code.try_into()?);
        }
        Ok(Self { packet_id, properties, reason_string, status })
    }
}

impl Unsubscribe {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let packet_id = NonZeroU16::decode(src)?;

        let prop_src = &mut utils::take_properties(src)?;
        let mut user_properties = Vec::new();
        while prop_src.has_remaining() {
            let prop_id = prop_src.get_u8();
            match prop_id {
                pt::USER => user_properties.push(UserProperty::decode(prop_src)?),
                _ => return Err(DecodeError::MalformedPacket),
            }
        }

        let mut topic_filters = Vec::new();
        while src.remaining() > 0 {
            topic_filters.push(ByteString::decode(src)?);
        }

        Ok(Self { packet_id, user_properties, topic_filters })
    }
}

impl UnsubscribeAck {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let packet_id = NonZeroU16::decode(src)?;
        let (properties, reason_string) = ack_props::decode(src)?;
        let mut status = Vec::with_capacity(src.remaining());
        for code in src.as_ref().iter().copied() {
            status.push(code.try_into()?);
        }
        Ok(Self { packet_id, properties, reason_string, status })
    }
}

impl EncodeLtd for Subscribe {
    fn encoded_size(&self, _limit: u32) -> usize {
        let prop_len = self.id.map_or(0, |v| 1 + var_int_len(v.get() as usize) as usize) // +1 to account for property type byte
            + self.user_properties.encoded_size();
        let payload_len =
            self.topic_filters.iter().fold(0, |acc, (filter, _opts)| acc + filter.encoded_size() + 1);
        self.packet_id.encoded_size() + var_int_len(prop_len) as usize + prop_len + payload_len
    }

    fn encode(&self, buf: &mut BytesMut, _: u32) -> Result<(), EncodeError> {
        self.packet_id.encode(buf)?;

        // encode properties
        let prop_len = self.id.map_or(0, |v| 1 + var_int_len(v.get() as usize))
            + self.user_properties.encoded_size() as u32; // safe: size was already checked against maximum
        utils::write_variable_length(prop_len, buf);

        if let Some(id) = self.id {
            buf.put_u8(pt::SUB_ID);
            write_variable_length(id.get(), buf);
        }

        self.user_properties.encode(buf)?;

        // payload
        for (filter, opts) in self.topic_filters.iter() {
            filter.encode(buf)?;
            opts.encode(buf)?;
        }

        Ok(())
    }
}

impl Decode for SubscriptionOptions {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        ensure!(src.has_remaining(), DecodeError::InvalidLength);
        let val = src.get_u8();
        let qos = (val & 0b0000_0011).try_into()?;
        let retain_handling = ((val & 0b0011_0000) >> 4).try_into()?;
        Ok(SubscriptionOptions {
            qos,
            no_local: val & 0b0000_0100 != 0,
            retain_as_published: val & 0b0000_1000 != 0,
            retain_handling,
        })
    }
}

impl Encode for SubscriptionOptions {
    fn encoded_size(&self) -> usize {
        1
    }
    fn encode(&self, buf: &mut BytesMut) -> Result<(), EncodeError> {
        buf.put_u8(
            u8::from(self.qos)
                | ((self.no_local as u8) << 2)
                | ((self.retain_as_published as u8) << 3)
                | (u8::from(self.retain_handling) << 4),
        );
        Ok(())
    }
}

impl EncodeLtd for SubscribeAck {
    fn encoded_size(&self, limit: u32) -> usize {
        let len = self.status.len();
        if len > (u32::MAX - 2) as usize {
            return usize::MAX; // bail to avoid overflow
        }

        2 + ack_props::encoded_size(&self.properties, &self.reason_string, limit - 2 - len as u32) + len
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        self.packet_id.encode(buf)?;
        let len = self.status.len() as u32; // safe: max size checked already
        ack_props::encode(&self.properties, &self.reason_string, buf, size - 2 - len)?;
        for &reason in self.status.iter() {
            buf.put_u8(reason.into());
        }
        Ok(())
    }
}

impl EncodeLtd for Unsubscribe {
    fn encoded_size(&self, _limit: u32) -> usize {
        let prop_len = self.user_properties.encoded_size();
        2 + var_int_len(prop_len) as usize
            + prop_len
            + self.topic_filters.iter().fold(0, |acc, filter| acc + 2 + filter.len())
    }

    fn encode(&self, buf: &mut BytesMut, _size: u32) -> Result<(), EncodeError> {
        self.packet_id.encode(buf)?;

        // properties
        let prop_len = self.user_properties.encoded_size();
        utils::write_variable_length(prop_len as u32, buf); // safe: max size check is done already
        self.user_properties.encode(buf)?;

        // payload
        for filter in self.topic_filters.iter() {
            filter.encode(buf)?;
        }
        Ok(())
    }
}

impl EncodeLtd for UnsubscribeAck {
    // todo: almost identical to SUBACK
    fn encoded_size(&self, limit: u32) -> usize {
        let len = self.status.len();
        2 + len + ack_props::encoded_size(&self.properties, &self.reason_string, reduce_limit(limit, 2 + len))
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        self.packet_id.encode(buf)?;
        let len = self.status.len() as u32;

        ack_props::encode(&self.properties, &self.reason_string, buf, size - 2 - len)?;
        for &reason in self.status.iter() {
            buf.put_u8(reason.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v5::{Codec, Packet};
    use tokio_util::codec::Decoder;
    use tokio_util::codec::Encoder;

    #[test]
    fn test_sub() {
        let pkt = Subscribe {
            packet_id: 12.try_into().unwrap(),
            id: Some(10.try_into().unwrap()),
            user_properties: vec![("a".into(), "1".into())],
            topic_filters: vec![("test".into(), SubscriptionOptions::default())],
        };

        let size = pkt.encoded_size(99999);
        let mut buf = BytesMut::with_capacity(size);
        pkt.encode(&mut buf, size as u32).unwrap();
        assert_eq!(buf.len(), size);
        assert_eq!(pkt, Subscribe::decode(&mut buf.freeze()).unwrap());

        let pkt = Unsubscribe {
            packet_id: 12.try_into().unwrap(),
            user_properties: vec![("a".into(), "1".into())],
            topic_filters: vec!["test".into()],
        };

        let size = pkt.encoded_size(99999);
        let mut buf = BytesMut::with_capacity(size);
        pkt.encode(&mut buf, size as u32).unwrap();
        assert_eq!(buf.len(), size);
        assert_eq!(pkt, Unsubscribe::decode(&mut buf.freeze()).unwrap());
    }

    #[test]
    fn test_sub_pkt() {
        let pkt = Packet::Subscribe(Subscribe {
            packet_id: 12.try_into().unwrap(),
            id: None,
            user_properties: vec![("a".into(), "1".into())],
            topic_filters: vec![("test".into(), SubscriptionOptions::default())],
        });
        let mut codec = Codec::default();

        let mut buf = BytesMut::new();
        codec.encode(pkt.clone(), &mut buf).unwrap();

        assert_eq!(pkt, codec.decode(&mut buf).unwrap().unwrap().0);
    }

    #[test]
    fn test_sub_ack() {
        let ack = SubscribeAck {
            packet_id: NonZeroU16::new(1).unwrap(),
            properties: Vec::new(),
            reason_string: Some("some reason".into()),
            status: Vec::new(),
        };

        let size = ack.encoded_size(99999);
        let mut buf = BytesMut::with_capacity(size);
        ack.encode(&mut buf, size as u32).unwrap();
        assert_eq!(ack, SubscribeAck::decode(&mut buf.freeze()).unwrap());

        let ack = SubscribeAck {
            packet_id: NonZeroU16::new(1).unwrap(),
            properties: vec![("prop1".into(), "val1".into()), ("prop2".into(), "val2".into())],
            reason_string: None,
            status: vec![SubscribeAckReason::GrantedQos0],
        };
        let size = ack.encoded_size(99999);
        let mut buf = BytesMut::with_capacity(size);
        ack.encode(&mut buf, size as u32).unwrap();
        assert_eq!(ack, SubscribeAck::decode(&mut buf.freeze()).unwrap());

        let ack = UnsubscribeAck {
            packet_id: NonZeroU16::new(1).unwrap(),
            properties: Vec::new(),
            reason_string: Some("some reason".into()),
            status: Vec::new(),
        };
        let mut buf = BytesMut::new();
        let size = ack.encoded_size(99999);
        ack.encode(&mut buf, size as u32).unwrap();
        assert_eq!(ack, UnsubscribeAck::decode(&mut buf.freeze()).unwrap());

        let ack = UnsubscribeAck {
            packet_id: NonZeroU16::new(1).unwrap(),
            properties: vec![("prop1".into(), "val1".into()), ("prop2".into(), "val2".into())],
            reason_string: None,
            status: vec![UnsubscribeAckReason::Success],
        };
        let size = ack.encoded_size(99999);
        let mut buf = BytesMut::with_capacity(size);
        ack.encode(&mut buf, size as u32).unwrap();
        assert_eq!(ack, UnsubscribeAck::decode(&mut buf.freeze()).unwrap());
    }
}
