//! Non-blocking cache with stale-while-revalidate semantics.
//!
//! Provides [`ValueCached<T>`] and [`ValueRef<'_, T>`] for caching
//! storage query results without blocking the caller.
//!
//! # Semantics
//! - `call` / `call_timeout` always return **immediately**.
//! - If a cached value exists (even if expired), `get()` returns `Ok(&T)`.
//! - If the cache is expired (or empty), a background `tokio::spawn` task
//!   is launched to refresh the value. The caller sees stale data briefly.
//! - An `AtomicBool` + CAS ensures at most one background refresh runs
//!   concurrently — redundant calls are silently skipped.
//! - Two mode families:
//!   - `call` / `call_timeout` — `f` is a `Future`, runs on `tokio::spawn`.
//!   - `call_sync` / `call_timeout_sync` — `f` is a `FnOnce`, runs on
//!     `tokio::task::spawn_blocking`.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use tokio::sync::RwLock;

use rmqtt::Result;

// ── ValueCached ──

#[derive(Clone)]
pub struct ValueCached<T> {
    inner: Arc<RwLock<ValueCachedInner<T>>>,
    /// Atomic flag: `false` = idle, `true` = a background refresh is running.
    /// Stays `true` until the spawned task finishes and writes the cache.
    refreshing: Arc<AtomicBool>,
}

struct ValueCachedInner<T> {
    cached_val: Option<Result<T>>,
    expire_interval: Duration,
    instant: Instant,
}

impl<T: Send + Sync + 'static> ValueCached<T> {
    #[inline]
    pub fn new(expire_interval: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(ValueCachedInner {
                cached_val: None,
                expire_interval,
                instant: Instant::now(),
            })),
            refreshing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Non-blocking call. `f` is an async future — runs on `tokio::spawn`.
    #[inline]
    #[allow(unused)]
    pub async fn call<F>(&self, f: F) -> ValueRef<'_, T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        let (has_value, expired) = {
            let inner = self.inner.read().await;
            (inner.cached_val.is_some(), inner.cached_val.as_ref().is_none_or(|_| inner.is_expired()))
        };
        let spawned = if expired { self._refresh(f, None) } else { false };
        ValueRef { val_guard: self.inner.read().await, has_value, spawned, expired }
    }

    /// Non-blocking call with timeout. `f` is an async future.
    #[inline]
    pub async fn call_timeout<F>(&self, f: F, timeout: Duration) -> ValueRef<'_, T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        let (has_value, expired) = {
            let inner = self.inner.read().await;
            (inner.cached_val.is_some(), inner.cached_val.as_ref().is_none_or(|_| inner.is_expired()))
        };
        let spawned = if expired { self._refresh(f, Some(timeout)) } else { false };
        ValueRef { val_guard: self.inner.read().await, has_value, spawned, expired }
    }

    /// Non-blocking call, sync variant. `f` runs on `tokio::task::spawn_blocking`.
    #[inline]
    #[allow(unused)]
    pub async fn call_sync<F>(&self, f: F) -> ValueRef<'_, T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let (has_value, expired) = {
            let inner = self.inner.read().await;
            (inner.cached_val.is_some(), inner.cached_val.as_ref().is_none_or(|_| inner.is_expired()))
        };
        let spawned = if expired { self._refresh_sync(f, None) } else { false };
        ValueRef { val_guard: self.inner.read().await, has_value, spawned, expired }
    }

    /// Non-blocking call with timeout, sync variant.
    #[inline]
    #[allow(unused)]
    pub async fn call_timeout_sync<F>(&self, f: F, timeout: Duration) -> ValueRef<'_, T>
    where
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let (has_value, expired) = {
            let inner = self.inner.read().await;
            (inner.cached_val.is_some(), inner.cached_val.as_ref().is_none_or(|_| inner.is_expired()))
        };
        let spawned = if expired { self._refresh_sync(f, Some(timeout)) } else { false };
        ValueRef { val_guard: self.inner.read().await, has_value, spawned, expired }
    }

    // ── Internal helpers ──

    /// Async refresh: spawn `f` onto `tokio::spawn`.
    /// Returns `true` if a background task was actually spawned.
    fn _refresh<F>(&self, f: F, timeout: Option<Duration>) -> bool
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        // CAS: only one task gets to refresh
        if self.refreshing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() {
            return false;
        }

        // Explicitly clone the Arcs so the spawned future owns Send+Sync data.
        let this_inner = self.inner.clone();
        let this_refreshing = self.refreshing.clone();
        tokio::spawn(async move {
            let result = if let Some(t) = timeout {
                match tokio::time::timeout(t, f).await {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(anyhow!("ValueCached refresh timeout")),
                }
            } else {
                f.await
            };

            let mut inner = this_inner.write().await;
            inner.cached_val = Some(result);
            inner.instant = Instant::now();
            drop(inner);

            this_refreshing.store(false, Ordering::Release);
        });

        true
    }

    /// Sync refresh: spawn `f` onto `tokio::task::spawn_blocking`.
    fn _refresh_sync<F>(&self, f: F, timeout: Option<Duration>) -> bool
    where
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        if self.refreshing.compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed).is_err() {
            return false;
        }

        let this_inner = self.inner.clone();
        let this_refreshing = self.refreshing.clone();
        tokio::spawn(async move {
            let result = if let Some(t) = timeout {
                match tokio::time::timeout(t, tokio::task::spawn_blocking(f)).await {
                    Ok(Ok(Ok(v))) => Ok(v),
                    Ok(Ok(Err(e))) => Err(e),
                    Ok(Err(_)) => Err(anyhow!("spawn_blocking panicked")),
                    Err(_) => Err(anyhow!("ValueCached refresh timeout")),
                }
            } else {
                match tokio::task::spawn_blocking(f).await {
                    Ok(Ok(v)) => Ok(v),
                    Ok(Err(e)) => Err(e),
                    Err(_) => Err(anyhow!("spawn_blocking panicked")),
                }
            };

            let mut inner = this_inner.write().await;
            inner.cached_val = Some(result);
            inner.instant = Instant::now();
            drop(inner);

            this_refreshing.store(false, Ordering::Release);
        });

        true
    }
}

impl<T> ValueCachedInner<T> {
    #[inline]
    fn is_expired(&self) -> bool {
        self.instant.elapsed() > self.expire_interval
    }
}

// ── ValueRef ──

/// Return type of [`ValueCached::call`] / [`ValueCached::call_timeout`].
///
/// Always constructed without awaiting — the caller is never blocked.
pub struct ValueRef<'a, T> {
    val_guard: tokio::sync::RwLockReadGuard<'a, ValueCachedInner<T>>,
    /// Whether a cached value existed at the time of the snapshot.
    #[allow(dead_code)]
    has_value: bool,
    /// Whether this call initiated a background refresh.
    #[allow(dead_code)]
    spawned: bool,
    /// Whether the cached value was expired (or missing) at snapshot time.
    #[allow(dead_code)]
    expired: bool,
}

impl<T> ValueRef<'_, T> {
    /// Returns a reference to the cached value.
    ///
    /// - If any cached value exists (even stale / from a previous error),
    ///   returns `Ok(&T)` — the caller can always read what we have.
    /// - If no value has been fetched yet and the first background refresh
    ///   is still in progress, returns `Err`.
    #[inline]
    pub fn get(&self) -> Result<Option<&T>> {
        match &self.val_guard.cached_val {
            Some(Ok(v)) => Ok(Some(v)),
            Some(Err(e)) => Err(anyhow!(e.to_string())),
            None => Ok(None),
        }
    }

    #[inline]
    #[allow(unused)]
    pub fn has_value(&self) -> bool {
        self.has_value
    }

    #[inline]
    #[allow(unused)]
    pub fn spawned(&self) -> bool {
        self.spawned
    }

    #[inline]
    #[allow(unused)]
    pub fn expired(&self) -> bool {
        self.expired
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// Helper: a short expire interval used in all tests.
    const SHORT: Duration = Duration::from_millis(50);

    /// Helper: wait long enough for a background refresh to complete.
    async fn wait() {
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    /// 1. First call (no cached value) → get() returns Err, bg refresh spawned.
    #[tokio::test]
    async fn first_call_no_value() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);
        let called = Arc::new(AtomicBool::new(false));
        let c = called.clone();

        let r = cached
            .call_timeout(
                async move {
                    c.store(true, Ordering::Release);
                    Ok(42usize)
                },
                Duration::from_millis(500),
            )
            .await;

        // No cached value yet — get() should return Ok(None).
        assert!(r.get().unwrap().is_none(), "first call should return None (no cached value)");
        assert!(!r.has_value(), "has_value should be false");
        assert!(r.spawned(), "should have spawned a background refresh");
        // Drop r to release the read lock so the background task can write.
        drop(r);

        // Wait for the background task to complete.
        wait().await;

        // Now the cache should have the value.
        let r2 = cached.call_timeout(async { Ok(99usize) }, Duration::from_millis(500)).await;
        assert_eq!(*r2.get().unwrap().unwrap(), 42, "should return the stored 42, not 99");
        assert!(called.load(Ordering::Acquire), "the closure should have been called");
    }

    /// 2. Fresh cache returns immediately without spawning.
    #[tokio::test]
    async fn fresh_cache_no_spawn() {
        let cached: ValueCached<usize> = ValueCached::new(Duration::from_millis(5000));
        let called = Arc::new(AtomicBool::new(false));

        // Seed the cache via first call.
        {
            let c = called.clone();
            let r = cached
                .call_timeout(
                    async move {
                        c.store(true, Ordering::Release);
                        Ok(42usize)
                    },
                    Duration::from_millis(500),
                )
                .await;
            drop(r); // release read lock so bg task can write
        }
        wait().await; // let the seed complete

        // Second call while still fresh.
        called.store(false, Ordering::Release);
        let c2 = called.clone();
        let r = cached
            .call_timeout(
                async move {
                    c2.store(true, Ordering::Release);
                    Ok(99usize)
                },
                Duration::from_millis(500),
            )
            .await;

        assert_eq!(*r.get().unwrap().unwrap(), 42, "should return the cached value");
        assert!(!r.spawned(), "should NOT have spawned a refresh");
        assert!(!called.load(Ordering::Acquire), "the closure should NOT have been called");
    }

    /// 3. Expired cache returns stale value AND spawns background refresh.
    #[tokio::test]
    async fn expired_cache_stale_while_revalidate() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);

        // Seed with initial value.
        {
            let r = cached.call_timeout(async { Ok(10usize) }, Duration::from_millis(500)).await;
            drop(r);
        }
        wait().await;

        // Wait for expiry.
        tokio::time::sleep(SHORT).await;

        let called = Arc::new(AtomicBool::new(false));
        let c = called.clone();

        let r = cached
            .call_timeout(
                async move {
                    c.store(true, Ordering::Release);
                    Ok(20usize)
                },
                Duration::from_millis(500),
            )
            .await;

        // Must return the stale 10 immediately, never Err.
        assert_eq!(*r.get().unwrap().unwrap(), 10, "should return stale value 10, not wait for 20");
        assert!(r.spawned(), "should have spawned a background refresh");
        assert!(!called.load(Ordering::Acquire), "the closure may not have run yet");
        // Drop r to release the read lock so the background task can write.
        drop(r);

        // Wait for the background refresh to complete.
        wait().await;

        // Now the cache should have the fresh value 20.
        let r2 = cached.call_timeout(async { Ok(99usize) }, Duration::from_millis(500)).await;
        assert_eq!(*r2.get().unwrap().unwrap(), 20, "should now return the refreshed value 20");
    }

    /// 4. CAS prevents redundant background refreshes.
    #[tokio::test]
    async fn cas_prevents_duplicate_refresh() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);
        let call_count = Arc::new(AtomicUsize::new(0));

        // First call to seed.
        {
            let cc = call_count.clone();
            let r = cached
                .call_timeout(
                    async move {
                        cc.fetch_add(1, Ordering::SeqCst);
                        Ok(1usize)
                    },
                    Duration::from_millis(500),
                )
                .await;
            drop(r);
        }
        wait().await;
        tokio::time::sleep(SHORT).await; // ensure expired

        // Fire two concurrent calls — only one should spawn a refresh.
        call_count.store(0, Ordering::SeqCst);
        let cc1 = call_count.clone();
        let cc2 = call_count.clone();

        let (r1, r2) = tokio::join!(
            cached.call_timeout(
                async move {
                    cc1.fetch_add(1, Ordering::SeqCst);
                    Ok(10usize)
                },
                Duration::from_millis(500),
            ),
            cached.call_timeout(
                async move {
                    cc2.fetch_add(1, Ordering::SeqCst);
                    Ok(20usize)
                },
                Duration::from_millis(500),
            ),
        );

        // Both should return the stale value 1.
        assert_eq!(*r1.get().unwrap().unwrap(), 1, "r1: stale value");
        assert_eq!(*r2.get().unwrap().unwrap(), 1, "r2: stale value");

        // At least one should have spawned.
        assert!(r1.spawned() || r2.spawned(), "at least one call should spawn");
        // The closure should have been called exactly once.
        drop(r1);
        drop(r2);
        wait().await;
        assert_eq!(call_count.load(Ordering::SeqCst), 1, "closure should be called exactly once");
    }

    /// 5. Background refresh eventually updates the cache value.
    #[tokio::test]
    async fn background_refresh_updates_cache() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);

        // Seed.
        {
            let r = cached.call_timeout(async { Ok(1usize) }, Duration::from_millis(500)).await;
            drop(r);
        }
        wait().await;
        tokio::time::sleep(SHORT).await; // expire

        // Spawn a slow refresh that takes 100ms. Drop the ValueRef immediately
        // so the spawned task can acquire the write lock when done.
        let barrier = Arc::new(AtomicBool::new(false));
        let b = barrier.clone();
        {
            let r = cached
                .call_timeout(
                    async move {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        b.store(true, Ordering::Release);
                        Ok(2usize)
                    },
                    Duration::from_millis(500),
                )
                .await;
            // r holds a read lock — drop it before the background task tries
            // to write.
            drop(r);
        }

        // Immediately after drop: still stale (bg task may still be sleeping).
        let r = cached.call_timeout(async { Ok(99usize) }, Duration::from_millis(500)).await;
        assert_eq!(*r.get().unwrap().unwrap(), 1, "still stale before refresh completes");
        drop(r);

        // Wait for the slow refresh to finish.
        wait().await;
        assert!(barrier.load(Ordering::Acquire), "slow refresh should have completed");

        // Now the value should be 2.
        let r = cached.call_timeout(async { Ok(99usize) }, Duration::from_millis(500)).await;
        assert_eq!(*r.get().unwrap().unwrap(), 2, "cache should now have the refreshed value");
    }

    /// 6. When the refresh closure fails, the error is cached.
    #[tokio::test]
    async fn refresh_error_cached() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);

        // Seed a successful value.
        {
            let r = cached.call_timeout(async { Ok(1usize) }, Duration::from_millis(500)).await;
            drop(r);
        }
        wait().await;
        tokio::time::sleep(SHORT).await; // expire

        // Trigger a refresh that fails.
        {
            let r = cached
                .call_timeout(
                    async { Err::<usize, _>(anyhow::anyhow!("storage error")) },
                    Duration::from_millis(500),
                )
                .await;
            drop(r);
        }
        wait().await;

        // get() should return the error.
        let r = cached.call_timeout(async { Ok(3usize) }, Duration::from_millis(500)).await;
        match r.get() {
            Err(e) => assert!(e.to_string().contains("storage error"), "error should be preserved"),
            Ok(v) => panic!("expected error, got Ok({v:?})"),
        }
    }

    /// 7. call_sync variant works with a blocking closure.
    #[tokio::test]
    #[allow(unused)]
    async fn sync_variant_works() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);
        let rc = Arc::new(AtomicUsize::new(0));

        // First call — sync variant.
        {
            let r = cached
                .call_timeout_sync(|| -> Result<usize> { Ok(42usize) }, Duration::from_millis(500))
                .await;
            // No value yet (first call, sync variant spawns blocking task).
            assert!(r.get().unwrap().is_none(), "first call should return None");
            drop(r);
        }
        wait().await;

        // Now the value should be available.
        let r = cached.call_timeout_sync(|| Ok(99usize), Duration::from_millis(500)).await;
        assert_eq!(*r.get().unwrap().unwrap(), 42, "should have the cached value from spawn_blocking");
    }

    /// 8. Multiple expiry cycles work correctly.
    #[tokio::test]
    async fn multiple_expiry_cycles() {
        let cached: ValueCached<usize> = ValueCached::new(SHORT);

        // Cycle 1: seed.
        {
            let r = cached.call_timeout(async { Ok(1usize) }, Duration::from_millis(500)).await;
            drop(r);
        }
        wait().await;

        // Cycle 2: expire and refresh.
        tokio::time::sleep(SHORT).await;
        {
            let r = cached.call_timeout(async { Ok(2usize) }, Duration::from_millis(500)).await;
            assert_eq!(*r.get().unwrap().unwrap(), 1, "stale 1 before refresh");
            assert!(r.spawned());
            drop(r);
        }
        wait().await;

        // Cycle 3: expire and refresh again.
        tokio::time::sleep(SHORT).await;
        {
            let r = cached.call_timeout(async { Ok(3usize) }, Duration::from_millis(500)).await;
            assert_eq!(*r.get().unwrap().unwrap(), 2, "stale 2 before refresh");
            assert!(r.spawned());
            drop(r);
        }
        wait().await;

        let r = cached.call_timeout(async { Ok(99usize) }, Duration::from_millis(500)).await;
        assert_eq!(*r.get().unwrap().unwrap(), 3, "final value should be 3");
    }
}
