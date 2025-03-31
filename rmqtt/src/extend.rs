use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

#[cfg(feature = "delayed")]
use crate::delayed::{DefaultDelayedSender, DelayedSender};
use crate::fitter::{DefaultFitterManager, FitterManager};
use crate::hook::{DefaultHookManager, HookManager};
#[cfg(feature = "msgstore")]
use crate::message::{DefaultMessageManager, MessageManager};
#[cfg(feature = "retain")]
use crate::retain::{DefaultRetainStorage, RetainStorage};
use crate::router::{DefaultRouter, Router};

use crate::session::{DefaultSessionManager, SessionManager};
use crate::shared::{DefaultShared, Shared};
#[cfg(feature = "auto-subscription")]
use crate::subscribe::{AutoSubscription, DefaultAutoSubscription};
#[cfg(feature = "shared-subscription")]
use crate::subscribe::{DefaultSharedSubscription, SharedSubscription};

// Defines a struct that manages a number of lock objects to different components that are
// part of an MQTT broker.
pub struct Manager {
    shared: RwLock<Box<dyn Shared>>,
    router: RwLock<Box<dyn Router>>,
    #[cfg(feature = "retain")]
    retain: RwLock<Box<dyn RetainStorage>>,
    fitter_mgr: RwLock<Box<dyn FitterManager>>,
    hook_mgr: Box<dyn HookManager>,
    #[cfg(feature = "shared-subscription")]
    shared_subscription: RwLock<Box<dyn SharedSubscription>>,
    session_mgr: RwLock<Box<dyn SessionManager>>,
    #[cfg(feature = "msgstore")]
    message_mgr: RwLock<Box<dyn MessageManager>>,
    #[cfg(feature = "delayed")]
    delayed_sender: RwLock<Box<dyn DelayedSender>>,
    #[cfg(feature = "auto-subscription")]
    auto_subscription: RwLock<Box<dyn AutoSubscription>>,
}

impl Manager {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            shared: RwLock::new(Box::new(DefaultShared::new(None))),
            router: RwLock::new(Box::new(DefaultRouter::new(None))),
            #[cfg(feature = "retain")]
            retain: RwLock::new(Box::new(DefaultRetainStorage::new())),
            fitter_mgr: RwLock::new(Box::new(DefaultFitterManager)),
            hook_mgr: Box::new(DefaultHookManager::new()),
            #[cfg(feature = "shared-subscription")]
            shared_subscription: RwLock::new(Box::new(DefaultSharedSubscription)),
            session_mgr: RwLock::new(Box::new(DefaultSessionManager)),
            #[cfg(feature = "msgstore")]
            message_mgr: RwLock::new(Box::new(DefaultMessageManager::new())),
            #[cfg(feature = "delayed")]
            delayed_sender: RwLock::new(Box::new(DefaultDelayedSender::new(None))),
            #[cfg(feature = "auto-subscription")]
            auto_subscription: RwLock::new(Box::new(DefaultAutoSubscription)),
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
    #[cfg(feature = "retain")]
    pub async fn retain(&self) -> RwLockReadGuard<'_, Box<dyn RetainStorage>> {
        self.retain.read().await
    }

    #[inline]
    #[cfg(feature = "retain")]
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
    pub fn hook_mgr(&self) -> &dyn HookManager {
        self.hook_mgr.as_ref()
    }

    // #[inline]
    // pub async fn hook_mgr(&self) -> RwLockReadGuard<'_, Box<dyn HookManager>> {
    //     self.hook_mgr.read().await
    // }

    #[inline]
    #[cfg(feature = "shared-subscription")]
    pub async fn shared_subscription(&self) -> RwLockReadGuard<'_, Box<dyn SharedSubscription>> {
        self.shared_subscription.read().await
    }

    #[inline]
    #[cfg(feature = "shared-subscription")]
    pub async fn shared_subscription_mut(&self) -> RwLockWriteGuard<'_, Box<dyn SharedSubscription>> {
        self.shared_subscription.write().await
    }

    #[inline]
    pub async fn session_mgr(&self) -> RwLockReadGuard<'_, Box<dyn SessionManager>> {
        self.session_mgr.read().await
    }

    #[inline]
    pub async fn session_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn SessionManager>> {
        self.session_mgr.write().await
    }

    #[inline]
    #[cfg(feature = "msgstore")]
    pub async fn message_mgr(&self) -> RwLockReadGuard<'_, Box<dyn MessageManager>> {
        self.message_mgr.read().await
    }

    #[inline]
    #[cfg(feature = "msgstore")]
    pub async fn message_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn MessageManager>> {
        self.message_mgr.write().await
    }

    #[inline]
    #[cfg(feature = "delayed")]
    pub async fn delayed_sender(&self) -> RwLockReadGuard<'_, Box<dyn DelayedSender>> {
        self.delayed_sender.read().await
    }

    #[inline]
    #[cfg(feature = "delayed")]
    pub async fn delayed_sender_mut(&self) -> RwLockWriteGuard<'_, Box<dyn DelayedSender>> {
        self.delayed_sender.write().await
    }

    #[inline]
    #[cfg(feature = "auto-subscription")]
    pub async fn auto_subscription(&self) -> RwLockReadGuard<'_, Box<dyn AutoSubscription>> {
        self.auto_subscription.read().await
    }

    #[inline]
    #[cfg(feature = "auto-subscription")]
    pub async fn auto_subscription_mut(&self) -> RwLockWriteGuard<'_, Box<dyn AutoSubscription>> {
        self.auto_subscription.write().await
    }
}
