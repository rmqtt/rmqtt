//! MQTT v5 conformance: Will Retain vs Retain Available (MQTT-3.2.2-13)
//!
//! GitHub issue #457 (https://github.com/rmqtt/rmqtt/issues/457):
//! rmqtt advertises `Retain Available = 0` in its CONNACK (retain feature
//! disabled) yet accepts a CONNECT whose Will Message has `Will Retain = 1`,
//! returning reason `0x00`. The specification requires the Server to reject
//! such a connection with CONNACK reason `0x9A` ("Retain not supported") and
//! close the Network Connection.
//!
//! Reproduction (run the harness with a config that does NOT load the
//! rmqtt-retainer plugin, see `rmqtt-test/configs/retain-disabled/rmqtt.toml`):
//!
//! ```text
//! mqtt_harness \
//!   --binary target/release/rmqttd \
//!   --config rmqtt-test/configs/retain-disabled/rmqtt.toml \
//!   --workspace . \
//!   --suites functional_v5 \
//!   --workers 1
//! ```
//!
//! Expected result today (before the fix): the test FAILS — the broker accepts
//! the `Will Retain = 1` connection with reason 0x00 instead of 0x9A, which is
//! exactly the issue being reproduced.

use std::time::{Duration, Instant};

use bytes::Bytes;
use bytestring::ByteString;
use rmqtt_codec::types::QoS;
use rmqtt_codec::v5::{Connect, ConnectAckReason, LastWill, Packet};
use uuid::Uuid;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::transport::tcp_v5::{packet_name_v5, TcpTransportV5Reader};

/// Test outcome used to distinguish a real result from "not applicable".
enum Outcome {
    Ok,
    /// The server advertises Retain Available = 1, so the scenario does not
    /// apply (mirrors the "not applicable" branch of the issue's repro script).
    NotApplicable,
}

/// Result of one raw CONNECT probe.
struct Probe {
    reason: ConnectAckReason,
    retain_available: bool,
    reader: TcpTransportV5Reader,
}

/// Open a raw TCP connection, send a CONNECT (optionally with a Will Message)
/// and read the CONNACK. Returns the reason code, the advertised
/// `retain_available` property and the reader half (for close verification).
async fn probe_connect(ctx: &TestContext, will_retain: Option<bool>) -> Result<Probe, anyhow::Error> {
    let (mut reader, mut writer) =
        crate::transport::tcp_v5::connect(&ctx.config.broker_addr, ctx.config.connect_timeout).await?;

    let last_will = will_retain.map(|retain| LastWill {
        qos: QoS::AtMostOnce,
        retain,
        topic: ByteString::from("will/retain-probe"),
        message: Bytes::from_static(b"bye"),
        will_delay_interval_sec: None,
        correlation_data: None,
        message_expiry_interval: None,
        content_type: None,
        user_properties: Vec::new(),
        is_utf8_payload: None,
        response_topic: None,
    });

    let connect = Connect {
        clean_start: true,
        keep_alive: 60,
        last_will,
        client_id: ByteString::from(format!("wr-probe-{}", Uuid::new_v4().as_simple())),
        ..Default::default()
    };

    writer.send_packet(&Packet::Connect(Box::new(connect))).await?;

    let pkt = tokio::time::timeout(Duration::from_secs(5), reader.read_packet())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for CONNACK"))??;

    match pkt {
        Packet::ConnectAck(ack) => {
            Ok(Probe { reason: ack.reason_code, retain_available: ack.retain_available, reader })
        }
        other => Err(anyhow::anyhow!("expected CONNACK, got {:?}", packet_name_v5(&other))),
    }
}

/// After a rejected CONNECT the Server MUST close the Network Connection
/// (MQTT-3.2.2-13). A clean EOF (or no further data within a short window) is
/// acceptable; receiving an extra MQTT packet is a violation.
async fn assert_connection_closed(reader: &mut TcpTransportV5Reader) -> Result<(), anyhow::Error> {
    match tokio::time::timeout(Duration::from_secs(2), reader.read_packet()).await {
        Ok(Ok(pkt)) => Err(anyhow::anyhow!(
            "expected the Network Connection to be closed after rejection, \
             but received an extra packet: {:?}",
            packet_name_v5(&pkt)
        )),
        Ok(Err(_)) => Ok(()), // EOF: connection closed by broker
        Err(_) => Ok(()),     // lenient: no further data within the window
    }
}

/// GitHub issue #457 reproduction: a CONNECT carrying `Will Retain = 1` must
/// be rejected with `0x9A` (RetainNotSupported) when the server advertises
/// `Retain Available = 0` (MQTT-3.2.2-13).
pub struct WillRetainRejectedWhenRetainUnavailableV5Test;

impl TestCase for WillRetainRejectedWhenRetainUnavailableV5Test {
    fn name(&self) -> &str {
        "v5_will_retain_rejected_when_retain_unavailable"
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: Result<Outcome, anyhow::Error> = rt.block_on(async {
            // 1) Probe: plain CONNECT (no will) -> inspect CONNACK `retain_available`.
            let p = probe_connect(ctx, None).await?;
            if p.retain_available {
                // Server supports retained messages: scenario not applicable.
                return Ok(Outcome::NotApplicable);
            }

            // 2) CONNECT with Will Retain = 1 MUST be rejected with 0x9A.
            let p = probe_connect(ctx, Some(true)).await?;
            if p.reason != ConnectAckReason::RetainNotSupported {
                // Actual rmqtt behaviour (issue #457): accepted with 0x00.
                return Err(anyhow::anyhow!(
                    "MQTT-3.2.2-13 violation: server advertises Retain Available = 0 \
                     but accepted CONNECT with Will Retain = 1, CONNACK reason = {:?} (0x{:02X}); \
                     expected RetainNotSupported (0x9A)",
                    p.reason,
                    u8::from(p.reason)
                ));
            }

            // 3) After the rejection the server must close the connection.
            let mut reader = p.reader;
            assert_connection_closed(&mut reader).await?;

            // 4) Control: CONNECT with Will Retain = 0 must still be accepted.
            let p = probe_connect(ctx, Some(false)).await?;
            if p.reason != ConnectAckReason::Success {
                return Err(anyhow::anyhow!(
                    "control failed: CONNECT with Will Retain = 0 (retain unavailable) \
                     should be accepted, got {:?} (0x{:02X})",
                    p.reason,
                    u8::from(p.reason)
                ));
            }

            Ok(Outcome::Ok)
        });

        match result {
            Ok(Outcome::Ok) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Outcome::NotApplicable) => TestResult::skipped(
                self.name(),
                "functional_v5",
                start.elapsed(),
                "server advertises Retain Available = 1 (retain feature enabled); \
                 scenario not applicable",
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
}
