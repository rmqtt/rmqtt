//! MQTT v3.1 Client
//!
//! Features:
//! - MQTT v3.1 (MQIsdp / level 3)
//! - Single reader loop architecture
//! - QoS 0/1/2 publish
//! - QoS 0/1/2 subscribe
//! - Async packet routing
//! - Proper SUBACK matching
//! - Incoming publish channel
//! - Protocol acknowledgments (PUBACK, PUBREC, PUBCOMP)
//! - CONNECT options (clean session, keep alive, will, username/password)
//!
//! Note on the CONNECT encoding: the v3 codec's `encode_connect` hardcodes
//! protocol level 4 (MQTT_LEVEL_311) regardless of the `Protocol` value, so a
//! `Packet::Connect` with `Protocol(3)` would be emitted as "MQIsdp" + level 4,
//! which is an invalid combination. To send a *true* MQTT v3.1 CONNECT
//! ("MQIsdp" + level 3), this client builds the CONNECT bytes by hand and
//! sends them via `send_raw`, bypassing the codec. All other packets go
//! through the codec as usual.
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
use rmqtt_codec::types::MQTT_LEVEL_31;
use rmqtt_codec::v3::{ConnectAck, ConnectAckReason, LastWill, Packet as PacketV3, SubscribeReturnCode};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time;

use crate::mqtt::common::session::PacketIdCounter;
use crate::mqtt::common::QoSTest;
use crate::transport::tcp_v3::{self, TcpTransportV3Writer};

/// Incoming publish message
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub topic: ByteString,
    pub payload: Bytes,
    pub qos: QoSTest,
    pub retain: bool,
    pub dup: bool,
}

/// Subscribe result
#[derive(Debug)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub status: Vec<SubscribeReturnCode>,
}

/// MQTT v3.0 Client - full QoS 0/1/2
pub struct MqttV3Client {
    writer: Arc<Mutex<TcpTransportV3Writer>>,
    connected: Arc<AtomicBool>,
    packet_id_counter: PacketIdCounter,

    /// Incoming publish receiver
    message_rx: mpsc::UnboundedReceiver<IncomingMessage>,

    /// Ack waiters for SUBACK
    suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>>,

    /// Whether to automatically answer incoming PUBREL with PUBCOMP (QoS 2 part 2).
    auto_pubcomp: Arc<AtomicBool>,

    /// Incoming PUBREL packet id receiver (broker -> client, QoS 2 part 2)
    pubrel_rx: mpsc::UnboundedReceiver<NonZeroU16>,

    connack: ConnectAck,
}

impl MqttV3Client {
    /// Connect to broker with default settings (MQIsdp / level 3)
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
        let auto_pubcomp = Arc::new(AtomicBool::new(true));
        let (pubrel_tx, pubrel_rx) = mpsc::unbounded_channel();

        //
        // SEND CONNECT (hand-built MQIsdp + level 3, see module docs)
        //
        {
            let connect_bytes = build_connect_bytes(
                client_id,
                clean_session,
                keep_alive,
                will.as_ref(),
                username.as_ref(),
                password.as_ref(),
            );
            writer.lock().await.send_raw(&connect_bytes).await?;
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
            let auto_pubcomp = auto_pubcomp.clone();
            let pubrel_tx = pubrel_tx.clone();

            tokio::spawn(async move {
                loop {
                    let pkt = match reader.read_packet().await {
                        Ok(pkt) => pkt,
                        Err(err) => {
                            eprintln!("mqtt read error: {:?}", err);
                            connected.store(false, Ordering::Relaxed);
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
                            };
                            let _ = message_tx.send(msg);

                            // Send protocol acknowledgment
                            if let Some(pkt_id) = packet_id {
                                if qos == QoSTest::AtLeastOnce {
                                    // QoS 1: send PUBACK
                                    let ack = PacketV3::PublishAck { packet_id: pkt_id };
                                    let _ = writer.lock().await.send_packet(&ack).await;
                                } else if qos == QoSTest::ExactlyOnce {
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
                            eprintln!("PUBACK received for packet_id: {}", packet_id);
                        }

                        // PUBREC from broker (QoS 2 publish received)
                        PacketV3::PublishReceived { packet_id, .. } => {
                            eprintln!("PUBREC received for packet_id: {}", packet_id);
                        }

                        // PUBCOMP from broker (QoS 2 publish complete)
                        PacketV3::PublishComplete { packet_id } => {
                            eprintln!("PUBCOMP received for packet_id: {}", packet_id);
                        }

                        // PINGRESP
                        PacketV3::PingResponse => {
                            // Handle ping response
                        }

                        // DISCONNECT
                        PacketV3::Disconnect => {
                            eprintln!("Received DISCONNECT from broker");
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
            auto_pubcomp,
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

    /// Publish a message with QoS and retain flag
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

        self.writer.lock().await.send_packet(&PacketV3::Publish(Box::new(publish))).await?;

        Ok(())
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
    pub fn set_auto_pubcomp(&self, enabled: bool) {
        self.auto_pubcomp.store(enabled, Ordering::Relaxed);
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
        let ack = time::timeout(Duration::from_secs(15), rx)
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

    /// Disconnect (sends DISCONNECT, no will triggered)
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

/// Build a raw MQTT v3.1 CONNECT packet ("MQIsdp" / level 3) by hand.
///
/// The v3 codec's `encode_connect` hardcodes protocol level 4, so this is the
/// only way to emit a true v3.1 CONNECT until that codec bug is fixed.
pub(crate) fn build_connect_bytes(
    client_id: &str,
    clean_session: bool,
    keep_alive: u16,
    will: Option<&LastWill>,
    username: Option<&ByteString>,
    password: Option<&Bytes>,
) -> Vec<u8> {
    let mut flags: u8 = 0;
    if clean_session {
        flags |= 0x02; // Clean Session
    }
    if let Some(w) = will {
        flags |= 0x04; // Will Flag
        flags |= (w.qos as u8) << 3; // Will QoS (bits 4-3)
        if w.retain {
            flags |= 0x20; // Will Retain (bit 5)
        }
    }
    if username.is_some() {
        flags |= 0x80; // User Name Flag
    }
    if password.is_some() {
        flags |= 0x40; // Password Flag
    }

    let mut body: Vec<u8> = Vec::new();
    // Protocol name "MQIsdp"
    body.extend_from_slice(&(6u16).to_be_bytes());
    body.extend_from_slice(b"MQIsdp");
    // Protocol level
    body.push(MQTT_LEVEL_31);
    // Connect flags
    body.push(flags);
    // Keep alive
    body.extend_from_slice(&keep_alive.to_be_bytes());

    // Client identifier
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);

    // Will topic + message
    if let Some(w) = will {
        let wt = w.topic.as_bytes();
        body.extend_from_slice(&(wt.len() as u16).to_be_bytes());
        body.extend_from_slice(wt);
        body.extend_from_slice(&(w.message.len() as u16).to_be_bytes());
        body.extend_from_slice(&w.message);
    }

    // Username / password
    if let Some(u) = username {
        let ub = u.as_bytes();
        body.extend_from_slice(&(ub.len() as u16).to_be_bytes());
        body.extend_from_slice(ub);
    }
    if let Some(p) = password {
        body.extend_from_slice(&(p.len() as u16).to_be_bytes());
        body.extend_from_slice(p);
    }

    // Fixed header: CONNECT (0x10) + remaining length
    let mut pkt = vec![0x10];
    write_variable_length(&mut pkt, body.len());
    pkt.extend_from_slice(&body);
    pkt
}

/// Encode a non-negative integer as a variable-length byte sequence.
fn write_variable_length(buf: &mut Vec<u8>, mut len: usize) {
    loop {
        let mut b = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        buf.push(b);
        if len == 0 {
            break;
        }
    }
}
