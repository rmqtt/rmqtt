//! MQTT v5 Connect functional tests

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use rmqtt_codec::v5::ConnectAckReason;

/// Fixed header byte of an MQTT CONNACK packet.
const PKT_CONNACK: u8 = 0x20;
/// Fixed header byte of an MQTT DISCONNECT packet.
const PKT_DISCONNECT: u8 = 0xE0;

/// Read the remainder of an MQTT packet whose fixed-header byte has already
/// been consumed, so a caller can branch on that byte first (a second CONNACK
/// must be rejected, a DISCONNECT must be accepted) without re-implementing
/// the framing.
fn read_packet_body(stream: &mut TcpStream, first: u8) -> anyhow::Result<Vec<u8>> {
    let mut buf = vec![first];
    let mut b = [0u8; 1];

    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        if stream.read(&mut b)? == 0 {
            return Err(anyhow::anyhow!("connection closed mid-header"));
        }
        buf.push(b[0]);
        remaining |= ((b[0] & 0x7F) as u32) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 21 {
            return Err(anyhow::anyhow!("malformed remaining length"));
        }
    }

    let mut rest = vec![0u8; remaining as usize];
    stream.read_exact(&mut rest)?;
    buf.extend_from_slice(&rest);
    Ok(buf)
}

/// Build a raw MQTT v5 CONNECT packet with arbitrary protocol name / level /
/// flags and an optional auth method. Used by negative tests.
#[allow(clippy::too_many_arguments)]
fn raw_connect_v5_bytes(
    protocol_name: &[u8],
    protocol_level: u8,
    connect_flags: u8,
    keep_alive: u16,
    client_id: &[u8],
    auth_method: Option<&[u8]>,
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&(protocol_name.len() as u16).to_be_bytes());
    body.extend_from_slice(protocol_name);
    body.push(protocol_level);
    body.push(connect_flags);
    body.extend_from_slice(&keep_alive.to_be_bytes());

    // Property length + properties
    let mut props: Vec<u8> = Vec::new();
    if let Some(am) = auth_method {
        props.push(0x15); // Authentication Method
        props.extend_from_slice(&(am.len() as u16).to_be_bytes());
        props.extend_from_slice(am);
    }
    body.push(props.len() as u8);
    body.extend_from_slice(&props);

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
/// Returns `Ok(Some((session_present, reason_code)))` on CONNACK,
/// `Ok(None)` when the connection was closed, `Err` on I/O failure.
fn raw_connect_v5_exchange(broker_addr: &str, packet: &[u8]) -> anyhow::Result<Option<(bool, u8)>> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(packet)?;
    stream.flush()?;

    // Read the full CONNACK (variable length: fixed header + remaining length)
    let mut buf = [0u8; 1];
    let n = stream.read(&mut buf)?;
    if n == 0 {
        return Ok(None);
    }
    if buf[0] != 0x20 {
        return Ok(None);
    }
    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Ok(None);
        }
        remaining |= ((buf[0] & 0x7F) as u32) << shift;
        if buf[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 21 {
            return Ok(None);
        }
    }
    let mut rest = vec![0u8; remaining as usize];
    stream.read_exact(&mut rest)?;
    if rest.len() < 2 {
        return Ok(None);
    }
    // rest[0] = ack flags (bit 0 = session present), rest[1] = reason code
    Ok(Some(((rest[0] & 0x01) != 0, rest[1])))
}

/// Test basic MQTT 5.0 connect with session expiry
pub struct ConnectV5Test;

impl TestCase for ConnectV5Test {
    fn name(&self) -> &str {
        "connect_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "connect-v5-test",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                Some(3600), // session_expiry_interval
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());

            // Verify connack has v5 fields
            let ack = client.connack();
            if ack.reason_code != ConnectAckReason::Success {
                return Err(anyhow::anyhow!("CONNACK failed: {:?}", ack.reason_code));
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
}

/// Test MQTT 5.0 reason code validation
pub struct ConnectV5ReasonCodeTest;

impl TestCase for ConnectV5ReasonCodeTest {
    fn name(&self) -> &str {
        "connect_v5_reason_codes"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-reason-code-test",
                ctx.config.connect_timeout,
            )
            .await?;

            // CONNACK should have reason code 0 (Success)
            let ack = client.connack();
            if ack.reason_code != ConnectAckReason::Success {
                return Err(anyhow::anyhow!("CONNACK reason code not success: {:?}", ack.reason_code));
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
}

/// Positive: Session Present = 0 on a fresh connection (clean_start = 1).
pub struct ConnectV5SessionPresentFreshTest;

impl TestCase for ConnectV5SessionPresentFreshTest {
    fn name(&self) -> &str {
        "connect_v5_session_present_fresh"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                &format!("v5-fresh-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(
                !client.connack().session_present,
                "session present must be 0 on a fresh clean_start=1 connection"
            );
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

/// Negative: a wrong protocol name (not "MQTT") must be rejected.
pub struct ConnectV5WrongProtocolNameTest;

impl TestCase for ConnectV5WrongProtocolNameTest {
    fn name(&self) -> &str {
        "connect_v5_wrong_protocol_name"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let packet = raw_connect_v5_bytes(b"MQTTP", 5, 0x02, 60, b"wrong-name", None);
        match raw_connect_v5_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some((_, code))) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v5",
                        start.elapsed(),
                        "broker accepted a CONNECT with an invalid protocol name".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v5", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: an unsupported protocol level must be rejected with CONNACK
/// reason 0x84 (Unsupported Protocol Version) or the connection closed.
pub struct ConnectV5UnsupportedLevelTest;

impl TestCase for ConnectV5UnsupportedLevelTest {
    fn name(&self) -> &str {
        "connect_v5_unsupported_level"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // "MQTT" with level 6 (invalid — not 3/4/5)
        let packet = raw_connect_v5_bytes(b"MQTT", 6, 0x02, 60, b"bad-level", None);
        match raw_connect_v5_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some((_, code))) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v5",
                        start.elapsed(),
                        "broker accepted protocol level 6".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v5", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: CONNECT with the reserved flag bit (bit 0) set must be rejected.
/// [MQTT-3.1.2-3]
pub struct ConnectV5ReservedFlagTest;

impl TestCase for ConnectV5ReservedFlagTest {
    fn name(&self) -> &str {
        "connect_v5_reserved_flag"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x03: clean start (0x02) + reserved bit 0 set (0x01)
        let packet = raw_connect_v5_bytes(b"MQTT", 5, 0x03, 60, b"reserved-flag", None);
        match raw_connect_v5_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some((_, code))) => {
                if code == 0 {
                    TestResult::failed(
                        self.name(),
                        "functional_v5",
                        start.elapsed(),
                        "broker accepted CONNECT with reserved flag bit set".into(),
                    )
                } else {
                    TestResult::passed(self.name(), "functional_v5", start.elapsed())
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a second CONNECT on an established connection is a protocol
/// violation. [MQTT-3.1.0-2] requires the broker to close the Network
/// Connection and forbids answering the second CONNECT with a CONNACK; it may
/// announce the close with a DISCONNECT first (MQTT 5.0 section 4.13.1).
pub struct ConnectV5SecondConnectTest;

impl TestCase for ConnectV5SecondConnectTest {
    fn name(&self) -> &str {
        "connect_v5_second_connect"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("v5-second-{uid}");
            let mut stream = TcpStream::connect(&ctx.config.broker_addr)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;

            // First CONNECT
            let first = raw_connect_v5_bytes(b"MQTT", 5, 0x02, 60, cid.as_bytes(), None);
            stream.write_all(&first)?;
            stream.flush()?;
            // Read the full CONNACK (variable length in v5)
            let mut buf = [0u8; 1];
            let n = stream.read(&mut buf)?;
            if n == 0 || buf[0] != PKT_CONNACK {
                return Err(anyhow::anyhow!("first CONNECT: no CONNACK"));
            }
            let mut remaining: u32 = 0;
            let mut shift = 0u32;
            loop {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    return Err(anyhow::anyhow!("first CONNECT: truncated"));
                }
                remaining |= ((buf[0] & 0x7F) as u32) << shift;
                if buf[0] & 0x80 == 0 {
                    break;
                }
                shift += 7;
                if shift > 21 {
                    return Err(anyhow::anyhow!("first CONNECT: malformed length"));
                }
            }
            let mut rest = vec![0u8; remaining as usize];
            stream.read_exact(&mut rest)?;
            if rest.len() < 2 || rest[1] != 0 {
                return Err(anyhow::anyhow!(
                    "first CONNECT failed: reason {:02x?}",
                    &rest[..rest.len().min(4)]
                ));
            }

            // Second CONNECT must be treated as a protocol violation
            let second = raw_connect_v5_bytes(b"MQTT", 5, 0x02, 60, cid.as_bytes(), None);
            stream.write_all(&second)?;
            stream.flush()?;

            // The broker must close the connection, and it must not answer the
            // second CONNECT with another CONNACK. [MQTT-3.1.0-2] requires the
            // close; MQTT 5.0 section 4.13.1 lets the Server say why first, so a
            // DISCONNECT followed by EOF is the conformant sequence and a bare EOF
            // is accepted too. A read that merely times out, on the other hand,
            // means the connection was left open — that fails.
            let n = stream.read(&mut buf)?;
            if n == 0 {
                return Ok(()); // closed without a word — acceptable
            }
            match buf[0] {
                PKT_CONNACK => Err(anyhow::anyhow!(
                    "broker answered the second CONNECT with another CONNACK [MQTT-3.1.0-2]"
                )),
                PKT_DISCONNECT => {
                    let pkt = read_packet_body(&mut stream, buf[0])?;
                    let code = pkt.get(2).copied().unwrap_or(0x00);
                    match stream.read(&mut buf) {
                        Ok(0) => Ok(()), // DISCONNECT then EOF — conformant
                        Ok(n) => Err(anyhow::anyhow!(
                            "after the second CONNECT the broker sent DISCONNECT 0x{code:02X} but \
                             left the connection open ({n} more bytes: {:02x?}) [MQTT-3.1.0-2]",
                            &buf[..n]
                        )),
                        Err(e) => Err(anyhow::anyhow!(
                            "after the second CONNECT the broker sent DISCONNECT 0x{code:02X} but \
                             did not close the connection ({e:?}) [MQTT-3.1.0-2]"
                        )),
                    }
                }
                other => Err(anyhow::anyhow!(
                    "unexpected first byte 0x{other:02X} after the second CONNECT [MQTT-3.1.0-2]"
                )),
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: a client id longer than `max_clientid_len` must be rejected with
/// reason 0x85 (Client Identifier Not Valid) — or accepted when the broker's
/// configured limit permits it.
pub struct ConnectV5ClientIdTooLongTest;

impl TestCase for ConnectV5ClientIdTooLongTest {
    fn name(&self) -> &str {
        "connect_v5_client_id_too_long"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let long_id = vec![b'a'; 65535]; // max u16 length field value
        let packet = raw_connect_v5_bytes(b"MQTT", 5, 0x02, 60, &long_id, None);
        match raw_connect_v5_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some((_, code))) => {
                if code == 0 {
                    // Broker accepted it (max_clientid_len is 65535) — boundary pass
                    TestResult::passed(self.name(), "functional_v5", start.elapsed())
                } else if code == 0x85 {
                    TestResult::passed(self.name(), "functional_v5", start.elapsed())
                } else {
                    TestResult::failed(
                        self.name(),
                        "functional_v5",
                        start.elapsed(),
                        format!("unexpected CONNACK reason code {code}"),
                    )
                }
            }
            Ok(None) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a CONNECT with an Authentication Method must be rejected with
/// reason 0x9C (Bad Authentication Method), because RMQTT does not implement
/// enhanced authentication (v5 §4.12).
pub struct ConnectV5AuthMethodRejectedTest;

impl TestCase for ConnectV5AuthMethodRejectedTest {
    fn name(&self) -> &str {
        "connect_v5_auth_method_rejected"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x02: clean start, with auth method "SCRAM-SHA-1"
        let packet = raw_connect_v5_bytes(b"MQTT", 5, 0x02, 60, b"auth-test", Some(b"SCRAM-SHA-1"));
        match raw_connect_v5_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some((_, code))) => {
                if code == 0x8C {
                    // 0x8C = Bad Authentication Method (RMQTT does not support
                    // enhanced authentication)
                    TestResult::passed(self.name(), "functional_v5", start.elapsed())
                } else {
                    TestResult::failed(
                        self.name(),
                        "functional_v5",
                        start.elapsed(),
                        format!("expected Bad Authentication Method (0x8C), got reason {code}"),
                    )
                }
            }
            Ok(None) => TestResult::failed(
                self.name(),
                "functional_v5",
                start.elapsed(),
                "broker closed the connection instead of returning 0x8C".into(),
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

// ---------------------------------------------------------------------------
// P0 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// Expect a CONNECT to be rejected: either the connection is closed without a
/// successful CONNACK, or a CONNACK with a non-zero reason code is returned.
fn expect_connect_rejected(ctx: &TestContext, packet: &[u8]) -> anyhow::Result<()> {
    match raw_connect_v5_exchange(&ctx.config.broker_addr, packet) {
        Ok(None) => Ok(()),
        Ok(Some((_, code))) if code != 0 => Ok(()),
        Ok(Some((_, code))) => {
            Err(anyhow::anyhow!("broker accepted the illegal CONNECT (CONNACK reason 0x{code:02X})"))
        }
        Err(e) => Err(e),
    }
}

/// Negative: CONNECT with Will Flag = 0 but Will QoS = 1 is a Malformed
/// Packet. [MQTT-3.1.2-11 / MQTT-3.1.2-13]
///
/// Known broker defect (registered as a GitHub issue): expected-fail.
pub struct ConnectV5WillFlagZeroButQosSetTest;

impl TestCase for ConnectV5WillFlagZeroButQosSetTest {
    fn name(&self) -> &str {
        "connect_v5_will_flag_zero_but_qos_set"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // flags = 0x0A: clean start (0x02) + Will QoS = 1 (0x08), Will Flag = 0
        let packet = raw_connect_v5_bytes(b"MQTT", 5, 0x0A, 60, b"willflag-qos", None);
        match expect_connect_rejected(ctx, &packet) {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    fn expectation(&self) -> crate::framework::testcase::Expectation {
        crate::framework::testcase::Expectation::ExpectedFail
    }
}

/// Negative: CONNECT with Will Flag = 0 but Will Retain = 1 is a Malformed
/// Packet. [MQTT-3.1.2-11 / MQTT-3.1.2-12]
///
/// Known broker defect (registered as a GitHub issue): expected-fail.
pub struct ConnectV5WillFlagZeroButRetainSetTest;

impl TestCase for ConnectV5WillFlagZeroButRetainSetTest {
    fn name(&self) -> &str {
        "connect_v5_will_flag_zero_but_retain_set"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // flags = 0x22: clean start (0x02) + Will Retain (0x20), Will Flag = 0
        let packet = raw_connect_v5_bytes(b"MQTT", 5, 0x22, 60, b"willflag-retain", None);
        match expect_connect_rejected(ctx, &packet) {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    fn expectation(&self) -> crate::framework::testcase::Expectation {
        crate::framework::testcase::Expectation::ExpectedFail
    }
}
