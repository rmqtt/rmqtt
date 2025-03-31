use std::num::{NonZeroU16, NonZeroU32};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;

use crate::codec::types::{Publish, MQTT_LEVEL_5};
use crate::types::*;
use crate::Result;

pub trait FitterManager: Sync + Send {
    fn create(&self, conn_info: Arc<ConnectInfo>, id: Id, cfg: ListenerConfig) -> FitterType;
}

// #[async_trait]
pub trait Fitter: Sync + Send {
    ///keep_alive - is client input value, unit: seconds
    fn keep_alive(&self, keep_alive: &mut u16) -> Result<u16>;

    ///Maximum length of message queue, default value: 1000
    fn max_mqueue_len(&self) -> usize;

    ///Pop up message speed from message queue, the number of pop-up messages from the queue within
    /// a specified time, which can effectively control the message flow rate,
    /// default value: 100 / 10s
    fn mqueue_rate_limit(&self) -> (NonZeroU32, Duration);

    ///max inflight
    fn max_inflight(&self) -> std::num::NonZeroU16;

    ///session expiry interval
    fn session_expiry_interval(&self, d: Option<&Disconnect>) -> Duration;

    ///message expiry interval
    fn message_expiry_interval(&self, publish: &Publish) -> Duration;

    ///client topic alias maximum, C -> S(Max Limit)
    fn max_client_topic_aliases(&self) -> u16;

    ///server topic alias maximum, S(Max Limit) -> C
    fn max_server_topic_aliases(&self) -> u16;
}

pub struct DefaultFitterManager;

impl FitterManager for DefaultFitterManager {
    #[inline]
    fn create(&self, conn_info: Arc<ConnectInfo>, id: Id, listen_cfg: ListenerConfig) -> FitterType {
        Arc::new(DefaultFitter::new(conn_info, id, listen_cfg))
    }
}

#[derive(Clone)]
pub struct DefaultFitter {
    conn_info: Arc<ConnectInfo>,
    listen_cfg: ListenerConfig,
}

impl DefaultFitter {
    #[inline]
    pub fn new(conn_info: Arc<ConnectInfo>, _id: Id, listen_cfg: ListenerConfig) -> Self {
        Self { conn_info, listen_cfg }
    }
}

#[async_trait]
impl Fitter for DefaultFitter {
    #[inline]
    fn keep_alive(&self, keep_alive: &mut u16) -> Result<u16> {
        if self.conn_info.proto_ver() == MQTT_LEVEL_5 {
            if *keep_alive == 0 {
                return if self.listen_cfg.allow_zero_keepalive {
                    Ok(0)
                } else {
                    Err(anyhow!("Keepalive must be greater than 0"))
                };
            } else if *keep_alive < self.listen_cfg.min_keepalive {
                *keep_alive = self.listen_cfg.min_keepalive;
            } else if *keep_alive > self.listen_cfg.max_keepalive {
                *keep_alive = self.listen_cfg.max_keepalive;
            }
        } else if *keep_alive == 0 {
            return if self.listen_cfg.allow_zero_keepalive {
                Ok(0)
            } else {
                Err(anyhow!("Keepalive must be greater than 0"))
            };
        } else if *keep_alive < self.listen_cfg.min_keepalive {
            return Err(anyhow!(format!(
                "Keepalive is too small and cannot be less than {}",
                self.listen_cfg.min_keepalive
            )));
        } else if *keep_alive > self.listen_cfg.max_keepalive {
            return Err(anyhow!(format!(
                "Keepalive is too large and cannot be greater than {}",
                self.listen_cfg.max_keepalive
            )));
        }

        if *keep_alive < 6 {
            Ok(*keep_alive + 3)
        } else {
            Ok(((*keep_alive as f32 * self.listen_cfg.keepalive_backoff) * 2.0) as u16)
        }
    }

    #[inline]
    fn max_mqueue_len(&self) -> usize {
        self.listen_cfg.max_mqueue_len
    }

    #[inline]
    fn mqueue_rate_limit(&self) -> (NonZeroU32, Duration) {
        self.listen_cfg.mqueue_rate_limit
    }

    #[inline]
    fn max_inflight(&self) -> NonZeroU16 {
        let receive_max = if let ConnectInfo::V5(_, connect) = self.conn_info.as_ref() {
            connect.receive_max
        } else {
            None
        };

        if let Some(receive_max) = receive_max {
            self.listen_cfg.max_inflight.min(receive_max)
        } else {
            self.listen_cfg.max_inflight
        }
    }

    #[inline]
    fn session_expiry_interval(&self, d: Option<&Disconnect>) -> Duration {
        let expiry_interval = || {
            if let ConnectInfo::V5(_, connect) = self.conn_info.as_ref() {
                Duration::from_secs(connect.session_expiry_interval_secs as u64)
            } else {
                self.listen_cfg.session_expiry_interval
            }
        };

        if let Some(Disconnect::V5(d)) = d {
            if let Some(interval_secs) = d.session_expiry_interval_secs {
                Duration::from_secs(interval_secs as u64)
            } else {
                expiry_interval()
            }
        } else {
            expiry_interval()
        }
    }

    #[inline]
    fn message_expiry_interval(&self, publish: &Publish) -> Duration {
        let expiry_interval = publish
            .properties
            .as_ref()
            .and_then(|p| p.message_expiry_interval.map(|i| Duration::from_secs(i.get() as u64)))
            .unwrap_or_else(|| self.listen_cfg.message_expiry_interval);
        log::debug!("{:?} message_expiry_interval: {:?}", self.conn_info.id(), expiry_interval);
        expiry_interval
    }

    #[inline]
    fn max_client_topic_aliases(&self) -> u16 {
        if let ConnectInfo::V5(_, _connect) = self.conn_info.as_ref() {
            self.listen_cfg.max_topic_aliases
        } else {
            0
        }
    }

    #[inline]
    fn max_server_topic_aliases(&self) -> u16 {
        if let ConnectInfo::V5(_, connect) = self.conn_info.as_ref() {
            connect.topic_alias_max.min(self.listen_cfg.max_topic_aliases)
        } else {
            0
        }
    }
}
