//! Delayed publish plugin for RMQTT.
//!
//! Implements MQTT delayed message delivery (`$delayed/<interval>/<topic>`)
//! with an in-memory priority queue:
//!
//! - Parses `$delayed/<interval>/<topic>` topics on the session publish path.
//! - Schedules messages in a per-node `BinaryHeap` (trigger time ordered).
//! - A background task forwards expired messages every 500 ms.
//! - Enforces the `publish_max` limit (overflow behavior is controlled by
//!   `publish_immediate`).
//!
//! # Enablement
//!
//! Loading this plugin enables the feature (`/features` reports
//! `delayed: true`); unloading it restores the core placeholder. On unload
//! the pending messages are flushed according to `publish_immediate`:
//! forwarded immediately (`true`) or dropped through the `message_dropped`
//! hook (`false`). Messages are in-memory only and are lost on node restart.
//!
//! # Related configuration
//!
//! File: `rmqtt-delayed.toml` (in the plugin config directory).
//!
//! - `publish_max` — maximum number of pending delayed messages.
//! - `publish_immediate` — forward immediately instead of dropping when the
//!   limit is reached.
//!
#![deny(unsafe_code)]

mod config;

use std::collections::BinaryHeap;
use std::ops::DerefMut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    session::SessionState,
    types::{
        DelayedPublish, DelayedPublishDetail, DelayedPublishInfo, From, Publish, Reason, TimestampMillis,
    },
    Result,
};

use config::PluginConfig;

register!(DelayedPlugin::new);

#[derive(Plugin)]
struct DelayedPlugin {
    scx: ServerContext,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl DelayedPlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(scx.plugins.read_config_default::<PluginConfig>(&name)?));
        log::info!("{} new, config: {:?}", name, cfg);
        Ok(Self { scx, cfg })
    }
}

#[async_trait]
impl Plugin for DelayedPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        Ok(())
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok, {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        *self.scx.extends.delayed_sender_mut().await =
            Box::new(MemDelayedSender::new(self.scx.clone(), self.cfg.clone()));
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        let pending = self.scx.extends.delayed_sender().await.len().await;
        log::info!("{} stop, pending delayed message(s): {pending}", self.name());
        *self.scx.extends.delayed_sender_mut().await = Box::new(rmqtt::delayed::DefaultDelayedSender::new());
        Ok(true)
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        serde_json::json!({
            "pending": self.scx.extends.delayed_sender().await.len().await,
        })
    }
}

/// In-memory delayed message sender.
///
/// Holds pending messages in a trigger-time ordered `BinaryHeap` and forwards
/// expired messages through a background task (500 ms tick). The background
/// task exits (within one tick) once every clone of the sender has been
/// dropped, so replacing the sender (e.g. on plugin stop) does not leak the
/// task or the retained `ServerContext`.
///
/// On exit, pending messages receive a final handling according to
/// `publish_immediate` (see [`Self::flush_on_exit`]): forwarded immediately
/// (`true`) or dropped through the `message_dropped` hook (`false`).
///
/// The capacity limit (`publish_max`) and the overflow behavior
/// (`publish_immediate`) are read from the shared, hot-reloadable
/// [`PluginConfig`]. When the limit is reached and `publish_immediate` is
/// `false`, the message is dropped here and the `message_dropped` hook fires
/// with `Reason::DelayedPublishRefused`.
#[derive(Clone)]
pub struct MemDelayedSender {
    scx: Option<ServerContext>,
    cfg: Arc<RwLock<PluginConfig>>,
    msgs: Arc<RwLock<BinaryHeap<DelayedPublish>>>,
    /// Set to `true` on drop; the background task checks it every tick and
    /// exits once it observes `true`.
    stopped: Arc<AtomicBool>,
}

impl Drop for MemDelayedSender {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
    }
}

impl MemDelayedSender {
    /// Creates a new sender and starts the background expiration task.
    #[inline]
    pub fn new(scx: ServerContext, cfg: Arc<RwLock<PluginConfig>>) -> MemDelayedSender {
        Self {
            scx: Some(scx),
            cfg,
            msgs: Arc::new(RwLock::new(BinaryHeap::default())),
            stopped: Arc::new(AtomicBool::new(false)),
        }
        .start()
    }

    fn start(self) -> Self {
        let s = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if s.stopped.load(Ordering::Acquire) {
                    Self::flush_on_exit(&s).await;
                    break;
                }
                loop {
                    let is_expired =
                        if let Some(is_expired) = s.msgs.read().await.peek().map(|p| p.is_expired()) {
                            is_expired
                        } else {
                            break;
                        };
                    if is_expired {
                        if let Some(dp) = s.msgs.write().await.pop() {
                            log::debug!("pop {:?} {:?}", dp.expired_time, dp.publish.topic);
                            if let Some(scx) = &s.scx {
                                Self::send(scx, dp).await;
                            }
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
            log::info!("delayed background task exited");
        });
        self
    }

    #[inline]
    async fn send(scx: &ServerContext, mut dp: DelayedPublish) {
        dp.publish.delay_interval = None;
        // `delay_interval` is cleared first, so `forwards` falls through to the
        // plain routing path (the same as the crate-internal `inner_forwards`).
        if let Err(e) = SessionState::forwards(
            scx,
            dp.from,
            dp.publish,
            dp.message_storage_available,
            dp.message_expiry_interval,
        )
        .await
        {
            log::warn!("delayed forwards error, {e}");
        }
    }

    /// Final handling of the pending messages when the background task exits
    /// (i.e. every clone of the sender has been dropped, e.g. on plugin stop):
    ///
    /// - `publish_immediate = true`: all pending messages are forwarded
    ///   immediately (trigger-time ordered, oldest first).
    /// - `publish_immediate = false`: each pending message is dropped through
    ///   the `message_dropped` hook with `Reason::DelayedPublishRefused`.
    async fn flush_on_exit(s: &MemDelayedSender) {
        let Some(scx) = &s.scx else {
            // No ServerContext (test-only construction): just drain the heap.
            s.msgs.write().await.clear();
            return;
        };
        let immediate = s.cfg.read().await.publish_immediate;
        let mut msgs = s.msgs.write().await;
        let pending = msgs.len();
        if pending == 0 {
            return;
        }
        if immediate {
            log::info!("delayed sender stopped, forwarding {pending} pending message(s) immediately");
            while let Some(dp) = msgs.pop() {
                Self::send(scx, dp).await;
            }
        } else {
            log::warn!("delayed sender stopped, dropping {pending} pending message(s)");
            while let Some(dp) = msgs.pop() {
                scx.extends
                    .hook_mgr()
                    .message_dropped(None, dp.from, dp.publish, Reason::DelayedPublishRefused)
                    .await;
            }
        }
    }

    #[cfg(test)]
    fn with_msgs(msgs: Vec<DelayedPublish>) -> Self {
        // list()/find() never touch `scx`; tests construct the sender without
        // a ServerContext and without spawning the background task.
        Self {
            scx: None,
            cfg: Arc::new(RwLock::new(PluginConfig::default())),
            msgs: Arc::new(RwLock::new(BinaryHeap::from(msgs))),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait]
impl rmqtt::delayed::DelayedSender for MemDelayedSender {
    #[inline]
    fn enable(&self) -> bool {
        true
    }

    #[inline]
    fn parse(&self, mut publish: Publish) -> Result<Publish> {
        let items = publish.topic.splitn(3, '/').collect::<Vec<_>>();
        if let (Some(&"$delayed"), Some(delay_interval), Some(topic)) =
            (items.first(), items.get(1), items.get(2))
        {
            let topic = rmqtt::types::TopicName::from(*topic);
            let interval_s = delay_interval.parse().map_err(|e| {
                anyhow!(format!(
                    "the delay time of $delayed must be an integer, topic: {}, {}",
                    publish.topic, e
                ))
            })?;
            publish.delay_interval = Some(interval_s);
            publish.deref_mut().topic = topic;
        }
        Ok(publish)
    }

    #[inline]
    async fn delay_publish(
        &self,
        from: From,
        publish: Publish,
        message_storage_available: bool,
        message_expiry_interval: Option<Duration>,
    ) -> Result<Option<(From, Publish)>> {
        // Decide under the heap lock; the drop hook fires after the lock is
        // released to keep the hook out of the critical section.
        let (result, dropped) = {
            let cfg = self.cfg.read().await;
            let mut msgs = self.msgs.write().await;
            if let Some(scx) = &self.scx {
                if msgs.len() < cfg.publish_max {
                    msgs.push(DelayedPublish::new(
                        from,
                        publish,
                        message_storage_available,
                        message_expiry_interval,
                    ));
                    scx.stats.delayed_publishs.max_max(msgs.len() as isize);
                    (Ok(None), None)
                } else if cfg.publish_immediate {
                    // Over the limit: forward immediately as a regular message.
                    (Ok(Some((from, publish))), None)
                } else {
                    // Over the limit: drop (hook fires after the lock below).
                    (Ok(None), Some((from, publish)))
                }
            } else {
                // No ServerContext (test-only construction): treat as pass-through.
                (Ok(Some((from, publish))), None)
            }
        };
        if let (Some((from, publish)), Some(scx)) = (dropped, &self.scx) {
            log::warn!("delayed publish limit reached, message dropped, topic: {}", publish.topic);
            scx.extends.hook_mgr().message_dropped(None, from, publish, Reason::DelayedPublishRefused).await;
        }
        result
    }

    #[inline]
    async fn len(&self) -> usize {
        self.msgs.read().await.len()
    }

    async fn list(&self, topic_filter: Option<&str>, max: usize) -> Vec<DelayedPublishInfo> {
        let mut infos: Vec<DelayedPublishInfo> = {
            let msgs = self.msgs.read().await;
            msgs.iter().map(DelayedPublishInfo::from).collect()
        };
        if let Some(tf) = topic_filter {
            if tf != "#" {
                match tf.parse::<rmqtt::topic::Topic>() {
                    Ok(t) => infos.retain(|i| t.matches_str(&i.topic)),
                    Err(_) => return Vec::new(),
                }
            }
        }
        infos.sort_by(|a, b| a.expired_time.cmp(&b.expired_time).then(a.topic.cmp(&b.topic)));
        infos.truncate(max);
        infos
    }

    async fn find(
        &self,
        topic: &str,
        expired_time: TimestampMillis,
        client_id: Option<&str>,
    ) -> Option<DelayedPublishDetail> {
        let msgs = self.msgs.read().await;
        for dp in msgs.iter() {
            if &*dp.publish.topic == topic
                && dp.expired_time == expired_time
                && client_id.map(|c| &*dp.from.id.client_id == c).unwrap_or(true)
            {
                return Some(DelayedPublishDetail {
                    info: DelayedPublishInfo::from(dp),
                    payload: dp.publish.payload.clone(),
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use bytestring::ByteString;
    use rmqtt::delayed::DelayedSender;
    use rmqtt::types::Publish as BrokerPublish;
    use rmqtt::types::{ClientId, Id, QoS};

    use super::*;

    /// Build a pending delayed publish with a fixed trigger timestamp
    /// (deterministic ordering for assertions). Uses a fixed 11-byte payload.
    fn test_dp(expired_at: i64, topic: &str, delay_s: u32) -> DelayedPublish {
        let inner = rmqtt_codec::types::Publish {
            dup: false,
            retain: false,
            qos: QoS::AtMostOnce,
            topic: ByteString::from(topic),
            packet_id: None,
            payload: Bytes::from_static(b"hello world"),
            properties: None,
        };
        DelayedPublish {
            expired_time: expired_at,
            from: From::from_custom(Id::from(1, ClientId::from("client-1"))),
            publish: BrokerPublish {
                inner: Box::new(inner),
                target_clientid: None,
                delay_interval: Some(delay_s),
                create_time: None,
            },
            message_storage_available: false,
            message_expiry_interval: None,
        }
    }

    #[tokio::test]
    async fn background_task_exits_after_sender_dropped() {
        // Build a live sender (spawns the background task) without a
        // ServerContext: the task ticks but never sends.
        let msgs = Arc::new(RwLock::new(BinaryHeap::<DelayedPublish>::default()));
        let s = MemDelayedSender {
            scx: None,
            cfg: Arc::new(RwLock::new(PluginConfig::default())),
            msgs: msgs.clone(),
            stopped: Arc::new(AtomicBool::new(false)),
        }
        .start();
        // test clone + sender clone + task clone
        assert_eq!(Arc::strong_count(&msgs), 3);

        drop(s);
        // Drop sets the stop flag synchronously; the task observes it within
        // one 500 ms tick and exits, releasing its clone. Poll until the
        // strong count drops to 1 (only the test's clone remains).
        for _ in 0..20 {
            if Arc::strong_count(&msgs) == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(Arc::strong_count(&msgs), 1, "background task must exit after the sender is dropped");
    }

    #[tokio::test]
    async fn flush_on_exit_drains_pending_messages() {
        // Without a ServerContext the flush just drains the heap; the point
        // here is that the heap is emptied when the sender is dropped.
        let s = MemDelayedSender::with_msgs(vec![test_dp(1000, "a/b", 10), test_dp(2000, "c/d", 10)]).start();
        let msgs = s.msgs.clone();
        assert_eq!(msgs.read().await.len(), 2);
        assert!(!s.stopped.load(Ordering::Acquire));

        drop(s);
        // The task observes the stop flag within one tick and flushes.
        for _ in 0..20 {
            if msgs.read().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(msgs.read().await.is_empty(), "pending messages must be flushed on exit");
    }

    #[tokio::test]
    async fn list_empty_heap_returns_empty() {
        let s = MemDelayedSender::with_msgs(vec![]);
        assert!(s.list(None, 100).await.is_empty());
    }

    #[tokio::test]
    async fn list_sorts_by_expired_time_then_topic() {
        let s = MemDelayedSender::with_msgs(vec![
            test_dp(3000, "b/t", 10),
            test_dp(1000, "z/t", 10),
            test_dp(3000, "a/t", 10),
            test_dp(2000, "m/t", 10),
        ]);
        let infos = s.list(None, 100).await;
        let seq: Vec<(i64, String)> = infos.iter().map(|i| (i.expired_time, i.topic.to_string())).collect();
        assert_eq!(
            seq,
            vec![
                (1000, "z/t".to_string()),
                (2000, "m/t".to_string()),
                (3000, "a/t".to_string()),
                (3000, "b/t".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn list_truncates_to_max_oldest_first() {
        let s = MemDelayedSender::with_msgs(vec![
            test_dp(3000, "c/t", 10),
            test_dp(1000, "a/t", 10),
            test_dp(2000, "b/t", 10),
        ]);
        let infos = s.list(None, 2).await;
        let topics: Vec<String> = infos.iter().map(|i| i.topic.to_string()).collect();
        assert_eq!(topics, vec!["a/t".to_string(), "b/t".to_string()]);
    }

    #[tokio::test]
    async fn list_filters_by_topic_wildcards() {
        let s = MemDelayedSender::with_msgs(vec![
            test_dp(1000, "sensor/room1/temp", 10),
            test_dp(2000, "sensor/room2/temp", 10),
            test_dp(3000, "device/status", 10),
        ]);
        // `+` wildcard matches a single level
        let infos = s.list(Some("sensor/+/temp"), 100).await;
        let topics: Vec<String> = infos.iter().map(|i| i.topic.to_string()).collect();
        assert_eq!(topics, vec!["sensor/room1/temp".to_string(), "sensor/room2/temp".to_string()]);
        // `#` wildcard matches the remaining levels
        let infos = s.list(Some("sensor/#"), 100).await;
        let topics: Vec<String> = infos.iter().map(|i| i.topic.to_string()).collect();
        assert_eq!(topics, vec!["sensor/room1/temp".to_string(), "sensor/room2/temp".to_string()]);
        // exact topic filter
        let infos = s.list(Some("device/status"), 100).await;
        let topics: Vec<String> = infos.iter().map(|i| i.topic.to_string()).collect();
        assert_eq!(topics, vec!["device/status".to_string()]);
        // `#` explicitly means all
        assert_eq!(s.list(Some("#"), 100).await.len(), 3);
        // filter matches nothing
        assert!(s.list(Some("other/#"), 100).await.is_empty());
    }

    #[tokio::test]
    async fn list_invalid_topic_filter_returns_empty() {
        let s = MemDelayedSender::with_msgs(vec![test_dp(1000, "a/b", 10)]);
        // `#` must be the last level -> invalid filter
        assert!(s.list(Some("a/#/b"), 100).await.is_empty());
    }

    #[tokio::test]
    async fn list_metadata_excludes_payload_content() {
        let s = MemDelayedSender::with_msgs(vec![test_dp(1000, "a/b", 300)]);
        let infos = s.list(None, 100).await;
        assert_eq!(infos.len(), 1);
        let info = &infos[0];
        assert_eq!(info.topic.to_string(), "a/b");
        assert_eq!(info.delay_interval, 300);
        assert_eq!(info.expired_time, 1000);
        assert_eq!(info.client_id.as_deref(), Some("client-1"));
        assert_eq!(info.qos, 0);
        assert!(!info.retain);
        // only the size is exposed, never the content
        assert_eq!(info.payload_len, b"hello world".len());
    }

    #[tokio::test]
    async fn find_returns_full_payload_on_composite_key_match() {
        let s = MemDelayedSender::with_msgs(vec![test_dp(1000, "a/b", 300)]);
        let detail = s.find("a/b", 1000, Some("client-1")).await.expect("found");
        assert_eq!(detail.info.topic.to_string(), "a/b");
        assert_eq!(detail.info.payload_len, 11);
        // the full content is only exposed through find(), never through list()
        assert_eq!(detail.payload.as_ref(), b"hello world");
    }

    #[tokio::test]
    async fn find_requires_all_key_parts_to_match() {
        let s = MemDelayedSender::with_msgs(vec![test_dp(1000, "a/b", 300)]);
        // wrong trigger timestamp
        assert!(s.find("a/b", 2000, Some("client-1")).await.is_none());
        // wrong topic
        assert!(s.find("a/c", 1000, Some("client-1")).await.is_none());
        // wrong publisher
        assert!(s.find("a/b", 1000, Some("other")).await.is_none());
        // client_id omitted -> first match on (topic, expired_time)
        assert!(s.find("a/b", 1000, None).await.is_some());
        // empty heap counterpart
        assert!(MemDelayedSender::with_msgs(vec![]).find("a/b", 1000, None).await.is_none());
    }

    #[tokio::test]
    async fn find_returns_first_match_on_duplicate_keys() {
        // Same composite key twice (same millisecond, topic, publisher):
        // the heap iteration order is unspecified, but exactly one entry with
        // the shared payload content must be returned.
        let s = MemDelayedSender::with_msgs(vec![test_dp(1000, "a/b", 300), test_dp(1000, "a/b", 300)]);
        let detail = s.find("a/b", 1000, Some("client-1")).await.expect("found");
        assert_eq!(detail.payload.as_ref(), b"hello world");
        assert_eq!(detail.info.payload_len, 11);
    }
}
