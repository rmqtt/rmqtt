[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-macros

[![crates.io page](https://img.shields.io/crates/v/rmqtt-macros.svg)](https://crates.io/crates/rmqtt-macros)
[![docs.rs page](https://docs.rs/rmqtt-macros/badge.svg)](https://docs.rs/rmqtt-macros/latest/rmqtt_macros)

Procedural macros for the RMQTT ecosystem. Feature-gated.

## Derive macros

### `#[derive(Metrics)]` — feature `metrics`

Generates `AtomicUsize`-based counters for each field. Generated methods:

```rust,ignore
impl MyStruct {
    pub fn new() -> Self;                              // zero-init all fields
    pub fn {field}_inc(&self);                         // fetch_add(1, SeqCst)
    pub fn {field}(&self) -> usize;                    // load(SeqCst)
    pub fn to_json(&self) -> serde_json::Value;        // {"field.name": value, ...}
    pub fn add(&mut self, other: &Self);               // atomic add merge
    pub fn build_prometheus_metrics(&self, label: &str, gauge_vec: &IntGaugeVec);
}
impl Clone for MyStruct { ... }                        // clone via AtomicUsize::new(load(...))
```

### `#[derive(Plugin)]` — feature `plugin`

Generates `impl rmqtt::plugin::PackageInfo` for the struct, reading metadata at compile time:

```rust,ignore
impl rmqtt::plugin::PackageInfo for MyStruct {
    fn name(&self) -> &str;              // env!("CARGO_PKG_NAME")
    fn version(&self) -> &str;           // env!("CARGO_PKG_VERSION")
    fn descr(&self) -> Option<&str>;     // env!("CARGO_PKG_DESCRIPTION")
    fn authors(&self) -> Option<Vec<&str>>;  // env!("CARGO_PKG_AUTHORS")
    fn homepage(&self) -> Option<&str>;  // env!("CARGO_PKG_HOMEPAGE")
    fn license(&self) -> Option<&str>;   // env!("CARGO_PKG_LICENSE")
    fn repository(&self) -> Option<&str>;// env!("CARGO_PKG_REPOSITORY")
}
```

## Cargo.toml

```toml
[dependencies]
rmqtt-macros = { version = "0.1", features = ["metrics", "plugin"] }
```

## License

MIT OR Apache-2.0
