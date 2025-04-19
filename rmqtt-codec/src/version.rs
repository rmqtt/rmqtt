use bytes::BytesMut;
use tokio_util::codec::{Decoder, Encoder};

use crate::error::{DecodeError, EncodeError};
use crate::types::{packet_type, MQISDP, MQTT, MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use crate::utils;

/// Represents supported MQTT protocol versions
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProtocolVersion {
    /// MQTT version 3.1 or 3.1.1
    MQTT3,
    /// MQTT version 5.0
    MQTT5,
}

/// Codec for detecting MQTT protocol version from initial handshake
///
/// This codec is specifically designed to handle the initial CONNECT packet
/// and determine the protocol version before switching to version-specific codecs
#[derive(Debug)]
pub struct VersionCodec;

impl Decoder for VersionCodec {
    type Item = ProtocolVersion;
    type Error = DecodeError;

    /// Decodes the protocol version from the initial CONNECT packet
    ///
    /// # Process
    /// 1. Checks for minimum packet length
    /// 2. Verifies CONNECT packet type
    /// 3. Reads variable length header
    /// 4. Validates protocol name (MQTT/MQIsdp)
    /// 5. Extracts protocol level byte
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

                    let protocol_len = u16::from_be_bytes(
                        src[consumed..consumed + 2].try_into().map_err(|_| DecodeError::InvalidProtocol)?,
                    );

                    // Validate protocol name matches MQTT spec
                    ensure!(
                        (protocol_len == 4 && &src[consumed + 2..consumed + 6] == MQTT)
                            || (protocol_len == 6 && &src[consumed + 2..consumed + 8] == MQISDP),
                        DecodeError::InvalidProtocol
                    );

                    // Extract protocol level byte (position after protocol name)
                    match src[consumed + 2 + protocol_len as usize] {
                        MQTT_LEVEL_31 | MQTT_LEVEL_311 => Ok(Some(ProtocolVersion::MQTT3)),
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

    /// Encoding not supported for version detection codec
    ///
    /// This codec is only used for initial protocol detection,
    /// actual packet encoding should be handled by version-specific codecs
    fn encode(&mut self, _: ProtocolVersion, _: &mut BytesMut) -> Result<(), Self::Error> {
        Err(EncodeError::UnsupportedVersion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    /// Test invalid protocol format detection
    #[test]
    fn test_invalid_protocol() {
        let mut buf = BytesMut::from(
            b"\x10\x7f\x7f\x00\x04MQTT\x06\xC0\x00\x3C\x00\x0512345\x00\x04user\x00\x04pass".as_ref(),
        );
        assert!(matches!(VersionCodec.decode(&mut buf), Err(DecodeError::InvalidProtocol)));
    }

    /// Test valid MQTT 3.1.1 protocol detection
    #[test]
    fn test_mqtt3_protocol_detection() {
        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT\x04\xc0\0\x0f\0\x02d1\0|testhub.".as_ref());
        assert_eq!(VersionCodec.decode(&mut buf).unwrap(), Some(ProtocolVersion::MQTT3));
    }

    /// Test valid MQTT 5.0 protocol detection
    #[test]
    fn test_mqtt5_protocol_detection() {
        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT\x05\xc0\0\x0f\0\x02d1\0|testhub.".as_ref());
        assert_eq!(VersionCodec.decode(&mut buf).unwrap(), Some(ProtocolVersion::MQTT5));
    }

    /// Test partial packet handling
    #[test]
    fn test_partial_packet_handling() {
        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04MQTT\x05".as_ref());
        assert_eq!(VersionCodec.decode(&mut buf).unwrap(), Some(ProtocolVersion::MQTT5));

        let mut buf = BytesMut::from(b"\x10\x98\x02\0\x04".as_ref());
        assert_eq!(VersionCodec.decode(&mut buf).unwrap(), None);
    }
}
