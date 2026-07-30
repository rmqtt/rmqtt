//! Command-line argument types for the RMQTT broker.
//!
//! This module defines the data structures used to parse and represent
//! command-line arguments, such as node identity, gRPC addresses, and
//! Raft peer configuration.

use crate::types::NodeId;
use crate::utils::NodeAddr;

/// Parsed command-line arguments for the RMQTT broker.
///
/// Holds optional overrides for node identity, gRPC peer addresses,
/// and Raft cluster configuration. These values can be supplied via
/// CLI flags to override the main configuration file.
#[derive(Debug, Clone, Default)]
pub struct CommandArgs {
    /// Node id
    pub node_id: Option<NodeId>,

    // The following parameter items are mainly for compatibility with older versions
    // and will be uniformly optimized in the future.
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
