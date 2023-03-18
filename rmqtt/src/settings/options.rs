use structopt::StructOpt;

use crate::NodeId;

use super::NodeAddr;

#[derive(StructOpt, Debug, Clone, Default)]
pub struct Options {
    /// Config filename
    #[structopt(name = "config", short = "f", long)]
    pub cfg_name: Option<String>,

    /// Node id
    #[structopt(name = "id", long)]
    pub node_id: Option<NodeId>,

    /// Launched Plug ins
    #[structopt(name = "plugins-default-startups", long)]
    pub plugins_default_startups: Option<Vec<String>>,

    ///Node gRPC service address list, --node-grpc-addrs "1@127.0.0.1:5363" "2@127.0.0.1:5364" "3@127.0.0.1:5365"
    #[structopt(name = "node-grpc-addrs", long)]
    pub node_grpc_addrs: Option<Vec<NodeAddr>>,

    ///Raft peer address list, --raft-peer-addrs "1@127.0.0.1:6003" "2@127.0.0.1:6004" "3@127.0.0.1:6005"
    #[structopt(name = "raft-peer-addrs", long)]
    pub raft_peer_addrs: Option<Vec<NodeAddr>>,
    // ///Node cookie
    // #[structopt(name = "cookie", long)]
    // pub node_cookie: Option<String>,

    // ///RPC server laddr
    // #[structopt(name = "rpc-server-laddr", long)]
    // pub rpc_server_laddr: Option<String>,
    //
    // ///RPC server workers
    // #[structopt(name = "rpc-server-workers", long)]
    // pub rpc_server_workers: Option<usize>,
    //
    // ///RPC client concurrency limit
    // #[structopt(name = "rpc-client-concurrency-limit", long)]
    // pub rpc_client_concurrency_limit: Option<usize>,
    //
    // ///RPC client timeout
    // #[structopt(name = "rpc-client-timeout", long)]
    // pub rpc_client_timeout: Option<Duration>,
    //
    // ///RPC batch size
    // #[structopt(name = "rpc-batch-size", long)]
    // pub rpc_batch_size: Option<usize>,
}
