use anyhow::anyhow;
use std::time::Duration;

pub(crate) use backoff::future::retry;
pub(crate) use backoff::{ExponentialBackoff, ExponentialBackoffBuilder};
use once_cell::sync::Lazy;
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use rmqtt::utils::{deserialize_duration, deserialize_duration_option, NodeAddr};
use rmqtt::{
    args::CommandArgs,
    grpc::MessageType,
    types::{Addr, NodeId},
    Result,
};
use rmqtt_raft::ReadOnlyOption;

pub(crate) static BACKOFF_STRATEGY: Lazy<ExponentialBackoff> = Lazy::new(|| {
    ExponentialBackoffBuilder::new()
        .with_max_elapsed_time(Some(Duration::from_secs(60)))
        .with_multiplier(2.5)
        .build()
});

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::worker_threads_default")]
    pub worker_threads: usize,

    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    pub laddr: Option<Addr>,

    pub node_grpc_addrs: Vec<NodeAddr>,

    pub raft_peer_addrs: Vec<NodeAddr>,

    #[serde(default)]
    pub leader_id: NodeId,

    #[serde(default = "PluginConfig::try_lock_timeout_default", deserialize_with = "deserialize_duration")]
    pub try_lock_timeout: Duration, //Message::HandshakeTryLock

    #[serde(default = "PluginConfig::task_exec_queue_workers_default")]
    pub task_exec_queue_workers: usize,

    #[serde(default = "PluginConfig::task_exec_queue_max_default")]
    pub task_exec_queue_max: usize,

    #[serde(default)]
    pub verify_addr: bool,

    #[serde(default)]
    pub compression: Option<Compression>,

    #[serde(default)]
    pub health: Health,

    #[serde(default = "PluginConfig::raft_default")]
    pub raft: RaftConfig,
}

impl PluginConfig {
    #[inline]
    pub fn leader(&self) -> Result<Option<&NodeAddr>> {
        if self.leader_id == 0 {
            Ok(None)
        } else {
            let leader = self
                .raft_peer_addrs
                .iter()
                .find(|leader| leader.id == self.leader_id)
                .ok_or_else(|| anyhow!("Leader does not exist"))?;
            Ok(Some(leader))
        }
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    fn worker_threads_default() -> usize {
        6
    }

    fn message_type_default() -> MessageType {
        198
    }

    fn try_lock_timeout_default() -> Duration {
        Duration::from_secs(10)
    }

    fn task_exec_queue_workers_default() -> usize {
        500
    }

    fn task_exec_queue_max_default() -> usize {
        100_000
    }

    fn raft_default() -> RaftConfig {
        RaftConfig { ..Default::default() }
    }

    pub fn merge(&mut self, opts: &CommandArgs) {
        if let Some(node_grpc_addrs) = opts.node_grpc_addrs.as_ref() {
            self.node_grpc_addrs.clone_from(node_grpc_addrs);
        }
        if let Some(raft_peer_addrs) = opts.raft_peer_addrs.as_ref() {
            self.raft_peer_addrs.clone_from(raft_peer_addrs);
        }
        if let Some(raft_leader_id) = opts.raft_leader_id.as_ref() {
            self.leader_id = *raft_leader_id;
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Health {
    #[serde(default = "Health::exit_on_node_unavailable_default")]
    pub exit_on_node_unavailable: bool,
    #[serde(default = "Health::exit_code_default")]
    pub exit_code: i32,
    #[serde(default = "Health::max_continuous_unavailable_count_default")]
    pub max_continuous_unavailable_count: usize,
    #[serde(
        default = "Health::unavailable_check_interval_default",
        deserialize_with = "deserialize_duration"
    )]
    pub unavailable_check_interval: Duration,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            exit_on_node_unavailable: Self::exit_on_node_unavailable_default(),
            exit_code: Self::exit_code_default(),
            max_continuous_unavailable_count: Self::max_continuous_unavailable_count_default(),
            unavailable_check_interval: Self::unavailable_check_interval_default(),
        }
    }
}

impl Health {
    fn exit_on_node_unavailable_default() -> bool {
        false
    }

    fn exit_code_default() -> i32 {
        -1
    }

    fn max_continuous_unavailable_count_default() -> usize {
        2
    }

    fn unavailable_check_interval_default() -> Duration {
        Duration::from_secs(2)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct RaftConfig {
    #[serde(default = "RaftConfig::grpc_reuseaddr_default")]
    pub grpc_reuseaddr: bool,
    #[serde(default = "RaftConfig::grpc_reuseport_default")]
    pub grpc_reuseport: bool,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub grpc_timeout: Option<Duration>,
    pub grpc_concurrency_limit: Option<usize>,
    pub grpc_message_size: Option<usize>,
    pub grpc_breaker_threshold: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub grpc_breaker_retry_interval: Option<Duration>,
    pub proposal_batch_size: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub proposal_batch_timeout: Option<Duration>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub snapshot_interval: Option<Duration>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub heartbeat: Option<Duration>,

    /// The number of node.tick invocations that must pass between
    /// elections. That is, if a follower does not receive any message from the
    /// leader of current term before ElectionTick has elapsed, it will become
    /// candidate and start an election. election_tick must be greater than
    /// HeartbeatTick. We suggest election_tick = 10 * HeartbeatTick to avoid
    /// unnecessary leader switching
    pub election_tick: Option<usize>,

    /// HeartbeatTick is the number of node.tick invocations that must pass between
    /// heartbeats. That is, a leader sends heartbeat messages to maintain its
    /// leadership every heartbeat ticks.
    pub heartbeat_tick: Option<usize>,

    /// Limit the max size of each append message. Smaller value lowers
    /// the raft recovery cost(initial probing and message lost during normal operation).
    /// On the other side, it might affect the throughput during normal replication.
    /// Note: math.MaxUusize64 for unlimited, 0 for at most one entry per message.
    pub max_size_per_msg: Option<u64>,

    /// Limit the max number of in-flight append messages during optimistic
    /// replication phase. The application transportation layer usually has its own sending
    /// buffer over TCP/UDP. Set to avoid overflowing that sending buffer.
    /// TODO: feedback to application to limit the proposal rate?
    pub max_inflight_msgs: Option<usize>,

    /// Specify if the leader should check quorum activity. Leader steps down when
    /// quorum is not active for an electionTimeout.
    pub check_quorum: Option<bool>,

    /// Enables the Pre-Vote algorithm described in raft thesis section
    /// 9.6. This prevents disruption when a node that has been partitioned away
    /// rejoins the cluster.
    pub pre_vote: Option<bool>,

    /// The range of election timeout. In some cases, we hope some nodes has less possibility
    /// to become leader. This configuration ensures that the randomized election_timeout
    /// will always be suit in [min_election_tick, max_election_tick).
    /// If it is 0, then election_tick will be chosen.
    pub min_election_tick: Option<usize>,

    /// If it is 0, then 2 * election_tick will be chosen.
    pub max_election_tick: Option<usize>,

    /// Choose the linearizability mode or the lease mode to read data. If you don’t care about the read consistency and want a higher read performance, you can use the lease mode.
    ///
    /// Setting this to `LeaseBased` requires `check_quorum = true`.
    #[serde(
        default = "RaftConfig::read_only_option_default",
        serialize_with = "RaftConfig::serialize_read_only_option",
        deserialize_with = "RaftConfig::deserialize_read_only_option"
    )]
    pub read_only_option: ReadOnlyOption,

    /// Don't broadcast an empty raft entry to notify follower to commit an entry.
    /// This may make follower wait a longer time to apply an entry. This configuration
    /// May affect proposal forwarding and follower read.
    pub skip_bcast_commit: Option<bool>,

    /// Batches every append msg if any append msg already exists
    pub batch_append: Option<bool>,

    /// The election priority of this node.
    pub priority: Option<i64>,

    /// Specify maximum of uncommitted entry size.
    /// When this limit is reached, all proposals to append new log will be dropped
    pub max_uncommitted_size: Option<u64>,

    /// Max size for committed entries in a `Ready`.
    pub max_committed_size_per_ready: Option<u64>,
}

impl RaftConfig {
    pub(crate) fn to_raft_config(&self) -> rmqtt_raft::Config {
        let mut cfg = rmqtt_raft::Config { ..Default::default() };
        cfg.reuseaddr = self.grpc_reuseaddr;
        cfg.reuseport = self.grpc_reuseport;
        if let Some(grpc_timeout) = self.grpc_timeout {
            cfg.grpc_timeout = grpc_timeout;
        }
        if let Some(grpc_concurrency_limit) = self.grpc_concurrency_limit {
            cfg.grpc_concurrency_limit = grpc_concurrency_limit;
        }
        if let Some(grpc_message_size) = self.grpc_message_size {
            cfg.grpc_message_size = grpc_message_size;
        }
        if let Some(grpc_breaker_threshold) = self.grpc_breaker_threshold {
            cfg.grpc_breaker_threshold = grpc_breaker_threshold;
        }
        if let Some(grpc_breaker_retry_interval) = self.grpc_breaker_retry_interval {
            cfg.grpc_breaker_retry_interval = grpc_breaker_retry_interval;
        }
        if let Some(proposal_batch_size) = self.proposal_batch_size {
            cfg.proposal_batch_size = proposal_batch_size;
        }
        if let Some(proposal_batch_timeout) = self.proposal_batch_timeout {
            cfg.proposal_batch_timeout = proposal_batch_timeout;
        }
        if let Some(snapshot_interval) = self.snapshot_interval {
            cfg.snapshot_interval = snapshot_interval;
        }
        if let Some(heartbeat) = self.heartbeat {
            cfg.heartbeat = heartbeat;
        }

        //---------------------------------------------------------------------------
        if let Some(election_tick) = self.election_tick {
            cfg.raft_cfg.election_tick = election_tick;
        }
        if let Some(heartbeat_tick) = self.heartbeat_tick {
            cfg.raft_cfg.heartbeat_tick = heartbeat_tick;
        }
        if let Some(max_size_per_msg) = self.max_size_per_msg {
            cfg.raft_cfg.max_size_per_msg = max_size_per_msg;
        }
        if let Some(max_inflight_msgs) = self.max_inflight_msgs {
            cfg.raft_cfg.max_inflight_msgs = max_inflight_msgs;
        }
        if let Some(check_quorum) = self.check_quorum {
            cfg.raft_cfg.check_quorum = check_quorum;
        }
        if let Some(pre_vote) = self.pre_vote {
            cfg.raft_cfg.pre_vote = pre_vote;
        }
        if let Some(min_election_tick) = self.min_election_tick {
            cfg.raft_cfg.min_election_tick = min_election_tick;
        }
        if let Some(max_election_tick) = self.max_election_tick {
            cfg.raft_cfg.max_election_tick = max_election_tick;
        }
        if let Some(skip_bcast_commit) = self.skip_bcast_commit {
            cfg.raft_cfg.skip_bcast_commit = skip_bcast_commit;
        }
        if let Some(batch_append) = self.batch_append {
            cfg.raft_cfg.batch_append = batch_append;
        }
        if let Some(priority) = self.priority {
            cfg.raft_cfg.priority = priority;
        }
        if let Some(max_uncommitted_size) = self.max_uncommitted_size {
            cfg.raft_cfg.max_uncommitted_size = max_uncommitted_size;
        }
        if let Some(max_committed_size_per_ready) = self.max_committed_size_per_ready {
            cfg.raft_cfg.max_committed_size_per_ready = max_committed_size_per_ready;
        }
        cfg.raft_cfg.read_only_option = self.read_only_option;
        cfg
    }

    fn read_only_option_default() -> ReadOnlyOption {
        ReadOnlyOption::Safe
    }

    fn grpc_reuseaddr_default() -> bool {
        true
    }

    fn grpc_reuseport_default() -> bool {
        false
    }

    pub fn deserialize_read_only_option<'de, D>(
        deserializer: D,
    ) -> std::result::Result<ReadOnlyOption, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?.to_lowercase();
        match v.as_str() {
            "safe" => Ok(ReadOnlyOption::Safe),
            "leasebased" => Ok(ReadOnlyOption::LeaseBased),
            _ => Err(de::Error::missing_field("read_only_option")),
        }
    }

    #[inline]
    pub fn serialize_read_only_option<S>(rop: &ReadOnlyOption, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let rop_str = match rop {
            ReadOnlyOption::Safe => "safe",
            ReadOnlyOption::LeaseBased => "leasebased",
        };
        rop_str.serialize(s)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    Zstd,
    Lz4,
    Zlib,
    Snappy,
}
