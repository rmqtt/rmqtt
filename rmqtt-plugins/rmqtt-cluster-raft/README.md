[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-cluster-raft

[![crates.io](https://img.shields.io/crates/v/rmqtt-cluster-raft.svg)](https://crates.io/crates/rmqtt-cluster-raft)

Raft consensus cluster plugin. Provides strongly consistent distributed clustering.

## Overview

Runs Raft in a dedicated OS thread with its own Tokio runtime. Supports configurable compression, health checks with auto-exit, and fine-grained Raft tuning.

## Usage

```toml
[dependencies]
rmqtt-cluster-raft = "0.21"
```

Requires `rmqtt` features: `plugin`, `grpc`, `stats`, `msgstore`.

```rust
rmqtt_cluster_raft::register(&scx, true, false).await?;
```

CLI example:

```bash
rmqttd --id 1 --plugins-default-startups "rmqtt-cluster-raft" \
  --node-grpc-addrs "1@10.0.0.1:5363" "2@10.0.0.2:5363" "3@10.0.0.3:5363" \
  --raft-peer-addrs "1@10.0.0.1:6003" "2@10.0.0.2:6003" "3@10.0.0.3:6003"
```

## Configuration

File: `rmqtt-cluster-raft.toml`

### General

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `worker_threads` | integer | `6` | Cluster Raft worker thread count |
| `message_type` | integer | `198` | gRPC message type |
| `try_lock_timeout` | string | `"10s"` | Handshake lock timeout |
| `task_exec_queue_workers` | integer | `500` | Task execution queue workers |
| `task_exec_queue_max` | integer | `100_000` | Task execution queue max capacity |
| `compression` | string | `"zstd"` | Snapshot compression algorithm |
| `leader_id` | integer | `0` | Specify a leader node ID. Intended for single-host pseudo-cluster dev/testing (0 = auto-select first node in config). Overridable via `--raft-leader-id` CLI flag |

Compression algorithms: `lz4_flex`, `lz4`, `zstd`, `snap`, `flate2`

### gRPC

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `node_grpc_addrs` | array of string | `["1@127.0.0.1:5363", ...]` | Cluster node gRPC addresses |
| `node_grpc_batch_size` | integer | `128` | Maximum messages sent in batch |
| `node_grpc_client_concurrency_limit` | integer | `128` | Client concurrent request limit |
| `node_grpc_client_timeout` | string | `"10s"` | Connect and send timeout |
| `node_grpc_circuit_breaker_enabled` | boolean | `true` | Enable gRPC circuit breaker |
| `node_grpc_circuit_failure_threshold` | integer | `10` | Consecutive failures before tripping to OPEN |
| `node_grpc_circuit_reset_timeout` | string | `"15s"` | Duration in OPEN before probing (HALF_OPEN) |
| `node_grpc_circuit_half_open_success_threshold` | integer | `3` | Consecutive probe successes to close the circuit |
| `laddr` | string | — | Raft cluster listening address (optional) |

### Raft Peers

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `raft_peer_addrs` | array of string | `["1@127.0.0.1:6003", ...]` | Raft peer addresses |

### Health Check

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `health.exit_on_node_unavailable` | boolean | `false` | Auto-exit on node unavailability |
| `health.exit_code` | integer | `-1` | Exit code on termination |
| `health.max_continuous_unavailable_count` | integer | `2` | Max consecutive failures before exit |
| `health.unavailable_check_interval` | string | `"2s"` | Unavailability check interval |

### Raft Tuning

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `raft.grpc_timeout` | string | `"6s"` | Raft gRPC timeout |
| `raft.grpc_concurrency_limit` | integer | `200` | Raft gRPC concurrency limit |
| `raft.grpc_breaker_threshold` | integer | `5` | Circuit breaker threshold |
| `raft.grpc_breaker_retry_interval` | string | `"2500ms"` | Circuit breaker retry interval |
| `raft.proposal_batch_size` | integer | `60` | Proposal batch size |
| `raft.proposal_batch_timeout` | string | `"200ms"` | Proposal batch timeout |
| `raft.snapshot_interval` | string | `"300s"` | Snapshot interval |
| `raft.heartbeat` | string | `"100ms"` | Heartbeat interval |
| `raft.election_tick` | integer | `10` | Election tick count |
| `raft.heartbeat_tick` | integer | `5` | Heartbeat tick count |
| `raft.max_size_per_msg` | integer | `0` | Max size per message (0 = unlimited) |
| `raft.max_inflight_msgs` | integer | `256` | Max in-flight messages |
| `raft.check_quorum` | boolean | `true` | Check quorum on election |
| `raft.pre_vote` | boolean | `true` | Enable pre-vote |
| `raft.min_election_tick` | integer | `0` | Min election tick (0 = use default) |
| `raft.max_election_tick` | integer | `0` | Max election tick (0 = use default) |
| `raft.read_only_option` | string | `"Safe"` | Read-only option (Safe, LeaseBased) |
| `raft.skip_bcast_commit` | boolean | `false` | Skip broadcast commit |
| `raft.batch_append` | boolean | `false` | Enable batch append |
| `raft.priority` | integer | `0` | Node priority for leader election |

## Dependencies

`rmqtt` (features: `plugin`, `grpc`, `stats`, `msgstore`), `rmqtt-raft`, `tokio`, `serde`

## License

MIT OR Apache-2.0
