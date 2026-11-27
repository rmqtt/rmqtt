[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-flapping

[![crates.io](https://img.shields.io/crates/v/rmqtt-flapping.svg)](https://crates.io/crates/rmqtt-flapping)

Connection flapping protection. Counts CONNECT attempts per ClientId, username and source address in a sliding window, and refuses the offenders for a configurable time once they cross the threshold.

## Overview

A broken device reconnecting in a loop, a runaway client, and a credential-stuffing attempt all look alike from the outside: many CONNECT packets from the same origin in a short time. The plugin screens them on the `ClientConnect` hook — the one hook **every** connection reaches, including the ones that carry no username and therefore never enter the authentication chain.

- **Sliding window per key**: a window of `window_time` and a counter of the attempts inside it. The `max_count`-th attempt inside the window is itself refused and bans the key.
- **Ban, then let it lapse**: a ban lasts `ban_time` and is never extended while it is in force. Creating a ban discards the key's window, so the post-ban budget starts from zero (otherwise, with `ban_time < window_time`, the first attempt after the ban would immediately re-ban).
- **Bounded tables**: at most `max_track` keys are tracked per dimension. Once a table is full, new keys are let through **untracked** rather than refused — refusing would turn a flood of random ClientIds into an outage for everyone else.
- **Cheap cleanup**: every deadline is also recorded in a time-ordered index, so the periodic sweep only walks the entries that actually expired, and an idle broker pays one clock reading per interval.
- **Readable from outside**: the bans are published through the core's `rmqtt::flapping::Flapping` extension slot, which is what backs the HTTP API endpoints and the `flapping_banned` stats.

### Refusal codes

| MQTT version | Code | Name |
|--------------|------|------|
| 5.0 | `0x8A` | Banned |
| 3.1.1 / 3.1 | `0x05` | Not authorized |

v3 has no "banned" code, so the closest one is used. `0x03` (*Server unavailable*) is deliberately not used: it invites an immediate reconnect.

## Usage

### Enable

In `rmqtt.toml`, uncomment `rmqtt-flapping` in `plugins.default_startups`:

```toml
plugins.default_startups = [
    #"rmqtt-acl",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    "rmqtt-flapping",       # <- enable
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

Or add the dependency to `rmqttd/Cargo.toml`, or go through the `rmqtt-plugins` meta-crate:

```toml
# direct dependency
rmqtt-flapping = { version = "0.25" }

# or via the meta-crate
rmqtt-plugins = { version = "0.25", features = ["flapping"] }
```

### Register

```rust
rmqtt_flapping::register(&scx, true, false).await?;
```

Arguments: `(scx, default_startup, immutable)`. When integrated through `rmqttd`, whether the plugin is registered at startup is controlled by the `package.metadata.plugins` entry in `rmqtt-bin/Cargo.toml` together with `plugins.default_startups` / `plugins.disabled_default_startups` in `rmqtt.toml` (`rmqtt-flapping` does **not** register at startup by default).

## Configuration

Configuration file: `rmqtt-flapping.toml` (in the plugin configuration directory). Loaded through `scx.plugins.read_config_default::<PluginConfig>("rmqtt-flapping")`; a reload re-reads the policy tables and the exemption lists without dropping the bans already in force.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable` | `bool` | `true` | Whether the gate screens connections at all. With `false` the plugin loads and reports its state, but refuses nothing. |
| `max_track` | `usize` | `100_000` | Upper bound on the keys tracked **per dimension**. Once it is full, attempts that would create a new key are let through untracked instead of being refused. |
| `gc_interval` | `duration` | `"10s"` | How often expired windows and lapsed bans are swept. Must be at least `1s`. |
| `by_clientid` | table | (absent) | Policy table for the `clientid` dimension. Presenting it enables the dimension. |
| `by_username` | table | (absent) | Policy table for the `username` dimension. Connections without a username are not counted. |
| `by_peerhost` | table | (absent) | Policy table for the source-IP dimension. The only one that catches a client cycling through random ClientIds. |
| `notify_sys_topic` | `bool` | `true` | Master switch for the `$SYS` announcements. |
| `sys_topic` | `string` | `"$SYS/brokers/{node}/flapping/banned"` | Topic bans are announced on. `{node}` is replaced with the node id. |
| `sys_topic_qos` | `u8` | `0` | QoS of the announcements. |
| `message_expiry_interval` | `duration` | `"5m"` | Lifetime of the announcements, consumed by `rmqtt-retainer` when it is loaded. |
| `notify_on_every_refusal` | `bool` | `false` | Also announce every attempt refused while a ban is in force. |
| `notify_on_unban` | `bool` | `true` | Announce a ban lifted through the management API. Bans that simply lapse are never announced. |
| `sys_topic_unban` | `string` | `"$SYS/brokers/{node}/flapping/unbanned"` | Topic manual unbans are announced on. |
| `allow_clientids` | `array` | `[]` | ClientIds never counted and never banned (exact match). |
| `allow_usernames` | `array` | `[]` | Usernames never counted and never banned (exact match). |
| `allow_peerhosts` | `array` | `[]` | Source IP addresses never counted and never banned (exact match, no port). |

Each policy table has three keys:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `window_time` | `duration` | `"1m"` | The width of the window attempts are counted in. |
| `max_count` | `usize` | `15` | The number of attempts inside the window that triggers a ban. The attempt that reaches the count is itself refused. `0` is replaced with `1`. |
| `ban_time` | `duration` | `"5m"` | How long a ban lasts. |

### Example

```toml
enable = true
max_track = 100_000
gc_interval = "10s"

[by_clientid]
window_time = "1m"
max_count = 15
ban_time = "5m"

#[by_username]
#window_time = "1m"
#max_count = 15
#ban_time = "5m"

#[by_peerhost]
#window_time = "30s"
#max_count = 100
#ban_time = "10m"

notify_sys_topic = true
sys_topic = "$SYS/brokers/{node}/flapping/banned"
notify_on_unban = true
sys_topic_unban = "$SYS/brokers/{node}/flapping/unbanned"

allow_clientids = []
allow_usernames = []
allow_peerhosts = []
```

## HTTP API

Requires the `rmqtt-http-api` plugin. Both endpoints answer for the node that receives the request; the ban table is node-local, so there is no cluster-wide aggregate.

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/flapping/banned` | List the bans in force. Query parameters `dimension` (`clientid`/`username`/`peerhost`, empty = all), `key` (exact match), `offset`, `limit`. Responds `{ "available": bool, "items": [...], "has_more": bool, "banned_count": n }`; `available` is `false` when no gate is installed or it is not screening, which is what tells "the feature is not in use" apart from "nothing is banned". `/api/v1/features` reports the same state under `flapping`. |
| `DELETE /api/v1/flapping/banned` | Lift one ban by hand. `dimension` and `key` are both **query parameters** (a key is matched as typed and may contain `/`). Responds `{ "dimension": ..., "key": ..., "unbanned": true }`; `404` when there was nothing to lift. |

## Metrics

| Metric | Where | Meaning |
|--------|-------|---------|
| `conn.flapping.banned` | metrics | Cumulative bans **created**. |
| `conn.flapping.refused` | metrics | Cumulative connections refused because a ban was in force. |
| `flapping_banned.count` | stats | Bans currently **in force**; falls when a ban lapses. |
| `flapping_banned.max` | stats | Historical peak of the count above. |

## Limitations

- **Node-local state.** A client reconnecting through a load balancer is counted separately on each node, so with `N` nodes an offender makes roughly `N × max_count` attempts before any single node refuses it. Set `max_count` accordingly.
- **In memory only.** A broker restart clears every ban.
- **Does not kick.** The plugin screens new connections; a client that was already connected when the ban was created stays connected. Use `DELETE /api/v1/clients/{clientid}` for that.
- **Fail-open at capacity**, as described above.
- **An earlier handler can silently bypass the gate.** The plugin registers at `Priority::MAX / 2` on `ClientConnect`; a handler above that which short-circuits stops the chain before the gate runs, with nothing reported. Keep such a handler below `MAX / 2` — the two observers (`rmqtt-counter`, `rmqtt-web-hook`) sit at `MAX` and never short-circuit.

## Dependencies

`rmqtt` (features: `plugin`, `msgstore`, `metrics`, `flapping`), `tokio`, `async-trait`, `log`, `serde`, `serde_json`, `bytes`, `bytestring`, `chrono`, `parking_lot`

## License

MIT OR Apache-2.0
