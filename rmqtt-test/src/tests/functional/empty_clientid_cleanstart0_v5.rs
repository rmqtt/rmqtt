//! MQTT v5 conformance: empty ClientId + CleanStart = 0 must be rejected (MQTT-3.1.3-8)
//!
//! The MQTT 5.0 specification requires:
//!
//! * [MQTT-3.1.3-7] If the ClientId is a zero-length byte string and CleanStart
//!   is 1, the Server MUST assign a unique ClientId and return it in the
//!   CONNACK `Assigned Client Identifier` property.
//! * [MQTT-3.1.3-8] If the ClientId is a zero-length byte string and CleanStart
//!   is 0, the Server MUST send a CONNACK with reason code `0x85`
//!   (Client Identifier not valid) and then close the Network Connection.
//!
//! Current rmqtt behaviour (defect under reproduction): the check itself
//! exists at the codec layer — `rmqtt-codec/src/v5/packet/connect.rs` rejects
//! an empty ClientId with `DecodeError::InvalidClientId` when CleanStart is 0
//! (MQTT-3.1.3-8). However `rmqtt/src/v5.rs` maps that decode error to a
//! CONNACK with reason `0x88` (ServerUnavailable) instead of the required
//! `0x85` (Client Identifier not valid), so the Server does not return the
//! reason code mandated by the specification.
//!
//! Expected result today (before the fix): this test FAILS — the broker
//! rejects the connection with reason 0x88 instead of 0x85, which is exactly
//! the MQTT-3.1.3-8 violation being reproduced.

use std::time::{Duration, Instant};

use bytestring::ByteString;
use rmqtt_codec::v5::{Connect, ConnectAckReason, Packet};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::transport::tcp_v5::{packet_name_v5, TcpTransportV5Reader};

/// Result of one raw CONNECT probe.
struct Probe {
    reason: ConnectAckReason,
    assigned_client_id: Option<ByteString>,
    reader: TcpTransportV5Reader,
}

/// Open a raw TCP connection and send a v5 CONNECT with an **empty** ClientId
/// and the given CleanStart flag, then read the CONNACK. Returns the reason
/// code, the `assigned_client_id` property and the reader half (for close
/// verification).
async fn probe_empty_clientid_connect(ctx: &TestContext, clean_start: bool) -> Result<Probe, anyhow::Error> {
    let (mut reader, mut writer) =
        crate::transport::tcp_v5::connect(&ctx.config.broker_addr, ctx.config.connect_timeout).await?;

    let connect =
        Connect { clean_start, keep_alive: 60, client_id: ByteString::from(""), ..Default::default() };

    writer.send_packet(&Packet::Connect(Box::new(connect))).await?;

    let pkt = tokio::time::timeout(Duration::from_secs(5), reader.read_packet())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for CONNACK"))??;

    match pkt {
        Packet::ConnectAck(ack) => {
            Ok(Probe { reason: ack.reason_code, assigned_client_id: ack.assigned_client_id, reader })
        }
        other => Err(anyhow::anyhow!("expected CONNACK, got {:?}", packet_name_v5(&other))),
    }
}

/// After a rejected CONNECT the Server MUST close the Network Connection
/// (MQTT-3.1.3-8). A clean EOF (or no further data within a short window) is
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

/// GitHub issue reproduction: an empty ClientId with CleanStart = 0 MUST be
/// rejected with `0x85` (ClientIdentifierNotValid) and the connection closed
/// (MQTT-3.1.3-8). Control: empty ClientId with CleanStart = 1 MUST be
/// accepted with an assigned ClientId (MQTT-3.1.3-7).
pub struct EmptyClientIdCleanStart0RejectedV5Test;

impl TestCase for EmptyClientIdCleanStart0RejectedV5Test {
    fn name(&self) -> &str {
        "v5_empty_clientid_cleanstart0_rejected"
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: Result<(), anyhow::Error> = rt.block_on(async {
            // 1) Empty ClientId + CleanStart = 0 MUST be rejected with 0x85.
            let p = probe_empty_clientid_connect(ctx, false).await?;
            if p.reason != ConnectAckReason::ClientIdentifierNotValid {
                // Actual rmqtt behaviour (the reproduced defect): the connection
                // is rejected, but with reason 0x88 instead of the required 0x85.
                return Err(anyhow::anyhow!(
                    "MQTT-3.1.3-8 violation: empty ClientId with CleanStart = 0 \
                     must be rejected with reason ClientIdentifierNotValid (0x85), \
                     but CONNACK reason = {:?} (0x{:02X})",
                    p.reason,
                    u8::from(p.reason)
                ));
            }

            // 2) After the rejection the server must close the connection.
            let mut reader = p.reader;
            assert_connection_closed(&mut reader).await?;

            // 3) Control: empty ClientId + CleanStart = 1 MUST be accepted with
            //    an assigned ClientId returned in the CONNACK (MQTT-3.1.3-7).
            let p = probe_empty_clientid_connect(ctx, true).await?;
            if p.reason != ConnectAckReason::Success {
                return Err(anyhow::anyhow!(
                    "control failed: empty ClientId with CleanStart = 1 \
                     should be accepted, got {:?} (0x{:02X})",
                    p.reason,
                    u8::from(p.reason)
                ));
            }
            match p.assigned_client_id {
                Some(id) if !id.is_empty() => {}
                other => {
                    return Err(anyhow::anyhow!(
                        "control failed: CONNACK for an assigned ClientId \
                         must carry a non-empty 'Assigned Client Identifier' property, \
                         got {:?}",
                        other
                    ));
                }
            }

            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
}
