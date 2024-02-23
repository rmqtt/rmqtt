use crate::PluginConfig;
use once_cell::sync::OnceCell;
use rmqtt::{async_trait::async_trait, log, once_cell, tokio::sync::RwLock, Runtime};
use rmqtt::{
    broker::{
        default::DefaultRetainStorage,
        types::{Retain, TopicFilter, TopicName},
        RetainStorage,
    },
    grpc::{Message, MessageBroadcaster, MessageReply, MessageType},
    Result,
};
use std::sync::Arc;

pub(crate) struct Retainer {
    inner: &'static DefaultRetainStorage,
    cfg: Arc<RwLock<PluginConfig>>,
    pub message_type: MessageType,
}

impl Retainer {
    #[inline]
    pub(crate) fn get_or_init(
        cfg: Arc<RwLock<PluginConfig>>,
        message_type: MessageType,
    ) -> &'static Retainer {
        static INSTANCE: OnceCell<Retainer> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { inner: DefaultRetainStorage::instance(), cfg, message_type })
    }

    #[inline]
    pub(crate) fn inner(&self) -> &'static DefaultRetainStorage {
        self.inner
    }

    #[inline]
    pub(crate) async fn remove_expired_messages(&self) {
        self.inner.remove_expired_messages().await;
    }
}

#[async_trait]
impl RetainStorage for &'static Retainer {
    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain) -> Result<()> {
        let (max_retained_messages, max_payload_size, expiry_interval) = {
            let cfg = self.cfg.read().await;
            let expiry_interval =
                if cfg.expiry_interval.is_zero() { None } else { Some(cfg.expiry_interval) };
            (cfg.max_retained_messages, *cfg.max_payload_size, expiry_interval)
        };

        if retain.publish.payload.len() > max_payload_size {
            log::warn!("Retain message payload exceeding limit, topic: {:?}, retain: {:?}", topic, retain);
            return Ok(());
        }

        if max_retained_messages > 0 && self.inner.count() >= max_retained_messages {
            log::warn!(
                "The retained message has exceeded the maximum limit of: {}, topic: {:?}, retain: {:?}",
                max_retained_messages,
                topic,
                retain
            );
            return Ok(());
        }

        self.inner.set_with_timeout(topic, retain, expiry_interval).await
    }

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        let mut retains = self.inner.get_message(topic_filter).await?;
        let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
        if grpc_clients.is_empty() {
            return Ok(retains);
        }

        //get retain info from other nodes
        let replys = MessageBroadcaster::new(
            grpc_clients,
            self.message_type,
            Message::GetRetains(topic_filter.clone()),
        )
        .join_all()
        .await;

        for (_, reply) in replys {
            match reply {
                Ok(reply) => {
                    if let MessageReply::GetRetains(o_retains) = reply {
                        if !o_retains.is_empty() {
                            retains.extend(o_retains);
                        }
                    }
                }
                Err(e) => {
                    log::error!(
                        "Get Message::GetRetains from other node, topic_filter: {:?}, error: {:?}",
                        topic_filter,
                        e
                    );
                }
            }
        }
        Ok(retains)
    }

    #[inline]
    fn count(&self) -> isize {
        self.inner.count()
    }

    #[inline]
    fn max(&self) -> isize {
        self.inner.max()
    }
}
