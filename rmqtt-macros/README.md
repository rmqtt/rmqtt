# rmqtt-macros

[![crates.io page](https://img.shields.io/crates/v/rmqtt-macros.svg)](https://crates.io/crates/rmqtt-macros/0.1.0)
[![docs.rs page](https://docs.rs/rmqtt-macros/badge.svg)](https://docs.rs/rmqtt-macros/0.1.0/rmqtt_macros)


✨ **rmqtt-macros** provides a collection of procedural macros to enhance the RMQTT ecosystem, including support for 
metrics collection and plugin systems. All macros are gated by feature flags for modular usage.

## 🔧 Features

- **`metrics`** – Enables the `#[derive(Metrics)]` macro for auto-generating metric collectors
- **`plugin`** – Enables the `#[derive(Plugin)]` macro for building dynamic plugin systems

## 📦 Example

```rust,ignore
#[cfg(feature = "metrics")]
#[derive(Metrics)]
struct NetworkMetrics {
    bytes_sent: Counter,
    bytes_received: Counter,
}

#[cfg(feature = "plugin")]
#[derive(Plugin)]
struct MyPlugin {
    config: PluginConfig,
}
```

## 📚 Crate Usage

To use a specific macro, enable the corresponding feature in your `Cargo.toml`:

```toml
[dependencies]
rmqtt-macros = { version = "0.1", features = ["metrics", "plugin"] }
```

## 🚀 Designed for RMQTT

This crate is intended for internal use within the [RMQTT](https://github.com/emqx/rmqtt) project but can be reused in other systems requiring similar functionality.

