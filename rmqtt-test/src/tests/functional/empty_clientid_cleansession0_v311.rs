//! MQTT 3.1.1 conformance: empty ClientId + CleanSession = 0 must be rejected (MQTT-3.1.3-6)
//!
//! The MQTT 3.1.1 specification requires:
//!
//! * [MQTT-3.1.3-6] If the ClientId is a zero-length byte string and CleanSession
//!   is 0, the Server MUST send a CONNACK with return code `0x02`
//!   (Identifier Rejected) and then close the Network Connection.
//! * [MQTT-3.1.3-7] If the ClientId is a zero-length byte string and CleanSession
//!   is 1, the Server MAY allow the Client to connect with a zero-byte ClientId
//!   and assign it a unique ClientId (Session Present is then 0).
//!
//! Current rmqtt behaviour (after the MQTT-3.1.3-6 fix in `rmqtt/src/v3.rs`):
//! the check exists at the codec layer — `rmqtt-codec/src/v3/decode.rs`
//! rejects an empty ClientId with `DecodeError::InvalidClientId` when
//! CleanSession is 0. `rmqtt/src/v3.rs` maps that decode error to a CONNACK
//! with return code `0x02` (Identifier Rejected). Before the fix it was
//! reported as `0x03` (ServiceUnavailable), which violated MQTT-3.1.3-6.
//!
//! This test protects the fix: it must PASS with the fix in place, and would
//! have FAILED before the fix (return code 0x03 instead of 0x02).

use std::time::{Duration, Instant};

use bytestring::ByteString;
use rmqtt_codec::types::{Protocol, MQTT_LEVEL_311};
use rmqtt_codec::v3::{Connect, ConnectAckReason, Packet};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::transport::tcp_v3::{packet_name_v3, TcpTransportV3Reader};

/// Result of one raw CONNECT probe.
struct Probe {
    reason: ConnectAckReason,
    reader: TcpTransportV3Reader,
}

/// Open a raw TCP connection and send a v3.1.1 CONNECT with an **empty**
/// ClientId and the given CleanSession flag, then read the CONNACK. Returns
/// the return code and the reader half (for close verification).
async fn probe_empty_clientid_connect(
    ctx: &TestContext,
    clean_session: bool,
) -> Result<Probe, anyhow::Error> {
    let (mut reader, mut writer) =
        crate::transport::tcp_v3::connect(&ctx.config.broker_addr, ctx.config.connect_timeout).await?;

    let connect = Connect {
        protocol: Protocol(MQTT_LEVEL_311),
        clean_session,
        keep_alive: 60,
        client_id: ByteString::from(""),
        ..Default::default()
    };

    writer.send_packet(&Packet::Connect(Box::new(connect))).await?;

    let pkt = tokio::time::timeout(Duration::from_secs(5), reader.read_packet())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for CONNACK"))??;

    match pkt {
        Packet::ConnectAck(ack) => Ok(Probe { reason: ack.return_code, reader }),
        other => Err(anyhow::anyhow!("expected CONNACK, got {:?}", packet_name_v3(&other))),
    }
}

/// After a rejected CONNECT the Server MUST close the Network Connection
/// (MQTT-3.1.3-6). A clean EOF (or no further data within a short window) is
/// acceptable; receiving an extra MQTT packet is a violation.
async fn assert_connection_closed(reader: &mut TcpTransportV3Reader) -> Result<(), anyhow::Error> {
    match tokio::time::timeout(Duration::from_secs(2), reader.read_packet()).await {
        Ok(Ok(pkt)) => Err(anyhow::anyhow!(
            "expected the Network Connection to be closed after rejection, \
             but received an extra packet: {:?}",
            packet_name_v3(&pkt)
        )),
        Ok(Err(_)) => Ok(()), // EOF: connection closed by broker
        Err(_) => Ok(()),     // lenient: no further data within the window
    }
}

/// GitHub issue reproduction: an empty ClientId with CleanSession = 0 MUST be
/// rejected with `0x02` (IdentifierRejected) and the connection closed
/// (MQTT-3.1.3-6). Control: empty ClientId with CleanSession = 1 MUST be
/// accepted (MQTT-3.1.3-7).
pub struct EmptyClientIdCleanSession0RejectedV311Test;

impl TestCase for EmptyClientIdCleanSession0RejectedV311Test {
    fn name(&self) -> &str {
        "v311_empty_clientid_cleansession0_rejected"
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: Result<(), anyhow::Error> = rt.block_on(async {
            // 1) Empty ClientId + CleanSession = 0 MUST be rejected with 0x02.
            let p = probe_empty_clientid_connect(ctx, false).await?;
            if p.reason != ConnectAckReason::IdentifierRejected {
                // Actual rmqtt behaviour before the fix: rejected with 0x03.
                return Err(anyhow::anyhow!(
                    "MQTT-3.1.3-6 violation: empty ClientId with CleanSession = 0 \
                     must be rejected with return code IdentifierRejected (0x02), \
                     but CONNACK return code = {:?} (0x{:02X})",
                    p.reason,
                    u8::from(p.reason)
                ));
            }

            // 2) After the rejection the server must close the connection.
            let mut reader = p.reader;
            assert_connection_closed(&mut reader).await?;

            // 3) Control: empty ClientId + CleanSession = 1 MUST be accepted
            //    (the broker assigns a ClientId, MQTT-3.1.3-7).
            let p = probe_empty_clientid_connect(ctx, true).await?;
            if p.reason != ConnectAckReason::ConnectionAccepted {
                return Err(anyhow::anyhow!(
                    "control failed: empty ClientId with CleanSession = 1 \
                     should be accepted, got {:?} (0x{:02X})",
                    p.reason,
                    u8::from(p.reason)
                ));
            }

            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }
}
