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

/// Central management component for MQTT broker subsystems
pub struct Manager {
    /// Thread-safe access to shared broker state
    pub shared: RwLock<Box<dyn Shared>>,
    /// Routing component for message distribution
    pub router: RwLock<Box<dyn Router>>,
    #[cfg(feature = "retain")]
    /// Retention storage for QoS 1/2 messages (feature-gated)
    pub retain: RwLock<Box<dyn RetainStorage>>,
    /// Message fitting and processing manager
    pub fitter_mgr: RwLock<Box<dyn FitterManager>>,
    /// Hook management system for event handling
    pub hook_mgr: Box<dyn HookManager>,
    #[cfg(feature = "shared-subscription")]
    /// Shared subscription handler (feature-gated)
    pub shared_subscription: RwLock<Box<dyn SharedSubscription>>,
    /// Session state management component
    pub session_mgr: RwLock<Box<dyn SessionManager>>,
    #[cfg(feature = "msgstore")]
    /// Message storage system (feature-gated)
    pub message_mgr: RwLock<Box<dyn MessageManager>>,
    #[cfg(feature = "delayed")]
    /// Delayed message sender (feature-gated)
    pub delayed_sender: RwLock<Box<dyn DelayedSender>>,
    #[cfg(feature = "auto-subscription")]
    /// Automatic subscription handler (feature-gated)
    pub auto_subscription: RwLock<Box<dyn AutoSubscription>>,
}

impl Manager {
    /// Creates new Manager with default implementations
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

    /// Gets read access to shared state
    #[inline]
    pub async fn shared(&self) -> RwLockReadGuard<'_, Box<dyn Shared>> {
        self.shared.read().await
    }

    /// Gets write access to shared state
    #[inline]
    pub async fn shared_mut(&self) -> RwLockWriteGuard<'_, Box<dyn Shared>> {
        self.shared.write().await
    }

    /// Gets read access to router component
    #[inline]
    pub async fn router(&self) -> RwLockReadGuard<'_, Box<dyn Router>> {
        self.router.read().await
    }

    /// Gets write access to router component
    #[inline]
    pub async fn router_mut(&self) -> RwLockWriteGuard<'_, Box<dyn Router>> {
        self.router.write().await
    }

    /// Gets read access to retain storage (requires "retain" feature)
    #[inline]
    #[cfg(feature = "retain")]
    pub async fn retain(&self) -> RwLockReadGuard<'_, Box<dyn RetainStorage>> {
        self.retain.read().await
    }

    /// Gets write access to retain storage (requires "retain" feature)
    #[inline]
    #[cfg(feature = "retain")]
    pub async fn retain_mut(&self) -> RwLockWriteGuard<'_, Box<dyn RetainStorage>> {
        self.retain.write().await
    }

    /// Gets read access to fitter manager
    #[inline]
    pub async fn fitter_mgr(&self) -> RwLockReadGuard<'_, Box<dyn FitterManager>> {
        self.fitter_mgr.read().await
    }

    /// Gets write access to fitter manager
    #[inline]
    pub async fn fitter_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn FitterManager>> {
        self.fitter_mgr.write().await
    }

    /// Gets reference to hook manager (non-async access)
    #[inline]
    pub fn hook_mgr(&self) -> &dyn HookManager {
        self.hook_mgr.as_ref()
    }

    /// Gets read access to shared subscriptions (requires "shared-subscription" feature)
    #[inline]
    #[cfg(feature = "shared-subscription")]
    pub async fn shared_subscription(&self) -> RwLockReadGuard<'_, Box<dyn SharedSubscription>> {
        self.shared_subscription.read().await
    }

    /// Gets write access to shared subscriptions (requires "shared-subscription" feature)
    #[inline]
    #[cfg(feature = "shared-subscription")]
    pub async fn shared_subscription_mut(&self) -> RwLockWriteGuard<'_, Box<dyn SharedSubscription>> {
        self.shared_subscription.write().await
    }

    /// Gets read access to session manager
    #[inline]
    pub async fn session_mgr(&self) -> RwLockReadGuard<'_, Box<dyn SessionManager>> {
        self.session_mgr.read().await
    }

    /// Gets write access to session manager
    #[inline]
    pub async fn session_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn SessionManager>> {
        self.session_mgr.write().await
    }

    /// Gets read access to message manager (requires "msgstore" feature)
    #[inline]
    #[cfg(feature = "msgstore")]
    pub async fn message_mgr(&self) -> RwLockReadGuard<'_, Box<dyn MessageManager>> {
        self.message_mgr.read().await
    }

    /// Gets write access to message manager (requires "msgstore" feature)
    #[inline]
    #[cfg(feature = "msgstore")]
    pub async fn message_mgr_mut(&self) -> RwLockWriteGuard<'_, Box<dyn MessageManager>> {
        self.message_mgr.write().await
    }

    /// Gets read access to delayed sender (requires "delayed" feature)
    #[inline]
    #[cfg(feature = "delayed")]
    pub async fn delayed_sender(&self) -> RwLockReadGuard<'_, Box<dyn DelayedSender>> {
        self.delayed_sender.read().await
    }

    /// Gets write access to delayed sender (requires "delayed" feature)
    #[inline]
    #[cfg(feature = "delayed")]
    pub async fn delayed_sender_mut(&self) -> RwLockWriteGuard<'_, Box<dyn DelayedSender>> {
        self.delayed_sender.write().await
    }

    /// Gets read access to auto-subscription (requires "auto-subscription" feature)
    #[inline]
    #[cfg(feature = "auto-subscription")]
    pub async fn auto_subscription(&self) -> RwLockReadGuard<'_, Box<dyn AutoSubscription>> {
        self.auto_subscription.read().await
    }

    /// Gets write access to auto-subscription (requires "auto-subscription" feature)
    #[inline]
    #[cfg(feature = "auto-subscription")]
    pub async fn auto_subscription_mut(&self) -> RwLockWriteGuard<'_, Box<dyn AutoSubscription>> {
        self.auto_subscription.write().await
    }
}
