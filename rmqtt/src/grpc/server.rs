use tonic::{transport, Response};

use super::pb::{
    self,
    node_service_server::{NodeService, NodeServiceServer},
};
use super::{Message, MessageReply, MessageType};
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
    #[inline]
    async fn send_message(
        &self,
        request: tonic::Request<pb::Message>,
    ) -> Result<tonic::Response<pb::MessageReply>, tonic::Status> {
        log::trace!("request: {:?}", request);
        let req = request.into_inner();
        let msg = Message::decode(&req.data)?;
        ACTIVE_REQUEST_COUNT.fetch_add(1, Ordering::SeqCst);
        let reply = Runtime::instance().extends.hook_mgr().await.grpc_message_received(req.typ, msg).await;
        ACTIVE_REQUEST_COUNT.fetch_sub(1, Ordering::SeqCst);
        Ok(Response::new(pb::MessageReply { data: reply?.encode()? }))
    }

    #[inline]
    async fn batch_send_messages(
        &self,
        request: tonic::Request<pb::BatchMessages>,
    ) -> Result<tonic::Response<pb::BatchMessagesReply>, tonic::Status> {
        log::trace!("request: {:?}", request);
        let req = request.into_inner();
        let msgs = bincode::deserialize::<Vec<(MessageType, Message)>>(&req.data)
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        ACTIVE_REQUEST_COUNT.fetch_add(1, Ordering::SeqCst);

        let hook_mgr = Runtime::instance().extends.hook_mgr().await;

        let mut futs = Vec::new();
        for (typ, msg) in msgs {
            futs.push(hook_mgr.grpc_message_received(typ, msg));
        }
        let reply = futures::future::join_all(futs)
            .await
            .drain(..)
            .map(|r| match r {
                Ok(r) => r,
                Err(e) => MessageReply::Error(e.to_string()),
            })
            .collect::<Vec<MessageReply>>();
        ACTIVE_REQUEST_COUNT.fetch_sub(1, Ordering::SeqCst);

        let reply = bincode::serialize(&reply).map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        Ok(Response::new(pb::BatchMessagesReply { data: reply }))
    }
}

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
lazy_static::lazy_static! {
    pub static ref ACTIVE_REQUEST_COUNT: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
}

pub fn active_grpc_requests() -> usize {
    ACTIVE_REQUEST_COUNT.load(Ordering::SeqCst)
}
