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
    PublishAckReason, PublishProperties, SubscriptionOptions, UnsubscribeAckReason, UserProperties,
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
    /// Subscription Identifiers attached by the broker to this delivery
    /// (empty for QoS 0-less brokers or when no Subscription Identifier
    /// was used in the matching subscriptions).
    pub subscription_ids: Vec<NonZeroU32>,
    /// Packet identifier of the QoS 1/2 delivery (None for QoS 0), so tests
    /// that disable auto-acknowledgement can ack manually.
    pub packet_id: Option<NonZeroU16>,
}

/// Subscribe result
#[derive(Debug)]
pub struct SubscribeAck {
    pub packet_id: NonZeroU16,
    pub status: Vec<rmqtt_codec::v5::SubscribeAckReason>,
}

/// Shared UNSUBACK waiter map: packet id -> one-shot sender of the ack.
type UnsubAckWaiters = Arc<Mutex<HashMap<u16, oneshot::Sender<Result<Vec<UnsubscribeAckReason>>>>>>;

/// MQTT v5.0 Client - enhanced with properties
pub struct MqttV5Client {
    writer: Arc<Mutex<TcpTransportV5Writer>>,
    connected: Arc<AtomicBool>,
    packet_id_counter: PacketIdCounter,

    /// Incoming publish receiver
    message_rx: mpsc::UnboundedReceiver<IncomingMessage>,

    /// Ack waiters for SUBACK
    suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>>,

    /// Whether to automatically answer incoming PUBREL with PUBCOMP (QoS 2 part 2).
    /// Disabling allows tests to leave a QoS 2 exchange incomplete.
    auto_pubcomp: Arc<AtomicBool>,

    /// Incoming PUBREL packet id receiver (broker -> client, QoS 2 part 2)
    pubrel_rx: mpsc::UnboundedReceiver<NonZeroU16>,

    /// Whether to automatically answer incoming QoS 1 PUBLISH with PUBACK.
    /// Disabling allows tests to stall broker -> client in-flight (flow
    /// control / Receive Maximum verification).
    auto_puback: Arc<AtomicBool>,

    /// PUBACK (QoS 1) reason codes from the broker, as (packet_id, reason_code u8)
    puback_rx: mpsc::UnboundedReceiver<(NonZeroU16, u8)>,

    /// PUBREC (QoS 2) reason codes from the broker, as (packet_id, reason_code u8)
    pubrec_rx: mpsc::UnboundedReceiver<(NonZeroU16, u8)>,

    /// Ack waiters for UNSUBACK
    unsuback_waiters: UnsubAckWaiters,

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
        let (puback_tx, puback_rx) = mpsc::unbounded_channel();
        let (pubrec_tx, pubrec_rx) = mpsc::unbounded_channel();
        let suback_waiters: Arc<Mutex<HashMap<u16, oneshot::Sender<Result<SubscribeAck>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let unsuback_waiters: UnsubAckWaiters = Arc::new(Mutex::new(HashMap::new()));
        let auto_pubcomp = Arc::new(AtomicBool::new(true));
        let auto_puback = Arc::new(AtomicBool::new(true));
        let (pubrel_tx, pubrel_rx) = mpsc::unbounded_channel::<NonZeroU16>();

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
            let unsuback_waiters = unsuback_waiters.clone();
            let auto_pubcomp = auto_pubcomp.clone();
            let auto_puback = auto_puback.clone();
            let pubrel_tx = pubrel_tx.clone();
            let puback_tx = puback_tx.clone();
            let pubrec_tx = pubrec_tx.clone();

            tokio::spawn(async move {
                loop {
                    let pkt = match reader.read_packet().await {
                        Ok(pkt) => pkt,
                        Err(err) => {
                            eprintln!("mqtt read error: {:?}", err);
                            connected.store(false, Ordering::Relaxed);
                            // Resolve all pending ack waiters with an error so
                            // callers fail fast instead of waiting for their
                            // own timeouts.
                            for (_, tx) in suback_waiters.lock().await.drain() {
                                let _ = tx.send(Err(anyhow!(
                                    "connection closed by broker (read error: {:?})",
                                    err
                                )));
                            }
                            for (_, tx) in unsuback_waiters.lock().await.drain() {
                                let _ = tx.send(Err(anyhow!(
                                    "connection closed by broker (read error: {:?})",
                                    err
                                )));
                            }
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
                                subscription_ids,
                            ) = if let Some(ref props) = pub_msg.properties {
                                (
                                    props.response_topic.clone(),
                                    props.correlation_data.clone(),
                                    props.content_type.clone(),
                                    props.user_properties.clone(),
                                    props.is_utf8_payload,
                                    props.message_expiry_interval,
                                    props.subscription_ids.clone(),
                                )
                            } else {
                                (None, None, None, Vec::new(), false, None, Vec::new())
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
                                subscription_ids,
                                packet_id,
                            };
                            let _ = message_tx.send(msg);

                            // Send protocol acknowledgment
                            if let Some(pkt_id) = packet_id {
                                if qos == QoSTest::AtLeastOnce && auto_puback.load(Ordering::Relaxed) {
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

                        // PUBREL (QoS 2 part 2): forward the event, send PUBCOMP if auto-ack is on
                        PacketV5::PublishRelease(pubrel) => {
                            let _ = pubrel_tx.send(pubrel.packet_id);
                            if auto_pubcomp.load(Ordering::Relaxed) {
                                let ack = PacketV5::PublishComplete(PublishAck2 {
                                    packet_id: pubrel.packet_id,
                                    reason_code: PublishAck2Reason::Success,
                                    properties: UserProperties::default(),
                                    reason_string: None,
                                });
                                let _ = writer.lock().await.send_packet(&ack).await;
                            }
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
                            let _ = puback_tx.send((puback.packet_id, puback.reason_code as u8));
                        }

                        // PUBREC (QoS 2) - broker ack for our publish
                        PacketV5::PublishReceived(pubrec) => {
                            let _ = pubrec_tx.send((pubrec.packet_id, pubrec.reason_code as u8));
                        }

                        // UNSUBACK - resolve the pending unsubscribe waiter
                        PacketV5::UnsubscribeAck(unsuback) => {
                            let tx = { unsuback_waiters.lock().await.remove(&unsuback.packet_id.get()) };
                            if let Some(tx) = tx {
                                let _ = tx.send(Ok(unsuback.status));
                            }
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
                        PacketV5::Disconnect(d) => {
                            eprintln!("Received DISCONNECT from broker, reason: {:?}", d.reason_code);
                            connected.store(false, Ordering::Relaxed);
                            // Resolve all pending ack waiters with an error so
                            // callers fail fast instead of waiting for their
                            // own timeouts.
                            for (_, tx) in suback_waiters.lock().await.drain() {
                                let _ = tx.send(Err(anyhow!(
                                    "broker disconnected without ack (reason {:?})",
                                    d.reason_code
                                )));
                            }
                            for (_, tx) in unsuback_waiters.lock().await.drain() {
                                let _ = tx.send(Err(anyhow!(
                                    "broker disconnected without ack (reason {:?})",
                                    d.reason_code
                                )));
                            }
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
            auto_pubcomp,
            auto_puback,
            pubrel_rx,
            puback_rx,
            pubrec_rx,
            unsuback_waiters,
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

        self.writer.lock().await.send_packet(&PacketV5::Publish(Box::new(publish))).await?;

        Ok(())
    }

    /// Send a PUBREL (QoS 2 part 2) with the given packet id
    pub async fn send_pubrel(&self, packet_id: NonZeroU16) -> Result<()> {
        let ack2 = PublishAck2 {
            packet_id,
            reason_code: PublishAck2Reason::Success,
            properties: UserProperties::default(),
            reason_string: None,
        };
        self.writer.lock().await.send_packet(&PacketV5::PublishRelease(ack2)).await?;
        Ok(())
    }

    /// Enable/disable the automatic PUBCOMP sent in reply to an incoming PUBREL.
    ///
    /// Disabling allows tests to leave a QoS 2 exchange incomplete (the broker
    /// keeps owing a PUBCOMP), e.g. to verify MQTT-4.4.0-1 PUBREL resend on resume.
    pub fn set_auto_pubcomp(&self, enabled: bool) {
        self.auto_pubcomp.store(enabled, Ordering::Relaxed);
    }

    /// Enable/disable the automatic PUBACK sent in reply to incoming QoS 1
    /// PUBLISH. Disabling lets a test stall the broker -> client in-flight
    /// window (e.g. to verify the broker honours the client's Receive Maximum)
    /// and acknowledge manually via [`Self::send_puback`].
    pub fn set_auto_puback(&self, enabled: bool) {
        self.auto_puback.store(enabled, Ordering::Relaxed);
    }

    /// Manually acknowledge an incoming QoS 1 PUBLISH (broker -> client).
    pub async fn send_puback(&self, packet_id: NonZeroU16) -> Result<()> {
        let ack = PacketV5::PublishAck(PublishAck {
            packet_id,
            reason_code: PublishAckReason::Success,
            properties: UserProperties::default(),
            reason_string: None,
        });
        self.writer.lock().await.send_packet(&ack).await?;
        Ok(())
    }

    /// Wait for an incoming PUBREL packet id (broker -> client, QoS 2 part 2)
    pub async fn recv_pubrel_timeout(&mut self, timeout: Duration) -> Option<u16> {
        time::timeout(timeout, self.pubrel_rx.recv()).await.ok().and_then(|r| r).map(|pid| pid.get())
    }

    /// Subscribe to a topic with a specific QoS
    pub async fn subscribe(&mut self, topic: &str, qos: QoSTest) -> Result<SubscribeAck> {
        self.subscribe_with_options(topic, qos, false, false, rmqtt_codec::v5::RetainHandling::AtSubscribe)
            .await
    }

    /// Subscribe multiple topic filters in a SINGLE SUBSCRIBE packet, each
    /// with its own QoS (default options otherwise). The SUBACK statuses are
    /// returned in filter order so tests can verify per-filter grants.
    pub async fn subscribe_many(&mut self, filters: &[(&str, QoSTest)]) -> Result<SubscribeAck> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let topic_filters = filters
            .iter()
            .map(|(topic, qos)| {
                (
                    ByteString::from(*topic),
                    SubscriptionOptions {
                        qos: *qos,
                        no_local: false,
                        retain_as_published: false,
                        retain_handling: rmqtt_codec::v5::RetainHandling::AtSubscribe,
                    },
                )
            })
            .collect();

        let subscribe_pkt = PacketV5::Subscribe(rmqtt_codec::v5::Subscribe {
            packet_id,
            id: None,
            user_properties: Vec::new(),
            topic_filters,
        });

        // REGISTER ACK WAITER
        let (tx, rx) = oneshot::channel();
        self.suback_waiters.lock().await.insert(packet_id.get(), tx);

        // SEND SUBSCRIBE
        self.writer.lock().await.send_packet(&subscribe_pkt).await?;

        // Bail out early if the connection died while writing the packet
        // (avoids racing the reader task's waiter cleanup).
        if !self.is_connected() {
            self.suback_waiters.lock().await.remove(&packet_id.get());
            return Err(anyhow!("connection closed by broker while waiting for SUBACK"));
        }

        // WAIT SUBACK
        let ack = time::timeout(Duration::from_secs(15), rx)
            .await
            .map_err(|_| anyhow!("subscribe timeout"))?
            .map_err(|_| anyhow!("suback waiter dropped"))??;

        Ok(ack)
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
        self.subscribe_full(topic, qos, no_local, retain_as_published, retain_handling, None).await
    }

    /// Subscribe with a Subscription Identifier (MQTT-3.8.2.1.2), used to
    /// verify that deliveries carry the identifier and that re-subscribing
    /// with a new identifier updates it.
    pub async fn subscribe_with_id(
        &mut self,
        topic: &str,
        qos: QoSTest,
        subscription_id: u32,
    ) -> Result<SubscribeAck> {
        self.subscribe_full(topic, qos, false, false, rmqtt_codec::v5::RetainHandling::AtSubscribe, {
            Some(
                NonZeroU32::new(subscription_id)
                    .ok_or_else(|| anyhow!("subscription id must be non-zero"))?,
            )
        })
        .await
    }

    async fn subscribe_full(
        &mut self,
        topic: &str,
        qos: QoSTest,
        no_local: bool,
        retain_as_published: bool,
        retain_handling: rmqtt_codec::v5::RetainHandling,
        subscription_id: Option<NonZeroU32>,
    ) -> Result<SubscribeAck> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let subscribe_pkt = PacketV5::Subscribe(rmqtt_codec::v5::Subscribe {
            packet_id,
            id: subscription_id,
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

        // Bail out early if the connection died while writing the packet
        // (avoids racing the reader task's waiter cleanup).
        if !self.is_connected() {
            self.suback_waiters.lock().await.remove(&packet_id.get());
            return Err(anyhow!("connection closed by broker while waiting for SUBACK"));
        }

        // WAIT SUBACK
        let ack = time::timeout(Duration::from_secs(15), rx)
            .await
            .map_err(|_| anyhow!("subscribe timeout"))?
            .map_err(|_| anyhow!("suback waiter dropped"))??;

        Ok(ack)
    }

    /// Unsubscribe from a topic
    pub async fn unsubscribe(&mut self, topic: &str) -> Result<()> {
        self.unsubscribe_with_ack(topic).await.map(|_| ())
    }

    /// Unsubscribe and wait for the UNSUBACK, returning the per-filter
    /// reason codes (e.g. 0x00 Success, 0x11 No Subscription Existed).
    pub async fn unsubscribe_with_ack(&mut self, topic: &str) -> Result<Vec<UnsubscribeAckReason>> {
        let packet_id = NonZeroU16::new(u16::from(self.packet_id_counter.next()))
            .ok_or_else(|| anyhow!("packet id overflow"))?;

        let unsub = PacketV5::Unsubscribe(rmqtt_codec::v5::Unsubscribe {
            packet_id,
            topic_filters: vec![ByteString::from(topic)],
            user_properties: Vec::new(),
        });

        // REGISTER ACK WAITER
        let (tx, rx) = oneshot::channel();
        self.unsuback_waiters.lock().await.insert(packet_id.get(), tx);

        self.writer.lock().await.send_packet(&unsub).await?;

        // Bail out early if the connection died while writing the packet
        // (avoids racing the reader task's waiter cleanup).
        if !self.is_connected() {
            self.unsuback_waiters.lock().await.remove(&packet_id.get());
            return Err(anyhow!("connection closed by broker while waiting for UNSUBACK"));
        }

        // WAIT UNSUBACK
        let status = time::timeout(Duration::from_secs(15), rx)
            .await
            .map_err(|_| anyhow!("unsubscribe timeout"))?
            .map_err(|_| anyhow!("unsuback waiter dropped"))??;

        Ok(status)
    }

    /// Wait for a PUBACK (QoS 1 ack) and return (packet_id, reason_code u8).
    pub async fn recv_puback_reason(&mut self, timeout: Duration) -> Option<(NonZeroU16, u8)> {
        time::timeout(timeout, self.puback_rx.recv()).await.ok().flatten()
    }

    /// Wait for a PUBREC (QoS 2 ack) and return (packet_id, reason_code u8).
    pub async fn recv_pubrec_reason(&mut self, timeout: Duration) -> Option<(NonZeroU16, u8)> {
        time::timeout(timeout, self.pubrec_rx.recv()).await.ok().flatten()
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

    /// Disconnect gracefully with a V5 session-expiry property, which
    /// overrides the CONNECT-requested value for the ending session.
    /// `None` omits the property (the session keeps the CONNECT value).
    pub async fn disconnect_with_session_expiry(&self, expiry_secs: Option<u32>) -> Result<()> {
        self.connected.store(false, Ordering::Relaxed);

        let disc = rmqtt_codec::v5::Disconnect {
            reason_code: rmqtt_codec::v5::DisconnectReasonCode::NormalDisconnection,
            session_expiry_interval_secs: expiry_secs,
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
