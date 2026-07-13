use std::collections::HashMap;
use std::time::Duration;

use anyhow::{anyhow, Result};
use prost::Message;
use serde::Deserialize;
use tikv_raft::eraftpb::{ConfChange, ConfChangeType};
use tonic::codec::ProstCodec;
use tonic::transport::Channel;

use rmqtt::types::NodeId;
use rmqtt_raft::Status;

#[derive(Clone, PartialEq, Message)]
struct RaftConfChange {
    #[prost(bytes = "vec", tag = "1")]
    inner: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct RaftResponseMessage {
    #[prost(bytes = "vec", tag = "2")]
    inner: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
enum RaftResponse {
    WrongLeader { leader_id: u64, leader_addr: Option<String> },
    JoinSuccess { assigned_id: u64, peer_addrs: HashMap<u64, String> },
    RequestId { leader_id: u64 },
    Error(String),
    Busy,
    Response { data: Vec<u8> },
    Status(Status),
    Ok,
}

pub(crate) async fn remove_node(
    leader_addr: &str,
    node_id: NodeId,
    timeout: Duration,
    concurrency_limit: usize,
    message_size: usize,
) -> Result<()> {
    let mut change = ConfChange::default();
    change.set_node_id(node_id);
    change.set_change_type(ConfChangeType::RemoveNode);

    let endpoint = Channel::from_shared(format!("http://{leader_addr}"))?
        .concurrency_limit(concurrency_limit)
        .connect_timeout(timeout)
        .timeout(timeout);
    let channel = endpoint.connect().await?;
    let mut grpc = tonic::client::Grpc::new(channel)
        .max_decoding_message_size(message_size)
        .max_encoding_message_size(message_size);

    grpc.ready().await?;
    let path = tonic::codegen::http::uri::PathAndQuery::from_static("/raftservice.RaftService/ChangeConfig");
    let codec = ProstCodec::<RaftConfChange, RaftResponseMessage>::default();
    let response = grpc
        .unary(tonic::Request::new(RaftConfChange { inner: change.encode_to_vec() }), path, codec)
        .await?;

    let response: RaftResponse = postcard::from_bytes(&response.into_inner().inner)?;
    match response {
        RaftResponse::Ok => Ok(()),
        RaftResponse::WrongLeader { leader_id, leader_addr } => Err(anyhow!(
            "raft remove_node sent to wrong leader, actual leader_id: {leader_id}, leader_addr: {leader_addr:?}"
        )),
        RaftResponse::Error(e) => Err(anyhow!(e)),
        RaftResponse::Busy => Err(anyhow!("raft leader is busy")),
        other => Err(anyhow!("unexpected raft remove_node response: {other:?}")),
    }
}
