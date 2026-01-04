# RMQTT-Server

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
[![docs.rs page](https://docs.rs/rmqtt/badge.svg)](https://docs.rs/rmqtt/latest/rmqtt/)


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
rmqtt = "0.18"
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
rmqtt = { version = "0.18", features = ["ws", "tls"] }
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
rmqtt = { version = "0.18", features = ["plugin"] }
rmqtt-plugins = { version = "0.18", features = ["full"] }
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

    let scx = ServerContext::new().plugins_config_dir("rmqtt-plugins/").build().await;

    rmqtt_plugins::acl::register(&scx, true, false).await?;
    rmqtt_plugins::http_api::register(&scx, true, false).await?;
    rmqtt_plugins::retainer::register(&scx, true, false).await?;

    MqttServer::new(scx)
        .listener(
            Builder::new()
                .name("external/tcp")
                .laddr(([0, 0, 0, 0], 1883).into())
                .allow_anonymous(false)
                .bind()?
                .tcp()?,
        )
        .build()
        .run()
        .await?;
    Ok(())
}

```

or

```toml
[dependencies]
rmqtt = { version = "0.18", features = ["plugin"] }
rmqtt-plugins = { version = "0.18", features = ["full"] }
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

  let rules = r###"rules = [
                ["allow", { user = "dashboard" }, "subscribe", ["$SYS/#"]],
                ["allow", { ipaddr = "127.0.0.1" }, "pubsub", ["$SYS/#", "#"]],
                ["deny", "all", "subscribe", ["$SYS/#", { eq = "#" }]],
                ["allow", "all"]
        ]"###;

  let scx = ServerContext::new()
          .plugins_config_dir("rmqtt-plugins/")
          .plugins_config_map_add("rmqtt-acl", rules)
          .build()
          .await;
  rmqtt_plugins::acl::register(&scx, true, false).await?;
  rmqtt_plugins::http_api::register(&scx, true, false).await?;
  rmqtt_plugins::retainer::register(&scx, true, false).await?;

  MqttServer::new(scx)
          .listener(
            Builder::new()
                    .name("external/tcp")
                    .laddr(([0, 0, 0, 0], 1883).into())
                    .allow_anonymous(false)
                    .bind()?
                    .tcp()?,
          )
          .build()
          .run()
          .await?;
  Ok(())
}

```

or

```toml
[dependencies]
rmqtt = { version = "0.18", features = ["plugin"] }
rmqtt-acl = "0.18"
rmqtt-retainer = "0.18"
rmqtt-http-api = "0.18"
rmqtt-web-hook = "0.18"
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
    // put plugin config files in ./rmqtt-plugins/ , like rmqtt_web_hook.toml
    let scx = ServerContext::new().plugins_config_dir("rmqtt-plugins/").build().await;
    // or load by string
    // let scx = ServerContext::new().plugins_config_map(load_config()).build().await;
    rmqtt_acl::register(&scx, true, false).await?;
    rmqtt_retainer::register(&scx, true, false).await?;
    rmqtt_http_api::register(&scx, true, false).await?;
    rmqtt_web_hook::register(&scx, true, false).await?;

    MqttServer::new(scx)
        .listener(Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?)
        .build()
        .run()
        .await?;
    Ok(())
}

use ahash::HashMapExt;
fn load_config() -> rmqtt::types::HashMap<String, String> {
  let mut config = HashMap::new();
  config.insert("rmqtt_web_hook".to_owned(), r#"worker_threads = 3
    queue_capacity = 300_000
    concurrency_limit = 128"#.to_owned());
  return config;
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

