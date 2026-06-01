[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-topic-rewrite

[![crates.io](https://img.shields.io/crates/v/rmqtt-topic-rewrite.svg)](https://crates.io/crates/rmqtt-topic-rewrite)

Topic rewrite plugin for RMQTT. Rewrites/remaps MQTT topic filters and topic names using configurable rules.

## Overview

Applies configurable rewrite rules to transform topic names during publish and topic filters during subscribe/unsubscribe. This allows internal topic namespace remapping without changing client behavior.

### Use Cases

- **Namespace migration**: Remap old topic structures to new ones without updating clients
- **Multi-tenant isolation**: Prefix topics with tenant/client identifiers
- **Protocol bridging**: Translate between different topic naming conventions
- **Access control**: Rewrite sensitive topic names before they reach certain clients

### How Rules Work

Each rule specifies:
- `action`: When to apply the rule (`publish`, `subscribe`, or `all`)
- `source_topic_filter`: The incoming topic filter to match
- `dest_topic`: The target topic after rewriting (can use `$N` capture groups and `${clientid}`/`${username}` placeholders)
- `regex`: Optional regular expression to extract capture groups from the source topic

## Usage

Add the dependency to `Cargo.toml`:

```toml
rmqtt-topic-rewrite = "0.22"
```

Register the plugin in your broker startup code:

```rust
rmqtt_topic_rewrite::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-topic-rewrite.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `rules` | Array of tables | `[]` | List of topic rewrite rules |

### Rule Fields

| Field | Type | Description |
|-------|------|-------------|
| `action` | String | Apply to: `"publish"`, `"subscribe"`, or `"all"` |
| `source_topic_filter` | String | Topic filter to match against the original topic |
| `dest_topic` | String | Destination topic after rewriting. Supports `$1`, `$2`... capture groups, `${clientid}`, `${username}` |
| `regex` | String (optional) | Regular expression to extract capture groups from the source topic |

### Variable Placeholders

| Placeholder | Description |
|-------------|-------------|
| `${clientid}` | Replaced with the client's client ID |
| `${username}` | Replaced with the client's username |
| `$1`, `$2`, ... | Capture groups from the regex match |

### Default Rules (all commented out)

The default configuration includes commented-out examples demonstrating various rewrite patterns:

```toml
rules = [
    # All actions: extract segments via regex and remap
    # { action = "all", source_topic_filter = "x/+/#", dest_topic = "xx/$1/$2", regex = "^x/(.+)/(.+)$" },

    # Direct topic mapping (no regex)
    # { action = "all", source_topic_filter = "x/y/1", dest_topic = "xx/y/1" },

    # Wildcard subscription rewrite using regex capture
    # { action = "all", source_topic_filter = "x/y/#", dest_topic = "xx/y/$1", regex = "^x/y/(.+)$" },

    # Client ID substitution
    # { action = "all", source_topic_filter = "iot/cid/#", dest_topic = "iot/${clientid}/$1", regex = "^iot/cid/(.+)$" },

    # Username substitution
    # { action = "all", source_topic_filter = "iot/uname/#", dest_topic = "iot/${username}/$1", regex = "^iot/uname/(.+)$" },

    # Multi-segment remapping
    # { action = "all", source_topic_filter = "a/+/+/+", dest_topic = "aa/$1/$2/$3", regex = "^a/(.+)/(.+)/(.+)$" }
]
```

### Example: Effective Rules

```toml
rules = [
    # Rewrite publish topics: "sensor/+/temp" → "telemetry/$1/temp"
    { action = "publish", source_topic_filter = "sensor/+/temp", dest_topic = "telemetry/$1/temp", regex = "^sensor/(.+)/temp$" },

    # Rewrite subscribe topics: "device/#" → "devices/${clientid}/#"
    { action = "subscribe", source_topic_filter = "device/#", dest_topic = "devices/${clientid}/$1", regex = "^device/(.+)$" },

    # Direct mapping for all actions
    { action = "all", source_topic_filter = "old/metrics/#", dest_topic = "new/metrics/$1", regex = "^old/metrics/(.+)$" }
]
```

## Dependencies

- `rmqtt` (feature `plugin`)

## License

MIT OR Apache-2.0
