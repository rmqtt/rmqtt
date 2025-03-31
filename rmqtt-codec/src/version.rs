use bytes::BytesMut;

use tokio_util::codec::{Decoder, Encoder};

use crate::error::{DecodeError, EncodeError};
use crate::types::{packet_type, MQISDP, MQTT, MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use crate::utils;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProtocolVersion {
    MQTT3,
    MQTT5,
}

#[derive(Debug)]
pub struct VersionCodec;

impl Decoder for VersionCodec {
    type Item = ProtocolVersion;
    type Error = DecodeError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let len = src.len();
        if len < 2 {
            return Ok(None);
        }

        let src_slice = src.as_ref();
        let first_byte = src_slice[0];
        match utils::decode_variable_length(&src_slice[1..])? {
            Some((_, mut consumed)) => {
                consumed += 1;

                if first_byte == packet_type::CONNECT {
                    if len <= consumed + 6 {
                        return Ok(None);
                    }

                    let len = u16::from_be_bytes(
                        src[consumed..consumed + 2].try_into().map_err(|_| DecodeError::InvalidProtocol)?,
                    );

                    ensure!(
                        (len == 4 && &src[consumed + 2..consumed + 6] == MQTT)
                            || (len == 6 && &src[consumed + 2..consumed + 8] == MQISDP),
                        DecodeError::InvalidProtocol
                    );

                    match src[consumed + 2 + len as usize] {
                        MQTT_LEVEL_31 => Ok(Some(ProtocolVersion::MQTT3)),
                        MQTT_LEVEL_311 => Ok(Some(ProtocolVersion::MQTT3)),
                        MQTT_LEVEL_5 => Ok(Some(ProtocolVersion::MQTT5)),
                        _ => Err(DecodeError::InvalidProtocol),
                    }
                } else {
                    Err(DecodeError::UnsupportedPacketType)
                }
            }
            None => Ok(None),
        }
    }
}

impl Encoder<ProtocolVersion> for VersionCodec {
    type Error = EncodeError;

    fn encode(&mut self, _: ProtocolVersion, _: &mut BytesMut) -> Result<(), Self::Error> {
        Err(EncodeError::UnsupportedVersion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_connect_packets() {
        let mut buf = BytesMut::from(
            b"\x10\x7f\x7f\x00\x04MQTT\x06\xC0\x00\x3C\x00\x0512345\x00\x04user\x00\x04pass".as_ref(),
        );
        assert_eq!(
            VersionCodec.decode(&mut buf).map_err(|e| matches!(e, DecodeError::InvalidProtocol)),
            Err(true)
        );

        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT\x04\xc0\0\x0f\0\x02d1\0|testhub.".as_ref());
        assert_eq!(ProtocolVersion::MQTT3, VersionCodec.decode(&mut buf).unwrap().unwrap());

        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT\x05\xc0\0\x0f\0\x02d1\0|testhub.".as_ref());
        assert_eq!(ProtocolVersion::MQTT5, VersionCodec.decode(&mut buf).unwrap().unwrap());

        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT\x05".as_ref());
        assert_eq!(ProtocolVersion::MQTT5, VersionCodec.decode(&mut buf).unwrap().unwrap());

        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04".as_ref());
        assert_eq!(None, VersionCodec.decode(&mut buf).unwrap());

        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT".as_ref());
        assert_eq!(None, VersionCodec.decode(&mut buf).unwrap());
    }
}
