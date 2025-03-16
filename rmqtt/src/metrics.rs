use std::sync::atomic::{AtomicUsize, Ordering};

use crate::macros::Metrics;

#[derive(Serialize, Deserialize, Debug, Default, Metrics)]
pub struct Metrics {
    client_authenticate: AtomicUsize,
    client_auth_anonymous: AtomicUsize,
    client_auth_anonymous_error: AtomicUsize,
    client_handshaking_timeout: AtomicUsize,
    client_connect: AtomicUsize,
    client_connack: AtomicUsize,
    client_connack_auth_error: AtomicUsize,
    client_connack_unavailable_error: AtomicUsize,
    client_connack_error: AtomicUsize,
    client_connected: AtomicUsize,
    client_disconnected: AtomicUsize,
    client_subscribe_check_acl: AtomicUsize,
    client_publish_check_acl: AtomicUsize,
    client_subscribe: AtomicUsize,
    client_unsubscribe: AtomicUsize,
    client_subscribe_error: AtomicUsize,
    client_subscribe_auth_error: AtomicUsize,
    client_publish_auth_error: AtomicUsize,
    client_publish_error: AtomicUsize,

    session_subscribed: AtomicUsize,
    session_unsubscribed: AtomicUsize,
    session_created: AtomicUsize,
    session_resumed: AtomicUsize,
    session_terminated: AtomicUsize,

    messages_publish: AtomicUsize,
    // messages_received: AtomicUsize,
    // messages_received_qos0: AtomicUsize,
    // messages_received_qos1: AtomicUsize,
    // messages_received_qos2: AtomicUsize,
    messages_delivered: AtomicUsize,
    // messages_forward: AtomicUsize,
    // messages_sent: AtomicUsize,
    messages_acked: AtomicUsize,
    messages_dropped: AtomicUsize,

    messages_publish_custom: AtomicUsize,
    messages_delivered_custom: AtomicUsize,
    messages_acked_custom: AtomicUsize,

    messages_publish_admin: AtomicUsize,
    messages_delivered_admin: AtomicUsize,
    messages_acked_admin: AtomicUsize,

    messages_publish_lastwill: AtomicUsize,
    messages_delivered_lastwill: AtomicUsize,
    messages_acked_lastwill: AtomicUsize,

    messages_publish_system: AtomicUsize,
    messages_delivered_system: AtomicUsize,
    messages_acked_system: AtomicUsize,

    messages_publish_bridge: AtomicUsize,
    messages_delivered_bridge: AtomicUsize,
    messages_acked_bridge: AtomicUsize,

    messages_delivered_retain: AtomicUsize,
    messages_acked_retain: AtomicUsize,

    messages_nonsubscribed: AtomicUsize,
    messages_nonsubscribed_custom: AtomicUsize,
    messages_nonsubscribed_admin: AtomicUsize,
    messages_nonsubscribed_lastwill: AtomicUsize,
    messages_nonsubscribed_system: AtomicUsize,
    messages_nonsubscribed_bridge: AtomicUsize,
}
