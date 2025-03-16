use std::net::SocketAddr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use once_cell::sync::Lazy;
use tonic::{transport, Response};

use crate::context::ServerContext;
use crate::Result;

use super::pb::{
    self,
    node_service_server::{NodeService, NodeServiceServer},
};
use super::{Message, MessageReply, MessageType, MESSAGE_TYPE_MESSAGE_GET};

pub struct Server {
    scx: ServerContext,
}

impl Server {
    pub(crate) fn new(scx: ServerContext) -> Self {
        Self { scx }
    }

    pub(crate) async fn listen_and_serve(
        &self,
        server_addr: SocketAddr,
        reuseaddr: bool,
        reuseport: bool,
    ) -> Result<()> {
        //start grpc server

        //NodeServiceServer::with_interceptor(RmqttNodeService::default(), Self::check_auth)

        log::info!(
            "gRPC Server listening on tcp://{:?}, reuseaddr: {}, reuseport: {}",
            server_addr,
            reuseaddr,
            reuseport
        );
        let server = transport::Server::builder()
            .add_service(NodeServiceServer::new(NodeGrpcService { scx: self.scx.clone() }));

        if reuseaddr || reuseport {
            let listener = tokio_stream::wrappers::TcpListenerStream::new(tokio::net::TcpListener::from_std(
                Self::bind(server_addr, 1024, reuseaddr, reuseport)?,
            )?);
            server.serve_with_incoming(listener).await?;
        } else {
            server.serve(server_addr).await?;
        }
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

    #[inline]
    pub fn bind(
        laddr: std::net::SocketAddr,
        backlog: i32,
        _reuseaddr: bool,
        _reuseport: bool,
    ) -> Result<std::net::TcpListener> {
        use socket2::{Domain, SockAddr, Socket, Type};
        let builder = Socket::new(Domain::for_address(laddr), Type::STREAM, None)?;
        builder.set_nonblocking(true)?;
        #[cfg(unix)]
        builder.set_reuse_address(_reuseaddr)?;
        #[cfg(unix)]
        builder.set_reuse_port(_reuseport)?;
        builder.bind(&SockAddr::from(laddr))?;
        builder.listen(backlog)?;
        Ok(std::net::TcpListener::from(builder))
    }
}

#[derive(Debug)]
pub struct NodeGrpcService {
    scx: ServerContext,
}

impl NodeGrpcService {
    async fn grpc_message_received(&self, typ: MessageType, msg: Message) -> Result<MessageReply> {
        match (typ, msg) {
            (MESSAGE_TYPE_MESSAGE_GET, Message::MessageGet(client_id, topic_filter, group)) => {
                match self
                    .scx
                    .extends
                    .message_mgr()
                    .await
                    .get(&client_id, &topic_filter, group.as_ref())
                    .await
                {
                    Err(e) => Ok(MessageReply::Error(e.to_string())),
                    Ok(msgs) => Ok(MessageReply::MessageGet(msgs)),
                }
            }
            (_, msg) => self.scx.extends.hook_mgr().grpc_message_received(typ, msg).await,
        }
    }
}

#[tonic::async_trait]
impl NodeService for NodeGrpcService {
    #[inline]
    async fn send_message(
        &self,
        request: tonic::Request<pb::Message>,
    ) -> std::result::Result<Response<pb::MessageReply>, tonic::Status> {
        log::trace!("request: {:?}", request);
        let req = request.into_inner();
        let msg = Message::decode(&req.data).map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        ACTIVE_REQUEST_COUNT.fetch_add(1, Ordering::SeqCst);
        let reply = self
            .grpc_message_received(req.typ, msg)
            .await
            .map_err(|e| tonic::Status::unavailable(e.to_string()));
        ACTIVE_REQUEST_COUNT.fetch_sub(1, Ordering::SeqCst);
        Ok(Response::new(pb::MessageReply {
            data: reply?.encode().map_err(|e| tonic::Status::unavailable(e.to_string()))?,
        }))
    }

    #[inline]
    async fn batch_send_messages(
        &self,
        request: tonic::Request<pb::BatchMessages>,
    ) -> std::result::Result<Response<pb::BatchMessagesReply>, tonic::Status> {
        log::trace!("request: {:?}", request);
        let req = request.into_inner();
        let msgs = bincode::deserialize::<Vec<(MessageType, Message)>>(&req.data)
            .map_err(|e| tonic::Status::unavailable(e.to_string()))?;
        ACTIVE_REQUEST_COUNT.fetch_add(1, Ordering::SeqCst);

        let mut futs = Vec::new();
        for (typ, msg) in msgs {
            futs.push(self.grpc_message_received(typ, msg));
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

    #[inline]
    async fn ping(
        &self,
        request: tonic::Request<pb::Empty>,
    ) -> std::result::Result<Response<pb::PingReply>, tonic::Status> {
        log::trace!("request: {:?}", request);
        Ok(Response::new(pb::PingReply {}))
    }
}

//@TODO... 将统计加到 Stats中
pub static ACTIVE_REQUEST_COUNT: Lazy<Arc<AtomicIsize>> = Lazy::new(|| Arc::new(AtomicIsize::new(0)));

pub fn active_grpc_requests() -> isize {
    ACTIVE_REQUEST_COUNT.load(Ordering::SeqCst)
}
