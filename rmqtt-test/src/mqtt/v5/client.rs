//! MQTT v5.0 Client - Enhanced with properties, reason codes, session expiry
//!
//! Features:
//! - MQTT 5.0 (MQTT / level 5)
//! - Single reader loop architecture
//! - QoS 0/1/2 publish
//! - QoS 0/1/2 subscribe
//! - Async packet routing
//! - Proper SUBACK matching
//! - Incoming publish channel
//! - V5 Properties support
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
use std::num::{NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use anyhow::Result;
use bytes::Bytes;
use bytestring::ByteString;
use rmqtt_codec::v5::ConnectAckReason;
use rmqtt_codec::v5::{
    Connect, ConnectAck, LastWill, Packet as PacketV5, PublishAck, PublishAck2, PublishAck2Reason,
    PublishAckReason, PublishProperties, SubscriptionOptions, UserProperties,
};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::time;

use crate::mqtt::common::session::PacketIdCounter;
use crate::mqtt::common::{QoS, QoSTest};
use crate::transport::tcp_v5::{self, TcpTransportV5Writer};

/// Incoming publish message
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub topic: ByteString,
    pub payload: Bytes,
    pub qos: QoSTest,
    pub retain: bool,
    pub dup: bool,
    pub response_topic: Option<ByteString>,
    pub correlation_data: Option<Bytes>,
    pub content_type: Option<ByteString>,
    pub user_properties: Vec<(ByteString, ByteString)>,
    pub is_utf8_payload: bool,
    pub message_expiry_interval: Option<NonZeroU32>,
}

/// Subscribe result
#[derive(Debug)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub status: Vec<rmqtt_codec::v5::SubscribeAckReason>,
}

/// MQTT v5.0 Client - enhanced with properties
pub struct MqttV5Client {
    writer: Arc<Mutex<TcpTransportV5Writer>>,
    connected: Arc<AtomicBool>,
    packet_id_counter: PacketIdCounter,

    /// Incoming publish receiver
    message_rx: mpsc::UnboundedReceiver<IncomingMessage>,

    /// Ack waiters for SUBACK
    suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>>,

    connack: Box<ConnectAck>,
}

impl MqttV5Client {
    /// Connect to broker with default settings
    pub async fn connect(broker_addr: &str, client_id: &str, connect_timeout: Duration) -> Result<Self> {
        Self::connect_with_options(
            broker_addr,
            client_id,
            connect_timeout,
            true,
            60,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
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
        session_expiry_interval: Option<u32>,
        receive_max: Option<NonZeroU16>,
        max_packet_size: Option<u32>,
    ) -> Result<Self> {
        let (mut reader, writer) = tcp_v5::connect(broker_addr, connect_timeout).await?;
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
                clean_start: clean_session,
                keep_alive,
                session_expiry_interval_secs: session_expiry_interval.unwrap_or(0),
                auth_method: None,
                auth_data: None,
                request_problem_info: false,
                request_response_info: false,
                receive_max,
                topic_alias_max: 0,
                user_properties: Vec::new(),
                max_packet_size: max_packet_size.and_then(NonZeroU32::new),
                last_will: will,
                client_id: ByteString::from(client_id),
                username,
                password,
                cert: None,
            };

            writer.lock().await.send_packet(&PacketV5::Connect(Box::new(conn))).await?;
        }

        //
        // WAIT CONNACK
        //
        let connack = {
            let pkt = reader.read_packet().await?;

            match pkt {
                PacketV5::ConnectAck(ack) => {
                    if ack.reason_code != ConnectAckReason::Success {
                        return Err(anyhow!("connect failed: {:?}", ack.reason_code));
                    }
                    *ack
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
                        PacketV5::Publish(pub_msg) => {
                            let qos = pub_msg.qos;
                            let packet_id = pub_msg.packet_id;

                            let (
                                response_topic,
                                correlation_data,
                                content_type,
                                user_properties,
                                is_utf8_payload,
                                message_expiry_interval,
                            ) = if let Some(ref props) = pub_msg.properties {
                                (
                                    props.response_topic.clone(),
                                    props.correlation_data.clone(),
                                    props.content_type.clone(),
                                    props.user_properties.clone(),
                                    props.is_utf8_payload,
                                    props.message_expiry_interval,
                                )
                            } else {
                                (None, None, None, Vec::new(), false, None)
                            };

                            let msg = IncomingMessage {
                                topic: pub_msg.topic.clone(),
                                payload: pub_msg.payload.clone(),
                                qos,
                                retain: pub_msg.retain,
                                dup: pub_msg.dup,
                                response_topic,
                                correlation_data,
                                content_type,
                                user_properties,
                                is_utf8_payload,
                                message_expiry_interval,
                            };
                            let _ = message_tx.send(msg);

                            // Send protocol acknowledgment
                            if let Some(pkt_id) = packet_id {
                                if qos == QoSTest::AtLeastOnce {
                                    // QoS 1: send PUBACK
                                    let ack = PacketV5::PublishAck(PublishAck {
                                        packet_id: pkt_id,
                                        reason_code: PublishAckReason::Success,
                                        properties: UserProperties::default(),
                                        reason_string: None,
                                    });
                                    let _ = writer.lock().await.send_packet(&ack).await;
                                } else if qos == QoSTest::ExactlyOnce {
                                    // QoS 2: send PUBREC
                                    let ack = PacketV5::PublishReceived(PublishAck {
                                        packet_id: pkt_id,
                                        reason_code: PublishAckReason::Success,
                                        properties: UserProperties::default(),
                                        reason_string: None,
                                    });
                                    let _ = writer.lock().await.send_packet(&ack).await;
                                }
                            }
                        }

                        // PUBREL (QoS 2 part 2): send PUBCOMP
                        PacketV5::PublishRelease(pubrel) => {
                            let ack = PacketV5::PublishComplete(PublishAck2 {
                                packet_id: pubrel.packet_id,
                                reason_code: PublishAck2Reason::Success,
                                properties: UserProperties::default(),
                                reason_string: None,
                            });
                            let _ = writer.lock().await.send_packet(&ack).await;
                        }

                        // SUBACK
                        PacketV5::SubscribeAck(suback) => {
                            let tx = { suback_waiters.lock().await.remove(&suback.packet_id.get()) };

                            if let Some(tx) = tx {
                                let _ = tx.send(Ok(SubscribeAck {
                                    packet_id: suback.packet_id,
                                    status: suback.status,
                                }));
                            }
                        }

                        // PUBACK (QoS 1) - broker ack for our publish
                        PacketV5::PublishAck(puback) => {
                            eprintln!("PUBACK received for packet_id: {}", puback.packet_id);
                        }

                        // PUBREC (QoS 2) - broker ack for our publish
                        PacketV5::PublishReceived(pubrec) => {
                            eprintln!("PUBREC received for packet_id: {}", pubrec.packet_id);
                        }

                        // PUBCOMP (QoS 2) - broker ack for our PUBREL
                        PacketV5::PublishComplete(pubcomp) => {
                            eprintln!("PUBCOMP received for packet_id: {}", pubcomp.packet_id);
                        }

                        // PINGRESP
                        PacketV5::PingResponse => {
                            // Handle ping response
                        }

                        // DISCONNECT
                        PacketV5::Disconnect(_) => {
                            eprintln!("Received DISCONNECT from broker");
                            break;
                        }

                        // AUTH
                        PacketV5::Auth(auth) => {
                            eprintln!("AUTH received: {:?}", auth);
                        }

                        // IGNORE OTHER PACKETS
                        other => {
                            tracing::debug!(packet = ?crate::transport::tcp_v5::packet_name_v5(&other), "ignored packet");
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
            connack: Box::new(connack),
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
        let packet_id = if qos != QoS::AtMostOnce {
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
            properties: None,
            payload: Bytes::copy_from_slice(payload),
        };

        self.writer.lock().await.send_packet(&PacketV5::Publish(Box::new(publish))).await?;

        Ok(())
    }

    /// Publish a message with V5 properties
    #[allow(clippy::too_many_arguments)]
    pub async fn publish_with_properties(
        &self,
        topic: &str,
        payload: &[u8],
        qos: QoSTest,
        retain: bool,
        payload_format_indicator: Option<bool>,
        message_expiry_interval: Option<u32>,
        response_topic: Option<&str>,
        correlation_data: Option<&[u8]>,
        content_type: Option<&str>,
        user_properties: Option<&[(String, String)]>,
    ) -> Result<()> {
        let packet_id = if qos != QoS::AtMostOnce {
            Some(
                NonZeroU16::new(u16::from(self.packet_id_counter.next()))
                    .ok_or_else(|| anyhow!("packet id overflow"))?,
            )
        } else {
            None
        };

        let props = PublishProperties {
            topic_alias: None,
            correlation_data: correlation_data.map(Bytes::copy_from_slice),
            message_expiry_interval: message_expiry_interval.and_then(NonZeroU32::new),
            content_type: content_type.map(ByteString::from),
            user_properties: user_properties
                .map(|ups| {
                    ups.iter()
                        .map(|(k, v)| (ByteString::from(k.as_str()), ByteString::from(v.as_str())))
                        .collect()
                })
                .unwrap_or_default(),
            is_utf8_payload: payload_format_indicator.unwrap_or(false),
            response_topic: response_topic.map(ByteString::from),
            subscription_ids: Vec::new(),
        };

        let publish = rmqtt_codec::types::Publish {
            dup: false,
            retain,
            qos,
            topic: ByteString::from(topic),
            packet_id,
            properties: Some(props),
            payload: Bytes::copy_from_slice(payload),
        };

        self.writer.lock().await.send_packet(&PacketV5::Publish(Box::new(publish))).await?;

        Ok(())
    }

    /// Subscribe to a topic with a specific QoS
    pub async fn subscribe(&mut self, topic: &str, qos: QoSTest) -> Result<SubscribeAck> {
        self.subscribe_with_options(topic, qos, false, false, rmqtt_codec::v5::RetainHandling::AtSubscribe)
            .await
    }

    /// Subscribe with MQTT 5.0 subscription options
    pub async fn subscribe_with_options(
        &mut self,
        topic: &str,
        qos: QoSTest,
        no_local: bool,
        retain_as_published: bool,
        retain_handling: rmqtt_codec::v5::RetainHandling,
    ) -> Result<SubscribeAck> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let subscribe_pkt = PacketV5::Subscribe(rmqtt_codec::v5::Subscribe {
            packet_id,
            id: None,
            user_properties: Vec::new(),
            topic_filters: vec![(
                ByteString::from(topic),
                SubscriptionOptions { qos, no_local, retain_as_published, retain_handling },
            )],
        });

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

        let unsub = PacketV5::Unsubscribe(rmqtt_codec::v5::Unsubscribe {
            packet_id,
            topic_filters: vec![ByteString::from(topic)],
            user_properties: Vec::new(),
        });

        self.writer.lock().await.send_packet(&unsub).await?;

        Ok(())
    }

    /// Send a PINGREQ
    pub async fn ping(&self) -> Result<()> {
        self.writer.lock().await.send_packet(&PacketV5::PingRequest).await
    }

    /// Disconnect gracefully
    pub async fn disconnect(&self) -> Result<()> {
        self.disconnect_with_reason(None).await
    }

    /// Disconnect with a V5 reason code
    pub async fn disconnect_with_reason(&self, reason_code: Option<u8>) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);

        let code = reason_code
            .and_then(|c| rmqtt_codec::v5::DisconnectReasonCode::try_from(c).ok())
            .unwrap_or(rmqtt_codec::v5::DisconnectReasonCode::NormalDisconnection);

        let disc = rmqtt_codec::v5::Disconnect {
            reason_code: code,
            session_expiry_interval_secs: None,
            server_reference: None,
            reason_string: None,
            user_properties: Vec::new(),
        };

        {
            let mut writer = self.writer.lock().await;
            let _ = writer.send_packet(&PacketV5::Disconnect(disc)).await;
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

    /// Receive incoming publish
    pub async fn recv_message(&mut self) -> Result<IncomingMessage> {
        self.message_rx.recv().await.ok_or_else(|| anyhow!("message channel closed"))
    }

    /// Receive incoming publish with timeout
    pub async fn recv_message_timeout(&mut self, timeout: Duration) -> Option<IncomingMessage> {
        time::timeout(timeout, self.recv_message()).await.ok().and_then(|r| r.ok())
    }
}
