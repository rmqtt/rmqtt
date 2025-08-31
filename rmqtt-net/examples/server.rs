use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use simple_logger::SimpleLogger;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::sleep;

use rmqtt_codec::types::Publish;
use rmqtt_codec::v3::{QoS, SubscribeReturnCode};
use rmqtt_codec::v5::{
    PublishAck2, PublishAckReason, SubscribeAckReason, UnsubscribeAckReason, UserProperties,
};
use rmqtt_codec::{v3, v5};
use rmqtt_net::{
    v3::MqttStream as MqttStreamV3, v5::MqttStream as MqttStreamV5, Builder, MqttError, MqttStream, Result,
};

#[tokio::main]
async fn main() -> Result<()> {
    SimpleLogger::new().with_level(log::LevelFilter::Info).init()?;

    let tcp_listener =
        Builder::new().name("external/tcp").laddr(([0, 0, 0, 0], 1883).into()).bind()?.tcp()?;

    let tls_listener = Builder::new()
        .name("external/tls")
        .laddr(([0, 0, 0, 0], 8883).into())
        .tls_key(Some("./rmqtt-bin/rmqtt.key"))
        .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
        .bind()?
        .tls()?;

    let ws_listener = Builder::new().name("external/ws").laddr(([0, 0, 0, 0], 8080).into()).bind()?.ws()?;

    let wss_listener = Builder::new()
        .name("external/wss")
        .laddr(([0, 0, 0, 0], 8443).into())
        .tls_key(Some("./rmqtt-bin/rmqtt.key"))
        .tls_cert(Some("./rmqtt-bin/rmqtt.pem"))
        .bind()?
        .wss()?;

    let tcp = async {
        loop {
            match tcp_listener.accept().await {
                Ok(a) => {
                    tokio::spawn(async move {
                        log::info!("tcp {:?}", a.remote_addr);
                        let d = match a.tcp() {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Failed to mqtt(tcp) accept, {e:?}");
                                return;
                            }
                        };
                        match d.mqtt().await {
                            Ok(MqttStream::V3(s)) => {
                                if let Err(e) = process_v3(s).await {
                                    log::warn!("Failed to process mqtt v3, {e:?}");
                                }
                            }
                            Ok(MqttStream::V5(s)) => {
                                if let Err(e) = process_v5(s).await {
                                    log::warn!("Failed to process mqtt v5, {e:?}");
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to probe MQTT version, {e:?}");
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to accept TCP socket connection, {e:?}");
                    sleep(Duration::from_millis(300)).await;
                }
            }
        }
    };

    let tls = async {
        loop {
            match tls_listener.accept().await {
                Ok(acceptor) => {
                    tokio::spawn(async move {
                        log::info!("tls {:?}", acceptor.remote_addr);
                        let d = match acceptor.tls().await {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Failed to mqtt(tls) accept, {e:?}");
                                return;
                            }
                        };
                        match d.mqtt().await {
                            Ok(MqttStream::V3(s)) => {
                                if let Err(e) = process_v3(s).await {
                                    log::warn!("Failed to process mqtt(tls) v3, {e:?}");
                                }
                            }
                            Ok(MqttStream::V5(s)) => {
                                if let Err(e) = process_v5(s).await {
                                    log::warn!("Failed to process mqtt(tls) v5, {e:?}");
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to probe MQTT(TLS) version, {e:?}");
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to accept TLS socket connection, {e:?}");
                    sleep(Duration::from_millis(300)).await;
                }
            }
        }
    };

    let ws = async {
        loop {
            match ws_listener.accept().await {
                Ok(acceptor) => {
                    tokio::spawn(async move {
                        log::info!("ws {:?}", acceptor.remote_addr);
                        let d = match acceptor.ws().await {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Failed to websocket accept, {e:?}");
                                return;
                            }
                        };
                        match d.mqtt().await {
                            Ok(MqttStream::V3(s)) => {
                                if let Err(e) = process_v3(s).await {
                                    log::warn!("Failed to process websocket mqtt v3, {e:?}");
                                }
                            }
                            Ok(MqttStream::V5(s)) => {
                                if let Err(e) = process_v5(s).await {
                                    log::warn!("Failed to process websocket mqtt v5, {e:?}");
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to websocket probe MQTT version, {e:?}");
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to websocket accept TCP socket connection, {e:?}");
                    sleep(Duration::from_millis(300)).await;
                }
            }
        }
    };

    let wss = async {
        loop {
            match wss_listener.accept().await {
                Ok(acceptor) => {
                    tokio::spawn(async move {
                        log::info!("wss {:?}", acceptor.remote_addr);
                        let d = match acceptor.wss().await {
                            Ok(d) => d,
                            Err(e) => {
                                log::warn!("Failed to websocket mqtt(tls) accept, {e:?}");
                                return;
                            }
                        };
                        match d.mqtt().await {
                            Ok(MqttStream::V3(s)) => {
                                if let Err(e) = process_v3(s).await {
                                    log::warn!("Failed to process websocket mqtt(tls) v3, {e:?}");
                                }
                            }
                            Ok(MqttStream::V5(s)) => {
                                if let Err(e) = process_v5(s).await {
                                    log::warn!("Failed to process websocket mqtt(tls) v5, {e:?}");
                                }
                            }
                            Err(e) => {
                                log::warn!("Failed to websocket probe MQTT(TLS) version, {e:?}");
                            }
                        }
                    });
                }
                Err(e) => {
                    log::warn!("Failed to websocket accept TLS socket connection, {e:?}");
                    sleep(Duration::from_millis(300)).await;
                }
            }
        }
    };

    futures::future::join4(tcp, tls, ws, wss).await;

    Ok(())
}

static PID_GEN: Lazy<AtomicU16> = Lazy::new(|| AtomicU16::new(65533));
fn gen_packet_id() -> NonZeroU16 {
    let id = PID_GEN.fetch_add(1, Ordering::SeqCst);
    log::info!("gen_packet_id: {id}");
    if id < u16::MAX {
        NonZeroU16::new(id).unwrap()
    } else {
        PID_GEN.store(1, Ordering::SeqCst);
        NonZeroU16::new(id).unwrap()
    }
}

async fn process_v3<Io>(mut s: MqttStreamV3<Io>) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(packet) = s.recv(Duration::from_secs(90)).await? {
        log::info!("recv packet: {packet:?}");
        match packet {
            v3::Packet::Connect(_c) => {
                s.send_connect_ack(v3::ConnectAckReason::ConnectionAccepted, false).await?;
            }
            v3::Packet::Subscribe { packet_id, topic_filters } => {
                let status =
                    topic_filters.iter().map(|(_tf, qos)| SubscribeReturnCode::Success(*qos)).collect();
                s.send_subscribe_ack(packet_id, status).await?;
            }
            v3::Packet::Unsubscribe { packet_id, topic_filters: _ } => {
                s.send_unsubscribe_ack(packet_id).await?;
            }
            v3::Packet::Publish(p) => {
                if p.qos == QoS::AtLeastOnce {
                    s.send_publish_ack(p.packet_id.ok_or(MqttError::ServiceUnavailable)?).await.unwrap();

                    s.send_publish(Box::new(Publish {
                        dup: false,
                        retain: false,
                        qos: p.qos,
                        packet_id: Some(gen_packet_id()),
                        topic: p.topic,
                        payload: p.payload,
                        properties: p.properties,
                    }))
                    .await?;
                } else {
                    log::info!("unimplemented!");
                }
            }
            v3::Packet::Disconnect => {}
            v3::Packet::PingRequest => {
                s.send_ping_response().await.unwrap();
            }
            v3::Packet::ConnectAck(_) => {}
            v3::Packet::PublishAck { .. } => {}
            v3::Packet::PublishReceived { packet_id } => {
                s.send_publish_release(packet_id).await?;
            }
            v3::Packet::PublishRelease { packet_id } => {
                s.send_publish_complete(packet_id).await?;
            }
            v3::Packet::PublishComplete { .. } => {}
            v3::Packet::SubscribeAck { .. } => {}
            v3::Packet::UnsubscribeAck { .. } => {}
            v3::Packet::PingResponse => {}
        }
    }
    Ok(())
}

async fn process_v5<Io>(mut s: MqttStreamV5<Io>) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    while let Some(packet) = s.recv(Duration::from_secs(90)).await? {
        log::info!("recv packet: {packet:?}");
        match packet {
            v5::Packet::Connect(_c) => s.send_connect_ack(v5::ConnectAck::default()).await?,
            v5::Packet::Subscribe(sub) => {
                s.send_subscribe_ack(v5::SubscribeAck {
                    packet_id: sub.packet_id,
                    properties: UserProperties::default(),
                    reason_string: None,
                    status: vec![SubscribeAckReason::GrantedQos1],
                })
                .await?;
            }
            v5::Packet::Unsubscribe(unsub) => {
                s.send_unsubscribe_ack(v5::UnsubscribeAck {
                    packet_id: unsub.packet_id,
                    properties: UserProperties::new(),
                    reason_string: None,
                    status: vec![UnsubscribeAckReason::Success],
                })
                .await?;
            }
            v5::Packet::Publish(p) => {
                if p.qos == QoS::AtLeastOnce {
                    let ack = v5::PublishAck {
                        packet_id: p.packet_id.ok_or(MqttError::ServiceUnavailable)?,
                        reason_code: PublishAckReason::Success,
                        properties: UserProperties::default(),
                        reason_string: None,
                    };

                    s.send_publish_ack(ack).await?;

                    s.send_publish(Box::new(Publish {
                        dup: false,
                        retain: false,
                        qos: p.qos,
                        packet_id: Some(gen_packet_id()),
                        topic: p.topic,
                        payload: p.payload,
                        properties: p.properties,
                    }))
                    .await?;
                } else {
                    log::info!("unimplemented!");
                }
            }
            v5::Packet::Disconnect(_dis) => {}
            v5::Packet::PingRequest => {
                s.send_ping_response().await?;
            }

            v5::Packet::ConnectAck(_) => {}
            v5::Packet::PublishAck(_) => {}
            v5::Packet::PublishReceived(ack) => {
                s.send_publish_release(PublishAck2 { packet_id: ack.packet_id, ..Default::default() })
                    .await?;
            }
            v5::Packet::PublishRelease(acks) => {
                s.send_publish_complete(acks).await?;
            }
            v5::Packet::PublishComplete(_) => {}
            v5::Packet::SubscribeAck(_) => {}
            v5::Packet::UnsubscribeAck(_) => {}
            v5::Packet::PingResponse => {}
            v5::Packet::Auth(_) => {}
        }
    }
    Ok(())
}
