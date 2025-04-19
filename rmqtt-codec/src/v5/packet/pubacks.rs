use std::num::NonZeroU16;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use bytestring::ByteString;
use serde::{Deserialize, Serialize};

use super::ack_props;
use crate::error::{DecodeError, EncodeError};
use crate::utils::{Decode, Encode};
use crate::v5::{encode::*, DisconnectReasonCode, ToReasonCode, UserProperties};

const HEADER_LEN: u32 = 2 + 1; // packet id + reason code

/// PUBACK/PUBREC message content
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct PublishAck {
    /// Packet Identifier
    pub packet_id: NonZeroU16,
    pub reason_code: PublishAckReason,
    pub properties: UserProperties,
    pub reason_string: Option<ByteString>,
}

/// PUBREL/PUBCOMP message content
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub struct PublishAck2 {
    /// Packet Identifier
    pub packet_id: NonZeroU16,
    pub reason_code: PublishAck2Reason,
    pub properties: UserProperties,
    pub reason_string: Option<ByteString>,
}

prim_enum! {
    /// PUBACK / PUBREC reason codes
    #[derive(Deserialize, Serialize)]
    pub enum PublishAckReason {
        Success = 0,
        NoMatchingSubscribers = 16,
        UnspecifiedError = 128,
        ImplementationSpecificError = 131,
        NotAuthorized = 135,
        TopicNameInvalid = 144,
        PacketIdentifierInUse = 145,
        QuotaExceeded = 151,
        PayloadFormatInvalid = 153
    }
}

impl PublishAckReason {
    #[inline]
    pub fn should_disconnect(&self) -> bool {
        matches!(
            self,
            PublishAckReason::UnspecifiedError
                | PublishAckReason::ImplementationSpecificError
                | PublishAckReason::NotAuthorized
                | PublishAckReason::TopicNameInvalid
        )
    }
}

impl ToReasonCode for PublishAckReason {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            PublishAckReason::Success | PublishAckReason::NoMatchingSubscribers => {
                DisconnectReasonCode::NormalDisconnection
            }
            PublishAckReason::UnspecifiedError => DisconnectReasonCode::UnspecifiedError,
            PublishAckReason::ImplementationSpecificError => {
                DisconnectReasonCode::ImplementationSpecificError
            }
            PublishAckReason::NotAuthorized => DisconnectReasonCode::NotAuthorized,
            PublishAckReason::TopicNameInvalid => DisconnectReasonCode::TopicNameInvalid,
            PublishAckReason::PacketIdentifierInUse => DisconnectReasonCode::ProtocolError,
            PublishAckReason::QuotaExceeded => DisconnectReasonCode::QuotaExceeded,
            PublishAckReason::PayloadFormatInvalid => DisconnectReasonCode::PayloadFormatInvalid,
        }
    }
}

impl From<PublishAckReason> for u8 {
    fn from(v: PublishAckReason) -> Self {
        match v {
            PublishAckReason::Success => 0,
            PublishAckReason::NoMatchingSubscribers => 16,
            PublishAckReason::UnspecifiedError => 128,
            PublishAckReason::ImplementationSpecificError => 131,
            PublishAckReason::NotAuthorized => 135,
            PublishAckReason::TopicNameInvalid => 144,
            PublishAckReason::PacketIdentifierInUse => 145,
            PublishAckReason::QuotaExceeded => 151,
            PublishAckReason::PayloadFormatInvalid => 153,
        }
    }
}

prim_enum! {
    /// PUBREL / PUBCOMP reason codes
    #[derive(Deserialize, Serialize)]
    pub enum PublishAck2Reason {
        Success = 0,
        PacketIdNotFound = 146
    }
}

impl PublishAck2Reason {
    #[inline]
    pub fn should_disconnect(&self) -> bool {
        matches!(self, PublishAck2Reason::PacketIdNotFound)
    }
}

impl ToReasonCode for PublishAck2Reason {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            PublishAck2Reason::Success => DisconnectReasonCode::NormalDisconnection,
            PublishAck2Reason::PacketIdNotFound => DisconnectReasonCode::ImplementationSpecificError,
        }
    }
}

impl From<PublishAck2Reason> for u8 {
    fn from(v: PublishAck2Reason) -> Self {
        match v {
            PublishAck2Reason::Success => 0,
            PublishAck2Reason::PacketIdNotFound => 146,
        }
    }
}

impl PublishAck {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let packet_id = NonZeroU16::decode(src)?;

        let ack = if src.has_remaining() {
            let reason_code = src.get_u8().try_into()?;
            if src.has_remaining() {
                let (properties, reason_string) = ack_props::decode(src)?;
                ensure!(!src.has_remaining(), DecodeError::InvalidLength); // no data should be left in src
                Self { packet_id, reason_code, properties, reason_string }
            } else {
                Self { packet_id, reason_code, ..Default::default() }
            }
        } else {
            Self { packet_id, ..Default::default() }
        };

        Ok(ack)
    }
}

impl Default for PublishAck {
    fn default() -> Self {
        Self {
            packet_id: NonZeroU16::new(1).unwrap(),
            reason_code: PublishAckReason::Success,
            properties: UserProperties::default(),
            reason_string: None,
        }
    }
}

impl PublishAck2 {
    pub(crate) fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let packet_id = NonZeroU16::decode(src)?;
        let ack = if src.has_remaining() {
            let reason_code = src.get_u8().try_into()?;
            if src.has_remaining() {
                let (properties, reason_string) = ack_props::decode(src)?;
                ensure!(!src.has_remaining(), DecodeError::InvalidLength); // no data should be left in src
                Self { packet_id, reason_code, properties, reason_string }
            } else {
                Self { packet_id, reason_code, ..Default::default() }
            }
        } else {
            Self { packet_id, ..Default::default() }
        };

        Ok(ack)
    }
}

impl Default for PublishAck2 {
    fn default() -> Self {
        Self {
            packet_id: NonZeroU16::new(1).unwrap(),
            reason_code: PublishAck2Reason::Success,
            properties: UserProperties::default(),
            reason_string: None,
        }
    }
}

impl EncodeLtd for PublishAck {
    fn encoded_size(&self, limit: u32) -> usize {
        let prop_len = ack_props::encoded_size(&self.properties, &self.reason_string, limit - HEADER_LEN - 4); // limit - HEADER_LEN - len(packet_len.max())
        HEADER_LEN as usize + prop_len
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        self.packet_id.get().encode(buf)?;
        buf.put_u8(self.reason_code.into());
        ack_props::encode(&self.properties, &self.reason_string, buf, size - HEADER_LEN)?;
        Ok(())
    }
}

impl EncodeLtd for PublishAck2 {
    fn encoded_size(&self, limit: u32) -> usize {
        const HEADER_LEN: u32 = 2 + 1; // fixed header + packet id + reason code
        let prop_len = ack_props::encoded_size(&self.properties, &self.reason_string, limit - HEADER_LEN - 4); // limit - HEADER_LEN - prop_len.max()
        HEADER_LEN as usize + prop_len
    }

    fn encode(&self, buf: &mut BytesMut, size: u32) -> Result<(), EncodeError> {
        self.packet_id.get().encode(buf)?;
        buf.put_u8(self.reason_code.into());
        ack_props::encode(&self.properties, &self.reason_string, buf, size - 3)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(b"\xFF\xFF\x00\x00", 65535, PublishAckReason::Success, vec![], None; "success_empty")]
    #[test_case(b"\x00\x01", 1, PublishAckReason::Success, vec![], None; "success_no_reason")]
    #[test_case(b"\x01\x01\x00", 257, PublishAckReason::Success, vec![], None; "success_no_prop_len")]
    #[test_case(b"\x00\x01\x87", 1, PublishAckReason::NotAuthorized, vec![], None; "no_success_no_prop_len")]
    #[test_case(b"\x00\x01\x83\x00", 1, PublishAckReason::ImplementationSpecificError, vec![], None; "no_success_min")]
    #[test_case(b"\x00\xFF\x80\x0D\x26\x00\x01a\x00\x01b\x1F\x00\x03123", 255, PublishAckReason::UnspecifiedError, vec![("a", "b")], Some("123"); "all_out")]
    fn puback_decode_success(
        input: &'static [u8],
        packet_id: u16,
        reason_code: PublishAckReason,
        properties: Vec<(&'static str, &'static str)>,
        reason_string: Option<&'static str>,
    ) {
        let mut input = input.into();
        let result = PublishAck::decode(&mut input);
        assert_eq!(
            result.unwrap(),
            PublishAck {
                packet_id: packet_id.try_into().unwrap(),
                reason_code,
                properties: properties.into_iter().map(|(k, v)| (k.into(), v.into())).collect(),
                reason_string: reason_string.map(|s| s.into())
            }
        );
        assert_eq!(input.len(), 0);
    }

    #[test_case(b"\x00\x00", DecodeError::MalformedPacket; "packet_id_zero")]
    #[test_case(b"\x00\x01\x00\x01", DecodeError::InvalidLength; "properties_promised")]
    fn puback_decode_must_fail(input: &'static [u8], error: DecodeError) {
        let mut input = input.into();
        let result = PublishAck::decode(&mut input);
        assert_eq!(result.map_err(|e| e.to_string()), Err(error.to_string()));
    }
}
