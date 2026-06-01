[**English**](README.md) | [简体中文](README-CN.md)

# rmqttd

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
![Rust](https://img.shields.io/badge/rust-1.89%2B-blue)

Official binary entry point for the RMQTT MQTT broker.

## What it does

- **Startup flow**: Parse CLI args → initialize `rmqtt_conf::Settings` singleton → install rustls crypto backend → init tracing logger → create `rmqtt::context::ServerContext` → start gRPC server → register plugins from `Cargo.toml` metadata → bind configured listeners → start MQTT server
- **Build-time plugin registration**: `build.rs` reads `[package.metadata.plugins]` from `Cargo.toml`, auto-generates `plugin.rs` with a `registers()` function. Each plugin is registered based on `default_startup` and `immutable` flags
- **Listener types**: TCP, TLS, WebSocket (WS), TLS-WebSocket (WSS), QUIC
- **Signal handling**: `Ctrl+C` on Windows, `SIGTERM` + `SIGINT` on Unix; 100ms graceful delay before exit
- **Logging**: Configured via `rmqtt_conf::logging::Log` — supports `off/console/file/both` modes, UTC+8 timestamps, non-blocking file writer
- **Linux allocator**: Uses `tikv-jemallocator` as the default memory allocator on Linux

## Build

```bash
cargo build -p rmqttd --release
# Artifact: target/release/rmqttd (or rmqttd.exe)
```

## Run

```bash
./target/release/rmqttd
./target/release/rmqttd -f /path/to/rmqtt.toml
./target/release/rmqttd --config /path/to/rmqtt.toml
./target/release/rmqttd --id 1
```

## CLI arguments

Defined by `rmqtt_conf::Options` (via `clap::Parser`):

| Argument | Type | Description |
|----------|------|-------------|
| `-f`, `--config` | `Option<String>` | Config file path |
| `-V`, `--version` | `bool` | Print version info |
| `--id` | `Option<u64>` | Node ID |
| `--plugins-default-startups` | `Option<Vec<String>>` | Override default plugin startups (repeatable) |
| `--node-grpc-addrs` | `Option<Vec<NodeAddr>>` | Node gRPC addresses, format `"1@127.0.0.1:5363"` (repeatable) |
| `--raft-peer-addrs` | `Option<Vec<NodeAddr>>` | Raft peer addresses, format `"1@127.0.0.1:6003"` (repeatable) |
| `--raft-leader-id` | `Option<u64>` | Raft leader ID; default 0 (first node becomes leader) |

## Configuration

Loaded by `rmqtt_conf::Settings` from the following paths (in priority order):

1. `/etc/rmqtt/rmqtt.{toml,json,...}` (optional)
2. `/etc/rmqtt.{toml,json,...}` (optional)
3. `./rmqtt.{toml,json,...}` (optional)
4. `-f` / `--config` specified file (optional)
5. `RMQTT_*` environment variables

## Docker

Three Dockerfiles for different architectures:
- `Dockerfile` — default
- `Dockerfile.amd64` — x86_64
- `Dockerfile.aarch64` — ARM64

```bash
docker build -t rmqttd .
```

## Related crates

- [rmqtt] — Core MQTT Broker library
- [rmqtt-conf] — Configuration management
- [rmqtt-plugins] — Plugin collection

[rmqtt]: https://crates.io/crates/rmqtt
[rmqtt-conf]: https://crates.io/crates/rmqtt-conf
[rmqtt-plugins]: https://crates.io/crates/rmqtt-plugins

## License

MIT OR Apache-2.0
