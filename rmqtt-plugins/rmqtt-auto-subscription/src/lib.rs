#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::sync::Arc;

use rmqtt::{
    async_trait::async_trait, log, once_cell::sync::OnceCell, serde_json, tokio::sync::RwLock, TopicFilter,
};
use rmqtt::{
    broker::{default::DefaultAutoSubscription, AutoSubscription},
    plugin::{PackageInfo, Plugin},
    register, Id, Message, Result, Runtime, Tx,
};

use config::PluginConfig;
use rmqtt::tokio::sync::oneshot;

mod config;

register!(AutoSubscriptionPlugin::new);

#[derive(Plugin)]
struct AutoSubscriptionPlugin {
    runtime: &'static Runtime,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl AutoSubscriptionPlugin {
    #[inline]
    async fn new<N: Into<String>>(runtime: &'static Runtime, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config::<PluginConfig>(&name)?;
        let cfg = Arc::new(RwLock::new(cfg));
        log::info!("{} AutoSubscriptionPlugin cfg: {:?}", name, cfg.read().await);
        Ok(Self { runtime, cfg })
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
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        *self.runtime.extends.auto_subscription_mut().await =
            Box::new(XAutoSubscription::get_or_init(self.cfg.clone()));
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        *self.runtime.extends.auto_subscription_mut().await = Box::new(DefaultAutoSubscription::instance());
        Ok(false)
    }
}

pub struct XAutoSubscription {
    cfg: Arc<RwLock<PluginConfig>>,
}

impl XAutoSubscription {
    #[inline]
    pub(crate) fn get_or_init(cfg: Arc<RwLock<PluginConfig>>) -> &'static XAutoSubscription {
        static INSTANCE: OnceCell<XAutoSubscription> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { cfg })
    }
}

#[async_trait]
impl AutoSubscription for &'static XAutoSubscription {
    #[inline]
    fn enable(&self) -> bool {
        true
    }

    #[inline]
    async fn subscribe(&self, id: &Id, msg_tx: &Tx) -> Result<()> {
        for item in self.cfg.read().await.subscribes.iter() {
            let (tx, rx) = oneshot::channel();
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
            if let Err(e) = msg_tx.unbounded_send(Message::Subscribe(sub, tx)) {
                log::error!("{} auto subscribe error, {:?}", id, e);
            }
            match rx.await {
                Ok(Ok(ret)) => {
                    if ret.failure() {
                        log::error!("{} auto subscribe failed, {:?}", id, ret.ack_reason);
                    }
                }
                Ok(Err(e)) => {
                    log::error!("{} auto subscribe error, {:?}", id, e);
                }
                Err(e) => {
                    log::error!("{} auto subscribe error, {:?}", id, e);
                }
            }
        }
        Ok(())
    }
}
