use once_cell::sync::OnceCell;

use rmqtt::{async_trait::async_trait, log, once_cell};
use rmqtt::{
    broker::{
        default::DefaultRetainStorage,
        types::{Retain, TopicFilter, TopicName},
        RetainStorage,
    },
    grpc::{GrpcClients, Message, MessageBroadcaster, MessageReply, MessageType},
    Result,
};

pub(crate) struct ClusterRetainer {
    inner: &'static DefaultRetainStorage,
    grpc_clients: GrpcClients,
    pub message_type: MessageType,
}

impl ClusterRetainer {
    #[inline]
    pub(crate) fn get_or_init(
        grpc_clients: GrpcClients,
        message_type: MessageType,
    ) -> &'static ClusterRetainer {
        static INSTANCE: OnceCell<ClusterRetainer> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { inner: DefaultRetainStorage::instance(), grpc_clients, message_type })
    }

    #[inline]
    pub(crate) fn inner(&self) -> Box<dyn RetainStorage> {
        Box::new(self.inner)
    }
}

#[async_trait]
impl RetainStorage for &'static ClusterRetainer {
    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain) -> Result<()> {
        self.inner.set(topic, retain).await
    }

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        let mut retains = self.inner.get(topic_filter).await?;

        //get retain info from other nodes
        let replys = MessageBroadcaster::new(
            self.grpc_clients.clone(),
            self.message_type,
            Message::GetRetains(topic_filter.clone()),
        )
        .join_all()
        .await;

        for (_, reply) in replys {
            match reply {
                Ok(reply) => {
                    if let MessageReply::GetRetains(o_retains) = reply {
                        retains.extend(o_retains);
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
