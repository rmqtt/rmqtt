# RMQTT-Server

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt/0.15.0-beta.4)
[![docs.rs page](https://docs.rs/rmqtt/badge.svg)](https://docs.rs/rmqtt/0.15.0-beta.4/rmqtt)


A high-performance, asynchronous MQTT server library built with [Tokio](https://tokio.rs). `rmqtt-server` is designed for flexibility, allowing you to configure multiple listeners with different protocols and security settings. Ideal for building custom or embedded MQTT services in Rust.

## ✨ Features

- **Async & High Performance**: Built on Tokio for efficient handling of thousands of concurrent connections.
- **Multi-Protocol Support**:
    - Native MQTT over TCP (with optional TLS)
    - MQTT over WebSocket (with optional TLS)
- **Multiple Listener Bindings**: Support for binding multiple listeners simultaneously — perfect for internal/external access separation.
- **Modular Builder Pattern**: Flexible listener configuration using a fluent builder interface.
- **TLS Encryption**: Easily enable TLS for any listener with custom certificate/key paths.
- **Lightweight and Easy to Use**: Minimal boilerplate required to spin up a complete MQTT server.
- **Logging Integration**: Compatible with common Rust logging libraries.

## 🚀 Quick Example

A basic MQTT server with RMQTT.

```toml
[dependencies]
rmqtt = "0.15.0-beta.4"
tokio = { version = "1", features = ["full"] }
simple_logger = "5"
log = "0.4"
```
Then, on your main.rs:

```rust
use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}
```

## 🚀 Multi-Protocol Multi-Listener Example

A flexible MQTT server with multi-protocol and multi-listener support using RMQTT.

Make sure you included the rmqtt crate with the required features in your Cargo.toml:

```toml
[dependencies]
rmqtt = { version = "0.15.0-beta.4", features = ["ws", "tls"] }
tokio = { version = "1", features = ["full"] }
simple_logger = "5"
log = "0.4"
```

```rust
use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
  SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

  let scx = ServerContext::new().build().await;

  MqttServer::new(scx)
          .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
          .listener(Builder::new().name("internal/tcp").laddr(([0, 0, 0, 0], 11883).into()).bind()?.tcp()?)
          .listener(
            Builder::new()
                    .name("external/tls")
                    .laddr(([0, 0, 0, 0], 8883).into())
                    .tls_key(Some("./rmqtt-bin/rmqtt.key"))
                    .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
                    .bind()?
                    .tls()?,
          )
          .listener(Builder::new().name("external/ws").laddr(([0, 0, 0, 0], 8080).into()).bind()?.ws()?)
          .listener(
            Builder::new()
                    .name("external/wss")
                    .laddr(([0, 0, 0, 0], 8443).into())
                    .tls_key(Some("./rmqtt-bin/rmqtt.key"))
                    .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
                    .bind()?
                    .wss()?,
          )
          .build()
          .run()
          .await?;
  Ok(())
}
```

## 🚀 Plugin Configuration Example

An MQTT server with plugin support using RMQTT.

Make sure you included the rmqtt crate with the required features in your Cargo.toml:

```toml
[dependencies]
rmqtt = { version = "0.15.0-beta.4", features = ["plugin"] }
rmqtt-acl = "0.1"
rmqtt-retainer = "0.1"
rmqtt-http-api = "0.1"
rmqtt-web-hook = "0.1"
tokio = { version = "1", features = ["full"] }
simple_logger = "5"
log = "0.4"
```

```rust
use rmqtt::{context::ServerContext, net::Builder, server::MqttServer, Result};
use simple_logger::SimpleLogger;

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let scx = ServerContext::new().plugins_dir("rmqtt-plugins/").build().await;

    rmqtt_acl::register(&scx, "rmqtt-acl", true, false).await?;
    rmqtt_retainer::register(&scx, "rmqtt-retainer", true, false).await?;
    rmqtt_http_api::register(&scx, "rmqtt-http-api", true, false).await?;
    rmqtt_web_hook::register(&scx, "rmqtt-web-hook", true, false).await?;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}

```

More examples can be found [here][examples]. For a larger "real world" example, see the
[rmqtt] repository.

[examples]: https://github.com/rmqtt/rmqtt/tree/master/rmqtt/examples
[rmqtt]: https://github.com/rmqtt/rmqtt

## 📦 Use Cases

- Custom MQTT servers for IoT gateways and edge devices
- Services requiring multiple entry points with different transport layers
- Projects that need secure, embedded, and efficient MQTT broker capabilities written in Rust

