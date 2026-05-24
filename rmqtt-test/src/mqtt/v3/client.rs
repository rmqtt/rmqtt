//! MQTT v3.1 Client
//!
//! Features:
//! - MQTT v3.1 (MQIsdp / level 3)
//! - Single reader loop architecture
//! - QoS 0 publish
//! - QoS 0 subscribe
//! - Async packet routing
//! - Proper SUBACK matching
//! - Incoming publish channel
//!
//! Architecture:
//!
//!                  TCP
//!                   |
//!            reader task
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
use rmqtt_codec::v3::{Connect, ConnectAck, ConnectAckReason, Packet as PacketV3, QoS, SubscribeReturnCode};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time;

use crate::mqtt::common::session::PacketIdCounter;
use crate::transport::tcp_v3::{self, TcpTransportV3Writer};

/// Incoming publish message
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub topic: ByteString,
    pub payload: Bytes,
    pub qos: QoS,
    pub retain: bool,
    pub dup: bool,
}

/// Subscribe result
#[derive(Debug)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub status: Vec<SubscribeReturnCode>,
}

/// MQTT v3.0 Client
pub struct MqttV3Client {
    writer: Arc<Mutex<TcpTransportV3Writer>>,
    connected: Arc<AtomicBool>,
    packet_id_counter: PacketIdCounter,

    /// Incoming publish receiver
    message_rx: mpsc::UnboundedReceiver<IncomingMessage>,

    /// Ack waiters
    suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>>,

    connack: ConnectAck,
}

impl MqttV3Client {
    /// Connect to broker
    pub async fn connect(broker_addr: &str, client_id: &str, connect_timeout: Duration) -> Result<Self> {
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
                protocol: rmqtt_codec::types::Protocol(3),
                clean_session: true,
                keep_alive: 60,
                last_will: None,
                client_id: ByteString::from(client_id),
                username: None,
                password: None,
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

                            // Send PUBACK for QoS 1 (e.g. forwarded from v311 publisher)
                            if let Some(pkt_id) = packet_id {
                                if matches!(qos, QoS::AtLeastOnce | QoS::ExactlyOnce) {
                                    let ack = PacketV3::PublishAck { packet_id: pkt_id };
                                    let _ = writer.lock().await.send_packet(&ack).await;
                                }
                            }
                        }

                        // SUBACK
                        PacketV3::SubscribeAck { packet_id, status } => {
                            let tx = { suback_waiters.lock().await.remove(&packet_id.get()) };

                            if let Some(tx) = tx {
                                let _ = tx.send(Ok(SubscribeAck { packet_id, status }));
                            }
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

    /// Publish QoS0
    pub async fn publish(&self, topic: &str, payload: &[u8]) -> Result<()> {
        let publish = rmqtt_codec::types::Publish {
            dup: false,
            retain: false,
            qos: QoS::AtMostOnce,
            topic: ByteString::from(topic),
            packet_id: None,
            payload: Bytes::copy_from_slice(payload),
            properties: None,
        };

        self.writer.lock().await.send_packet(&PacketV3::Publish(Box::new(publish))).await?;

        Ok(())
    }

    /// Subscribe QoS0
    pub async fn subscribe(&mut self, topic: &str) -> Result<SubscribeAck> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let subscribe_pkt = PacketV3::Subscribe {
            packet_id,
            topic_filters: vec![(ByteString::from(topic), QoS::AtMostOnce)],
        };

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
