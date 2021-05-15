use tonic::{transport, Response};

use super::pb::{
    self,
    node_service_server::{NodeService, NodeServiceServer},
};
use super::Message;
use crate::{Result, Runtime};

pub struct Server {}

impl Server {
    pub(crate) fn new() -> Self {
        Self {}
    }

    pub(crate) async fn listen_and_serve(&self) -> Result<()> {
        //start grpc server
        let addr = Runtime::instance().settings.rpc.server_addr;

        //NodeServiceServer::with_interceptor(RmqttNodeService::default(), Self::check_auth)

        log::info!("grpc server is listening on tcp://{:?}", addr);
        transport::Server::builder()
            .add_service(NodeServiceServer::new(NodeGrpcService::default()))
            .serve(addr)
            .await
            .map_err(anyhow::Error::new)?;
        Ok(())
    }

    // fn check_auth(req: Request<()>) -> std::result::Result<Request<()>, Status> {
    //     log::debug!("check_auth, req: {:?}", req);
    //
    //     let token = MetadataValue::from_str(Runtime::instance().settings.node.cookie.as_str())
    //         .map_err(|e| Status::new(Code::Unauthenticated, e.to_string()))?;
    //     match req.metadata().get("authorization") {
    //         Some(t) if token == t => Ok(req),
    //         _ => Err(Status::unauthenticated("No valid auth token")),
    //     }
    // }
}

#[derive(Debug, Default)]
pub struct NodeGrpcService {}

#[tonic::async_trait]
impl NodeService for NodeGrpcService {
    async fn send_message(
        &self,
        request: tonic::Request<pb::Message>,
    ) -> Result<tonic::Response<pb::MessageReply>, tonic::Status> {
        log::trace!("request: {:?}", request);
        let reply = Runtime::instance()
            .extends
            .hook_mgr()
            .await
            .grpc_message_received(Message::decode(&request.into_inner().data)?)
            .await;
        Ok(Response::new(pb::MessageReply { data: reply?.encode()? }))
    }
}
