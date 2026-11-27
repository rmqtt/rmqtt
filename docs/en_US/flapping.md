[**English**](flapping.md) | [简体中文](../zh_CN/flapping.md)

# Connection Flapping Protection

A broken device that reconnects in a tight loop, a runaway client, or a credential-stuffing
attempt all look the same from the outside: many CONNECT packets from the same origin in a
short time. `rmqtt-flapping` counts those attempts per ClientId, username and source address
in a sliding window, and once a key crosses its threshold it is **banned** — every further
attempt from it is refused for a configurable time, then the ban lapses on its own.

#### Why a plugin

The decision hangs on the `ClientConnect` hook, which **every** connection reaches, including
the ones that carry no username. Those never enter the authentication chain at all (with
`allow_anonymous = true` and no username, `client_authenticate` returns immediately), so an
authentication plugin cannot see them and cannot gate them. `ClientConnect` can, so nothing in
the broker core has to change to screen connections.

The screen runs **before authentication**: a banned client costs the broker a hash lookup
instead of a password verification, and the ban cannot be bypassed by presenting valid
credentials.

#### Plugin

```
rmqtt-flapping
```

#### Plugin configuration file

```
plugins/rmqtt-flapping.toml
```

#### Plugin configuration

```toml
##--------------------------------------------------------------------
## rmqtt-flapping
##--------------------------------------------------------------------
## See https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/flapping.md

## Whether the gate screens connections at all.
enable = true

## Upper bound on the keys tracked *per dimension*. Once it is full the
## attempts that would have created a new key are let through untracked
## instead of being refused.
max_track = 100_000

## How often expired windows and lapsed bans are swept. Must be at least 1s.
gc_interval = "10s"

## A dimension is enabled by presenting its policy table. The three tables
## below are commented out except `by_clientid`, so this file enables exactly
## one dimension.
[by_clientid]
## The width of the window attempts are counted in.
window_time = "1m"
## The number of attempts inside the window that triggers a ban. The attempt
## that reaches the count is itself refused.
max_count = 15
## How long a ban lasts. A ban is never extended while it is in force.
ban_time = "5m"

#[by_username]
#window_time = "1m"
#max_count = 15
#ban_time = "5m"

#[by_peerhost]
#window_time = "30s"
#max_count = 100
#ban_time = "10m"

## Notification
notify_sys_topic = true
sys_topic = "$SYS/brokers/{node}/flapping/banned"
sys_topic_qos = 0
message_expiry_interval = "5m"
notify_on_every_refusal = false
notify_on_unban = true
sys_topic_unban = "$SYS/brokers/{node}/flapping/unbanned"

## Exemptions (exact matches, checked per dimension)
allow_clientids = []
allow_usernames = []
allow_peerhosts = []
```

## Algorithm

### Dimensions

A *dimension* is an independent policy and an independent table. A connection is screened once
per enabled dimension, and the first dimension that refuses it decides the outcome.

| Dimension | Keyed by | Config table | Notes |
|-----------|----------|--------------|-------|
| `clientid` | ClientId | `[by_clientid]` | A client that connects with an empty ClientId and `CleanSession` / `CleanStart` set to `1` gets a **server-generated** ClientId, so those clients never share a key. Only `by_peerhost` catches them. |
| `username` | Username | `[by_username]` | Connections that carry no username are not counted. |
| `peerhost` | Source IP address | `[by_peerhost]` | No port. This is the dimension that catches a client cycling through random ClientIds. |

A dimension that is not configured is simply not screened. With **all** tables absent the
plugin loads but is inert — it reports its (empty) state and refuses nothing.

### Sliding window and the threshold

Each key has a window of `window_time` and a counter of the attempts seen inside it.

* `max_count` is the number of attempts inside the window that triggers the ban, and **the
  attempt that reaches the count is itself refused**. With `max_count = 4`, attempts 1, 2 and 3
  are accepted and attempt 4 is refused and bans the key.
* A ban lasts `ban_time`. It is **never extended while it is in force**: an offender that keeps
  hammering at the door does not push its own release further away, and the moment the ban
  lapses it regains a full budget of attempts.
* `max_count = 0` is treated as a configuration mistake and replaced with `1`, with a warning.

### Why the window is dropped when a ban is created

If the offending window survived the ban, then with `ban_time` shorter than `window_time` the
very first attempt after the ban lapsed would find the old attempts still in the window and
ban the key again — the ban would never end. So creating a ban **discards the key's window**,
and the post-ban budget always starts from zero. Pick `ban_time` and `window_time` with that in
mind: the pair describes "N attempts per window, then a rest of M".

### Capacity, GC and the deadline index

The window tables and the ban tables are bounded by `max_track` keys **per dimension**. This
bound is what keeps a flood of random ClientIds from growing a table without limit.

When a key that has never been seen arrives and the table is full, the gate runs one cleanup
pass and, if that freed nothing, **lets the attempt through untracked**. Refusing instead would
turn a flood of unknown keys into an outage for every other client, so the gate fails open: an
attacker cycling through random ClientIds simply stops being counted once the table is
saturated. Size `max_track` above the number of *legitimate* keys you expect per dimension, and
rely on `by_peerhost` — whose key space is a small set of IP addresses — for that attack.

Cleanup runs every `gc_interval` and drops the entries that have expired. It is deliberately
cheap: every deadline (window expiry and ban expiry) is also recorded in a time-ordered index
(`BTreeSet<(deadline_ms, key)>`), and a sweep only pops entries off the **front** of that index
until it meets one that is not due yet. The cost of a sweep is therefore proportional to the
number of entries that actually expired, not to the size of the tables, and an idle broker pays
one clock reading per interval. `gc_interval` only trades memory retention against wake-ups —
the hot path never touches the index, and the index lock is never held while a table lock is
taken.

## Refusal codes

The plugin answers every refusal with the same reason: **banned**.

| MQTT version | Code | Name | Why this code |
|--------------|------|------|---------------|
| 5.0 | `0x8A` | Banned | The v5 code that means exactly this. |
| 3.1.1 / 3.1 | `0x05` | Not authorized | v3 has no "banned" code, so the closest one is used. |

For v3, `0x03` (*Server unavailable*) is deliberately **not** used: it tells the client the
server is temporarily unavailable, which invites an immediate reconnect — precisely the
behaviour the ban is meant to stop. `0x05` is a terminal answer.

The reason text of the v5 code is available from `rmqtt-codec`
(`ConnectAckReason::reason()` → `"banned"`).

## Notification

### `$SYS` topics

One message per event, published by the broker's internal `system` client, so any plugin can
see and rewrite it through the ordinary publish hook.

| Option | Default | Effect |
|--------|---------|--------|
| `notify_sys_topic` | `true` | Master switch for the two topics below. |
| `sys_topic` | `$SYS/brokers/{node}/flapping/banned` | `{node}` is replaced with the node id. |
| `sys_topic_qos` | `0` | Kept at most once by default: a ban notice must not be retried into a business system that is already struggling. |
| `message_expiry_interval` | `5m` | Consumed by `rmqtt-retainer` when it happens to be loaded. |
| `notify_on_every_refusal` | `false` | Also announce **every** attempt refused while a ban is in force, not just the ban itself. Off by default: a determined offender would otherwise fill the topic with one message per reconnect. |
| `notify_on_unban` | `true` | Announce a ban lifted through the management API. A ban that simply lapses is **never** announced, so the topic stays free of routine noise. |
| `sys_topic_unban` | `$SYS/brokers/{node}/flapping/unbanned` | Topic for manual unbans. |

**Ban created** — `event` is `"banned"`:

```json
{
  "node": 1,
  "event": "banned",
  "dimension": "clientid",
  "key": "device-01",
  "clientid": "device-01",
  "username": "sensor",
  "ipaddress": "192.168.1.10:54321",
  "listener_id": 1,
  "proto_ver": 5,
  "count": 4,
  "window_time_secs": 60,
  "ban_time_secs": 300,
  "banned_at": "2026-09-27 11:02:14.118",
  "banned_until": "2026-09-27 11:07:14.118"
}
```

`clientid`, `username` and `ipaddress` are repeated from the attempt that triggered the ban:
the ban itself may be keyed by any of the three dimensions, and a reader of the topic wants to
know who was behind it. `ipaddress` is the source address **with its port**
(`192.168.1.10:54321`), unlike the `by_peerhost` key and `allow_peerhosts`, which take the bare
address; `listener_id` is the numeric id of the listener the attempt arrived on; `username` is
`null` when the client sent none.

**Manual unban** — `event` is `"unbanned"`:

```json
{
  "node": 1,
  "event": "unbanned",
  "reason": "manual",
  "dimension": "clientid",
  "key": "device-01",
  "count": 4,
  "clientid": "device-01",
  "ipaddress": "192.168.1.10:54321",
  "banned_at": "2026-09-27 11:02:14.118",
  "banned_until": "2026-09-27 11:07:14.118",
  "unbanned_at": "2026-09-27 11:03:02.005"
}
```

`reason` is always `"manual"`: bans that lapse are not announced.

**Refusal while banned** — `event` is `"refused"`, only when `notify_on_every_refusal = true`:

```json
{
  "node": 1,
  "event": "refused",
  "dimension": "clientid",
  "key": "device-01",
  "clientid": "device-01",
  "username": "sensor",
  "ipaddress": "192.168.1.10:54321",
  "listener_id": 1,
  "proto_ver": 5,
  "time": "2026-09-27 11:02:31.777"
}
```

All timestamps are formatted as `%Y-%m-%d %H:%M:%S%.3f` in local time, like the rest of the
broker's `$SYS` messages, so they can be compared as strings.

### WebHook

A refusal also raises the `client_connack` web-hook event (`rmqtt-web-hook`), whose body
already carries `clientid`, `username`, `ipaddress`, `proto_ver` and `conn_ack`. Enabling
`client_connack` in the web-hook rules is enough to receive refusals; no extra plugin support is
needed.

`conn_ack` carries the refusal reason — `"Banned"` for v5, `"Not authorized"` for v3.1.1 —
which is drawn from the same wording as the `$SYS` payload above.

## Metrics

| Metric | Where | Meaning |
|--------|-------|---------|
| `conn.flapping.banned` | `/api/v1/metrics`, Prometheus | Cumulative number of bans **created** (one per detection, not one per refused attempt). |
| `conn.flapping.refused` | `/api/v1/metrics`, Prometheus | Cumulative number of connections refused because a ban was in force. |
| `flapping_banned.count` | `/api/v1/stats`, Prometheus | Bans currently **in force** on this node. It **falls** when a ban lapses, which is why it lives in `stats` rather than in the monotonic counters above. |
| `flapping_banned.max` | `/api/v1/stats`, Prometheus | Historical peak of `flapping_banned.count`. |

All four stay at zero when no gate plugin is installed. `flapping_banned.count` is refreshed by
the ordinary stats snapshot, so it follows the broker's stats interval rather than a clock of
its own.

## HTTP API

Requires the `rmqtt-http-api` plugin to be loaded. Both endpoints answer **for the node that
receives the request**, not for the cluster — see *Cluster semantics* below.

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/flapping/banned` | List the bans in force on this node. |
| `DELETE /api/v1/flapping/banned` | Lift one ban by hand. |

### List bans

```
GET /api/v1/flapping/banned?dimension=clientid&key=device-01&offset=0&limit=100
```

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `dimension` | `string` | (all) | `clientid`, `username` or `peerhost`. Empty means every dimension. An unknown name is a `400`, not a silently ignored filter. |
| `key` | `string` | (all) | Exact match against the ban's key. |
| `offset` | `usize` | `0` | Entries to skip. |
| `limit` | `usize` | `max_row_limit` | Page size, clamped to `max_row_limit`. |

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/flapping/banned?dimension=clientid"

{
  "available": true,
  "items": [
    {
      "dimension": "clientid",
      "key": "device-01",
      "count": 4,
      "banned_at": "2026-09-27 11:02:14.118",
      "banned_until": "2026-09-27 11:07:14.118",
      "remaining_ms": 282000,
      "last_clientid": "device-01",
      "last_ipaddress": "192.168.1.10:54321"
    }
  ],
  "has_more": false,
  "banned_count": 1
}
```

`count` is the number of attempts in the window that triggered the ban; `remaining_ms` is
measured when the answer is built.

`available` says whether a gate is installed and screening on this node. It is `false` when the
plugin is not loaded, or is loaded but not screening (no dimension enabled, or `enable = false`);
every other field is then truthfully empty. That flag is what tells "the feature is not in use"
apart from "the gate is screening and nothing is banned" — `banned_count` alone cannot, since
both read `0`. The answer stays a `200` in that state: an empty list is a valid answer, and the
endpoint has no error to report. `GET /api/v1/features` reports the same state under `flapping`,
for a caller that would rather ask once.

### Lift a ban

```
DELETE /api/v1/flapping/banned?dimension=clientid&key=device-01
```

Both parameters travel as **query parameters**, never as path segments: a key is matched
exactly as typed (a ClientId may legally contain a space or a `/`), so it cannot be safely
squeezed into a path.

```bash
$ curl -i -X DELETE "http://localhost:6060/api/v1/flapping/banned?dimension=clientid&key=device-01"

{ "dimension": "clientid", "key": "device-01", "unbanned": true }
```

* `400` — `dimension` missing or unknown, or `key` missing/empty.
* `404` — there was no ban on that key to lift. A gate that is not screening holds no bans at
  all, so it answers `404` for every key: there is nothing to lift, which is what this code
  already says.
* `200` — the ban was lifted; the next connection from that key is accepted.

Lifting a ban announces it on `sys_topic_unban` when `notify_on_unban` is set (the default).

## Exemptions

```toml
allow_clientids = ["monitor-01"]
allow_usernames = ["admin"]
allow_peerhosts = ["192.168.1.10"]
```

Exact matches, checked per dimension. Exempt keys are neither counted nor banned, so a
monitoring client that reconnects often does not trip the gate. `allow_peerhosts` takes bare
addresses, without a port. The lists are re-read on a configuration reload; a reload does not
touch the tables themselves, so reloading a configuration never drops a ban in force.

## Cluster semantics

Both the counters and the ban table are **node-local**. Nothing ties a ClientId to a node, so a
client that reconnects through a load balancer spreads its attempts over the cluster and is
counted separately on each node. This matches EMQX's behaviour for the same feature, and it
means `max_count` has to be set with the cluster size in mind:

> With `N` nodes behind the balancer, an offender can make roughly `N` times `max_count`
> attempts before any single node refuses it.

The `$SYS` announcements are node-local for the same reason — subscribe to
`$SYS/brokers/+/flapping/banned` to see all of them. The HTTP API deliberately does **not**
aggregate: an aggregate would advertise bans this node does not enforce, so the ban list is
answered per node.

## Enabling

By default the plugin is **not** started. Add it to `plugins.default_startups` in `rmqtt.toml`:

```toml
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
plugins.dir = "rmqtt-plugins/"
plugins.default_startups = [
    #"rmqtt-acl",
    #"rmqtt-retainer",
    "rmqtt-flapping",       # <- enable
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

The configuration file name follows the crate name (`rmqtt-flapping.toml`). The gate refuses
nothing until `enable = true` **and** at least one policy table are present — without a
dimension there is nothing to screen — so a configuration that loads the plugin but configures
no dimension is a no-op rather than a mistake.

## Limitations

* **Node-local state**, as described under *Cluster semantics*.
* **The ban table is in memory only.** A broker restart clears every ban. To make bans survive
  a restart, put the plugin's state behind a cluster-wide store (not implemented).
* **No kicking.** The plugin screens new connections; it does not disconnect a client that was
  already connected when the ban was created. Use `DELETE /api/v1/clients/{clientid}` for that.
* **Fail-open at capacity.** Once `max_track` keys are tracked in a dimension, new keys are let
  through untracked instead of being refused (see *Capacity, GC and the deadline index*).
* **Per-node throughput is unaffected**, since the screen is a hash lookup per enabled
  dimension on a path that already takes one.
* **A handler running earlier can silently bypass the gate.** The plugin takes
  `Priority::MAX / 2` on `ClientConnect`, behind the two pure observers (`rmqtt-counter` and
  `rmqtt-web-hook`, both at `Priority::MAX` and neither of which ever short-circuits). Add a
  `ClientConnect` handler **above `MAX / 2`** that short-circuits (`proceed = false`) and the
  chain stops there: the gate never runs, and nothing says so. A handler that has to coexist
  with the gate belongs **below `MAX / 2`**.

## License

MIT OR Apache-2.0
