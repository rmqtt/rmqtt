#![deny(unsafe_code)]

use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::{self, sync::RwLock};

use rmqtt::{
    codec::v5::SubscribeAckReason,
    context::ServerContext,
    hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    types::{AuthResult, PublishAclResult, SubscribeAclResult, Topic},
    Result,
};

use config::{Access, Control, PluginConfig, PH_C, PH_U};

mod config;

register!(AclPlugin::new);

#[derive(Plugin)]
struct AclPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl AclPlugin {
    #[inline]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(scx.plugins.read_config::<PluginConfig>(&name)?));
        log::debug!("{} AclPlugin cfg: {:?}", name, cfg.read().await);
        let register = scx.extends.hook_mgr().register();
        Ok(Self { scx, register, cfg })
    }
}

#[async_trait]
impl Plugin for AclPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;
        let priority = cfg.read().await.priority;
        self.register.add_priority(Type::ClientConnected, priority, Box::new(AclHandler::new(cfg))).await;
        self.register.add_priority(Type::ClientAuthenticate, priority, Box::new(AclHandler::new(cfg))).await;
        self.register
            .add_priority(Type::ClientSubscribeCheckAcl, priority, Box::new(AclHandler::new(cfg)))
            .await;
        self.register
            .add_priority(Type::MessagePublishCheckAcl, priority, Box::new(AclHandler::new(cfg)))
            .await;
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
        log::warn!("{} stop, the default ACL plug-in, it cannot be stopped", self.name());
        //self.register.stop().await;
        Ok(false)
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
            Parameter::ClientConnected(session) => {
                let cfg = self.cfg.clone();
                let client_id = session.id.client_id.clone();
                let username = session.id.username.clone();
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
                        log::debug!("rule.users: {:?}", rule.users);
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

                    let allow = matches!(rule.access, Access::Allow);
                    let (hit, superuser) = rule.hit(
                        connect_info.id(),
                        connect_info.password(),
                        Some(connect_info.proto_ver()),
                        allow,
                    );
                    if hit {
                        log::debug!("{:?} ClientAuthenticate, rule: {:?}", connect_info.id(), rule);
                        return if allow {
                            (false, Some(HookResult::AuthResult(AuthResult::Allow(superuser, None))))
                        } else {
                            (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)))
                        };
                    }
                }
                return (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)));
            }

            Parameter::ClientSubscribeCheckAcl(session, subscribe) => {
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

                    let allow = matches!(rule.access, Access::Allow);
                    let (hit, _) =
                        rule.hit(&session.id, session.password(), session.protocol().await.ok(), allow);
                    if !hit {
                        continue;
                    }
                    if !rule.topics.is_match(&topic, topic_filter).await {
                        continue;
                    }
                    log::debug!(
                        "{:?} ClientSubscribeCheckAcl, {}, is_match ok: topic_filter: {}",
                        session.id,
                        idx,
                        topic_filter
                    );
                    return if allow {
                        (
                            false,
                            Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_success(
                                subscribe.opts.qos(),
                                None,
                            ))),
                        )
                    } else {
                        (
                            false,
                            Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                                SubscribeAckReason::UnspecifiedError,
                            ))),
                        )
                    };
                }
                return (
                    false,
                    Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                        SubscribeAckReason::UnspecifiedError,
                    ))),
                );
            }

            Parameter::MessagePublishCheckAcl(session, publish) => {
                if let Some(HookResult::PublishAclResult(PublishAclResult::Rejected(_))) = &acc {
                    return (false, acc);
                }
                let topic_str = &publish.topic;
                let topic = Topic::from_str(topic_str).unwrap_or_else(|_| Topic::from(Vec::new()));
                let disconnect_if_pub_rejected = self.cfg.read().await.disconnect_if_pub_rejected;
                for (idx, rule) in self.cfg.read().await.rules().iter().enumerate() {
                    if !matches!(rule.control, Control::Publish | Control::Pubsub | Control::All) {
                        continue;
                    }

                    let allow = matches!(rule.access, Access::Allow);
                    let (hit, _) =
                        rule.hit(&session.id, session.password(), session.protocol().await.ok(), allow);
                    if !hit {
                        continue;
                    }
                    if !rule.topics.is_match(&topic, topic_str).await {
                        continue;
                    }
                    log::debug!(
                        "{:?} MessagePublishCheckAcl, {}, is_match ok: topic_str: {}",
                        session.id,
                        idx,
                        topic_str
                    );
                    return if allow {
                        (false, Some(HookResult::PublishAclResult(PublishAclResult::Allow)))
                    } else {
                        (
                            false,
                            Some(HookResult::PublishAclResult(PublishAclResult::Rejected(
                                disconnect_if_pub_rejected,
                            ))),
                        )
                    };
                }
                return (
                    false,
                    Some(HookResult::PublishAclResult(PublishAclResult::Rejected(
                        disconnect_if_pub_rejected,
                    ))),
                );
            }
            _ => {
                log::error!("parameter is: {:?}", param);
            }
        }
        (true, acc)
    }
}
