#![deny(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    subscribe::{AutoSubscription, DefaultAutoSubscription},
    Id, Result, Subscribe, TopicFilter,
};

use config::PluginConfig;

mod config;

register!(AutoSubscriptionPlugin::new);

#[derive(Plugin)]
struct AutoSubscriptionPlugin {
    scx: ServerContext,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl AutoSubscriptionPlugin {
    #[inline]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = scx.plugins.read_config::<PluginConfig>(&name)?;
        let cfg = Arc::new(RwLock::new(cfg));
        log::info!("{} AutoSubscriptionPlugin cfg: {:?}", name, cfg.read().await);
        Ok(Self { scx, cfg })
    }
}

#[async_trait]
impl Plugin for AutoSubscriptionPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
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
        *self.scx.extends.auto_subscription_mut().await = Box::new(XAutoSubscription::new(self.cfg.clone()));
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        *self.scx.extends.auto_subscription_mut().await = Box::new(DefaultAutoSubscription);
        Ok(false)
    }
}

pub struct XAutoSubscription {
    cfg: Arc<RwLock<PluginConfig>>,
}

impl XAutoSubscription {
    #[inline]
    pub(crate) fn new(cfg: Arc<RwLock<PluginConfig>>) -> XAutoSubscription {
        Self { cfg }
    }
}

#[async_trait]
impl AutoSubscription for XAutoSubscription {
    #[inline]
    fn enable(&self) -> bool {
        true
    }

    #[inline]
    async fn subscribes(&self, id: &Id) -> Result<Vec<Subscribe>> {
        let mut subs = Vec::new();
        for item in self.cfg.read().await.subscribes.iter() {
            let mut sub = item.sub.clone();
            if item.has_clientid_placeholder {
                sub.topic_filter = TopicFilter::from(sub.topic_filter.replace("${clientid}", &id.client_id));
            }
            if item.has_username_placeholder {
                if let Some(username) = &id.username {
                    sub.topic_filter = TopicFilter::from(sub.topic_filter.replace("${username}", username));
                } else {
                    log::warn!("{} auto subscribe failed, username is not exist", id);
                    continue;
                }
            }
            subs.push(sub);
        }
        Ok(subs)
    }
}
