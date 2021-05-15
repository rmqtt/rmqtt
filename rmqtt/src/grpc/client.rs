use tonic::transport::{Channel, Endpoint};
use tower::timeout::Timeout;

use super::pb::{self, node_service_client::NodeServiceClient};
use super::{Message, MessageReply};
use crate::{MqttError, Result, Runtime};

pub struct NodeGrpcClient {
    grpc_client: Option<NodeServiceClient<Timeout<Channel>>>,
    endpoint: Endpoint,
}

impl NodeGrpcClient {
    pub fn new(server_addr: std::net::SocketAddr) -> Result<Self> {
        let endpoint = Channel::from_shared(format!("http://{}", server_addr.to_string()))
            .map_err(anyhow::Error::new)?;

        Ok(Self { grpc_client: None, endpoint })
    }

    async fn _connect(&self) -> Result<NodeServiceClient<Timeout<Channel>>> {
        let channel = self.endpoint.connect().await.map_err(anyhow::Error::new)?;
        let timeout_channel = Timeout::new(channel, Runtime::instance().settings.rpc.timeout);
        let client = NodeServiceClient::new(timeout_channel);
        Ok(client)
    }

    async fn connect(&mut self) -> Result<&mut NodeServiceClient<Timeout<Channel>>> {
        if self.grpc_client.is_none() {
            self.grpc_client = Some(self._connect().await?);
        }
        self.grpc_client.as_mut().ok_or_else(|| MqttError::from("unreachable!"))
    }

    pub async fn send_message(&mut self, msg: Message) -> Result<MessageReply> {
        let grpc_client = self.connect().await?;
        let response = grpc_client
            .send_message(tonic::Request::new(pb::Message { data: msg.encode()? }))
            .await
            .map_err(anyhow::Error::new)?;
        log::trace!("response: {:?}", response);
        let message_reply = response.into_inner();
        Ok(MessageReply::decode(&message_reply.data)?)
    }
}
