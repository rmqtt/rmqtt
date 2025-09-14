//! rmqtt-plugins
//!
//! A collection of plugins for the rmqtt MQTT broker, providing functionality
//! for various features such as authentication, message retention, HTTP APIs,
//! and more. Each plugin can be conditionally included based on feature flags,
//! allowing for modularity and customization. The plugins are categorized into
//! core, bridge, storage, utility, and cluster plugins, making it easy to extend
//! the rmqtt functionality with different backend systems or protocols.
//
//! The following categories of plugins are available:
//! - **Core Plugins**: Fundamental plugins for core functionality such as ACL,
//!   retention, and HTTP API support.
//! - **Bridge Plugins**: Plugins to integrate with external message brokers like
//!   Kafka, Pulsar, NATS, and more.
//! - **Storage Plugins**: Plugins for message and session storage with support
//!   for different backends like Redis and SLED.
//! - **Utility Plugins**: Additional utility features such as system topics,
//!   topic rewrites, and web hooks.
//! - **Cluster Plugins**: Support for clustering features such as Raft and
//!   broadcast modes for distributed setups.

#![deny(missing_docs)]

// ---- Core Plugins ----
#[cfg(feature = "acl")]
pub use rmqtt_acl as acl;

#[cfg(any(
    feature = "retainer",
    feature = "retainer-ram",
    feature = "retainer-sled",
    feature = "retainer-redis"
))]
pub use rmqtt_retainer as retainer;

#[cfg(feature = "http-api")]
pub use rmqtt_http_api as http_api;

#[cfg(feature = "counter")]
pub use rmqtt_counter as counter;

#[cfg(feature = "auth-http")]
pub use rmqtt_auth_http as auth_http;

#[cfg(feature = "auth-jwt")]
pub use rmqtt_auth_jwt as auth_jwt;

#[cfg(feature = "auto-subscription")]
pub use rmqtt_auto_subscription as auto_subscription;

// ---- Bridge Plugins ----
#[cfg(feature = "bridge-egress-kafka")]
pub use rmqtt_bridge_egress_kafka as bridge_egress_kafka;

#[cfg(feature = "bridge-ingress-kafka")]
pub use rmqtt_bridge_ingress_kafka as bridge_ingress_kafka;

#[cfg(feature = "bridge-egress-mqtt")]
pub use rmqtt_bridge_egress_mqtt as bridge_egress_mqtt;

#[cfg(feature = "bridge-ingress-mqtt")]
pub use rmqtt_bridge_ingress_mqtt as bridge_ingress_mqtt;

#[cfg(feature = "bridge-egress-pulsar")]
pub use rmqtt_bridge_egress_pulsar as bridge_egress_pulsar;

#[cfg(feature = "bridge-ingress-pulsar")]
pub use rmqtt_bridge_ingress_pulsar as bridge_ingress_pulsar;

#[cfg(feature = "bridge-egress-nats")]
pub use rmqtt_bridge_egress_nats as bridge_egress_nats;

#[cfg(feature = "bridge-egress-reductstore")]
pub use rmqtt_bridge_egress_reductstore as bridge_egress_reductstore;

// ---- Storage Plugins ----
#[cfg(any(
    feature = "message-storage",
    feature = "message-storage-ram",
    feature = "message-storage-redis",
    feature = "message-storage-redis-cluster"
))]
pub use rmqtt_message_storage as message_storage;

#[cfg(any(
    feature = "session-storage",
    feature = "session-storage-sled",
    feature = "session-storage-redis",
    feature = "session-storage-redis-cluster"
))]
pub use rmqtt_session_storage as session_storage;

// ---- Utility Plugins ----
#[cfg(feature = "sys-topic")]
pub use rmqtt_sys_topic as sys_topic;

#[cfg(feature = "topic-rewrite")]
pub use rmqtt_topic_rewrite as topic_rewrite;

#[cfg(feature = "web-hook")]
pub use rmqtt_web_hook as web_hook;

#[cfg(feature = "p2p-messaging")]
pub use rmqtt_p2p_messaging as p2p_messaging;

// ---- Cluster Plugins ----
#[cfg(feature = "cluster-raft")]
pub use rmqtt_cluster_raft as cluster_raft;

#[cfg(feature = "cluster-broadcast")]
pub use rmqtt_cluster_broadcast as cluster_broadcast;
