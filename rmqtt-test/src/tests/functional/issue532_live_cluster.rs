//! Issue #532 — the incident at **scale**: 100 000 *live* sessions against a
//! three-node broadcast cluster.
//!
//! Companion of [`super::issue532_cluster_connect_ramp_stall`] and
//! [`super::issue532_tls_cluster_connect_ramp_stall`], and deliberately a
//! separate case rather than another flag on them, because it differs in the
//! one dimension those two cannot reach:
//!
//! * they ramp **connect → CONNACK → RST**, so the steady state is ~400
//!   *in-flight* handshakes and no session ever lives;
//! * the reported incident ramped to **100 000 concurrent sessions** at about
//!   500 conn/s and kept them: every one holds a session, a router entry, a
//!   cluster-route registration and a keepalive timer on its node. That is the
//!   scale the incident was observed at, and it is exactly what the other two
//!   variants never touch.
//!
//! # Why this case runs under Docker (and skips on Windows)
//!
//! 100 000 held sockets need a client `nofile` limit of a few hundred thousand,
//! several GB of address space and a **clean CPU/memory budget for each broker**
//! (the incident's nodes were resource-limited pods) — none of which is
//! reproducible in a Windows dev session. So the case **skips on Windows** and
//! the supported entry point is
//!
//! ```text
//! rmqtt-test/docker/issue532-live/run.sh        # docker compose, Linux only
//! ```
//!
//! The compose stack owns the three brokers; this case owns the load. It never
//! spawns a broker (`--no-broker`) and never reads a fixture file — node
//! addresses come from the environment, so the same binary works from a host
//! or from inside the compose network.
//!
//! # Environment
//!
//! * `RMQTT_LIVE_NODES` — one entry per cluster node, `addr[|addr…]`, where the
//!   extra addresses are additional MQTT listeners of the **same** node:
//!   `127.0.0.1:1920|127.0.0.1:1921,127.0.0.1:1922|127.0.0.1:1923,…`.
//!   The first address of each node is what the watchdog probes and what the
//!   fan-out uses.
//! * `RMQTT_LIVE_TOTAL` — live sessions to reach (default 100 000).
//! * `RMQTT_LIVE_RAMP_RATE` — cluster-wide connect rate (default 500 /s).
//! * `RMQTT_LIVE_HOLD_SECS` — how long to hold them after the ramp (default 60).
//!
//! The extra listeners are not decoration: one client host can hold only about
//! 28 k **concurrent** sockets per `(destination IP, port)` pair, because the
//! source port comes from `ip_local_port_range`. 100 k spread over three nodes
//! lands right on that ceiling, so the fixture gives every node a second
//! listener and the load uses six pairs (~169 k of headroom). A run that hits
//! the ceiling anyway is reported as an **Error**, never as a reproduction —
//! see [`classify`].
//!
//! # Detector
//!
//! Same criterion as the sibling cluster cases, per node: `STALL_PROBES`
//! consecutive failed CONNECT → CONNACK probes against that node, that node's
//! accept counter frozen over the same window, and it had accepted before. On
//! top of that the case records the **live-session curve** (peak / median /
//! last) and how many sessions the broker dropped at a keepalive write, because
//! a collapsing live count is the other way this scale can fail and the hard
//! criterion alone would not see it.

use std::io::ErrorKind;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use bytestring::ByteString;
use rmqtt_codec::v5::{Codec as V5Codec, Connect, ConnectAckReason, Packet as PacketV5};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Semaphore};
use tokio::time;
use tokio_util::codec::{Decoder, Encoder};

use crate::framework::context::TestContext;
use crate::framework::testcase::{Expectation, TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;
use crate::tests::functional::issue532_connect_ramp_stall::{rate_sampler, Counters, RateStats};

/// Reported under the self-managed cluster suite, like its siblings.
const SUITE: &str = "functional_v5_cluster";
/// Case name; `-t issue532_live` isolates it.
const CASE: &str = "issue532_live_cluster_connect_ramp_stall";

// ---------------------------------------------------------------------------
// Load shape
// ---------------------------------------------------------------------------

/// MQTT keepalive advertised by every held session.
const KEEP_ALIVE: u16 = 60;
/// PINGREQ cadence — well inside `1.5 × KEEP_ALIVE`, so the broker never drops
/// a held session for silence. A session that disappears here disappeared for
/// some *other* reason, which is the point.
const PING_INTERVAL: Duration = Duration::from_secs(20);
/// Budget for one connect + CONNACK.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Connects allowed in flight at once: enough to sustain the programmed rate,
/// few enough that the generator is not the bottleneck.
const IN_FLIGHT: usize = 256;
/// Watchdog cadence and budget.
const PROBE_INTERVAL: Duration = Duration::from_millis(500);
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Consecutive failed probes against one node that count as a stall.
const STALL_PROBES: u32 = 4;
/// Live-session curve sampling period.
const SAMPLE_INTERVAL: Duration = Duration::from_millis(500);
/// Wait for the compose brokers before starting the ramp.
const BRINGUP_TIMEOUT: Duration = Duration::from_secs(60);
/// Long-lived QoS 0 publishers per node, publishing back-to-back, so the
/// cluster fan-out runs alongside the sessions (the incident's amplification).
const PUBLISHERS_PER_NODE: usize = 8;
const FANOUT_TOPIC: &str = "issue532/fanout";
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
/// ClientId prefix, so the load is recognisable in a broker log.
const CLIENT_PREFIX: &str = "i532live";

/// One node of the stack: its addresses, primary first.
type NodeAddrs = Vec<String>;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default)
}

/// Parses `RMQTT_LIVE_NODES`. `None` means "not configured", which the case
/// reports as a skip rather than a failure.
fn nodes_from_env() -> Option<Vec<NodeAddrs>> {
    let raw = std::env::var("RMQTT_LIVE_NODES").ok()?;
    let nodes: Vec<NodeAddrs> = raw
        .split(',')
        .map(|n| n.split('|').map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect())
        .filter(|addrs: &NodeAddrs| !addrs.is_empty())
        .collect();
    (!nodes.is_empty()).then_some(nodes)
}

/// Human-readable reason a connect failed.
///
/// Linux errnos on purpose — the case is Linux-only. Reporting "the load
/// generator ran out of resources" as a *reproduction* of issue #532 would be
/// the worst possible failure mode here, so those cases are named explicitly.
fn classify(e: &std::io::Error) -> &'static str {
    match (e.kind(), e.raw_os_error()) {
        (_, Some(99)) | (ErrorKind::AddrNotAvailable, _) => "source ports exhausted (EADDRNOTAVAIL)",
        (_, Some(24)) => "client fd limit (EMFILE)",
        (_, Some(23)) => "system fd limit (ENFILE)",
        (_, Some(111)) | (ErrorKind::ConnectionRefused, _) => "broker not listening (ECONNREFUSED)",
        (_, Some(110)) | (ErrorKind::TimedOut, _) => "connect timeout",
        _ => "other",
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Counters shared by the ramp, the keeper, the watchdog and the fan-out.
struct Shared {
    /// Connect attempts started.
    attempted: AtomicU64,
    /// Sessions that completed CONNECT → CONNACK and went to the keeper.
    accepted: AtomicU64,
    /// Sessions that failed to be established.
    failed: AtomicU64,
    /// Sessions the broker dropped (a keepalive write failed).
    dropped: AtomicU64,
    /// Sessions currently held.
    live: AtomicUsize,
    /// Accepted handshakes per node — the watchdog's per-node progress signal.
    ///
    /// Deliberately *only* the ramp increments these: the fan-out has its own
    /// counters, because a publish reconnect must never look like a handshake
    /// the watchdog can count as progress.
    ramp: Vec<Arc<Counters>>,
    /// Long-lived publisher counters, per node.
    fanout: Vec<Arc<Counters>>,
    /// Connect failures by [`classify`] reason.
    reasons: parking_lot::Mutex<std::collections::BTreeMap<&'static str, u64>>,
    /// Most recent error, for the report.
    last_error: parking_lot::Mutex<String>,
}

impl Shared {
    fn new(nodes: usize) -> Self {
        Self {
            attempted: AtomicU64::new(0),
            accepted: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            live: AtomicUsize::new(0),
            ramp: (0..nodes).map(|_| Arc::new(Counters::default())).collect(),
            fanout: (0..nodes).map(|_| Arc::new(Counters::default())).collect(),
            reasons: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
            last_error: parking_lot::Mutex::new(String::new()),
        }
    }

    fn note(&self, msg: String) {
        *self.last_error.lock() = msg;
    }

    fn failed_with(&self, e: &std::io::Error) {
        self.failed.fetch_add(1, Ordering::Relaxed);
        *self.reasons.lock().entry(classify(e)).or_insert(0) += 1;
        self.note(format!("{e}"));
    }

    fn reason_breakdown(&self) -> String {
        let map = self.reasons.lock();
        if map.is_empty() {
            return "none".to_string();
        }
        map.iter().map(|(k, v)| format!("{k}: {v}")).collect::<Vec<_>>().join("; ")
    }
}

/// One node's stall, as reported.
struct NodeStall {
    node: usize,
    at: Instant,
    window: Duration,
    probes_ok: u64,
    probe_failures: u64,
    last_error: String,
}

/// Everything the verdict needs.
struct Report {
    stalls: Vec<NodeStall>,
    /// (live, accepted) samples over the run.
    curve: Vec<(usize, u64)>,
    /// Per-node accepted-handshake rate profile.
    rates: Vec<Vec<u64>>,
    /// Cross-node deliveries counted by the fan-out subscriber.
    delivered: u64,
    attempted: u64,
    failed: u64,
    dropped: u64,
    reasons: String,
    per_node_accepted: Vec<u64>,
    per_node_published: Vec<u64>,
    last_error: String,
}

/// (peak, median, last) of the live-session curve.
///
/// The final sample is taken while the load is being torn down, so it is not a
/// "live" measurement in the same sense as the rest.
fn live_stats(curve: &[(usize, u64)]) -> (usize, usize, usize) {
    if curve.is_empty() {
        return (0, 0, 0);
    }
    let last = curve.last().map(|(l, _)| *l).unwrap_or(0);
    let mut peaks: Vec<usize> = curve.iter().rev().skip(1).map(|(l, _)| *l).collect();
    peaks.sort_unstable();
    let peak = peaks.last().copied().unwrap_or(last);
    let median = peaks.get(peaks.len() / 2).copied().unwrap_or(peak);
    (peak, median, last)
}

// ---------------------------------------------------------------------------
// Session: connect and hold
// ---------------------------------------------------------------------------

/// Opens one session and returns its write half. The read half is dropped on
/// purpose (the socket stays open while one half lives): 100 k read tasks cost
/// far more than they could tell us, and with no subscription the broker sends
/// nothing but the two-byte PINGRESP.
///
/// `SO_LINGER(0)` again — at teardown 100 k sockets must not become 100 k
/// TIME_WAIT entries.
async fn connect_hold(addr: &str, client_id: &str) -> anyhow::Result<OwnedWriteHalf> {
    let stream = time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;
    socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO))?;

    let (mut reader, mut writer) = stream.into_split();
    let mut codec = V5Codec::new(0, 0);
    let connect = Connect {
        clean_start: true,
        keep_alive: KEEP_ALIVE,
        client_id: ByteString::from(client_id),
        ..Default::default()
    };
    let mut out = BytesMut::with_capacity(128);
    codec.encode(PacketV5::Connect(Box::new(connect)), &mut out)?;
    writer.write_all(&out).await?;
    writer.flush().await?;

    let mut inbuf = BytesMut::with_capacity(1024);
    let packet = time::timeout(CONNECT_TIMEOUT, async {
        loop {
            if let Some((packet, _)) = codec.decode(&mut inbuf)? {
                return Ok::<PacketV5, anyhow::Error>(packet);
            }
            if reader.read_buf(&mut inbuf).await? == 0 {
                return Err(anyhow::anyhow!("broker closed the connection before CONNACK"));
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("no CONNACK within {CONNECT_TIMEOUT:?}"))??;

    match packet {
        PacketV5::ConnectAck(ack) if ack.reason_code == ConnectAckReason::Success => Ok(writer),
        PacketV5::ConnectAck(ack) => Err(anyhow::anyhow!("CONNACK {:?}", ack.reason_code)),
        other => Err(anyhow::anyhow!("expected CONNACK, got {other:?}")),
    }
}

enum Msg {
    Add(usize, OwnedWriteHalf),
    Finish,
}

/// Owns every held session (single-threaded, so no lock on the hot path), keeps
/// them alive with PINGREQ and counts the ones the broker drops.
async fn keeper(mut rx: mpsc::UnboundedReceiver<Msg>, shared: Arc<Shared>, stop: Arc<AtomicBool>) {
    let mut held: Vec<(usize, OwnedWriteHalf)> = Vec::new();
    let ping_req = {
        let mut codec = V5Codec::new(0, 0);
        let mut buf = BytesMut::with_capacity(4);
        let _ = codec.encode(PacketV5::PingRequest, &mut buf);
        buf.freeze()
    };
    let mut ping = time::interval(PING_INTERVAL);
    let mut sample = time::interval(SAMPLE_INTERVAL);

    loop {
        tokio::select! {
            biased;
            msg = rx.recv() => match msg {
                Some(Msg::Add(node, w)) => {
                    held.push((node, w));
                    shared.live.store(held.len(), Ordering::Relaxed);
                }
                Some(Msg::Finish) | None => break,
            },
            _ = ping.tick() => {
                // Sequential on purpose: a held session's send buffer is empty
                // (2 bytes per KEEP_ALIVE), so this cannot block for long — and
                // if it does, the write error is itself evidence.
                let mut dropped = 0u64;
                for (_, w) in held.iter_mut() {
                    if w.write_all(&ping_req).await.is_err() {
                        dropped += 1;
                    }
                }
                if dropped > 0 {
                    shared.dropped.fetch_add(dropped, Ordering::Relaxed);
                    shared.note(format!("broker dropped {dropped} session(s) at a keepalive write"));
                }
            }
            _ = sample.tick() => {
                shared.live.store(held.len(), Ordering::Relaxed);
                if stop.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
    // Dropping the write halves closes every socket (abortively: SO_LINGER(0)).
    shared.live.store(0, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Watchdog and fan-out
// ---------------------------------------------------------------------------

/// Probes every node once per cycle and confirms a stall when one node fails
/// `STALL_PROBES` consecutive probes while its accept counter is frozen.
async fn watchdog(
    shared: Arc<Shared>,
    nodes: Arc<Vec<NodeAddrs>>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
) -> Vec<NodeStall> {
    let n_nodes = nodes.len();
    let mut stalls = Vec::new();
    let mut ok = vec![0u64; n_nodes];
    let mut failed = vec![0u64; n_nodes];
    let mut streak = vec![0u32; n_nodes];
    let mut window_start = vec![Instant::now(); n_nodes];
    let mut accepted_at = vec![0u64; n_nodes];
    let mut last_error = vec![String::new(); n_nodes];
    let mut stalled = vec![false; n_nodes];
    let mut seq = 0u64;

    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        time::sleep(PROBE_INTERVAL).await;
        for (n, addrs) in nodes.iter().enumerate() {
            if stalled[n] {
                continue;
            }
            seq += 1;
            let client_id = format!("{CLIENT_PREFIX}-probe-{seq}");
            let outcome = match time::timeout(PROBE_TIMEOUT, TcpStream::connect(&addrs[0])).await {
                Ok(Ok(stream)) => {
                    stream.set_nodelay(true).ok();
                    socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO)).ok();
                    let (mut r, mut w) = stream.into_split();
                    let probe = async {
                        let mut codec = V5Codec::new(0, 0);
                        let connect = Connect {
                            clean_start: true,
                            keep_alive: KEEP_ALIVE,
                            client_id: ByteString::from(client_id.as_str()),
                            ..Default::default()
                        };
                        let mut out = BytesMut::with_capacity(128);
                        codec.encode(PacketV5::Connect(Box::new(connect)), &mut out)?;
                        w.write_all(&out).await?;
                        w.flush().await?;
                        let mut inbuf = BytesMut::with_capacity(1024);
                        loop {
                            if let Some((packet, _)) = codec.decode(&mut inbuf)? {
                                return Ok::<PacketV5, anyhow::Error>(packet);
                            }
                            if r.read_buf(&mut inbuf).await? == 0 {
                                return Err(anyhow::anyhow!("broker closed before CONNACK"));
                            }
                        }
                    };
                    match time::timeout(PROBE_TIMEOUT, probe).await {
                        Ok(Ok(PacketV5::ConnectAck(ack))) if ack.reason_code == ConnectAckReason::Success => {
                            ok[n] += 1;
                            None
                        }
                        Ok(Ok(PacketV5::ConnectAck(ack))) => Some(format!("CONNACK {:?}", ack.reason_code)),
                        Ok(Ok(other)) => Some(format!("expected CONNACK, got {other:?}")),
                        Ok(Err(e)) => Some(e.to_string()),
                        Err(_) => Some(format!("no CONNACK within {PROBE_TIMEOUT:?}")),
                    }
                }
                Ok(Err(e)) => Some(format!("connect: {e}")),
                Err(_) => Some(format!("connect timed out after {PROBE_TIMEOUT:?}")),
            };

            match outcome {
                None => streak[n] = 0,
                Some(err) => {
                    if streak[n] == 0 {
                        window_start[n] = Instant::now();
                        accepted_at[n] = shared.ramp[n].accepted.load(Ordering::Relaxed);
                    }
                    streak[n] += 1;
                    failed[n] += 1;
                    last_error[n] = err;
                }
            }

            if streak[n] >= STALL_PROBES {
                let frozen =
                    accepted_at[n] > 0 && shared.ramp[n].accepted.load(Ordering::Relaxed) == accepted_at[n];
                if frozen {
                    stalled[n] = true;
                    stalls.push(NodeStall {
                        node: n,
                        at: Instant::now(),
                        window: window_start[n].elapsed(),
                        probes_ok: ok[n],
                        probe_failures: failed[n],
                        last_error: last_error[n].clone(),
                    });
                    stop.store(true, Ordering::Relaxed);
                } else {
                    // Either the node still completes handshakes (slow, not
                    // stalled) or it never completed one (reported as an Error).
                    streak[n] = 0;
                }
            }
        }
    }
    stalls
}

/// One long-lived publisher: every publish fans out to the other two nodes.
async fn publisher(
    addr: String,
    idx: usize,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    counters: Arc<Counters>,
) {
    let client_id = format!("{CLIENT_PREFIX}-pub-{idx}");
    let payload = vec![0x5Au8; 64];
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        let Ok(client) = MqttV5Client::connect(&addr, &client_id, PUBLISH_TIMEOUT).await else {
            counters.errors.fetch_add(1, Ordering::Relaxed);
            time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        counters.accepted.fetch_add(1, Ordering::Relaxed);
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            counters.attempts.fetch_add(1, Ordering::Relaxed);
            if let Err(e) = client.publish(FANOUT_TOPIC, &payload, QoS::AtMostOnce, false).await {
                counters.errors.fetch_add(1, Ordering::Relaxed);
                counters.note(e.to_string());
                break;
            }
        }
        let _ = client.disconnect().await;
    }
}

/// Counts what the fan-out actually delivers across the cluster.
async fn subscriber(addr: String, stop: Arc<AtomicBool>, deadline: Instant, delivered: Arc<AtomicU64>) {
    let client_id = format!("{CLIENT_PREFIX}-sub");
    let Ok(mut client) = MqttV5Client::connect(&addr, &client_id, PUBLISH_TIMEOUT).await else {
        return;
    };
    if client.subscribe(FANOUT_TOPIC, QoS::AtMostOnce).await.is_err() {
        return;
    }
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        if client.recv_message_timeout(Duration::from_millis(100)).await.is_some() {
            delivered.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ---------------------------------------------------------------------------
// The run
// ---------------------------------------------------------------------------

/// Drives the ramp, the hold, the fan-out, the watchdog and the live curve.
async fn run(nodes: Arc<Vec<NodeAddrs>>, total: usize, rate: usize, hold: Duration) -> Report {
    let shared = Arc::new(Shared::new(nodes.len()));
    let stop = Arc::new(AtomicBool::new(false));
    let delivered = Arc::new(AtomicU64::new(0));
    let ramp_secs = (total as f64 / rate.max(1) as f64).ceil() as u64;
    let deadline = Instant::now() + Duration::from_secs(ramp_secs + hold.as_secs() + 60);

    // Keeper first: it owns every held session.
    let (tx, rx) = mpsc::unbounded_channel::<Msg>();
    let keeper_task = tokio::spawn(keeper(rx, shared.clone(), stop.clone()));

    let watchdog_task = tokio::spawn(watchdog(shared.clone(), nodes.clone(), stop.clone(), deadline));

    // Live-session curve.
    let curve = Arc::new(parking_lot::Mutex::new(Vec::<(usize, u64)>::new()));
    let curve_task = {
        let shared = shared.clone();
        let curve = curve.clone();
        let stop = stop.clone();
        tokio::spawn(async move {
            while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                curve
                    .lock()
                    .push((shared.live.load(Ordering::Relaxed), shared.accepted.load(Ordering::Relaxed)));
                time::sleep(SAMPLE_INTERVAL).await;
            }
        })
    };

    // Per-node accept-rate profiles.
    let profiles: Vec<Arc<parking_lot::Mutex<Vec<u64>>>> =
        (0..nodes.len()).map(|_| Arc::new(parking_lot::Mutex::new(Vec::new()))).collect();
    for (n, profile) in profiles.iter().enumerate() {
        tokio::spawn(rate_sampler(deadline, stop.clone(), shared.ramp[n].clone(), profile.clone()));
    }

    // Fan-out load, on the primary address of every node.
    let mut fanout = Vec::new();
    for (n, addrs) in nodes.iter().enumerate() {
        for i in 0..PUBLISHERS_PER_NODE {
            fanout.push(tokio::spawn(publisher(
                addrs[0].clone(),
                n * PUBLISHERS_PER_NODE + i,
                stop.clone(),
                deadline,
                shared.fanout[n].clone(),
            )));
        }
    }
    fanout.push(tokio::spawn(subscriber(
        nodes[nodes.len() - 1][0].clone(),
        stop.clone(),
        deadline,
        delivered.clone(),
    )));

    // Ramp: connection `i` is due at `i / rate`, round-robin over every address
    // of every node (see the module header for why there is more than one).
    let flat: Vec<(usize, String)> =
        nodes.iter().enumerate().flat_map(|(n, addrs)| addrs.iter().map(move |a| (n, a.clone()))).collect();
    let sem = Arc::new(Semaphore::new(IN_FLIGHT));
    let t0 = Instant::now();
    let mut connects = Vec::with_capacity(total.min(4096));
    for i in 0..total {
        let due = t0 + Duration::from_secs_f64(i as f64 / rate.max(1) as f64);
        time::sleep_until(time::Instant::from_std(due)).await;
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(permit) = sem.clone().acquire_owned().await else {
            break;
        };
        let (node, addr) = flat[i % flat.len()].clone();
        let shared = shared.clone();
        let tx = tx.clone();
        let client_id = format!("{CLIENT_PREFIX}-{node}-{i}");
        shared.attempted.fetch_add(1, Ordering::Relaxed);
        shared.ramp[node].attempts.fetch_add(1, Ordering::Relaxed);
        connects.push(tokio::spawn(async move {
            let _permit = permit;
            match connect_hold(&addr, &client_id).await {
                Ok(w) => {
                    shared.accepted.fetch_add(1, Ordering::Relaxed);
                    shared.ramp[node].accepted.fetch_add(1, Ordering::Relaxed);
                    let _ = tx.send(Msg::Add(node, w));
                }
                Err(e) => match e.downcast_ref::<std::io::Error>() {
                    Some(io) => shared.failed_with(io),
                    None => {
                        shared.failed.fetch_add(1, Ordering::Relaxed);
                        *shared.reasons.lock().entry("protocol/timeout").or_insert(0) += 1;
                        shared.note(e.to_string());
                    }
                },
            }
        }));
    }

    // Let the ramp finish (every connect is bounded by CONNECT_TIMEOUT).
    for c in connects {
        let _ = c.await;
    }
    // Hold, then tear down.
    time::sleep(hold).await;
    let _ = tx.send(Msg::Finish);
    let _ = keeper_task.await;
    for t in fanout {
        let _ = t.await;
    }
    let stalls = watchdog_task.await.unwrap_or_default();
    let _ = curve_task.await;

    // Bind the report to a local first: building it directly as the tail
    // expression would keep two mutex guards alive across the function's
    // locals dropping.
    let report = Report {
        stalls,
        curve: curve.lock().clone(),
        rates: profiles.iter().map(|p| p.lock().clone()).collect(),
        delivered: delivered.load(Ordering::Relaxed),
        attempted: shared.attempted.load(Ordering::Relaxed),
        failed: shared.failed.load(Ordering::Relaxed),
        dropped: shared.dropped.load(Ordering::Relaxed),
        reasons: shared.reason_breakdown(),
        per_node_accepted: (0..nodes.len())
            .map(|n| shared.ramp[n].accepted.load(Ordering::Relaxed))
            .collect(),
        per_node_published: (0..nodes.len())
            .map(|n| shared.fanout[n].attempts.load(Ordering::Relaxed))
            .collect(),
        last_error: shared.last_error.lock().clone(),
    };
    report
}

/// The load/health summary both verdicts use.
fn evidence(
    nodes: usize,
    total: usize,
    rate: usize,
    hold: Duration,
    elapsed: Duration,
    r: &Report,
) -> String {
    let (peak, median, last) = live_stats(&r.curve);
    let per_node = (0..nodes)
        .map(|n| {
            format!(
                "node{}: {} accepted, {} published, ramp rate: {}",
                n + 1,
                r.per_node_accepted.get(n).copied().unwrap_or(0),
                r.per_node_published.get(n).copied().unwrap_or(0),
                RateStats::of(r.rates.get(n).map(|v| v.as_slice()).unwrap_or(&[])).describe(),
            )
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "{nodes} nodes; ramp to {total} sessions at {rate}/s (wall {wall:.0}s) then held {hold:?}; \
         live sessions: peak {peak}, median {median}, last {last}; connect attempts {}, failed {}, \
         dropped by broker {}; cross-node deliveries {}; {per_node}; connect failures: {}; last error: {}",
        r.attempted,
        r.failed,
        r.dropped,
        r.delivered,
        r.reasons,
        if r.last_error.is_empty() { "none" } else { r.last_error.as_str() },
        wall = elapsed.as_secs_f64(),
    )
}

// ---------------------------------------------------------------------------
// Case
// ---------------------------------------------------------------------------

/// The 100 K-live-session reproduction of issue #532. Linux/Docker only.
pub struct Issue532LiveClusterTest;

impl TestCase for Issue532LiveClusterTest {
    fn name(&self) -> &str {
        CASE
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        if cfg!(windows) {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "the 100K-live-session variant is Linux-only: it needs a few hundred thousand client \
                 sockets and per-broker CPU/memory budgets, which is what the compose stack provides. Run it \
                 with rmqtt-test/docker/issue532-live/run.sh on Linux. On Windows the other issue #532 cases \
                 (issue532_connect_ramp_stall, issue532_cluster_connect_ramp_stall and their TLS variants) \
                 still run.",
            );
        }
        if ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "requires --no-broker: the three brokers are the compose stack \
                 (rmqtt-test/docker/issue532-live) and this case only generates load. Run: \
                 rmqtt-test/docker/issue532-live/run.sh",
            );
        }
        let Some(nodes) = nodes_from_env() else {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "RMQTT_LIVE_NODES is not set, so there is no cluster to talk to. Start the stack \
                 (rmqtt-test/docker/issue532-live/run.sh), or set it yourself, e.g. RMQTT_LIVE_NODES=\
                 '127.0.0.1:1920|127.0.0.1:1921,127.0.0.1:1922|127.0.0.1:1923,127.0.0.1:1924|127.0.0.1:1925'",
            );
        };
        let total = env_usize("RMQTT_LIVE_TOTAL", 100_000);
        let rate = env_usize("RMQTT_LIVE_RAMP_RATE", 500);
        let hold = Duration::from_secs(env_u64("RMQTT_LIVE_HOLD_SECS", 60));

        // Bring-up: the brokers must be reachable, otherwise a "no handshake
        // ever completed" verdict would be about the setup, not the incident.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let down = rt.block_on(async {
            let up_deadline = Instant::now() + BRINGUP_TIMEOUT;
            let mut down = Vec::new();
            for addrs in nodes.iter() {
                let mut ok = false;
                while Instant::now() < up_deadline {
                    if time::timeout(PROBE_TIMEOUT, TcpStream::connect(&addrs[0])).await.is_ok() {
                        ok = true;
                        break;
                    }
                    time::sleep(Duration::from_millis(200)).await;
                }
                if !ok {
                    down.push(addrs[0].clone());
                }
            }
            down
        });
        if !down.is_empty() {
            return TestResult::error(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "cluster not reachable: {} of {} nodes do not accept connections ({}). Start the stack \
                     with rmqtt-test/docker/issue532-live/run.sh, or check RMQTT_LIVE_NODES.",
                    down.len(),
                    nodes.len(),
                    down.join(", ")
                ),
            );
        }

        let nodes = Arc::new(nodes);
        let report = rt.block_on(run(nodes.clone(), total, rate, hold));
        let elapsed = start.elapsed();
        let ev = evidence(nodes.len(), total, rate, hold, elapsed, &report);
        let (peak, _, _) = live_stats(&report.curve);

        if let Some(stall) = report.stalls.first() {
            let reason = format!(
                "node{} stopped completing handshakes while its process stayed alive (issue #532, \
                 100K-live variant): no CONNACK for {:.2}s ({} probes, {} ok before) and that node accepted \
                 nothing in the same window. {ev}; last probe error: {}",
                stall.node + 1,
                stall.window.as_secs_f64(),
                stall.probe_failures,
                stall.probes_ok,
                stall.last_error,
            );
            tracing::warn!(at = ?stall.at, "{reason}");
            return TestResult::failed(self.name(), SUITE, elapsed, reason);
        }

        // The load must have been built: a run that never reached its target
        // says nothing about the incident, whatever it says about the broker.
        if peak * 10 < total * 9 {
            return TestResult::error(
                self.name(),
                SUITE,
                elapsed,
                format!(
                    "only reached {peak} of {total} live sessions, so this run does not test the incident's \
                     scale — the load generator (not the broker) hit a limit. {ev}"
                ),
            );
        }

        let note =
            format!("no stall: the {peak}-live-session cluster stayed healthy for the whole run; {ev}");
        tracing::info!("{note}");
        TestResult::passed_with_note(self.name(), SUITE, elapsed, &note)
    }

    fn timeout(&self) -> Duration {
        // Bring-up + ramp (total / rate) + hold + drain, with headroom.
        let total = env_usize("RMQTT_LIVE_TOTAL", 100_000);
        let rate = env_usize("RMQTT_LIVE_RAMP_RATE", 500).max(1);
        let hold = env_u64("RMQTT_LIVE_HOLD_SECS", 60);
        Duration::from_secs(BRINGUP_TIMEOUT.as_secs() + total as u64 / rate as u64 + hold + 180)
    }

    /// Assertive, like its siblings: a reproduced stall fails the suite. A run
    /// that cannot build the load is an Error, never a silent green pass.
    fn expectation(&self) -> Expectation {
        Expectation::Normal
    }
}
