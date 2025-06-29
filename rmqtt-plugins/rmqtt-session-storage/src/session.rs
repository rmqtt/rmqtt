use std::ops::Deref;
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use rmqtt::context::ServerContext;
use rmqtt::session::DefaultSession;
use rmqtt::{
    inflight::OutInflightMessage,
    session::{SessionLike, SessionManager},
    types::{
        ClientId, ConnectInfo, ConnectInfoType, DashMap, Disconnect, FitterType, From, Id, IsPing,
        ListenerConfig, MessageQueueType, Password, Publish, Reason, SessionSubMap, SessionSubs,
        SubscriptionOptions, Subscriptions, TimestampMillis, TopicFilter, UserName,
    },
    types::{DisconnectInfo, OutInflightType},
    utils::timestamp_millis,
    Result,
};
use rmqtt_storage::{DefaultStorageDB, List, Map, StorageList, StorageMap};

use crate::{make_list_stored_key, make_map_stored_key, OfflineMessageOptionType};

pub(crate) const LAST_TIME: &[u8] = b"1";
pub(crate) const DISCONNECT_INFO: &[u8] = b"2";
pub(crate) const SESSION_SUB_MAP: &[u8] = b"3";
pub(crate) const BASIC: &[u8] = b"4";
pub(crate) const INFLIGHT_MESSAGES: &[u8] = b"5";

#[derive(Clone)]
pub(crate) struct StorageSessionManager {
    storage_db: DefaultStorageDB,
    _stored_session_infos: StoredSessionInfos,
}

impl StorageSessionManager {
    pub(crate) fn new(
        storage_db: DefaultStorageDB,
        _stored_session_infos: StoredSessionInfos,
    ) -> StorageSessionManager {
        Self { storage_db, _stored_session_infos }
    }
}

#[async_trait]
impl SessionManager for StorageSessionManager {
    #[allow(clippy::too_many_arguments)]
    async fn create(
        &self,
        id: Id,
        scx: ServerContext,
        listen_cfg: ListenerConfig,
        fitter: FitterType,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        outinflight: OutInflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,

        last_id: Option<Id>,
    ) -> Result<Arc<dyn SessionLike>> {
        let clean_start = conn_info.clean_start();
        let inner = DefaultSession::new(
            id,
            scx,
            listen_cfg,
            subscriptions,
            deliver_queue,
            outinflight,
            conn_info,
            created_at,
            connected_at,
            session_present,
            superuser,
            connected,
            disconnect_info,
        );

        if clean_start {
            Ok(Arc::new(inner))
        } else {
            let id_str = inner.id().to_string();
            let session_info_map = self.storage_db.map(make_map_stored_key(id_str.as_str()), None).await?;
            let offline_messages_list =
                self.storage_db.list(make_list_stored_key(id_str.as_str()), None).await?;

            //Only when 'clean_session' is equal to false or 'clean_start' is equal to false, the
            // session information persistence feature will be initiated.
            let s = Arc::new(StorageSession::new(
                inner,
                fitter,
                self.storage_db.clone(),
                session_info_map,
                offline_messages_list,
            ));
            if connected {
                let s1 = s.clone();
                tokio::spawn(async move {
                    if let Err(e) = s1.save_to_db().await {
                        log::error!("Save session info error to db, {:?}", e);
                    }
                    if let Some(last_id) = last_id {
                        log::debug!("Remove last offline session info from db, last_id: {:?}", last_id,);

                        let map = s1.storage_db.map(make_map_stored_key(last_id.to_string()), None).await;
                        let list = s1.storage_db.list(make_list_stored_key(last_id.to_string()), None).await;

                        if let Ok(map) = map {
                            if let Err(e) = map.clear().await {
                                log::warn!(
                                    "Remove last offline session info error from db, last_id: {:?}, {:?}",
                                    last_id,
                                    e
                                );
                            }
                        }

                        if let Ok(list) = list {
                            if let Err(e) = list.clear().await {
                                log::warn!(
                                    "Remove last offline session info error from db, last_id: {:?}, {:?}",
                                    last_id,
                                    e
                                );
                            }
                        }
                    }
                });
            }
            Ok(s)
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Basic {
    pub id: Id,
    #[serde(
        serialize_with = "Basic::serialize_conn_info",
        deserialize_with = "Basic::deserialize_conn_info"
    )]
    pub conn_info: Arc<ConnectInfo>,
    pub created_at: TimestampMillis,
    pub connected_at: TimestampMillis,
}

impl Basic {
    #[inline]
    fn serialize_conn_info<S>(conn_info: &Arc<ConnectInfo>, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        conn_info.as_ref().serialize(s)
    }

    #[inline]
    pub fn deserialize_conn_info<'de, D>(deserializer: D) -> std::result::Result<Arc<ConnectInfo>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Arc::new(ConnectInfo::deserialize(deserializer)?))
    }
}

pub struct StorageSession {
    inner: DefaultSession,
    fitter: FitterType,
    //----------------------------------
    storage_db: DefaultStorageDB,
    session_info_map: StorageMap,
    offline_messages_list: StorageList,
    last_time: AtomicI64,
}

impl StorageSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        inner: DefaultSession,
        fitter: FitterType,
        storage_db: DefaultStorageDB,
        session_info_map: StorageMap,
        offline_messages_list: StorageList,
    ) -> Self {
        Self {
            inner,
            fitter,
            storage_db,
            session_info_map,
            offline_messages_list,
            last_time: AtomicI64::new(timestamp_millis()),
        }
    }

    #[inline]
    async fn update_last_time(&self, save_enable: bool) {
        let now = timestamp_millis();
        let old = self.last_time.swap(now, Ordering::SeqCst);
        if save_enable || (now - old) > (1000 * 60) {
            let id = self.id().clone();
            let session_info_map = self.session_info_map.clone();
            tokio::spawn(async move { Self::_update_last_time(&id, session_info_map, now).await });
            log::debug!("{:?} update last time", self.id());
        }
    }

    #[inline]
    async fn _update_last_time(id: &Id, session_info_map: StorageMap, now: TimestampMillis) {
        if let Err(e) = session_info_map.insert(LAST_TIME, &now).await {
            log::warn!("{:?} save last time to db error, {}", id, e);
        }
    }

    #[inline]
    pub(crate) async fn delete_from_db(&self) {
        let id = self.id().clone();
        let session_info_map = self.session_info_map.clone();
        let offline_messages_list = self.offline_messages_list.clone();
        tokio::spawn(async move {
            Self::_delete_from_db(&id, &session_info_map, &offline_messages_list).await;
        });
    }

    #[inline]
    async fn _delete_from_db(id: &Id, session_info_map: &StorageMap, offline_messages_list: &StorageList) {
        if let Err(e) = session_info_map.clear().await {
            log::error!("{:?} remove session info error from db, {}", id, e);
        }
        if let Err(e) = offline_messages_list.clear().await {
            log::error!("{:?} remove session offline messages error from db, {}", id, e);
        }
    }

    #[inline]
    pub(crate) async fn save_to_db(&self) -> Result<()> {
        self.update_last_time(true).await;
        self.save_basic_info().await;
        self.save_subscriptions().await;
        log::debug!("{:?} save to db ...", self.id());
        Ok(())
    }

    #[inline]
    async fn save_basic_info(&self) {
        if let Err(e) = self._save_basic_info().await {
            log::error!("save basic info error, {:?}", e);
        }
    }

    #[inline]
    async fn _save_basic_info(&self) -> Result<()> {
        let basic = Basic {
            id: self.id().clone(),
            conn_info: self.connect_info().await?,
            created_at: self.created_at().await?,
            connected_at: self.connected_at().await?,
        };
        self.session_info_map.insert(BASIC, &basic).await?;
        Ok(())
    }

    #[inline]
    async fn save_subscriptions(&self) {
        let subs = self.inner.subscriptions.clone();
        let session_info_map = self.session_info_map.clone();
        tokio::spawn(async move {
            let subs = subs.read().await.clone();
            match tokio::time::timeout(
                Duration::from_secs(8),
                Self::_save_subscriptions(&session_info_map, &subs),
            )
            .await
            {
                Ok(()) => {}
                Err(e) => {
                    log::error!("save subscriptions error, {}", e);
                }
            }
        });
    }

    #[inline]
    async fn _save_subscriptions(session_info_map: &StorageMap, subs: &SessionSubMap) {
        if let Err(e) = session_info_map.insert(SESSION_SUB_MAP, subs).await {
            log::error!("save subscriptions error, {}", e);
        }
    }

    #[inline]
    async fn _subscriptions_clear(&self) -> Result<()> {
        self.inner.subscriptions.clear(self.context()).await;
        Ok(())
    }

    #[inline]
    async fn save_disconnect_info(&self) {
        let session_info_map = self.session_info_map.clone();
        let disconnect_info = self.inner.disconnect_info.read().await.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::_save_disconnect_info(session_info_map, &disconnect_info).await {
                log::error!("save disconnect info error, {}", e);
            }
        });
    }

    #[inline]
    async fn _save_disconnect_info(
        session_info_map: StorageMap,
        disconnect_info: &DisconnectInfo,
    ) -> Result<()> {
        session_info_map.insert(DISCONNECT_INFO, disconnect_info).await?;
        Ok(())
    }

    #[inline]
    async fn _disconnect_info(&self) -> DisconnectInfo {
        self.inner.disconnect_info.read().await.clone()
    }

    #[inline]
    async fn _set_map_stored_key_ttl(
        id: &Id,
        session_info_map: &StorageMap,
        session_expiry_interval_millis: i64,
    ) {
        match session_info_map.expire(session_expiry_interval_millis).await {
            Err(e) => {
                log::warn!("{:?} set map ttl to db error, {}", id, e);
            }
            Ok(res) => {
                log::debug!(
                    "{:?} set map ttl to db ok, {:?}, {}",
                    id,
                    Duration::from_millis(session_expiry_interval_millis as u64),
                    res
                );
            }
        }
    }

    #[inline]
    async fn _set_list_stored_key_ttl(
        id: &Id,
        offline_messages_list: &StorageList,
        session_expiry_interval_millis: i64,
    ) {
        match offline_messages_list.expire(session_expiry_interval_millis).await {
            Err(e) => {
                log::warn!("{:?} set list ttl to db error, {}", id, e);
            }
            Ok(res) => {
                log::debug!(
                    "{:?} set list ttl to db ok, {:?}, {}",
                    id,
                    Duration::from_millis(session_expiry_interval_millis as u64),
                    res
                );
            }
        }
    }

    #[inline]
    async fn _disconnected_set(
        id: &Id,
        offline_messages_list: StorageList,
        session_info_map: StorageMap,
        disconnect_info: DisconnectInfo,
        session_expiry_interval: i64,
    ) -> Result<()> {
        Self::_set_map_stored_key_ttl(id, &session_info_map, session_expiry_interval).await;
        match offline_messages_list.push::<OfflineMessageOptionType>(&None).await {
            Ok(()) => {
                Self::_set_list_stored_key_ttl(id, &offline_messages_list, session_expiry_interval).await;
            }
            Err(e) => {
                log::warn!("{:?} save offline messages error, {}", id, e)
            }
        }

        Self::_save_disconnect_info(session_info_map, &disconnect_info).await?;

        Ok(())
    }
}

#[async_trait]
impl SessionLike for StorageSession {
    #[inline]
    fn id(&self) -> &Id {
        self.inner.id()
    }

    #[inline]
    fn context(&self) -> &ServerContext {
        self.inner.context()
    }

    #[inline]
    fn listen_cfg(&self) -> &ListenerConfig {
        self.inner.listen_cfg()
    }

    #[inline]
    fn deliver_queue(&self) -> &MessageQueueType {
        self.inner.deliver_queue()
    }

    #[inline]
    fn out_inflight(&self) -> &OutInflightType {
        self.inner.out_inflight()
    }

    #[inline]
    async fn subscriptions(&self) -> Result<SessionSubs> {
        self.inner.subscriptions().await
    }

    #[inline]
    async fn subscriptions_add(
        &self,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Result<Option<SubscriptionOptions>> {
        let opts = self.inner.subscriptions_add(topic_filter, opts).await?;
        self.save_subscriptions().await;
        Ok(opts)
    }

    #[inline]
    async fn subscriptions_remove(
        &self,
        topic_filter: &str,
    ) -> Result<Option<(TopicFilter, SubscriptionOptions)>> {
        let sub = self.inner.subscriptions_remove(topic_filter).await?;
        self.save_subscriptions().await;
        Ok(sub)
    }

    #[inline]
    async fn subscriptions_drain(&self) -> Result<Subscriptions> {
        let subs = self.inner.subscriptions_drain().await?;
        self.save_subscriptions().await;
        Ok(subs)
    }

    #[inline]
    async fn subscriptions_extend(&self, other: Subscriptions) -> Result<()> {
        self.inner.subscriptions_extend(other).await?;
        self.save_subscriptions().await;
        Ok(())
    }

    #[inline]
    async fn created_at(&self) -> Result<TimestampMillis> {
        self.inner.created_at().await
    }

    #[inline]
    async fn session_present(&self) -> Result<bool> {
        self.inner.session_present().await
    }

    #[inline]
    async fn connect_info(&self) -> Result<Arc<ConnectInfo>> {
        self.inner.connect_info().await
    }

    #[inline]
    fn username(&self) -> Option<&UserName> {
        self.inner.username()
    }

    #[inline]
    fn password(&self) -> Option<&Password> {
        self.inner.password()
    }

    #[inline]
    async fn protocol(&self) -> Result<u8> {
        self.inner.protocol().await
    }

    #[inline]
    async fn superuser(&self) -> Result<bool> {
        self.inner.superuser().await
    }

    #[inline]
    async fn connected(&self) -> Result<bool> {
        self.inner.connected().await
    }

    #[inline]
    async fn connected_at(&self) -> Result<TimestampMillis> {
        self.inner.connected_at().await
    }

    #[inline]
    async fn disconnected_at(&self) -> Result<TimestampMillis> {
        self.inner.disconnected_at().await
    }
    #[inline]
    async fn disconnected_reasons(&self) -> Result<Vec<Reason>> {
        self.inner.disconnected_reasons().await
    }
    #[inline]
    async fn disconnected_reason(&self) -> Result<Reason> {
        self.inner.disconnected_reason().await
    }
    #[inline]
    async fn disconnected_reason_has(&self) -> bool {
        self.inner.disconnected_reason_has().await
    }
    #[inline]
    async fn disconnected_reason_add(&self, r: Reason) -> Result<()> {
        self.inner.disconnected_reason_add(r).await?;
        self.save_disconnect_info().await;
        log::debug!("{:?} disconnected_reason_add ... ", self.id());
        Ok(())
    }
    #[inline]
    async fn disconnected_reason_take(&self) -> Result<Reason> {
        let r = self.inner.disconnected_reason_take().await;
        log::debug!("{:?} disconnected_reason_take ... ", self.id());
        r
    }
    #[inline]
    async fn disconnect(&self) -> Result<Option<Disconnect>> {
        self.inner.disconnect().await
    }
    #[inline]
    async fn disconnected_set(&self, d: Option<Disconnect>, reason: Option<Reason>) -> Result<()> {
        let session_expiry_interval = self.fitter.session_expiry_interval(d.as_ref()).as_millis() as i64;
        log::debug!(
            "{:?} disconnected_set session_expiry_interval: {:?}",
            self.id(),
            session_expiry_interval
        );

        self.inner.disconnected_set(d, reason).await?;

        let id = self.id().clone();
        let offline_messages_list = self.offline_messages_list.clone();
        let session_info_map = self.session_info_map.clone();
        let disconnect_info = self._disconnect_info().await;

        tokio::spawn(async move {
            if let Err(e) = Self::_disconnected_set(
                &id,
                offline_messages_list,
                session_info_map,
                disconnect_info,
                session_expiry_interval,
            )
            .await
            {
                log::error!("{:?} disconnected set error, {}", id, e)
            }
        });

        log::debug!("{:?} disconnected_set ... ", self.id());
        Ok(())
    }

    #[inline]
    async fn on_drop(&self) -> Result<()> {
        log::debug!("{:?} StorageSession on_drop ...", self.id());
        if let Err(e) = self._subscriptions_clear().await {
            log::error!("{:?} subscriptions clear error, {}", self.id(), e);
        }
        self.delete_from_db().await;
        Ok(())
    }

    #[inline]
    async fn keepalive(&self, ping: IsPing) {
        log::debug!("ping: {}", ping);
        if ping {
            self.update_last_time(true).await;
        } else {
            self.update_last_time(false).await;
        }
    }
}

// const SESSION_PRESENT: u8 = 0b00000001;
// const SUPERUSER: u8 = 0b00000010;
// const CONNECTED: u8 = 0b00000100;
// const REMOVE_AND_SAVE: u8 = 0b00001000;

//const EMPTY: u8 = u8::MIN;
//const ALL: u8 = u8::MAX;

pub trait AtomicFlags {
    type T;
    #[allow(dead_code)]
    fn empty() -> Self;
    #[allow(dead_code)]
    fn all() -> Self;
    fn get(&self) -> Self::T;
    #[allow(dead_code)]
    fn insert(&self, other: Self::T);
    #[allow(dead_code)]
    fn contains(&self, other: Self::T) -> bool;
    #[allow(dead_code)]
    fn remove(&self, other: Self::T);
    #[allow(dead_code)]
    fn equal_exchange(
        &self,
        current: Self::T,
        new: Self::T,
        mask: Self::T,
    ) -> std::result::Result<Self::T, Self::T>;
    fn difference(&self, other: Self::T) -> Self::T;
}

impl AtomicFlags for AtomicU8 {
    type T = u8;

    #[inline]
    fn empty() -> Self {
        AtomicU8::new(0)
    }

    #[inline]
    fn all() -> Self {
        AtomicU8::new(0xff)
    }

    #[inline]
    fn get(&self) -> Self::T {
        self.load(Ordering::SeqCst)
    }

    #[inline]
    fn insert(&self, other: Self::T) {
        self.fetch_or(other, Ordering::SeqCst);
    }

    #[inline]
    fn contains(&self, other: Self::T) -> bool {
        self.get() & other == other
    }

    #[inline]
    fn remove(&self, other: Self::T) {
        self.store(self.difference(other), Ordering::SeqCst);
    }

    #[inline]
    fn equal_exchange(
        &self,
        current: Self::T,
        new: Self::T,
        mask: Self::T,
    ) -> std::result::Result<Self::T, Self::T> {
        self.fetch_update(Ordering::SeqCst, Ordering::SeqCst, move |v| {
            let flags = current & mask;
            if (v & mask) == flags {
                Some((v & !flags) | (new & mask))
            } else {
                None
            }
        })
    }

    #[inline]
    fn difference(&self, other: Self::T) -> Self::T {
        self.get() & !other
    }
}

pub(crate) type StoredKey = Bytes;

pub(crate) struct StoredSessionInfo {
    pub id_key: StoredKey,
    pub basic: Basic,
    pub subs: Option<SessionSubMap>,
    pub disconnect_info: Option<DisconnectInfo>,
    pub offline_messages: Vec<(From, Publish)>,
    pub inflight_messages: Vec<OutInflightMessage>,
    pub last_time: TimestampMillis,
}

impl StoredSessionInfo {
    #[inline]
    pub fn from(id_key: StoredKey, basic: Basic) -> Self {
        let last_time = basic.connected_at;
        Self {
            id_key,
            basic,
            subs: None,
            disconnect_info: None,
            offline_messages: Vec::new(),
            inflight_messages: Vec::new(),
            last_time,
        }
    }

    #[inline]
    #[allow(clippy::mutable_key_type)]
    pub fn set_subs(&mut self, subs: SessionSubMap) {
        self.subs.replace(subs);
    }

    #[inline]
    pub fn set_disconnect_info(&mut self, disconnect_info: DisconnectInfo) {
        self.disconnect_info.replace(disconnect_info);
    }

    #[inline]
    pub fn set_last_time(&mut self, last_time: TimestampMillis) {
        if self.last_time < last_time {
            self.last_time = last_time;
        }
    }
}

#[derive(Clone)]
pub(crate) struct StoredSessionInfos(Arc<DashMap<ClientId, Vec<StoredSessionInfo>>>);

impl Deref for StoredSessionInfos {
    type Target = Arc<DashMap<ClientId, Vec<StoredSessionInfo>>>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl StoredSessionInfos {
    #[inline]
    pub fn new() -> Self {
        Self(Arc::new(DashMap::default()))
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn add(&mut self, stored: StoredSessionInfo) {
        self.0.entry(stored.basic.id.client_id.clone()).or_default().push(stored);
    }

    #[inline]
    pub fn set_offline_messages(
        &mut self,
        id_key: StoredKey,
        offline_messages: Vec<OfflineMessageOptionType>,
    ) -> bool {
        let mut exist = false;
        log::debug!("set_offline_messages id_key: {:?}", id_key);
        for (cid, f, p) in offline_messages.into_iter().flatten() {
            if let Some(mut entry) = self.0.get_mut(&cid) {
                let storeds = entry.value_mut();
                for stored in storeds {
                    if stored.id_key == id_key {
                        exist = true;
                        stored.offline_messages.push((f, p));
                        break;
                    }
                }
            }
        }
        exist
    }

    #[inline]
    pub fn retain_latests(&mut self) -> Vec<StoredKey> {
        let mut removeds = Vec::new();
        for mut entry in self.0.iter_mut() {
            let storeds = entry.value_mut();
            if storeds.len() > 1 {
                if let Some(mut latest) = storeds.pop() {
                    while let Some(stored) = storeds.pop() {
                        if stored.last_time > latest.last_time {
                            removeds.push(latest.id_key);
                            latest = stored;
                        } else {
                            removeds.push(stored.id_key);
                        }
                    }
                    storeds.push(latest);
                }
            }
        }
        log::info!("retain_latests removeds: {:?}", removeds.len());
        removeds
    }
}
