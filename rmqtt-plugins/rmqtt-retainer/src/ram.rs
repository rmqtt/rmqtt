//! In-memory retainer implementation.
//!
//! Wraps [`DefaultRetainStorage`] to provide a [`RetainStorage`] that
//! enforces configurable message count and payload size limits.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use rmqtt::utils::StatsMergeMode;
use rmqtt::{
    retain::DefaultRetainStorage,
    retain::RetainStorage,
    types::{Retain, TopicFilter, TopicName},
    Result,
};

use crate::PluginConfig;

#[derive(Clone)]
pub(crate) struct RamRetainer {
    pub(crate) inner: Arc<DefaultRetainStorage>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl RamRetainer {
    #[inline]
    pub(crate) fn new(cfg: Arc<RwLock<PluginConfig>>) -> RamRetainer {
        Self { inner: Arc::new(DefaultRetainStorage::new()), cfg }
    }

    #[inline]
    pub(crate) async fn remove_expired_messages(&self) -> usize {
        self.inner.remove_expired_messages().await
    }
}

#[async_trait]
impl RetainStorage for RamRetainer {
    #[inline]
    fn enable(&self) -> bool {
        true
    }

    #[inline]
    fn merge_on_read(&self) -> bool {
        false
    }

    #[inline]
    fn need_sync(&self) -> bool {
        true
    }

    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain, expiry_interval: Option<Duration>) -> Result<()> {
        let (max_retained_messages, max_payload_size, retained_message_ttl) = {
            let cfg = self.cfg.read().await;
            (cfg.max_retained_messages, *cfg.max_payload_size, cfg.retained_message_ttl)
        };

        if retain.publish.payload.len() > max_payload_size {
            log::warn!("Retain message payload exceeding limit, topic: {topic:?}, retain: {retain:?}");
            return Ok(());
        }

        if max_retained_messages > 0 && self.inner.count().await >= max_retained_messages {
            log::warn!(
                "The retained message has exceeded the maximum limit of: {max_retained_messages}, topic: {topic:?}, retain: {retain:?}"
            );
            return Ok(());
        }

        let expiry_interval = retained_message_ttl
            .map(|ttl| if ttl.is_zero() { None } else { Some(ttl) })
            .unwrap_or(expiry_interval);

        self.inner.set_with_timeout(topic, retain, expiry_interval).await
    }

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        Ok(self.inner.get_message(topic_filter).await?)
    }

    async fn get_all_paginated(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<(TopicName, Retain, Option<Duration>)>, bool)> {
        self.inner.get_all_paginated(offset, limit).await
    }

    #[inline]
    async fn count(&self) -> isize {
        self.inner.count().await
    }

    #[inline]
    async fn max(&self) -> isize {
        self.inner.max().await
    }

    #[inline]
    fn stats_merge_mode(&self) -> StatsMergeMode {
        StatsMergeMode::Max
    }
}
