//! Egress bridge to remote MQTT brokers.
//!
//! Forwards local MQTT publish messages to remote MQTT brokers (v3.1.1 or v5).
//! Manages client connections, topic matching via a trie, and message translation
//! between local and remote MQTT protocol versions.

use std::borrow::Cow;
use std::collections::HashSet;
use std::convert::From as _;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ntex_mqtt::v3::codec::Publish as PublishV3;
use ntex_mqtt::v5::codec::Publish as PublishV5;

use ntex::connect::rustls::TlsConnector;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};

use anyhow::anyhow;
use bytestring::ByteString;
use futures::channel::mpsc;
use futures::SinkExt;
use tokio::sync::RwLock;

use rmqtt::{
    topic::is_valid_topic_name,
    trie::{TopicTree, VecToTopic},
    types::{ClientId, DashMap, From, NodeId, Publish, Topic},
    Result,
};

use rmqtt::codec::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use rmqtt::codec::v5::PublishProperties;

use crate::config::{Bridge, Entry, PluginConfig};
use crate::v4::Client as ClientV4;
use crate::v5::Client as ClientV5;

/// Commands for controlling a bridge client connection.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum Command {
    Connect,
    Publish(BridgePublish),
    Close,
}

/// Mailbox for sending commands to a bridge client.
#[derive(Clone)]
pub struct CommandMailbox {
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) client_id: ClientId,
    cmd_tx: mpsc::Sender<Command>,
}

impl CommandMailbox {
    /// Creates a new `CommandMailbox` with bridge config and channel sender.
    pub(crate) fn new(cfg: Arc<Bridge>, client_id: ClientId, cmd_tx: mpsc::Sender<Command>) -> Self {
        CommandMailbox { cfg, client_id, cmd_tx }
    }

    /// Sends a command to the bridge client.
    #[inline]
    pub(crate) async fn send(&self, cmd: Command) -> Result<()> {
        self.cmd_tx.clone().send(cmd).await.map_err(|e| anyhow!(e))?;
        Ok(())
    }

    /// Sends a `Close` command to stop the bridge client.
    #[inline]
    pub(crate) async fn stop(&mut self) -> Result<()> {
        self.send(Command::Close).await
    }
}

/// A publish message variant for MQTT v3 or v5.
#[derive(Debug)]
pub enum BridgePublish {
    V3(PublishV3),
    V5(PublishV5),
}

/// A named bridge instance identifier.
pub(crate) type BridgeName = ByteString;
type SourceKey = (BridgeName, EntryIndex);

type EntryIndex = usize;

type MqttVer = u8;

/// Manages bridge client connections and message routing.
///
/// Maintains a topic trie for matching inbound MQTT topics to remote bridge
/// entries, manages client mailboxes for each bridge instance, and handles
/// message translation between MQTT versions.
#[derive(Clone)]
pub(crate) struct BridgeManager {
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sinks: Arc<DashMap<SourceKey, Vec<CommandMailbox>>>,
    topics: Arc<RwLock<TopicTree<(BridgeName, EntryIndex, MqttVer)>>>,
    /// Entries that have already been reported for mapping a message to an
    /// illegal MQTT topic name.
    ///
    /// A misconfigured entry can be hit by every matching message — in issue
    /// #451 that turned into an endless reconnect-and-resend loop — so only the
    /// first occurrence per `(bridge, entry)` is logged at `warn`; afterwards
    /// the same condition is demoted to `debug` to keep the log readable.
    warned: Arc<DashMap<SourceKey, ()>>,
}

impl BridgeManager {
    /// Creates a new `BridgeManager` with the given node ID and configuration.
    pub fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self {
            node_id,
            cfg,
            sinks: Arc::new(DashMap::default()),
            topics: Arc::new(RwLock::new(TopicTree::default())),
            warned: Arc::new(DashMap::default()),
        }
    }

    /// Starts all bridge entries with automatic retry on failure.
    pub async fn start(&mut self) {
        while let Err(e) = self._start().await {
            log::error!("start bridge-egress-mqtt error, {e}");
            self.stop().await;
            tokio::time::sleep(Duration::from_millis(3000)).await;
        }
    }

    async fn _start(&mut self) -> Result<()> {
        let mut topics = self.topics.write().await;
        let bridges = self.cfg.read().await.bridges.clone();
        let mut bridge_names: HashSet<&str> = HashSet::default();
        for b_cfg in &bridges {
            if !b_cfg.enable {
                continue;
            }
            if bridge_names.contains(&b_cfg.name as &str) {
                return Err(anyhow!(format!("The bridge name already exists! {:?}", b_cfg.name)));
            }

            bridge_names.insert(&b_cfg.name);
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                log::debug!("entry.local.topic_filter: {}", entry.local.topic_filter);
                topics.insert(
                    &Topic::from_str(entry.local.topic_filter.as_str())?,
                    (b_cfg.name.clone(), entry_idx, b_cfg.mqtt_ver.level()),
                );
                for client_no in 0..b_cfg.concurrent_client_limit {
                    match b_cfg.mqtt_ver.level() {
                        MQTT_LEVEL_311 => {
                            let mailbox =
                                ClientV4::connect(b_cfg.clone(), entry_idx, self.node_id, client_no)?;
                            self.sinks.entry((b_cfg.name.clone(), entry_idx)).or_default().push(mailbox);
                        }
                        MQTT_LEVEL_5 => {
                            let mailbox =
                                ClientV5::connect(b_cfg.clone(), entry_idx, self.node_id, client_no)?;
                            self.sinks.entry((b_cfg.name.clone(), entry_idx)).or_default().push(mailbox);
                        }
                        MQTT_LEVEL_31 => {
                            log::warn!("Connection to MQTT 3.1 broker not implemented!")
                        }
                        _ => {
                            log::error!("Wrong MQTT version, {}", b_cfg.mqtt_ver.level())
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Stops all bridge clients and clears the sink map.
    pub async fn stop(&mut self) {
        for mut entry in &mut self.sinks.iter_mut() {
            let ((bridge_name, entry_idx), mailboxs) = entry.pair_mut();
            for (client_no, mailbox) in mailboxs.iter_mut().enumerate() {
                log::debug!(
                    "stop bridge_name: {bridge_name:?}, entry_idx: {entry_idx:?}, client_no: {client_no:?}"
                );
                if let Err(e) = mailbox.stop().await {
                    log::error!(
                    "stop BridgeMqttIngressPlugin error, bridge_name: {bridge_name}, entry_idx: {entry_idx}, client_no: {client_no}, {e}"
                );
                }
            }
        }
        self.sinks.clear();
    }

    /// Returns a reference to the internal mailbox (client sink) map.
    pub(crate) fn sinks(&self) -> &DashMap<SourceKey, Vec<CommandMailbox>> {
        &self.sinks
    }

    /// Routes an MQTT publish message to matching remote bridge clients.
    ///
    /// Matches the publish topic against the topic trie, selects a client
    /// via random distribution, applies topic skip-levels, and translates
    /// the message to the target MQTT version (v3 or v5).
    #[inline]
    pub(crate) async fn send(&self, _f: &From, p: &Publish) -> Result<()> {
        let topic: Topic = Topic::from_str(&p.topic)?;
        // `p.topic` is a `ByteString` whose `AsRef` is ambiguous without a
        // target type; bind the text once for the diagnostics below.
        let local_topic: &str = p.topic.as_ref();
        let rnd = rand::random::<u64>() as usize;
        for (topic_filter, bridge_infos) in { self.topics.read().await.matches(&topic) }.iter() {
            let topic_filter = topic_filter.to_topic_filter();
            log::debug!("topic_filter: {topic_filter:?}");
            log::debug!("bridge_infos: {bridge_infos:?}");
            for (name, entry_idx, mqtt_ver) in bridge_infos {
                if let Some(mailboxs) = self.sinks.get(&(name.clone(), *entry_idx)) {
                    let client_no = rnd % mailboxs.len();
                    if let Some(mailbox) = mailboxs.get(client_no) {
                        let entry: &Entry = if let Some(entry) = mailbox.cfg.entries.get(*entry_idx) {
                            entry
                        } else {
                            log::error!("unreachable!(), entry_idx: {}", *entry_idx);
                            continue;
                        };
                        let new_local_topic: Cow<str> = if entry.remote.skip_levels > 0 {
                            Cow::Owned(topic.to_string_skip(entry.remote.skip_levels))
                        } else {
                            Cow::Borrowed(p.topic.as_ref())
                        };
                        log::debug!(
                            "new_local_topic: {new_local_topic}, skip_levels: {}",
                            entry.remote.skip_levels
                        );
                        // `remote.topic` is a template that is filled from runtime data,
                        // so a configuration that is legal on its own can still collapse
                        // to an illegal topic NAME — an empty one, for instance, once
                        // `skip_levels` reaches the local level count. Such a name is not
                        // something a peer accepts: mosquitto answers it with
                        // `disconnected: malformed packet.`, the bridge reconnects and
                        // sends it again — the loop of issue #451. Drop the message
                        // locally instead of putting it on the wire, and report it once.
                        let remote_topic = entry.remote.make_topic(new_local_topic.as_ref());
                        if !is_valid_topic_name(&*remote_topic) {
                            if self.warned.insert((name.clone(), *entry_idx), ()).is_none() {
                                log::warn!(
                                    "bridge {name}, entry {entry_idx}: dropped a publish mapped to \
                                     the illegal MQTT topic name {remote_topic:?} (local topic \
                                     {local_topic:?}, skip_levels = {skip}) — a topic name must not \
                                     be empty and must not contain the wildcards `+` or `#` \
                                     (MQTT 5.0 §4.7.3 [MQTT-4.7.3-1], [MQTT-3.3.2-2]); fix \
                                     `remote.topic` and/or `skip_levels` for this entry — repeats \
                                     from this entry are logged at debug level",
                                    skip = entry.remote.skip_levels,
                                );
                            } else {
                                log::debug!(
                                    "bridge {name}, entry {entry_idx}: dropped a publish mapped to \
                                     the illegal MQTT topic name {remote_topic:?} (local topic \
                                     {local_topic:?})",
                                );
                            }
                            continue;
                        }
                        match *mqtt_ver {
                            MQTT_LEVEL_311 => {
                                if let Err(e) = mailbox
                                    .send(Command::Publish(BridgePublish::V3(self.to_v3_publish(
                                        entry,
                                        p,
                                        remote_topic,
                                    ))))
                                    .await
                                {
                                    log::warn!("{e}");
                                }
                            }
                            MQTT_LEVEL_5 => {
                                if let Err(e) = mailbox
                                    .send(Command::Publish(BridgePublish::V5(self.to_v5_publish(
                                        entry,
                                        p,
                                        remote_topic,
                                    ))))
                                    .await
                                {
                                    log::warn!("{e}");
                                }
                            }
                            MQTT_LEVEL_31 => {
                                log::warn!("Connection to MQTT 3.1 broker not implemented!")
                            }
                            _ => {
                                log::error!("Wrong MQTT version, {}", *mqtt_ver)
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Translates a local publish into the v3.1.1 wire form.
    ///
    /// `remote_topic` is the already-mapped, already-validated remote topic
    /// name; `send()` builds it once so the same string is both checked and
    /// sent.
    #[inline]
    fn to_v3_publish(
        &self,
        cfg_entry: &Entry,
        p: &Publish,
        remote_topic: ntex::util::ByteString,
    ) -> PublishV3 {
        PublishV3 {
            dup: false,
            retain: cfg_entry.remote.make_retain(p.retain),
            qos: cfg_entry.remote.make_qos(p.qos),
            topic: remote_topic,
            packet_id: None,
            payload: ntex::util::Bytes::from(p.payload.to_vec()), //@TODO ...
        }
    }

    /// Translates a local publish into the v5 wire form.
    ///
    /// See `to_v3_publish` for the `remote_topic` contract.
    #[inline]
    fn to_v5_publish(
        &self,
        cfg_entry: &Entry,
        p: &Publish,
        remote_topic: ntex::util::ByteString,
    ) -> PublishV5 {
        PublishV5 {
            dup: false,
            retain: cfg_entry.remote.make_retain(p.retain),
            qos: cfg_entry.remote.make_qos(p.qos),
            topic: remote_topic,
            packet_id: None,
            payload: ntex::util::Bytes::from(p.payload.to_vec()), //@TODO ...
            properties: p.properties.as_ref().map(to_properties).unwrap_or_default(),
        }
    }
}

#[inline]
fn to_properties(props: &PublishProperties) -> ntex_mqtt::v5::codec::PublishProperties {
    let user_properties: ntex_mqtt::v5::codec::UserProperties = props
        .user_properties
        .iter()
        .map(|(k, v)| {
            (ntex::util::ByteString::from(k.to_string()), ntex::util::ByteString::from(v.to_string()))
        })
        .collect();
    ntex_mqtt::v5::codec::PublishProperties {
        // Topic Alias is per-connection state (MQTT 5.0 §3.3.2.3.4): the alias
        // the local connection agreed on means nothing on the remote one, so
        // forwarding it would make the peer resolve the wrong topic (or reject
        // the packet). Drop it and let the peer see the full topic name.
        topic_alias: None,
        correlation_data: props.correlation_data.as_ref().map(|data| ntex::util::Bytes::from(data.to_vec())),
        message_expiry_interval: props.message_expiry_interval,
        content_type: props.content_type.as_ref().map(|data| ntex::util::ByteString::from(data.to_string())),
        user_properties,
        is_utf8_payload: props.is_utf8_payload,
        response_topic: props
            .response_topic
            .as_ref()
            .map(|data| ntex::util::ByteString::from(data.to_string())),
        // Subscription Identifier is a Server-to-Client property (MQTT 5.0
        // §3.3.2.3.8): the ids carried here belong to the bridge's subscription
        // on the *local* broker (filled in when it is delivered, see
        // rmqtt/src/shared.rs) and are illegal on the client-to-server PUBLISH
        // sent to the remote broker.
        subscription_ids: Vec::new(),
    }
}

/// Builds a TLS connector from bridge configuration.
///
/// Loads system root certificates, optionally adds a custom root CA,
/// and configures client certificate authentication if provided.
pub(crate) fn build_tls_connector(cfg: &super::config::Bridge) -> Result<TlsConnector<String>> {
    let mut root_store = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.into() };

    if let Some(c) = &cfg.root_cert {
        root_store.add_parsable_certificates(
            CertificateDer::pem_file_iter(c)
                .map_err(|e| anyhow!(e))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| anyhow!(e))?,
        );
    }

    let config = ClientConfig::builder().with_root_certificates(root_store);
    let config = if let (Some(client_key), Some(client_cert)) = (&cfg.client_key, &cfg.client_cert) {
        let c_certs = CertificateDer::pem_file_iter(client_cert)
            .map_err(|e| anyhow!(e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| anyhow!(e))?;
        let c_key = rustls::pki_types::PrivateKeyDer::from_pem_file(client_key).map_err(|e| anyhow!(e))?;
        config.with_client_auth_cert(c_certs, c_key).map_err(|e| anyhow!(e))?
    } else {
        config.with_no_client_auth()
    };
    Ok(TlsConnector::new(config))
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU16, NonZeroU32};

    use super::to_properties;
    use bytestring::ByteString;
    use rmqtt::codec::v5::PublishProperties;

    /// A property block must lose the two connection-scoped / wrong-direction
    /// properties while every other property survives the hop.
    #[test]
    fn to_properties_strips_alias_and_subscription_ids() {
        let src = PublishProperties {
            topic_alias: NonZeroU16::new(3),
            subscription_ids: vec![NonZeroU32::new(9).expect("9 != 0")],
            user_properties: vec![(ByteString::from("x-marker"), ByteString::from("keep"))],
            content_type: Some(ByteString::from("text/plain")),
            is_utf8_payload: true,
            ..Default::default()
        };

        let out = to_properties(&src);

        assert_eq!(
            out.topic_alias, None,
            "Topic Alias is per-connection (MQTT 5.0 §3.3.2.3.4) and must not cross to the peer"
        );
        assert!(
            out.subscription_ids.is_empty(),
            "Subscription Identifier is Server-to-Client (MQTT 5.0 §3.3.2.3.8) and is illegal on \
             the client-to-server PUBLISH the bridge sends"
        );
        // Control arm: the rest of the property block must still be forwarded,
        // otherwise "both are absent" would also hold if it were dropped whole.
        assert_eq!(out.user_properties.len(), 1);
        assert_eq!(&*out.user_properties[0].0, "x-marker");
        assert_eq!(&*out.user_properties[0].1, "keep");
        assert_eq!(out.content_type.as_deref(), Some("text/plain"));
        assert!(out.is_utf8_payload);
    }
}
