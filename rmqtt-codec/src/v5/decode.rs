use bytes::Bytes;
use bytestring::ByteString;

use super::{packet::*, UserProperty};
use crate::error::DecodeError;
use crate::types::packet_type;
use crate::utils::Decode;

pub(super) fn decode_packet(mut src: Bytes, first_byte: u8) -> Result<Packet, DecodeError> {
    match first_byte {
        packet_type::PUBLISH_START..=packet_type::PUBLISH_END => {
            Ok(Packet::Publish(Box::new(Publish::decode(src, first_byte & 0b0000_1111)?)))
        }
        packet_type::PUBACK => Ok(Packet::PublishAck(PublishAck::decode(&mut src)?)),
        packet_type::PINGREQ => Ok(Packet::PingRequest),
        packet_type::PINGRESP => Ok(Packet::PingResponse),
        packet_type::SUBSCRIBE => Ok(Packet::Subscribe(Subscribe::decode(&mut src)?)),
        packet_type::SUBACK => Ok(Packet::SubscribeAck(SubscribeAck::decode(&mut src)?)),
        packet_type::UNSUBSCRIBE => Ok(Packet::Unsubscribe(Unsubscribe::decode(&mut src)?)),
        packet_type::UNSUBACK => Ok(Packet::UnsubscribeAck(UnsubscribeAck::decode(&mut src)?)),
        packet_type::CONNECT => Ok(Packet::Connect(Box::new(Connect::decode(&mut src)?))),
        packet_type::CONNACK => Ok(Packet::ConnectAck(Box::new(ConnectAck::decode(&mut src)?))),
        packet_type::DISCONNECT => Ok(Packet::Disconnect(Disconnect::decode(&mut src)?)),
        packet_type::AUTH => Ok(Packet::Auth(Auth::decode(&mut src)?)),
        packet_type::PUBREC => Ok(Packet::PublishReceived(PublishAck::decode(&mut src)?)),
        packet_type::PUBREL => Ok(Packet::PublishRelease(PublishAck2::decode(&mut src)?)),
        packet_type::PUBCOMP => Ok(Packet::PublishComplete(PublishAck2::decode(&mut src)?)),
        _ => Err(DecodeError::UnsupportedPacketType),
    }
}

impl Decode for UserProperty {
    fn decode(src: &mut Bytes) -> Result<Self, DecodeError> {
        let key = ByteString::decode(src)?;
        let val = ByteString::decode(src)?;
        Ok((key, val))
    }
}

#[cfg(test)]
mod tests {
    use bytes::BytesMut;
    use std::num::{NonZeroU16, NonZeroU32};
    use tokio_util::codec::Encoder;

    use super::*;
    use crate::utils::{decode_variable_length, timestamp_millis};
    use crate::v5::UserProperties;

    fn packet_id(v: u16) -> NonZeroU16 {
        NonZeroU16::new(v).unwrap()
    }

    fn assert_decode_packet<B: AsRef<[u8]>>(bytes: B, res: Packet) {
        let bytes = bytes.as_ref();
        let fixed = bytes[0];
        let (_len, consumed) = decode_variable_length(&bytes[1..]).unwrap().unwrap();
        let cur = Bytes::copy_from_slice(&bytes[consumed + 1..]);
        let mut tmp = BytesMut::with_capacity(4096);
        Encoder::encode(&mut crate::v5::codec::Codec::default(), res.clone(), &mut tmp).unwrap();
        let decoded = decode_packet(cur, fixed);
        let res = Ok(&res);
        if decoded.as_ref().map_err(|e| e.to_string()) != res {
            panic!("decoded packet does not match expectations.\nexpected: {:?}\nactual: {:?}\nencoding output for expected: {:X?}", res, decoded, tmp.as_ref());
        }
        //assert_eq!(, Ok(res));
    }

    #[test]
    fn test_decode_connect_packets() {
        assert_eq!(
            Connect::decode(&mut Bytes::from_static(
                b"\x00\x04MQTT\x05\xC0\x00\x3C\x00\x00\x0512345\x00\x04user\x00\x04pass"
            ))
            .unwrap(),
            Connect {
                clean_start: false,
                keep_alive: 60,
                client_id: ByteString::from_static("12345"),
                last_will: None,
                username: Some(ByteString::from_static("user")),
                password: Some(Bytes::from_static(&b"pass"[..])),
                session_expiry_interval_secs: 0,
                auth_method: None,
                auth_data: None,
                request_problem_info: true,
                request_response_info: false,
                receive_max: None,
                topic_alias_max: 0,
                user_properties: Vec::new(),
                max_packet_size: None,
            }
        );

        assert_eq!(
            Connect::decode(&mut Bytes::from_static(
                b"\x00\x04MQTT\x05\x14\x00\x3C\x00\x00\x0512345\x00\x00\x05topic\x00\x07message"
            ))
            .unwrap(),
            Connect {
                clean_start: false,
                keep_alive: 60,
                client_id: ByteString::from_static("12345"),
                last_will: Some(LastWill {
                    qos: QoS::ExactlyOnce,
                    retain: false,
                    topic: ByteString::from_static("topic"),
                    message: Bytes::from_static(&b"message"[..]),
                    will_delay_interval_sec: None,
                    correlation_data: None,
                    message_expiry_interval: None,
                    content_type: None,
                    user_properties: Vec::new(),
                    is_utf8_payload: None,
                    response_topic: None,
                }),
                username: None,
                password: None,
                session_expiry_interval_secs: 0,
                auth_method: None,
                auth_data: None,
                request_problem_info: true,
                request_response_info: false,
                receive_max: None,
                topic_alias_max: 0,
                user_properties: Vec::new(),
                max_packet_size: None,
            }
        );

        assert_eq!(
            Connect::decode(&mut Bytes::from_static(b"\x00\x02MQ00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::InvalidProtocol)),
            Err(true),
        );
        assert_eq!(
            Connect::decode(&mut Bytes::from_static(b"\x00\x04MQAA00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::InvalidProtocol)),
            Err(true),
        );
        assert_eq!(
            Connect::decode(&mut Bytes::from_static(b"\x00\x04MQTT\x0300000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::UnsupportedProtocolLevel)),
            Err(true),
        );
        assert_eq!(
            Connect::decode(&mut Bytes::from_static(b"\x00\x04MQTT\x05\xff00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::ConnectReservedFlagSet)),
            Err(true)
        );

        assert_eq!(
            ConnectAck::decode(&mut Bytes::from_static(b"\x01\x86\x00")).unwrap(),
            ConnectAck {
                session_present: true,
                reason_code: ConnectAckReason::BadUserNameOrPassword,
                ..ConnectAck::default()
            }
        );

        assert_eq!(
            ConnectAck::decode(&mut Bytes::from_static(b"\x03\x86\x00"))
                .map_err(|e| matches!(e, DecodeError::ConnAckReservedFlagSet)),
            Err(true)
        );

        assert_decode_packet(
            b"\x20\x03\x01\x86\x00",
            Packet::ConnectAck(Box::new(ConnectAck {
                session_present: true,
                reason_code: ConnectAckReason::BadUserNameOrPassword,
                ..ConnectAck::default()
            })),
        );

        assert_decode_packet([0b1110_0000, 0], Packet::Disconnect(Disconnect::default()));
    }

    fn default_test_publish() -> Publish {
        Publish {
            dup: false,
            retain: false,
            qos: QoS::AtMostOnce,
            topic: ByteString::default(),
            packet_id: Some(packet_id(1)),
            payload: Bytes::new(),
            properties: Some(PublishProperties::default()),
            delay_interval: None,
            create_time: Some(timestamp_millis()),
        }
    }

    #[test]
    fn test_decode_publish_packets() {
        //assert_eq!(
        //    decode_publish_packet(b"\x00\x05topic\x12\x34"),
        //    Done(&b""[..], ("topic".to_owned(), 0x1234))
        //);

        assert_decode_packet(
            b"\x3d\x0E\x00\x05topic\x43\x21\x00data",
            Packet::Publish(Box::new(Publish {
                dup: true,
                retain: true,
                qos: QoS::ExactlyOnce,
                topic: ByteString::from_static("topic"),
                packet_id: Some(packet_id(0x4321)),
                payload: Bytes::from_static(b"data"),
                ..default_test_publish()
            })),
        );

        assert_decode_packet(
            b"\x30\x0C\x00\x05topic\x00data",
            Packet::Publish(Box::new(Publish {
                dup: false,
                retain: false,
                qos: QoS::AtMostOnce,
                topic: ByteString::from_static("topic"),
                packet_id: None,
                payload: Bytes::from_static(b"data"),
                ..default_test_publish()
            })),
        );

        assert_decode_packet(
            b"\x40\x02\x43\x21",
            Packet::PublishAck(PublishAck {
                packet_id: packet_id(0x4321),
                reason_code: PublishAckReason::Success,
                properties: UserProperties::default(),
                reason_string: None,
            }),
        );
        assert_decode_packet(
            b"\x50\x02\x43\x21",
            Packet::PublishReceived(PublishAck {
                packet_id: packet_id(0x4321),
                reason_code: PublishAckReason::Success,
                properties: UserProperties::default(),
                reason_string: None,
            }),
        );
        assert_decode_packet(
            b"\x62\x02\x43\x21",
            Packet::PublishRelease(PublishAck2 {
                packet_id: packet_id(0x4321),
                reason_code: PublishAck2Reason::Success,
                properties: UserProperties::default(),
                reason_string: None,
            }),
        );
        assert_decode_packet(
            b"\x70\x02\x43\x21",
            Packet::PublishComplete(PublishAck2 {
                packet_id: packet_id(0x4321),
                reason_code: PublishAck2Reason::Success,
                properties: UserProperties::default(),
                reason_string: None,
            }),
        );
    }

    #[test]
    fn test_decode_subscribe_packets() {
        let p = Packet::Subscribe(Subscribe {
            packet_id: packet_id(0x1234),
            topic_filters: vec![
                (
                    ByteString::from_static("test"),
                    SubscriptionOptions {
                        qos: QoS::AtLeastOnce,
                        no_local: false,
                        retain_as_published: false,
                        retain_handling: RetainHandling::AtSubscribe,
                    },
                ),
                (
                    ByteString::from_static("filter"),
                    SubscriptionOptions {
                        qos: QoS::ExactlyOnce,
                        no_local: false,
                        retain_as_published: false,
                        retain_handling: RetainHandling::AtSubscribe,
                    },
                ),
            ],
            id: None,
            user_properties: Vec::new(),
        });

        assert_decode_packet(b"\x82\x13\x12\x34\x00\x00\x04test\x01\x00\x06filter\x02", p);

        let p = Packet::Subscribe(Subscribe {
            packet_id: packet_id(0x1234),
            topic_filters: vec![
                (
                    ByteString::from_static("test"),
                    SubscriptionOptions {
                        qos: QoS::AtLeastOnce,
                        no_local: false,
                        retain_as_published: false,
                        retain_handling: RetainHandling::AtSubscribe,
                    },
                ),
                (
                    ByteString::from_static("filter"),
                    SubscriptionOptions {
                        qos: QoS::ExactlyOnce,
                        no_local: false,
                        retain_as_published: false,
                        retain_handling: RetainHandling::AtSubscribe,
                    },
                ),
            ],
            id: Some(NonZeroU32::new(1).unwrap()),
            user_properties: Vec::new(),
        });

        assert_decode_packet(b"\x82\x15\x12\x34\x02\x0b\x01\x00\x04test\x01\x00\x06filter\x02", p);

        let p = Packet::SubscribeAck(SubscribeAck {
            packet_id: packet_id(0x1234),
            status: vec![
                SubscribeAckReason::GrantedQos1,
                SubscribeAckReason::UnspecifiedError,
                SubscribeAckReason::GrantedQos2,
            ],
            properties: UserProperties::default(),
            reason_string: None,
        });

        assert_decode_packet(b"\x90\x05\x12\x34\x00\x01\x80\x02", p);

        let p = Packet::Unsubscribe(Unsubscribe {
            packet_id: packet_id(0x1234),
            topic_filters: vec![ByteString::from_static("test"), ByteString::from_static("filter")],
            user_properties: UserProperties::default(),
        });

        assert_eq!(
            Packet::Unsubscribe(
                Unsubscribe::decode(&mut Bytes::from_static(b"\x12\x34\x00\x00\x04test\x00\x06filter"))
                    .unwrap()
            ),
            p.clone()
        );
        assert_decode_packet(b"\xa2\x11\x12\x34\x00\x00\x04test\x00\x06filter", p);

        assert_decode_packet(
            b"\xb0\x03\x43\x21\x00",
            Packet::UnsubscribeAck(UnsubscribeAck {
                packet_id: packet_id(0x4321),
                properties: UserProperties::default(),
                reason_string: None,
                status: vec![],
            }),
        );
    }

    #[test]
    fn test_decode_ping_packets() {
        assert_decode_packet(b"\xc0\x00", Packet::PingRequest);
        assert_decode_packet(b"\xd0\x00", Packet::PingResponse);
    }
}
