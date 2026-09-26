# Changelog

All notable changes to RMQTT are documented in this file.

## [Unreleased]

### Major Changes

- **`ClientConnect` Hook Can Now Refuse a Connection**: `HookManager::client_connect` returned `Option<UserProperties>`, a value with no producer and no consumer in the repository — both call sites (`v3.rs` / `v5.rs`) discarded it with `let _ =` — so the hook could only observe a CONNECT, never answer it. It now returns `Option<ConnectRefuse>`, and the handshake refuses the Client with the reason code its protocol version defines, before authenticating it and before taking over the existing session of the same ClientId. This closes the one connection-stage gap `client_authenticate` cannot cover: while `allow_anonymous` is enabled, a Client that sends no username short-circuits that hook's chain entirely, so a policy that must see every connection attempt — a per-clientid or per-IP rate limiter, a ban list — had nowhere to live. The new public `ConnectRefuse` enum names the reason rather than the wire code (`Banned`, `ConnectionRateExceeded`, `NotAuthorized`, `ServerBusy`, `QuotaExceeded`, `UnspecifiedError`) and carries `to_v3()` / `to_v5()`, so a handler neither has to know which protocol level it is answering nor can emit a code that protocol does not define: MQTT 5.0 maps each variant to its own reason code (0x8A Banned, 0x9F Connection rate exceeded, 0x87 Not authorized, 0x89 Server busy, 0x97 Quota exceeded, 0x80 Unspecified error), while the six CONNACK return codes of MQTT 3.1 / 3.1.1 force a fallback chosen by what the Client is meant to do next — 0x03 Service unavailable for a refusal it may retry, 0x05 Not authorized for a policy refusal. `HookResult::UserProperties` is removed with it, equally unused (`hook.rs` held its only reference, the match that produced it), and a handler that refuses must return `proceed = false` alongside the refusal, since a chain accumulates into a single slot. Plugins are unaffected — they implement `Handler` and speak `HookResult` — so only an out-of-tree implementor of the `HookManager` trait has to adapt.

### Bug Fixes

- **Oversized Queued Message Destroys the Session** (issue #513): A queued Application Message larger than the Maximum Packet Size the client declared at CONNECT ([MQTT-3.1.2.11.4]) surfaced as an encoding error out of `Sink::publish`; the error propagated out of the session run loop, closed the sink and stranded every message queued behind it, leaving a session that never recovered yet still answered CONNACK as if healthy. The condition is now classified one layer up instead of being treated as fatal: the packet is discarded without being sent and the delivery is treated as **completed**, exactly as [MQTT-3.1.2-25] requires ("behave as if it had completed sending that Application Message"). `Sink::publish` returns `PublishOutcome::{Sent, DiscardedTooLarge}` rather than a bare success, the discard path fires the `message_dropped` hook with a new `Reason::MessageTooLarge` (v5 reason code `0x95` *Packet too large*, `disconnect: false`), and a new `rmqtt-net::is_over_max_packet_size()` helper recognises the codec's `OverMaxPacketSize` both when wrapped in `MqttError` and when surfaced as a bare `EncodeError` (`send_timeout = 0`). Only v5 sessions can take this path — the v3 codec has no outbound size limit. Regression cases: `rmqtt-test` (`functional_v5`) `oversized_queued_message_stalls_queue_v5` and `retained_oversized_message_keeps_session_v5`.
- **Will Message Suppressed by Any DISCONNECT** (issue #514): A stored Will Message is deleted by exactly one DISCONNECT reason code — `0x00` Normal disconnection ([MQTT-3.1.2-8/9]) — so `0x04` Disconnect with Will Message (the client explicitly asking for the Will to be sent) and every other code must leave it due. The broker honoured none of that: `last_will_enable()` keyed off `StateFlags::DisconnectReceived`, which the v5 DISCONNECT reader sets unconditionally, so any DISCONNECT suppressed the Will (reported for `0x04`, but equally broken for `0x80`, `0x82` and `0x93`). A new `Disconnect::deletes_will()` encodes the rule, true only for a v5 DISCONNECT whose reason code is `0x00` — MQTT 3.1.1 carries no reason code, so every v3 DISCONNECT stays a normal disconnection — and `last_will_enable()` now decides from the recorded packet. The same change drops the `DisconnectReceived` conjunction, which was merely redundant on the online path but actively wrong on the `offline_restart` path, where the flag starts empty and is read back from session storage: a session rebuilt after a broker restart could re-publish a Will its connection had already deleted. Deliberately unchanged: the Will Delay Interval still governs when a due Will is published (`0x04` does not bypass it), session takeover still deletes the Will, and the broker still echoes a DISCONNECT to the client. Regression cases: `rmqtt-test` (`functional_v5`) `will_disconnect_reason_v5` (4 cases, 2 of which failed before the change).
- **Message Expiry Discarded an In-Progress QoS 2 Exchange** (issue #513): A subscriber that had answered PUBREC and then disconnected without sending PUBCOMP lost the exchange when its Message Expiry Interval elapsed before the session resumed — `Session::reforward` ran the expiry check *before* the `UnComplete` branch and returned early, so the owed PUBREL was never re-sent with its original Packet Identifier, which [MQTT-4.4.0-1] requires on resume, and no `message_dropped` hook was raised either, leaving the loss silent while the receiver's Packet Identifier stayed reserved forever. The check was wrong on the specification's own terms: [MQTT-4.3.3-7] says the sender *"MUST NOT apply Application Message expiry if a PUBLISH packet has been sent"* and PUBREC proves it was, while [MQTT-3.3.2-5] only permits deleting a copy whose onward delivery has *not* started. The gate is gone and `reforward` always re-sends the PUBREL, which also fixes the same silent drop on the **online** retry path (the session event loop's `deliver_timeout_delay` → `pop_front_timeout`), where a still-connected client waiting for its PUBREL had the exchange discarded under it. `hook.message_expiry_check()` now has exactly one caller left in `rmqtt/src`, `Session::deliver`, the single point at which a message has not been sent yet; keeping a stuck exchange bounded is left to session expiry and `max_inflight`. Applies to all protocol versions, not just v5: a plain v3 session carrying no Message Expiry property falls back to the server-wide `message_expiry_interval`. Regression case: `rmqtt-test` (`functional_v5`) `message_expiry_deletes_qos2_inflight_v5`.
- **Retained Message Expiry Never Decremented** (issue #513): A retained Application Message was delivered with its original Message Expiry Interval however long it had been sitting in the retain store. [MQTT-3.3.2-6] requires the outgoing value to be *"the received value minus the time that the Application Message has been waiting in the Server"*, and here the store is what provides the wait time: `hook.message_expiry_check()` computes `remaining = now - publish.create_time` and subtracts it, but `_send_retain_messages` overwrote `create_time` with "now" on every retained delivery, so `remaining` was always zero and the subtraction was a no-op. A message retained with an interval of 60 s was therefore still announced as 60 s after 20 s of waiting (it should read ≈40 s). The overwrite is removed and the timestamp stamped when the server first received the PUBLISH — the start of the message's lifetime, preserved verbatim by all retain backends — reaches the arithmetic intact. `create_time` is the only clock available on this path: with the default `retained_message_ttl = "0m"` the storage backend holds no expiry deadline of its own (`remaining()` is `None`), and `Retain.from.create_time` is the *session* creation time, shared by every retained message that session published. Two consequences, both intended: the same field is what `rmqtt-web-hook` reports as `pts` on `message_delivered` / `message_acked` / `message_dropped`, so it now matches its documented meaning ("the timestamp when the Publish message was received") instead of the delivery time; and a retained message whose interval has already elapsed is now dropped with `Reason::MessageExpiration` per [MQTT-3.3.2-5] rather than resurrected with a full-lifetime interval — under the default `retained_message_ttl = "0m"` such an entry stays in the store until overwritten, since storage TTL and message expiry are independent axes. Regression case: `rmqtt-test` (`functional_v5`) `retained_message_expiry_not_decremented_v5`.
- **Every Server-Originated DISCONNECT Was Dead Code** (follow-up to issue #513): A v5 Client could not tell "the Server ended my connection, and here is why" apart from "the network died". `Session::run` called `sink.close()` — which shuts the write half down — *before* building and sending the DISCONNECT, so `send_disconnect()` failed on `poll_shutdown` on every teardown and the error was discarded by a `let _ =`: a bare FIN on the wire and no log line for the operator. Since `session.rs` holds the only call site of `send_disconnect` in the workspace, every Server-originated Reason Code was unreachable at once — 0x8D Keep Alive timeout, 0x8E Session taken over, 0x93 Receive Maximum exceeded, 0x95 Packet too large, 0x81/0x82 for malformed packets. `close()` now runs after the DISCONNECT has been written and flushed, and a failed send is logged instead of swallowed. Two further corrections travel with it. The DISCONNECT is now also skipped when the *Client* ended the connection — it sent its own DISCONNECT (`StateFlags::DisconnectReceived`) or closed its transport (`Reason::ConnectRemoteClose`) — because there is then nothing for the Server to announce and possibly nothing left to write to; `Reason::ConnectDisconnect` alone cannot decide that, since the auth plugins reuse it to force a close server-side (auth-http / auth-jwt token expiry) and only the flag separates the two cases. And the takeover path stops reporting the wrong code: `v5.rs` / `v3.rs` kick the existing session with `is_admin = false`, which mapped to 0x87 NotAuthorized, whereas [MQTT-3.1.4-3] requires 0x8E Session taken over — the displaced Client was not rejected, it was displaced. This restores behaviour that `a70360078` (2025-09-24, "Move `sink.close()` earlier in session termination process") had silently removed. Regression case: `rmqtt-test` (`functional_v5`) `takeover_sends_disconnect_0x8e_v5`.
- **Unknown Topic Alias Reported the Wrong Reason Code**: An alias-only PUBLISH whose Topic Alias had never been established on the connection was answered with 0x83 Implementation specific error. MQTT 5.0 defines a Reason Code for exactly that condition — 0x94 Topic Alias invalid — and the broker never produced it: `DisconnectReasonCode::TopicAliasInvalid` existed in the codec but was unreachable, because `ClientTopicAliases::set_and_get` reported the failure as a `MqttError::PublishAckReason` (a *PUBACK* reason code, of which 0x83 is legal and 0x94 is not) which the blanket `MqttError` mapping turned into a DISCONNECT carrying 0x83. A new `MqttError::TopicAliasInvalid(alias)` names the alias in its message and maps to 0x94, so the DISCONNECT the Client receives identifies the actual problem; the alias-limit branch of the same function is deliberately left on 0x83, where the failure really is implementation-specific. The defect stayed invisible until the Server's DISCONNECT packets started reaching the wire — before that the connection was closed without a word and the reproduction's test accepted the silence. Regression case: `rmqtt-test` (`functional_v5`) `topic_alias_v5_unknown_alias`.
- **Topic Alias Above the Advertised Maximum Was Accepted**: A PUBLISH carrying a Topic Alias greater than the Topic Alias Maximum the Server had itself returned in the CONNACK was accepted and delivered rather than refused. [MQTT-3.3.2-9] forbids a Client to send such a PUBLISH, which makes the advertised maximum the Server's own statement of which aliases it honours, and MQTT 5.0 defines 0x94 Topic Alias invalid as the Reason Code for an invalid Topic Alias. `ClientTopicAliases::set_and_get` (`rmqtt/src/types.rs`) capped only how *many* aliases a connection may register, never that an individual alias lies within that maximum, so `Topic Alias Maximum + 1` was stored, the PUBLISH was routed like any other, and a PUBACK proved it. The check is now made on entry, reusing the `MqttError::TopicAliasInvalid` the alias-only branch already carried, so the Client receives a DISCONNECT 0x94 and the connection ends: 0x94 exists in the DISCONNECT code table only (PUBACK has no such value), which is why the connection ends rather than the PUBLISH being nacked. `MqttError::TopicAliasInvalid`'s message no longer claims the alias "has not been established", which was true of the alias-only case alone; it now reads "is invalid on this connection". Two things are deliberately left alone. The older `len >= max_topic_aliases` branch still reports 0x83 Implementation specific error, although it is now unreachable — with every stored alias bounded by the maximum, `max` distinct keys taken from `1..=max` can only be the whole range, so an unregistered in-range alias cannot exist once the map is full. And a Server advertising a maximum of 0 (the default configuration) still ignores incoming aliases instead of refusing them, a separate behaviour change that belongs in its own round. Regression case: `rmqtt-test` (`functional_v5`) `topic_alias_v5_over_max`, registered as a bare failure while the check was missing and now passing.
- **`client_connack` Hook Never Saw a Refused Connection**: A refusal answered from the handshake path was invisible to plugins, and its reason code could not be changed. `refused_ack_v3` / `refused_ack` (`rmqtt/src/v3.rs` / `v5.rs`) raise the `ClientConnack` hook only when they are handed the client information, and their single call site passed `connect_info = None`, so that branch was dead code: the refusal was reported by a `log::info!` line whose identifier field rendered as `None`, consumers that key off the event covered successful connections only, and `rmqtt-counter`'s `client_connack_error` / `*_auth_error` / `*_unavailable_error` branches were unreachable in production. The handshake now wraps the client information in the very `Arc<ConnectInfo>` the session stores as soon as the CONNECT packet is decoded, and the error type carries it — `(ConnectAckReason, Error, Option<Arc<ConnectInfo>>)`, a refcount bump with no copy of the decoded CONNECT on any failure exit, and `None` for failures that happen before the CONNECT packet is decoded (an undecodable packet, an overloaded node, a handshake task that timed out) — so every refusal of a decoded client raises the hook with the real reason code, and a v5 handler can answer `0x8A` Banned or `0x9F` Connection rate exceeded where the protocol previously forced `0x05`. A refusal stays a refusal: a handler answering with a successful reason code is ignored, because no session was created and a CONNACK 0x00 followed by a close would tell the client it had connected. The success path is deliberately unchanged — `handshake` still sends `ConnectionAccepted` unconditionally, so the hook's return value remains unused there (known gap, left deliberately). Regression case: `rmqtt-test` (`functional_v311`) `webhook_connack_refused_v311`, which asserts both halves — the anonymous CONNECT is refused with 0x05 **and** a `client_connack` event carrying that reason reaches the mock web-hook receiver; before the change it failed with "the CONNECT was refused with CONNACK 0x05 (as expected) but no client_connack web-hook event arrived within 10s".

### Test Improvements

- **Harness Test Filter**: `mqtt_harness` gained `-t/--test <NAME>`, a case-name substring filter applied after `split_suites_by_config`, so a single case — or a handful — can be re-run without paying for the whole suite; sub-suites left empty by the filter are discarded. Documented in `docs/en_US/development/testing.md`, `docs/zh_CN/development/testing.md` and the `rmqtt-test` READMEs, together with the new Will Message reason-code cases.
- **Cluster QoS 2 Reproduction Case No Longer Needs Manually Started Nodes**: `qos2_pubrel_resume_collision_cluster` (`functional_v5_cluster`) assumed two manually started rmqttd nodes on 1884/1885 but reached node 1 through `ctx.config.broker_addr`, so running it the ordinary way — harness-managed broker on 1883, node 2 never started — spent phase 1 against an unrelated single-node broker and then died on the first node 2 connection with a bare `os error 10061` (WSAECONNREFUSED), indistinguishable from a broker defect. The case now owns its cluster: both addresses are hard-coded, a node that already accepts TCP connections is **reused** (the original two-terminal flow), a missing one is **spawned** from `configs/pubrel-collision-cluster/node{N}/` and killed on exit, and the cross-node probe names both addresses when they do not form a converged cluster. Running without `--no-broker` reports SKIPPED with the exact command instead of failing, because a harness-managed broker binds the default listeners node 1 also binds (1883/11883/8883/8080/8443/9443) and would silently stand in for node 1; node 1's config now disables those listeners, as node 2's already did. The verdict's suite tag was corrected from `functional_v5` to `functional_v5_cluster`.
- **Topic Alias Negative Cases No Longer Pass Vacuously**: `topic_alias_v5_zero` and `topic_alias_v5_over_max` published at QoS 0 — where the protocol expects no answer at all — and treated a read timeout as "the connection was closed", so both passed whether the broker refused the illegal alias or quietly delivered it (`over_max` measured the full 5 s read timeout, i.e. it measured nothing). They now publish at QoS 1, which makes the PUBACK the single answer that proves acceptance and a timeout with the connection still open a failure. `topic_alias_v5_zero` keeps passing — the broker does refuse alias 0, promptly. `topic_alias_v5_over_max` turned red and **found a new gap**: `ClientTopicAliases::set_and_get` capped how *many* aliases a connection may register but never checked an individual alias against the maximum the Server advertised in its own CONNACK, so `Topic Alias Maximum + 1` was stored, accepted and delivered like any other PUBLISH, with a PUBACK to prove it. It was kept as a plain failure rather than an expected-fail, so it stayed visible until the check was added; the check is now in place and the case passes (see the Topic Alias entry under Bug Fixes).
- **Web-Hook Coverage for a Refused Connection**: `webhook_connack_refused_v311` (`functional_v311`) pins the refusal side of the `ClientConnack` hook end to end, which no case covered before. It starts its own broker from the new `configs/webhook-connack-refused/` (MQTT 1901 / gRPC 5378, `allow_anonymous = false`, `rmqtt-acl` disabled so the undecided authentication is not promoted to an ALLOW, and only `rmqtt-web-hook` started), sends an anonymous CONNECT, and requires both halves of the contract: the client is refused with CONNACK 0x05 **and** a `client_connack` event carrying the reason `Connection Refused, not authorized` arrives at an in-test mock HTTP receiver bound to an ephemeral port. A readiness probe proves the receiver is serving before the broker's first CONNECT, so a transport failure can never be mistaken for a missing event. The case was red before the fix with the CONNACK assertion passing and the event assertion timing out — the exact signature of the dead hook branch.

### Configuration Changes

- `rmqtt.toml`: removed the dead `listener.workers` key.
- Added `rmqtt-plugins/rmqtt-counter.toml`, the default configuration file for the `rmqtt-counter` plugin, which was missing from the repository.

### Dependency Upgrades

- `rmqtt-net` 0.5.0 → **0.6.0** (new `is_over_max_packet_size()` helper, re-exported at the crate root; new `MqttError::TopicAliasInvalid` variant)
- `rmqtt-conf` 0.5.0 → **0.6.0** (follows `rmqtt-net`)

## [0.24.0] - 2026-09-19

### Security

- **Constant-Time Bearer Token Comparison** (CWE-208 hardening): `rmqtt-http-api` now verifies the `Authorization: Bearer` header against `http_bearer_token` by comparing fixed-length SHA-256 digests with `subtle::ConstantTimeEq`, instead of a plain `==` on the raw values. This removes both timing side channels reported in a private disclosure — the byte-by-byte early-exit of the equality check and the length short-circuit — so neither the token content nor its length can be inferred from response timing. Behavior is unchanged otherwise (same exact `Bearer ` prefix semantics, same 401 handling, hoop only mounted when a token is configured). Unit tests added for correct/wrong/missing/empty/prefix/truncated/superset/no-scheme header values and SHA-256 known vectors. New dependencies: `sha2 0.10`, `subtle 2.6` (both already present in the dependency tree). Design notes: `designs/security-http-api-bearer-ct-compare.md`. Per-client-IP rate limiting on the HTTP API is tracked separately as defense-in-depth follow-up.

### New Features

- **`rmqtt-delayed` Plugin — Delayed Publishing Extracted from Core**: The delayed-publish engine moved out of the broker crate into a standalone `rmqtt-delayed` plugin, driven by `MemDelayedSender` — a `BinaryHeap` scheduler with periodic expiry forwarding, a per-node `publish_max` cap (default `100_000`), configurable overflow behaviour (forward immediately, or drop and fire the `message_dropped` hook with `DelayedPublishRefused`), flush-on-unload, and `list` / `find` queries that match a topic filter against the target topic with the `$delayed/<interval>/` prefix stripped. Core keeps only the `DelayedSender` extension trait plus a no-op `DefaultDelayedSender`; the real sender is injected through `extends.delayed_sender_mut()` when the plugin starts, and `session.rs` keys its enable check on `delayed_sender().enable()` instead of a listener-level flag. The plugin is **off by default** — present but commented out in `plugins.default_startups`. New HTTP API endpoints `GET /api/v1/delayed_publishs` (cluster-wide, merged and sorted by trigger time, offset/limit pagination, metadata only) and `GET /api/v1/delayed_publishs/detail` (single entry with the full payload, keyed by topic + expired_time + client_id), backed by the new cross-node gRPC messages `DelayedPublishsQuery` / `DelayedPublishsGet`. The dashboard gained a "Delayed Publish" page with navigation, styles and i18n strings for all 12 locales.
- **Offline Session Routing Across Broker Restart** (issue #475): Persistent sessions restored from `rmqtt-session-storage` are now routable and loss-free. Two defects were behind the message loss: `rebuild_offline_sessions` restored sessions into the router's peers but never registered their subscriptions, so between a broker restart and the client's reconnect any matching publish was PUBACKed and silently dropped while the reconnect still saw `session_present = 1` ([MQTT-3.2.2-2]); and `offline_run_loop` acknowledged a session Kick and returned immediately, dropping the `Message::Forwards` still queued in its receiver — those messages are now drained into the deliver queue (with the same `offline_message` hook as the main loop) before the kick is acknowledged. Subscriptions are registered immediately after the session entry is stored, so restored sessions are routable at once.
- **Bounded Offline-Message Persistence** (#495): Offline-message persistence was dispatched with a raw `tokio::spawn` — one detached task per routed message per offline session, each owning a cloned payload — so a workload creating tasks faster than the storage backend drained them accumulated futures on the heap without limit and OOM-killed the broker (measured: 93 → 231 372 live tasks in 21 s, OOM at a 1 GiB limit after 48 s; sled and redis behaved identically, and `max_mqueue_len` was irrelevant because `push_limit` bounds what is stored, not the backlog waiting to store it). Writes now go through a bounded `TaskExecQueue` from the server context, as the plugin already does for `SESSION_REBUILD_EXEC`; when the queue is full the task is discarded and counted instead of spawned, which matches the existing `push_limit` behaviour (awaiting the write inline would instead stall the routing path for connected clients too). On the same workload live tasks stay at 83–84 and anonymous memory plateaus at ~230 MiB. Contributed by @SylteA.

### Bug Fixes

- **MQTT 3.1.1 Decode Conformance**: three gaps where malformed packets were accepted instead of closing the connection are now rejected — PUBLISH with an empty Topic Name ([MQTT-4.7.3-1]), SUBSCRIBE/UNSUBSCRIBE with no topic filters or an empty-string filter ([MQTT-3.8.3-1] / [MQTT-3.10.3-1] / [MQTT-4.7.3-1]), and CONNECT with Will Flag = 0 while Will QoS or Will Retain is set ([MQTT-3.1.2-11/13]). New `DecodeError` variants `InvalidTopicName`, `InvalidTopicFilter` and `InvalidConnectFlags` map to v5 `TopicNameInvalid` (`0x90`), `TopicFilterInvalid` (`0x8F`) and Protocol Error respectively.
- **SUBSCRIBE/UNSUBSCRIBE With an Empty Payload** (v5): the v5 codec silently accepted packets with zero topic filters and the broker replied with a SUBACK carrying zero return codes — a double violation of [MQTT-3.8.3-3] / [MQTT-3.10.3-2] (an empty payload is a Protocol Error) and [MQTT-3.8.4] (a SUBACK payload MUST contain at least one return code). An `is_empty()` guard restores v3/v5 parity; rejection maps to DISCONNECT `0x8F`. Regression tests: `protocol_error_v5_subscribe_empty_payload` / `protocol_error_v5_unsubscribe_empty_payload`.
- **v5 Subscription Options Reserved Bits**: the reserved bits (6–7) of the Subscription Options byte are now validated during decode per [MQTT-3.8.3-4]; a non-zero value is a Malformed Packet instead of being ignored.
- **v5 AUTH Packets Rejected**: RMQTT does not support MQTT 5.0 enhanced authentication, so — since no authentication method can have been negotiated in the CONNACK — an AUTH packet from a client is now a Protocol Error per [MQTT-4.12.0], answered with DISCONNECT `0x82`, instead of being processed.
- **Per-Filter SUBACK Failure** (v5): a malformed topic filter in a SUBSCRIBE (for example a malformed shared-subscription filter) made the whole subscribe fail and dropped the connection; per MQTT 5.0 section 3.8.4, per-filter errors must be reported as individual SUBACK reason codes, so parse errors now push `SubscribeAckReason::TopicFilterInvalid` into the SUBACK and processing continues with the remaining filters. The v3.1.1 path deliberately keeps its disconnect semantics, because legacy devices often ignore a refused SUBACK and hang forever; disconnecting forces them to re-subscribe after reconnect.
- **Disconnect Reasons Drained with `mem::take`**: draining disconnect reason codes into a new `Vec` now uses `std::mem::take`, swapping in an empty `Vec` in O(1) and retaining the original capacity, avoiding an extra allocation and element copies.
- **Dashboard History Chart Spikes**: the cluster overview line charts (in/out messages, connections, topics, subscriptions) showed huge spikes alternating with zeros — and a Y axis blown up to 1.5 M — whenever the browser tab polled from the background. Root cause chain: each node truncated its own history to `limit`, the node timestamps were then unioned and summed, so the union always carried an edge bucket contributed by only some nodes; the frontend popped only the newest bucket, leaving an oldest partial-sum bucket in the series; and `toRate()` differentiated the cumulative counters, turning partial→full into a ~2.1 M spike and full→partial into a negative value that `Math.max(0, ...)` clamped to 0 — that comb pattern. The only correction window was `5 × merge_window` (25 s at the 15 m range), which a 5 s foreground poll could hit but a background poll throttled to ≥30–60 s could not, so partial sums were baked in permanently. The backend now fetches `limit + 2` buckets per node (local and remote) and drops every timestamp not contributed by *all* nodes (each node reports at most one point per timestamp, so the point count equals the contributing node count), skipping the completeness requirement when a node returned an empty series for the window — e.g. it restarted inside the queried range — and truncating globally newest-first. The frontend sizes its incremental query from the real polling gap (bounded by the selected range, capped at 2000 points), refreshes on `visibilitychange`, and re-lays out the ECharts canvases on window resize. Measured on the same polling data (24 rounds, 21 of which contained partial-sum buckets under the old rules), the worst-case background delta fell from 12 260 735 to 11 074 (30 s cadence) and from 12 244 125 to 29 478 (60 s cadence), with zero spikes in both. 4 unit tests cover the aggregation rules. The dashboard JavaScript comments were also translated to English per the workspace convention (code comments English, Markdown docs Chinese; 22 files, comments only, no behavior change).

### Refactoring

- **Delayed Publishing Removed from Core**: `rmqtt/src/delayed.rs` is reduced to the `DelayedSender` extension trait plus a no-op placeholder; `ServerContext` loses `mqtt_delayed_publish_max` / `mqtt_delayed_publish_immediate`, and `rmqtt-net::Builder::delayed_publish()` and the `ListenerInner::delayed_publish` option are gone. `delay_publish` now returns `Ok(None)` when the injected sender handled the message — scheduled, or refused and dropped with the `message_dropped` hook fired by the sender itself.
- **Session Storage Startup Load**: a load-time pre-check on `last_time + session_expiry_interval` drops sessions the rebuild pass would remove anyway, cutting the serial scan (5 sled gets per session) from O(all sessions) to O(active sessions) and keeping startup inside the broker health-check window under stress. Also fixed `disconnected_set` being invoked twice during shutdown: the second call (with `None`) reset the storage TTL back to the CONNECT value, clobbering a DISCONNECT-extended session expiry — the incoming DISCONNECT property is now preferred, falling back to the stored one.
- **`HistoryKind` Instead of Pre-Encoded gRPC Payloads**: the history query path passes a `HistoryKind` enum so the requester knows which gRPC message to encode, removing four copies of `Message::...encode()` and a shadowed local node id.

### Test Improvements

- **MQTT 3.1.1 Conformance Coverage**: implemented the P0–P3 gap-fill plan (G1–G33) from `designs/mqtt-311-standalone-test-gap-analysis.md`, covering invalid UTF-8 in CONNECT fields and PUBLISH topics, remaining-length var-int boundaries, CONNECT flag consistency, QoS negotiation, PUBREL/PUBREC/PUBCOMP fixed-header flags, truncated packets and declared-length mismatches, reserved packet type `0x0F`, CONNACK return codes (with `@auth-denied` / `@auth-jwt-denied` sub-suites), session takeover, empty topic levels, overlapping wildcards, retained-message recovery and transport suites (TLS / WSS / WS / mTLS). Added a `functional_transport` suite and harness configs `auth-denied`, `auth-jwt-denied`, `retain-sled`, `rl-boundary`, `transport-tls`.
- **v5 Gap-Analysis Coverage**: 16 cases (G13–G27) from `designs/mqtt-5.0-standalone-test-gap-analysis.md` — QoS downgrade matrix, PUBACK/UNSUBACK legal reason values, Will properties forwarding, Content Type / Correlation Data passthrough, multi-filter SUBSCRIBE at mixed QoS, Request Problem Information / Request Response Information, publication expiry on queued vs forwarded delivery, QoS 1 ordering, session expiry on reconnect, malformed shared-subscription filters, subscription identifier updates, and broker→client flow control at Receive Maximum = 1. The v5 client gained an `auto_puback` toggle, PUBACK/PUBREC reason-code channels, UNSUBACK waiters with fail-fast on connection close, and `subscription_ids` / `packet_id` on incoming messages.
- **Offline Session Routing Suites** (issue #475): single-node `chaos_broker_restart_session_routing`, cluster reproductions `cluster_restart_session_routing_{broadcast,raft}` and `cluster_whole_restart_session_routing_{broadcast,raft}` with self-managed nodes, plus a stress suite of 5 tests × 1000 sessions × 100 QoS 1 messages across broadcast/raft with single-node and whole-cluster restarts. Self-contained sled configs (`session-sled(-stress)`, `cluster-*-sled(-stress)`) isolate the stress data from the reproduction tests, and the harness broker health-check timeout was raised from 10 s to 60 s because sled startup can be slow once the store has grown large.
- **Delayed Publish Suite**: 18 cases covering basic delivery/routing/QoS/ordering, subscription timing, properties passthrough, error paths, retain semantics, `publish_max` overflow under the dedicated `delayed-max1` / `delayed-max1-drop` configs, plugin-unload flush, and a 2-node cross-node cluster case. The suite also installs the `rustls` `aws-lc-rs` `CryptoProvider` at startup, fixing a panic when feature unification enables both `aws-lc-rs` and `ring`.
- **Chaos Bounded-Memory Repro** (#495): a replayed workload (8 publishers × 50 msg/s QoS 1, 256 B, ~97 cycling client ids, ~49 offline sessions) that OOM-killed the broker before the fix. Chaos broker-restart tests now SKIP in `--no-broker` mode instead of failing, and the stress drain wait is bounded by an HTTP timeout with plateau fail-fast.
- **Pulsar Bridge End-to-End Suite**: an end-to-end egress/ingress suite for the Pulsar bridge plugins.
- **Auth-Chain Repro** (#501): `functional_v311` reproduces the `auth`-plugin `ignore` × `rmqtt-acl` fall-through interaction.
- **Harness Robustness**: added a `port_free_sync()` pre-flight check so broker startup fails fast with a clear message when the MQTT port is already occupied instead of burning the full health-check timeout, replaced the fixed 500 ms restart sleep with `wait_port_free_sync()` (up to 5 s) to close the `EADDRINUSE` restart race, made `ClusterNode::wait_healthy` detect early broker exit via `try_wait()` and dump the log tail, and fixed packet wire order in the test clients (PUBLISH QoS > 0 is topic, packet id, then properties — the previous order was malformed and let cases pass or fail vacuously), plus `-f` handling, absolute `plugins.dir` rewriting and mock auth-server form parsing in the self-managed auth broker cases.

### Dependency Upgrades

- `rmqtt-net` 0.4.0 → **0.5.0** (drops `Builder::delayed_publish()`)
- `rmqtt-conf` 0.4.0 → **0.5.0** (drops `mqtt.delayed_publish_max` / `mqtt.delayed_publish_immediate` and `listener.<proto>.<name>.delayed_publish`)
- `rmqtt-codec` 0.3.0 → **0.3.1** (empty topic name / topic filter validation, new `DecodeError` variants)
- Pulsar client pinned to **`=6.9.0`** in both the ingress and egress bridge plugins (previously declared `6.4.1`, resolving to 6.8.0); the egress serializer sets the new `partition_key_b64_encoded: Option<bool>` metadata field to `false` when a key is present, matching upstream's `utf8_partition_key()`
- Docker base image reverted to `alpine:3.18.12` (amd64) / `arm64v8/alpine:3.18.12` (aarch64)
- Minimum supported Rust version (MSRV) raised from **1.89.0** to **1.94.0**

### Configuration Changes

- Delayed publish settings moved out of `rmqtt.toml` into the `rmqtt-delayed` plugin: `mqtt.delayed_publish_max` and `mqtt.delayed_publish_immediate` are now `publish_max` and `publish_immediate` in `rmqtt-delayed.toml`, and the per-listener `listener.<proto>.<name>.delayed_publish` switch is gone entirely — the plugin's own enable flag decides. `#"rmqtt-delayed"` is listed but commented out in `plugins.default_startups`.
- `rmqtt.toml`: added a warning next to `plugins.default_startups` explaining that the built-in `rmqtt-acl` `["allow", "all"]` rule resolves to *all* operations including CONNECT, so connections a custom auth plugin decided to `ignore` are explicitly allowed even with `allow_anonymous = false`; enabling a custom auth plugin should switch that rule to `["deny", "all"]` for fail-closed behaviour.

## [0.23.0] - 2026-08-09

### New Features

- **Unified Circuit Breaker**: Integrated a sliding-window circuit breaker (`CircuitBreakerConfig`) across all storage plugins — `rmqtt-retainer`, `rmqtt-message-storage`, and `rmqtt-session-storage`. When the storage backend failure rate exceeds the threshold, the circuit opens and all operations fast-fail, preventing cascading failures. Configurable via `circuit_breaker.*` settings in each plugin's TOML.
- **gRPC Client Circuit Breaker**: Added per-peer-node circuit breaker to `GrpcClient`, covering `send_message`, `quick_send_message`, `notify`, and `quick_notify`. Configurable via `node_grpc_circuit_breaker_enabled`, `node_grpc_circuit_failure_threshold`, `node_grpc_circuit_reset_timeout`, and `node_grpc_circuit_half_open_success_threshold` in cluster plugin configs.
- **Retainer Cluster Synchronization**: Implemented cluster-wide retain message synchronization with two modes — `Full` (broadcast full payload, used by ram/sled) and `TopicOnly` (broadcast topic name only, used by redis). New `RetainStorage` trait methods: `retain_sync_mode()` and `sync_retain_topic()`.
- **gRPC Quick Path**: Added `quick_send_message` / `quick_notify` fast paths that bypass request-queue-full checks for higher-priority operations. All HTTP API cross-node gRPC calls now use the quick path.
- **Retainer In-Memory Topic Trie**: Built a `RetainTree` index in memory on startup for O(1) exact-topic lookups and fast wildcard matching, replacing the previous SCAN+MATCH approach.
- **Retainer Batch Storage**: Messages are collected into a channel and processed in batches via `batch_insert` / `batch_remove`, controlled by `batch_messages_limit`.
- **Rate Counter**: Added `rmqtt-utils::RateCounter` (lock-free `AtomicU64`) for tracking message processing throughput. Enabled by default via the `rate-counter` feature.
- **Message Storage Timeout**: Added configurable `backend_timeout` for storage I/O operations, channel sends, and circuit breaker per-operation timeout in `rmqtt-message-storage`. Unified the previous separate `timeout` and `backend_timeout` fields into a single `backend_timeout` (default: `"15s"`).
- **Cluster Broadcast Exec Queues**: Added `exec` and `forwards_exec` `TaskExecQueue` to `rmqtt-cluster-broadcast` for queue back-pressure management and busyness detection, aligning with `rmqtt-cluster-raft`.
- **Retainer Enabled by Default**: `rmqtt-retainer` is now included in `plugins.default_startups` by default.
- **HTTP API Feature Support Query**: Added `GET /api/v1/features` and `GET /api/v1/features/{id}` to `rmqtt-http-api`. They report the support state of six features (`retain`, `message_storage`, `session_storage`, `delayed`, `shared_subscription`, `auto_subscription`) per node. The cluster-wide response includes a consistency summary (`consistent` / `conflicts` / `nodes`) and emits a `features inconsistent across cluster` warning log when nodes disagree.
- **HTTP API Retained Messages Query**: Added `GET /api/v1/retains` to `rmqtt-http-api` to query retained messages with `topic_filter` / `offset` / `limit` parameters. The full pagination path (`topic_filter=#`) is served from the storage layer with `remaining_ttl`; filtered queries paginate in memory. Payload is base64-encoded.
- **Dashboard Enhancements**: Added a retained messages page (`#/retains`) with pagination and payload preview/detail dialog, a dedicated "Feature Support" tab with a per-node feature matrix and cluster consistency alert, dual-tab (abnormal / non-subscriber) switching on the message-drop trend panel, a client detail page, and an i18n-aware custom datetime picker. The Dashboard SPA can be served from an external directory via `dashboard_static_dir` for hot-swapping without recompiling.
- **TCP Keepalive on Accepted Connections** (issue #465): Accepted MQTT/TCP connections now enable `SO_KEEPALIVE` by default, so the kernel's `net.ipv4.tcp_keepalive_*` settings (Linux) / registry values (Windows) take effect and dead peers behind cellular/CGNAT NAT black holes are probed and reclaimed (previously the option was never set and connections piled up as ESTABLISHED/FIN-WAIT-1). Configurable via `listener.<proto>.<name>.tcp_keepalive` — `false` (disabled) / `true` (default, enabled with OS probe defaults). The socket option is set with a plain `setsockopt(SO_KEEPALIVE)` (`socket2::SockRef`), avoiding the blocking `WSAIoctl(SIO_KEEPALIVE_VALS)` path on Windows that stalled the tokio worker threads under high connection concurrency. Regression tests added in `rmqtt-test` (`functional_v5`).
- **Stats/Metrics History Persistence**: Added a complete history subsystem to `rmqtt-http-api` — Stats and Metrics are snapshotted periodically (default `flush_interval = "5s"`), cached in an in-memory LRU, and asynchronously persisted to a configurable backend (`storage.type`: `redb` / `sled` / `redis` / `redis-cluster`) with TTL-based expiration (`history_retention`, default `"7d"`). Expired entries are discarded during warmup and removed from storage. New endpoints `GET /api/v1/stats/history` and `GET /api/v1/metrics/history` support cross-node cluster-wide queries via gRPC, with results merged by timestamp. A 30s recovery loop retries failed entries and a global `UNPERSISTED_COUNT` tracks pending writes.
- **Dashboard Embedded via rust-embed**: The `rmqtt-dashboard` SPA is now compiled into the `rmqtt-http-api` binary at build time (`rust-embed`), so the dashboard is served at `http://127.0.0.1:6060/` with zero configuration and no filesystem dependency. The optional `dashboard_static_dir` setting still allows a filesystem override for development hot-reloading.
- **Dashboard Retained Message Deletion**: Added the ability to delete individual retained messages from the retained messages page (`#/retains`), backed by the new HTTP API endpoint `DELETE /api/v1/retains?topic={topic}` (rejects wildcard topics, returns 404 when no retained message exists, uses MQTT empty-payload retained publish semantics, and propagates the deletion to all cluster peers via `retain_set_broadcast`). The dashboard shows a confirmation dialog with optimistic UI updates.

### Bug Fixes

- **QoS 2 Exactly-Once on Replayed PUBLISH** (issue #456): A replayed QoS 2 PUBLISH (same Packet Identifier, `DUP=1`, before the PUBREL exchange completes) is now answered with PUBREC and **no longer delivered to the subscriber a second time** (`[MQTT-4.3.3-10]`). Implemented via an `InInflight::exist` check on the inbound inflight set, plus a new `client_publish_duplicate` metrics counter.
- **Inflight Packet-ID Space Isolation on Session Resume**: Fixed a QoS 2 session-resume bug where transferred inflight messages kept their old packet-ids (1..N) while the new session's `OutInflight` allocator restarted at 1, so a concurrently delivered stored message could be assigned the same id and silently overwritten by `push_back` (`HashMap::insert`), permanently destroying its QoS 2 state (no resend, no ack hook, possible loss). The allocator is now advanced past the transferred id range via a new `OutInflight::advance_next_id()` before any concurrent delivery path can allocate, and `send_rerelease` gained a defensive existence check.
- **Session Resume Reforwards All Inflight Messages** (issue #456): On session transfer/resume, every inflight message is now reforwarded regardless of status — `UnComplete` messages were previously skipped, so an owed PUBREL was lost. A re-sent PUBREL now always acknowledges Success instead of the previous `PacketIdNotFound` choice (`[MQTT-4.4.0-1]`).
- **CONNACK Reason Code for Empty ClientId**: A CONNECT with a zero-length ClientId while CleanStart (v5) / CleanSession (v3.1.1) is 0 is now rejected with the spec-mandated reason codes — v5 `0x85` Client Identifier not valid (`[MQTT-3.1.3-8]`) and v3.1.1 `0x02` Identifier Rejected (`[MQTT-3.1.3-6]`) — instead of the previous generic 0x88 / 0x03.
- **Will Retain Rejected When Retain Unavailable** (issue #457): A CONNECT whose Will Message has Will Retain = 1 is now rejected with CONNACK reason `0x9A` (Retain not supported) when the server advertises Retain Available = 0 (`[MQTT-3.2.2-13]`); previously such connections were accepted with reason 0x00.
- **Cluster Sync Handles MessageReply::Success**: `MessageReply::Success` responses received during cluster retain synchronization and message loading were treated as errors and logged at `warn!` level. Both `rmqtt-cluster-broadcast` and `rmqtt-cluster-raft` now handle them explicitly (`debug!` level, empty result, end-of-sync), reducing production log noise.

### Refactoring

- **Circuit Breaker Simplification**: Simplified `CircuitBreaker` in `rmqtt-utils` — consolidated configuration into `CircuitBreakerConfig` with sliding-window semantics (failure rate, slow call detection, per-operation timeout).
- **Message Storage Refactor**: Removed `merge_on_read` and `TaskExecQueue` from `rmqtt-message-storage`; added `with_timeout` wrapper for storage operations; introduced async callback support and back-pressure limits. Unified `timeout` and `backend_timeout` into a single `backend_timeout` field.
- **Session Storage Improvements**: Added detailed timing metrics for rebuild operations; improved init timing and timeout handling.
- **Error Logging Normalization**: Normalized error logging across the workspace to use `Display` instead of `Debug` formatting.
- **Cluster Retain Exec Queue**: Added a dedicated `retainer_exec` `TaskExecQueue` in `rmqtt-cluster-raft` for retain operations, separating from the main `exec` queue.
- **Unified Server Error Handling**: `rmqtt-bin` startup logic was split into `main()` + `run()`; listener bind failures now propagate through the `anyhow` error chain with the listener address included, and are logged once in `main()` before exiting (previously logged redundantly without address context). Also filled in the empty error messages on `Listener::accept()` / `accept_quic()` in `rmqtt-net`.

### Test Improvements

- **MQTT Spec-Conformance Coverage**: Expanded `rmqtt-test` to systematically cover the MQTT 3.1, 3.1.1 and 5.0 specifications with positive, negative and boundary cases — the three functional suites grew from ~100 to **174 cases**, all passing (v3: 47, v3.1.1: 64, v5: 62 + 1 intentional skip). New modules cover protocol errors (SUBSCRIBE QoS 3, reserved flag bits, second CONNECT), keepalive, last will, QoS 2 conformance (`qos2_conformance_v3/v311/v5`), retain edge cases, wildcards, and CONNACK capability advertisement.
- **Per-Case Broker Config Switching**: Test cases can now declare their required broker config via `TestCase::broker_config()` and are split at suite-build time into `{suite}@{config}` sub-suites (e.g. `functional_v5@retain-disabled`, `functional_v5@tcp-keepalive`, `functional_v5@pubrel-collision`); the scheduler restarts the broker to switch configs only at suite boundaries. All test broker configs are self-contained under `rmqtt-test/configs/`, and `rmqttd` is always started with an explicit `-f` config.
- **QoS 2 Regression Suites**: Added single-node `qos2_pubrel_resume_collision` (functional_v5) and a new cluster end-to-end `functional_v5_cluster` suite (`qos2_pubrel_resume_collision_cluster`, two manually started nodes) — the cluster suite reproduced the bug 3/3 rounds before the fix and passes 3/3 after. Chaos broker-restart tests now SKIP in `--no-broker` mode instead of failing.
- **Test Harness Fixes**: Fixed a broker child-process leak on failure exit (the managed broker is now killed before `std::process::exit`), fixed `clippy::type_complexity` in suite splitting, and added `TestResult::note` / `TestContext::guard_retain_required` so retain-dependent tests skip with a note when the `rmqtt-retainer` plugin is not loaded.

### Dependency Upgrades

- `rmqtt-net` 0.3.5 → **0.4.0** (`Builder::tcp_keepalive()`)
- `rmqtt-conf` 0.3.5 → **0.4.0** (new `tcp_keepalive` listener option, `bool`)
- `rmqtt-storage` 0.10.2 → **0.11.1** (history storage backend for `rmqtt-http-api`)
- Docker base images: Alpine 3.22.4 → **3.24.1** (amd64) / arm64v8/alpine 3.22.4 → **3.24.1** (aarch64)

### Configuration Changes

- `rmqtt-retainer.toml`: Circuit breaker config changed from flat fields (`circuit_breaker_enabled`, `circuit_failure_threshold`, `circuit_reset_timeout`, `circuit_half_open_success_threshold`) to nested `circuit_breaker.*` sliding-window format. `retained_message_ttl` and `batch_messages_limit` are now uncommented by default.
- `rmqtt-message-storage.toml`: `storage.ram.encode` default changed from `true` to `false`. Added `circuit_breaker.*` section. Unified `timeout` and `backend_timeout` into a single `backend_timeout` field (default: `"15s"`).
- `rmqtt-session-storage.toml`: Added `circuit_breaker.*` section.
- `rmqtt-cluster-broadcast.toml` / `rmqtt-cluster-raft.toml`: Added gRPC circuit breaker settings (`node_grpc_circuit_breaker_enabled`, etc.). `rmqtt-cluster-raft.toml`: `node_grpc_client_timeout` default changed from `"60s"` to `"10s"`. `raft.snapshot_interval` default changed from `"600s"` to `"300s"`.
- `rmqtt-http-api.toml`: Added optional `[storage]` section for Stats/Metrics history persistence (`storage.type` = `redb` / `sled` / `redis` / `redis-cluster`), plus `flush_interval` (default `"5s"`) and `history_retention` (default `"7d"`). The `dashboard_static_dir` setting is now commented out by default — the dashboard is embedded in the binary via rust-embed; uncomment to serve from a filesystem directory for development.
- `rmqtt-retainer.toml`: Default storage type switched from `ram` to `sled`.
- `rmqtt.toml`: `rmqtt-retainer` is included in `plugins.default_startups` by default.

---

## [0.22.0] - 2026-05

### Major Changes

- **Serialization migration**: Migrated from `bincode` to `postcard` across the entire workspace for improved performance and reduced binary size. **Note**: Raft log state must be cleared when upgrading from 0.21.x due to format change.
- **CLI framework migration**: Migrated from `structopt` to `clap v4` for modern argument parsing with better error messages and auto-completion support.
- **Logging ecosystem migration**: Replaced `slog` with the `tracing` ecosystem (`tracing-subscriber`, `tracing-appender`) for structured, async-aware logging with file rotation and env-filter support.
- **Feature flag cleanup**: Removed unused `bridge-ingress-nats` re-export from `rmqtt-plugins` lib.rs (feature still exists in Cargo.toml).

### New Features

- **Bridge Origin plugin**: Added `rmqtt-bridge-origin` plugin to identify bridge client connections by client_id markers. Stores origin in `session.extra_attrs` for anti-loop and routing decisions.
- **TLS Certificate Subject DN as Username**: Added `cert_subject_dn_as_username` listener option alongside existing `cert_cn_as_username`. Useful when multiple CAs are trusted on the same listener.
- **Certificate Info Collection**: Added `collect_cert_info` listener option to conditionally extract TLS certificate metadata.
- **Client Certificate Authentication**: Added `tls_client_ca_certs` and `tls_cross_certificate` options for mutual TLS authentication.
- **Offline Message Webhook**: Added `offline_message` event support to the webhook plugin.
- **Client-level ACL Management**: Added per-client ACL rule management in `rmqtt-acl` plugin.
- **Advanced MQTT v5 Tests**: Comprehensive v5 feature tests including topic aliases, subscription identifiers, request/response, flow control.

### Dependency Upgrades

| Dependency | Old | New | Scope |
|-----------|-----|-----|-------|
| `tokio` | 1.40 | 1.52 | Workspace |
| `reqwest` | 0.12 | 0.13 | Workspace |
| `prometheus` | 0.13 | 0.14 | rmqtt-core |
| `rdkafka` | ~0.36 | 0.38 | Bridge Kafka |
| `rdkafka-sys` | — | pinned | Bridge Kafka |
| `salvo` | 0.76 | 0.90 | HTTP API |
| `async-nats` | 0.38 | 0.49 | Bridge NATS |
| `clap` | 3.x (structopt) | 4.x | CLI |
| `postcard` | — | added | Workspace (replaces bincode) |
| `tracing` | — | added | Workspace (replaces slog) |

### Other Changes

- Bumped `rmqtt-conf` to 0.3.5
- Bumped `rmqtt-macros` to 0.1.2
- Bumped `rmqtt-net` to 0.3.5 (removed linger setting)
- Upgraded Alpine base images to latest stable for Docker builds
- Optimized Docker build context (from 11.4GB to 119MB)
- Added GitHub CI workflow for Linux builds
- HTTP API: added startup synchronization and improved reload handling
- Improved HTTP API hot-reload: old server shuts down after new one starts

### Documentation

- Added comprehensive module-level doc comments across all crates
- Added/improved doc comments for all `.rs` files across workspace
- Updated CLI usage examples for rmqtt-test
- Created bilingual README files for all sub-crates and plugins
- Added bridge-origin documentation (`.toml` config and usage docs)

### Test Improvements

- Added comprehensive MQTT v5 feature tests and enhanced v5 client API
- Added advanced functional tests for MQTT v311 and v5 features
- Added missing functional, stress, and chaos test modules
- Added rmqtt-test to workspace members
- Simplified `max_packet_size` enforcement test
- Formatted CLI test arrays for better readability

---

## [0.21.0] - 2026-04

### New Features

- **Test Harness (rmqtt-test)**: New crate providing industrial-grade test harness with functional, stress, and chaos test suites. Five suite types covering MQTT 3.1, 3.1.1, 5.0, load testing, and fault injection.
- **Topic Rewrite Plugin**: Added `rmqtt-topic-rewrite` for flexible topic filter and topic name remapping.
- **P2P Messaging Plugin**: Added `rmqtt-p2p-messaging` for direct client-to-client message delivery.
- **HTTP API Metrics**: Added Prometheus metrics endpoint integration. View at `/api/v1/metrics`.
- **Shared Subscription Improvements**: Enhanced `$share/{group}/{topic}` subscription handling.

### Dependency Upgrades

- Upgraded `tokio` to 1.44
- Upgraded multiple workspace dependencies to latest compatible versions
- Upgraded Docker base images

### Fixes

- Fixed clippy warnings across all crates
- Fixed Docker build errors related to outdated dependencies
- Fixed subscription matching logic edge cases

---

## [0.20.0] - 2026-03

### New Features

- **NATS Bridging**: Added both ingress and egress NATS bridge plugins (`rmqtt-bridge-ingress-nats`, `rmqtt-bridge-egress-nats`).
- **ReductStore Bridge**: Added egress bridge for ReductStore time-series database.
- **Webhook Offline Messages**: Added `offline_message` event to webhook plugin.
- **Cluster HTTP API**: Enhanced HTTP API with cluster-wide operations via gRPC forwarding.

### Dependency Upgrades

- Upgraded `rdkafka` to 0.38 with pinned `rdkafka-sys`
- Improved Kafka delivery status logging

### Fixes

- Docker build improvements (reduced context size, fixed compile errors)
- Fixed warning about redundant message collection iterator usage

---

## [0.19.1] - 2026-02

### New Features

- **TLS Certificate Info Collection**: Added configurable `collect_cert_info` option for TLS listeners.
- **Propagate Certificate Info to Auth Events**: Certificate metadata now available during authentication hook.

### Fixes

- Suppressed clippy `large_err` warnings in raft store
- Fixed feature flag configuration for TLS
- Enabled `tls` feature for `rmqtt-net` dependency in `rmqtt-conf`

---

## [0.19.0] - 2026-01

### New Features

- **Client Certificate Authentication**: Added `tls_client_ca_certs` and `tls_cross_certificate` options for mutual TLS authentication.
- **Separate Client CA Bundle**: TLS now supports separate CA certificates for client authentication vs server verification.
- **Client-level ACL Management**: Added per-client ACL rule management in `rmqtt-acl` plugin.
- **Pulsar Bridge**: Added Pulsar ingress/egress bridge plugins.

### Changes

- Improved TLS configuration flexibility with separate CA trust anchors
- Enhanced ACL rule management API

---

## [0.18.0] - 2025-12

### New Features

- **Kafka Bridging**: Added Kafka ingress/egress bridge plugins (`rmqtt-bridge-ingress-kafka`, `rmqtt-bridge-egress-kafka`).
- **Webhook Plugin**: Added `rmqtt-web-hook` for HTTP-based event notifications.
- **Sys Topic Plugin**: Added `rmqtt-sys-topic` for `$SYS/` system metrics publishing.
- **Auto Subscription Plugin**: Added `rmqtt-auto-subscription` for auto-subscribing clients on connect.
- **Plugin System Maturity**: Stabilized plugin registration API with `register!` macro and `PackageInfo` trait.

### Changes

- Refactored MQTT codec (inspired by ntex-mqtt)
- Improved hook system with priority-based handler registration

---

## [0.17.0] - 2025-10

### New Features

- **Raft Clustering**: Production-ready `rmqtt-cluster-raft` plugin with configurable compression, health checks, and auto-exit.
- **Broadcast Clustering**: `rmqtt-cluster-broadcast` plugin for high-throughput eventual consistency.
- **Configuration Hot-Reload**: HTTP API plugin supports restartless config reload via graceful server swap.
- **Session/Message Storage**: Added `rmqtt-session-storage` (Sled/Redis) and `rmqtt-message-storage` (RAM/Redis) plugins.

---

## [0.16.0] - 2025-08

### New Features

- **MQTT v5.0 Protocol Support**: Complete implementation including:
  - Session Expiry, Message Expiry
  - Topic Aliases, Subscription Identifiers
  - User Properties, Request/Response
  - Flow Control, Server Keep Alive
  - Assigned Client ID, Maximum Packet Size
- **Retained Message Storage**: `rmqtt-retainer` plugin with RAM, Sled, and Redis backends.
- **HTTP API Plugin**: Initial REST API for broker management.
- **ACL Plugin**: File-based ACL rule engine.

---

## [0.15.0] - 2025-06

### Major Changes

- **Plugin System**: Introduced modular plugin architecture with `#[derive(Plugin)]` and hook-based extension.
- **Codec Rewrite**: MQTT encoding/decoding rewritten with inspiration from ntex-mqtt. Zero-copy, version-negotiating codec.
- **Feature Flag Restructure**: Modular feature flags replacing monolithic builds.
- **Rustls TLS Backend**: Migrated from native-tls to rustls for cross-platform TLS support.

---

## [0.13.0] and earlier

Earlier versions relied on maintained forks of `ntex` and `ntex-mqtt` as dependencies.
