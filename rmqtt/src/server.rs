use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use itertools::Itertools;

use crate::context::ServerContext;
use crate::net::MqttStream;
use crate::net::{Listener, ListenerType, Result};
use crate::{v3, v5};

pub struct MqttServerBuilder {
    scx: ServerContext,
    listeners: Vec<Listener>,
}

impl MqttServerBuilder {
    fn new(scx: ServerContext) -> Self {
        Self { scx, listeners: Vec::default() }
    }

    pub fn listener(mut self, listen: Listener) -> Self {
        self.listeners.push(listen);
        self
    }

    pub fn listeners<I: IntoIterator<Item = Listener>>(mut self, listens: I) -> Self {
        self.listeners.extend(listens);
        self
    }

    pub fn build(self) -> MqttServer {
        MqttServer { inner: Arc::new(MqttServerInner { scx: self.scx, listeners: self.listeners }) }
    }
}

#[derive(Clone)]
pub struct MqttServer {
    inner: Arc<MqttServerInner>,
}

pub struct MqttServerInner {
    scx: ServerContext,
    listeners: Vec<Listener>,
}

impl Deref for MqttServer {
    type Target = MqttServerInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl MqttServer {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(scx: ServerContext) -> MqttServerBuilder {
        MqttServerBuilder::new(scx)
    }

    pub fn start(self) {
        tokio::spawn(async move {
            if let Err(e) = self.run().await {
                log::error!("Failed to start the MQTT server! {}", e);
                std::process::exit(1);
            }
        });
    }

    pub async fn run(self) -> Result<()> {
        futures::future::join_all(
            self.listeners
                .iter()
                .map(|l| match l.typ {
                    ListenerType::TCP => listen_tcp(self.scx.clone(), l).boxed(),
                    ListenerType::TLS => listen_tls(self.scx.clone(), l).boxed(),
                    ListenerType::WS => listen_ws(self.scx.clone(), l).boxed(),
                    ListenerType::WSS => listen_wss(self.scx.clone(), l).boxed(),
                })
                .collect_vec(),
        )
        .await;
        Ok(())
    }
}

async fn listen_tcp(scx: ServerContext, l: &Listener) {
    loop {
        match l.accept().await {
            Ok(a) => {
                let scx = scx.clone();
                tokio::spawn(async move {
                    log::info!("tcp listen addr:{:?}, remote addr:{:?}", a.cfg.laddr, a.remote_addr);
                    let d = match a.tcp() {
                        Ok(d) => d,
                        Err(e) => {
                            log::warn!("Failed to mqtt(tcp) accept, {:?}", e);
                            return;
                        }
                    };

                    match d.mqtt().await {
                        Ok(MqttStream::V3(s)) => {
                            if let Err(e) = v3::process(scx, s).await {
                                log::warn!("Failed to process mqtt v3, {:?}", e);
                            }
                        }
                        Ok(MqttStream::V5(s)) => {
                            if let Err(e) = v5::process(scx, s).await {
                                log::warn!("Failed to process mqtt v5, {:?}", e);
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to probe MQTT version, {:?}", e);
                        }
                    }
                });
            }
            Err(e) => {
                log::warn!("Failed to accept TCP socket connection, {:?}", e);
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
}

async fn listen_tls(scx: ServerContext, l: &Listener) {
    loop {
        match l.accept().await {
            Ok(a) => {
                let scx = scx.clone();
                tokio::spawn(async move {
                    log::info!("tls listen addr:{:?}, remote addr:{:?}", a.cfg.laddr, a.remote_addr);
                    let d = match a.tls().await {
                        Ok(d) => d,
                        Err(e) => {
                            log::warn!("Failed to mqtt(tls) accept, {:?}", e);
                            return;
                        }
                    };
                    match d.mqtt().await {
                        Ok(MqttStream::V3(s)) => {
                            if let Err(e) = v3::process(scx, s).await {
                                log::warn!("Failed to process mqtt(tls) v3, {:?}", e);
                            }
                        }
                        Ok(MqttStream::V5(s)) => {
                            if let Err(e) = v5::process(scx, s).await {
                                log::warn!("Failed to process mqtt(tls) v5, {:?}", e);
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to probe MQTT(TLS) version, {:?}", e);
                        }
                    }
                });
            }
            Err(e) => {
                log::warn!("Failed to accept TLS socket connection, {:?}", e);
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
}

async fn listen_ws(scx: ServerContext, l: &Listener) {
    loop {
        match l.accept().await {
            Ok(a) => {
                let scx = scx.clone();
                tokio::spawn(async move {
                    log::info!("ws listen addr:{:?}, remote addr:{:?}", a.cfg.laddr, a.remote_addr);
                    let d = match a.ws().await {
                        Ok(d) => d,
                        Err(e) => {
                            log::warn!("Failed to websocket accept, {:?}", e);
                            return;
                        }
                    };
                    match d.mqtt().await {
                        Ok(MqttStream::V3(s)) => {
                            if let Err(e) = v3::process(scx, s).await {
                                log::warn!("Failed to process websocket mqtt v3, {:?}", e);
                            }
                        }
                        Ok(MqttStream::V5(s)) => {
                            if let Err(e) = v5::process(scx, s).await {
                                log::warn!("Failed to process websocket mqtt v5, {:?}", e);
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to websocket probe MQTT version, {:?}", e);
                        }
                    }
                });
            }
            Err(e) => {
                log::warn!("Failed to websocket accept TCP socket connection, {:?}", e);
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
}

async fn listen_wss(scx: ServerContext, l: &Listener) {
    loop {
        match l.accept().await {
            Ok(a) => {
                let scx = scx.clone();
                tokio::spawn(async move {
                    log::info!("wss listen addr:{:?}, remote addr:{:?}", a.cfg.laddr, a.remote_addr);
                    let d = match a.wss().await {
                        Ok(d) => d,
                        Err(e) => {
                            log::warn!("Failed to websocket mqtt(tls) accept, {:?}", e);
                            return;
                        }
                    };
                    match d.mqtt().await {
                        Ok(MqttStream::V3(s)) => {
                            if let Err(e) = v3::process(scx, s).await {
                                log::warn!("Failed to process websocket mqtt(tls) v3, {:?}", e);
                            }
                        }
                        Ok(MqttStream::V5(s)) => {
                            if let Err(e) = v5::process(scx, s).await {
                                log::warn!("Failed to process websocket mqtt(tls) v5, {:?}", e);
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to websocket probe MQTT(TLS) version, {:?}", e);
                        }
                    }
                });
            }
            Err(e) => {
                log::warn!("Failed to websocket accept TLS socket connection, {:?}", e);
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }
}
