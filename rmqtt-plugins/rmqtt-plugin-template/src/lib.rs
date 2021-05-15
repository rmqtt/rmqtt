use async_trait::async_trait;

use rmqtt::broker::topic::Level;
use rmqtt::{
    broker::{
        hook::{self, Handler, HookResult, Parameter, Register, ReturnType},
        types::{From, Publish, Subscribe, SubscribeAclResult},
    },
    grpc::{client::NodeGrpcClient, Message},
    plugin::Plugin,
    Result, Runtime,
};

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    runtime: &'static Runtime,
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(Box::new(Template::new(runtime, name.into(), descr.into()).await), default_startup)
        .await?;
    Ok(())
}

struct Template {
    _runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
}

impl Template {
    #[inline]
    async fn new(runtime: &'static Runtime, name: String, descr: String) -> Self {
        let register = runtime.extends.hook_mgr().await.register();
        Self { _runtime: runtime, name, descr, register }
    }
}

#[async_trait]
impl Plugin for Template {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::debug!("{} init", self.name);
        self.register.add(hook::Type::ClientConnack, Box::new(HookHandler::new()?));
        self.register.add(hook::Type::MessageDelivered, Box::new(HookHandler::new()?));
        self.register.add(hook::Type::MessagePublish, Box::new(HookHandler::new()?));
        self.register.add(hook::Type::ClientSubscribeCheckAcl, Box::new(HookHandler::new()?));

        self.register.add(hook::Type::GrpcMessageReceived, Box::new(HookHandler::new()?));

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        self.register.start();
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
        self.register.stop();
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
}

struct HookHandler {
    client: NodeGrpcClient,
}

impl HookHandler {
    fn new() -> Result<Self> {
        let client = NodeGrpcClient::new(([127, 0, 0, 1], 5363).into())?;
        Ok(Self { client })
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&mut self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnack(connect_info, r) => {
                log::debug!("client connack, {:?}, {:?}", connect_info, r);
            }
            Parameter::MessagePublish(_session, c, publish) => {
                log::debug!("{:?} message publish, {:?}", c.id, publish);

                if publish.topic().to_string().contains("x/y/z") {
                    let p = match *publish {
                        Publish::V3(p) => {
                            let mut p = p.clone();
                            p.packet.topic =
                                bytestring::ByteString::from(format!("{}/{}", "test", p.packet.topic));
                            p.topic.insert(0, Level::Normal("test".into()));
                            Publish::V3(p)
                        }
                        Publish::V5(p) => Publish::V5(p.clone()),
                    };
                    log::debug!("{:?} MessageDelivered, {:?}", c.id, p.topic().to_string());
                    return (true, Some(HookResult::Publish(p)));
                }

                let response =
                    self.client.send_message(Message::Forward(c.id.clone(), (*publish).clone())).await;

                log::debug!("{:?} response: {:?}", c.id, response);
            }
            Parameter::MessageDelivered(_session, c, from, _publish) => {
                log::debug!("{:?} MessageDelivered, {:?}", c.id, from);
            }
            Parameter::ClientSubscribeCheckAcl(_s, _c, subscribe) => {
                log::debug!("{:?} ClientSubscribeCheckAcl, {:?}", _c.id, subscribe);
                let mut acl_result = match subscribe {
                    Subscribe::V3(_) => SubscribeAclResult::V3(Vec::new()),
                    Subscribe::V5(_) => SubscribeAclResult::V5(Vec::new()),
                };
                for (tf, qos) in subscribe.topic_filters() {
                    if tf.to_string() == "/x/y/z" {
                        acl_result.add_not_authorized();
                    } else {
                        acl_result.add_success(qos)
                    }
                }
                log::debug!("{:?} acl_result, {:?}", _c.id, acl_result);
                return (true, Some(HookResult::SubscribeAclResult(acl_result)));
            }

            Parameter::GrpcMessageReceived(msg) => {
                log::debug!("GrpcMessageReceived, {:?}", msg);
                // match msg {
                //     Message::Forward(from, publish) => {
                //         tokio::spawn(forwards(vec![(from.clone(), publish.clone())]));
                //     }
                //     Message::Forwards(publishs) => {
                //         tokio::spawn(forwards(publishs.clone()));
                //     }
                //     _ => {}
                // }
            }

            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

async fn _forwards(publishs: Vec<(From, Publish)>) {
    for (from, publish) in publishs {
        if let Err(droppeds) = { Runtime::instance().extends.shared().await.forwards(from, publish).await } {
            for (to, from, publish, reason) in droppeds {
                //hook, message_dropped
                Runtime::instance()
                    .extends
                    .hook_mgr()
                    .await
                    .message_dropped(Some(to), from, publish, reason)
                    .await;
            }
        }
    }
}
