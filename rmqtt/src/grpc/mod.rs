use std::collections::HashMap;
use bytes::Bytes;
use std::sync::Arc;
use std::net::SocketAddr;
use futures::FutureExt;

use crate::broker::{ClearSubscriptions, SubRelations, SubRelationsMap};
use crate::broker::session::SessionOfflineInfo;
use crate::broker::types::{NodeId, From, Id, Publish, Retain, TopicFilter, TopicName};
use crate::node::{BrokerInfo, NodeInfo};
use crate::Result;

use client::NodeGrpcClient;

pub mod client;
pub mod server;

#[allow(dead_code)]
pub(crate) mod pb {
    include!(concat!(env!("OUT_DIR"), "/pb.rs"));
}

///Reserved within 1000
pub type MessageType = u64;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message {
    Forwards(From, Publish),
    ForwardsTo(From, Publish, SubRelations),
    Kick(Id, ClearSubscriptions),
    GetRetains(TopicFilter),
    NumberOfClients,
    NumberOfSessions,
    BrokerInfo,
    NodeInfo,
    Bytes(Bytes),
}

impl Message {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Message> {
        Ok(bincode::deserialize::<Message>(data).map_err(anyhow::Error::new)?)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MessageReply {
    Success,
    Forwards(SubRelationsMap),
    Error(String),
    Kick(Option<SessionOfflineInfo>),
    GetRetains(Vec<(TopicName, Retain)>),
    NumberOfClients(usize),
    NumberOfSessions(usize),
    BrokerInfo(BrokerInfo),
    NodeInfo(NodeInfo),
    Bytes(Vec<u8>),
}

impl MessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<MessageReply> {
        Ok(bincode::deserialize::<MessageReply>(data).map_err(anyhow::Error::new)?)
    }
}


pub struct MessageSender {
    client: NodeGrpcClient,
    msg_type: MessageType,
    msg: Message,
}

impl MessageSender {

    #[inline]
    pub fn new(client: NodeGrpcClient, msg_type: MessageType, msg: Message) -> Self {
        Self { client, msg_type, msg }
    }

    #[inline]
    pub async fn send(self) -> Result<MessageReply> {
        match self.client.send_message(self.msg_type, self.msg).await {
            Ok(reply) => {
                return Ok(reply);
            }
            Err(e) => {
                log::warn!("error sending message, {:?}", e);
                return Err(e);
            }
        }
    }
}

pub type GrpcClients = Arc<HashMap<NodeId, (SocketAddr, NodeGrpcClient), ahash::RandomState>>;

pub struct MessageBroadcaster {
    grpc_clients: GrpcClients,
    msg_type: MessageType,
    msg: Option<Message>,
}

impl MessageBroadcaster {

    #[inline]
    pub fn new(grpc_clients: GrpcClients, msg_type: MessageType, msg: Message) -> Self {
        assert!(!grpc_clients.is_empty(), "gRPC clients is empty!");
        Self { grpc_clients, msg_type, msg: Some(msg) }
    }

    #[inline]
    pub async fn join_all(&mut self) -> Vec<Result<MessageReply>> {
        let msg = self.msg.take().unwrap();
        let mut senders = Vec::new();
        let max_idx = self.grpc_clients.len() - 1;
        for (i, (_, (_, grpc_client))) in self.grpc_clients.iter().enumerate() {
            if i == max_idx {
                senders.push(grpc_client.send_message(self.msg_type, msg));
                break;
            } else {
                senders.push(grpc_client.send_message(self.msg_type, msg.clone()));
            }
        }
        futures::future::join_all(senders).await
    }

    #[inline]
    pub async fn select_ok<F: Fn(MessageReply) -> Result<MessageReply> + Send + Sync>(
        &mut self,
        check_fn: &F,
    ) -> Result<MessageReply> {
        let msg = self.msg.take().unwrap();
        let mut senders = Vec::new();
        let max_idx = self.grpc_clients.len() - 1;
        for (i, (_, (_, grpc_client))) in self.grpc_clients.iter().enumerate() {
            if i == max_idx {
                senders.push(Self::send(grpc_client, self.msg_type, msg, check_fn).boxed());
                break;
            } else {
                senders.push(Self::send(grpc_client, self.msg_type, msg.clone(), check_fn).boxed());
            }
        }
        let (reply, _) = futures::future::select_ok(senders).await?;
        Ok(reply)
    }

    #[inline]
    async fn send<F: Fn(MessageReply) -> Result<MessageReply> + Send + Sync>(
        grpc_client: &NodeGrpcClient,
        typ: MessageType,
        msg: Message,
        check_fn: &F,
    ) -> Result<MessageReply> {
        match grpc_client.send_message(typ, msg).await {
            Ok(r) => {
                log::debug!("OK reply: {:?}", r);
                check_fn(r)
            }
            Err(e) => {
                log::debug!("ERROR reply: {:?}", e);
                Err(e)
            }
        }
    }
}
