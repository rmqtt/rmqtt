//! MQTT bridge egress: illegal forwarded topic names + v5 property bleed
//! (GitHub issue #451)
//!
//! The reporter runs `rmqtt-bridge-ingress-mqtt` and
//! `rmqtt-bridge-egress-mqtt` on the same broker and sees the remote
//! mosquitto answer `disconnected: malformed packet.` on every egress
//! connection, so the bridge reconnects, forwards once, and is dropped again —
//! with no detail on the RMQTT side. The reporter's own configuration could
//! not be obtained, so this suite reproduces the mechanism instead of the
//! configuration:
//!
//! * `Remote::make_topic()` returns `remote.topic` verbatim when it contains no
//!   `${local.topic}`, and substitutes the local topic when it does — it never
//!   validates the RESULT (`rmqtt-plugins/rmqtt-bridge-egress-mqtt/src/config.rs`).
//! * `BridgeManager::send()` puts that string straight into the outgoing
//!   PUBLISH (`.../egress-mqtt/src/bridge.rs`), and `to_string_skip()` returns
//!   `""` once `skip_levels` reaches the local level count
//!   (`rmqtt/src/topic.rs`).
//! * `Local::make_topic()` on the ingress side has the same shape
//!   (`.../ingress-mqtt/src/config.rs`), so an ingress mapping can hand the
//!   broker a local topic that is not a legal topic NAME — which the egress
//!   then carries out with `${local.topic}`.
//!
//! A peer that checks the topic name (mosquitto does: `handle_publish.c` ->
//! `mosquitto_pub_topic_check()`) answers with `MOSQ_ERR_MALFORMED_PACKET`, and
//! every other guess (topic alias, receive maximum, maximum packet size, QoS,
//! clean start) maps onto a DIFFERENT mosquitto message, so the illegal topic
//! name is the only explanation consistent with the report. A normal client can
//! never publish such a topic, which is why the defect only reproduces with a
//! particular mapping shape — the maintainer's own docker attempt used
//! `local.topic = "${remote.topic}"` and could not reproduce it (see
//! designs/issue-451-egress-malformed-packet-analysis.md).
//!
//! # How the suite observes the wire
//!
//! Both bridges are pointed at MOCK MQTT brokers that live inside the test
//! process (`MockBroker`): one upstream (`127.0.0.1:2915`) the ingress bridge
//! subscribes on, one downstream (`127.0.0.1:3915`) the egress bridge publishes
//! to. No docker, no mosquitto, no external service — and the bytes the bridge
//! puts on the wire become an assertable object:
//!
//! ```text
//!   mock upstream --PUBLISH--> ingress bridge        (entry I<n>)
//!        --local publish (local.topic mapping)-->
//!        [ broker under test ] --MessagePublish hook--> egress bridge
//!        (entry E<n>) --PUBLISH (remote.topic mapping)--> mock downstream
//! ```
//!
//! The egress cases drive the chain with a plain v5 client of the broker under
//! test; C3 and the ingress cases feed it through the mock upstream so the
//! `local.topic` mapping shapes are enumerated instead of guessed.
//!
//! # Cases
//!
//! All nine cases are green, except the two that guard the still-open issue
//! #540 (the topic FILTER side of an entry and what a refused subscription does
//! to the entry). Two are the happy path of either direction (a local publish
//! must reach the peer, an upstream publish must reach a local subscriber); the
//! rest are the regression net for the two guards that close issue #451
//! (load-time validation of the configured topic skeleton, and the `send()`-time
//! topic-name check).
//!
//! | case | shape under test | expected |
//! |---|---|---|
//! | `..._egress_forward_local_publish`              | egress `remote.topic = "mirror/control/${local.topic}"` — the happy path, asserted on its own | PASS: forwarded verbatim |
//! | `..._egress_empty_topic_after_skip_levels`      | C2: `remote.topic = "${local.topic}"` + `skip_levels = 8` collapses to the empty topic | PASS: dropped + one WARN |
//! | `..._egress_v5_cross_connection_properties`     | C3: v5 properties that cannot cross a connection (Topic Alias, Subscription Identifier) | PASS: stripped, the rest forwarded |
//! | `..._ingress_local_topic_placeholder_transparent` | ingress `local.topic = "${remote.topic}"` | PASS (control) |
//! | `..._ingress_local_topic_literal_plain`         | ingress `local.topic = "ing/plain"` | PASS (control) |
//! | `..._ingress_local_topic_with_suffix`           | ingress `local.topic = "${remote.topic}/suffix"` | PASS (control) |
//! | `..._ingress_delivers_to_local_subscriber`      | the I1 mapping, asserted at a plain LOCAL subscriber rather than at the egress carrier | PASS |
//! | `issue540_ingress_share_filter_subscribed_verbatim` | ingress `remote.topic = "$share/g/up/share/#"` must reach the peer byte for byte | PASS: subscription seen verbatim (guards against over-rejection) |
//! | `issue540_ingress_refused_subscription_is_bounded`  | the peer answers SUBACK `TopicFilterInvalid` (0x8F) for one entry | FAIL today: retried every 5 s, forever — a bounded retry (or none) is expected |
//!
//! The two `issue540_*` cases cover the topic FILTER field of an entry (the
//! sibling of the topic NAME field #451 fixed): `remote.topic` is the filter the
//! bridge sends to the peer, and a peer is entitled to refuse it. What the
//! broker does NOT do with such a filter while loading the config is asserted by
//! the `bridge_config_reject` suite, which owns its own brokers for it.
//!
//! Three earlier cases — C1 (`remote.topic = "mirror/hash/#"`), C4 (log
//! diagnosability of such a forward) and I3 (`local.topic = "ing/mirror/#"`)
//! — were retired: a literal of filter shape is now rejected while the config
//! is loaded, so the broker refuses to start and the case could no longer
//! reach the wire. That guard is covered by the plugins' own unit tests
//! (`PluginConfig::validate`), not from here.
//!
//! Every negative assertion is paired with a positive control arm on the same
//! broker (`eg/control/#` and friends): the control proves the mocks really do
//! observe the bridge's output, so "nothing arrived" can never be mistaken for
//! a pass. The happy path is additionally guarded by its own two cases, so that
//! its coverage does not live inside a control arm — an arm exists to prove an
//! arm, not to guard a behaviour.
//!
//! # Running it
//!
//! The suite owns `configs/bridge-egress-mqtt/` (MQTT 1915 / gRPC 5395 /
//! HTTP 6075) and is deliberately NOT part of the default full run: it starts
//! its own mock brokers on fixed ports and needs a real `rmqttd` binary.
//!
//! ```text
//! target/debug/mqtt_harness --binary target/debug/rmqttd --workspace . \
//!     --suites bridge_egress_mqtt --workers 1
//! ```
//!
//! The scheduler switches the harness broker to the fixture at the sub-suite
//! boundary, and `broker_addr()` retargets the health probe at 1915 (the
//! harness-wide `--addr` is never used by these cases — see the note in
//! `flapping_v5.rs`). Out of scope on purpose: the same defect over MQTT 3.1.1
//! (`mqtt_ver = "v4"`), which needs a v3 mock and can be added as a follow-up
//! entry once the topic-name check lands.

use std::num::{NonZeroU16, NonZeroU32};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use bytestring::ByteString;
use futures::{SinkExt, StreamExt};
use rmqtt_codec::types::Publish;
use rmqtt_codec::v5::{
    Codec as V5Codec, ConnectAck, ConnectAckReason, Packet, PublishProperties, SubscribeAck,
    SubscribeAckReason,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::codec::Framed;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::client::IncomingMessage;
use crate::tests::functional::connack_return_codes_v311::copy_dir_recursive;

/// Suite (and sub-suite prefix) name.
pub(crate) const SUITE: &str = "bridge_egress_mqtt";
/// MQTT port of the fixture broker; must equal `listener.tcp.external.addr` of
/// `configs/bridge-egress-mqtt/rmqtt.toml` AND what [`broker_addr`] declares.
const BROKER_ADDR: &str = "127.0.0.1:1915";
/// Mock upstream MQTT broker the ingress bridge connects to.
const UPSTREAM_PORT: u16 = 2915;
/// Mock downstream MQTT broker the egress bridge connects to.
const DOWNSTREAM_PORT: u16 = 3915;
/// How long to wait for a bridge client to (re)attach to a mock. The fixture
/// sets `reconnect_interval = "1s"`, so this is ~20 attempts.
const BRIDGE_WAIT: Duration = Duration::from_secs(20);
/// How long to wait for one forwarded PUBLISH to show up at a mock.
const RECV_WAIT: Duration = Duration::from_secs(5);
/// Settle time after a bridge connection appears and before it is driven: the
/// bridge installs its publish sink only once it has processed our CONNACK,
/// and a PUBLISH that arrives before that is dropped by the bridge.
const CONNECT_SETTLE: Duration = Duration::from_millis(400);
/// Attempts of the driving publish/inject before a case gives up. Each one is
/// idempotent for the assertions here (they look at the topic, not at counts),
/// so retrying cannot mask anything — it only absorbs the settle race above.
const DRIVE_ATTEMPTS: usize = 3;
/// Window in which a subscription the peer REFUSED must not be retried on an
/// unbounded fixed cadence (issue #540). The ingress plugin's retry loop sleeps
/// 5 s, so 12 s holds exactly two retries on the current code.
const RETRY_WINDOW: Duration = Duration::from_secs(12);
/// Retries allowed inside [`RETRY_WINDOW`] after the first refusal: give up, or
/// back off — a fixed cadence is what the case forbids.
const MAX_REFUSAL_RETRIES: usize = 1;

// ---------------------------------------------------------------------------
// Fixture entry shapes (indices are the declaration order in the fixture's
// plugin configs; the egress entry index is embedded in the bridge client id).
// ---------------------------------------------------------------------------

/// E2 — `remote.topic = "${local.topic}"` + `skip_levels = 8`.
///
/// (E1 — index 0 — is a retired placeholder in the fixture: it used to carry a
/// `remote.topic` literal of filter shape, which the bridge now rejects while
/// loading its config. The entry stays declared so this index — and the one
/// embedded in the bridge client id — keeps its meaning.)
const EGRESS_ENTRY_SKIP_LEVELS: usize = 1;
/// E3 — `remote.topic = "mirror/props/${local.topic}"` (C3 carrier).
const EGRESS_ENTRY_PROPS: usize = 2;
/// E4 — `remote.topic = "mirror/control/${local.topic}"` (positive control).
const EGRESS_ENTRY_CONTROL: usize = 3;
/// E5 — `local.topic_filter = "ing/#"` (carrier for the ingress shapes).
const EGRESS_ENTRY_INGRESS_CARRIER: usize = 4;
// index 5 (E6) is a retired placeholder — nothing addresses it any more.
/// E7 — `local.topic_filter = "up/suffix/#"` (I4 carrier).
const EGRESS_ENTRY_SUFFIX: usize = 6;
/// E8 — `local.topic_filter = "up/transparent/#"` (I1 carrier).
const EGRESS_ENTRY_TRANSPARENT: usize = 7;

/// I1 — `local.topic = "${remote.topic}"` (transparent).
const INGRESS_FILTER_TRANSPARENT: &str = "up/transparent/#";
/// Topic the upstream injects for I1. The transparent mapping keeps it
/// verbatim, so it is also what a plain LOCAL subscriber of the broker must
/// see — the observation point of `ingress_delivers_to_local_subscriber`.
const INGRESS_REMOTE_TOPIC_TRANSPARENT: &str = "up/transparent/msg";
/// I2 — `local.topic = "ing/plain"` (literal, no wildcard).
const INGRESS_FILTER_PLAIN: &str = "up/plain/#";
// I3 (`up/wildcard/#`) is a retired placeholder — nothing addresses it any
// more.
/// I4 — `local.topic = "${remote.topic}/suffix"` (placeholder + suffix).
const INGRESS_FILTER_SUFFIX: &str = "up/suffix/#";
/// I5 — transparent carrier feeding the egress entry E3 (C3).
const INGRESS_FILTER_PROPS: &str = "eg/props/#";
/// I6 — `remote.topic = "$share/g/up/share/#"`: a shared-subscription filter,
/// which is a topic FILTER that must be accepted and forwarded to the peer
/// verbatim (issue #540).
const INGRESS_FILTER_SHARE: &str = "$share/g/up/share/#";

/// Local topic of the positive control arm (`eg/control/#` -> E4).
const LOCAL_TOPIC_CONTROL: &str = "eg/control/local";
/// The topic the control arm must produce downstream.
const CONTROL_FORWARDED: &str = "mirror/control/eg/control/local";

// ---------------------------------------------------------------------------
// Fixture preparation
// ---------------------------------------------------------------------------

/// `target/bridge-egress-mqtt/` — the prepared copy of the fixture config.
fn prepared_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target").join("bridge-egress-mqtt")
}

/// Log file the fixture broker writes to (`log.to = "file"`), i.e. the most
/// convenient place for a case to observe what RMQTT logged. The harness
/// starts a self-managed broker with `Stdio::piped()` and drains both pipes
/// into `tracing::debug!` (`broker/lifecycle.rs`), so console output is only
/// visible with `--verbose`; a file is deterministic and greppable.
fn log_path() -> PathBuf {
    prepared_dir().join("logs").join("bridge-egress-mqtt.log")
}

/// Path of the fixture's `rmqtt.toml`, prepared once per process.
///
/// Called by [`TestCase::broker_config`], i.e. during suite building, so the
/// file exists before the scheduler restarts the broker at the sub-suite
/// boundary.
fn prepared_config() -> PathBuf {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| prepare_config().expect("prepare the bridge-egress-mqtt fixture config")).clone()
}

/// Copy `configs/bridge-egress-mqtt/` into `target/bridge-egress-mqtt/` and
/// make it CWD-independent:
///
/// 1. `plugins.dir` -> absolute path of the copied `plugins/` (the broker
///    resolves a relative path against its own working directory);
/// 2. `log.dir` -> absolute path of the copied `logs/` (created here), so the
///    diagnosability case reads a known file.
///
/// Returns the `rmqtt.toml` FILE: `-f` must point at the file, a directory is
/// silently ignored by rmqtt-conf and the built-in defaults would be used.
fn prepare_config() -> anyhow::Result<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join("bridge-egress-mqtt");
    let dst = prepared_dir();
    copy_dir_recursive(&src, &dst)?;

    let logs = dst.join("logs");
    std::fs::create_dir_all(&logs)?;

    let abs_plugins = std::fs::canonicalize(dst.join("plugins"))?.to_string_lossy().replace('\\', "/");
    let abs_logs = std::fs::canonicalize(&logs)?.to_string_lossy().replace('\\', "/");

    let main_cfg = dst.join("rmqtt.toml");
    let content = std::fs::read_to_string(&main_cfg)?;
    let updated = content.replace("@@PLUGINS_DIR@@", &abs_plugins).replace("@@LOG_DIR@@", &abs_logs);
    std::fs::write(&main_cfg, updated)?;
    Ok(main_cfg)
}

// ---------------------------------------------------------------------------
// Mock MQTT broker (v5)
// ---------------------------------------------------------------------------

/// One PUBLISH the mock received from the bridge under test.
#[derive(Clone, Debug)]
struct Captured {
    /// Client id the bridge used on this connection (carries `egress`/`ingress`,
    /// the node id and the entry index).
    conn_client_id: String,
    /// Topic name as it appeared on the wire. This is the object of interest:
    /// an empty string or one containing `+`/`#` is what mosquitto rejects.
    topic: String,
    payload: Vec<u8>,
    properties: PublishProperties,
}

/// A connection accepted by the mock.
struct Conn {
    client_id: String,
    subscriptions: Vec<String>,
    /// Feeds packets to this connection's writer half.
    tx: mpsc::UnboundedSender<Packet>,
}

#[derive(Default)]
struct MockState {
    conns: Vec<Conn>,
    received: Vec<Captured>,
    /// Filters this mock refuses with SUBACK `TopicFilterInvalid` (issue #540):
    /// a peer is free to refuse a subscription, and what the broker then does
    /// with the entry is the behaviour under test.
    refused: Vec<String>,
    /// How many times each filter was subscribed, across all connections.
    attempts: std::collections::HashMap<String, usize>,
    /// Whether a filter was ever refused, so a case can prove the mock was
    /// selective instead of refusing everything.
    refused_seen: std::collections::HashMap<String, bool>,
}

/// A minimal MQTT v5 broker that lives in the test process.
///
/// It answers CONNECT/CONNACK and SUBSCRIBE/SUBACK, answers PINGREQ, records
/// every PUBLISH it receives, and can push a PUBLISH to a chosen connection —
/// which is what makes it usable both as the ingress bridge's upstream (the
/// case injects a message) and as the egress bridge's downstream (the case
/// asserts on what the bridge produced).
///
/// It deliberately does NOT enforce topic-name legality. A peer that tears the
/// connection down on a bad topic (mosquitto does) would only hide the byte
/// sequence under test, and keeping the mock permissive makes the verdict a
/// plain assertion on what was recorded instead of an inferred disconnect.
struct MockBroker {
    role: &'static str,
    state: Arc<Mutex<MockState>>,
    _accept: tokio::task::JoinHandle<()>,
}

impl MockBroker {
    /// A permissive mock: every subscription is granted.
    async fn spawn(role: &'static str, port: u16) -> anyhow::Result<Self> {
        Self::spawn_refusing(role, port, &[]).await
    }

    /// A mock that answers SUBACK `TopicFilterInvalid` (0x8F) for the listed
    /// filters — a peer is entitled to refuse a subscription it will not grant
    /// (an ACL, a filter it does not implement), and issue #540 is about what the
    /// broker then does with the entry.
    async fn spawn_refusing(role: &'static str, port: u16, refused: &[&str]) -> anyhow::Result<Self> {
        // The previous case's mock closed its listener moments ago and the
        // bridge under test is still holding sockets to it, so the port can be
        // momentarily unavailable. Retry instead of failing the case.
        let listener = {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                match TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => break listener,
                    Err(e) if Instant::now() >= deadline => {
                        return Err(anyhow::anyhow!(
                            "mock {role} broker cannot bind 127.0.0.1:{port}: {e} \
                             (a residual process may still hold the port)"
                        ));
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                }
            }
        };
        let state = Arc::new(Mutex::new(MockState {
            refused: refused.iter().map(|f| f.to_string()).collect(),
            ..MockState::default()
        }));
        let st = state.clone();
        let accept = tokio::spawn(async move {
            loop {
                let Ok((stream, peer)) = listener.accept().await else { break };
                let st = st.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_conn(stream, st).await {
                        tracing::debug!("mock {role} connection from {peer} ended: {e:#}");
                    }
                });
            }
        });
        Ok(Self { role, state, _accept: accept })
    }

    /// `(client id, subscriptions)` of every connection seen so far — used in
    /// failure messages so a wait timeout is self-explanatory.
    fn conn_snapshot(&self) -> Vec<(String, Vec<String>)> {
        self.state
            .lock()
            .unwrap()
            .conns
            .iter()
            .map(|c| (c.client_id.clone(), c.subscriptions.clone()))
            .collect()
    }

    /// Every PUBLISH received so far.
    fn captured_snapshot(&self) -> Vec<Captured> {
        self.state.lock().unwrap().received.clone()
    }

    /// How many SUBSCRIBE attempts the bridge made for `filter` (issue #540:
    /// a refused subscription must not be retried on an unbounded fixed
    /// cadence).
    fn subscribe_attempts(&self, filter: &str) -> usize {
        self.state.lock().unwrap().attempts.get(filter).copied().unwrap_or(0)
    }

    /// `Some(refused)` when `filter` was ever subscribed, with `refused` saying
    /// whether this mock answered `TopicFilterInvalid`. This is the control arm
    /// of the refusal case: it proves the mock was selective and that both
    /// filters really were subscribed.
    fn refusal_of(&self, filter: &str) -> Option<bool> {
        self.state.lock().unwrap().refused_seen.get(filter).copied()
    }

    /// Poll `probe` until it returns an index or `timeout` elapses.
    async fn wait_index<F>(&self, timeout: Duration, probe: F) -> Option<usize>
    where
        F: Fn(&MockState) -> Option<usize>,
    {
        let deadline = Instant::now() + timeout;
        loop {
            // Scoped so the guard never lives across the await below.
            let found = {
                let st = self.state.lock().unwrap();
                probe(&st)
            };
            if let Some(idx) = found {
                return Some(idx);
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Index of the connection whose client id contains `needle`.
    async fn wait_client_id(&self, needle: &str, timeout: Duration) -> anyhow::Result<usize> {
        self.wait_index(timeout, |st| st.conns.iter().position(|c| c.client_id.contains(needle)))
            .await
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "the mock {} saw no bridge client whose id contains {needle:?} within {timeout:?}; \
                     connections seen: {:?}",
                    self.role,
                    self.conn_snapshot()
                )
            })
    }

    /// Index of the connection that subscribed to `filter` (the ingress bridge
    /// subscribes with the entry's `remote.topic`).
    async fn wait_subscription(&self, filter: &str, timeout: Duration) -> anyhow::Result<usize> {
        self.wait_index(timeout, |st| {
            st.conns.iter().position(|c| c.subscriptions.iter().any(|f| f == filter))
        })
        .await
        .ok_or_else(|| {
            anyhow::anyhow!(
                "the mock {} saw no connection subscribing to {filter:?} within {timeout:?}; \
                 connections seen: {:?}",
                self.role,
                self.conn_snapshot()
            )
        })
    }

    /// Send a QoS 0 PUBLISH to the connection at `idx` (the mock acting as a
    /// broker: this is how the ingress case injects a message).
    fn send_publish(
        &self,
        idx: usize,
        topic: &str,
        payload: &[u8],
        properties: PublishProperties,
    ) -> anyhow::Result<()> {
        let tx = {
            let st = self.state.lock().unwrap();
            st.conns.get(idx).map(|c| c.tx.clone())
        }
        .ok_or_else(|| anyhow::anyhow!("mock {}: no connection at index {idx}", self.role))?;

        let packet = Packet::Publish(Box::new(Publish {
            dup: false,
            retain: false,
            qos: QoS::AtMostOnce,
            topic: ByteString::from(topic),
            packet_id: None,
            properties: Some(properties),
            payload: Bytes::copy_from_slice(payload),
        }));
        tx.send(packet).map_err(|e| anyhow::anyhow!("mock {}: cannot queue a PUBLISH: {e}", self.role))
    }

    /// Poll for a PUBLISH whose payload equals `payload` (the per-case marker).
    async fn wait_publish(&self, payload: &[u8], timeout: Duration) -> Option<Captured> {
        let deadline = Instant::now() + timeout;
        loop {
            let hit = {
                let st = self.state.lock().unwrap();
                st.received.iter().find(|r| r.payload == payload).cloned()
            };
            if hit.is_some() {
                return hit;
            }
            if Instant::now() >= deadline {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Failure detail for a PUBLISH that never arrived.
    fn no_forward_detail(&self) -> String {
        format!(
            "downstream mock {}: connections {:?}, captured {} PUBLISH(es)",
            self.role,
            self.conn_snapshot(),
            self.captured_snapshot().len()
        )
    }
}

/// Serve one accepted connection: CONNECT/CONNACK handshake, then a select loop
/// that reads inbound packets and drains the outbound channel.
async fn serve_conn(stream: tokio::net::TcpStream, state: Arc<Mutex<MockState>>) -> anyhow::Result<()> {
    let mut framed = Framed::new(stream, V5Codec::new(0, 0));

    let first =
        framed.next().await.ok_or_else(|| anyhow::anyhow!("peer closed the connection before CONNECT"))?;
    let (packet, _) = first.map_err(|e| anyhow::anyhow!("decode error while reading CONNECT: {e}"))?;
    let client_id = match packet {
        Packet::Connect(c) => c.client_id.to_string(),
        other => return Err(anyhow::anyhow!("expected CONNECT, got {}", packet_name(&other))),
    };

    framed
        .send(Packet::ConnectAck(Box::new(ConnectAck {
            reason_code: ConnectAckReason::Success,
            ..Default::default()
        })))
        .await
        .map_err(|e| anyhow::anyhow!("encode error while sending CONNACK: {e}"))?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Packet>();
    let idx = {
        let mut st = state.lock().unwrap();
        st.conns.push(Conn { client_id: client_id.clone(), subscriptions: Vec::new(), tx });
        st.conns.len() - 1
    };

    let (mut sink, mut stream) = framed.split();
    loop {
        tokio::select! {
            outgoing = rx.recv() => match outgoing {
                Some(packet) => sink
                    .send(packet)
                    .await
                    .map_err(|e| anyhow::anyhow!("encode error: {e}"))?,
                // The `Conn` was dropped (mock going away).
                None => break,
            },
            incoming = stream.next() => match incoming {
                None => break,
                Some(Err(e)) => return Err(anyhow::anyhow!("decode error: {e}")),
                Some(Ok((packet, _))) => match packet {
                    Packet::Subscribe(s) => {
                        let filters: Vec<String> =
                            s.topic_filters.iter().map(|(f, _)| f.to_string()).collect();
                        // The refusal policy lives in the mock's state, so a case
                        // can pick which filter the peer "will not have".
                        let refused = state.lock().unwrap().refused.clone();
                        let status: Vec<SubscribeAckReason> = filters
                            .iter()
                            .map(|f| {
                                if refused.iter().any(|r| r == f) {
                                    SubscribeAckReason::TopicFilterInvalid
                                } else {
                                    SubscribeAckReason::GrantedQos0
                                }
                            })
                            .collect();
                        {
                            let mut st = state.lock().unwrap();
                            for (f, reason) in filters.iter().zip(status.iter()) {
                                *st.attempts.entry(f.clone()).or_insert(0) += 1;
                                st.refused_seen.insert(
                                    f.clone(),
                                    matches!(reason, SubscribeAckReason::TopicFilterInvalid),
                                );
                            }
                            st.conns[idx].subscriptions.extend(filters);
                        }
                        sink.send(Packet::SubscribeAck(SubscribeAck {
                            packet_id: s.packet_id,
                            properties: Vec::new(),
                            reason_string: None,
                            status,
                        }))
                        .await
                        .map_err(|e| anyhow::anyhow!("encode error while sending SUBACK: {e}"))?;
                    }
                    Packet::PingRequest => {
                        sink.send(Packet::PingResponse)
                            .await
                            .map_err(|e| anyhow::anyhow!("encode error while sending PINGRESP: {e}"))?;
                    }
                    Packet::Publish(p) => {
                        let captured = Captured {
                            conn_client_id: client_id.clone(),
                            topic: p.topic.to_string(),
                            payload: p.payload.to_vec(),
                            properties: p.properties.clone().unwrap_or_default(),
                        };
                        state.lock().unwrap().received.push(captured);
                    }
                    Packet::Disconnect(_) => break,
                    _ => {}
                },
            },
        }
    }
    Ok(())
}

fn packet_name(packet: &Packet) -> &'static str {
    match packet {
        Packet::Connect(_) => "CONNECT",
        Packet::ConnectAck(_) => "CONNACK",
        Packet::Publish(_) => "PUBLISH",
        Packet::PublishAck(_) => "PUBACK",
        Packet::PublishReceived(_) => "PUBREC",
        Packet::PublishRelease(_) => "PUBREL",
        Packet::PublishComplete(_) => "PUBCOMP",
        Packet::Subscribe(_) => "SUBSCRIBE",
        Packet::SubscribeAck(_) => "SUBACK",
        Packet::Unsubscribe(_) => "UNSUBSCRIBE",
        Packet::UnsubscribeAck(_) => "UNSUBACK",
        Packet::PingRequest => "PINGREQ",
        Packet::PingResponse => "PINGRESP",
        Packet::Disconnect(_) => "DISCONNECT",
        Packet::Auth(_) => "AUTH",
    }
}

// ---------------------------------------------------------------------------
// Assertions / helpers
// ---------------------------------------------------------------------------

/// `Some(reason)` when `topic` is not a legal PUBLISH topic NAME.
///
/// MQTT 5.0 §3.3.2.1 / §4.7.3: the Topic Name must be at least one character
/// long and must not contain the wildcard characters `+` or `#`.
fn topic_name_violation(topic: &str) -> Option<String> {
    if topic.is_empty() {
        return Some("it is EMPTY".to_string());
    }
    if topic.contains('#') {
        return Some(format!("it contains the multi-level wildcard '#': {topic:?}"));
    }
    if topic.contains('+') {
        return Some(format!("it contains the single-level wildcard '+': {topic:?}"));
    }
    if topic.len() > 65535 {
        return Some(format!("it is {} bytes long (limit 65535)", topic.len()));
    }
    None
}

/// Turn a [`topic_name_violation`] into the failure the suite reports, with the
/// consequence spelled out (the peer tears the connection down).
fn assert_legal_topic(captured: &Captured, what: &str) -> anyhow::Result<()> {
    match topic_name_violation(&captured.topic) {
        None => Ok(()),
        Some(reason) => Err(anyhow::anyhow!(
            "the egress bridge forwarded an illegal topic name to the peer ({what}): {reason}. \
             A peer that validates the topic name (mosquitto: `disconnected: malformed packet.`) \
             drops the connection, the bridge reconnects and forwards once more — the loop \
             reported in issue #451. Captured PUBLISH: topic={:?}, payload={:?}, client_id={:?}",
            captured.topic,
            String::from_utf8_lossy(&captured.payload),
            captured.conn_client_id
        )),
    }
}

fn nonce() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Run one case body on a thread with a 32 MiB stack (the established pattern
/// for cases that build clients; see `tests/functional/pulsar/common.rs`).
fn run_case<F>(name: &str, body: F) -> TestResult
where
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>
        + Send
        + 'static,
{
    let start = Instant::now();
    let spawned = std::thread::Builder::new()
        .name("bridge-egress-mqtt".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| anyhow::anyhow!("failed to create a tokio runtime: {e}"))?;
            rt.block_on(body())
        });

    match spawned {
        Ok(handle) => match handle.join() {
            Ok(Ok(())) => TestResult::passed(name, SUITE, start.elapsed()),
            Ok(Err(e)) => TestResult::failed(name, SUITE, start.elapsed(), format!("{e:#}")),
            Err(_) => TestResult::failed(name, SUITE, start.elapsed(), "test thread panicked".to_string()),
        },
        Err(e) => {
            TestResult::error(name, SUITE, start.elapsed(), format!("failed to spawn a test thread: {e}"))
        }
    }
}

/// A plain v5 client of the broker under test (the harness-wide `--addr` is
/// NOT this fixture's port, so the address is pinned here).
///
/// It plays both local roles a case needs: PUBLISHER, to drive the egress
/// mappings, and SUBSCRIBER, to observe what an ingress mapping delivered into
/// the broker without routing that assertion through the egress bridge.
struct LocalClient {
    inner: crate::mqtt::v5::MqttV5Client,
}

impl LocalClient {
    async fn connect(marker: &str) -> anyhow::Result<Self> {
        let inner = crate::mqtt::v5::MqttV5Client::connect(
            BROKER_ADDR,
            &format!("bridge-egress-mqtt-{marker}"),
            Duration::from_secs(5),
        )
        .await?;
        Ok(Self { inner })
    }

    /// Subscribe and REQUIRE a granting SUBACK, so that a later "nothing
    /// arrived" can never really be "the subscription was silently refused".
    async fn subscribe(&mut self, filter: &str, qos: QoS) -> anyhow::Result<()> {
        let ack = self.inner.subscribe(filter, qos).await?;
        let granted = matches!(
            ack.status.first(),
            Some(
                SubscribeAckReason::GrantedQos0
                    | SubscribeAckReason::GrantedQos1
                    | SubscribeAckReason::GrantedQos2
            )
        );
        if !granted {
            return Err(anyhow::anyhow!(
                "the broker did not grant a subscription to {filter:?}: SUBACK status {:?}",
                ack.status
            ));
        }
        Ok(())
    }

    async fn publish(&self, topic: &str, payload: &[u8]) -> anyhow::Result<()> {
        self.inner.publish(topic, payload, QoS::AtMostOnce, false).await
    }

    /// Wait for the delivery whose payload equals `payload` (the case's
    /// marker) and return it.
    ///
    /// Non-matching deliveries are skipped rather than treated as failure: all
    /// cases of this suite share one fixture broker, so a stray delivery from
    /// another subscriber of the same topic simply is not this case's message.
    async fn wait_delivery(&mut self, payload: &[u8], timeout: Duration) -> Option<IncomingMessage> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            match self.inner.recv_message_timeout(remaining).await {
                Some(msg) if &msg.payload[..] == payload => return Some(msg),
                Some(_) => continue,
                None => return None,
            }
        }
    }

    async fn disconnect(self) {
        let _ = self.inner.disconnect().await;
    }
}

/// Publish `local_topic` as a plain client of the broker under test and return
/// the PUBLISH the egress entry `entry_idx` produced on the mock downstream.
///
/// Retries the local publish: the bridge drops a command while its sink is not
/// attached yet (the mailbox logs `mqtt sink is None` and moves on), so the
/// first attempt can be lost to the settle race after CONNACK.
async fn forward_locally(
    downstream: &MockBroker,
    entry_idx: usize,
    local_topic: &str,
    marker: &str,
) -> anyhow::Result<Captured> {
    downstream.wait_client_id(&format!(":egress:1:{entry_idx}:0"), BRIDGE_WAIT).await?;
    tokio::time::sleep(CONNECT_SETTLE).await;

    let client = LocalClient::connect(marker).await?;
    let mut captured = None;
    for attempt in 1..=DRIVE_ATTEMPTS {
        client.publish(local_topic, marker.as_bytes()).await?;
        captured = downstream.wait_publish(marker.as_bytes(), RECV_WAIT).await;
        if captured.is_some() {
            break;
        }
        tracing::debug!(
            "bridge_egress_mqtt: local publish {attempt}/{DRIVE_ATTEMPTS} on {local_topic:?} not forwarded yet"
        );
    }
    client.disconnect().await;

    captured.ok_or_else(|| {
        anyhow::anyhow!(
            "the egress bridge never forwarded a local publish on {local_topic:?} \
             ({DRIVE_ATTEMPTS} attempts, {RECV_WAIT:?} each); {}",
            downstream.no_forward_detail()
        )
    })
}

/// Inject one PUBLISH into the ingress bridge through the mock upstream (entry
/// selected by the topic filter it subscribed with) and return what the egress
/// bridge forwarded to the mock downstream.
async fn drive_ingress(
    upstream: &MockBroker,
    downstream: &MockBroker,
    ingress_filter: &str,
    egress_entry_idx: usize,
    remote_topic: &str,
    marker: &str,
    properties: PublishProperties,
) -> anyhow::Result<Captured> {
    let up_conn = upstream.wait_subscription(ingress_filter, BRIDGE_WAIT).await?;
    downstream.wait_client_id(&format!(":egress:1:{egress_entry_idx}:0"), BRIDGE_WAIT).await?;
    tokio::time::sleep(CONNECT_SETTLE).await;

    let mut captured = None;
    for attempt in 1..=DRIVE_ATTEMPTS {
        upstream.send_publish(up_conn, remote_topic, marker.as_bytes(), properties.clone())?;
        captured = downstream.wait_publish(marker.as_bytes(), RECV_WAIT).await;
        if captured.is_some() {
            break;
        }
        tracing::debug!(
            "bridge_egress_mqtt: ingress inject {attempt}/{DRIVE_ATTEMPTS} on {remote_topic:?} not forwarded yet"
        );
    }

    captured.ok_or_else(|| {
        anyhow::anyhow!(
            "the ingress->egress chain never forwarded {remote_topic:?} to the peer \
             ({DRIVE_ATTEMPTS} injections, {RECV_WAIT:?} each); upstream connections: {:?}; {}",
            upstream.conn_snapshot(),
            downstream.no_forward_detail()
        )
    })
}

/// Read the broker log appended since `offset`.
fn log_since(offset: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};

    let path = log_path();
    let Ok(mut file) = std::fs::File::open(&path) else {
        return String::new();
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return String::new();
    }
    let mut out = String::new();
    let _ = file.read_to_string(&mut out);
    out
}

/// Current log size (the baseline for [`log_since`]); 0 when the file does not
/// exist yet.
fn log_len() -> u64 {
    std::fs::metadata(log_path()).map(|m| m.len()).unwrap_or(0)
}

/// Tail of the broker log, for failure messages.
fn log_tail(lines: usize) -> String {
    let content = std::fs::read_to_string(log_path()).unwrap_or_default();
    let all: Vec<&str> = content.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// The levels the broker's log format uses, as whole whitespace-separated
/// tokens: `2026-10-16 21:26:58.320  INFO rmqtt_net::builder: 598: ...`.
///
/// Matching a *token* rather than a substring is what keeps a record from being
/// credited with another record's level: a continuation line must not be taken
/// for a record start, and a message that merely mentions a word such as
/// `error` must not promote its own record.
const LOG_LEVELS: [&str; 5] = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];

/// `Some(level)` when `line` STARTS a log record, `None` when it continues one.
pub(crate) fn line_level(line: &str) -> Option<&str> {
    line.split_whitespace().find(|tok| LOG_LEVELS.contains(tok))
}

/// Split a log window into `(level, text)` records.
///
/// A record starts at a physical line carrying a level token and absorbs the
/// lines that follow it: the console/file layer wraps a long message over
/// several physical lines, so matching line by line would be brittle. Whatever
/// precedes the first record start is dropped (it is not a whole record).
pub(crate) fn records(window: &str) -> Vec<(&str, String)> {
    let mut out: Vec<(&str, String)> = Vec::new();
    for line in window.lines() {
        match line_level(line) {
            Some(level) => out.push((level, line.to_string())),
            None => {
                if let Some(last) = out.last_mut() {
                    last.1.push('\n');
                    last.1.push_str(line);
                }
            }
        }
    }
    out
}

/// Split a log window into WARN records.
///
/// The suite's original entry point: C2 counts the WARN records naming one
/// entry. See [`records`] for the record boundary rule.
fn warn_records(window: &str) -> Vec<String> {
    records(window).into_iter().filter(|(level, _)| *level == "WARN").map(|(_, text)| text).collect()
}

/// Is there a WARN or ERROR record in `window` that mentions `needle`?
///
/// "Mentions" is deliberately textual: the case that uses this asserts on the
/// LEVEL and on the subject of the record, not on the fix's wording.
pub(crate) fn warn_or_error_record_mentioning(window: &str, needle: &str) -> bool {
    records(window).iter().any(|(level, text)| matches!(*level, "WARN" | "ERROR") && text.contains(needle))
}

// ---------------------------------------------------------------------------
// Cases: egress side
// ---------------------------------------------------------------------------

/// C2 — a mapping that collapses to the EMPTY topic must not reach the peer,
/// and its drop must be attributable from the broker's own log.
///
/// `remote.topic = "${local.topic}"` with `remote.skip_levels = 8` against a
/// three-level local topic: `Topic::to_string_skip()` returns `""` and the
/// substitution keeps it, so the mapping resolves to a PUBLISH with no topic
/// name at all (MQTT 5.0 §4.7.3 [MQTT-4.7.3-1]).
///
/// Unlike a literal of filter shape (rejected while the config is loaded, see
/// `Remote::validate`), this configuration is perfectly legal — the skeleton
/// `${local.topic}` passes the load-time check, and the same entry still maps a
/// deeper topic correctly — so the defect can only be caught at send time. The
/// case therefore asserts the runtime guard in `send()`:
///
/// 1. the peer NEVER receives a PUBLISH for that entry (the observable half of
///    issue #451: no illegal topic name reaches the wire);
/// 2. the broker log gains exactly ONE WARN naming that entry (the
///    attributable half: an operator can tell where the message went, and the
///    warn-once rule keeps a chatty publisher from flooding the log).
///
/// The control arm (E4) runs first and must still deliver, so "the peer
/// received nothing" can never be a broken chain in disguise.
async fn c2_empty_after_skip_levels() -> anyhow::Result<()> {
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    // Wait for BOTH connections this case drives: the bridge installs its
    // publish sink only after it has processed our CONNACK, and a PUBLISH that
    // arrives before that is dropped by the bridge itself — which would make a
    // "nothing arrived" assertion meaningless.
    downstream.wait_client_id(&format!(":egress:1:{EGRESS_ENTRY_SKIP_LEVELS}:0"), BRIDGE_WAIT).await?;
    downstream.wait_client_id(&format!(":egress:1:{EGRESS_ENTRY_CONTROL}:0"), BRIDGE_WAIT).await?;
    tokio::time::sleep(CONNECT_SETTLE).await;

    // Baseline before this case publishes anything: the log is shared with the
    // other cases of the sub-suite, and the WARN below is counted from here.
    let baseline = log_len();

    // Control arm: the same broker, driven the same way, forwards a legal
    // mapping. Everything below is relative to this having worked.
    let control_marker = format!("issue451-c2-control-{}", nonce());
    let control =
        forward_locally(&downstream, EGRESS_ENTRY_CONTROL, LOCAL_TOPIC_CONTROL, &control_marker).await?;
    if control.topic != CONTROL_FORWARDED {
        return Err(anyhow::anyhow!(
            "control arm: expected the legal mapping to forward {CONTROL_FORWARDED:?}, got {:?}",
            control.topic
        ));
    }

    // Drive the illegal mapping several times: the first send emits the WARN,
    // every later one must be demoted to debug (assertion 2 counts them).
    const LOCAL: &str = "eg/skip/local";
    let marker = format!("issue451-c2-{}", nonce());
    let client = LocalClient::connect(&marker).await?;
    for _ in 0..DRIVE_ATTEMPTS {
        client.publish(LOCAL, marker.as_bytes()).await?;
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    client.disconnect().await;

    // Let the bridge drain its queue and flush the log before judging.
    tokio::time::sleep(RECV_WAIT).await;

    // 1) nothing illegal reached the peer.
    let offending: Vec<Captured> = downstream
        .captured_snapshot()
        .into_iter()
        .filter(|c| c.conn_client_id.contains(&format!(":egress:1:{EGRESS_ENTRY_SKIP_LEVELS}:")))
        .collect();
    if let Some(first) = offending.first() {
        return Err(anyhow::anyhow!(
            "the egress bridge still forwarded {} PUBLISH(es) for the illegal mapping \
             (remote.topic = \"${{local.topic}}\" with skip_levels = 8 against a three-level \
             local topic, i.e. the empty topic name) — the defect of issue #451 is still on the \
             wire. First: topic={:?}, payload={:?}",
            offending.len(),
            first.topic,
            String::from_utf8_lossy(&first.payload),
        ));
    }

    // 2) the drop is attributable, exactly once.
    let warns: Vec<String> = warn_records(&log_since(baseline))
        .into_iter()
        .filter(|r| r.contains(&format!("entry {EGRESS_ENTRY_SKIP_LEVELS}")))
        .collect();
    if warns.len() != 1 {
        return Err(anyhow::anyhow!(
            "expected exactly 1 WARN naming entry {EGRESS_ENTRY_SKIP_LEVELS} after {DRIVE_ATTEMPTS} \
             illegal sends (warn once, then debug), got {}: {warns:#?}\nLog tail:\n{}",
            warns.len(),
            log_tail(20)
        ));
    }
    tracing::info!("bridge_egress_mqtt: diagnosable warning found: {}", warns[0].trim());
    Ok(())
}

/// The egress happy path, asserted on its own rather than as the control arm
/// of C2: a plain local publish must reach the peer, on the mapped topic, with
/// the payload intact.
///
/// It exists because C2's control arm is otherwise the ONLY place asserting a
/// successful forward — and a control arm proves an ARM, it does not guard a
/// behaviour. If C2 is ever rewritten or retired, "local publish -> peer" would
/// silently lose its only coverage while the suite stayed green.
///
/// It also pins the rest of the happy-path contract:
///
/// * the local publish is QoS 0 and carries no properties of its own, so the
///   forwarded PUBLISH must carry no Topic Alias and no Subscription Identifier
///   — seen on a REAL delivery, not only in the C3 injection;
/// * the topic must be exactly `mirror/control/${local.topic}` (entry E4).
async fn egress_forward_local_publish() -> anyhow::Result<()> {
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    let marker = format!("issue451-egress-happy-{}", nonce());
    let captured = forward_locally(&downstream, EGRESS_ENTRY_CONTROL, LOCAL_TOPIC_CONTROL, &marker).await?;

    assert_legal_topic(&captured, "egress remote.topic = \"mirror/control/${local.topic}\"")?;

    if captured.topic != CONTROL_FORWARDED {
        return Err(anyhow::anyhow!(
            "the egress bridge forwarded the local publish on the wrong topic: expected \
             {CONTROL_FORWARDED:?} (remote.topic = \"mirror/control/${{local.topic}}\"), got {:?}",
            captured.topic
        ));
    }

    // Payload fidelity: the peer must receive the very bytes the local
    // publisher sent, not a re-encoded or truncated copy.
    if &captured.payload[..] != marker.as_bytes() {
        return Err(anyhow::anyhow!(
            "payload altered in flight: expected {:?}, got {:?}",
            marker.as_bytes(),
            String::from_utf8_lossy(&captured.payload)
        ));
    }

    // The happy path must not smuggle the connection-scoped properties either.
    // A plain local publish has an empty block, so their presence on the wire
    // could only have been invented by the bridge.
    if let Some(alias) = captured.properties.topic_alias {
        return Err(anyhow::anyhow!(
            "the forwarded PUBLISH of a plain local publish carries Topic Alias {alias}; the alias \
             space is per-connection (MQTT 5.0 §3.3.2.3.4) and the local publish had none"
        ));
    }
    if !captured.properties.subscription_ids.is_empty() {
        return Err(anyhow::anyhow!(
            "the forwarded PUBLISH carries Subscription Identifier(s) {:?}; they are a \
             Server-to-Client property (MQTT 5.0 §3.3.2.3.8) and are illegal on the \
             client->server PUBLISH the bridge sent",
            captured.properties.subscription_ids
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Cases: ingress side (the `local.topic` mapping shapes)
// ---------------------------------------------------------------------------

/// Ingress shape (a): `local.topic = "${remote.topic}"` — transparent.
///
/// This is the shape the maintainer's own reproduction used, and the reason it
/// could not reproduce issue #451: the local topic is always a real topic name.
/// It is asserted as a POSITIVE control: the whole chain must deliver, and the
/// forwarded topic must equal the injected one with the carrier's prefix.
async fn ingress_placeholder_transparent() -> anyhow::Result<()> {
    let upstream = MockBroker::spawn("upstream", UPSTREAM_PORT).await?;
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    let marker = format!("issue451-i1-{}", nonce());
    let captured = drive_ingress(
        &upstream,
        &downstream,
        INGRESS_FILTER_TRANSPARENT,
        EGRESS_ENTRY_TRANSPARENT,
        INGRESS_REMOTE_TOPIC_TRANSPARENT,
        &marker,
        PublishProperties::default(),
    )
    .await?;

    assert_legal_topic(&captured, "ingress local.topic = \"${remote.topic}\"")?;
    let expected = "mirror/transparent/up/transparent/msg";
    if captured.topic != expected {
        return Err(anyhow::anyhow!(
            "transparent mapping must keep the topic: expected {expected:?}, got {:?}",
            captured.topic
        ));
    }
    Ok(())
}

/// Ingress shape (b): `local.topic = "ing/plain"` — a literal without wildcard.
///
/// Legal, though it collapses every remote topic onto one local name. The
/// assertion pins the collapse (the egress carrier E5 maps `ing/#`), so the
/// shape is documented rather than assumed.
async fn ingress_literal_plain() -> anyhow::Result<()> {
    let upstream = MockBroker::spawn("upstream", UPSTREAM_PORT).await?;
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    let marker = format!("issue451-i2-{}", nonce());
    let captured = drive_ingress(
        &upstream,
        &downstream,
        INGRESS_FILTER_PLAIN,
        EGRESS_ENTRY_INGRESS_CARRIER,
        "up/plain/msg",
        &marker,
        PublishProperties::default(),
    )
    .await?;

    assert_legal_topic(&captured, "ingress local.topic = \"ing/plain\" (literal)")?;
    let expected = "mirror/ing/ing/plain";
    if captured.topic != expected {
        return Err(anyhow::anyhow!(
            "a literal local topic must be forwarded verbatim: expected {expected:?}, got {:?}",
            captured.topic
        ));
    }
    Ok(())
}

/// Ingress shape (d): `local.topic = "${remote.topic}/suffix"` — a placeholder
/// with a literal suffix.
///
/// Still a legal topic name; asserted so the enumeration covers the "mixed"
/// shape and not only the two extremes.
async fn ingress_with_suffix() -> anyhow::Result<()> {
    let upstream = MockBroker::spawn("upstream", UPSTREAM_PORT).await?;
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    let marker = format!("issue451-i4-{}", nonce());
    let captured = drive_ingress(
        &upstream,
        &downstream,
        INGRESS_FILTER_SUFFIX,
        EGRESS_ENTRY_SUFFIX,
        "up/suffix/msg",
        &marker,
        PublishProperties::default(),
    )
    .await?;

    assert_legal_topic(&captured, "ingress local.topic = \"${remote.topic}/suffix\"")?;
    let expected = "mirror/suffix/up/suffix/msg/suffix";
    if captured.topic != expected {
        return Err(anyhow::anyhow!(
            "placeholder plus suffix must append to the received topic: expected {expected:?}, got {:?}",
            captured.topic
        ));
    }
    Ok(())
}

/// Ingress delivers into the LOCAL broker — observed at a plain local
/// subscriber, not at the egress carrier.
///
/// Every ingress case above judges the mapping by what the EGRESS bridge
/// forwarded. That proves the chain works, but not WHERE it worked: if such a
/// case goes red, the failure is not attributable to the ingress mapping, to
/// the local routing of its result, or to the egress bridge. And "the message
/// reached a plain subscriber of the broker" — the whole point of an ingress
/// bridge — is never asserted directly.
///
/// This case closes that gap: a plain local v5 client subscribes to the I1
/// filter, the upstream injects one PUBLISH, and the subscriber must receive it
/// with the topic preserved (I1 is `local.topic = "${remote.topic}"`).
///
/// The downstream mock is started but never asserted on, so that the egress
/// bridges have somewhere to publish: without it every egress entry would
/// reconnect in a loop and flood the very log C2 reads.
async fn ingress_delivers_to_local_subscriber() -> anyhow::Result<()> {
    let upstream = MockBroker::spawn("upstream", UPSTREAM_PORT).await?;
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    let mut observer = LocalClient::connect("ingress-local-observer").await?;
    observer.subscribe(INGRESS_FILTER_TRANSPARENT, QoS::AtMostOnce).await?;

    let up_conn = upstream.wait_subscription(INGRESS_FILTER_TRANSPARENT, BRIDGE_WAIT).await?;
    tokio::time::sleep(CONNECT_SETTLE).await;

    let marker = format!("issue451-ingress-local-{}", nonce());
    let mut received = None;
    for attempt in 1..=DRIVE_ATTEMPTS {
        upstream.send_publish(
            up_conn,
            INGRESS_REMOTE_TOPIC_TRANSPARENT,
            marker.as_bytes(),
            PublishProperties::default(),
        )?;
        received = observer.wait_delivery(marker.as_bytes(), RECV_WAIT).await;
        if received.is_some() {
            break;
        }
        tracing::debug!(
            "bridge_egress_mqtt: ingress->local delivery {attempt}/{DRIVE_ATTEMPTS} not seen yet"
        );
    }
    // Diagnosis, failure path only: it runs after the window under test has
    // closed, so it cannot perturb what is being measured. It splits the chain
    // in two, which the bare assertion cannot:
    //
    // * egress echo — entry E8 (`local.topic_filter = "up/transparent/#"`)
    //   forwards whatever the ingress published into the local broker to the
    //   DOWNSTREAM mock. Seeing the echo proves the ingress did publish locally
    //   and moves the suspicion to the local subscriber's subscription.
    // * local control — a PUBLISH from the subscriber's own client on the very
    //   same topic. If that does not arrive either, the local subscription or
    //   the route to it is broken; if it does, only the ingress→local hop is.
    let mut diag = String::new();
    if received.is_none() {
        let echo: Vec<String> = downstream.captured_snapshot().into_iter().map(|c| c.topic).collect();
        let control_marker = format!("issue451-ingress-control-{}", nonce());
        let published =
            observer.publish(INGRESS_REMOTE_TOPIC_TRANSPARENT, control_marker.as_bytes()).await.is_ok();
        let control_seen = published
            && observer.wait_delivery(control_marker.as_bytes(), Duration::from_secs(2)).await.is_some();
        diag = format!(
            " [diag] egress echo captured on the downstream: {echo:?} ({}); a local control PUBLISH \
             on the same topic, by the subscriber's own client, was delivered: {control_seen}",
            downstream.no_forward_detail()
        );
    }
    observer.disconnect().await;

    let message = received.ok_or_else(|| {
        anyhow::anyhow!(
            "the ingress bridge never delivered {INGRESS_REMOTE_TOPIC_TRANSPARENT:?} to a plain \
             LOCAL subscriber on {INGRESS_FILTER_TRANSPARENT:?} ({DRIVE_ATTEMPTS} injections, \
             {RECV_WAIT:?} each). The subscriber was already granted its SUBACK, so either the \
             mapping or the local routing is at fault. Upstream connections the mock saw: {:?}.{diag}",
            upstream.conn_snapshot()
        )
    })?;

    if &*message.topic != INGRESS_REMOTE_TOPIC_TRANSPARENT {
        return Err(anyhow::anyhow!(
            "ingress local.topic = \"${{remote.topic}}\" must deliver the received topic verbatim: \
             expected {INGRESS_REMOTE_TOPIC_TRANSPARENT:?}, got {:?}",
            &*message.topic
        ));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Cases: the topic FILTER field of an entry (issue #540)
// ---------------------------------------------------------------------------

/// Ingress `$share/<group>/<filter>` must reach the peer VERBATIM (issue #540).
///
/// `$share` is meaningful in the filter a bridge sends to the PEER: the peer is
/// the one that load-balances across the bridge's clients, and `bridge.rs` reads
/// the prefix to decide how many clients to open. A topic-filter validator must
/// therefore accept the three-part form — `$share`, a non-empty ShareName, then
/// the Topic Filter ([MQTT-4.8.2-1/2]) — instead of rejecting everything that is
/// not a plain filter.
///
/// This is the guard against over-rejection, and the reason `$share//up/a`
/// (empty ShareName) is a separate negative case: the prefix has to be handled
/// explicitly rather than waved through by the generic parser.
///
/// The assertion is exact-string equality on what the mock received, so a fix
/// that rewrites the filter on its way out (dropping the prefix, say) fails
/// here. If a fix started rejecting `$share` in `remote.topic` altogether, the
/// fixture broker itself could no longer start — a loud failure for the whole
/// suite, which is the correct signal for that mistake.
async fn ingress_share_filter_subscribed_verbatim() -> anyhow::Result<()> {
    let upstream = MockBroker::spawn("upstream", UPSTREAM_PORT).await?;
    // The egress entries need a peer to publish to: without one every egress
    // bridge reconnects in a loop and floods the log.
    let _downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    upstream.wait_subscription(INGRESS_FILTER_SHARE, BRIDGE_WAIT).await?;
    Ok(())
}

/// A subscription the PEER refuses must be attributable and must not be retried
/// as if the failure were transient (issue #540).
///
/// Issue #451 fixed what leaves the broker; this is the other direction. A peer
/// is entitled to answer a SUBSCRIBE with `TopicFilterInvalid` (0x8F) — an ACL
/// it does not disclose, a filter it does not implement — and the ingress plugin
/// then does exactly what it does for a transient error: logs and sleeps 5 s,
/// forever. The entry delivers nothing for the lifetime of the process, and the
/// only trace is one line per 5 s at a level an operator may have filtered out.
///
/// Two answers are acceptable: give up, or back off. What is not acceptable is a
/// fixed 5 s cadence, so the case allows at most [`MAX_REFUSAL_RETRIES`] further
/// attempt inside [`RETRY_WINDOW`] (12 s holds exactly two retries on the
/// current code, and one or zero under any bounded policy).
///
/// Control arms, in order:
///
/// 1. the mock's policy is SELECTIVE — I2 refused, I1 granted — so "few
///    attempts" cannot hold because the bridge never subscribed at all;
/// 2. the refusal is already reported with its reason before the window opens
///    (the half of the issue's ask that holds today), which proves the SUBACK
///    was received and processed rather than dropped.
async fn ingress_refused_subscription_is_bounded() -> anyhow::Result<()> {
    // Baseline taken before any connection can carry the refusal we count.
    let baseline = log_len();

    let upstream = MockBroker::spawn_refusing("upstream", UPSTREAM_PORT, &[INGRESS_FILTER_PLAIN]).await?;
    let _downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    // Control arm 1: both filters, with opposite outcomes, on the same peer.
    let deadline = Instant::now() + BRIDGE_WAIT;
    loop {
        if upstream.refusal_of(INGRESS_FILTER_PLAIN) == Some(true)
            && upstream.refusal_of(INGRESS_FILTER_TRANSPARENT) == Some(false)
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!(
                "control arm 1: within {BRIDGE_WAIT:?} the mock did not see {INGRESS_FILTER_PLAIN:?} \
                 refused AND {INGRESS_FILTER_TRANSPARENT:?} granted (refused={:?}, granted={:?}); \
                 connections the mock saw: {:?}",
                upstream.refusal_of(INGRESS_FILTER_PLAIN),
                upstream.refusal_of(INGRESS_FILTER_TRANSPARENT),
                upstream.conn_snapshot()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Control arm 2: the refusal is on the record, at a level that survives
    // `log.level = "info"`, and it names the filter.
    if !warn_records(&log_since(baseline)).iter().any(|r| r.contains(INGRESS_FILTER_PLAIN)) {
        return Err(anyhow::anyhow!(
            "control arm 2: the mock answered SUBACK TopicFilterInvalid for {INGRESS_FILTER_PLAIN:?}, \
             but no WARN record naming that filter appeared in the broker log — the refusal is \
             invisible to an operator. Log tail:\n{}",
            log_tail(20)
        ));
    }

    let after_first = upstream.subscribe_attempts(INGRESS_FILTER_PLAIN);
    tokio::time::sleep(RETRY_WINDOW).await;
    let retries = upstream.subscribe_attempts(INGRESS_FILTER_PLAIN).saturating_sub(after_first);

    if retries > MAX_REFUSAL_RETRIES {
        return Err(anyhow::anyhow!(
            "the peer refused the subscription to {INGRESS_FILTER_PLAIN:?} and the bridge retried it \
             {retries} more time(s) within {RETRY_WINDOW:?} (allowed: {MAX_REFUSAL_RETRIES}): a \
             permanent failure is being retried on a fixed 5 s cadence that hides it from the \
             operator, while the entry delivers nothing — see the `'subscribe: loop` of \
             rmqtt-bridge-ingress-mqtt/src/v5.rs (and src/v4.rs for MQTT 3.1.1, covered separately \
             by the bridge_config_reject suite)"
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Case: v5 properties (C3)
// ---------------------------------------------------------------------------

/// C3 — v5 properties that cannot cross a connection are stripped.
///
/// Both bridges translate a whole property block (`to_properties()` in
/// `ingress-mqtt/src/bridge.rs` and `egress-mqtt/src/bridge.rs`), and two of
/// the properties must not be copied across such a hop:
///
/// * **Topic Alias** is per-connection state (MQTT 5.0 §3.3.2.3.4): the alias
///   the upstream connection assigned has no meaning on the downstream one.
/// * **Subscription Identifier** is a Server-to-Client property (§3.3.2.3.8):
///   it is illegal on the client-to-server PUBLISH the bridge sends, and the
///   id space of the remote subscription is meaningless locally anyway.
///
/// `to_properties()` therefore sets both to `None` / empty on either side —
/// the ingress one so the peer's alias and subscription ids never enter the
/// local broker, the egress one so nothing invalid reaches the wire. This case
/// is the end-to-end half of that guard (the unit tests next to each
/// `to_properties()` are the other half).
///
/// It injects both (plus a user property) upstream, then requires the
/// forwarded PUBLISH to carry neither. The user property is the control: it
/// MUST come through, which proves the mock really decodes the property block
/// and that a pass is not "the properties were dropped wholesale".
async fn c3_cross_connection_properties() -> anyhow::Result<()> {
    let upstream = MockBroker::spawn("upstream", UPSTREAM_PORT).await?;
    let downstream = MockBroker::spawn("downstream", DOWNSTREAM_PORT).await?;

    let properties = PublishProperties {
        topic_alias: NonZeroU16::new(1),
        subscription_ids: vec![NonZeroU32::new(7).expect("7 != 0")],
        user_properties: vec![(ByteString::from("x-bridge-marker"), ByteString::from("forward-me"))],
        ..Default::default()
    };

    let marker = format!("issue451-c3-{}", nonce());
    let captured = drive_ingress(
        &upstream,
        &downstream,
        INGRESS_FILTER_PROPS,
        EGRESS_ENTRY_PROPS,
        "eg/props/c3",
        &marker,
        properties,
    )
    .await?;

    let mut problems = Vec::new();
    if let Some(alias) = captured.properties.topic_alias {
        problems.push(format!(
            "Topic Alias {alias} was copied out of the upstream connection: the alias space is \
             per-connection (MQTT 5.0 §3.3.2.3.4), so the peer resolves it against its own \
             mapping — or rejects it"
        ));
    }
    if !captured.properties.subscription_ids.is_empty() {
        problems.push(format!(
            "Subscription Identifier(s) {:?} were copied out: they are a Server-to-Client \
             property (MQTT 5.0 §3.3.2.3.8) and are illegal on the client->server PUBLISH the \
             bridge sent",
            captured.properties.subscription_ids
        ));
    }

    // Control: legitimate passthrough must still work, otherwise "both are
    // absent" would also hold if the whole property block were dropped.
    let marker_forwarded = captured
        .properties
        .user_properties
        .iter()
        .any(|(k, v)| &**k == "x-bridge-marker" && &**v == "forward-me");
    if !marker_forwarded {
        return Err(anyhow::anyhow!(
            "control arm: the user property x-bridge-marker=forward-me did not survive the chain, \
             so the property assertions below would be vacuous. Forwarded properties: {:?}",
            captured.properties
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "the egress bridge forwarded per-connection / wrong-direction v5 properties to the peer: \
             {}. Forwarded properties: {:?}",
            problems.join("; "),
            captured.properties
        ))
    }
}

// ---------------------------------------------------------------------------
// Registered cases
// ---------------------------------------------------------------------------

/// Declares a case over an async body: it is pinned to the fixture config and
/// port, and runs its body on a large-stack thread (see [`run_case`]).
macro_rules! bridge_case {
    ($ty:ident, $case:literal, $body:ident) => {
        pub struct $ty;

        impl TestCase for $ty {
            fn name(&self) -> &str {
                $case
            }

            fn execute(&self, _ctx: &mut TestContext) -> TestResult {
                run_case(self.name(), || Box::pin($body()))
            }

            fn broker_config(&self) -> Option<PathBuf> {
                Some(prepared_config())
            }

            fn broker_addr(&self) -> Option<&'static str> {
                Some(BROKER_ADDR)
            }

            fn timeout(&self) -> Duration {
                // Broker switch + bridge attach (BRIDGE_WAIT) + a control arm +
                // the case itself, with a wide margin.
                Duration::from_secs(120)
            }
        }
    };
}

bridge_case!(
    Issue451EgressEmptyTopicAfterSkipLevelsTest,
    "issue451_egress_empty_topic_after_skip_levels",
    c2_empty_after_skip_levels
);
bridge_case!(
    Issue451EgressForwardLocalPublishTest,
    "issue451_egress_forward_local_publish",
    egress_forward_local_publish
);
bridge_case!(
    Issue451IngressLocalTopicPlaceholderTransparentTest,
    "issue451_ingress_local_topic_placeholder_transparent",
    ingress_placeholder_transparent
);
bridge_case!(
    Issue451IngressLocalTopicLiteralPlainTest,
    "issue451_ingress_local_topic_literal_plain",
    ingress_literal_plain
);
bridge_case!(
    Issue451IngressLocalTopicWithSuffixTest,
    "issue451_ingress_local_topic_with_suffix",
    ingress_with_suffix
);
bridge_case!(
    Issue451IngressDeliversToLocalSubscriberTest,
    "issue451_ingress_delivers_to_local_subscriber",
    ingress_delivers_to_local_subscriber
);
bridge_case!(
    Issue451EgressV5CrossConnectionPropertiesTest,
    "issue451_egress_v5_cross_connection_properties",
    c3_cross_connection_properties
);
bridge_case!(
    Issue540IngressShareFilterSubscribedVerbatimTest,
    "issue540_ingress_share_filter_subscribed_verbatim",
    ingress_share_filter_subscribed_verbatim
);
bridge_case!(
    Issue540IngressRefusedSubscriptionIsBoundedTest,
    "issue540_ingress_refused_subscription_is_bounded",
    ingress_refused_subscription_is_bounded
);
