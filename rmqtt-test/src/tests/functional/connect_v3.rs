//! MQTT v3.1 Connect/Disconnect functional tests (MQIsdp / level 3)
//!
//! Covers MQTT v3.1 spec section 3.1 (CONNECT) and 3.2 (CONNACK):
//! - positive: valid MQIsdp + level 3 connect, CONNACK return code 0
//! - negative: wrong protocol name / protocol level, reserved flag bit set
//! - boundary: empty client id + clean session 0 -> 0x02, long client id
//! - session present flag on first connect

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Build a raw CONNECT packet with arbitrary protocol name / level / flags.
///
/// Used by negative tests to exercise broker validation of the CONNECT
/// variable header. Returns the complete wire bytes.
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
///
/// Returns `Ok(Some(return_code))` when the broker replies with a CONNACK
/// (return code = byte 3), `Ok(None)` when the connection was closed without
/// a CONNACK, and `Err` on I/O failure.
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

/// Positive: a true MQTT v3.1 client ("MQIsdp" / level 3) connects and gets
/// CONNACK return code 0 with session present = false on a fresh connection.
pub struct ConnectV3Test;

impl TestCase for ConnectV3Test {
    fn name(&self) -> &str {
        "connect_v3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "connect-v3-test",
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client.is_connected());
            assert_eq!(
                client.connack().return_code,
                rmqtt_codec::v3::ConnectAckReason::ConnectionAccepted,
                "CONNACK return code must be 0"
            );
            assert!(!client.connack().session_present, "session present must be false on a fresh connection");
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: connect with will / username / password set (no auth configured,
/// so username/password are accepted; the will is registered).
pub struct ConnectV3WithOptionsTest;

impl TestCase for ConnectV3WithOptionsTest {
    fn name(&self) -> &str {
        "connect_v3_with_options"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let will = rmqtt_codec::v3::LastWill {
                qos: rmqtt_codec::v3::QoS::AtLeastOnce,
                retain: false,
                topic: bytestring::ByteString::from("v3/will/options"),
                message: bytes::Bytes::from_static(b"will-opts"),
            };
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                "connect-v3-opts",
                ctx.config.connect_timeout,
                true,
                30,
                Some(will),
                Some(bytestring::ByteString::from("user")),
                Some(bytes::Bytes::from_static(b"pass")),
            )
            .await?;
            assert!(client.is_connected());
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: unknown protocol name must be rejected (broker replies with a
/// non-accept CONNACK or closes the connection).
pub struct ConnectV3WrongProtocolNameTest;

impl TestCase for ConnectV3WrongProtocolNameTest {
    fn name(&self) -> &str {
        "connect_v3_wrong_protocol_name"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // "MQTTP" is not a valid protocol name (not "MQTT", not "MQIsdp")
        let packet = raw_connect_bytes(b"MQTTP", 3, 0x02, 60, b"wrong-name");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                // CONNACK received: must not be 0 (accepted)
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v3",
                        start.elapsed(),
                        "broker accepted a CONNECT with an invalid protocol name".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v3", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: unsupported protocol level must be rejected with CONNACK return
/// code 1 (Unacceptable Protocol Version) or the connection closed.
pub struct ConnectV3UnsupportedLevelTest;

impl TestCase for ConnectV3UnsupportedLevelTest {
    fn name(&self) -> &str {
        "connect_v3_unsupported_level"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // "MQIsdp" with level 6 (invalid)
        let packet = raw_connect_bytes(b"MQIsdp", 6, 0x02, 60, b"bad-level");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v3",
                        start.elapsed(),
                        "broker accepted protocol level 6".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v3", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: CONNECT with the reserved flag bit (bit 0) set must be rejected.
/// Per MQTT-3.1.2-3 the flags byte bit 0 is reserved and must be 0.
pub struct ConnectV3ReservedFlagTest;

impl TestCase for ConnectV3ReservedFlagTest {
    fn name(&self) -> &str {
        "connect_v3_reserved_flag"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x03: clean session (0x02) + reserved bit 0 set (0x01)
        let packet = raw_connect_bytes(b"MQIsdp", 3, 0x03, 60, b"reserved-flag");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v3",
                        start.elapsed(),
                        "broker accepted CONNECT with reserved flag bit set".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v3", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Boundary: empty client id with clean session = 0 must be rejected with
/// CONNACK return code 2 (Identifier Rejected), MQTT-3.1.3-6.
pub struct ConnectV3EmptyClientIdCleanSession0Test;

impl TestCase for ConnectV3EmptyClientIdCleanSession0Test {
    fn name(&self) -> &str {
        "connect_v3_empty_clientid_cleansession0"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x00: no clean session, empty client id
        let packet = raw_connect_bytes(b"MQIsdp", 3, 0x00, 60, b"");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 2 {
                    TestResult::passed(self.name(), "functional_v3", start.elapsed())
                } else {
                    TestResult::failed(
                        self.name(),
                        "functional_v3",
                        start.elapsed(),
                        format!("expected Identifier Rejected (0x02), got return code {code}"),
                    )
                }
            }
            Ok(None) => TestResult::failed(
                self.name(),
                "functional_v3",
                start.elapsed(),
                "broker closed the connection instead of returning 0x02".into(),
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Boundary: empty client id with clean session = 1 must be accepted; the
/// broker assigns a client-generated id (v3.1 has no assigned client id
/// field, so the broker generates a UUID-like id internally).
pub struct ConnectV3EmptyClientIdCleanSession1Test;

impl TestCase for ConnectV3EmptyClientIdCleanSession1Test {
    fn name(&self) -> &str {
        "connect_v3_empty_clientid_cleansession1"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x02: clean session, empty client id
        let packet = raw_connect_bytes(b"MQIsdp", 3, 0x02, 60, b"");
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::passed(self.name(), "functional_v3", start.elapsed())
                } else {
                    TestResult::failed(
                        self.name(),
                        "functional_v3",
                        start.elapsed(),
                        format!("expected Connection Accepted (0x00), got return code {code}"),
                    )
                }
            }
            Ok(None) => TestResult::failed(
                self.name(),
                "functional_v3",
                start.elapsed(),
                "broker closed the connection for empty client id + clean session 1".into(),
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Boundary: a client id longer than the MQTT v3.1 spec's 1-23 chars.
///
/// RMQTT's default `max_clientid_len` is 65535 (v3.1.1 relaxed policy), so a
/// 30-char id is accepted. This documents the actual behavior; when the
/// listener is configured with `max_clientid_len = 23`, the same packet must
/// instead be rejected with 0x02.
pub struct ConnectV3LongClientIdTest;

impl TestCase for ConnectV3LongClientIdTest {
    fn name(&self) -> &str {
        "connect_v3_long_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let long_id = "client-id-30-chars-long-0123456789ab";
        assert!(long_id.len() > 23, "test client id must exceed 23 chars");

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let client = crate::mqtt::v3::MqttV3Client::connect(
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
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: a client id of the maximum wire-encodable length (65535 bytes,
/// the u16 length field limit) is handled gracefully by the broker.
///
/// RMQTT's default `max_clientid_len` is 65535, so a 65535-byte id sits
/// exactly at the limit and is accepted. This documents the boundary behavior
/// for the largest possible client id encoding.
pub struct ConnectV3ClientIdMaxLengthTest;

impl TestCase for ConnectV3ClientIdMaxLengthTest {
    fn name(&self) -> &str {
        "connect_v3_client_id_max_length"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let long_id = vec![b'a'; 65535]; // max u16 length field value
        let packet = raw_connect_bytes(b"MQIsdp", 3, 0x02, 60, &long_id);
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(code)) => {
                if code == 0 {
                    TestResult::passed(self.name(), "functional_v3", start.elapsed())
                } else {
                    TestResult::failed(
                        self.name(),
                        "functional_v3",
                        start.elapsed(),
                        format!("expected Connection Accepted (0x00), got return code {code}"),
                    )
                }
            }
            Ok(None) => TestResult::failed(
                self.name(),
                "functional_v3",
                start.elapsed(),
                "broker closed the connection for a max-length client id".into(),
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}
