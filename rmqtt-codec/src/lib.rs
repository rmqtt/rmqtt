#![deny(unsafe_code)]

#[macro_use]
mod utils;
pub mod error;
pub mod types;
pub mod v3;
pub mod v5;
pub mod version;

#[derive(Debug)]
pub enum MqttCodec {
    V3(v3::Codec),
    V5(v5::Codec),
    Version(version::VersionCodec),
}

#[derive(Debug)]
pub enum MqttPacket {
    V3(v3::Packet),
    V5(v5::Packet),
    Version(version::ProtocolVersion),
}

impl tokio_util::codec::Encoder<MqttPacket> for MqttCodec {
    type Error = error::EncodeError;

    #[inline]
    fn encode(&mut self, item: MqttPacket, dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        match self {
            MqttCodec::V3(codec) => match item {
                MqttPacket::V3(p) => {
                    codec.encode(p, dst)?;
                }
                _ => return Err(error::EncodeError::MalformedPacket),
            },
            MqttCodec::V5(codec) => match item {
                MqttPacket::V5(p) => {
                    codec.encode(p, dst)?;
                }
                _ => return Err(error::EncodeError::MalformedPacket),
            },
            MqttCodec::Version(_) => return Err(error::EncodeError::UnsupportedVersion),
        };
        Ok(())
    }
}

impl tokio_util::codec::Decoder for MqttCodec {
    type Item = (MqttPacket, u32);
    type Error = error::DecodeError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let p = match self {
            MqttCodec::V3(codec) => codec.decode(src)?.map(|(p, remaining)| (MqttPacket::V3(p), remaining)),
            MqttCodec::V5(codec) => codec.decode(src)?.map(|(p, remaining)| (MqttPacket::V5(p), remaining)),
            MqttCodec::Version(codec) => codec.decode(src)?.map(|v| (MqttPacket::Version(v), 0)),
        };
        Ok(p)
    }
}
