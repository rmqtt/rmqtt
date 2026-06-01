[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-bridge-origin

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-origin.svg)](https://crates.io/crates/rmqtt-bridge-origin)

Bridge origin identification plugin. Identifies whether an MQTT client originates from a bridge-ingress or bridge-egress connection.

## Overview

Checks the client_id against configurable marker substrings to classify clients as ingress bridge, egress bridge, or normal clients. The identified bridge origin is stored in `session.extra_attrs` under a configurable key, making it available to other plugins (e.g., for anti-loop or routing decisions).

## Usage

```toml
rmqtt-bridge-origin = "0.22"
```

```rust
rmqtt_bridge_origin::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-bridge-origin.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `ingress_marker` | `string` | `":ingress:"` | Substring to identify ingress bridge clients |
| `egress_marker` | `string` | `":egress:"` | Substring to identify egress bridge clients |
| `attr_key` | `string` | `"bridge_origin"` | Key in `session.extra_attrs` to store the origin |

### Config Source

Loaded via `scx.plugins.load_config_default::<PluginConfig>("rmqtt-bridge-origin")`.

```toml
# rmqtt-bridge-origin.toml
ingress_marker = ":ingress:"
egress_marker = ":egress:"
attr_key = "bridge_origin"
```

## Dependencies

`rmqtt` (feature `plugin`)

## License

MIT OR Apache-2.0
