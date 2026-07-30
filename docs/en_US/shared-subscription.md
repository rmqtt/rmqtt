English | [简体中文](../zh_CN/shared-subscription.md)

# Shared Subscription

Shared subscription enables load-balanced consumer groups using the `$share/{group}/{topic}` topic filter format. When multiple clients subscribe to the same shared group, published messages are delivered to **only one** subscriber in the group, enabling horizontal scaling of message consumers.

This feature is provided by the [rmqtt-shared-subscription](https://github.com/rmqtt/rmqtt/tree/master/rmqtt-plugins/rmqtt-shared-subscription) plugin.

## How It Works

A shared subscription topic filter follows the format:

```
$share/{group}/{topic}
```

- `$share` — The shared subscription prefix
- `{group}` — An arbitrary group name (e.g., `group1`, `sensors`)
- `{topic}` — A standard MQTT topic filter (e.g., `sensor/+/temp`)

All clients subscribed to the same `{group}` and `{topic}` form a consumer group. When a message matches the topic filter, the broker selects **one** subscriber from the group to receive it.

## Selection Strategies

The plugin supports seven configurable strategies for subscriber selection:

| Strategy | Description | Use Case |
|----------|-------------|----------|
| `random` | Randomly selects an online subscriber | Basic load balancing |
| `round_robin` (default) | Sequential round-robin using an atomic counter | Equal-capacity consumers |
| `round_robin_per_group` | Independent round-robin per shared group | Large cluster with many groups |
| `sticky` | Binds a publisher to a fixed subscriber via LRU cache | Stateful processing, session affinity |
| `local` | Prefers subscribers on the same broker node as the publisher | Reduce cross-node traffic |
| `hash_clientid` | Deterministic selection by hashing the publisher's ClientId | Per-device message ordering |
| `hash_topic` | Deterministic selection by hashing the topic name | Topic sharding |

### sticky strategy

When using the `sticky` strategy, the plugin maintains an LRU cache of publisher-to-subscriber bindings. The cache size is configurable with `sticky_cache_size`. When the cache is full, the least recently used binding is evicted automatically.

## Configuration

#### Plugin:

```bash
rmqtt-shared-subscription
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-shared-subscription.toml
```

#### Plugin Configuration Options:

```bash
##--------------------------------------------------------------------
## rmqtt-shared-subscription
##--------------------------------------------------------------------

# Selection strategy
# Valid values: random, round_robin, round_robin_per_group, sticky, local, hash_clientid, hash_topic
# Default: round_robin
strategy = "round_robin"

# Maximum sticky bindings in the LRU cache (only applies to "sticky" strategy).
# When the cache fills up, the least recently used binding is evicted.
# Default: 100000
# sticky_cache_size = 100000
```

#### Enabling the Plugin

Shared subscriptions are automatically available on all listeners once the plugin is loaded. Add `rmqtt-shared-subscription` to the `plugins.default_startups` list in `rmqtt.toml`:

```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
plugins.dir = "rmqtt-plugins/"
plugins.default_startups = [
    "rmqtt-shared-subscription",
    # ... other plugins
]
```

No per-listener configuration is required.
