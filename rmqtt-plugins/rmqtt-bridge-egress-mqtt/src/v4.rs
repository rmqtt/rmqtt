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
use ntex_mqtt::{self, v3};

use rmqtt::ntex_mqtt::types::MQTT_LEVEL_311;

use rmqtt::futures::channel::mpsc;
use rmqtt::futures::StreamExt;
use rmqtt::log;
use rmqtt::{ClientId, MqttError, NodeId, Result};

use crate::bridge::{BridgePublish, Command, CommandMailbox};
use crate::config::Bridge;

#[derive(Clone)]
pub struct Client {
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) server_addr: SocketAddr,
    pub(crate) client_id: ClientId,
    closed: Rc<AtomicBool>,
    sink: Rc<RefCell<Option<v3::MqttSink>>>,
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

        let mut builder = v3::client::MqttConnector::new(client.server_addr)
            .client_id(ByteString::from(client.client_id.as_ref()))
            .keep_alive(Seconds(client.cfg.keepalive.as_secs() as u16))
            .handshake_timeout(Seconds(client.cfg.connect_timeout.as_secs() as u16));

        if let Some(username) = client.cfg.username.as_ref() {
            builder = builder.username(username.clone());
        }
        if let Some(password) = client.cfg.password.as_ref() {
            builder = builder.password(Bytes::from(password.clone()));
        }

        if client.cfg.mqtt_ver.level() == MQTT_LEVEL_311 {
            if client.cfg.v4.clean_session {
                builder = builder.clean_session()
            };

            if let Some(last_will) = client.cfg.v4.last_will.as_ref() {
                builder = builder.last_will(last_will.clone());
            }

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
                Some(Command::Publish(BridgePublish::V3(p))) => {
                    log::debug!("{} Command::Publish, {:?}", self.client_id, p);
                    let sink = self.sink.borrow().as_ref().cloned();
                    if let Some(sink) = sink {
                        if matches!(p.qos, ntex_mqtt::QoS::AtMostOnce) {
                            if let Err(e) = sink.publish_pkt(p).send_at_most_once() {
                                log::warn!("{}", e);
                            }
                        } else if let Err(e) = sink.publish_pkt(p).send_at_least_once().await {
                            log::warn!("{}", e);
                        }
                    } else {
                        log::error!("mqtt sink is None");
                    }
                }
                Some(Command::Publish(BridgePublish::V5(_))) => {
                    log::error!("unreachable!()");
                }
            }
        }
    }

    async fn start(self, builder: v3::client::MqttConnector<SocketAddr, Connector<SocketAddr>>) {
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
    //     sink: v3::MqttSink,
    //     topic_filter: ByteString,
    //     qos: v3::QoS,
    //     client_id: ByteString,
    // ) {
    //     'subscribe: loop {
    //         match sink.subscribe().topic_filter(topic_filter.clone(), qos).send().await {
    //             Ok(rets) => {
    //                 for ret in rets {
    //                     if let SubscribeReturnCode::Failure = ret {
    //                         log::info!("{} Subscribe failure, topic_filter: {:?}", client_id, topic_filter);
    //                         time::sleep(Duration::from_secs(5)).await;
    //                         continue 'subscribe;
    //                     } else {
    //                         log::info!("{} Successfully subscribed to {:?}", client_id, topic_filter,);
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

    async fn ev_loop(self, c: v3::client::Client) {
        if let Err(e) = c
            .start(move |control: v3::client::ControlMessage<()>| match control {
                v3::client::ControlMessage::Publish(publish) => {
                    log::debug!("{} publish: {:?}", self.client_id, publish);
                    // self.on_message.fire((
                    //     BridgeClient::V4(self.clone()),
                    //     Some(self.server_addr),
                    //     None,
                    //     BridgePublish::V3(publish.packet().clone()),
                    // ));
                    Ready::Ok(publish.ack())
                }
                v3::client::ControlMessage::Error(msg) => {
                    log::info!("{} Codec error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v3::client::ControlMessage::ProtocolError(msg) => {
                    log::info!("{} Protocol error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v3::client::ControlMessage::PeerGone(msg) => {
                    log::info!("{} Peer closed connection: {:?}", self.client_id, msg.err());
                    Ready::Ok(msg.ack())
                }
                v3::client::ControlMessage::Closed(msg) => {
                    log::info!("{} Server closed connection", self.client_id);
                    Ready::Ok(msg.ack())
                }
            })
            .await
        {
            log::error!("Start ev_loop error! {:?}", e);
        }
    }
}
