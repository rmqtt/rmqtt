//! Delayed Message Publishing System
//!
//! Implements MQTT's delayed message delivery mechanism with:
//! - Time-based message scheduling
//! - Priority queue for efficient expiration handling
//! - Configurable storage limits
//!
//! ## Core Functionality
//! 1. ​**​Topic Parsing​**​:
//!    - Recognizes `$delayed/<interval>/<topic>` format
//!    - Extracts delay intervals from topic strings
//!    - Validates delay parameter formatting
//!
//! 2. ​**​Message Scheduling​**​:
//!    - Maintains time-ordered priority queue (BinaryHeap)
//!    - Periodic expiration checks (500ms intervals)
//!    - Automatic forwarding of expired messages
//!
//! 3. ​**​Resource Management​**​:
//!    - Enforces maximum delayed message limit
//!    - Tracks statistics through ServerContext
//!    - Provides atomic length checks
//!
//! ## Implementation Details
//! - Uses RwLock for thread-safe queue operations
//! - Tokio-based async task for background processing  
//! - Zero-copy topic parsing with Vec allocation
//! - Graceful handling of storage limits
//!
//! Typical workflow:
//! 1. Parse incoming publish for delay parameters
//! 2. Schedule message if within limits
//! 3. Background task forwards expired messages
//! 4. Statistics updated throughout lifecycle

use std::collections::BinaryHeap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::context::ServerContext;
use crate::session::SessionState;
use crate::types::{DelayedPublish, From, Publish, TopicName};
use crate::Result;

#[async_trait]
pub trait DelayedSender: Sync + Send {
    ///Parse the topic and extract the delayed sending parameters.
    fn parse(&self, publish: Publish) -> Result<Publish>;

    ///Delayed publish
    async fn delay_publish(
        &self,
        from: From,
        publish: Publish,
        message_storage_available: bool,
        message_expiry_interval: Option<Duration>,
    ) -> Result<Option<(From, Publish)>>;

    ///Delayed message count
    async fn len(&self) -> usize;

    #[inline]
    async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

#[derive(Clone)]
pub struct DefaultDelayedSender {
    scx: Option<ServerContext>,
    msgs: Arc<RwLock<BinaryHeap<DelayedPublish>>>,
}

impl DefaultDelayedSender {
    #[inline]
    pub fn new(scx: Option<ServerContext>) -> DefaultDelayedSender {
        Self { scx, msgs: Arc::new(RwLock::new(BinaryHeap::default())) }.start()
    }

    #[inline]
    pub(crate) fn context(&self) -> &ServerContext {
        if let Some(scx) = &self.scx {
            scx
        } else {
            unreachable!()
        }
    }

    fn start(self) -> Self {
        let s = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
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
                            Self::send(s.context(), dp).await;
                        } else {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        });
        self
    }

    #[inline]
    async fn send(scx: &ServerContext, dp: DelayedPublish) {
        if let Err(e) = SessionState::forwards(
            scx,
            dp.from,
            dp.publish,
            dp.message_storage_available,
            dp.message_expiry_interval,
        )
        .await
        {
            log::warn!("delayed forwards error, {:?}", e);
        }
    }
}

#[async_trait]
impl DelayedSender for DefaultDelayedSender {
    #[inline]
    fn parse(&self, mut publish: Publish) -> Result<Publish> {
        let items = publish.topic.splitn(3, '/').collect::<Vec<_>>();
        if let (Some(&"$delayed"), Some(delay_interval), Some(topic)) =
            (items.first(), items.get(1), items.get(2))
        {
            let interval_s = delay_interval.parse().map_err(|e| {
                anyhow!(format!(
                    "the delay time of $delayed must be an integer, topic: {}, {}",
                    publish.topic, e
                ))
            })?;
            publish.delay_interval = Some(interval_s);
            publish.topic = TopicName::from(*topic);
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
        let mut msgs = self.msgs.write().await;
        if msgs.len() < self.context().mqtt_delayed_publish_max {
            msgs.push(DelayedPublish::new(from, publish, message_storage_available, message_expiry_interval));
            #[cfg(feature = "stats")]
            self.context().stats.delayed_publishs.max_max(msgs.len() as isize);
            Ok(None)
        } else {
            Ok(Some((from, publish)))
        }
    }

    #[inline]
    async fn len(&self) -> usize {
        self.msgs.read().await.len()
    }
}
