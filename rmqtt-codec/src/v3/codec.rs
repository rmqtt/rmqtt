use std::cell::Cell;

use bytes::{Buf, BytesMut};
use tokio_util::codec::{Decoder, Encoder};

use super::{decode, encode, Packet};
use crate::error::{DecodeError, EncodeError};
use crate::types::{FixedHeader, QoS};
use crate::utils::decode_variable_length;
use crate::v3::packet::Publish;

#[derive(Debug, Clone)]
/// Mqtt v3.1.1 protocol codec
pub struct Codec {
    state: Cell<DecodeState>,
    max_size: Cell<u32>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
enum DecodeState {
    FrameHeader,
    Frame(FixedHeader),
}

impl Codec {
    /// Create `Codec` instance
    pub fn new(max_packet_size: u32) -> Self {
        Codec { state: Cell::new(DecodeState::FrameHeader), max_size: Cell::new(max_packet_size) }
    }

    /// Set max inbound frame size.
    ///
    /// If max size is set to `0`, size is unlimited.
    /// By default max size is set to `0`
    pub fn set_max_size(&mut self, size: u32) {
        self.max_size.set(size);
    }
}

impl Default for Codec {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Decoder for Codec {
    type Item = (Packet, u32);
    type Error = DecodeError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, DecodeError> {
        loop {
            match self.state.get() {
                DecodeState::FrameHeader => {
                    if src.len() < 2 {
                        return Ok(None);
                    }
                    let src_slice = src.as_ref();
                    let first_byte = src_slice[0];
                    match decode_variable_length(&src_slice[1..])? {
                        Some((remaining_length, consumed)) => {
                            // check max message size
                            let max_size = self.max_size.get();
                            if max_size != 0 && max_size < remaining_length {
                                return Err(DecodeError::MaxSizeExceeded);
                            }
                            src.advance(consumed + 1);
                            self.state.set(DecodeState::Frame(FixedHeader { first_byte, remaining_length }));
                            // todo: validate remaining_length against max frame size config
                            let remaining_length = remaining_length as usize;
                            if src.len() < remaining_length {
                                // todo: subtract?
                                src.reserve(remaining_length); // extend receiving buffer to fit the whole frame -- todo: too eager?
                                return Ok(None);
                            }
                        }
                        None => {
                            return Ok(None);
                        }
                    }
                }
                DecodeState::Frame(fixed) => {
                    if src.len() < fixed.remaining_length as usize {
                        return Ok(None);
                    }
                    let packet_buf = src.split_to(fixed.remaining_length as usize);
                    let packet = decode::decode_packet(packet_buf.freeze(), fixed.first_byte)?;
                    self.state.set(DecodeState::FrameHeader);
                    src.reserve(2);
                    return Ok(Some((packet, fixed.remaining_length)));
                }
            }
        }
    }
}

impl Encoder<Packet> for Codec {
    // type Item = Packet;
    type Error = EncodeError;

    fn encode(&mut self, item: Packet, dst: &mut BytesMut) -> Result<(), EncodeError> {
        if let Packet::Publish(Publish { qos, packet_id, .. }) = item {
            if (qos == QoS::AtLeastOnce || qos == QoS::ExactlyOnce) && packet_id.is_none() {
                return Err(EncodeError::PacketIdRequired);
            }
        }
        let content_size = encode::get_encoded_size(&item);
        dst.reserve(content_size + 5);
        encode::encode(&item, dst, content_size as u32)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use bytestring::ByteString;

    #[test]
    fn test_max_size() {
        let mut codec = Codec::default();
        codec.set_max_size(5);

        let mut buf = BytesMut::new();
        buf.extend_from_slice(b"\0\x09");
        assert_eq!(codec.decode(&mut buf).map_err(|e| matches!(e, DecodeError::MaxSizeExceeded)), Err(true));
    }

    #[test]
    fn test_packet() {
        let mut codec = Codec::default();
        let mut buf = BytesMut::new();

        let pkt = Publish {
            dup: false,
            retain: false,
            qos: QoS::AtMostOnce,
            topic: ByteString::from_static("/test"),
            packet_id: None,
            payload: Bytes::from(Vec::from("a".repeat(260 * 1024))),
        };
        codec.encode(Packet::Publish(pkt.clone()), &mut buf).unwrap();

        let pkt2 =
            if let (Packet::Publish(v), _) = codec.decode(&mut buf).unwrap().unwrap() { v } else { panic!() };
        assert_eq!(pkt, pkt2);
    }
}
