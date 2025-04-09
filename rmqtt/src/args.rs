use crate::utils::NodeAddr;
use crate::NodeId;

#[derive(Debug, Clone, Default)]
pub struct CommandArgs {
    /// Node id
    pub node_id: Option<NodeId>,

    //下面的参数项主要是为了兼容，旧版本，之后会统一优化掉
    /// Launched Plug ins
    pub plugins_default_startups: Option<Vec<String>>,
    ///Node gRPC service address list, --node-grpc-addrs "1@127.0.0.1:5363" "2@127.0.0.1:5364" "3@127.0.0.1:5365"
    pub node_grpc_addrs: Option<Vec<NodeAddr>>,
    ///Raft peer address list, --raft-peer-addrs "1@127.0.0.1:6003" "2@127.0.0.1:6004" "3@127.0.0.1:6005"
    pub raft_peer_addrs: Option<Vec<NodeAddr>>,
    ///Specify a leader id, when the value is 0 or not specified, the first node
    ///will be designated as the Leader. Default value: 0
    pub raft_leader_id: Option<NodeId>,
}
