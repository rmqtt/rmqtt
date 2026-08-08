//! MQTT v3.1.1 Connect/Disconnect functional tests

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Build a raw CONNECT packet with arbitrary protocol name / level / flags.
/// Used by negative tests exercising the CONNECT variable header validation.
fn raw_connect_bytes(
    protocol_name: &[u8],
    protocol_level: u8,
    connect_flags: u8,
    keep_alive: u16,
    client_id: &[u8],
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&(protocol_name.len() as u16).to_be_bytes());
    body.extend_from_slice(protocol_name);
    body.push(protocol_level);
    body.push(connect_flags);
    body.extend_from_slice(&keep_alive.to_be_bytes());
    body.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
    body.extend_from_slice(client_id);

    let mut pkt = vec![0x10]; // CONNECT
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
    pkt
}

/// Send a raw CONNECT and read the broker's response.
/// Returns `Ok(Some(return_code))` on CONNACK, `Ok(None)` when the connection
/// was closed without a CONNACK, `Err` on I/O failure.
fn raw_connect_exchange(broker_addr: &str, packet: &[u8]) -> anyhow::Result<Option<u8>> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(packet)?;
    stream.flush()?;

    let mut buf = [0u8; 8];
    match stream.read(&mut buf) {
        Ok(n) if n >= 4 && buf[0] == 0x20 => Ok(Some(buf[3])),
        Ok(_) => Ok(None),
        Err(_) => Ok(None), // timed out / closed: no CONNACK
    }
}

/// Test basic MQTT 3.1.1 connect and disconnect
pub struct ConnectV311Test;

impl TestCase for ConnectV311Test {
    fn name(&self) -> &str {
        "connect_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "connect-v311-test",
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client.is_connected());
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }
}

/// Test connect with empty client ID (clean session required)
pub struct ConnectEmptyClientIdTest;

impl TestCase for ConnectEmptyClientIdTest {
    fn name(&self) -> &str {
        "connect_empty_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "",
                ctx.config.connect_timeout,
            )
            .await?; // should succeed with clean session
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }
}

/// Test multiple concurrent connections
pub struct MultipleConnectionsTest {
    pub count: usize,
}

impl Default for MultipleConnectionsTest {
    fn default() -> Self {
        Self { count: 10 }
    }
}

impl TestCase for MultipleConnectionsTest {
    fn name(&self) -> &str {
        "multiple_connections"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let count = self.count;
        let addr = ctx.config.broker_addr.clone();
        let connect_timeout = ctx.config.connect_timeout;

        let result = rt.block_on(async {
            let mut clients = Vec::new();
            for i in 0..count {
                let client = crate::mqtt::v311::MqttV311Client::connect(
                    &addr,
                    &format!("multi-conn-{}", i),
                    connect_timeout,
                )
                .await?;
                clients.push(client);
            }
            // Verify all connected
            for client in &clients {
                assert!(client.is_connected());
            }
            // Disconnect all
            for client in clients {
                client.disconnect().await?;
            }
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

/// Positive: Session Present must be 0 on a fresh connection with
/// clean_session = 1 (no stored session state exists).
pub struct ConnectV311SessionPresentFreshTest;

impl TestCase for ConnectV311SessionPresentFreshTest {
    fn name(&self) -> &str {
        "connect_v311_session_present_fresh"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-fresh-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(
                !client.connack().session_present,
                "session present must be 0 on a fresh clean_session=1 connection"
            );
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

/// Negative: a wrong protocol name (not "MQTT") must be rejected; the broker
/// replies with a non-accept CONNACK or closes the connection.
pub struct ConnectV311WrongProtocolNameTest;

impl TestCase for ConnectV311WrongProtocolNameTest {
    fn name(&self) -> &str {
        "connect_v311_wrong_protocol_name"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // "MQTTP" is not a valid protocol name
        let packet = raw_connect_bytes(b"MQTTP", 4, 0x02, 60, b"wrong-name");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v311",
                        start.elapsed(),
                        "broker accepted a CONNECT with an invalid protocol name".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v311", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: an unsupported protocol level must be rejected with CONNACK
/// return code 1 (Unacceptable Protocol Version) or the connection closed.
/// [MQTT-3.1.2-2]
pub struct ConnectV311UnsupportedLevelTest;

impl TestCase for ConnectV311UnsupportedLevelTest {
    fn name(&self) -> &str {
        "connect_v311_unsupported_level"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // "MQTT" with level 6 (invalid)
        let packet = raw_connect_bytes(b"MQTT", 6, 0x02, 60, b"bad-level");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v311",
                        start.elapsed(),
                        "broker accepted protocol level 6".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v311", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: CONNECT with the reserved flag bit (bit 0) set must be rejected.
/// [MQTT-3.1.2-3]
pub struct ConnectV311ReservedFlagTest;

impl TestCase for ConnectV311ReservedFlagTest {
    fn name(&self) -> &str {
        "connect_v311_reserved_flag"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x03: clean session (0x02) + reserved bit 0 set (0x01)
        let packet = raw_connect_bytes(b"MQTT", 4, 0x03, 60, b"reserved-flag");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v311",
                        start.elapsed(),
                        "broker accepted CONNECT with reserved flag bit set".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v311", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a second CONNECT on an established connection is a protocol
/// violation and must cause the broker to close the connection. [MQTT-3.1.0-2]
pub struct ConnectV311SecondConnectTest;

impl TestCase for ConnectV311SecondConnectTest {
    fn name(&self) -> &str {
        "connect_v311_second_connect"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("v311-second-{uid}");

            // Connect normally first
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client.is_connected());

            // Now send a raw second CONNECT on the same connection. The v311
            // client's writer is shared; craft the packet and send it raw.
            // We need access to the raw writer, so rebuild a raw CONNECT from
            // scratch on a second path: the client does not expose its writer,
            // so this test uses a dedicated raw socket instead.
            let mut stream = TcpStream::connect(&ctx.config.broker_addr)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;

            // First CONNECT
            let first = raw_connect_bytes(b"MQTT", 4, 0x02, 60, cid.as_bytes());
            stream.write_all(&first)?;
            stream.flush()?;
            let mut buf = [0u8; 8];
            let n = stream.read(&mut buf)?;
            if n < 4 || buf[0] != 0x20 || buf[3] != 0 {
                return Err(anyhow::anyhow!("first CONNECT failed: {:02x?}", &buf[..n]));
            }

            // Second CONNECT must be treated as a protocol violation
            let second = raw_connect_bytes(b"MQTT", 4, 0x02, 60, cid.as_bytes());
            stream.write_all(&second)?;
            stream.flush()?;

            // Broker must close the connection (EOF) — NOT reply CONNACK again
            let closed = matches!(stream.read(&mut buf), Ok(0) | Err(_));

            let _ = client.disconnect().await;

            if closed {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close on second CONNECT [MQTT-3.1.0-2]"))
            }
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

/// Boundary: a client id longer than the spec's 1-23 char guideline is still
/// accepted by default (RMQTT `max_clientid_len` defaults to 65535, per
/// MQTT-3.1.3-5 MAY clause allowing longer ids).
pub struct ConnectV311LongClientIdTest;

impl TestCase for ConnectV311LongClientIdTest {
    fn name(&self) -> &str {
        "connect_v311_long_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let long_id = "client-id-30-chars-long-0123456789ab";
            assert!(long_id.len() > 23);
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                long_id,
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client.is_connected());
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
