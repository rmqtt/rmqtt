# rmqtt-codec

[![crates.io page](https://img.shields.io/crates/v/rmqtt-codec.svg)](https://crates.io/crates/rmqtt-codec/0.1.1)
[![docs.rs page](https://docs.rs/rmqtt-codec/badge.svg)](https://docs.rs/rmqtt-codec/0.1.1/rmqtt_codec)


🚀 **rmqtt-codec** is a high-performance MQTT protocol codec library designed for async environments. It provides full 
support for multiple MQTT versions with automatic negotiation and zero-copy efficiency, seamlessly integrating with the Tokio ecosystem.

## ✨ Core Features

- **Multi-Version Support**: Full implementation of MQTT v3.1, v3.1.1, and v5.0 specifications
- **Automatic Protocol Negotiation**: Smart version detection during the CONNECT handshake phase
- **Zero-Copy Encoding/Decoding**: Efficient binary processing using `bytes::BytesMut` to minimize memory overhead
- **Tokio Integration**: Built with `tokio_util::codec` for smooth async I/O operations
- **Memory Safety**: Enforces strict message size limits (default 1MB) to prevent memory-related vulnerabilities

## 🧩 Architecture Components

- `MqttCodec`: Main codec dispatcher for version-aware encoding and decoding
- `MqttPacket`: Unified representation of all MQTT packet types across versions
- `version::ProtocolVersion`: Handshake-based protocol version detection
- `EncodeError` / `DecodeError`: Dedicated error types for robust error handling during encoding and decoding

## 📚 Crate Usage

Please add a dependency in 'Cargo. toml':

```toml
[dependencies]
rmqtt-codec = "0.1"
```

## 🔧 Use Cases

- Embedded MQTT brokers or clients
- High-performance message middleware
- Protocol gateways or custom network services

