#[macro_use]
extern crate serde;

mod config;

use rmqtt::async_trait::async_trait;
use rmqtt::{serde_json, tokio};
use std::sync::Arc;
use tokio::sync::RwLock;

use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::{AuthResult, PublishAclResult, SubscribeAclResult},
    plugin::Plugin,
    Result, Runtime,
};

use config::{Access, Control, PluginConfig, PH_C, PH_U};

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    runtime: &'static Runtime,
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(Box::new(AclPlugin::new(runtime, name.into(), descr.into()).await?), default_startup)
        .await?;
    Ok(())
}

struct AclPlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl AclPlugin {
    #[inline]
    async fn new(runtime: &'static Runtime, name: String, descr: String) -> Result<Self> {
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} AclPlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, name, descr, register, cfg })
    }
}

#[async_trait]
impl Plugin for AclPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        let cfg = &self.cfg;
        self.register.add(Type::ClientConnected, Box::new(AclHandler::new(cfg))).await;
        self.register.add(Type::ClientAuthenticate, Box::new(AclHandler::new(cfg))).await;
        self.register.add(Type::ClientSubscribeCheckAcl, Box::new(AclHandler::new(cfg))).await;
        self.register.add(Type::MessagePublishCheckAcl, Box::new(AclHandler::new(cfg))).await;
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
        self.register.stop().await;
        Ok(true)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.1"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(&self.name)?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }
}

struct AclHandler {
    cfg: Arc<RwLock<PluginConfig>>,
    // cache_map: CacheMap,
}

impl AclHandler {
    fn new(cfg: &Arc<RwLock<PluginConfig>>) -> Self {
        Self { cfg: cfg.clone() }
    }
}

#[async_trait]
impl Handler for AclHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnected(_session, client) => {
                let cfg = self.cfg.clone();
                let client_id = client.id.client_id.clone();
                let username = client.connect_info.username().cloned();
                let build_placeholders = async move {
                    for rule in cfg.read().await.rules() {
                        for ph_tf in &rule.topics.placeholders {
                            let mut tf = ph_tf.replace(PH_C, &client_id);
                            if let Some(un) = &username {
                                tf = tf.replace(PH_U, un);
                            } else {
                                tf = tf.replace(PH_U, "");
                            }
                            if let Err(e) = rule.add_topic_filter(&tf).await {
                                log::error!(
                                    "acl config error, build_placeholders, add topic filter error, {:?}",
                                    e
                                );
                            }
                        }

                        for eq_ph_t in &rule.topics.eq_placeholders {
                            let mut t = eq_ph_t.replace(PH_C, &client_id);
                            if let Some(un) = &username {
                                t = t.replace(PH_U, un);
                            } else {
                                t = t.replace(PH_U, "");
                            }
                            rule.add_topic_to_eqs(t);
                        }

                        log::debug!("rule.access: {:?}", rule.access);
                        log::debug!("rule.user: {:?}", rule.user);
                        log::debug!("rule.control: {:?}", rule.control);
                        log::debug!("rule.topics.eqs: {:?}", rule.topics.eqs);
                        log::debug!("rule.topics.tree: {:?}", rule.topics.tree.read().await.list(100));
                    }
                };
                tokio::spawn(build_placeholders);
            }

            Parameter::ClientAuthenticate(_session, client_info, _password) => {
                if let Some(HookResult::AuthResult(_)) = acc {
                    return (false, acc);
                }

                for rule in self.cfg.read().await.rules() {
                    if !matches!(rule.control, Control::Connect | Control::All) {
                        continue;
                    }
                    if rule.user.hit(client_info) {
                        log::debug!("{:?} ClientAuthenticate, rule: {:?}", client_info.id, rule);
                        return if matches!(rule.access, Access::Allow) {
                            (true, acc)
                        } else {
                            (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)))
                        };
                    }
                }
                return (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)));
            }

            Parameter::ClientSubscribeCheckAcl(_session, client_info, subscribe) => {
                if let Some(HookResult::SubscribeAclResult(SubscribeAclResult::Failure)) = &acc {
                    return (false, acc);
                }
                let topic_filter_str = subscribe.topic_filter.to_string();
                for (idx, rule) in self.cfg.read().await.rules().iter().enumerate() {
                    if !matches!(rule.control, Control::Subscribe | Control::Pubsub | Control::All) {
                        continue;
                    }
                    if !rule.user.hit(client_info) {
                        continue;
                    }
                    if !rule.topics.is_match(subscribe.topic_filter, &topic_filter_str).await {
                        continue;
                    }
                    log::debug!(
                        "{:?} ClientSubscribeCheckAcl, {}, is_match ok: topic_filter: {}",
                        client_info.id,
                        idx,
                        topic_filter_str
                    );
                    return if matches!(rule.access, Access::Allow) {
                        (
                            true,
                            Some(HookResult::SubscribeAclResult(SubscribeAclResult::Success(subscribe.qos))),
                        )
                    } else {
                        (false, Some(HookResult::SubscribeAclResult(SubscribeAclResult::Failure)))
                    };
                }
                return (false, Some(HookResult::SubscribeAclResult(SubscribeAclResult::Failure)));
            }

            Parameter::MessagePublishCheckAcl(_session, client_info, publish) => {
                if let Some(HookResult::PublishAclResult(PublishAclResult::Rejected(_))) = &acc {
                    return (false, acc);
                }
                let topic = publish.topic();
                let topic_str = topic.to_string();
                for (idx, rule) in self.cfg.read().await.rules().iter().enumerate() {
                    if !matches!(rule.control, Control::Publish | Control::Pubsub | Control::All) {
                        continue;
                    }
                    if !rule.user.hit(client_info) {
                        continue;
                    }
                    if !rule.topics.is_match(topic, &topic_str).await {
                        continue;
                    }
                    log::debug!(
                        "{:?} MessagePublishCheckAcl, {}, is_match ok: topic_str: {}",
                        client_info.id,
                        idx,
                        topic_str
                    );
                    return if matches!(rule.access, Access::Allow) {
                        (true, Some(HookResult::PublishAclResult(PublishAclResult::Allow)))
                    } else {
                        (false, Some(HookResult::PublishAclResult(PublishAclResult::Rejected(true))))
                    };
                }
                return (false, Some(HookResult::PublishAclResult(PublishAclResult::Rejected(true))));
            }
            _ => {
                log::error!("parameter is: {:?}", param);
            }
        }
        (true, acc)
    }
}
