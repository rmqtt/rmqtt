use anyhow::anyhow;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::{channel::mpsc, StreamExt};
use ntex::connect::rustls::TlsConnector;
use ntex::connect::Connector;
use ntex::time::Seconds;
use ntex::util::ByteString;
use ntex::util::Bytes;
use ntex::{service::fn_service, util::Ready};
use ntex_mqtt::{self, v5};

use rmqtt::{codec::types::MQTT_LEVEL_5, ClientId, NodeId, Result};

use crate::bridge::{build_tls_connector, BridgePublish, Command, CommandMailbox};
use crate::config::Bridge;

enum MqttConnector {
    Tcp(v5::client::MqttConnector<String, Connector<String>>),
    Tls(v5::client::MqttConnector<String, TlsConnector<String>>),
}

impl MqttConnector {
    async fn connect(&self) -> Result<v5::client::Client> {
        Ok(match self {
            MqttConnector::Tcp(c) => c.connect().await.map_err(|e| anyhow!(e))?,
            MqttConnector::Tls(c) => c.connect().await.map_err(|e| anyhow!(e))?,
        })
    }
}

#[derive(Clone)]
pub struct Client {
    pub(crate) cfg: Arc<Bridge>,
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

        let client = Self {
            cfg: Arc::new(cfg),
            client_id: ClientId::from(client_id),
            closed: Rc::new(AtomicBool::new(false)),
            sink: Rc::new(RefCell::new(None)),
        };

        let mut builder = v5::client::MqttConnector::new(client.cfg.server.addr.clone())
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

            builder = builder.max_receive(client.cfg.v5.receive_maximum);
            builder = builder.max_packet_size(client.cfg.v5.maximum_packet_size.as_u32());

            builder = builder.packet(|pkt| {
                pkt.session_expiry_interval_secs = client.cfg.v5.session_expiry_interval.as_secs() as u32;
                pkt.topic_alias_max = client.cfg.v5.topic_alias_maximum;
                pkt.last_will.clone_from(&client.cfg.v5.last_will)
            });

            if client.cfg.server.is_tls() {
                let builder = builder.connector(build_tls_connector(&client.cfg)?);
                ntex::rt::spawn(client.clone().start(MqttConnector::Tls(builder)));
            } else {
                ntex::rt::spawn(client.clone().start(MqttConnector::Tcp(builder)));
            }
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
                                log::warn!("{:?}", e);
                            }
                        } else if let Err(e) = sink.clone().publish_pkt(p).send_at_least_once().await {
                            log::warn!("{:?}", e);
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

    async fn start(self, builder: MqttConnector) {
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
                    log::warn!(
                        "{} Connect to {:?} fail, {:?}",
                        client.client_id,
                        client.cfg.server,
                        e.to_string()
                    );
                }
            }
            if client.is_closed() {
                break;
            } else {
                ntex::time::sleep(sleep_interval).await;
            }
        }
        log::info!("{} Exit 'rmqtt-bridge-egress-mqtt' client", client.client_id);
    }

    async fn ev_loop(self, c: v5::client::Client) {
        if let Err(e) = c
            .start(fn_service(move |control: v5::client::Control<()>| match control {
                v5::client::Control::Publish(publish) => {
                    log::debug!("{} publish: {:?}", self.client_id, publish);
                    let reason_code = v5::codec::PublishAckReason::Success;
                    Ready::Ok(publish.ack(reason_code))
                }
                v5::client::Control::Disconnect(msg) => {
                    log::info!("{} Server disconnecting: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v5::client::Control::Error(msg) => {
                    log::info!("{} Codec error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack(v5::codec::DisconnectReasonCode::NormalDisconnection))
                }
                v5::client::Control::ProtocolError(msg) => {
                    log::info!("{} Protocol error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v5::client::Control::PeerGone(msg) => {
                    log::info!("{} Peer closed connection: {:?}", self.client_id, msg.error());
                    Ready::Ok(msg.ack())
                }
                v5::client::Control::Closed(msg) => {
                    log::info!("{} Server closed connection", self.client_id);
                    Ready::Ok(msg.ack())
                }
            }))
            .await
        {
            log::error!("Start ev_loop error! {:?}", e);
        }
    }
}
