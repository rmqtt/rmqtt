[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-codec

[![crates.io page](https://img.shields.io/crates/v/rmqtt-codec.svg)](https://crates.io/crates/rmqtt-codec)
[![docs.rs page](https://docs.rs/rmqtt-codec/badge.svg)](https://docs.rs/rmqtt-codec/latest/rmqtt_codec)

MQTT 协议编解码库 — v3.1.1 / v5.0，支持基于握手的版本协商。

## 公开类型

### `MqttCodec`（枚举）

```rust
pub enum MqttCodec {
    V3(v3::Codec),
    V5(v5::Codec),
    Version(version::VersionCodec),
}
```

实现了 `tokio_util::codec::Encoder<MqttPacket>`（错误：`EncodeError`）和 `tokio_util::codec::Decoder`（返回 `(MqttPacket, u32)` — 数据包 + 消费字节数；错误：`DecodeError`）。

### `MqttPacket`（枚举）

```rust
pub enum MqttPacket {
    V3(v3::Packet),
    V5(v5::Packet),
    Version(version::ProtocolVersion),
}
```

## 公开模块

| 模块 | 内容 |
|--------|----------|
| `v3` | `Codec`、`Packet`（Connect、ConnAck、Publish、PubAck、PubRec、PubRel、PubComp、Subscribe、SubAck、Unsubscribe、UnsubAck、PingRequest、PingResponse、Disconnect）、`ConnAckReason` |
| `v5` | `Codec`、`Packet`（同上 + Auth）、`PublishProperties`、`Auth`、`DisconnectReasonCode`、`SubscribeAckReason`、`UserProperties`、`ToReasonCode` trait |
| `version` | `VersionCodec`、`ProtocolVersion`（MQTT3、MQTT5）— 握手检测 |
| `error` | `HandshakeError`、`ProtocolError`、`DecodeError`、`EncodeError`、`SendPacketError` |
| `types` | `QoS`（AtMostOnce=0、AtLeastOnce=1、ExactlyOnce=2）、`Protocol`、`SubscribeReason`、`PacketId`、`Publish` |
| `cert` | 证书相关辅助 |

### 错误类型

```rust
pub enum HandshakeError { Protocol(ProtocolError), Timeout }
pub enum ProtocolError { Decode(DecodeError), Encode(EncodeError), Timeout, ... }
pub enum DecodeError { ... }
pub enum EncodeError { MalformedPacket, UnsupportedVersion, ... }
pub enum SendPacketError { Disconnected(ByteString), Full, ... }
```

## 使用方式

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

## 依赖

`tokio-util` (codec)、`bytes`、`bytestring`、`serde`、`thiserror`、`bitflags`、`chrono`、`nonzero_ext`、`rmqtt-utils`

无 features。

## 许可证

MIT OR Apache-2.0
