use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::sync::mpsc::{
    unbounded_channel as channel, UnboundedReceiver as Receiver, UnboundedSender as Sender,
};
use tokio::sync::oneshot::Sender as OneshotSender;
use tokio::sync::RwLock;
use tonic::transport::{Channel, Endpoint};

use crate::{MqttError, Result, Runtime};

use super::{Message, MessageReply, MessageType};
use super::pb::{self, node_service_client::NodeServiceClient};

type NodeServiceClientType = NodeServiceClient<Channel>;

#[derive(Clone)]
pub struct NodeGrpcClient {
    grpc_client: Arc<RwLock<Option<NodeServiceClientType>>>,
    active_tasks: Arc<AtomicUsize>,
    channel_tasks: Arc<AtomicUsize>,
    endpoint: Endpoint,
    tx: Sender<(MessageType, Message, OneshotSender<Result<MessageReply>>)>,
}

impl NodeGrpcClient {
    #[inline]
    pub async fn new(server_addr: &std::net::SocketAddr) -> Result<Self> {
        log::debug!("rpc.client_timeout: {:?}", Runtime::instance().settings.rpc.client_timeout);
        let concurrency_limit = Runtime::instance().settings.rpc.client_concurrency_limit + 1;
        let endpoint = Channel::from_shared(format!("http://{}", server_addr))
            .map(|endpoint| {
                endpoint
                    .concurrency_limit(concurrency_limit)
                    .timeout(Runtime::instance().settings.rpc.client_timeout)
            })
            .map_err(anyhow::Error::new)?;
        let active_tasks = Arc::new(AtomicUsize::new(0));
        let channel_tasks = Arc::new(AtomicUsize::new(0));
        let grpc_client = Arc::new(RwLock::new(None));
        let (tx, rx) = channel();
        let c = Self { grpc_client, active_tasks, channel_tasks, endpoint, tx };
        c.start(rx);
        Ok(c)
    }

    #[inline]
    pub fn active_tasks(&self) -> usize {
        self.active_tasks.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn channel_tasks(&self) -> usize {
        self.channel_tasks.load(Ordering::SeqCst)
    }

    #[inline]
    async fn _connect(endpoint: &Endpoint) -> Result<NodeServiceClientType> {
        let channel =
            tokio::time::timeout(Runtime::instance().settings.rpc.client_timeout, endpoint.connect())
                .await
                .map_err(anyhow::Error::new)?
                .map_err(anyhow::Error::new)?;
        let client = NodeServiceClient::new(channel);
        Ok(client)
    }

    #[inline]
    async fn connect(&self) -> Result<NodeServiceClientType> {
        if let Some(c) = self.grpc_client.read().await.as_ref() {
            return Ok(c.clone());
        }
        let c = Self::_connect(&self.endpoint).await?;
        self.grpc_client.write().await.replace(c.clone());
        Ok(c)
    }

    #[inline]
    pub async fn batch_send_message(&self, typ: MessageType, msg: Message) -> Result<MessageReply> {
        let (r_tx, r_rx) = tokio::sync::oneshot::channel::<Result<MessageReply>>();
        self.tx
            .send((typ, msg, r_tx))
            .map(|_| self.channel_tasks.fetch_add(1, Ordering::SeqCst))
            .map_err(|e| anyhow::Error::msg(e.to_string()))?;
        let reply = r_rx.await.map_err(anyhow::Error::new)??;
        let reply = match reply {
            MessageReply::Error(e) => return Err(MqttError::from(e)),
            _ => reply,
        };
        Ok(reply)
    }

    #[inline]
    pub async fn send_message(&self, typ: MessageType, msg: Message) -> Result<MessageReply> {
        self.batch_send_message(typ, msg).await
    }

    #[inline]
    async fn inner_send_message(&self, typ: MessageType, msg: Message) -> Result<MessageReply> {
        let mut grpc_client = self.connect().await?;
        self.active_tasks.fetch_add(1, Ordering::SeqCst);
        let result = Self::_inner_send_message(&mut grpc_client, typ, msg).await;
        self.active_tasks.fetch_sub(1, Ordering::SeqCst);
        result
    }

    #[inline]
    async fn _inner_send_message(
        c: &mut NodeServiceClientType,
        typ: MessageType,
        msg: Message,
    ) -> Result<MessageReply> {
        let response = c
            .send_message(tonic::Request::new(pb::Message { typ, data: msg.encode()? }))
            .await
            .map_err(anyhow::Error::new)?;
        log::trace!("response: {:?}", response);
        let message_reply = response.into_inner();
        MessageReply::decode(&message_reply.data)
    }

    #[inline]
    async fn inner_batch_send_messages(
        &self,
        msgs: Vec<(MessageType, Message)>,
    ) -> Result<Vec<MessageReply>> {
        let mut grpc_client = self.connect().await?;
        self.active_tasks.fetch_add(1, Ordering::SeqCst);
        let result = Self::_inner_batch_send_messages(&mut grpc_client, msgs).await;
        self.active_tasks.fetch_sub(1, Ordering::SeqCst);
        result
    }

    #[inline]
    async fn _inner_batch_send_messages(
        c: &mut NodeServiceClientType,
        msgs: Vec<(MessageType, Message)>,
    ) -> Result<Vec<MessageReply>> {
        let data = bincode::serialize(&msgs).map_err(anyhow::Error::new)?;
        let response = c
            .batch_send_messages(tonic::Request::new(pb::BatchMessages { data }))
            .await
            .map_err(anyhow::Error::new)?;
        log::trace!("response: {:?}", response);
        let message_reply = response.into_inner();

        Ok(bincode::deserialize::<Vec<MessageReply>>(&message_reply.data).map_err(anyhow::Error::new)?)
    }

    fn start(&self, mut rx: Receiver<(MessageType, Message, OneshotSender<Result<MessageReply>>)>) {
        let endpoint = self.endpoint.clone();
        let client = self.clone();
        let channel_tasks = self.channel_tasks.clone();
        tokio::task::spawn(async move {
            let mut merger_msgs = Vec::new();
            let mut merger_txs = Vec::new();
            let batch_size = Runtime::instance().settings.rpc.batch_size;
            while let Some((typ, msg, r_tx)) = rx.recv().await {
                channel_tasks.fetch_sub(1, Ordering::SeqCst);
                log::debug!("recv, type: {}, message: {:?}", typ, msg);
                merger_msgs.push((typ, msg));
                merger_txs.push(r_tx);
                while merger_msgs.len() < batch_size {
                    match tokio::time::timeout(Duration::from_millis(0), rx.recv()).await {
                        Ok(Some((typ, msg, r_tx))) => {
                            channel_tasks.fetch_sub(1, Ordering::SeqCst);
                            log::debug!("try_recv, type: {}, message: {:?}", typ, msg);
                            merger_msgs.push((typ, msg));
                            merger_txs.push(r_tx);
                        }
                        _ => break,
                    }
                }
                log::debug!(
                    "merger_msgs.len: {}, merger_txs.len(): {:?}",
                    merger_msgs.len(),
                    merger_txs.len()
                );
                //merge and send
                let msgs = merger_msgs.drain(..).collect::<Vec<(MessageType, Message)>>();
                let r_txs = merger_txs.drain(..).collect::<Vec<OneshotSender<Result<MessageReply>>>>();

                if client.active_tasks() < Runtime::instance().settings.rpc.client_concurrency_limit {
                    tokio::task::spawn(Self::_send(client.clone(), msgs, r_txs));
                } else {
                    Self::_send(client.clone(), msgs, r_txs).await;
                }
            }
            log::info!("exit NodeGrpcClient, {:?}", endpoint);
        });
    }

    async fn _send(
        client: NodeGrpcClient,
        mut msgs: Vec<(MessageType, Message)>,
        mut r_txs: Vec<OneshotSender<Result<MessageReply>>>,
    ) {
        if msgs.len() == 1 {
            let (typ, msg) = msgs.remove(0);
            let r_tx = r_txs.remove(0);
            let reply = client.inner_send_message(typ, msg).await;
            if let Err(r) = r_tx.send(reply) {
                log::error!("Failed to return result, reply message: {:?}", r);
            }
        } else {
            match client.inner_batch_send_messages(msgs).await {
                Err(e) => {
                    for r_tx in r_txs {
                        if let Err(r) = r_tx.send(Err(MqttError::from(e.to_string()))) {
                            log::debug!("Failed to return result, reply error message: {:?}", r);
                        }
                    }
                }
                Ok(mut replys) => {
                    for msg in replys.drain(..) {
                        if let Err(r) = r_txs.remove(0).send(Ok(msg)) {
                            log::debug!("Failed to return result, reply message: {:?}", r);
                        }
                    }
                }
            }
        }
    }
}
