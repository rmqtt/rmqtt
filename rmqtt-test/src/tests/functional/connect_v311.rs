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

/// Build a raw CONNECT packet with optional Will / User Name / Password.
///
/// The CONNECT payload field order is fixed by [MQTT-3.1.3-1]:
/// Client Identifier -> Will Topic -> Will Message -> User Name -> Password.
/// Optional fields are appended only when their `Option` is `Some`; callers
/// must ensure the connect flags match the supplied fields.
#[allow(clippy::too_many_arguments)]
fn raw_connect_full(
    protocol_name: &[u8],
    protocol_level: u8,
    connect_flags: u8,
    keep_alive: u16,
    client_id: &[u8],
    will_topic: Option<&[u8]>,
    will_message: Option<&[u8]>,
    username: Option<&[u8]>,
    password: Option<&[u8]>,
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&(protocol_name.len() as u16).to_be_bytes());
    body.extend_from_slice(protocol_name);
    body.push(protocol_level);
    body.push(connect_flags);
    body.extend_from_slice(&keep_alive.to_be_bytes());
    // Client Identifier
    body.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
    body.extend_from_slice(client_id);
    // Will Topic / Will Message
    if let Some(wt) = will_topic {
        body.extend_from_slice(&(wt.len() as u16).to_be_bytes());
        body.extend_from_slice(wt);
    }
    if let Some(wm) = will_message {
        body.extend_from_slice(&(wm.len() as u16).to_be_bytes());
        body.extend_from_slice(wm);
    }
    // User Name / Password
    if let Some(u) = username {
        body.extend_from_slice(&(u.len() as u16).to_be_bytes());
        body.extend_from_slice(u);
    }
    if let Some(p) = password {
        body.extend_from_slice(&(p.len() as u16).to_be_bytes());
        body.extend_from_slice(p);
    }

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

// ---------------------------------------------------------------------------
// MQTT 3.1.1 CONNECT flag / encoding conformance (P0 gap fill)
// ---------------------------------------------------------------------------

/// Well-formed-UTF-8 check helper: run one CONNECT and assert the broker
/// either rejects it (CONNACK code != 0) or closes the connection without a
/// CONNACK — both are conformant outcomes for a malformed CONNECT.
fn assert_connect_rejected_or_closed(
    name: &str,
    ctx: &TestContext,
    start: Instant,
    packet: &[u8],
) -> TestResult {
    match raw_connect_exchange(&ctx.config.broker_addr, packet) {
        Ok(Some(0)) => TestResult::failed(
            name,
            "functional_v311",
            start.elapsed(),
            "broker accepted a malformed CONNECT".into(),
        ),
        Ok(Some(code)) => TestResult::passed_with_note(
            name,
            "functional_v311",
            start.elapsed(),
            &format!("broker rejected CONNECT with return code 0x{code:02x}"),
        ),
        Ok(None) => TestResult::passed(name, "functional_v311", start.elapsed()),
        Err(e) => TestResult::failed(name, "functional_v311", start.elapsed(), e.to_string()),
    }
}

/// Negative: a CONNECT whose Client Identifier contains invalid UTF-8 must
/// not be accepted; the broker rejects it or closes the connection.
/// [MQTT-1.5.3-1] (UTF-8 string fields must be well-formed)
pub struct ConnectV311InvalidUtf8ClientIdTest;

impl TestCase for ConnectV311InvalidUtf8ClientIdTest {
    fn name(&self) -> &str {
        "connect_v311_invalid_utf8_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // overlong encoding of U+0000, a surrogate half, and a truncated
        // multi-byte sequence — all forbidden by [MQTT-1.5.3-1]
        let bad_ids: [&[u8]; 3] = [&[0xC0, 0x80], &[0xED, 0xA0, 0x80], &[0xE2, 0x82]];
        for bad in bad_ids {
            let packet = raw_connect_full(b"MQTT", 4, 0x02, 60, bad, None, None, None, None);
            let result = assert_connect_rejected_or_closed(self.name(), ctx, start, &packet);
            if !result.verdict.is_passed() {
                return result;
            }
        }
        TestResult::passed(self.name(), "functional_v311", start.elapsed())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: a CONNECT whose Will Topic contains invalid UTF-8 (Will Flag
/// set) must be rejected / the connection closed. [MQTT-1.5.3-1]
pub struct ConnectV311InvalidUtf8WillTopicTest;

impl TestCase for ConnectV311InvalidUtf8WillTopicTest {
    fn name(&self) -> &str {
        "connect_v311_invalid_utf8_will_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let bad_topics: [&[u8]; 3] = [&[0xC0, 0x80], &[0xED, 0xA0, 0x80], &[0xE2, 0x82]];
        for bad in bad_topics {
            // flags = 0x02 (clean) | 0x04 (will flag)
            let packet =
                raw_connect_full(b"MQTT", 4, 0x06, 60, b"utf8-will", Some(bad), Some(b"msg"), None, None);
            let result = assert_connect_rejected_or_closed(self.name(), ctx, start, &packet);
            if !result.verdict.is_passed() {
                return result;
            }
        }
        TestResult::passed(self.name(), "functional_v311", start.elapsed())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: a CONNECT whose User Name contains invalid UTF-8 (User Name
/// Flag set) must be rejected / the connection closed. [MQTT-1.5.3-1]
pub struct ConnectV311InvalidUtf8UsernameTest;

impl TestCase for ConnectV311InvalidUtf8UsernameTest {
    fn name(&self) -> &str {
        "connect_v311_invalid_utf8_username"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let bad_names: [&[u8]; 3] = [&[0xC0, 0x80], &[0xED, 0xA0, 0x80], &[0xE2, 0x82]];
        for bad in bad_names {
            // flags = 0x02 (clean) | 0x80 (user name flag)
            let packet = raw_connect_full(b"MQTT", 4, 0x82, 60, b"utf8-user", None, None, Some(bad), None);
            let result = assert_connect_rejected_or_closed(self.name(), ctx, start, &packet);
            if !result.verdict.is_passed() {
                return result;
            }
        }
        TestResult::passed(self.name(), "functional_v311", start.elapsed())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: Will QoS bits must be 0 when the Will Flag is 0.
/// [MQTT-3.1.2-13]
///
/// The codec rejected this case with `DecodeError::InvalidConnectFlags`
/// (2026-08-23); previously it only validated the CONNECT reserved bit, so
/// the broker accepted the malformed CONNECT.
pub struct ConnectV311WillFlagZeroButQosSetTest;

impl TestCase for ConnectV311WillFlagZeroButQosSetTest {
    fn name(&self) -> &str {
        "connect_v311_will_flag_zero_but_qos_set"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = 0x02 (clean) | 0x08 (Will QoS bit 1, Will Flag = 0)
        let pkt_qos = raw_connect_full(b"MQTT", 4, 0x0A, 60, b"will-qos-set", None, None, None, None);
        let r1 = assert_connect_rejected_or_closed(self.name(), ctx, start, &pkt_qos);
        if !r1.verdict.is_passed() {
            return r1;
        }

        // flags = 0x02 (clean) | 0x20 (Will Retain, Will Flag = 0)
        let pkt_retain = raw_connect_full(b"MQTT", 4, 0x22, 60, b"will-retain-set", None, None, None, None);
        let r2 = assert_connect_rejected_or_closed(self.name(), ctx, start, &pkt_retain);
        if !r2.verdict.is_passed() {
            return r2;
        }

        TestResult::passed(self.name(), "functional_v311", start.elapsed())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: Will QoS = 3 is illegal even when the Will Flag is set.
/// [MQTT-3.1.2-14]
pub struct ConnectV311WillQos3Test;

impl TestCase for ConnectV311WillQos3Test {
    fn name(&self) -> &str {
        "connect_v311_will_qos3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // flags = 0x02 (clean) | 0x04 (will) | 0x18 (Will QoS = 3)
        let packet = raw_connect_full(
            b"MQTT",
            4,
            0x1E,
            60,
            b"will-qos3",
            Some(b"will/topic"),
            Some(b"msg"),
            None,
            None,
        );
        assert_connect_rejected_or_closed(self.name(), ctx, start, &packet)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: User Name Flag = 1 without a User Name field in the payload.
/// [MQTT-3.1.2-18/19]
pub struct ConnectV311UsernameFlagMismatchTest;

impl TestCase for ConnectV311UsernameFlagMismatchTest {
    fn name(&self) -> &str {
        "connect_v311_username_flag_mismatch"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // flags = 0x02 (clean) | 0x80 (user name flag), but no user name bytes
        let packet = raw_connect_full(b"MQTT", 4, 0x82, 60, b"un-mismatch", None, None, None, None);
        assert_connect_rejected_or_closed(self.name(), ctx, start, &packet)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: Password Flag = 1 without a Password field, and Password Flag
/// with User Name Flag = 0. [MQTT-3.1.2-20/21/22]
pub struct ConnectV311PasswordFlagMismatchTest;

impl TestCase for ConnectV311PasswordFlagMismatchTest {
    fn name(&self) -> &str {
        "connect_v311_password_flag_mismatch"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // Password Flag = 1 while User Name Flag = 0 (flags = 0x02 | 0x40),
        // no password bytes either — violates [MQTT-3.1.2-22] outright.
        let pkt1 = raw_connect_full(b"MQTT", 4, 0x42, 60, b"pw-no-user", None, None, None, None);
        let r1 = assert_connect_rejected_or_closed(self.name(), ctx, start, &pkt1);
        if !r1.verdict.is_passed() {
            return r1;
        }

        // Password Flag = 1 with a user name but NO password field
        // (flags = 0x02 | 0x80 | 0x40 = 0xC2) — violates [MQTT-3.1.2-21].
        let pkt2 = raw_connect_full(b"MQTT", 4, 0xC2, 60, b"pw-missing", None, None, Some(b"user"), None);
        let r2 = assert_connect_rejected_or_closed(self.name(), ctx, start, &pkt2);
        if !r2.verdict.is_passed() {
            return r2;
        }

        TestResult::passed(self.name(), "functional_v311", start.elapsed())
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

// ---------------------------------------------------------------------------
// P3 optional / low-risk gap fill (G29, G30, 2026-08-23)
// ---------------------------------------------------------------------------

/// Boundary: a 65535-byte client id (the max u16 length-field value) is
/// accepted by RMQTT (`max_clientid_len` defaults to 65535). Mirrors the v3
/// `connect_v3_client_id_max_length` test for v3.1.1. [MQTT-3.1.3-5]
pub struct ConnectV311ClientId65535Test;

impl TestCase for ConnectV311ClientId65535Test {
    fn name(&self) -> &str {
        "connect_v311_client_id_65535"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let long_id = vec![b'a'; 65535]; // max u16 length field value
        let packet = raw_connect_bytes(b"MQTT", 4, 0x02, 60, &long_id);
        match raw_connect_exchange(&ctx.config.broker_addr, &packet) {
            Ok(Some(0)) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Ok(Some(code)) => TestResult::failed(
                self.name(),
                "functional_v311",
                start.elapsed(),
                format!("expected CONNACK 0x00 for 65535-byte client id, got 0x{code:02x}"),
            ),
            Ok(None) => TestResult::failed(
                self.name(),
                "functional_v311",
                start.elapsed(),
                "broker closed the connection for a 65535-byte client id".into(),
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: the CONNECT payload fields must appear in the fixed order
/// Client Identifier -> Will Topic -> Will Message -> User Name -> Password
/// [MQTT-3.1.3-1]. A payload whose fields are scrambled (here: username
/// declared with a length that runs past the end of the packet) must be
/// rejected or the connection closed.
pub struct ConnectV311PayloadOrderErrorTest;

impl TestCase for ConnectV311PayloadOrderErrorTest {
    fn name(&self) -> &str {
        "protocol_error_v311_connect_payload_order"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // flags = clean (0x02) | user name (0x80) | password (0x40) = 0xC2.
        // Payload is [client_id, username-with-overlong-length]: the broker
        // consumes client_id, then tries to read the username of 8 bytes but
        // only 2 remain -> InvalidLength -> the connection must be closed.
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(&[0x00, 0x04]);
        body.extend_from_slice(b"MQTT");
        body.push(4);
        body.push(0xC2);
        body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60
        body.extend_from_slice(&[0x00, 0x05]);
        body.extend_from_slice(b"cid01"); // client id
        body.extend_from_slice(&[0x00, 0x08]); // username: declares 8 bytes...
        body.extend_from_slice(b"ab"); // ...but only 2 are present
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

        assert_connect_rejected_or_closed(self.name(), ctx, start, &pkt)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
