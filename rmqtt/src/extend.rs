use crate::broker::{
    default::{
        DefaultFitterManager, DefaultHookManager, DefaultLimiterManager, DefaultRetainStorage,
        DefaultRouter, DefaultShared,
    },
    fitter::FitterManager,
    hook::HookManager,
    LimiterManager, RetainStorage, Router, Shared,
};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct Manager {
    shared: RwLock<Box<dyn Shared>>,
    router: RwLock<Box<dyn Router>>,
    retain: RwLock<Box<dyn RetainStorage>>,
    fitter_mgr: RwLock<Box<dyn FitterManager>>,
    hook_mgr: RwLock<Box<dyn HookManager>>,
    limiter_mgr: RwLock<Box<dyn LimiterManager>>,
}

impl Manager {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            shared: RwLock::new(DefaultShared::instance()),
            router: RwLock::new(DefaultRouter::instance()),
            retain: RwLock::new(DefaultRetainStorage::instance()),
            fitter_mgr: RwLock::new(DefaultFitterManager::instance()),
            hook_mgr: RwLock::new(DefaultHookManager::instance()),
            limiter_mgr: RwLock::new(DefaultLimiterManager::instance()),
        }
    }

    #[inline]
    pub async fn shared(&self) -> RwLockReadGuard<'_, Box<dyn Shared>> {
        self.shared.read().await
    }

    #[inline]
    pub async fn shared_mut(&self) -> RwLockWriteGuard<'_, Box<dyn Shared>> {
        self.shared.write().await
    }

    #[inline]
    pub async fn router(&self) -> RwLockReadGuard<'_, Box<dyn Router>> {
        self.router.read().await
    }

    #[inline]
    pub async fn router_mut(&self) -> RwLockWriteGuard<'_, Box<dyn Router>> {
        self.router.write().await
    }

    #[inline]
    pub async fn retain(&self) -> RwLockReadGuard<'_, Box<dyn RetainStorage>> {
        self.retain.read().await
    }

    #[inline]
    pub async fn retain_mut(&self) -> RwLockWriteGuard<'_, Box<dyn RetainStorage>> {
        self.retain.write().await
    }

    #[inline]
    pub async fn fitter_mgr(&self) -> RwLockReadGuard<'_, Box<dyn FitterManager>> {
        self.fitter_mgr.read().await
    }

    #[inline]
    pub async fn fitter_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn FitterManager>> {
        self.fitter_mgr.write().await
    }

    #[inline]
    pub async fn hook_mgr(&self) -> RwLockReadGuard<'_, Box<dyn HookManager>> {
        self.hook_mgr.read().await
    }

    #[inline]
    pub async fn hook_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn HookManager>> {
        self.hook_mgr.write().await
    }

    #[inline]
    pub async fn limiter_mgr(&self) -> RwLockReadGuard<'_, Box<dyn LimiterManager>> {
        self.limiter_mgr.read().await
    }

    #[inline]
    pub async fn limiter_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn LimiterManager>> {
        self.limiter_mgr.write().await
    }
}
