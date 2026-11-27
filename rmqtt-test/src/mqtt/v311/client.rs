//! MQTT v3.1.1 Client - Full QoS 0/1/2 support
//!
//! Features:
//! - MQTT 3.1.1 (MQTT / level 4)
//! - Single reader loop architecture
//! - QoS 0/1/2 publish
//! - QoS 0/1/2 subscribe
//! - Async packet routing
//! - Proper SUBACK matching
//! - Incoming publish channel
//! - Protocol acknowledgments (PUBACK, PUBREC, PUBCOMP)
//!
//! Architecture:
//!
//!                  TCP
//!                   |
//!            reader task
//!                   |   writes PUBACK/PUBREC/PUBCOMP
//!                   |
//!        ┌──────────┴──────────┐
//!        │                     │
//!   publish channel      ack router
//!
//! Only ONE task reads from socket.

use std::collections::HashMap;
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use bytestring::ByteString;
use rmqtt_codec::v3::{
    Connect, ConnectAck, ConnectAckReason, LastWill, Packet as PacketV3, SubscribeReturnCode,
};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time;

use crate::mqtt::common::session::PacketIdCounter;
use crate::mqtt::common::QoSTest;
use crate::mqtt::common::MQTT_LEVEL_311;
use crate::transport::tcp_v3::{self, TcpTransportV3Writer};

/// Incoming publish message
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub topic: ByteString,
    pub payload: Bytes,
    pub qos: QoSTest,
    pub retain: bool,
    pub dup: bool,
    /// Packet identifier for QoS 1/2 deliveries, `None` for QoS 0.
    pub packet_id: Option<u16>,
}

/// Subscribe result
#[derive(Debug)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub status: Vec<SubscribeReturnCode>,
}

/// Which acknowledgement a `publish()` call is currently waiting for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AckPhase {
    /// QoS 1: waiting for PUBACK
    Ack,
    /// QoS 2 step 1: waiting for PUBREC
    Rec,
    /// QoS 2 step 2: waiting for PUBCOMP
    Comp,
}

/// A registered publish-ack waiter for one packet id.
struct WaiterEntry {
    phase: AckPhase,
    tx: oneshot::Sender<()>,
}

/// MQTT v3.1.1 Client - full QoS 0/1/2
pub struct MqttV311Client {
    writer: Arc<Mutex<TcpTransportV3Writer>>,
    connected: Arc<AtomicBool>,
    packet_id_counter: PacketIdCounter,

    /// Incoming publish receiver
    message_rx: mpsc::UnboundedReceiver<IncomingMessage>,

    /// Ack waiters for SUBACK
    suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>>,

    /// Waiters for QoS 1/2 publish acknowledgements (PUBACK / PUBREC /
    /// PUBCOMP), keyed by packet id. Only one phase per packet id at a time.
    ack_waiters: Arc<Mutex<HashMap<u16, WaiterEntry>>>,

    /// Whether to automatically answer incoming PUBREL with PUBCOMP (QoS 2 part 2).
    /// Disabling allows tests to leave a QoS 2 exchange incomplete.
    auto_pubcomp: Arc<AtomicBool>,

    /// Whether to automatically answer incoming QoS 1 PUBLISH with PUBACK.
    /// Disabling allows tests to leave a QoS 1 exchange incomplete (the broker
    /// then owes a redelivery with DUP=1 on session resume, MQTT-4.4.0-1).
    auto_puback: Arc<AtomicBool>,

    /// Whether to automatically answer incoming QoS 2 PUBLISH with PUBREC.
    /// Disabling allows tests to leave the broker->client QoS 2 exchange
    /// incomplete at the PUBREC stage (MQTT-4.3.3).
    auto_pubrec: Arc<AtomicBool>,

    /// Incoming PUBREL packet id receiver (broker -> client, QoS 2 part 2)
    pubrel_rx: mpsc::UnboundedReceiver<NonZeroU16>,

    connack: ConnectAck,
}

impl MqttV311Client {
    /// Connect to broker with default settings
    pub async fn connect(broker_addr: &str, client_id: &str, connect_timeout: Duration) -> Result<Self> {
        Self::connect_with_options(broker_addr, client_id, connect_timeout, true, 60, None, None, None).await
    }

    /// Connect to broker with full options
    #[allow(clippy::too_many_arguments)]
    pub async fn connect_with_options(
        broker_addr: &str,
        client_id: &str,
        connect_timeout: Duration,
        clean_session: bool,
        keep_alive: u16,
        will: Option<LastWill>,
        username: Option<ByteString>,
        password: Option<Bytes>,
    ) -> Result<Self> {
        let (mut reader, writer) = tcp_v3::connect(broker_addr, connect_timeout).await?;
        let writer = Arc::new(Mutex::new(writer));
        let connected = Arc::new(AtomicBool::new(true));

        let (message_tx, message_rx) = mpsc::unbounded_channel();
        let suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let ack_waiters: Arc<Mutex<HashMap<u16, WaiterEntry>>> = Arc::new(Mutex::new(HashMap::new()));
        let auto_pubcomp = Arc::new(AtomicBool::new(true));
        let auto_puback = Arc::new(AtomicBool::new(true));
        let auto_pubrec = Arc::new(AtomicBool::new(true));
        let (pubrel_tx, pubrel_rx) = mpsc::unbounded_channel();

        //
        // SEND CONNECT
        //
        {
            let conn = Connect {
                protocol: rmqtt_codec::types::Protocol(MQTT_LEVEL_311),
                clean_session,
                keep_alive,
                last_will: will,
                client_id: ByteString::from(client_id),
                username,
                password,
                cert: None,
            };

            writer.lock().await.send_packet(&PacketV3::Connect(Box::new(conn))).await?;
        }

        //
        // WAIT CONNACK
        //
        let connack = {
            let pkt = reader.read_packet().await?;

            match pkt {
                PacketV3::ConnectAck(ack) => {
                    if ack.return_code != ConnectAckReason::ConnectionAccepted {
                        return Err(anyhow!("connect failed: {:?}", ack.return_code));
                    }
                    ack
                }
                other => {
                    return Err(anyhow!("expected CONNACK, got: {:?}", other));
                }
            }
        };

        //
        // START SINGLE READER LOOP
        //
        {
            let writer = writer.clone();
            let connected = connected.clone();
            let suback_waiters = suback_waiters.clone();
            let ack_waiters = ack_waiters.clone();
            let auto_pubcomp = auto_pubcomp.clone();
            let auto_puback = auto_puback.clone();
            let auto_pubrec = auto_pubrec.clone();
            let pubrel_tx = pubrel_tx.clone();

            tokio::spawn(async move {
                loop {
                    let pkt = match reader.read_packet().await {
                        Ok(pkt) => pkt,
                        Err(err) => {
                            eprintln!("mqtt read error: {:?}", err);
                            connected.store(false, Ordering::Relaxed);
                            // Wake any publish() waiting for an ack: their
                            // oneshot senders are dropped, so the awaits get
                            // a cancellation error.
                            ack_waiters.lock().await.clear();
                            break;
                        }
                    };

                    match pkt {
                        // PUBLISH
                        PacketV3::Publish(pub_msg) => {
                            let qos = pub_msg.qos;
                            let packet_id = pub_msg.packet_id;

                            let msg = IncomingMessage {
                                topic: pub_msg.topic.clone(),
                                payload: pub_msg.payload.clone(),
                                qos,
                                retain: pub_msg.retain,
                                dup: pub_msg.dup,
                                packet_id: packet_id.map(|p| p.get()),
                            };
                            let _ = message_tx.send(msg);

                            // Send protocol acknowledgment
                            if let Some(pkt_id) = packet_id {
                                if qos == QoSTest::AtLeastOnce && auto_puback.load(Ordering::Relaxed) {
                                    // QoS 1: send PUBACK
                                    let ack = PacketV3::PublishAck { packet_id: pkt_id };
                                    let _ = writer.lock().await.send_packet(&ack).await;
                                } else if qos == QoSTest::ExactlyOnce && auto_pubrec.load(Ordering::Relaxed) {
                                    // QoS 2: send PUBREC
                                    let ack = PacketV3::PublishReceived { packet_id: pkt_id };
                                    let _ = writer.lock().await.send_packet(&ack).await;
                                }
                            }
                        }

                        // PUBREL (QoS 2 part 2): forward the event, send PUBCOMP if auto-ack is on
                        PacketV3::PublishRelease { packet_id, .. } => {
                            let _ = pubrel_tx.send(packet_id);
                            if auto_pubcomp.load(Ordering::Relaxed) {
                                let ack = PacketV3::PublishComplete { packet_id };
                                let _ = writer.lock().await.send_packet(&ack).await;
                            }
                        }

                        // SUBACK
                        PacketV3::SubscribeAck { packet_id, status } => {
                            let tx = { suback_waiters.lock().await.remove(&packet_id.get()) };

                            if let Some(tx) = tx {
                                let _ = tx.send(Ok(SubscribeAck { packet_id, status }));
                            }
                        }

                        // PUBACK from broker (QoS 1 publish ack)
                        PacketV3::PublishAck { packet_id, .. } => {
                            if let Some(w) = ack_waiters.lock().await.remove(&packet_id.get()) {
                                if w.phase == AckPhase::Ack {
                                    let _ = w.tx.send(());
                                }
                            }
                        }

                        // PUBREC from broker (QoS 2 publish received)
                        PacketV3::PublishReceived { packet_id, .. } => {
                            if let Some(w) = ack_waiters.lock().await.remove(&packet_id.get()) {
                                if w.phase == AckPhase::Rec {
                                    let _ = w.tx.send(());
                                }
                            }
                        }

                        // PUBCOMP from broker (QoS 2 publish complete)
                        PacketV3::PublishComplete { packet_id } => {
                            if let Some(w) = ack_waiters.lock().await.remove(&packet_id.get()) {
                                if w.phase == AckPhase::Comp {
                                    let _ = w.tx.send(());
                                }
                            }
                        }

                        // PINGRESP
                        PacketV3::PingResponse => {
                            // Handle ping response
                        }

                        // DISCONNECT
                        PacketV3::Disconnect => {
                            eprintln!("Received DISCONNECT from broker");
                            ack_waiters.lock().await.clear();
                            break;
                        }

                        // IGNORE OTHER PACKETS
                        other => {
                            eprintln!("ignored packet: {:?}", other);
                        }
                    }
                }
            });
        }

        Ok(Self {
            writer,
            connected,
            packet_id_counter: PacketIdCounter::new(),
            message_rx,
            suback_waiters,
            ack_waiters,
            auto_pubcomp,
            auto_puback,
            auto_pubrec,
            pubrel_rx,
            connack,
        })
    }

    /// Get CONNACK
    pub fn connack(&self) -> &ConnectAck {
        &self.connack
    }

    /// Check connected
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Publish a message with QoS and retain flag.
    ///
    /// Performs the full protocol acknowledgement exchange:
    /// - QoS 0: fire-and-forget
    /// - QoS 1: waits for the broker's PUBACK
    /// - QoS 2: waits for PUBREC, sends PUBREL, then waits for PUBCOMP
    ///
    /// (Previously this was fire-and-forget for all QoS levels, which left the
    /// broker's QoS 2 state machine stuck in WAIT_PUBREC and exhausted
    /// `max_inflight` under sustained load.)
    pub async fn publish(&self, topic: &str, payload: &[u8], qos: QoSTest, retain: bool) -> Result<()> {
        let packet_id = if qos != QoSTest::AtMostOnce {
            Some(
                NonZeroU16::new(u16::from(self.packet_id_counter.next()))
                    .ok_or_else(|| anyhow!("packet id overflow"))?,
            )
        } else {
            None
        };

        let publish = rmqtt_codec::types::Publish {
            dup: false,
            retain,
            qos,
            topic: ByteString::from(topic),
            packet_id,
            payload: Bytes::copy_from_slice(payload),
            properties: None,
        };

        match qos {
            QoSTest::AtMostOnce => {
                self.writer.lock().await.send_packet(&PacketV3::Publish(Box::new(publish))).await?;
                Ok(())
            }
            QoSTest::AtLeastOnce => {
                let pid = packet_id.ok_or_else(|| anyhow!("QoS 1 publish without packet id"))?;
                // Register before sending so a fast PUBACK cannot be missed.
                let rx = self.register_ack(pid, AckPhase::Ack).await;
                self.writer.lock().await.send_packet(&PacketV3::Publish(Box::new(publish))).await?;
                self.wait_ack(pid, rx, "PUBACK").await
            }
            QoSTest::ExactlyOnce => {
                let pid = packet_id.ok_or_else(|| anyhow!("QoS 2 publish without packet id"))?;
                let rx = self.register_ack(pid, AckPhase::Rec).await;
                self.writer.lock().await.send_packet(&PacketV3::Publish(Box::new(publish))).await?;
                self.wait_ack(pid, rx, "PUBREC").await?;
                let rx = self.register_ack(pid, AckPhase::Comp).await;
                self.writer.lock().await.send_packet(&PacketV3::PublishRelease { packet_id: pid }).await?;
                self.wait_ack(pid, rx, "PUBCOMP").await
            }
        }
    }

    /// Register a waiter for the given ack phase, keyed by packet id.
    async fn register_ack(&self, packet_id: NonZeroU16, phase: AckPhase) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        self.ack_waiters.lock().await.insert(packet_id.get(), WaiterEntry { phase, tx });
        rx
    }

    /// Wait for the registered ack (with a timeout) and clean up the waiter.
    async fn wait_ack(
        &self,
        packet_id: NonZeroU16,
        rx: oneshot::Receiver<()>,
        label: &'static str,
    ) -> Result<()> {
        let result = tokio::time::timeout(Duration::from_secs(10), rx).await;
        self.ack_waiters.lock().await.remove(&packet_id.get());
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(anyhow!("{label} wait canceled (connection closed) for packet {packet_id}")),
            Err(_) => Err(anyhow!("{label} timeout for packet {packet_id}")),
        }
    }

    /// Publish a message with an explicit packet id and DUP flag.
    ///
    /// Useful for QoS 2 conformance tests that need to replay a PUBLISH with
    /// the same Packet Identifier (e.g. MQTT-4.3.3-10 duplicate handling).
    pub async fn publish_with_packet_id(
        &self,
        topic: &str,
        payload: &[u8],
        qos: QoSTest,
        retain: bool,
        dup: bool,
        packet_id: NonZeroU16,
    ) -> Result<()> {
        let publish = rmqtt_codec::types::Publish {
            dup,
            retain,
            qos,
            topic: ByteString::from(topic),
            packet_id: Some(packet_id),
            properties: None,
            payload: Bytes::copy_from_slice(payload),
        };

        self.writer.lock().await.send_packet(&PacketV3::Publish(Box::new(publish))).await?;

        Ok(())
    }

    /// Send a PUBREL (QoS 2 part 2) with the given packet id
    pub async fn send_pubrel(&self, packet_id: NonZeroU16) -> Result<()> {
        self.writer.lock().await.send_packet(&PacketV3::PublishRelease { packet_id }).await?;
        Ok(())
    }

    /// Enable/disable the automatic PUBCOMP sent in reply to an incoming PUBREL.
    ///
    /// Disabling allows tests to leave a QoS 2 exchange incomplete (the broker
    /// keeps owing a PUBCOMP), e.g. to verify MQTT-4.4.0-1 PUBREL resend on resume.
    pub fn set_auto_pubcomp(&self, enabled: bool) {
        self.auto_pubcomp.store(enabled, Ordering::Relaxed);
    }

    /// Enable/disable the automatic PUBACK sent in reply to an incoming
    /// QoS 1 PUBLISH.
    ///
    /// Disabling allows tests to leave a QoS 1 exchange incomplete (the broker
    /// keeps owing a redelivery), e.g. to verify MQTT-4.4.0-1 PUBLISH resend
    /// with DUP=1 and the original packet identifier on session resume.
    pub fn set_auto_puback(&self, enabled: bool) {
        self.auto_puback.store(enabled, Ordering::Relaxed);
    }

    /// Enable/disable the automatic PUBREC sent in reply to an incoming
    /// QoS 2 PUBLISH.
    ///
    /// Disabling allows tests to leave the broker->client QoS 2 exchange
    /// incomplete at the PUBREC stage, e.g. to verify the broker retransmits
    /// the PUBLISH with DUP=1 (MQTT-4.3.3).
    pub fn set_auto_pubrec(&self, enabled: bool) {
        self.auto_pubrec.store(enabled, Ordering::Relaxed);
    }

    /// Wait for an incoming PUBREL packet id (broker -> client, QoS 2 part 2)
    pub async fn recv_pubrel_timeout(&mut self, timeout: Duration) -> Option<u16> {
        time::timeout(timeout, self.pubrel_rx.recv()).await.ok().and_then(|r| r).map(|pid| pid.get())
    }

    /// Subscribe to a topic with a specific QoS
    pub async fn subscribe(&mut self, topic: &str, qos: QoSTest) -> Result<SubscribeAck> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let subscribe_pkt =
            PacketV3::Subscribe { packet_id, topic_filters: vec![(ByteString::from(topic), qos)] };

        // REGISTER ACK WAITER
        let (tx, rx) = oneshot::channel();
        self.suback_waiters.lock().await.insert(packet_id.get(), tx);

        // SEND SUBSCRIBE
        self.writer.lock().await.send_packet(&subscribe_pkt).await?;

        // WAIT SUBACK
        let ack = time::timeout(Duration::from_secs(30), rx)
            .await
            .map_err(|_| anyhow!("subscribe timeout"))?
            .map_err(|_| anyhow!("suback waiter dropped"))??;

        Ok(ack)
    }

    /// Unsubscribe from a topic
    pub async fn unsubscribe(&mut self, topic: &str) -> Result<()> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let unsub = PacketV3::Unsubscribe { packet_id, topic_filters: vec![ByteString::from(topic)] };

        self.writer.lock().await.send_packet(&unsub).await?;

        Ok(())
    }

    /// Send a PINGREQ
    pub async fn ping(&self) -> Result<()> {
        self.writer.lock().await.send_packet(&PacketV3::PingRequest).await
    }

    /// Receive incoming publish
    pub async fn recv_message(&mut self) -> Result<IncomingMessage> {
        self.message_rx.recv().await.ok_or_else(|| anyhow!("message channel closed"))
    }

    /// Receive incoming publish with timeout
    pub async fn recv_message_timeout(&mut self, timeout: Duration) -> Option<IncomingMessage> {
        time::timeout(timeout, self.recv_message()).await.ok().and_then(|r| r.ok())
    }

    /// Disconnect
    pub async fn disconnect(&self) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);
        {
            let mut writer = self.writer.lock().await;
            let _ = writer.send_packet(&PacketV3::Disconnect).await;
            writer.shutdown().await?;
        }
        Ok(())
    }

    /// Abort connection without sending DISCONNECT (simulates unclean disconnect)
    /// Used for testing Last Will and Testament
    pub async fn abort_connection(&self) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);
        self.writer.lock().await.shutdown().await?;
        Ok(())
    }
}
