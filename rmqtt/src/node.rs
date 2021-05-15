use std::net::SocketAddr;

use crate::grpc::client::NodeGrpcClient;
use crate::grpc::server::Server;
use crate::settings::Settings;
use crate::{NodeId, Result};

pub struct Node {
    id: NodeId,
}

impl Node {
    pub(crate) fn new(settings: Settings) -> Self {
        Self { id: settings.node.id }
    }

    #[inline]
    pub fn id(&self) -> NodeId {
        self.id
    }

    #[inline]
    pub fn new_grpc_client(&self, remote_addr: SocketAddr) -> Result<NodeGrpcClient> {
        NodeGrpcClient::new(remote_addr)
    }

    pub fn start_grpc_server(&self) {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .thread_name("grpc-server-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();
            let runner = async { Server::new().listen_and_serve().await };
            rt.block_on(runner).unwrap()
        });
    }
}
