[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-counter

[![crates.io](https://img.shields.io/crates/v/rmqtt-counter.svg)](https://crates.io/crates/rmqtt-counter)

Metrics counter plugin. Tracks MQTT events through 15 hook callbacks at `Priority::MAX`.

## Events Tracked

**Client**: `ClientConnect`, `ClientAuthenticate`, `ClientConnack`, `ClientConnected`, `ClientDisconnected`
**Session**: `SessionCreated`, `SessionTerminated`, `SessionSubscribed`, `SessionUnsubscribed`
**Subscribe**: `ClientSubscribe`, `ClientUnsubscribe`, `ClientSubscribeCheckAcl`, `MessagePublishCheckAcl`
**Message**: `MessagePublish`, `MessageDelivered`, `MessageAcked`, `MessageDropped`, `MessageNonsubscribed`

Messages are further broken down by QoS level (0, 1, 2) and source type (Custom, Admin, System, LastWill, Bridge).

## Usage

### Build

Requires `rmqtt` features: `plugin`, `metrics`.

```toml
rmqtt-counter = "0.21"
```

### Register

```rust
rmqtt_counter::register(&scx, true, false).await?;
// or with explicit name:
rmqtt_counter::register_named(&scx, "rmqtt-counter", true, false).await?;
```

Parameters: `(scx, default_startup, immutable)`.

This plugin has no configuration file (no `cfg` field). All counters are accessed via `scx.metrics.*`.

## Dependencies

`rmqtt` (features: `plugin`, `metrics`)

## License

MIT OR Apache-2.0
