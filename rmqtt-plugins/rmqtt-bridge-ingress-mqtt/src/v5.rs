//! MQTT v5 bridge client for ingress subscription.
//!
//! Connects to a remote MQTT v5 broker, subscribes to configured topics,
//! and forwards received messages into the local broker via an event system.

use std::cell::RefCell;
use std::net::{SocketAddr, ToSocketAddrs};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::anyhow;
use futures::{channel::mpsc, StreamExt};
use ntex::connect::rustls::TlsConnector;
use ntex::connect::Connector;
use ntex::time::Seconds;
use ntex::util::ByteString;
use ntex::util::Bytes;
use ntex::{service::fn_service, util::Ready};
use ntex_mqtt::error::SendPacketError;
use ntex_mqtt::v5::codec::SubscribeAckReason;
use ntex_mqtt::{self, v5};

use rmqtt::{
    codec::types::MQTT_LEVEL_5,
    types::{ClientId, NodeId, UserName},
    Result,
};

use crate::bridge::{
    build_tls_connector, BridgeClient, BridgePublish, Command, CommandMailbox, OnMessageEvent,
};
use crate::config::{Bridge, INACTIVE_REMINDER_INTERVAL, MAX_SUBSCRIBE_ATTEMPTS, MAX_SUBSCRIBE_BACKOFF};

enum MqttConnector {
    Tcp(v5::client::MqttConnector<String, Connector<String>>),
    Tls(v5::client::MqttConnector<String, TlsConnector<String>>),
}

impl MqttConnector {
    async fn connect(&self) -> Result<v5::client::Client> {
        Ok(match self {
            MqttConnector::Tcp(c) => c.connect().await.map_err(|e| anyhow!(e))?,
            MqttConnector::Tls(c) => c.connect().await.map_err(|e| anyhow!(e))?,
        })
    }
}

/// `true` when `reason` says the peer will not grant this filter on this
/// connection, so retrying it only delays the log line.
///
/// The remaining refusals — `UnspecifiedError`, `ImplementationSpecificError`,
/// `PacketIdentifierInUse` and `QuotaExceeded` — can clear on their own (a peer
/// still loading its rules, a quota that is raised again), which is why those are
/// retried on a bounded backoff.
fn is_permanent_refusal(reason: SubscribeAckReason) -> bool {
    matches!(
        reason,
        SubscribeAckReason::NotAuthorized
            | SubscribeAckReason::TopicFilterInvalid
            | SubscribeAckReason::SharedSubscriptionNotSupported
            | SubscribeAckReason::SubscriptionIdentifiersNotSupported
            | SubscribeAckReason::WildcardSubscriptionsNotSupported
    )
}

/// An MQTT v5 ingress client that subscribes to remote topics.
#[derive(Clone)]
pub struct Client {
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) server_addr: SocketAddr,
    pub(crate) entry_idx: usize,
    pub(crate) client_id: ClientId,
    pub(crate) username: UserName,
    closed: Rc<AtomicBool>,
    sink: Rc<RefCell<Option<v5::MqttSink>>>,
    on_message: OnMessageEvent,
}

impl Client {
    /// Returns `true` if the client connection has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn close(&self) -> bool {
        if let Some(sink) = self.sink.borrow().as_ref() {
            sink.close();
            self.closed.store(true, Ordering::SeqCst);
            true
        } else {
            false
        }
    }

    pub(crate) fn connect(
        cfg: Bridge,
        entry_idx: usize,
        node_id: NodeId,
        client_no: usize,
        on_message: OnMessageEvent,
    ) -> Result<CommandMailbox> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(10);

        let client_id =
            format!("{}:{}:ingress:{}:{}:{}", cfg.client_id_prefix, cfg.name, node_id, entry_idx, client_no);
        let username = cfg.username.clone().unwrap_or("undefined".into());

        let server_addr = cfg.server.addr.to_socket_addrs()?.next().ok_or_else(|| anyhow!("None"))?;

        let client = Self {
            cfg: Arc::new(cfg),
            server_addr,
            entry_idx,
            client_id: ClientId::from(client_id),
            username: UserName::from(username),
            // disconnected: Rc::new(AtomicBool::new(false)),
            closed: Rc::new(AtomicBool::new(false)),
            sink: Rc::new(RefCell::new(None)),
            on_message,
        };

        let mut builder = v5::client::MqttConnector::new(client.cfg.server.addr.clone())
            .client_id(ByteString::from(client.client_id.to_string()))
            .keep_alive(Seconds(client.cfg.keepalive.as_secs() as u16))
            .handshake_timeout(Seconds(client.cfg.connect_timeout.as_secs() as u16));

        if let Some(username) = client.cfg.username.as_ref() {
            builder = builder.username(ByteString::from(username.as_str()));
        }
        if let Some(password) = client.cfg.password.as_ref() {
            builder = builder.password(Bytes::from(password.clone()));
        }

        if client.cfg.mqtt_ver.level() == MQTT_LEVEL_5 {
            if client.cfg.v5.clean_start {
                builder = builder.clean_start()
            };

            builder = builder.max_receive(client.cfg.v5.receive_maximum);
            builder = builder.max_packet_size(client.cfg.v5.maximum_packet_size.as_u32());

            builder = builder.packet(|pkt| {
                pkt.session_expiry_interval_secs = client.cfg.v5.session_expiry_interval.as_secs() as u32;
                pkt.topic_alias_max = client.cfg.v5.topic_alias_maximum;
                pkt.last_will.clone_from(&client.cfg.v5.last_will)
            });

            if client.cfg.server.is_tls() {
                let builder = builder.connector(build_tls_connector(&client.cfg)?);
                ntex::rt::spawn(client.clone().start(MqttConnector::Tls(builder)));
            } else {
                ntex::rt::spawn(client.clone().start(MqttConnector::Tcp(builder)));
            }

            ntex::rt::spawn(client.clone().cmd_loop(cmd_rx));
        } else {
            log::error!("unreachable!(), mqtt_ver: {:?}", client.cfg.mqtt_ver);
            return Err(anyhow!("unreachable!()"));
        }

        Ok(CommandMailbox::new(client.client_id, cmd_tx))
    }

    async fn cmd_loop(self, mut cmd_rx: mpsc::Receiver<Command>) {
        while !self.is_closed() {
            match cmd_rx.next().await {
                None => break,
                Some(Command::Connect) => {
                    log::debug!("{} Command::Connect ...", self.client_id);
                }
                Some(Command::Close) => {
                    log::debug!("{} Command::Close ...", self.client_id);
                    self.close();
                    break;
                }
            }
        }
    }

    async fn start(self, builder: MqttConnector) {
        let client = self;
        let sleep_interval = client.cfg.reconnect_interval;
        loop {
            match builder.connect().await {
                Ok(c) => {
                    log::info!("{} Successfully connected to {:?}", client.client_id, client.cfg.server);

                    let sink = c.sink();
                    client.sink.replace(Some(sink.clone()));

                    //subscribes
                    if let Some(entry) = client.cfg.entries.get(client.entry_idx) {
                        let topic_filter = ByteString::from(entry.remote.topic.clone());
                        let qos = entry.remote.qos;
                        ntex::rt::spawn(client.clone().subscribe(
                            sink.clone(),
                            topic_filter,
                            qos,
                            ByteString::from(client.client_id.to_string()),
                        ));
                    }

                    //client event loop
                    client.clone().ev_loop(c).await;
                }
                Err(e) => {
                    log::warn!(
                        "{} Connect to {:?} fail, {:?}",
                        client.client_id,
                        client.cfg.server,
                        e.to_string()
                    );
                }
            }
            if client.is_closed() {
                break;
            } else {
                ntex::time::sleep(sleep_interval).await;
            }
        }
        log::info!("{} Exit 'rmqtt-bridge-ingress-mqtt' client", client.client_id);
    }

    /// Subscribes to `topic_filter`, retrying a transient refusal on a bounded
    /// backoff.
    ///
    /// MQTT 5 does say why a subscription was refused, and the reason decides the
    /// answer: `NotAuthorized`, `TopicFilterInvalid` and the three
    /// "…NotSupported" codes will not change on this connection, so retrying them
    /// only hides the problem (issue #540), while `QuotaExceeded`,
    /// `PacketIdentifierInUse`, `UnspecifiedError` and
    /// `ImplementationSpecificError` can clear on their own. Only the latter are
    /// retried — on an exponentially growing backoff, at most
    /// [`MAX_SUBSCRIBE_ATTEMPTS`] times per connection. Once given up on, the
    /// entry is [`remind_inactive`]d about until the connection goes away. A
    /// reconnect starts over.
    async fn subscribe(
        self,
        sink: v5::MqttSink,
        topic_filter: ByteString,
        qos: v5::QoS,
        client_id: ByteString,
    ) {
        let opts = v5::codec::SubscriptionOptions {
            qos,
            no_local: false,
            retain_as_published: false,
            retain_handling: v5::codec::RetainHandling::AtSubscribe,
        };
        let max_attempts = MAX_SUBSCRIBE_ATTEMPTS;
        let mut attempt = 1;
        let mut backoff = self.cfg.reconnect_interval;
        loop {
            match sink.subscribe(None).topic_filter(topic_filter.clone(), opts).send().await {
                Ok(ack) => {
                    let refused = ack.status.iter().copied().find(|reason| {
                        !matches!(
                            reason,
                            SubscribeAckReason::GrantedQos0
                                | SubscribeAckReason::GrantedQos1
                                | SubscribeAckReason::GrantedQos2
                        )
                    });
                    match refused {
                        None => {
                            log::info!("{client_id} Successfully subscribed to {topic_filter:?}",);
                            return;
                        }
                        Some(reason) if is_permanent_refusal(reason) => {
                            let why = format!(
                                "the peer refused the subscription with {reason:?}, so nothing \
                                 it should forward is delivered on this connection"
                            );
                            log::warn!(
                                "{client_id} Subscribe refused by the peer, topic_filter: \
                                 {topic_filter:?}, reason: {reason:?}; giving up on this connection"
                            );
                            log::error!(
                                "{client_id} entry {topic_filter:?} stays inactive until the bridge \
                                 reconnects: {why} (issue #540)"
                            );
                            self.remind_inactive(sink, topic_filter, why).await;
                            return;
                        }
                        Some(reason) if attempt >= max_attempts => {
                            let why =
                                format!("the peer answered SUBACK with {reason:?} on all {attempt} attempts");
                            log::error!(
                                "{client_id} entry {topic_filter:?} stays inactive until the bridge \
                                 reconnects: {why} (issue #540)"
                            );
                            self.remind_inactive(sink, topic_filter, why).await;
                            return;
                        }
                        Some(reason) => {
                            log::warn!(
                                "{client_id} Subscribe refused by the peer, topic_filter: \
                                 {topic_filter:?}, reason: {reason:?} (retryable); retrying in \
                                 {backoff:?} (attempt {attempt}/{max_attempts})"
                            );
                        }
                    }
                }
                Err(SendPacketError::Disconnected) => {
                    log::info!("{client_id} Subscribe error, Disconnected");
                    return;
                }
                Err(e) => {
                    if attempt >= max_attempts {
                        let why = format!("{attempt} subscribe attempts failed, last error: {e}");
                        log::error!(
                            "{client_id} entry {topic_filter:?} stays inactive until the bridge \
                             reconnects: {why} (issue #540)"
                        );
                        self.remind_inactive(sink, topic_filter, why).await;
                        return;
                    }
                    log::warn!(
                        "{client_id} Subscribe to {topic_filter:?} failed ({e}); retrying in \
                         {backoff:?} (attempt {attempt}/{max_attempts})"
                    );
                }
            }
            ntex::time::sleep(backoff).await;
            if self.is_closed() {
                log::info!(
                    "{client_id} Stopped retrying the subscription to {topic_filter:?}, the \
                     bridge is closing"
                );
                return;
            }
            attempt += 1;
            backoff = (backoff * 2).min(MAX_SUBSCRIBE_BACKOFF);
        }
    }

    /// After a subscription is given up on, repeats on every
    /// [`INACTIVE_REMINDER_INTERVAL`] that the entry is still inactive, for as
    /// long as THIS connection stays up: the one-time `error` at give-up is easy
    /// to miss in a log that has been rolling for hours, and nothing else
    /// reports the gap while the connection is healthy (issue #540). The
    /// reminders end when the connection drops (`sink.is_open()`) or the bridge
    /// closes; the reconnect then starts a fresh `subscribe`, which is also how
    /// a since-fixed peer ACL or quota takes effect.
    async fn remind_inactive(self, sink: v5::MqttSink, topic_filter: ByteString, why: String) {
        loop {
            ntex::time::sleep(INACTIVE_REMINDER_INTERVAL).await;
            if self.is_closed() || !sink.is_open() {
                return;
            }
            log::warn!(
                "{} entry {topic_filter:?} is STILL inactive: {why} (issue #540); it is retried \
                 when the bridge reconnects",
                self.client_id
            );
        }
    }

    async fn ev_loop(self, c: v5::client::Client) {
        if let Err(e) = c
            .start(fn_service(move |control: v5::client::Control<()>| match control {
                v5::client::Control::Publish(publish) => {
                    log::debug!("{} publish: {:?}", self.client_id, publish);
                    self.on_message.fire((
                        BridgeClient::V5(self.clone()),
                        Some(self.server_addr),
                        None,
                        BridgePublish::V5(publish.packet().clone()),
                    ));
                    let reason_code = v5::codec::PublishAckReason::Success;
                    Ready::Ok(publish.ack(reason_code))
                }
                v5::client::Control::Disconnect(msg) => {
                    log::info!("{} Server disconnecting: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v5::client::Control::Error(msg) => {
                    log::info!("{} Codec error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack(v5::codec::DisconnectReasonCode::NormalDisconnection))
                }
                v5::client::Control::ProtocolError(msg) => {
                    log::info!("{} Protocol error: {:?}", self.client_id, msg);
                    Ready::Ok(msg.ack())
                }
                v5::client::Control::PeerGone(msg) => {
                    log::info!("{} Peer closed connection: {:?}", self.client_id, msg.error());
                    Ready::Ok(msg.ack())
                }
                v5::client::Control::Closed(msg) => {
                    log::info!("{} Server closed connection", self.client_id);
                    Ready::Ok(msg.ack())
                }
            }))
            .await
        {
            log::error!("Start ev_loop error! {e}");
        }
    }
}

#[derive(Debug)]
struct Error;

impl std::convert::TryFrom<Error> for v5::PublishAck {
    type Error = Error;

    fn try_from(err: Error) -> std::result::Result<Self, Self::Error> {
        Err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::is_permanent_refusal;
    use ntex_mqtt::v5::codec::SubscribeAckReason;

    /// Every reason code a SUBACK can carry is pinned here.
    ///
    /// The classification decides whether an entry gives up at once or retries on
    /// a bounded backoff, so it must be a decision rather than a default: a reason
    /// added to the protocol later and forgotten in [`is_permanent_refusal`] lands
    /// on the retryable side, which wastes a few attempts but stays safe — the
    /// dangerous direction (calling a transient code permanent) is what the
    /// "may clear" arm below guards.
    #[test]
    fn test_refusal_classification() {
        // Not refusals at all: these are grants. They reach
        // `is_permanent_refusal` only by a caller's mistake.
        for reason in [
            SubscribeAckReason::GrantedQos0,
            SubscribeAckReason::GrantedQos1,
            SubscribeAckReason::GrantedQos2,
        ] {
            assert!(!is_permanent_refusal(reason), "{reason:?} is a grant, not a refusal");
        }

        // Permanent: no amount of waiting makes the peer grant THIS filter on
        // THIS connection — an ACL, a feature the peer does not implement.
        for reason in [
            SubscribeAckReason::NotAuthorized,
            SubscribeAckReason::TopicFilterInvalid,
            SubscribeAckReason::SharedSubscriptionNotSupported,
            SubscribeAckReason::SubscriptionIdentifiersNotSupported,
            SubscribeAckReason::WildcardSubscriptionsNotSupported,
        ] {
            assert!(is_permanent_refusal(reason), "{reason:?} will not clear on its own");
        }

        // Transient: rules still loading, a quota that gets raised, a packet id
        // still in flight. Retrying on a real backoff is the whole point, so
        // treating any of these as permanent would be the bug.
        for reason in [
            SubscribeAckReason::UnspecifiedError,
            SubscribeAckReason::ImplementationSpecificError,
            SubscribeAckReason::PacketIdentifierInUse,
            SubscribeAckReason::QuotaExceeded,
        ] {
            assert!(!is_permanent_refusal(reason), "{reason:?} may clear, so it must be retried");
        }
    }
}
