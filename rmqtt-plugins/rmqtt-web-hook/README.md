[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-web-hook

[![crates.io](https://img.shields.io/crates/v/rmqtt-web-hook.svg)](https://crates.io/crates/rmqtt-web-hook)

Webhook plugin. Sends MQTT events as HTTP POST notifications to external systems. Supports 15 hook event types.

## Overview

Uses an async producer-consumer architecture. Handlers serialize events to JSON and send them through an mpsc channel. A background task consumes and dispatches to configured URLs. Supports HTTP POST and local file output. Uses exponential backoff retry on HTTP failures.

### Events

**Session**: session_created, session_terminated, session_subscribed, session_unsubscribed

**Client**: client_connect, client_connack, client_connected, client_disconnected, client_subscribe, client_unsubscribe

**Message**: message_publish, message_delivered, message_acked, message_dropped, offline_message

## Usage

```toml
[dependencies]
rmqtt-web-hook = "0.22"
```

```rust
rmqtt_web_hook::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-web-hook.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `queue_capacity` | integer | `300_000` | Maximum queued tasks before rejection |
| `concurrency_limit` | integer | `128` | Maximum concurrent task execution |
| `urls` | array of string | `["file:///var/log/rmqtt/hook.log"]` | HTTP POST or file output URLs |
| `http_timeout` | string | `"8s"` | HTTP request timeout |
| `retry_max_elapsed_time` | string | `"60s"` | Maximum retry elapsed time |
| `retry_multiplier` | float | `2.5` | Exponential backoff multiplier |
| `rule.session_created` | array of rule | `[{action = "session_created"}]` | Session created hook rules |
| `rule.session_terminated` | array of rule | `[{action = "session_terminated"}]` | Session terminated hook rules |
| `rule.session_subscribed` | array of rule | `[{action = "session_subscribed"}]` | Session subscribed hook rules |
| `rule.session_unsubscribed` | array of rule | `[{action = "session_unsubscribed"}]` | Session unsubscribed hook rules |
| `rule.client_connect` | array of rule | `[{action = "client_connect"}]` | Client connect hook rules |
| `rule.client_connack` | array of rule | `[{action = "client_connack"}]` | Client connack hook rules |
| `rule.client_connected` | array of rule | `[{action = "client_connected"}]` | Client connected hook rules |
| `rule.client_disconnected` | array of rule | `[{action = "client_disconnected"}]` | Client disconnected hook rules |
| `rule.client_subscribe` | array of rule | `[{action = "client_subscribe"}]` | Client subscribe hook rules |
| `rule.client_unsubscribe` | array of rule | `[{action = "client_unsubscribe"}]` | Client unsubscribe hook rules |
| `rule.message_publish` | array of rule | `[{action = "message_publish", topics = ["#", "$SYS/#"]}]` | Message publish hook rules |
| `rule.message_delivered` | array of rule | `[{action = "message_delivered", topics = ["#", "$SYS/#"]}]` | Message delivered hook rules |
| `rule.message_acked` | array of rule | `[{action = "message_acked", topics = ["#", "$SYS/#"]}]` | Message acked hook rules |
| `rule.message_dropped` | array of rule | `[{action = "message_dropped"}]` | Message dropped hook rules |
| `rule.offline_message` | array of rule | `[{action = "offline_message", topics = ["#", "$SYS/#"]}]` | Offline message hook rules |

Each `rule.*` entry is an array of rule objects. Each rule supports:

| Rule Field | Type | Description |
|------------|------|-------------|
| `action` | string | Action name matching the event type |
| `topics` | array of string | (Optional) Topic filter. Only trigger for matching topics |

Example:

```toml
rule.session_created = [{action = "session_created"}]
rule.message_publish = [{action = "message_publish", topics = ["#", "$SYS/#"]}]
```

## Dependencies

`rmqtt` (feature `plugin`), `reqwest` (rustls-tls, json)

## License

MIT OR Apache-2.0
