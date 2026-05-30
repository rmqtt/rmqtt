use clap::Parser;

use rmqtt_utils::{NodeAddr, NodeId};

#[derive(Parser, Debug, Clone, Default)]
#[command(disable_version_flag = true)]
pub struct Options {
    /// Prints version information
    #[arg(short = 'V', long = "version")]
    pub version: bool,

    /// Config filename
    #[arg(short = 'f', long = "config")]
    pub cfg_name: Option<String>,

    /// Node id
    #[arg(long = "id")]
    pub node_id: Option<NodeId>,

    /// Launched Plug ins
    #[arg(long = "plugins-default-startups")]
    pub plugins_default_startups: Option<Vec<String>>,

    ///Node gRPC service address list, --node-grpc-addrs "1@127.0.0.1:5363" "2@127.0.0.1:5364" "3@127.0.0.1:5365"
    #[arg(long = "node-grpc-addrs")]
    pub node_grpc_addrs: Option<Vec<NodeAddr>>,

    ///Raft peer address list, --raft-peer-addrs "1@127.0.0.1:6003" "2@127.0.0.1:6004" "3@127.0.0.1:6005"
    #[arg(long = "raft-peer-addrs")]
    pub raft_peer_addrs: Option<Vec<NodeAddr>>,

    ///Specify a leader id, when the value is 0 or not specified, the first node
    ///will be designated as the Leader. Default value: 0
    #[arg(long = "raft-leader-id")]
    pub raft_leader_id: Option<NodeId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default() {
        let opts = Options::parse_from(["test"]);
        assert!(!opts.version);
        assert!(opts.cfg_name.is_none());
        assert!(opts.node_id.is_none());
        assert!(opts.plugins_default_startups.is_none());
        assert!(opts.node_grpc_addrs.is_none());
        assert!(opts.raft_peer_addrs.is_none());
        assert!(opts.raft_leader_id.is_none());
    }

    #[test]
    fn test_version_short() {
        let opts = Options::parse_from(["test", "-V"]);
        assert!(opts.version);
    }

    #[test]
    fn test_version_long() {
        let opts = Options::parse_from(["test", "--version"]);
        assert!(opts.version);
    }

    #[test]
    fn test_config_short() {
        let opts = Options::parse_from(["test", "-f", "rmqtt.toml"]);
        assert_eq!(opts.cfg_name.as_deref(), Some("rmqtt.toml"));
    }

    #[test]
    fn test_config_long() {
        let opts = Options::parse_from(["test", "--config", "/etc/rmqtt/rmqtt.toml"]);
        assert_eq!(opts.cfg_name.as_deref(), Some("/etc/rmqtt/rmqtt.toml"));
    }

    #[test]
    fn test_node_id() {
        let opts = Options::parse_from(["test", "--id", "42"]);
        assert_eq!(opts.node_id, Some(42));
    }

    #[test]
    fn test_plugins_default_startups_single() {
        let opts = Options::parse_from(["test", "--plugins-default-startups", "rmqtt-acl"]);
        assert_eq!(opts.plugins_default_startups, Some(vec!["rmqtt-acl".to_string()]));
    }

    #[test]
    fn test_plugins_default_startups_multiple() {
        let opts = Options::parse_from(["test", "--plugins-default-startups", "rmqtt-acl", "--plugins-default-startups", "rmqtt-retainer"]);
        assert_eq!(opts.plugins_default_startups, Some(vec!["rmqtt-acl".to_string(), "rmqtt-retainer".to_string()]));
    }

    #[test]
    fn test_node_grpc_addrs_single() {
        let opts = Options::parse_from(["test", "--node-grpc-addrs", "1@127.0.0.1:5363"]);
        let addrs = opts.node_grpc_addrs.unwrap();
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].id, 1);
    }

    #[test]
    fn test_node_grpc_addrs_multiple() {
        let opts = Options::parse_from([
            "test",
            "--node-grpc-addrs", "1@127.0.0.1:5363",
            "--node-grpc-addrs", "2@127.0.0.1:5364",
        ]);
        let addrs = opts.node_grpc_addrs.unwrap();
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].id, 1);
        assert_eq!(addrs[1].id, 2);
    }

    #[test]
    fn test_raft_peer_addrs() {
        let opts = Options::parse_from([
            "test",
            "--raft-peer-addrs", "1@127.0.0.1:6003",
            "--raft-peer-addrs", "2@127.0.0.1:6004",
            "--raft-peer-addrs", "3@127.0.0.1:6005",
        ]);
        let addrs = opts.raft_peer_addrs.unwrap();
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0].id, 1);
        assert_eq!(addrs[1].id, 2);
        assert_eq!(addrs[2].id, 3);
    }

    #[test]
    fn test_raft_leader_id() {
        let opts = Options::parse_from(["test", "--raft-leader-id", "1"]);
        assert_eq!(opts.raft_leader_id, Some(1));
    }

    #[test]
    fn test_all_options() {
        let opts = Options::parse_from([
            "test",
            "-V",
            "-f", "config.toml",
            "--id", "99",
            "--plugins-default-startups", "rmqtt-acl",
            "--plugins-default-startups", "rmqtt-retainer",
            "--node-grpc-addrs", "1@127.0.0.1:5363",
            "--raft-peer-addrs", "2@127.0.0.1:6004",
            "--raft-leader-id", "2",
        ]);
        assert!(opts.version);
        assert_eq!(opts.cfg_name.as_deref(), Some("config.toml"));
        assert_eq!(opts.node_id, Some(99));
        assert_eq!(opts.plugins_default_startups.as_deref(), Some(&["rmqtt-acl".to_string(), "rmqtt-retainer".to_string()][..]));
        assert_eq!(opts.node_grpc_addrs.as_ref().map(|a| a.len()), Some(1));
        assert_eq!(opts.raft_peer_addrs.as_ref().map(|a| a.len()), Some(1));
        assert_eq!(opts.raft_leader_id, Some(2));
    }
}
