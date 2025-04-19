# rmqtt-net

[![crates.io page](https://img.shields.io/crates/v/rmqtt-net.svg)](https://crates.io/crates/rmqtt-net/0.1.0)
[![docs.rs page](https://docs.rs/rmqtt-net/badge.svg)](https://docs.rs/rmqtt-net/0.1.0/rmqtt_net)


🔌 **rmqtt-net** provides a foundational implementation of an MQTT server network layer, supporting MQTT v3.1.1 and v5.0 protocols over TCP, TLS, and WebSocket transports. It is designed for flexibility, performance, and easy integration into custom broker logic.

## ✨ Features

- ✅ MQTT v3.1.1 & v5.0 protocol support
- 🔐 Optional TLS via [`rustls`] backend (AWS-LC or ring, depending on platform)
- 🌐 WebSocket transport support (optional via feature flag)
- ⚡ Built with async I/O using Tokio
- 🛠️ Simple `Builder`-based API for configuration and extensibility

## 🚀 Basic Usage

```rust,no_run
use rmqtt_net::{Builder, ListenerType};
use std::net::SocketAddr;


#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let tcp_listener =
        Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?;
        
   let tcp = async {
        loop {
            match tcp_listener.accept().await {
                Ok(a) => {
                    tokio::spawn(async move {
                        log::info!("tcp {:?}", a.remote_addr);
                        let d = match a.tcp() {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Failed to mqtt(tcp) accept, {:?}", e);
                                return;
                            }
                        };
                        match d.mqtt().await {
                            Ok(MqttStream::V3(s)) => {
                                if let Err(e) = process_v3(s).await {
                                    log::warn!("Failed to process mqtt v3, {:?}", e);
                                }
                            }
                            Ok(MqttStream::V5(s)) => {
                                if let Err(e) = process_v5(s).await {
                                    log::warn!("Failed to process mqtt v5, {:?}", e);
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to probe MQTT version, {:?}", e);
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to accept TCP socket connection, {:?}", e);
                    sleep(Duration::from_millis(300)).await;
                }
            }
        }
    };
    
    tcp.await;
    
    Ok(())
}

async fn process_v3<Io>(mut s: MqttStreamV3<Io>) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{

    ...
    
    Ok(())
}

async fn process_v5<Io>(mut s: MqttStreamV5<Io>) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{

    ...
    
    Ok(())
}
```

## 🔧 Crate Usage

Add `rmqtt-net` to your `Cargo.toml`, with optional TLS/WebSocket support:

```toml
[dependencies]
rmqtt-net = { version = "0.1", features = ["tls", "ws"] }
```

- `tls`: Enables TLS support using `rustls`
- `ws`: Enables WebSocket transport

## 📦 Exposed Components

- `Builder` / `Listener` – Configure and bind MQTT listeners
- `MqttStream` – Abstract stream wrapper supporting v3/v5 logic
- `MqttError` – Common error type for network operations
- Platform-specific TLS provider via `tls_provider` alias
- Feature-gated modules for TLS and WebSocket

## ✅ Platform Notes

- On **non-Windows**: uses `aws-lc-rs` as TLS backend
- On **Windows**: uses `ring` as TLS backend

