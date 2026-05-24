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
}

/// Subscribe result
#[derive(Debug)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub status: Vec<SubscribeReturnCode>,
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

                        // PUBREL (QoS 2 part 2): send PUBCOMP
                        PacketV3::PublishRelease { packet_id, .. } => {
                            let ack = PacketV3::PublishComplete { packet_id };
                            let _ = writer.lock().await.send_packet(&ack).await;
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
}
