#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::sync::Arc;

use rmqtt::{async_trait::async_trait, log, serde_json, tokio::sync::RwLock};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    plugin::{PackageInfo, Plugin},
    register, Publish, Result, Runtime, Session, TopicFilter,
};

use config::{Action, PluginConfig};

mod config;

register!(TopicRewritePlugin::new);

#[derive(Plugin)]
struct TopicRewritePlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl TopicRewritePlugin {
    #[inline]
    async fn new<N: Into<String>>(runtime: &'static Runtime, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config::<PluginConfig>(&name)?;
        let cfg = Arc::new(RwLock::new(cfg));
        log::info!("{} TopicRewritePlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, register, cfg })
    }
}

#[async_trait]
impl Plugin for TopicRewritePlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;
        self.register.add(Type::MessagePublish, Box::new(TopicRewriteHandler::new(cfg))).await;
        self.register.add(Type::ClientSubscribe, Box::new(TopicRewriteHandler::new(cfg))).await;
        self.register.add(Type::ClientUnsubscribe, Box::new(TopicRewriteHandler::new(cfg))).await;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(self.name())?;
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

struct TopicRewriteHandler {
    cfg: Arc<RwLock<PluginConfig>>,
}

impl TopicRewriteHandler {
    fn new(cfg: &Arc<RwLock<PluginConfig>>) -> Self {
        Self { cfg: cfg.clone() }
    }

    #[inline]
    pub async fn rewrite_publish_topic(&self, s: Option<&Session>, p: &Publish) -> Result<Option<Publish>> {
        match self.cfg.read().await.rewrite_topic(Action::Publish, s, &p.topic).await? {
            Some(topic) => {
                log::debug!("new_topic: {}", topic);
                let new_p = Publish {
                    dup: p.dup,
                    retain: p.retain,
                    qos: p.qos,
                    topic,
                    packet_id: p.packet_id,
                    payload: p.payload.clone(),
                    properties: p.properties.clone(),
                    create_time: p.create_time,
                };
                Ok(Some(new_p))
            }
            None => Ok(None),
        }
    }

    #[inline]
    pub async fn rewrite_subscribe_topic(
        &self,
        s: Option<&Session>,
        topic_filter: &str,
    ) -> Result<Option<TopicFilter>> {
        match self.cfg.read().await.rewrite_topic(Action::Subscribe, s, topic_filter).await? {
            Some(new_tf) => {
                log::debug!("new_tf: {}", new_tf);
                Ok(Some(new_tf))
            }
            None => Ok(None),
        }
    }
}

#[async_trait]
impl Handler for TopicRewriteHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::MessagePublish(s, _f, p) => {
                log::debug!("{:?} MessagePublish ..", s.map(|s| &s.id));

                match self.rewrite_publish_topic(s.as_ref().map(|s| *s), p).await {
                    Err(e) => {
                        log::error!("{:?} topic format error, {:?}", s.map(|s| &s.id), e);
                        return (true, acc);
                    }
                    Ok(Some(p)) => {
                        return (true, Some(HookResult::Publish(p)));
                    }
                    Ok(None) => {}
                }
            }
            Parameter::ClientSubscribe(s, sub) => {
                match self.rewrite_subscribe_topic(Some(*s), &sub.topic_filter).await {
                    Err(e) => {
                        log::error!("{} topic_filter format error, {:?}", s.id, e);
                        return (true, acc);
                    }
                    Ok(Some(tf)) => {
                        return (true, Some(HookResult::TopicFilter(Some(tf))));
                    }
                    Ok(None) => {}
                }
            }
            Parameter::ClientUnsubscribe(s, unsub) => {
                match self.rewrite_subscribe_topic(Some(*s), &unsub.topic_filter).await {
                    Err(e) => {
                        log::error!("{} topic_filter format error, {:?}", s.id, e);
                        return (true, acc);
                    }
                    Ok(Some(tf)) => {
                        return (true, Some(HookResult::TopicFilter(Some(tf))));
                    }
                    Ok(None) => {}
                }
            }
            _ => {
                log::error!("parameter is: {:?}", param);
            }
        }
        (true, acc)
    }
}
