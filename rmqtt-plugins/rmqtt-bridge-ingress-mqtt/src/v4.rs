use std::cell::RefCell;
use std::net::{SocketAddr, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ntex::connect::rustls::TlsConnector;
use ntex::connect::Connector;
use ntex::service::fn_service;
use ntex::time;
use ntex::time::Seconds;
use ntex::util::ByteString;
use ntex::util::Bytes;
use ntex::util::Ready;
use ntex_mqtt::error::SendPacketError;
use ntex_mqtt::v3::codec::SubscribeReturnCode;
use ntex_mqtt::{self, v3};

use rmqtt::ntex_mqtt::types::MQTT_LEVEL_311;

use rmqtt::futures::channel::mpsc;
use rmqtt::futures::StreamExt;
use rmqtt::log;
use rmqtt::{ClientId, MqttError, NodeId, Result, UserName};

use crate::bridge::{
    build_tls_connector, BridgeClient, BridgePublish, Command, CommandMailbox, OnMessageEvent,
};
use crate::config::Bridge;

enum MqttConnector {
    Tcp(v3::client::MqttConnector<String, Connector<String>>),
    Tls(v3::client::MqttConnector<String, TlsConnector<String>>),
}

impl MqttConnector {
    async fn connect(&self) -> Result<v3::client::Client> {
        Ok(match self {
            MqttConnector::Tcp(c) => c.connect().await.map_err(|e| MqttError::Anyhow(e.into()))?,
            MqttConnector::Tls(c) => c.connect().await.map_err(|e| MqttError::Anyhow(e.into()))?,
        })
    }
}

#[derive(Clone)]
pub struct Client {
    pub(crate) cfg: Rc<Bridge>,
    pub(crate) server_addr: SocketAddr,
    pub(crate) entry_idx: usize,
    pub(crate) client_id: ClientId,
    pub(crate) username: UserName,
    closed: Rc<AtomicBool>,
    sink: Rc<RefCell<Option<v3::MqttSink>>>,
    on_message: OnMessageEvent,
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
        on_message: OnMessageEvent,
    ) -> Result<CommandMailbox> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(10);

        let client_id =
            format!("{}:{}:ingress:{}:{}:{}", cfg.client_id_prefix, cfg.name, node_id, entry_idx, client_no);
        let username = cfg.username.clone().unwrap_or("undefined".into());

        let server_addr = cfg.server.addr.to_socket_addrs()?.next().ok_or_else(|| MqttError::from("None"))?;

        let client = Self {
            cfg: Rc::new(cfg),
            server_addr,
            entry_idx,
            client_id: ClientId::from(client_id),
            username: UserName::from(username),
            closed: Rc::new(AtomicBool::new(false)),
            sink: Rc::new(RefCell::new(None)),
            on_message,
        };

        log::info!(
            "server_addr: {}, server_addr: {}, cfg.server: {:?}",
            client.server_addr,
            server_addr,
            &client.cfg.server
        );

        let mut builder = v3::client::MqttConnector::new(client.cfg.server.addr.clone())
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

        Ok(CommandMailbox::new(client.client_id, cmd_tx))
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

                    //subscribes
                    if let Some(entry) = client.cfg.entries.get(client.entry_idx) {
                        let topic_filter = ByteString::from(entry.remote.topic.clone());
                        let qos = entry.remote.qos;
                        ntex::rt::spawn(client.clone().subscribe(
                            sink.clone(),
                            topic_filter,
                            qos,
                            ByteString::from(client.client_id.as_ref()),
                        ));
                    }

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
        log::info!("{} Exit 'rmqtt-bridge-ingress-mqtt' client", client.client_id);
    }

    async fn subscribe(
        self,
        sink: v3::MqttSink,
        topic_filter: ByteString,
        qos: v3::QoS,
        client_id: ByteString,
    ) {
        'subscribe: loop {
            match sink.subscribe().topic_filter(topic_filter.clone(), qos).send().await {
                Ok(rets) => {
                    for ret in rets {
                        if let SubscribeReturnCode::Failure = ret {
                            log::info!("{} Subscribe failure, topic_filter: {:?}", client_id, topic_filter);
                            time::sleep(Duration::from_secs(5)).await;
                            continue 'subscribe;
                        } else {
                            log::info!("{} Successfully subscribed to {:?}", client_id, topic_filter,);
                        }
                    }
                    break;
                }
                Err(SendPacketError::Disconnected) => {
                    log::info!("{} Subscribe error, Disconnected", client_id);
                    break;
                }
                Err(e) => {
                    log::info!("{} Subscribe error, {:?}", client_id, e);
                    break;
                }
            }
        }
    }

    async fn ev_loop(self, c: v3::client::Client) {
        if let Err(e) = c
            .start(fn_service(move |control: v3::client::Control<()>| match control {
                v3::client::Control::Publish(publish) => {
                    log::debug!("{} publish: {:?}", self.client_id, publish);
                    self.on_message.fire((
                        BridgeClient::V4(self.clone()),
                        Some(self.server_addr),
                        None,
                        BridgePublish::V3(publish.packet().clone()),
                    ));
                    Ready::Ok(publish.ack())
                }
                v3::client::Control::Error(msg) => {
                    log::info!("{} Codec error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v3::client::Control::ProtocolError(msg) => {
                    log::info!("{} Protocol error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v3::client::Control::PeerGone(msg) => {
                    log::info!("{} Peer closed connection: {:?}", self.client_id, msg.err());
                    Ready::Ok(msg.ack())
                }
                v3::client::Control::Closed(msg) => {
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
