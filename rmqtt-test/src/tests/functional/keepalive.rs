//! KeepAlive and PINGREQ/PINGRESP functional tests

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Open a raw v3.1.1 connection with a custom keep-alive and consume the
/// CONNACK. Returns the stream.
fn raw_connect_keepalive(broker_addr: &str, client_id: &str, keep_alive: u16) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(4); // v3.1.1
    body.push(0x02); // clean session
    body.extend_from_slice(&keep_alive.to_be_bytes());
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);

    let mut pkt = vec![0x10];
    let mut len = body.len();
    loop {
        let mut b = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        pkt.push(b);
        if len == 0 {
            break;
        }
    }
    pkt.extend_from_slice(&body);
    stream.write_all(&pkt)?;
    stream.flush()?;

    let mut buf = [0u8; 4];
    let n = stream.read(&mut buf)?;
    if n < 4 || buf[0] != 0x20 || buf[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &buf[..n]));
    }
    Ok(stream)
}

/// Test that sending PINGREQs keeps the connection alive with short keep_alive (v3.1.1)
pub struct KeepAliveV311Test;

impl TestCase for KeepAliveV311Test {
    fn name(&self) -> &str {
        "keepalive_v311_ping_keeps_alive"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "keepalive-v311",
                ctx.config.connect_timeout,
                true,
                5, // 5 second keep alive
                None,
                None,
                None,
            )
            .await?;

            // Send PINGREQs at 2s intervals for 15s (3x the keepalive)
            for _ in 0..6 {
                client.ping().await?;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            // Should still be connected
            if !client.is_connected() {
                return Err(anyhow::anyhow!("client disconnected despite sending PINGREQs"));
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Test that without PINGREQs, broker disconnects after keep_alive expires (v3.1.1)
pub struct KeepAliveTimeoutTest;

impl TestCase for KeepAliveTimeoutTest {
    fn name(&self) -> &str {
        "keepalive_v311_timeout"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "keepalive-timeout",
                ctx.config.connect_timeout,
                true,
                5, // 5 second keep alive
                None,
                None,
                None,
            )
            .await?;

            // Don't send any PINGREQs - wait for timeout
            // MQTT spec says broker should wait keepalive * 1.5 before disconnecting
            tokio::time::sleep(Duration::from_secs(15)).await;

            // After 15s with keepalive=5, should be disconnected
            if client.is_connected() {
                return Err(anyhow::anyhow!("client should have been disconnected by keepalive timeout"));
            }

            // Attempting to disconnect should not panic
            let _ = client.disconnect().await;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Test PINGREQ/PINGRESP with MQTT 5.0 client
pub struct PingV5Test;

impl TestCase for PingV5Test {
    fn name(&self) -> &str {
        "ping_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "ping-v5-test",
                ctx.config.connect_timeout,
                true,
                10,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

            // Send multiple PINGREQs
            for _ in 0..5 {
                client.ping().await?;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }

            assert!(client.is_connected());
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: keep_alive = 0 disables the keep-alive timeout; the broker must
/// not disconnect a silent client. [MQTT-3.1.2-24 keep-alive = 0 clause]
pub struct KeepAliveV311ZeroTest;

impl TestCase for KeepAliveV311ZeroTest {
    fn name(&self) -> &str {
        "keepalive_v311_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "v311-keepalive-zero",
                ctx.config.connect_timeout,
                true,
                0, // keep_alive = 0 → no timeout
                None,
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());

            // Stay silent well beyond a typical keep-alive window.
            tokio::time::sleep(Duration::from_secs(5)).await;
            assert!(
                client.is_connected(),
                "keep_alive = 0 must disable the timeout, but the broker disconnected us"
            );

            // The connection must still be usable.
            client.ping().await?;
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Boundary: keep_alive = 65535 (the max u16) is accepted and the connection
/// stays open (the keep-alive window is huge).
pub struct KeepAliveV311MaxValueTest;

impl TestCase for KeepAliveV311MaxValueTest {
    fn name(&self) -> &str {
        "keepalive_v311_max_value"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "v311-keepalive-max",
                ctx.config.connect_timeout,
                true,
                u16::MAX, // 65535 seconds
                None,
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(client.is_connected(), "max keep alive should not disconnect");
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

// ---------------------------------------------------------------------------
// P2 conformance gap fill (G24 / G23, 2026-08-23)
// ---------------------------------------------------------------------------

/// Explicit PINGRESP verification: a PINGREQ must be answered with a PINGRESP
/// packet [MQTT-3.13.0-1]. The existing keepalive tests only assert that the
/// connection stays alive after pings (implicit); this test reads the
/// PINGRESP bytes directly.
pub struct KeepAliveV311PingRespExplicitTest;

impl TestCase for KeepAliveV311PingRespExplicitTest {
    fn name(&self) -> &str {
        "keepalive_v311_pingresp_explicit"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = (|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_keepalive(&ctx.config.broker_addr, &format!("pingresp-{uid}"), 60)?;

            // PINGREQ = 0xC0 0x00
            stream.write_all(&[0xC0, 0x00])?;
            stream.flush()?;

            let mut buf = [0u8; 8];
            let n = stream.read(&mut buf)?;
            if n < 2 || buf[0] != 0xD0 || buf[1] != 0x00 {
                return Err(anyhow::anyhow!(
                    "expected PINGRESP (0xD0 0x00) for PINGREQ, got {:02x?} [MQTT-3.13.0-1]",
                    &buf[..n]
                ));
            }
            Ok(())
        })();

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Fine-grained keep-alive window boundary: with keep_alive = 1s the broker
/// must NOT disconnect a client that stays silent for longer than the nominal
/// keep-alive but still within the grace window, and a PINGREQ sent inside
/// that window must refresh the connection. [MQTT-3.1.2-24]
///
/// rmqtt adjusts the effective window dynamically (fitter.rs): keep_alive < 6s
/// gets +3s, so a 1s keep-alive yields a ~4s window. Waiting 1.2s (> 1x, well
/// inside the window) proves the broker does not cut the connection at 1x.
pub struct KeepAliveV311WindowBoundaryTest;

impl TestCase for KeepAliveV311WindowBoundaryTest {
    fn name(&self) -> &str {
        "keepalive_v311_window_boundary"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = (|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_keepalive(&ctx.config.broker_addr, &format!("kawin-{uid}"), 1)?;

            // Stay silent for 1.2s: > 1x keep_alive, well inside the grace window.
            std::thread::sleep(Duration::from_millis(1200));

            // Probe: the connection must still be open (read timeout = open,
            // EOF = the broker dropped us at 1x).
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            let mut probe = [0u8; 4];
            match stream.read(&mut probe) {
                Ok(0) => Err(anyhow::anyhow!(
                    "broker disconnected after 1.2s with keep_alive=1s (no 1.5x grace window) [MQTT-3.1.2-24]"
                )),
                Ok(n) => Err(anyhow::anyhow!("unexpected data while idle: {:02x?}", &probe[..n])),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(()),
                Err(e) => Err(anyhow::anyhow!("read error: {e}")),
            }?;

            // A PINGREQ inside the window must be answered and keep us alive.
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.write_all(&[0xC0, 0x00])?;
            stream.flush()?;
            let mut buf = [0u8; 8];
            let n = stream.read(&mut buf)?;
            if n < 2 || buf[0] != 0xD0 {
                return Err(anyhow::anyhow!(
                    "expected PINGRESP after window-boundary PINGREQ, got {:02x?}",
                    &buf[..n]
                ));
            }
            Ok(())
        })();

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
