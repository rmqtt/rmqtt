//! Boundary and edge case tests (v3.1.1 client)

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// MQTT port of the self-managed remaining-length broker (avoids the harness
/// 1883, the cluster tests 1886-1890 and the session-expiry-cleanup 1884).
const RL_BROKER_ADDR: &str = "127.0.0.1:1891";
const RL_NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

/// Spawn the self-managed remaining-length broker (`configs/rl-boundary/`,
/// max_packet_size = 512 MB). Returns the node handle; dropping it kills the
/// broker. The default harness broker stays untouched on 1883.
fn spawn_rl_broker() -> Result<(ClusterNode, PathBuf), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let config = crate::tests::config_path("rl-boundary");
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join("rl-boundary-node.log");
    let mut node = ClusterNode::new(config, RL_BROKER_ADDR, log_file);
    node.spawn(&binary)?;
    if !node.wait_healthy(RL_NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("rl-boundary broker did not become healthy"));
    }
    Ok((node, binary))
}

/// Encode a MQTT remaining length into 1..=4 bytes (per MQTT-2.2.3).
fn encode_remaining_length(len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut l = len;
    loop {
        let mut b = (l % 128) as u8;
        l /= 128;
        if l > 0 {
            b |= 0x80;
        }
        out.push(b);
        if l == 0 {
            break;
        }
    }
    out
}

/// Build a raw QoS 0 PUBLISH with a hand-encoded remaining length. The
/// remaining length equals `2 + topic.len() + payload.len()`; passing a
/// payload sized so the total lands exactly on a var-int boundary exercises
/// the 1/2/3/4-byte remaining-length encodings.
fn raw_publish_qos0(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    body.extend_from_slice(topic.as_bytes());
    body.extend_from_slice(payload);

    let mut pkt = vec![0x30]; // PUBLISH, QoS 0
    pkt.extend_from_slice(&encode_remaining_length(body.len()));
    pkt.extend_from_slice(&body);
    pkt
}

/// Open a raw v3.1.1 connection and consume the CONNACK. Returns the stream.
fn raw_connect_v311(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(4);
    body.push(0x02); // clean session
    body.extend_from_slice(&[0x00, 0x3C]);
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);

    let mut pkt = vec![0x10];
    pkt.extend_from_slice(&encode_remaining_length(body.len()));
    pkt.extend_from_slice(&body);

    stream.write_all(&pkt)?;
    stream.flush()?;
    let mut buf = [0u8; 8];
    let n = stream.read(&mut buf)?;
    if n < 4 || buf[0] != 0x20 || buf[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &buf[..n]));
    }
    Ok(stream)
}

/// Test connecting with the maximum allowed client ID length (23 chars per MQTT spec)
pub struct MaxClientIdTest;

impl TestCase for MaxClientIdTest {
    fn name(&self) -> &str {
        "boundary_max_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let client_id = "A".repeat(23);
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &client_id,
                ctx.config.connect_timeout,
            )
            .await?;
            client.disconnect().await?;
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Test subscribing and publishing with a 200-character topic
pub struct LongTopicTest;

impl TestCase for LongTopicTest {
    fn name(&self) -> &str {
        "boundary_long_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-long-topic-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-long-topic-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"long topic msg", QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"long topic msg" && m.topic.as_bytes() == topic.as_bytes() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "unexpected message: topic={}, payload={:?}",
                            m.topic,
                            m.payload
                        ))
                    }
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
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

/// Test publishing messages with empty (zero-byte) payload at all QoS levels
pub struct EmptyPayloadTest;

impl TestCase for EmptyPayloadTest {
    fn name(&self) -> &str {
        "boundary_empty_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            // QoS 0
            {
                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-pub-qos0",
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-sub-qos0",
                    ctx.config.connect_timeout,
                )
                .await?;

                let topic = "test/boundary/empty/qos0";
                subscriber.subscribe(topic, QoS::AtMostOnce).await?;

                tokio::time::sleep(Duration::from_millis(100)).await;

                publisher.publish(topic, b"", QoS::AtMostOnce, false).await?;

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                publisher.disconnect().await?;
                subscriber.disconnect().await?;

                match msg {
                    Some(m) => {
                        if !m.payload.is_empty() {
                            return Err(anyhow::anyhow!(
                                "QoS 0: expected empty payload, got {} bytes",
                                m.payload.len()
                            ));
                        }
                    }
                    None => return Err(anyhow::anyhow!("QoS 0: no message received within timeout")),
                }
            }

            // QoS 1
            {
                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-pub-qos1",
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-sub-qos1",
                    ctx.config.connect_timeout,
                )
                .await?;

                let topic = "test/boundary/empty/qos1";
                subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

                tokio::time::sleep(Duration::from_millis(100)).await;

                publisher.publish(topic, b"", QoS::AtLeastOnce, false).await?;

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                publisher.disconnect().await?;
                subscriber.disconnect().await?;

                match msg {
                    Some(m) => {
                        if !m.payload.is_empty() {
                            return Err(anyhow::anyhow!(
                                "QoS 1: expected empty payload, got {} bytes",
                                m.payload.len()
                            ));
                        }
                    }
                    None => return Err(anyhow::anyhow!("QoS 1: no message received within timeout")),
                }
            }

            // QoS 2
            {
                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-pub-qos2",
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-sub-qos2",
                    ctx.config.connect_timeout,
                )
                .await?;

                let topic = "test/boundary/empty/qos2";
                subscriber.subscribe(topic, QoS::ExactlyOnce).await?;

                tokio::time::sleep(Duration::from_millis(100)).await;

                publisher.publish(topic, b"", QoS::ExactlyOnce, false).await?;

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                publisher.disconnect().await?;
                subscriber.disconnect().await?;

                match msg {
                    Some(m) => {
                        if !m.payload.is_empty() {
                            return Err(anyhow::anyhow!(
                                "QoS 2: expected empty payload, got {} bytes",
                                m.payload.len()
                            ));
                        }
                    }
                    None => return Err(anyhow::anyhow!("QoS 2: no message received within timeout")),
                }
            }

            Ok(())
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

/// Test publishing a ~64KB payload at QoS 1
pub struct LargePayloadTest;

impl TestCase for LargePayloadTest {
    fn name(&self) -> &str {
        "boundary_large_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-large-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-large-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/boundary/large";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            let payload = vec![0x42u8; 65536];
            publisher.publish(topic, &payload, QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(10)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.len() != 65536 {
                        return Err(anyhow::anyhow!("expected 65536 bytes, got {} bytes", m.payload.len()));
                    }
                    if m.payload[0] != 0x42 {
                        return Err(anyhow::anyhow!(
                            "first byte mismatch: expected 0x42, got 0x{:02x}",
                            m.payload[0]
                        ));
                    }
                    if m.payload[65535] != 0x42 {
                        return Err(anyhow::anyhow!(
                            "last byte mismatch: expected 0x42, got 0x{:02x}",
                            m.payload[65535]
                        ));
                    }
                    Ok(())
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
            }
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

/// Test publishing to a topic with special characters (Unicode)
pub struct SpecialCharsTopicTest;

impl TestCase for SpecialCharsTopicTest {
    fn name(&self) -> &str {
        "boundary_special_chars_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-special-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-special-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/special/你好/世界";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"unicode topic", QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"unicode topic" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload))
                    }
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
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

/// Test rapid subscribe/unsubscribe cycles and verify no crashes
pub struct RapidSubscribeTest;

impl TestCase for RapidSubscribeTest {
    fn name(&self) -> &str {
        "boundary_rapid_subscribe"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let mut client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-rapid-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/boundary/rapid/sub";

            for i in 0..10 {
                client.subscribe(topic, QoS::AtLeastOnce).await?;
                client.unsubscribe(topic).await?;
                if !client.is_connected() {
                    return Err(anyhow::anyhow!("client disconnected during iteration {}", i));
                }
            }

            // Final subscribe, publish, and verify delivery
            client.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-rapid-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            publisher.publish(topic, b"after rapid sub/unsub", QoS::AtLeastOnce, false).await?;

            let msg = client.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            client.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"after rapid sub/unsub" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload after rapid cycles: {:?}", m.payload))
                    }
                }
                None => Err(anyhow::anyhow!("no message received after rapid sub/unsub cycles")),
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

// ---------------------------------------------------------------------------
// P0 conformance gap fill (G2): remaining-length var-int encoding boundaries
// ---------------------------------------------------------------------------

/// Boundary: a remaining length of 268435455 (the 4-byte var-int maximum,
/// MQTT-2.2.3) is a *valid* encoding — the broker must accept and parse it
/// rather than closing the connection (which is the correct response only
/// for the *illegal* 5-byte encoding covered in
/// `protocol_error_v311_bad_remaining_length`).
///
/// Runs against a self-managed broker (`configs/rl-boundary/`) because the
/// default harness broker caps packets at 1 MB, which would reject the
/// 256 MB declaration before the encoding could be validated.
pub struct RemainingLengthMaxV311Test;

impl TestCase for RemainingLengthMaxV311Test {
    fn name(&self) -> &str {
        "boundary_remaining_length_max_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let verdict = (|| -> anyhow::Result<()> {
            let (_node, _binary) = spawn_rl_broker()?;
            let mut stream = raw_connect_v311(RL_BROKER_ADDR, "rl-max")?;
            // PINGREQ whose remaining length is the 4-byte maximum:
            // 0xFF 0xFF 0xFF 0x7F = 268435455. Sending the header with no body
            // means the broker parses the (valid) encoding and keeps waiting
            // for the body — it must NOT treat the encoding itself as an error.
            let pkt = [0xC0u8, 0xFF, 0xFF, 0xFF, 0x7F];
            stream.write_all(&pkt)?;
            stream.flush()?;

            // If the broker closes the connection, the valid encoding was
            // rejected — fail. If it stays open (read timeout), it accepted
            // the max remaining length.
            let mut buf = [0u8; 16];
            match stream.read(&mut buf) {
                Ok(0) => Err(anyhow::anyhow!(
                    "broker closed the connection on the valid 4-byte max remaining length \
                     268435455 [MQTT-2.2.3]"
                )),
                Ok(n) => Err(anyhow::anyhow!("unexpected data while waiting for body: {:02x?}", &buf[..n])),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(()),
                Err(e) => Err(anyhow::anyhow!("read error: {e}")),
            }
        })();

        match verdict {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: PUBLISH remaining-length values that cross the var-int
/// 1→2→3→4-byte encoding thresholds (127/128, 16383/16384, 2097151/2097152)
/// must round-trip intact. [MQTT-2.2.3]
///
/// A raw publisher hand-encodes the remaining length so the test does not
/// share the broker's codec on the send side. Runs against a self-managed
/// broker (`configs/rl-boundary/`) because the 4-byte boundary payloads
/// (~2 MB) exceed the default harness broker's 1 MB packet cap.
pub struct RemainingLengthTransitionV311Test;

impl TestCase for RemainingLengthTransitionV311Test {
    fn name(&self) -> &str {
        "remaining_length_transition_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_rl_broker()?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                RL_BROKER_ADDR,
                "rl-transition-sub",
                Duration::from_secs(10),
            )
            .await?;
            let topic = "rl/b"; // 4 bytes -> remaining length = 6 + payload.len()
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let mut stream = raw_connect_v311(RL_BROKER_ADDR, "rl-transition-pub")?;

            // boundary targets and the payload size that lands exactly on them
            let targets: [(usize, usize); 6] = [
                (127, 121),         // 1 -> 2 bytes
                (128, 122),         // 2 bytes
                (16383, 16377),     // 2 -> 3 bytes
                (16384, 16378),     // 3 bytes
                (2097151, 2097145), // 3 -> 4 bytes
                (2097152, 2097146), // 4 bytes
            ];
            let mut expected = Vec::new();
            for (rl, plen) in targets {
                assert_eq!(2 + topic.len() + plen, rl, "test construction error");
                let payload = vec![0x5Au8; plen];
                let pkt = raw_publish_qos0(topic, &payload);
                stream.write_all(&pkt)?;
                stream.flush()?;
                expected.push((rl, plen));
            }
            let _ = stream.shutdown(std::net::Shutdown::Both);

            // Collect 6 messages and verify each length matches in order.
            for (rl, plen) in expected {
                let msg = subscriber
                    .recv_message_timeout(Duration::from_secs(5))
                    .await
                    .ok_or_else(|| anyhow::anyhow!("missing message for remaining length {rl}"))?;
                if msg.payload.len() != plen {
                    return Err(anyhow::anyhow!(
                        "remaining length {rl}: expected payload {} bytes, got {}",
                        plen,
                        msg.payload.len()
                    ));
                }
                if msg.payload.first() != Some(&0x5A) {
                    return Err(anyhow::anyhow!("remaining length {rl}: first payload byte corrupted"));
                }
            }
            let _ = subscriber.disconnect().await;
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
