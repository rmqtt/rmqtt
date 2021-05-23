use std::net::SocketAddr;

use crate::grpc::client::NodeGrpcClient;
use crate::grpc::server::Server;
use crate::{NodeId, Result, Runtime};

pub struct Node {}

impl Node {
    pub(crate) fn new() -> Self {
        Self {}
    }

    #[inline]
    pub fn id(&self) -> NodeId {
        Runtime::instance().settings.node.id
    }

    #[inline]
    pub async fn new_grpc_client(&self, remote_addr: &SocketAddr) -> Result<NodeGrpcClient> {
        NodeGrpcClient::new(remote_addr).await
    }

    pub fn start_grpc_server(&self) {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(Runtime::instance().settings.rpc.server_workers)
                .thread_name("grpc-server-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();
            let runner = async { Server::new().listen_and_serve().await };
            rt.block_on(runner).unwrap()
        });
    }
}
