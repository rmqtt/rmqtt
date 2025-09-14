#![deny(unsafe_code)]

use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    types::TopicName,
    Result,
};

use crate::config::Mode;
use config::PluginConfig;
use rmqtt::types::ClientId;

mod config;

register!(P2PMessagingPlugin::new);

#[derive(Plugin)]
struct P2PMessagingPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl P2PMessagingPlugin {
    #[inline]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name)?;
        let cfg = Arc::new(RwLock::new(cfg));
        log::info!("{} P2PMessagingPlugin cfg: {:?}", name, cfg.read().await);
        let register = scx.extends.hook_mgr().register();
        Ok(Self { scx, register, cfg })
    }
}

#[async_trait]
impl Plugin for P2PMessagingPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;
        self.register.add(Type::MessagePublish, Box::new(P2PMessagingHandler::new(cfg))).await;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        Ok(false)
    }
}

#[derive(Debug)]
struct P2PMatch {
    clientid: String,
    topic: String,
}

struct P2PMessagingHandler {
    cfg: Arc<RwLock<PluginConfig>>,
}

impl P2PMessagingHandler {
    fn new(cfg: &Arc<RwLock<PluginConfig>>) -> Self {
        Self { cfg: cfg.clone() }
    }

    #[inline]
    async fn parse_p2p_topic(&self, topic: &str) -> Result<Option<P2PMatch>> {
        let cfg = self.cfg.read().await;
        let mode = cfg.mode;
        let parts: Vec<&str> = topic.split('/').collect();

        let p2p_match = match mode {
            Mode::Prefix | Mode::Both if parts.len() >= 3 && parts[0] == "p2p" => {
                Some(P2PMatch { clientid: parts[1].to_string(), topic: parts[2..].join("/") })
            }
            Mode::Suffix | Mode::Both if parts.len() >= 3 && parts[parts.len() - 2] == "p2p" => {
                let clientid = parts
                    .last()
                    .ok_or_else(|| anyhow!("Invalid topic format, clientid not found: {topic:?}"))?;
                Some(P2PMatch { clientid: clientid.to_string(), topic: parts[..parts.len() - 2].join("/") })
            }
            _ => None,
        };

        if let Some(ref m) = p2p_match {
            if m.clientid.is_empty() {
                return Err(anyhow!("Invalid topic format, clientid not found: {topic:?}"));
            }
            if m.topic.is_empty() {
                return Err(anyhow!("Invalid topic format, sub topic not found: {topic:?}"));
            }
        }

        Ok(p2p_match)
    }
}

#[async_trait]
impl Handler for P2PMessagingHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::MessagePublish(s, _f, p) => {
                log::debug!("{:?} MessagePublish ..", s.map(|s| &s.id));

                match self.parse_p2p_topic(&p.topic).await {
                    Ok(Some(p2p_match)) => {
                        log::debug!("{p2p_match:?}");
                        let mut p1 = (*p).clone();
                        p1.topic = TopicName::from(p2p_match.topic);
                        p1.target_clientid = Some(ClientId::from(p2p_match.clientid));
                        return (true, Some(HookResult::Publish(p1)));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!("{e:?}");
                    }
                }
            }
            _ => {
                log::error!("parameter is: {param:?}");
            }
        }
        (true, acc)
    }
}
