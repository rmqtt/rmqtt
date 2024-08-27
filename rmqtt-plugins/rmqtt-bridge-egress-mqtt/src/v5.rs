use std::cell::RefCell;
use std::net::{SocketAddr, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ntex::connect::Connector;
use ntex::time::Seconds;
use ntex::util::ByteString;
use ntex::util::Bytes;
use ntex::util::Ready;
use ntex_mqtt::v5::client::ControlMessage;
use ntex_mqtt::{self, v5};

use rmqtt::{
    futures::{channel::mpsc, StreamExt},
    log,
    ntex_mqtt::types::MQTT_LEVEL_5,
};
use rmqtt::{ClientId, MqttError, NodeId, Result};

use crate::bridge::{BridgePublish, Command, CommandMailbox};
use crate::config::Bridge;

#[derive(Clone)]
pub struct Client {
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) server_addr: SocketAddr,
    pub(crate) client_id: ClientId,
    closed: Rc<AtomicBool>,
    sink: Rc<RefCell<Option<v5::MqttSink>>>,
}

impl Client {
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn close(&self) -> bool {
        if let Some(sink) = self.sink.borrow().as_ref() {
            sink.close();
            self.closed.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub(crate) fn connect(
        cfg: Bridge,
        entry_idx: usize,
        node_id: NodeId,
        client_no: usize,
    ) -> Result<CommandMailbox> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(cfg.message_channel_capacity);

        let client_id =
            format!("{}:{}:egress:{}:{}:{}", cfg.client_id_prefix, cfg.name, node_id, entry_idx, client_no);

        let server_addr = cfg.server.to_socket_addrs()?.next().ok_or_else(|| MqttError::from("None"))?;

        let client = Self {
            cfg: Arc::new(cfg),
            server_addr,
            client_id: ClientId::from(client_id),
            closed: Rc::new(AtomicBool::new(false)),
            sink: Rc::new(RefCell::new(None)),
        };

        let mut builder = v5::client::MqttConnector::new(client.server_addr)
            .client_id(ByteString::from(client.client_id.as_ref()))
            .keep_alive(Seconds(client.cfg.keepalive.as_secs() as u16))
            .handshake_timeout(Seconds(client.cfg.connect_timeout.as_secs() as u16));

        if let Some(username) = client.cfg.username.as_ref() {
            builder = builder.username(ByteString::from(username.as_str()));
        }
        if let Some(password) = client.cfg.password.as_ref() {
            builder = builder.password(Bytes::from(password.clone()));
        }

        if client.cfg.mqtt_ver.level() == MQTT_LEVEL_5 {
            if client.cfg.v5.clean_start {
                builder = builder.clean_start()
            };

            builder = builder.receive_max(client.cfg.v5.receive_maximum);
            builder = builder.max_packet_size(client.cfg.v5.maximum_packet_size.as_u32());

            builder = builder.packet(|pkt| {
                pkt.session_expiry_interval_secs = client.cfg.v5.session_expiry_interval.as_secs() as u32;
                pkt.topic_alias_max = client.cfg.v5.topic_alias_maximum;
                pkt.last_will.clone_from(&client.cfg.v5.last_will)
            });

            ntex::rt::spawn(client.clone().start(builder));
            ntex::rt::spawn(client.clone().cmd_loop(cmd_rx));
        } else {
            unreachable!()
        }

        Ok(CommandMailbox::new(client.cfg.clone(), client.client_id, cmd_tx))
    }

    async fn cmd_loop(self, mut cmd_rx: mpsc::Receiver<Command>) {
        while !self.is_closed() {
            match cmd_rx.next().await {
                None => break,
                Some(Command::Connect) => {
                    log::debug!("{} Command::Connect ...", self.client_id);
                }
                Some(Command::Close) => {
                    log::debug!("{} Command::Close ...", self.client_id);
                    self.close();
                    break;
                }
                Some(Command::Publish(BridgePublish::V5(p))) => {
                    log::debug!("{} Command::Publish, {:?}", self.client_id, p);
                    let sink = self.sink.borrow().as_ref().cloned();
                    if let Some(sink) = sink {
                        if matches!(p.qos, ntex_mqtt::QoS::AtMostOnce) {
                            if let Err(e) = sink.publish_pkt(p).send_at_most_once() {
                                log::warn!("{}", e);
                            }
                        } else if let Err(e) = sink.clone().publish_pkt(p).send_at_least_once().await {
                            log::warn!("{}", e);
                        }
                    } else {
                        log::error!("mqtt sink is None");
                    }
                }
                Some(Command::Publish(BridgePublish::V3(_))) => {
                    log::error!("unreachable!()");
                }
            }
        }
    }

    async fn start(self, builder: v5::client::MqttConnector<SocketAddr, Connector<SocketAddr>>) {
        let client = self;
        let sleep_interval = client.cfg.reconnect_interval;
        loop {
            match builder.connect().await {
                Ok(c) => {
                    log::info!("{} Successfully connected to {:?}", client.client_id, client.cfg.server);

                    let sink = c.sink();
                    client.sink.replace(Some(sink.clone()));

                    //client event loop
                    client.clone().ev_loop(c).await;
                }
                Err(e) => {
                    log::warn!("{} Connect to {:?} fail, {:?}", client.client_id, client.cfg.server, e);
                }
            }
            if client.is_closed() {
                break;
            } else {
                ntex::time::sleep(sleep_interval).await;
            }
        }
        log::info!("{} Exit 'rmqtt-bridge-ingress-mqtt' client", client.client_id);
    }

    // async fn subscribe(
    //     self,
    //     sink: v5::MqttSink,
    //     topic_filter: ByteString,
    //     qos: v5::QoS,
    //     client_id: ByteString,
    // ) {
    //     let opts = v5::codec::SubscriptionOptions {
    //         qos,
    //         no_local: false,
    //         retain_as_published: false,
    //         retain_handling: v5::codec::RetainHandling::AtSubscribe,
    //     };
    //     'subscribe: loop {
    //         match sink.subscribe(None).topic_filter(topic_filter.clone(), opts).send().await {
    //             Ok(ack) => {
    //                 for reason in &ack.status {
    //                     if matches!(
    //                         reason,
    //                         SubscribeAckReason::GrantedQos0
    //                             | SubscribeAckReason::GrantedQos1
    //                             | SubscribeAckReason::GrantedQos2
    //                     ) {
    //                         log::info!("{} Successfully subscribed to {:?}", client_id, topic_filter,);
    //                     } else {
    //                         log::warn!(
    //                             "{} Subscribe failure, topic_filter: {:?}, reason: {:?}",
    //                             client_id,
    //                             topic_filter,
    //                             reason
    //                         );
    //                         time::sleep(Duration::from_secs(5)).await;
    //                         continue 'subscribe;
    //                     }
    //                 }
    //                 break;
    //             }
    //             Err(SendPacketError::Disconnected) => {
    //                 log::info!("{} Subscribe error, Disconnected", client_id);
    //                 break;
    //             }
    //             Err(e) => {
    //                 log::info!("{} Subscribe error, {:?}", client_id, e);
    //                 break;
    //             }
    //         }
    //     }
    // }

    async fn ev_loop(self, c: v5::client::Client) {
        if let Err(e) = c
            .start(move |control: ControlMessage<()>| match control {
                ControlMessage::Publish(publish) => {
                    log::debug!("{} publish: {:?}", self.client_id, publish);
                    // self.on_message.fire((
                    //     BridgeClient::V5(self.clone()),
                    //     Some(self.server_addr),
                    //     None,
                    //     BridgePublish::V5(publish.packet().clone()),
                    // ));
                    let reason_code = v5::codec::PublishAckReason::Success;
                    Ready::Ok(publish.ack(reason_code))
                }
                ControlMessage::Error(msg) => {
                    log::info!("{} Codec error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack(v5::codec::DisconnectReasonCode::NormalDisconnection))
                }
                ControlMessage::ProtocolError(msg) => {
                    log::info!("{} Protocol error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                ControlMessage::PeerGone(msg) => {
                    log::info!("{} Peer closed connection: {:?}", self.client_id, msg.error());
                    Ready::Ok(msg.ack())
                }
                ControlMessage::Closed(msg) => {
                    log::info!("{} Server closed connection", self.client_id);
                    Ready::Ok(msg.ack())
                }
                ControlMessage::Disconnect(msg) => {
                    log::info!("{} Server disconnect connection: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
            })
            .await
        {
            log::error!("Start ev_loop error! {:?}", e);
        }
    }
}
