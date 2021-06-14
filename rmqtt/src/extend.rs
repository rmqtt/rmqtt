use crate::broker::{
    default::{
        DefaultFitterManager, DefaultHookManager, DefaultLimiterManager, DefaultRetainStorage, DefaultRouter,
        DefaultShared, DefaultSharedSubscription,
    },
    fitter::FitterManager,
    hook::HookManager,
    LimiterManager, RetainStorage, Router, Shared, SharedSubscription,
};
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub struct Manager {
    shared: RwLock<Box<dyn Shared>>,
    router: RwLock<Box<dyn Router>>,
    retain: RwLock<Box<dyn RetainStorage>>,
    fitter_mgr: RwLock<Box<dyn FitterManager>>,
    hook_mgr: RwLock<Box<dyn HookManager>>,
    limiter_mgr: RwLock<Box<dyn LimiterManager>>,
    shared_subscription: RwLock<Box<dyn SharedSubscription>>,
}

impl Manager {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            shared: RwLock::new(Box::new(DefaultShared::instance())),
            router: RwLock::new(Box::new(DefaultRouter::instance())),
            retain: RwLock::new(Box::new(DefaultRetainStorage::instance())),
            fitter_mgr: RwLock::new(Box::new(DefaultFitterManager::instance())),
            hook_mgr: RwLock::new(Box::new(DefaultHookManager::instance())),
            limiter_mgr: RwLock::new(Box::new(DefaultLimiterManager::instance())),
            shared_subscription: RwLock::new(Box::new(DefaultSharedSubscription::instance())),
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

    // #[inline]
    // pub async fn hook_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn HookManager>> {
    //     self.hook_mgr.write().await
    // }

    #[inline]
    pub async fn limiter_mgr(&self) -> RwLockReadGuard<'_, Box<dyn LimiterManager>> {
        self.limiter_mgr.read().await
    }

    #[inline]
    pub async fn limiter_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn LimiterManager>> {
        self.limiter_mgr.write().await
    }

    #[inline]
    pub async fn shared_subscription(&self) -> RwLockReadGuard<'_, Box<dyn SharedSubscription>> {
        self.shared_subscription.read().await
    }

    #[inline]
    pub async fn shared_subscription_mut(&self) -> RwLockWriteGuard<'_, Box<dyn SharedSubscription>> {
        self.shared_subscription.write().await
    }
}
