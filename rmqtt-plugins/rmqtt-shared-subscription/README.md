[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-shared-subscription

[![crates.io](https://img.shields.io/crates/v/rmqtt-shared-subscription.svg)](https://crates.io/crates/rmqtt-shared-subscription)

Shared subscription plugin for RMQTT. Implements subscriber selection strategies for `$share/{group}/{topic}` subscriptions.

## Overview

When multiple clients subscribe to a shared subscription group topic (e.g., `$share/group1/sensor/#`), published messages are delivered to **only one** subscriber in the group. This enables load-balanced consumer groups, commonly used in IoT and MQTT-based streaming scenarios.

This plugin registers a [`SharedSubscription`](https://docs.rs/rmqtt/latest/rmqtt/subscribe/trait.SharedSubscription.html) trait implementation that dispatches subscriber selection to one of seven configurable strategies.

## Strategies

| Strategy | Key | Description | Use Case |
|----------|-----|-------------|----------|
| `random` | `Random` | Selects a random online subscriber each time | Basic load balancing |
| `round_robin` (default) | `RoundRobin` | Global atomic counter rotates through subscribers | Equal-capacity consumers |
| `round_robin_per_group` | `RoundRobinPerGroup` | Independent counter per shared group | Large cluster with many groups |
| `sticky` | `Sticky` | Binds a publisher to a fixed subscriber via LRU cache | Stateful processing, session affinity |
| `local` | `Local` | Prefers subscribers on the same node as the publisher | Reduce cross-node traffic |
| `hash_clientid` | `HashClientId` | Hash by publisher's ClientId | Per-device message ordering |
| `hash_topic` | `HashTopic` | Hash by topic name | Topic sharding |

### Strategy Details

**random** — Picks a random online subscriber. Falls back to a local-node subscriber if none are online.

**round_robin** — Uses an atomic counter to distribute messages sequentially across all subscribers. Online subscribers are preferred; offline ones are skipped.

**round_robin_per_group** — Similar to round_robin but maintains a separate counter per shared group, so each group independently round-robins.

**sticky** — Once a subscriber is selected for a `(shared_group, publisher)` pair, that binding is cached and reused for subsequent messages. If the bound subscriber goes offline, the binding is removed and a new one is created. The cache uses an LRU eviction policy — see `sticky_cache_size` below.

**local** — Prioritises subscribers on the same broker node as the publisher. Falls back to cross-node subscribers if no local subscriber is online, then to any subscriber (even offline) on the local node.

**hash_clientid** — Uses `std::hash::DefaultHasher` on the publisher's ClientId to pick a subscriber deterministically. If that subscriber is offline, falls back to random online selection.

**hash_topic** — Same as `hash_clientid` but hashes the topic name instead, ensuring messages on the same topic always go to the same subscriber.

## Usage

### Enable the plugin

Add to the `plugins.default_startups` list in `rmqtt.toml`:

```toml
plugins.default_startups = [
    "rmqtt-shared-subscription",
    # ... other plugins
]
```

The plugin registers itself at startup via `plugins.default_startups`. No per-listener configuration is needed — shared subscriptions are automatically available on all listeners.

### Register from code (optional)

```rust
rmqtt_shared_subscription::register(register_fn);
```

## Configuration

File: `rmqtt-shared-subscription.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `strategy` | String | `"round_robin"` | Selection strategy — see Strategies table above |
| `sticky_cache_size` | Integer | `100000` | Max sticky bindings in LRU cache (only applies to `sticky` strategy) |

### Example

```toml
# Selection strategy
# Valid values: random, round_robin, round_robin_per_group, sticky, local, hash_clientid, hash_topic
strategy = "round_robin"

# Maximum sticky bindings in the LRU cache (only applies to "sticky" strategy).
# When the cache fills up, the least recently used binding is evicted.
sticky_cache_size = 100000
```

### Runtime Metrics

The plugin exposes internal state via the `attrs()` method (used by the plugin system's HTTP API):

| Field | Description | Available when strategy is |
|-------|-------------|---------------------------|
| `strategy` | Current strategy name | Always |
| `rr_counter` | Global round-robin counter value | `round_robin`, `round_robin_per_group` |
| `rr_group_count` | Number of per-group counters tracked | `round_robin`, `round_robin_per_group` |
| `rr_group_details` | Detailed per-group counter list (max 1000 entries) | `round_robin`, `round_robin_per_group` |
| `sticky_binding_count` | Number of active sticky bindings | `sticky` |
| `sticky_binding_details` | Detailed sticky binding list (max 1000 entries) | `sticky` |

## MQTT Protocol

Shared subscriptions use the `$share/{group}/{topic}` topic filter format:

- `$share/group1/sensor/#` — Subscribe to all topics under `sensor/` with group "group1"
- Messages matching the topic filter are delivered to exactly one subscriber in "group1"

## Dependencies

- `rmqtt` (features `plugin`, `shared-subscription`)
- `lru` 0.18 — LRU cache for sticky bindings

## License

MIT OR Apache-2.0
