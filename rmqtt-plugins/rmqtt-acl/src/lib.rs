#[macro_use]
extern crate serde;

use std::str::FromStr;
use std::sync::Arc;

use config::{Access, Control, PH_C, PH_U, PluginConfig};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::{AuthResult, PublishAclResult, SubscribeAckReason, SubscribeAclResult, Topic},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};
use rmqtt::{async_trait::async_trait, log, serde_json, tokio::{self, sync::RwLock}};

mod config;

#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    descr: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(async move {
                AclPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
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
    async fn new<N: Into<String>, D: Into<String>>(
        runtime: &'static Runtime,
        name: N,
        descr: D,
    ) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} AclPlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, name, descr: descr.into(), register, cfg })
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
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(&self.name)?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, the default ACL plug-in, it cannot be stopped", self.name);
        //self.register.stop().await;
        Ok(false)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.1"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }
}

struct AclHandler {
    cfg: Arc<RwLock<PluginConfig>>,
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

            Parameter::ClientAuthenticate(connect_info) => {
                log::debug!("ClientAuthenticate acl");
                if matches!(
                    acc,
                    Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword))
                        | Some(HookResult::AuthResult(AuthResult::NotAuthorized))
                ) {
                    return (false, acc);
                }

                for rule in self.cfg.read().await.rules() {
                    if !matches!(rule.control, Control::Connect | Control::All) {
                        continue;
                    }
                    if rule.user.hit(connect_info) {
                        log::debug!("{:?} ClientAuthenticate, rule: {:?}", connect_info.id(), rule);
                        return if matches!(rule.access, Access::Allow) {
                            (true, Some(HookResult::AuthResult(AuthResult::Allow)))
                        } else {
                            (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)))
                        };
                    }
                }
                return (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)));
            }

            Parameter::ClientSubscribeCheckAcl(_session, client_info, subscribe) => {
                if let Some(HookResult::SubscribeAclResult(acl_result)) = &acc {
                    if acl_result.failure() {
                        return (false, acc);
                    }
                }
                let topic =
                    Topic::from_str(&subscribe.topic_filter).unwrap_or_else(|_| Topic::from(Vec::new()));
                let topic_filter = &subscribe.topic_filter;
                for (idx, rule) in self.cfg.read().await.rules().iter().enumerate() {
                    if !matches!(rule.control, Control::Subscribe | Control::Pubsub | Control::All) {
                        continue;
                    }
                    if !rule.user.hit(&client_info.connect_info) {
                        continue;
                    }
                    if !rule.topics.is_match(&topic, topic_filter).await {
                        continue;
                    }
                    log::debug!(
                        "{:?} ClientSubscribeCheckAcl, {}, is_match ok: topic_filter: {}",
                        client_info.id,
                        idx,
                        topic_filter
                    );
                    return if matches!(rule.access, Access::Allow) {
                        (
                            true,
                            Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_success(
                                subscribe.qos,
                            ))),
                        )
                    } else {
                        (
                            false,
                            Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                                SubscribeAckReason::NotAuthorized,
                            ))),
                        )
                    };
                }
                return (
                    false,
                    Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                        SubscribeAckReason::NotAuthorized,
                    ))),
                );
            }

            Parameter::MessagePublishCheckAcl(_session, client_info, publish) => {
                if let Some(HookResult::PublishAclResult(PublishAclResult::Rejected(_))) = &acc {
                    return (false, acc);
                }
                let topic_str = publish.topic();
                let topic = Topic::from_str(topic_str).unwrap_or_else(|_| Topic::from(Vec::new()));
                for (idx, rule) in self.cfg.read().await.rules().iter().enumerate() {
                    if !matches!(rule.control, Control::Publish | Control::Pubsub | Control::All) {
                        continue;
                    }
                    if !rule.user.hit(&client_info.connect_info) {
                        continue;
                    }
                    if !rule.topics.is_match(&topic, topic_str).await {
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
