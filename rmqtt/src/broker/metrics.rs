use std::sync::atomic::{AtomicUsize, Ordering};

use rmqtt_macros::Metrics;

#[derive(Serialize, Deserialize, Debug, Default, Metrics)]
pub struct Metrics {
    client_authenticate: AtomicUsize,
    client_auth_anonymous: AtomicUsize,
    client_connect: AtomicUsize,
    client_connack: AtomicUsize,
    client_connected: AtomicUsize,
    client_disconnected: AtomicUsize,
    client_subscribe_check_acl: AtomicUsize,
    client_publish_check_acl: AtomicUsize,
    client_subscribe: AtomicUsize,
    client_unsubscribe: AtomicUsize,

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

}

