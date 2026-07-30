//! Shared Subscription Plugin for RMQTT.
//!
//! Provides multiple shared subscription selection strategies:
//!
//! | Strategy | Description | Use Case |
//! |---|---|---|
//! | `random` | Random subscriber selection | Basic load balancing |
//! | `round_robin` (default) | Sequential round-robin | Equal-capacity consumers |
//! | `round_robin_per_group` | Per-node independent round-robin | Large cluster deployment |
//! | `sticky` | Fixed subscriber per publisher | Stateful processing |
//! | `local` | Prefer publisher's node | Reduce cross-node traffic |
//! | `hash_clientid` | Hash by publisher ClientId | Per-device message ordering |
//! | `hash_topic` | Hash by topic | Topic sharding |
//!
//! Configure via `rmqtt-shared-subscription.toml`:
//! ```toml
//! strategy = "round_robin"
//! ```
//!
#![deny(unsafe_code)]
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use lru::LruCache;
use tokio::sync::{Mutex, RwLock};

use rmqtt::{
    context::ServerContext,
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    types::SharedGroup,
    Result,
};

use config::PluginConfig;

mod config;
mod strategies;

register!(SharedSubscriptionPlugin::new);

pub(crate) use config::Strategy;
use strategies::{SharedSubscriptionImpl, StickyMap};

#[derive(Plugin)]
struct SharedSubscriptionPlugin {
    scx: ServerContext,
    cfg: Arc<RwLock<PluginConfig>>,
    /// Global atomic counter shared with the impl for round_robin.
    rr_counter: Arc<AtomicUsize>,
    /// Per-group counters shared with the impl.
    rr_group_counters: Arc<Mutex<HashMap<SharedGroup, usize>>>,
    /// Sticky map shared with the impl.
    sticky_map: StickyMap,
}

impl SharedSubscriptionPlugin {
    #[inline]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name);
        log::info!("{name} cfg: {cfg:?}");
        let cfg = Arc::new(RwLock::new(cfg?));
        let sticky_cache_size = cfg.read().await.sticky_cache_size;
        Ok(Self {
            scx,
            cfg,
            rr_counter: Arc::new(AtomicUsize::new(0)),
            rr_group_counters: Arc::new(Mutex::new(HashMap::default())),
            sticky_map: Arc::new(Mutex::new(LruCache::new(sticky_cache_size))),
        })
    }
}

#[async_trait]
impl Plugin for SharedSubscriptionPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());

        // Load strategy from config
        let strategy = self.cfg.read().await.strategy;

        // Create and register the implementation with shared state
        let mut shared_sub = self.scx.extends.shared_subscription_mut().await;
        *shared_sub = Box::new(SharedSubscriptionImpl::new(
            strategy,
            self.rr_counter.clone(),
            self.rr_group_counters.clone(),
            self.sticky_map.clone(),
        ));

        log::info!("Shared subscription handler registered, strategy: {:?}", strategy);
        Ok(())
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        let strategy = new_cfg.strategy;
        let sticky_cache_size = new_cfg.sticky_cache_size;
        *self.cfg.write().await = new_cfg;
        self.sticky_map = Arc::new(Mutex::new(LruCache::new(sticky_cache_size)));
        let mut shared_sub = self.scx.extends.shared_subscription_mut().await;
        *shared_sub = Box::new(SharedSubscriptionImpl::new(
            strategy,
            self.rr_counter.clone(),
            self.rr_group_counters.clone(),
            self.sticky_map.clone(),
        ));
        log::info!("load_config ok, {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop, resetting to default handler", self.name());
        let mut shared_sub = self.scx.extends.shared_subscription_mut().await;
        *shared_sub = Box::new(rmqtt::subscribe::DefaultSharedSubscription);
        Ok(true)
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        let strategy = self.cfg.read().await.strategy;
        let rr_counter = self.rr_counter.load(Ordering::Relaxed);

        let (rr_group_count, rr_group_truncated, rr_group_details) = {
            let map = self.rr_group_counters.lock().await;
            let count = map.len();
            let details: Vec<_> = map
                .iter()
                .take(1000)
                .map(|(group, count)| {
                    serde_json::json!({
                        "group": group,
                        "counter": count,
                    })
                })
                .collect();
            (count, count > 1000, details)
        };

        let (sticky_count, sticky_truncated, sticky_details) = {
            let map = self.sticky_map.lock().await;
            let count = map.len();
            let details: Vec<_> = map
                .iter()
                .take(1000)
                .map(|((group, publisher_cid), subscriber_cid)| {
                    serde_json::json!({
                        "group": group,
                        "publisher_client_id": publisher_cid,
                        "subscriber_client_id": subscriber_cid,
                    })
                })
                .collect();
            (count, count > 1000, details)
        };

        use serde_json::map::Map;

        let mut obj = Map::new();
        obj.insert("strategy".into(), serde_json::json!(format!("{:?}", strategy)));

        if matches!(strategy, Strategy::RoundRobin) {
            obj.insert("rr_counter".into(), serde_json::json!(rr_counter));
        }

        if matches!(strategy, Strategy::RoundRobinPerGroup) {
            obj.insert("rr_group_count".into(), serde_json::json!(rr_group_count));
            obj.insert("rr_group_truncated".into(), serde_json::json!(rr_group_truncated));
            obj.insert("rr_group_details".into(), serde_json::json!(rr_group_details));
        }

        if strategy == Strategy::Sticky {
            obj.insert("sticky_binding_count".into(), serde_json::json!(sticky_count));
            obj.insert("sticky_binding_truncated".into(), serde_json::json!(sticky_truncated));
            obj.insert("sticky_binding_details".into(), serde_json::json!(sticky_details));
        }

        serde_json::Value::Object(obj)
    }
}
