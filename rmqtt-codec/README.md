[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-codec

[![crates.io page](https://img.shields.io/crates/v/rmqtt-codec.svg)](https://crates.io/crates/rmqtt-codec)
[![docs.rs page](https://docs.rs/rmqtt-codec/badge.svg)](https://docs.rs/rmqtt-codec/latest/rmqtt_codec)

MQTT protocol codec — v3.1.1 / v5.0 encoding and decoding with handshake-based version negotiation.

## Public types

### `MqttCodec` (enum)

```rust
pub enum MqttCodec {
    V3(v3::Codec),
    V5(v5::Codec),
    Version(version::VersionCodec),
}
```

Implements `tokio_util::codec::Encoder<MqttPacket>` (error: `EncodeError`) and `tokio_util::codec::Decoder` (yields `(MqttPacket, u32)` — packet + consumed bytes; error: `DecodeError`).

### `MqttPacket` (enum)

```rust
pub enum MqttPacket {
    V3(v3::Packet),
    V5(v5::Packet),
    Version(version::ProtocolVersion),
}
```

## Public modules

| Module | Contents |
|--------|----------|
| `v3` | `Codec`, `Packet` (Connect, ConnAck, Publish, PubAck, PubRec, PubRel, PubComp, Subscribe, SubAck, Unsubscribe, UnsubAck, PingRequest, PingResponse, Disconnect), `ConnAckReason` |
| `v5` | `Codec`, `Packet` (same + Auth), `PublishProperties`, `Auth`, `DisconnectReasonCode`, `SubscribeAckReason`, `UserProperties`, `ToReasonCode` trait |
| `version` | `VersionCodec`, `ProtocolVersion` (MQTT3, MQTT5) — handshake detection |
| `error` | `HandshakeError`, `ProtocolError`, `DecodeError`, `EncodeError`, `SendPacketError` |
| `types` | `QoS` (AtMostOnce=0, AtLeastOnce=1, ExactlyOnce=2), `Protocol` (wrapper around u8), `SubscribeReason`, `PacketId`, `Publish` |
| `cert` | Certificate-related helpers |

### Error types

```rust
pub enum HandshakeError { Protocol(ProtocolError), Timeout }
pub enum ProtocolError { Decode(DecodeError), Encode(EncodeError), Timeout, ... }
pub enum DecodeError { ... }
pub enum EncodeError { MalformedPacket, UnsupportedVersion, ... }
pub enum SendPacketError { Disconnected(ByteString), Full, ... }
```

## Usage

```rust
use bytes::BytesMut;
use rmqtt_codec::{MqttCodec, MqttPacket, v3};
use tokio_util::codec::{Encoder, Decoder};

let mut codec = MqttCodec::V3(v3::Codec::new(1024 * 1024)); // max_packet_size
let mut buf = BytesMut::new();
codec.encode(MqttPacket::V3(v3::Packet::PingRequest), &mut buf).unwrap();
```

## Cargo.toml

```toml
[dependencies]
rmqtt-codec = "0.2"
```

## Dependencies

`tokio-util` (codec), `bytes`, `bytestring`, `serde`, `thiserror`, `bitflags`, `chrono`, `nonzero_ext`, `rmqtt-utils`.

No features.

## License

MIT OR Apache-2.0
