//! Stats/Metrics history flush module.
//!
//! Architecture:
//!   Flusher (interval) → writes to LRU cache first, then async flush to storage
//!   Recovery (30s) → retries FAILED entries from LRU to storage
//!   Warmup (startup) → loads existing data from storage into LRU
//!
//! LRU is the single source of truth for queries; storage is async persistence.
//!
//! Key alignment: timestamp rounding granularity is derived from the
//! configurable `flush_interval`, so all components (flusher, query, recovery)
//! use the same interval for consistent key generation.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use lru::LruCache;
use tokio::sync::RwLock;

use rmqtt::{context::ServerContext, types::NodeId};
use rmqtt_storage::DefaultStorageDB as StorageDb;

use crate::types::{CacheEntry, EntryFlags};

/// Global counter of entries that failed to persist (flags = FAILED).
pub(crate) static UNPERSISTED_COUNT: AtomicI64 = AtomicI64::new(0);

/// Shared LRU cache type alias.
pub(crate) type HistoryCache = Arc<RwLock<LruCache<u64, CacheEntry>>>;

/// Holds both LRU caches.
/// Injected into depot so HTTP handlers can access them.
#[derive(Clone)]
pub(crate) struct HistoryCaches {
    pub stats: HistoryCache,
    pub metrics: HistoryCache,
}

impl HistoryCaches {
    pub fn new(stats: HistoryCache, metrics: HistoryCache) -> Self {
        Self { stats, metrics }
    }
}

/// Key prefix for stored Stats history entries.
pub(crate) const STATS_PREFIX: &str = "stats_hist";

/// Key prefix for stored Metrics history entries.
pub(crate) const METRICS_PREFIX: &str = "metrics_hist";

/// Create a new LRU cache with the given capacity (must be > 0).
pub(crate) fn new_cache(capacity: usize) -> anyhow::Result<HistoryCache> {
    let non_zero = std::num::NonZeroUsize::new(capacity)
        .ok_or_else(|| anyhow::anyhow!("[http-api] LRU cache capacity must be > 0, got {capacity}"))?;
    Ok(Arc::new(RwLock::new(LruCache::new(non_zero))))
}

// ═══════════════════════════════════════════════════════════════════════
//  Flusher — every `interval`, snapshot Stats/Metrics → LRU → async Storage
// ═══════════════════════════════════════════════════════════════════════

pub fn start_flusher(
    scx: ServerContext,
    storage_db: StorageDb,
    stats_cache: HistoryCache,
    metrics_cache: HistoryCache,
    interval: Duration,
    retention: Duration,
) -> tokio::task::JoinHandle<()> {
    let node_id = scx.node.id();
    tokio::spawn(async move {
        let mut timer = tokio::time::interval(interval);
        timer.tick().await; // baseline on start

        loop {
            timer.tick().await;
            let ts = rounded_timestamp_ms(interval);

            // ── Stats ──────────────────────────────────────────────
            let stats = scx.stats.clone(&scx).await;
            let stats_value = stats.to_json(&scx).await;
            if let Ok(stats_json) = serde_json::to_string(&stats_value) {
                let storage_key = make_key(STATS_PREFIX, node_id, ts);
                let entry = CacheEntry::new(stats_json);
                stats_cache.write().await.put(ts, entry);
                // Async flush to storage
                let db = storage_db.clone();
                let cache = stats_cache.clone();
                tokio::spawn(async move {
                    flush_one(&db, &cache, &storage_key, ts, retention).await;
                });
            }

            // ── Metrics ────────────────────────────────────────────
            let metrics = scx.metrics.clone();
            let metrics_value = metrics.to_json();
            if let Ok(metrics_json) = serde_json::to_string(&metrics_value) {
                let storage_key = make_key(METRICS_PREFIX, node_id, ts);
                let entry = CacheEntry::new(metrics_json);
                metrics_cache.write().await.put(ts, entry);
                let db = storage_db.clone();
                let cache = metrics_cache.clone();
                tokio::spawn(async move {
                    flush_one(&db, &cache, &storage_key, ts, retention).await;
                });
            }
        }
    })
}

/// Try to persist one entry: insert + expire must both succeed.
/// On failure: mark entry FAILED and increment counter.
async fn flush_one(
    storage_db: &StorageDb,
    cache: &HistoryCache,
    storage_key: &str,
    ts: u64,
    retention: Duration,
) {
    let ok = {
        let key = storage_key.as_bytes();
        storage_db
            .insert(key, &cache.read().await.peek(&ts).map(|e| e.json.clone()).unwrap_or_default())
            .await
            .is_ok()
            && storage_db.expire(key, retention.as_millis() as i64).await.is_ok()
    };
    if ok {
        if let Some(entry) = cache.write().await.get_mut(&ts) {
            entry.flags = EntryFlags::new(EntryFlags::NONE);
        }
    } else {
        log::error!("[http-api] flush {} error", storage_key);
        if let Some(entry) = cache.write().await.get_mut(&ts) {
            entry.flags = EntryFlags::new(EntryFlags::FAILED);
        }
        UNPERSISTED_COUNT.fetch_add(1, Ordering::Release);
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Recovery loop — every 30s, retry FAILED entries from LRU → Storage
// ═══════════════════════════════════════════════════════════════════════

pub fn start_recovery_loop(
    stats_cache: HistoryCache,
    metrics_cache: HistoryCache,
    storage_db: StorageDb,
    retention: Duration,
    node_id: NodeId,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            if UNPERSISTED_COUNT.load(Ordering::Acquire) == 0 {
                continue;
            }
            recover_one_cache(&stats_cache, &storage_db, STATS_PREFIX, node_id, retention).await;
            recover_one_cache(&metrics_cache, &storage_db, METRICS_PREFIX, node_id, retention).await;
        }
    })
}

async fn recover_one_cache(
    cache: &HistoryCache,
    storage_db: &StorageDb,
    prefix: &str,
    node_id: NodeId,
    retention: Duration,
) {
    let failed: Vec<(u64, String)> = {
        let guard = cache.read().await;
        guard
            .iter()
            .filter(|(_, e)| e.flags.needs_recovery())
            .take(200)
            .map(|(ts, e)| (*ts, e.json.clone()))
            .collect()
    };
    if failed.is_empty() {
        return;
    }
    for (ts, json) in &failed {
        let storage_key = make_key(prefix, node_id, *ts);
        let ok = storage_db.insert(storage_key.as_bytes(), json).await.is_ok()
            && storage_db.expire(storage_key.as_bytes(), retention.as_millis() as i64).await.is_ok();
        if ok {
            if let Some(entry) = cache.write().await.get_mut(ts) {
                entry.flags = EntryFlags::new(EntryFlags::NONE);
            }
            let _ = UNPERSISTED_COUNT.fetch_update(Ordering::Release, Ordering::Acquire, |v| {
                if v > 0 {
                    Some(v - 1)
                } else {
                    None
                }
            });
        } else {
            log::error!("[http-api] recovery flush {} error", storage_key);
            break;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  Warmup — on startup, load existing data from storage into LRU
// ═══════════════════════════════════════════════════════════════════════

pub fn start_warmup(
    stats_cache: HistoryCache,
    metrics_cache: HistoryCache,
    storage_db: StorageDb,
    node_id: NodeId,
    retention: Duration,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let stats_count = load_into_cache(&stats_cache, &storage_db, STATS_PREFIX, node_id, retention).await;
        let metrics_count =
            load_into_cache(&metrics_cache, &storage_db, METRICS_PREFIX, node_id, retention).await;
        log::info!("[http-api] history cache warmup complete, stats={stats_count}, metrics={metrics_count}");
    })
}

/// Load history data from storage into LRU cache, discarding expired entries.
///
/// For each key in storage, the embedded timestamp (`ts`) is compared against the
/// current wall-clock time minus `retention`. If the data point has expired
/// (`now - ts > retention`), it is skipped (not loaded into the LRU) and also
/// **deleted from storage** so it does not re-appear on a subsequent restart.
async fn load_into_cache(
    cache: &HistoryCache,
    storage_db: &StorageDb,
    prefix: &str,
    node_id: NodeId,
    retention: Duration,
) -> usize {
    let pattern = format!("{}:{}:*", prefix, node_id);
    // Collect (timestamp, key_bytes) pairs from storage scan, then sort by
    // timestamp ascending so the LRU cache evicts oldest entries first on
    // capacity pressure.
    let retention_ms = retention.as_millis() as u64;
    let now_ms =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()
            as u64;
    let cutoff = now_ms.saturating_sub(retention_ms);

    let keys_sorted: Vec<(u64, Vec<u8>)> = {
        let mut iter_storage_db = storage_db.clone();
        let mut pairs = Vec::new();
        if let Ok(mut iter) = iter_storage_db.scan(pattern.as_bytes()).await {
            while let Some(key) = iter.next().await {
                if let Ok(key_bytes) = key {
                    let key_str = String::from_utf8_lossy(&key_bytes);
                    if let Some(ts) = key_str.rsplit(':').next().and_then(|s| s.parse::<u64>().ok()) {
                        pairs.push((ts, key_bytes));
                    }
                }
            }
        }
        pairs.sort_by_key(|(ts, _)| *ts);
        pairs
    };
    let mut count = 0usize;
    for (ts, key_bytes) in &keys_sorted {
        //Expiration check: discard data points older than `retention`.
        if *ts < cutoff {
            log::debug!(
                "[http-api] warmup discarding expired entry ts={ts} (< cutoff={cutoff}), removing from storage"
            );
            let _ = storage_db.remove(key_bytes).await;
            continue;
        }
        if let Ok(Some(json)) = storage_db.get::<_, String>(key_bytes).await {
            let entry = CacheEntry { json, flags: EntryFlags::new(EntryFlags::NONE) };
            cache.write().await.put(*ts, entry);
            count += 1;
        }
    }
    count
}

// ═══════════════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════════════

/// Build storage key: `{prefix}:{node_id}:{ts}`.
#[inline]
pub(crate) fn make_key(prefix: &str, node_id: NodeId, ts: u64) -> String {
    format!("{prefix}:{node_id}:{ts}")
}

/// Current Unix timestamp in ms, rounded down to the nearest `interval`
/// boundary. This ensures all flusher-generated keys align with query
/// step boundaries regardless of the configured flush interval.
#[inline]
pub(crate) fn rounded_timestamp_ms(interval: Duration) -> u64 {
    let interval_ms = interval.as_millis() as u64;
    let ms =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()
            as u64;
    ms / interval_ms * interval_ms
}
