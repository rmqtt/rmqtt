//! `$SYS` announcements of bans, refusals and manual unbans.
//!
//! The second notification channel — the `client_connack` web-hook event —
//! needs no code here: a refusal already raises that hook, and the event body
//! carries the ClientId, the username and the source address. What it *cannot*
//! carry is the fact that the refusal was a ban rather than, say, a bad
//! password, so the reason code the plugin answers with is matched to the
//! `conn_ack` wording by `rmqtt-codec` (`ConnectAckReasonV5::reason`). This
//! module covers the operators who would rather subscribe to a topic than run
//! an HTTP endpoint.

use std::convert::From as _;
use std::time::Duration;

use bytes::Bytes;
use chrono::TimeZone as _;
use serde_json::json;

use rmqtt::{
    codec::v5::PublishProperties,
    context::ServerContext,
    flapping::Dimension,
    session::SessionState,
    types::{ClientId, CodecPublish, ConnectInfo, From, Id, NodeId, Publish, QoS, TopicName, UserName},
    utils::timestamp_millis,
};

use crate::config::Policy;

/// Substitutes the `{node}` placeholder of a topic template.
pub(crate) fn resolve_topic(template: &str, node_id: NodeId) -> String {
    template.replace("{node}", &node_id.to_string())
}

/// Formats a millisecond timestamp the way the rest of the broker's `$SYS`
/// messages do, so a subscriber can compare them as strings.
pub(crate) fn fmt_time(ms: u64) -> String {
    chrono::Local
        .timestamp_millis_opt(ms as i64)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

/// The body of a ban announcement.
///
/// `clientid`, `username` and `ipaddress` are repeated from the triggering
/// attempt because they are what a business system keys on: the ban itself may
/// be keyed by any of the three dimensions, and an operator reading the topic
/// wants to know who was behind it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn ban_payload(
    node_id: NodeId,
    dimension: Dimension,
    key: &str,
    ci: &ConnectInfo,
    count: usize,
    banned_at: u64,
    banned_until: u64,
    policy: &Policy,
) -> serde_json::Value {
    json!({
        "node": node_id,
        "event": "banned",
        "dimension": dimension.as_str(),
        "key": key,
        "clientid": ci.client_id(),
        "username": ci.username(),
        "ipaddress": ci.id().remote_addr,
        "listener_id": ci.id().lid(),
        "proto_ver": ci.proto_ver(),
        "count": count,
        "window_time_secs": policy.window_time.as_secs(),
        "ban_time_secs": policy.ban_time.as_secs(),
        "banned_at": fmt_time(banned_at),
        "banned_until": fmt_time(banned_until),
    })
}

/// The body of a refusal announcement (`notify_on_every_refusal`).
pub(crate) fn refused_payload(
    node_id: NodeId,
    dimension: Dimension,
    key: &str,
    ci: &ConnectInfo,
    now: u64,
) -> serde_json::Value {
    json!({
        "node": node_id,
        "event": "refused",
        "dimension": dimension.as_str(),
        "key": key,
        "clientid": ci.client_id(),
        "username": ci.username(),
        "ipaddress": ci.id().remote_addr,
        "listener_id": ci.id().lid(),
        "proto_ver": ci.proto_ver(),
        "time": fmt_time(now),
    })
}

/// The body of a manual-unban announcement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn unban_payload(
    node_id: NodeId,
    dimension: Dimension,
    key: &str,
    count: usize,
    banned_at: u64,
    banned_until: u64,
    last_clientid: Option<&str>,
    last_ipaddress: Option<&str>,
) -> serde_json::Value {
    json!({
        "node": node_id,
        "event": "unbanned",
        "reason": "manual",
        "dimension": dimension.as_str(),
        "key": key,
        "count": count,
        "clientid": last_clientid,
        "ipaddress": last_ipaddress,
        "banned_at": fmt_time(banned_at),
        "banned_until": fmt_time(banned_until),
        "unbanned_at": fmt_time(crate::now_ms()),
    })
}

/// Publishes `$SYS` messages without blocking its caller.
pub(crate) struct Notifier {
    scx: ServerContext,
    node_id: NodeId,
}

impl Notifier {
    pub(crate) fn new(scx: &ServerContext) -> Self {
        Self { scx: scx.clone(), node_id: scx.node.id() }
    }

    /// Publishes one message, detached.
    ///
    /// The connection path calls this while it is holding a refusal in its
    /// hand: the CONNACK cannot wait for a route lookup, so the publish is
    /// spawned. When the runtime is already gone the message is dropped rather
    /// than blocking a shutdown.
    pub(crate) fn publish(&self, topic: String, qos: QoS, expiry: Duration, payload: serde_json::Value) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            log::debug!("flapping: no async runtime available, dropping the $SYS notice on {topic}");
            return;
        };
        let scx = self.scx.clone();
        let node_id = self.node_id;
        handle.spawn(async move { sys_publish(scx, node_id, topic, qos, payload, expiry).await });
    }
}

/// Publishes one payload on a system topic.
///
/// This mirrors `rmqtt-sys-topic`'s publisher: the message is attributed to the
/// internal `system` client, run through the publish hook so that any other
/// plugin can see (and rewrite) it, and forwarded through the same path as a
/// client publish.
async fn sys_publish(
    scx: ServerContext,
    nodeid: NodeId,
    topic: String,
    qos: QoS,
    payload: serde_json::Value,
    message_expiry_interval: Duration,
) {
    let payload = match serde_json::to_string(&payload) {
        Ok(payload) => payload,
        Err(e) => {
            log::error!("flapping: cannot serialize the $SYS payload, {e}");
            return;
        }
    };

    let from = From::from_system(Id::new(
        nodeid,
        0,
        None,
        None,
        ClientId::from_static("system"),
        Some(UserName::from("system")),
    ));

    let p = CodecPublish {
        dup: false,
        retain: false,
        qos,
        topic: TopicName::from(topic),
        packet_id: None,
        payload: Bytes::from(payload),
        properties: Some(PublishProperties::default()),
    };
    let p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());

    // Hook, message_publish
    let p = scx.extends.hook_mgr().message_publish(None, from.clone(), &p).await.unwrap_or(p);

    let storage_available = scx.extends.message_mgr().await.enable();

    if let Err(e) =
        SessionState::forwards(&scx, from, p, storage_available, Some(message_expiry_interval)).await
    {
        log::warn!("flapping: cannot forward the $SYS notice, {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_node_placeholder_is_substituted() {
        assert_eq!(resolve_topic("$SYS/brokers/{node}/flapping/banned", 7), "$SYS/brokers/7/flapping/banned");
        assert_eq!(resolve_topic("$SYS/x", 7), "$SYS/x");
    }

    #[test]
    fn timestamps_are_formatted_and_sort_chronologically() {
        let early = fmt_time(1_700_000_000_000);
        let late = fmt_time(1_700_000_060_000);
        assert_eq!(early.len(), 23);
        assert!(early < late, "{early} should sort before {late}");
    }
}
