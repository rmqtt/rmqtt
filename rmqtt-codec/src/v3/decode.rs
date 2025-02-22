use std::num::NonZeroU16;

use bytes::{Buf, Bytes};
use bytestring::ByteString;

use crate::error::DecodeError;
use crate::types::{packet_type, Protocol, QoS, MQISDP, MQTT, MQTT_LEVEL_31, MQTT_LEVEL_311, WILL_QOS_SHIFT};
use crate::utils::{timestamp_millis, Decode};

use super::packet::{Connect, ConnectAck, LastWill, Packet, Publish, SubscribeReturnCode};
use super::{ConnectAckFlags, ConnectFlags};

pub(crate) fn decode_packet(mut src: Bytes, first_byte: u8) -> Result<Packet, DecodeError> {
    match first_byte {
        packet_type::CONNECT => decode_connect_packet(&mut src),
        packet_type::CONNACK => decode_connect_ack_packet(&mut src),
        packet_type::PUBLISH_START..=packet_type::PUBLISH_END => {
            decode_publish_packet(&mut src, first_byte & 0b0000_1111)
        }
        packet_type::PUBACK => decode_ack(src, |packet_id| Packet::PublishAck { packet_id }),
        packet_type::PUBREC => decode_ack(src, |packet_id| Packet::PublishReceived { packet_id }),
        packet_type::PUBREL => decode_ack(src, |packet_id| Packet::PublishRelease { packet_id }),
        packet_type::PUBCOMP => decode_ack(src, |packet_id| Packet::PublishComplete { packet_id }),
        packet_type::SUBSCRIBE => decode_subscribe_packet(&mut src),
        packet_type::SUBACK => decode_subscribe_ack_packet(&mut src),
        packet_type::UNSUBSCRIBE => decode_unsubscribe_packet(&mut src),
        packet_type::UNSUBACK => decode_ack(src, |packet_id| Packet::UnsubscribeAck { packet_id }),
        packet_type::PINGREQ => Ok(Packet::PingRequest),
        packet_type::PINGRESP => Ok(Packet::PingResponse),
        packet_type::DISCONNECT => Ok(Packet::Disconnect),
        _ => Err(DecodeError::UnsupportedPacketType),
    }
}

#[inline]
fn decode_ack(mut src: Bytes, f: impl Fn(NonZeroU16) -> Packet) -> Result<Packet, DecodeError> {
    let packet_id = NonZeroU16::decode(&mut src)?;
    ensure!(!src.has_remaining(), DecodeError::InvalidLength);
    Ok(f(packet_id))
}

fn decode_connect_packet(src: &mut Bytes) -> Result<Packet, DecodeError> {
    ensure!(src.remaining() >= 10, DecodeError::InvalidLength);
    let len = src.get_u16();

    if len == 4 && &src.as_ref()[0..4] == MQTT {
        src.advance(4);
    } else if len == 6 && &src.as_ref()[0..6] == MQISDP {
        src.advance(6);
    } else {
        return Err(DecodeError::InvalidProtocol);
    }

    let level = src.get_u8();
    // ensure!(level == MQTT_LEVEL_311, DecodeError::UnsupportedProtocolLevel);
    ensure!(level == MQTT_LEVEL_311 || level == MQTT_LEVEL_31, DecodeError::UnsupportedProtocolLevel);

    let flags = ConnectFlags::from_bits(src.get_u8()).ok_or(DecodeError::ConnectReservedFlagSet)?;

    let keep_alive = u16::decode(src)?;
    let client_id = ByteString::decode(src)?;

    ensure!(!client_id.is_empty() || flags.contains(ConnectFlags::CLEAN_START), DecodeError::InvalidClientId);

    let last_will = if flags.contains(ConnectFlags::WILL) {
        let topic = ByteString::decode(src)?;
        let message = Bytes::decode(src)?;
        Some(LastWill {
            qos: QoS::try_from((flags & ConnectFlags::WILL_QOS).bits() >> WILL_QOS_SHIFT)?,
            retain: flags.contains(ConnectFlags::WILL_RETAIN),
            topic,
            message,
        })
    } else {
        None
    };
    let username = if flags.contains(ConnectFlags::USERNAME) { Some(ByteString::decode(src)?) } else { None };
    let password = if flags.contains(ConnectFlags::PASSWORD) { Some(Bytes::decode(src)?) } else { None };
    Ok(Connect {
        protocol: Protocol(level),
        clean_session: flags.contains(ConnectFlags::CLEAN_START),
        keep_alive,
        client_id,
        last_will,
        username,
        password,
    }
    .into())
}

fn decode_connect_ack_packet(src: &mut Bytes) -> Result<Packet, DecodeError> {
    ensure!(src.remaining() >= 2, DecodeError::InvalidLength);
    let flags = ConnectAckFlags::from_bits(src.get_u8()).ok_or(DecodeError::ConnAckReservedFlagSet)?;

    let return_code = src.get_u8().try_into()?;
    Ok(Packet::ConnectAck(ConnectAck {
        return_code,
        session_present: flags.contains(ConnectAckFlags::SESSION_PRESENT),
    }))
}

fn decode_publish_packet(src: &mut Bytes, packet_flags: u8) -> Result<Packet, DecodeError> {
    let topic = ByteString::decode(src)?;
    let qos = QoS::try_from((packet_flags & 0b0110) >> 1)?;
    let packet_id = if qos == QoS::AtMostOnce {
        None
    } else {
        Some(NonZeroU16::decode(src)?) // packet id = 0 encountered
    };

    Ok(Packet::Publish(Publish {
        dup: (packet_flags & 0b1000) == 0b1000,
        qos,
        retain: (packet_flags & 0b0001) == 0b0001,
        topic,
        packet_id,
        payload: src.split_off(0),
        properties: None,
        delay_interval: None,
        create_time: Some(timestamp_millis()),
    }))
}

fn decode_subscribe_packet(src: &mut Bytes) -> Result<Packet, DecodeError> {
    let packet_id = NonZeroU16::decode(src)?;
    let mut topic_filters = Vec::new();
    while src.has_remaining() {
        let topic = ByteString::decode(src)?;
        ensure!(src.remaining() >= 1, DecodeError::InvalidLength);
        let qos = (src.get_u8() & 0b0000_0011).try_into()?;
        topic_filters.push((topic, qos));
    }

    Ok(Packet::Subscribe { packet_id, topic_filters })
}

fn decode_subscribe_ack_packet(src: &mut Bytes) -> Result<Packet, DecodeError> {
    let packet_id = NonZeroU16::decode(src)?;
    let mut status = Vec::with_capacity(src.len());
    for code in src.as_ref().iter() {
        status.push(if *code == 0x80 {
            SubscribeReturnCode::Failure
        } else {
            SubscribeReturnCode::Success(QoS::try_from(*code)?)
        });
    }
    Ok(Packet::SubscribeAck { packet_id, status })
}

fn decode_unsubscribe_packet(src: &mut Bytes) -> Result<Packet, DecodeError> {
    let packet_id = NonZeroU16::decode(src)?;
    let mut topic_filters = Vec::new();
    while src.remaining() > 0 {
        topic_filters.push(ByteString::decode(src)?);
    }
    Ok(Packet::Unsubscribe { packet_id, topic_filters })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::decode_variable_length;
    use crate::v3::ConnectAckReason;

    macro_rules! assert_decode_packet (
        ($bytes:expr, $res:expr) => {{
            let first_byte = $bytes.as_ref()[0];
            let (_len, consumed) = decode_variable_length(&$bytes[1..]).unwrap().unwrap();
            let cur = Bytes::from_static(&$bytes[consumed + 1..]);
            assert_eq!(decode_packet(cur, first_byte).unwrap(), $res);
        }};
    );

    fn packet_id(v: u16) -> NonZeroU16 {
        NonZeroU16::new(v).unwrap()
    }

    #[test]
    fn test_decode_connect_packets() {
        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(
                b"\x00\x04MQTT\x04\xC0\x00\x3C\x00\x0512345\x00\x04user\x00\x04pass"
            ))
            .unwrap(),
            Packet::Connect(Box::new(Connect {
                clean_session: false,
                keep_alive: 60,
                client_id: ByteString::try_from(Bytes::from_static(b"12345")).unwrap(),
                last_will: None,
                username: Some(ByteString::try_from(Bytes::from_static(b"user")).unwrap()),
                password: Some(Bytes::from(&b"pass"[..])),
            }))
        );

        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(
                b"\x00\x04MQTT\x04\x14\x00\x3C\x00\x0512345\x00\x05topic\x00\x07message"
            ))
            .unwrap(),
            Packet::Connect(Box::new(Connect {
                clean_session: false,
                keep_alive: 60,
                client_id: ByteString::try_from(Bytes::from_static(b"12345")).unwrap(),
                last_will: Some(LastWill {
                    qos: QoS::ExactlyOnce,
                    retain: false,
                    topic: ByteString::try_from(Bytes::from_static(b"topic")).unwrap(),
                    message: Bytes::from(&b"message"[..]),
                }),
                username: None,
                password: None,
            }))
        );

        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(b"\x00\x02MQ00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::InvalidProtocol)),
            Err(true),
        );
        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(b"\x00\x10MQ00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::InvalidProtocol)),
            Err(true),
        );
        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(b"\x00\x04MQAA00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::InvalidProtocol)),
            Err(true),
        );
        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(b"\x00\x04MQTT\x0300000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::UnsupportedProtocolLevel)),
            Err(true),
        );
        assert_eq!(
            decode_connect_packet(&mut Bytes::from_static(b"\x00\x04MQTT\x04\xff00000000000000000000"))
                .map_err(|e| matches!(e, DecodeError::ConnectReservedFlagSet)),
            Err(true)
        );

        assert_eq!(
            decode_connect_ack_packet(&mut Bytes::from_static(b"\x01\x04")).unwrap(),
            Packet::ConnectAck(ConnectAck {
                session_present: true,
                return_code: ConnectAckReason::BadUserNameOrPassword
            })
        );

        assert_eq!(
            decode_connect_ack_packet(&mut Bytes::from_static(b"\x03\x04"))
                .map_err(|e| matches!(e, DecodeError::ConnAckReservedFlagSet)),
            Err(true)
        );

        assert_decode_packet!(
            b"\x20\x02\x01\x04",
            Packet::ConnectAck(ConnectAck {
                session_present: true,
                return_code: ConnectAckReason::BadUserNameOrPassword,
            })
        );

        assert_decode_packet!(b"\xe0\x00", Packet::Disconnect);
    }

    #[test]
    fn test_decode_publish_packets() {
        //assert_eq!(
        //    decode_publish_packet(b"\x00\x05topic\x12\x34"),
        //    Done(&b""[..], ("topic".to_owned(), 0x1234))
        //);

        assert_decode_packet!(
            b"\x3d\x0D\x00\x05topic\x43\x21data",
            Packet::Publish(Publish {
                dup: true,
                retain: true,
                qos: QoS::ExactlyOnce,
                topic: ByteString::try_from(Bytes::from_static(b"topic")).unwrap(),
                packet_id: Some(packet_id(0x4321)),
                payload: Bytes::from_static(b"data"),
            })
        );
        assert_decode_packet!(
            b"\x30\x0b\x00\x05topicdata",
            Packet::Publish(Publish {
                dup: false,
                retain: false,
                qos: QoS::AtMostOnce,
                topic: ByteString::try_from(Bytes::from_static(b"topic")).unwrap(),
                packet_id: None,
                payload: Bytes::from_static(b"data"),
            })
        );

        assert_decode_packet!(b"\x40\x02\x43\x21", Packet::PublishAck { packet_id: packet_id(0x4321) });
        assert_decode_packet!(b"\x50\x02\x43\x21", Packet::PublishReceived { packet_id: packet_id(0x4321) });
        assert_decode_packet!(b"\x62\x02\x43\x21", Packet::PublishRelease { packet_id: packet_id(0x4321) });
        assert_decode_packet!(b"\x70\x02\x43\x21", Packet::PublishComplete { packet_id: packet_id(0x4321) });
    }

    #[test]
    fn test_decode_subscribe_packets() {
        let p = Packet::Subscribe {
            packet_id: packet_id(0x1234),
            topic_filters: vec![
                (ByteString::try_from(Bytes::from_static(b"test")).unwrap(), QoS::AtLeastOnce),
                (ByteString::try_from(Bytes::from_static(b"filter")).unwrap(), QoS::ExactlyOnce),
            ],
        };

        assert_eq!(
            decode_subscribe_packet(&mut Bytes::from_static(b"\x12\x34\x00\x04test\x01\x00\x06filter\x02"))
                .unwrap(),
            p
        );
        assert_decode_packet!(b"\x82\x12\x12\x34\x00\x04test\x01\x00\x06filter\x02", p);

        let p = Packet::SubscribeAck {
            packet_id: packet_id(0x1234),
            status: vec![
                SubscribeReturnCode::Success(QoS::AtLeastOnce),
                SubscribeReturnCode::Failure,
                SubscribeReturnCode::Success(QoS::ExactlyOnce),
            ],
        };

        assert_eq!(decode_subscribe_ack_packet(&mut Bytes::from_static(b"\x12\x34\x01\x80\x02")).unwrap(), p);
        assert_decode_packet!(b"\x90\x05\x12\x34\x01\x80\x02", p);

        let p = Packet::Unsubscribe {
            packet_id: packet_id(0x1234),
            topic_filters: vec![
                ByteString::try_from(Bytes::from_static(b"test")).unwrap(),
                ByteString::try_from(Bytes::from_static(b"filter")).unwrap(),
            ],
        };

        assert_eq!(
            decode_unsubscribe_packet(&mut Bytes::from_static(b"\x12\x34\x00\x04test\x00\x06filter"))
                .unwrap(),
            p
        );
        assert_decode_packet!(b"\xa2\x10\x12\x34\x00\x04test\x00\x06filter", p);

        assert_decode_packet!(b"\xb0\x02\x43\x21", Packet::UnsubscribeAck { packet_id: packet_id(0x4321) });
    }

    #[test]
    fn test_decode_ping_packets() {
        assert_decode_packet!(b"\xc0\x00", Packet::PingRequest);
        assert_decode_packet!(b"\xd0\x00", Packet::PingResponse);
    }
}
