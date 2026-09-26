//! `client_connack` web-hook on a REFUSED connection (rmqtt-web-hook)
//!
//! Background: the `ClientConnack` hook used to be raised ONLY on the
//! success path — `v3.rs` / `v5.rs` call `refused_ack_v3` / `refused_ack` with
//! `connect_info = None`, so the `if let Some(connect_info)` branch inside
//! those helpers never ran. A plugin could therefore neither observe nor
//! rewrite the reason code of a refused CONNECT, and consumers that key off
//! the event (rmqtt-web-hook, the `client_connack*` counters) silently covered
//! successful connections only.
//!
//! This case pins the behaviour end to end through a real transport:
//!
//! 1. `configs/webhook-connack-refused/` starts ONLY `rmqtt-web-hook`, disables
//!    anonymous access and keeps `rmqtt-acl` disabled (its shipped final rule
//!    `["allow", "all"]` also covers CONNECT and would promote the undecided
//!    authentication into an ALLOW — issue #501 fail-open), so a CONNECT
//!    without credentials is refused with CONNACK 0x05 (Not authorized);
//! 2. the plugin POSTs every `client_connack` event to an in-test mock HTTP
//!    receiver on an ephemeral port (the config carries a placeholder port);
//! 3. the test asserts BOTH halves: the client really is refused (0x05) and a
//!    `client_connack` event carrying that refusal reason actually arrives.
//!
//! Step 3 is the regression guard: without the ConnectInfo-aware error exits
//! the refusal never reaches the hook, no event is emitted, and this case
//! fails with "no client_connack web-hook event … within 10s" while the
//! CONNACK code assertion still passes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::connack_return_codes_v311::{
    build_connect, connect_return_code, copy_dir_recursive, spawn_auth_broker_with_config,
};

/// MQTT port of the self-managed broker (see the config header for the port
/// map: 1901 and gRPC 5378 are free).
const WEBHOOK_ADDR: &str = "127.0.0.1:1901";
/// `urls` port placeholder replaced with the ephemeral mock port.
const MOCK_PORT_PLACEHOLDER: &str = "9098";
/// Client identifier used for the refused CONNECT; also used to select the
/// event out of the recorded bodies.
const CLIENT_ID: &str = "webhook-refused";
const SUITE: &str = "functional_v311";
/// `ConnectAckReasonV3::NotAuthorized::reason()` — the `conn_ack` value the
/// web-hook reports for the refusal this case provokes.
const REFUSAL_REASON: &str = "Connection Refused, not authorized";
/// How long to wait for the web-hook POST after the CONNACK was received.
const EVENT_WAIT: Duration = Duration::from_secs(10);

/// Request bodies recorded by the mock receiver.
type Recorded = Arc<Mutex<Vec<String>>>;

/// Copy `configs/webhook-connack-refused/` into
/// `target/webhook-connack-refused-<mock_port>/` and rewrite:
/// 1. the mock receiver port in the web-hook plugin config,
/// 2. `plugins.dir` to an ABSOLUTE path (removes the broker-CWD dependence).
///
/// Returns the `rmqtt.toml` FILE: `-f` must point at the file, a directory is
/// silently ignored by rmqtt-conf and the built-in defaults would be used.
fn prepare_config(mock_port: u16) -> anyhow::Result<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join("webhook-connack-refused");
    let dst = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(format!("webhook-connack-refused-{mock_port}"));
    copy_dir_recursive(&src, &dst)?;

    // 1) web-hook plugin: point the URL at the ephemeral mock port.
    let plugin_cfg = dst.join("plugins").join("rmqtt-web-hook.toml");
    let content = std::fs::read_to_string(&plugin_cfg)?;
    std::fs::write(&plugin_cfg, content.replace(MOCK_PORT_PLACEHOLDER, &mock_port.to_string()))?;

    // 2) main config: absolute plugins.dir.
    let main_cfg = dst.join("rmqtt.toml");
    let content = std::fs::read_to_string(&main_cfg)?;
    let abs_plugins_dir = std::fs::canonicalize(dst.join("plugins"))?.to_string_lossy().replace('\\', "/");
    let updated = content
        .replace("rmqtt-test/configs/webhook-connack-refused/plugins/", &format!("{abs_plugins_dir}/"));
    std::fs::write(&main_cfg, updated)?;
    Ok(main_cfg)
}

/// Start the in-test mock web-hook receiver on an EPHEMERAL port.
///
/// It records the body of every POST and answers `200 OK`. The readiness probe
/// after the accept loop is spawned performs a real HTTP exchange before the
/// broker starts, so a transport problem can never be misread as "the hook
/// raised no event" — the two failures this case distinguishes.
async fn spawn_webhook_receiver() -> anyhow::Result<(tokio::task::JoinHandle<()>, u16, Recorded)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mock_port = listener.local_addr()?.port();
    let addr = format!("127.0.0.1:{mock_port}");
    let recorded: Recorded = Arc::new(Mutex::new(Vec::new()));
    let sink = recorded.clone();

    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            let sink = sink.clone();
            tokio::spawn(async move {
                let mut buf: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 4096];

                // Request head, up to the header terminator.
                let head_end = loop {
                    match sock.read(&mut tmp).await {
                        Ok(0) => return,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break i + 4;
                            }
                        }
                        Err(_) => return,
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let content_len = head
                    .lines()
                    .find_map(|line| {
                        let (k, v) = line.split_once(':')?;
                        if k.eq_ignore_ascii_case("content-length") {
                            v.trim().parse::<usize>().ok()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0);

                // Body, as announced by Content-Length (reqwest always sets it
                // for a JSON string body).
                while buf.len() < head_end + content_len {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let body_end = (head_end + content_len).min(buf.len());
                let body = String::from_utf8_lossy(&buf[head_end..body_end]).into_owned();

                // Only POSTs carry a hook event; the readiness probe is a GET.
                if head.starts_with("POST") && !body.is_empty() {
                    if let Ok(mut v) = sink.lock() {
                        v.push(body);
                    }
                }
                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    // Readiness probe: a real HTTP exchange before the broker starts.
    {
        let mut probe = tokio::net::TcpStream::connect(&addr).await?;
        probe.write_all(b"GET /readiness HTTP/1.1\r\nHost: probe\r\n\r\n").await?;
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(3), probe.read(&mut buf))
            .await
            .map_err(|_| anyhow::anyhow!("mock web-hook receiver did not become ready on {addr}"))??;
        if n == 0 || !buf.starts_with(b"HTTP/1.1 200") {
            return Err(anyhow::anyhow!("mock web-hook receiver replied unexpectedly: {buf:?}"));
        }
    }

    Ok((handle, mock_port, recorded))
}

/// Poll the recorded bodies until a `client_connack` event for [`CLIENT_ID`]
/// shows up, or `timeout` elapses. The edge-triggered web-hook write is
/// asynchronous, so the CONNACK can be observed before the POST lands.
async fn wait_for_connack_event(recorded: &Recorded, timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let hit = recorded.lock().ok().and_then(|v| {
            v.iter().find(|b| b.contains("\"action\":\"client_connack\"") && b.contains(CLIENT_ID)).cloned()
        });
        if let Some(hit) = hit {
            return Some(hit);
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A refused CONNECT must still reach the `client_connack` hook, so the
/// web-hook reports the refusal reason (see the module docs).
pub struct WebhookConnackRefusedV311Test;

impl TestCase for WebhookConnackRefusedV311Test {
    fn name(&self) -> &str {
        "webhook_connack_refused_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let (receiver, mock_port, recorded) = spawn_webhook_receiver().await?;
            let outcome = async {
                let config = prepare_config(mock_port)?;
                let (_node, _binary) =
                    spawn_auth_broker_with_config(config, "webhook-connack-refused", WEBHOOK_ADDR)?;
                // flags = clean session only: no username, no password. The
                // listener sets allow_anonymous = false and no auth plugin is
                // started, so the auth chain must refuse with CONNACK 0x05.
                let code = connect_return_code(WEBHOOK_ADDR, &build_connect(0x02, CLIENT_ID, None, None))?;
                let event = wait_for_connack_event(&recorded, EVENT_WAIT).await;
                Ok::<_, anyhow::Error>((code, event))
            }
            .await;
            // Always stop the receiver (abort + await), even when an assertion
            // fails early: the accept loop closes the listener with the task.
            receiver.abort();
            let _ = receiver.await;
            outcome
        });

        match result {
            Ok((Some(0x05), Some(event))) => {
                if !event.contains(REFUSAL_REASON) {
                    return TestResult::failed(
                        self.name(),
                        SUITE,
                        start.elapsed(),
                        format!(
                            "client_connack web-hook event does not carry the refusal reason \
                             {REFUSAL_REASON:?}: {event}"
                        ),
                    );
                }
                TestResult::passed(self.name(), SUITE, start.elapsed())
            }
            Ok((Some(0x05), None)) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "the CONNECT was refused with CONNACK 0x05 (as expected) but no \
                     client_connack web-hook event arrived within {}s — the refusal path \
                     never reaches the ClientConnack hook",
                    EVENT_WAIT.as_secs()
                ),
            ),
            Ok((Some(code), _)) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "expected CONNACK 0x05 (Not authorized) for an anonymous CONNECT against a \
                     broker with allow_anonymous = false, got 0x{code:02x}"
                ),
            ),
            Ok((None, _)) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                "broker closed the connection without a CONNACK".into(),
            ),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // Must exceed NODE_START_TIMEOUT (20s) + receiver probe + EVENT_WAIT.
        Duration::from_secs(45)
    }
}
