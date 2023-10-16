#![deny(unsafe_code)]

use rmqtt::serde_json::Number;
use rmqtt::{async_trait::async_trait, bytes::Bytes, log, serde_json};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Publish, Result, Runtime,
};

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
                TimeInjectorPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct TimeInjectorPlugin {
    name: String,
    descr: String,
    register: Box<dyn Register>,
}

impl TimeInjectorPlugin {
    #[inline]
    async fn new<N: Into<String>, D: Into<String>>(
        runtime: &'static Runtime,
        name: N,
        descr: D,
    ) -> Result<Self> {
        let name = name.into();
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { name, descr: descr.into(), register })
    }
}

#[async_trait]
impl Plugin for TimeInjectorPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        self.register.add(Type::MessagePublish, Box::new(PublishHandler {})).await;
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
        Ok(false)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.0"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }
}

struct PublishHandler;

#[async_trait]
impl Handler for PublishHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::MessagePublish(_s, _c, _from, p) => {
                log::debug!("MessagePublish, publish: {:?}", p);
                match serde_json::from_slice::<serde_json::Value>(p.payload()) {
                    Err(e) => {
                        log::error!("MessagePublish, publish payload format error, must be JSON, {:?}", e);
                    }
                    Ok(mut payload) => {
                        if let Some(payload) = payload.as_object_mut() {
                            payload
                                .insert("rts".into(), serde_json::Value::Number(Number::from(p.create_time)));
                        }
                        match serde_json::to_string(&payload) {
                            Err(e) => {
                                log::error!("MessagePublish, publish payload format error, {:?}", e);
                            }
                            Ok(payload) => {
                                let new_p = Publish {
                                    dup: p.dup,
                                    retain: p.retain,
                                    qos: p.qos,
                                    topic: p.topic.clone(),
                                    packet_id: p.packet_id,
                                    payload: Bytes::from(payload),

                                    properties: p.properties.clone(),
                                    create_time: p.create_time,
                                };
                                return (true, Some(HookResult::Publish(new_p)));
                            }
                        }
                    }
                }
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}
