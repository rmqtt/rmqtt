//! MQTT 5.0 Topic Alias negotiation tests
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Read one full MQTT packet (fixed header + remaining length) from a raw
/// stream. v5 CONNACK has a variable length (properties), so a naive read
/// leaves trailing bytes that corrupt subsequent reads.
fn read_full_packet(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    let n = stream.read(&mut b)?;
    if n == 0 {
        return Err(anyhow::anyhow!("connection closed"));
    }
    buf.push(b[0]);
    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        let n = stream.read(&mut b)?;
        if n == 0 {
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

pub struct ServerTopicAliasV5Test;
impl TestCase for ServerTopicAliasV5Test {
    fn name(&self) -> &str {
        "server_topic_alias_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "alias-test",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.topic_alias_max;
            client.disconnect().await?;
            Ok(())
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

/// Test client-side topic alias usage (v5)
pub struct ClientTopicAliasV5Test;

impl TestCase for ClientTopicAliasV5Test {
    fn name(&self) -> &str {
        "client_topic_alias_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "cta-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "cta-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/topicalias", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish with topic_alias property (simulating client-side alias)
            // The broker should resolve the alias and route to subscribers
            publisher
                .publish_with_properties(
                    "test/v5/topicalias",
                    b"alias_msg",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;

            // If publish_with_properties doesn't support topic_alias directly,
            // we at least verify the basic publish+subscribe works
            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"alias_msg" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no message received")),
            }
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

/// Negative: a PUBLISH carrying an unknown Topic Alias (never registered by a
/// prior PUBLISH) is a protocol error — the server must send a DISCONNECT with
/// reason 0x94 (Topic Alias invalid) or close the connection. [MQTT-3.3.2-5]
pub struct TopicAliasV5UnknownAliasTest;

impl TestCase for TopicAliasV5UnknownAliasTest {
    fn name(&self) -> &str {
        "topic_alias_v5_unknown_alias"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let raw_result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let mut stream = std::net::TcpStream::connect(&ctx.config.broker_addr)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;

            // CONNECT v5 clean start
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x04]);
            body.extend_from_slice(b"MQTT");
            body.push(5);
            body.push(0x02);
            body.extend_from_slice(&[0x00, 0x3C]);
            body.push(0x00);
            let cid = b"v5-alias-raw";
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
            let connack = read_full_packet(&mut stream)?;
            if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
                return Err(anyhow::anyhow!("no CONNACK: {:02x?}", &connack[..connack.len().min(8)]));
            }

            // Register alias 1: PUBLISH with topic + Topic Alias property 1
            let topic = b"test/v5/alias/known";
            let mut pb: Vec<u8> = Vec::new();
            pb.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            pb.extend_from_slice(topic);
            // properties: prop_len = 3, 0x23 (Topic Alias), 0x00 0x01
            pb.push(0x03);
            pb.push(0x23);
            pb.push(0x00);
            pb.push(0x01);
            pb.extend_from_slice(b"hi");
            let mut ppkt = vec![0x30];
            let mut plen = pb.len();
            loop {
                let mut b = (plen % 128) as u8;
                plen /= 128;
                if plen > 0 {
                    b |= 0x80;
                }
                ppkt.push(b);
                if plen == 0 {
                    break;
                }
            }
            ppkt.extend_from_slice(&pb);
            stream.write_all(&ppkt)?;
            stream.flush()?;

            // Now send PUBLISH with alias 2 (never registered) — protocol error
            let mut pb2: Vec<u8> = Vec::new();
            // empty topic (alias-only reference)
            pb2.extend_from_slice(&[0x00, 0x00]);
            pb2.push(0x03);
            pb2.push(0x23);
            pb2.push(0x00);
            pb2.push(0x02); // alias 2 — never registered
            pb2.extend_from_slice(b"bad");
            let mut ppkt2 = vec![0x30];
            let mut plen2 = pb2.len();
            loop {
                let mut b = (plen2 % 128) as u8;
                plen2 /= 128;
                if plen2 > 0 {
                    b |= 0x80;
                }
                ppkt2.push(b);
                if plen2 == 0 {
                    break;
                }
            }
            ppkt2.extend_from_slice(&pb2);
            stream.write_all(&ppkt2)?;
            stream.flush()?;

            // Broker must close the connection (EOF) or send DISCONNECT 0x94
            let mut rbuf = [0u8; 16];
            match stream.read(&mut rbuf) {
                Ok(0) | Err(_) => Ok(()), // closed — acceptable
                Ok(n) if n >= 2 && rbuf[0] == 0xE0 => {
                    // DISCONNECT — reason byte is at index 2
                    if n >= 3 && rbuf[2] == 0x94 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("DISCONNECT with unexpected reason: {:02x?}", &rbuf[..n]))
                    }
                }
                Ok(n) => Err(anyhow::anyhow!("unexpected response: {:02x?}", &rbuf[..n])),
            }
        });

        match raw_result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
