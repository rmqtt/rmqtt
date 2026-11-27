//! Issue #532 — a connect ramp that leaves the node unable to complete any
//! further handshake while the process stays alive.
//!
//! Background: issue #532 reports a three-node broadcast cluster whose
//! members stop serving during a 100K-client connect ramp (about 500 conn/s
//! cluster-wide). The first node hangs roughly two minutes in: its log ends,
//! CPU drops to zero, new TLS connections stop completing and the process
//! stays alive; a second node follows minutes later, and the last one stops
//! completing connections as well for the rest of the run. Nothing recovers.
//! The last log lines of a hung node are `handy_grpc::client` warnings, which
//! is what the reproduction must be able to reach — but they are a symptom of
//! a peer that stopped answering, not a cause.
//!
//! What this case reproduces is the *node-level* observable, on a single
//! node, with no cluster in the picture: under a sustained connect ramp the
//! node can stop completing handshakes while the process stays alive, TCP
//! still accepts at the kernel level, and the broker's own log goes quiet.
//! That is exactly the state the reporter's hung nodes were in.
//!
//! What this case deliberately does NOT cover: the cluster fan-out. The
//! incident's peer-side symptoms (duplex transfer failures, responses arriving
//! after the caller gave up, the last node serving nothing) come from the
//! broadcast plugin's synchronous fan-out and are analysed in
//! `designs/issue-532-broadcast-cluster-hang-analysis.md`. That shape is
//! [`super::issue532_cluster_connect_ramp_stall`], and the TLS transport — the
//! one the reporter's deployment used — is covered by
//! [`super::issue532_tls_connect_ramp_stall`] and
//! [`super::issue532_tls_cluster_connect_ramp_stall`]. All four share the ramp,
//! the watchdog, the rate profile and the verdict below, so they differ only in
//! the transport and in whether the cluster is in the picture.
//!
//! ## The detector
//!
//! The probe is a **full CONNECT → CONNACK round trip on a fresh socket**,
//! never a bare TCP connect. `broker_healthy()` and the harness's start-up
//! check are TCP-only (`broker/healthcheck.rs`), and they stay green through
//! this stall — that asymmetry *is* the incident: the pod looked alive.
//!
//! The verdict is "stalled" only when **both** independent observers agree:
//!
//! * the watchdog fails `STALL_PROBES` consecutive probes (each bounded by
//!   `PROBE_TIMEOUT`, so ~12 s of continuous failure against a wedged broker),
//!   and
//! * the ramp's accepted-handshake counter did not move during that window
//!   (so a merely slow broker cannot trip it),
//!
//! and only if the broker had been serving handshakes before that window —
//! a fixture that never completes one is reported as an Error, not as a
//! reproduction (see the `None` arm of `execute`).
//!
//! On a stall the case collects evidence and reports it, and **fails** — the
//! point of the case is to reproduce the incident, so a reproduced stall is a
//! red test until the broker (or the dependency stack) is fixed. A run that
//! completes without a stall passes; see the `expectation` override for the
//! alternative (issue-backed `ExpectedFail`) if a green suite is ever
//! preferred over a red-until-fixed gate.
//!
//! ## Why the fixture is shaped the way it is
//!
//! `configs/issue532-ramp/` carries the full reasoning; the load-bearing
//! choices are the two-worker handshake pool (`max_handshaking_limit = 2`,
//! which makes the per-port `TaskExecQueue` dispatcher park on nearly every
//! connection instead of rarely), `max_connections` far above the ramp's
//! concurrency (so only the worker pool can be the saturated resource),
//! `rmqtt-sys-topic` at a one-second stats interval (the periodic visitor of
//! the same executor) and file logging (the harness never drains rmqttd's
//! pipes, and a frozen log is itself evidence).
//!
//! ## Why connections are closed with RST
//!
//! A handshake needs a fresh TCP connection, and the loopback 4-tuple space
//! (`127.0.0.1:1904` ↔ ~28k ephemeral ports) plus a 60 s TIME_WAIT caps a
//! plain reconnect ramp at roughly 470 conn/s — far below the rate this case
//! needs. Setting `SO_LINGER(0)` makes the client's close abortive (RST), so
//! no TIME_WAIT is created on either side and the ramp runs at the broker's
//! own limit.

use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use bytes::BytesMut;
use bytestring::ByteString;
use rmqtt_codec::v5::{Codec as V5Codec, Connect, ConnectAckReason, Packet as PacketV5};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::ClientConfig;
use tokio_rustls::TlsConnector;
use tokio_util::codec::{Decoder, Encoder};

use crate::framework::context::TestContext;
use crate::framework::testcase::{Expectation, TestCase, TestResult};

/// MQTT address of `configs/issue532-ramp/rmqtt.toml`. The fixture owns its
/// ports instead of sharing the harness-wide `--addr`, so this case
/// advertises it through `broker_addr()` — the harness health-probes 1904
/// while the sub-suite runs — and connects here itself. Must match
/// `listener.tcp.external.addr` in that file.
pub(crate) const BROKER_ADDR: &str = "127.0.0.1:1904";
/// HTTP port of `configs/issue532-ramp/plugins/rmqtt-http-api.toml`. The
/// harness knows nothing about the plugin's own port, so it is pinned here
/// and must match that file.
pub(crate) const HTTP_ADDR: &str = "127.0.0.1:6068";
/// Log sink of the fixture (relative to the repository root, where the
/// harness and the broker run). Read after a stall: a frozen mtime is the
/// "its log ends" symptom of the incident.
const LOG_PATH: &str = "target/rmqtt-test-logs/issue532-ramp.log";
/// Suite the case is reported under; the scheduler splits it into
/// `functional_v5@issue532-ramp` because the case declares its own config.
const SUITE: &str = "functional_v5";

/// Server name the TLS fixtures' certificate is issued for (`rmqtt-bin/rmqtt.pem`
/// carries a `localhost` / `127.0.0.1` SAN, as `transport_tls.rs` documents).
pub(crate) const SERVER_NAME: &str = "localhost";
/// CA the TLS client trusts. The repository's own test CA, the same one
/// `transport_tls.rs` and `configs/default/rmqtt.toml` use — so the TLS variant
/// needs no certificate of its own and its fixtures stay plain `.toml`.
pub(crate) const CA_PEM: &str = "rmqtt-bin/root.pem";

// ---------------------------------------------------------------------------
// Ramp shape
// ---------------------------------------------------------------------------

/// Concurrent connectors. Comfortably above the two handshake workers (so the
/// pool stays saturated for the whole run) and comfortably below
/// `max_connections = 4096` (so the listener cap and the admission gate can
/// never be the reason a handshake fails).
const RAMP_CLIENTS: usize = 256;
/// Grace period before the watchdog starts probing: the first connections pay
/// session creation, which is not the failure under test.
const WARMUP: Duration = Duration::from_secs(2);
/// How long the ramp and the watchdog run. The incident needed ~2 minutes at
/// a much lower per-node rate; the two-worker pool here produces orders of
/// magnitude more park/wake cycles per second, so a minute is generous.
const RAMP_DURATION: Duration = Duration::from_secs(60);
/// Budget for one attempt (TCP connect + CONNECT + CONNACK).
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
/// Watchdog cadence.
const PROBE_INTERVAL: Duration = Duration::from_millis(250);
/// Budget for one watchdog probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Consecutive probe failures that count as a stall. Each failure costs up to
/// `PROBE_TIMEOUT` (a wedged broker answers nothing, so probes fail by
/// timeout), so 6 probes mean roughly **12 s of continuous failure** with no
/// accepted handshake anywhere in the window — the incident's nodes never
/// recovered at all, so this is deliberately far above a "slow moment".
const STALL_PROBES: u32 = 6;
/// Sampling period of the per-second handshake rate profile. The profile is
/// what turns the evidence from "stalled / not stalled" into "how the node
/// behaved": the incident is a collapse to zero, but a *partial* or
/// self-healing dip would show up here and nowhere else.
const RATE_INTERVAL: Duration = Duration::from_secs(1);

/// Distinguishes the watchdog's ClientIds from each other and from the ramp's.
static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Everything that differs between the plain and the TLS variant of this case.
///
/// The ramp, the watchdog, the rate profile and the verdict are shared, so the
/// two variants differ in exactly one axis — the transport — and their results
/// are directly comparable.
pub(crate) struct Endpoint {
    /// MQTT (or MQTTS) listener the ramp connects to.
    pub(crate) addr: &'static str,
    /// HTTP API port, probed as part of the stall evidence.
    pub(crate) http: &'static str,
    /// Broker log sink, read for the "its log ends" evidence.
    pub(crate) log_path: &'static str,
    /// ClientId prefix, so the ramp and its probes are distinguishable in the
    /// broker's own view.
    pub(crate) client_prefix: &'static str,
    /// Fixture directory under `rmqtt-test/configs/`, named in the verdict so
    /// the reader can find the broker config a run used.
    pub(crate) fixture: &'static str,
    /// Plain TCP or TLS.
    pub(crate) transport: Transport,
}

/// Handshake outcomes of the whole run, shared by the ramp workers and read
/// by the watchdog/verdict.
///
/// Shared with the cluster reproduction
/// ([`super::issue532_cluster_connect_ramp_stall`]), which keeps one instance
/// per node — hence `pub(crate)` on the struct and its fields.
#[derive(Default)]
pub(crate) struct Counters {
    pub(crate) attempts: AtomicU64,
    pub(crate) accepted: AtomicU64,
    pub(crate) refused: AtomicU64,
    pub(crate) errors: AtomicU64,
    /// Most recent transport error or unexpected CONNACK, for the report.
    last_error: parking_lot::Mutex<String>,
}

impl Counters {
    #[inline]
    pub(crate) fn note(&self, msg: String) {
        *self.last_error.lock() = msg;
    }

    #[inline]
    fn take_error(&self) -> String {
        let e = self.last_error.lock().clone();
        if e.is_empty() {
            "none".to_string()
        } else {
            e
        }
    }
}

/// A detected stall plus everything the report needs to be actionable.
pub(crate) struct Stall {
    pub(crate) probes_ok: u64,
    pub(crate) probes_failed: u64,
    pub(crate) streak: u32,
    pub(crate) window: Duration,
    pub(crate) at: chrono::DateTime<chrono::Local>,
    pub(crate) http: String,
    pub(crate) last_probe_error: String,
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// How the ramp reaches the broker.
///
/// The incident ran TLS, and TLS is not just "the same handshake, encrypted"
/// here: `rmqtt/src/server.rs::listen_tls` performs the TLS handshake inside a
/// per-connection `tokio::spawn`, i.e. **outside** the per-port
/// `TaskExecQueue` that `max_handshaking_limit` sizes. TLS therefore adds CPU
/// work (rustls + certificate verification) at unbounded concurrency next to
/// the queue's dispatcher task — which is what makes it worth a variant of its
/// own.
#[derive(Clone)]
pub(crate) enum Transport {
    /// Plain TCP (the default in this suite).
    Plain,
    /// TLS, with the client config built once per case rather than once per
    /// connection.
    Tls(Arc<ClientConfig>),
}

/// Builds the TLS client config for the fixtures: trusts `ca_pem`, presents no
/// client certificate (`configs/default/rmqtt.toml` is server-auth only).
pub(crate) fn tls_client_config(ca_pem: &str) -> anyhow::Result<Arc<ClientConfig>> {
    Ok(Arc::new(super::transport_tls::rustls_config_for(ca_pem, None)?))
}

/// Connects with `SO_LINGER(0)`, so that dropping the socket is an abortive
/// close (RST). Without it the ramp burns through the loopback ephemeral port
/// range and every later attempt fails with `EADDRNOTAVAIL` after TIME_WAIT —
/// a test artefact that would look exactly like a broker stall. See the module
/// header.
async fn connect_abortive(addr: &str, timeout: Duration) -> anyhow::Result<TcpStream> {
    let stream = time::timeout(timeout, TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;
    socket2::SockRef::from(&stream).set_linger(Some(Duration::ZERO))?;
    Ok(stream)
}

/// `connect_abortive`, then a TLS handshake over it. The linger option is set
/// on the TCP socket *before* the TLS layer wraps it, so the abortive close
/// still applies to the TLS variant.
async fn connect_abortive_tls(
    addr: &str,
    cfg: &Arc<ClientConfig>,
    timeout: Duration,
) -> anyhow::Result<TlsStream<TcpStream>> {
    let stream = connect_abortive(addr, timeout).await?;
    let connector = TlsConnector::from(cfg.clone());
    let name = ServerName::try_from(SERVER_NAME).map_err(|_| anyhow!("bad server name {SERVER_NAME:?}"))?;
    let tls = time::timeout(timeout, connector.connect(name, stream))
        .await
        .map_err(|_| anyhow!("no TLS handshake within {timeout:?}"))??;
    Ok(tls)
}

/// One CONNECT → CONNACK round trip on a fresh connection, then an abortive
/// close. MQTT 5.0 with Clean Start, like the incident's clients; a new
/// ClientId per attempt, so the broker never has to take a session over —
/// that path is a different mechanism (`kick`) and would blur the evidence.
///
/// Shared with the cluster reproductions, which probe every node with it.
pub(crate) async fn handshake(
    addr: &str,
    transport: &Transport,
    client_id: &str,
    connect_timeout: Duration,
    connack_timeout: Duration,
) -> anyhow::Result<ConnectAckReason> {
    match transport {
        Transport::Plain => {
            let stream = connect_abortive(addr, connect_timeout).await?;
            handshake_over(stream, client_id, connack_timeout).await
        }
        Transport::Tls(cfg) => {
            let stream = connect_abortive_tls(addr, cfg, connect_timeout).await?;
            handshake_over(stream, client_id, connack_timeout).await
        }
    }
}

/// The MQTT part of [`handshake`], over any byte stream (plain or TLS).
pub(crate) async fn handshake_over<S>(
    stream: S,
    client_id: &str,
    connack_timeout: Duration,
) -> anyhow::Result<ConnectAckReason>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    let mut codec = V5Codec::new(0, 0);

    let connect = Connect {
        clean_start: true,
        keep_alive: 60,
        client_id: ByteString::from(client_id),
        ..Default::default()
    };
    let mut out = BytesMut::with_capacity(128);
    codec.encode(PacketV5::Connect(Box::new(connect)), &mut out)?;
    writer.write_all(&out).await?;
    writer.flush().await?;

    let mut inbuf = BytesMut::with_capacity(4096);
    let packet = time::timeout(connack_timeout, async {
        loop {
            if let Some((packet, _consumed)) = codec.decode(&mut inbuf)? {
                return Ok::<PacketV5, anyhow::Error>(packet);
            }
            if reader.read_buf(&mut inbuf).await? == 0 {
                return Err(anyhow!("broker closed the connection before CONNACK"));
            }
        }
    })
    .await
    .map_err(|_| anyhow!("no CONNACK within {connack_timeout:?}"))??;

    match packet {
        PacketV5::ConnectAck(ack) => Ok(ack.reason_code),
        other => Err(anyhow!("expected CONNACK, got {other:?}")),
    }
}

// ---------------------------------------------------------------------------
// Ramp
// ---------------------------------------------------------------------------

/// Reconnects as fast as the broker lets it until `stop` is set or `deadline`
/// passes. One fresh ClientId per attempt.
async fn ramp_worker(
    worker: usize,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    c: Arc<Counters>,
    ep: Arc<Endpoint>,
) {
    let mut seq = 0u64;
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        seq += 1;
        let client_id = format!("{}-{worker}-{seq}", ep.client_prefix);
        c.attempts.fetch_add(1, Ordering::Relaxed);
        match handshake(ep.addr, &ep.transport, &client_id, ATTEMPT_TIMEOUT, ATTEMPT_TIMEOUT).await {
            Ok(ConnectAckReason::Success) => {
                c.accepted.fetch_add(1, Ordering::Relaxed);
            }
            Ok(other) => {
                c.refused.fetch_add(1, Ordering::Relaxed);
                c.note(format!("unexpected CONNACK {other:?}"));
            }
            Err(e) => {
                c.errors.fetch_add(1, Ordering::Relaxed);
                c.note(e.to_string());
            }
        }
    }
}

/// Probes the broker until `deadline`, and returns as soon as the stall
/// criterion holds: `STALL_PROBES` consecutive failed probes *and* no accepted
/// handshake anywhere in that window.
///
/// `deadline` is load-bearing: the harness only checks a test's declared
/// timeout *after* it returns (`framework/scheduler.rs`), so a watchdog that
/// looped without one would hang the whole run instead of reporting a verdict.
async fn watchdog(
    c: Arc<Counters>,
    stop: Arc<AtomicBool>,
    deadline: Instant,
    ep: Arc<Endpoint>,
) -> Option<Stall> {
    let mut probes_ok = 0u64;
    let mut probes_failed = 0u64;
    let mut streak = 0u32;
    let mut window_start = Instant::now();
    let mut accepted_at_window_start = c.accepted.load(Ordering::Relaxed);
    let mut last_probe_error = String::new();

    time::sleep(WARMUP).await;

    while Instant::now() < deadline {
        time::sleep(PROBE_INTERVAL).await;
        let client_id = format!("{}-probe-{}", ep.client_prefix, PROBE_SEQ.fetch_add(1, Ordering::Relaxed));

        let failed = match handshake(ep.addr, &ep.transport, &client_id, PROBE_TIMEOUT, PROBE_TIMEOUT).await {
            Ok(ConnectAckReason::Success) => {
                probes_ok += 1;
                None
            }
            Ok(other) => {
                probes_failed += 1;
                Some(format!("unexpected CONNACK {other:?}"))
            }
            Err(e) => {
                probes_failed += 1;
                Some(e.to_string())
            }
        };

        match failed {
            None => {
                streak = 0;
            }
            Some(err) => {
                if streak == 0 {
                    window_start = Instant::now();
                    accepted_at_window_start = c.accepted.load(Ordering::Relaxed);
                }
                streak += 1;
                last_probe_error = err;
            }
        }

        if streak >= STALL_PROBES {
            let ramp_progress = c.accepted.load(Ordering::Relaxed) - accepted_at_window_start;
            // `accepted_at_window_start > 0` is what separates "the node stopped
            // working" from "the node never worked": a broker that refuses to
            // start, or a fixture pointing at the wrong port, also completes
            // zero handshakes, and that must not be reported as a repro of the
            // incident. The `None` path below turns it into an Error instead.
            if ramp_progress == 0 && accepted_at_window_start > 0 {
                stop.store(true, Ordering::Relaxed);
                return Some(Stall {
                    probes_ok,
                    probes_failed,
                    streak,
                    window: window_start.elapsed(),
                    at: chrono::Local::now(),
                    http: http_probe(ep.http).await,
                    last_probe_error,
                });
            }
            // The probes are failing but the ramp is still getting CONNACKs:
            // the broker is slow, not stalled.
            streak = 0;
        }
    }

    // Deadline reached without a confirmed stall: the broker kept completing
    // handshakes for the whole ramp.
    None
}

/// Samples the accepted-handshake rate once per `RATE_INTERVAL` so a collapse
/// (or a dip) is visible in the evidence even when the hard stall criterion is
/// not met.
pub(crate) async fn rate_sampler(
    deadline: Instant,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    out: Arc<parking_lot::Mutex<Vec<u64>>>,
) {
    let mut prev = counters.accepted.load(Ordering::Relaxed);
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        time::sleep(RATE_INTERVAL).await;
        let now = counters.accepted.load(Ordering::Relaxed);
        out.lock().push(now.saturating_sub(prev));
        prev = now;
    }
}

/// Summary of the sampled rate profile.
///
/// Shared with the TLS variant and the cluster reproductions, which report one
/// of these per node.
pub(crate) struct RateStats {
    pub(crate) buckets: usize,
    pub(crate) median: u64,
    pub(crate) min: u64,
    /// Buckets at or below a quarter of the median (and below 10/s), i.e. the
    /// seconds in which the node was effectively not serving.
    pub(crate) dead_seconds: usize,
}

impl RateStats {
    pub(crate) fn of(profile: &[u64]) -> Self {
        let mut sorted = profile.to_vec();
        sorted.sort_unstable();
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or(0);
        let min = sorted.first().copied().unwrap_or(0);
        let floor = (median / 4).max(1);
        let dead_seconds = profile.iter().filter(|&&v| v <= floor && v < 10).count();
        Self { buckets: profile.len(), median, min, dead_seconds }
    }

    pub(crate) fn describe(&self) -> String {
        format!(
            "handshake rate: median {} /s, min {} /s over {}s ({} s at or below 25% of the median)",
            self.median, self.min, self.buckets, self.dead_seconds
        )
    }
}

/// Runs the ramp plus the watchdog and stops the ramp as soon as a stall is
/// confirmed, so a detection does not have to wait out `RAMP_DURATION`.
///
/// Returns the stall (if any) and the per-second rate profile of the ramp.
pub(crate) async fn run_ramp(c: Arc<Counters>, ep: Arc<Endpoint>) -> (Option<Stall>, Vec<u64>) {
    let deadline = Instant::now() + WARMUP + RAMP_DURATION;
    let stop = Arc::new(AtomicBool::new(false));
    let profile = Arc::new(parking_lot::Mutex::new(Vec::new()));

    let mut workers = Vec::with_capacity(RAMP_CLIENTS);
    for worker in 0..RAMP_CLIENTS {
        workers.push(tokio::spawn(ramp_worker(worker, deadline, stop.clone(), c.clone(), ep.clone())));
    }
    tokio::spawn(rate_sampler(deadline, stop.clone(), c.clone(), profile.clone()));

    let stall = watchdog(c.clone(), stop.clone(), deadline, ep).await;
    for worker in workers {
        let _ = worker.await;
    }
    let profile = profile.lock().clone();
    (stall, profile)
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// `GET /api/v1/nodes`, bounded. Whether the API still answers while
/// handshakes do not complete separates "the runtime is stuck" from "the
/// per-port handshake queue is stuck".
async fn http_probe(http_addr: &str) -> String {
    let request = format!(
        "GET /api/v1/nodes HTTP/1.1\r\nHost: {http_addr}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let probe = async {
        let mut stream = TcpStream::connect(http_addr).await?;
        stream.write_all(request.as_bytes()).await?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        Ok::<String, anyhow::Error>(String::from_utf8_lossy(&buf).into_owned())
    };
    match time::timeout(Duration::from_secs(2), probe).await {
        Ok(Ok(response)) => format!("answered: {}", response.lines().next().unwrap_or("").trim()),
        Ok(Err(e)) => format!("no answer: {e}"),
        Err(_) => "no answer within 2s".to_string(),
    }
}

/// Size, last-write age and last log line of the fixture's log sink. A frozen
/// mtime is the "its log ends" symptom; a growing one points at a stall that
/// is narrower than the whole runtime.
fn log_evidence(log_path: &str) -> String {
    let meta = match std::fs::metadata(log_path) {
        Ok(meta) => meta,
        Err(e) => return format!("{log_path}: {e}"),
    };
    let age = meta
        .modified()
        .ok()
        .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
        .map(|d| format!("{}s", d.as_secs()))
        .unwrap_or_else(|| "unknown".to_string());
    format!("{log_path}: {} bytes, last write {age} ago, last line: {}", meta.len(), log_tail(log_path))
}

/// Last non-empty line of `path`, read from the tail only (the file can be
/// tens of MB by the end of a ramp).
fn log_tail(path: &str) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return "<unreadable>".to_string();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(4096);
    if file.seek(SeekFrom::Start(from)).is_err() {
        return "<unreadable>".to_string();
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return "<unreadable>".to_string();
    }
    // Read from the tail only, so the slice may start mid-character.
    let text = String::from_utf8_lossy(&buf).into_owned();
    text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("<empty>").trim().to_string()
}

/// How long ago the fixture wrote its last log line.
///
/// Used to flag the one false positive this case can produce: a test host that
/// is itself starved (broker and 256 connectors share one machine) also makes
/// every probe time out. A *wedged* runtime stops logging altogether — that is
/// the "its log ends" symptom of the report — so a log written within the last
/// seconds while the probes fail points at the host, not at the defect. It only
/// annotates the verdict; it never changes it, because a stalled-but-chatty
/// broker is still a defect worth seeing.
pub(crate) fn log_age(log_path: &str) -> Option<Duration> {
    let modified = std::fs::metadata(log_path).ok()?.modified().ok()?;
    std::time::SystemTime::now().duration_since(modified).ok()
}

// ---------------------------------------------------------------------------
// Verdict
// ---------------------------------------------------------------------------

/// Turns a finished ramp into a verdict. Shared by the plain and the TLS
/// variant of this case, so "what counts as a reproduction" and the evidence
/// format cannot drift between the two.
///
/// `tcp_alive` is the TCP-level health check taken *after* the ramp: the whole
/// point of the incident is that it still succeeds while handshakes do not.
#[allow(clippy::too_many_arguments)]
pub(crate) fn verdict(
    case: &str,
    suite: &str,
    ep: &Endpoint,
    counters: &Counters,
    stall: Option<Stall>,
    profile: &[u64],
    elapsed: Duration,
    tcp_alive: bool,
) -> TestResult {
    let attempts = counters.attempts.load(Ordering::Relaxed);
    let accepted = counters.accepted.load(Ordering::Relaxed);
    let refused = counters.refused.load(Ordering::Relaxed);
    let errors = counters.errors.load(Ordering::Relaxed);
    let rate = RateStats::of(profile).describe();
    let totals = format!(
        "ramp over {secs:.1}s: {attempts} attempts, {accepted} accepted, {refused} unexpected CONNACKs, \
         {errors} transport errors; {rate}; last error: {last}",
        secs = elapsed.as_secs_f64(),
        last = counters.take_error()
    );

    match stall {
        Some(stall) => {
            let starvation = match log_age(ep.log_path) {
                Some(age) if age < Duration::from_secs(2) => {
                    " NOTE: the broker was still writing its log when the stall was confirmed — \
                     rule out test-host starvation (broker and ramp on one machine) before \
                     trusting this."
                }
                _ => "",
            };
            let reason = format!(
                "node stopped completing handshakes while the process stayed alive (issue #532): \
                 no CONNACK for {window:.2}s ({streak}/{stall_probes} consecutive probes) and the ramp made \
                 no progress in the same window. {totals}. \
                 probes before the stall: {ok} ok / {failed} failed; last probe error: {probe_err}. \
                 TCP-level health check during the stall: {tcp_alive}. \
                 HTTP API (/api/v1/nodes): {http}. {log}.{starvation}",
                window = stall.window.as_secs_f64(),
                streak = stall.streak,
                stall_probes = STALL_PROBES,
                ok = stall.probes_ok,
                failed = stall.probes_failed,
                probe_err = stall.last_probe_error,
                http = stall.http,
                log = log_evidence(ep.log_path),
            );
            tracing::warn!(at = %stall.at, "{reason}");
            TestResult::failed(case, suite, elapsed, reason)
        }
        None => {
            // A broker that never completed a single handshake is a
            // fixture/harness problem, not a clean run: report it as such
            // instead of letting it count as "no stall observed".
            if accepted == 0 {
                let reason = format!(
                    "no handshake was ever completed in {secs:.1}s of ramp — the fixture broker \
                     ({fixture}, {addr}) is not serving CONNECTs at all, so this run says nothing \
                     about issue #532. {totals}; {log}",
                    secs = elapsed.as_secs_f64(),
                    fixture = ep.fixture,
                    addr = ep.addr,
                    log = log_evidence(ep.log_path)
                );
                tracing::error!("{reason}");
                return TestResult::error(case, suite, elapsed, reason);
            }
            // A dip that never became a full stall is the other thing this
            // case must not hide: report the worst second loudly, even though
            // the hard criterion was not met.
            let stats = RateStats::of(profile);
            let degraded = if stats.dead_seconds > 0 {
                format!(
                    " DIP: {} s of the ramp ran at or below 25% of the median rate ({} /s vs {} /s) — \
                     not a full stall, but not a clean run either.",
                    stats.dead_seconds, stats.min, stats.median
                )
            } else {
                String::new()
            };
            let note = format!(
                "no stall in {secs:.1}s of ramp; {totals};{degraded} {log}",
                secs = elapsed.as_secs_f64(),
                log = log_evidence(ep.log_path)
            );
            if degraded.is_empty() {
                tracing::info!("{note}");
            } else {
                tracing::warn!("{note}");
            }
            TestResult::passed_with_note(case, suite, elapsed, &note)
        }
    }
}

// ---------------------------------------------------------------------------
// Case
// ---------------------------------------------------------------------------

/// The plain-TCP endpoint of this case: its own fixture ports, its own log
/// sink, its own ClientId prefix.
fn plain_endpoint() -> Arc<Endpoint> {
    Arc::new(Endpoint {
        addr: BROKER_ADDR,
        http: HTTP_ADDR,
        log_path: LOG_PATH,
        client_prefix: "i532",
        fixture: "configs/issue532-ramp",
        transport: Transport::Plain,
    })
}

/// The connect-ramp stall of issue #532, reproduced on a single node.
pub struct Issue532ConnectRampStallTest;

impl TestCase for Issue532ConnectRampStallTest {
    fn name(&self) -> &str {
        "issue532_connect_ramp_stall"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(crate::tests::config_path("issue532-ramp"))
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let counters = Arc::new(Counters::default());
        let ep = plain_endpoint();

        let (stall, profile) = rt.block_on(run_ramp(counters.clone(), ep.clone()));
        // The TCP-level checks are collected *now*, on purpose: the whole point
        // of the incident is that they still succeed.
        let tcp_alive = ctx.broker_healthy();
        verdict(self.name(), SUITE, &ep, &counters, stall, &profile, start.elapsed(), tcp_alive)
    }

    fn timeout(&self) -> Duration {
        // RAMP_DURATION + WARMUP + the ramp's own drain after a stall + the
        // health/HTTP/log evidence probes.
        Duration::from_secs(120)
    }

    /// Assertive on purpose: the case exists to reproduce issue #532, so a
    /// reproduced stall must fail the suite. Switch to
    /// `Expectation::ExpectedFail` if the suite is ever required to stay green
    /// while the defect is open (the harness then records the repro as
    /// EXPECTED-FAIL with the evidence, and reports UNEXPECTED-PASS once the
    /// stall stops firing).
    fn expectation(&self) -> Expectation {
        Expectation::Normal
    }
}
