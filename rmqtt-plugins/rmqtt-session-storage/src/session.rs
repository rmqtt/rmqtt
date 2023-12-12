use serde::{Deserialize, Deserializer, Serialize, Serializer};

use std::convert::From as _;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmqtt::{
    async_trait::async_trait,
    chrono, log,
    once_cell::sync::OnceCell,
    tokio,
    tokio::sync::{RwLock, RwLockWriteGuard},
    DashMap,
};
use rmqtt::{
    broker::default::DefaultSessionManager,
    broker::inflight::InflightMessage,
    broker::session::{SessionLike, SessionManager},
    broker::types::DisconnectInfo,
    settings::Listener,
    ClientId, ConnectInfo, ConnectInfoType, Disconnect, FitterType, From, Id, InflightType, IsPing,
    MessageQueueType, MqttError, Password, Publish, Reason, Result, SessionSubMap, SessionSubs,
    SubscriptionOptions, Subscriptions, TimestampMillis, TopicFilter, UserName,
};

use crate::{make_list_stored_key, make_map_stored_key, OfflineMessageOptionType};
use rmqtt::bytes::Bytes;
use rmqtt_storage::{DefaultStorageDB, List, Map, StorageList, StorageMap};

pub(crate) const LAST_TIME: &[u8] = b"1";
pub(crate) const DISCONNECT_INFO: &[u8] = b"2";
pub(crate) const SESSION_SUB_MAP: &[u8] = b"3";
pub(crate) const BASIC: &[u8] = b"4";
pub(crate) const INFLIGHT_MESSAGES: &[u8] = b"5";

pub(crate) struct StorageSessionManager {
    storage_db: DefaultStorageDB,
    _stored_session_infos: StoredSessionInfos,
}

impl StorageSessionManager {
    #[inline]
    pub(crate) fn get_or_init(
        storage_db: DefaultStorageDB,
        _stored_session_infos: StoredSessionInfos,
    ) -> &'static StorageSessionManager {
        static INSTANCE: OnceCell<StorageSessionManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { storage_db, _stored_session_infos })
    }
}

impl SessionManager for &'static StorageSessionManager {
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        id: Id,
        listen_cfg: Listener,
        fitter: FitterType,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: InflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,

        last_id: Option<Id>,
    ) -> Arc<dyn SessionLike> {
        if conn_info.clean_start() {
            DefaultSessionManager::instance().create(
                id,
                listen_cfg,
                fitter,
                subscriptions,
                deliver_queue,
                inflight_win,
                conn_info,
                created_at,
                connected_at,
                session_present,
                superuser,
                connected,
                disconnect_info,
                last_id,
            )
        } else {
            let session_info_map = self.storage_db.map(make_map_stored_key(id.to_string()));
            let offline_messages_list = self.storage_db.list(make_list_stored_key(id.to_string()));

            //Only when 'clean_session' is equal to false or 'clean_start' is equal to false, the
            // session information persistence feature will be initiated.
            let s = Arc::new(StorageSession::new(
                id,
                listen_cfg,
                fitter,
                subscriptions,
                deliver_queue,
                inflight_win,
                conn_info,
                created_at,
                connected_at,
                session_present,
                superuser,
                connected,
                disconnect_info,
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
                        log::debug!(
                            "last session_info_map.is_empty(): {:?}",
                            s1.storage_db
                                .clone()
                                .map(make_map_stored_key(last_id.to_string()))
                                .is_empty()
                                .await
                        );
                        log::debug!(
                            "last offline_messages_list.len(): {:?}",
                            s1.storage_db.clone().list(make_list_stored_key(last_id.to_string())).len().await
                        );

                        log::debug!("Remove last offline session info from db, last_id: {:?}", last_id,);
                        if let Err(e) = s1.storage_db.clone().remove(last_id.to_string().as_bytes()).await {
                            log::warn!(
                                "Remove last offline session info error from db, last_id: {:?}, {:?}",
                                last_id,
                                e
                            );
                        }
                    }
                });
            }
            s
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
    id: Id,
    listen_cfg: Listener,
    fitter: FitterType,
    deliver_queue: MessageQueueType,
    inflight_win: InflightType,

    basic: RwLock<Option<Basic>>,
    subscriptions: RwLock<Option<SessionSubs>>,

    state_flags: AtomicU8,
    password: Option<Password>,
    disconnect_info: RwLock<Option<DisconnectInfo>>,

    //----------------------------------
    storage_db: DefaultStorageDB,
    session_info_map: StorageMap,
    offline_messages_list: StorageList,
    last_time: AtomicI64,
}

impl StorageSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: Id,
        listen_cfg: Listener,
        fitter: FitterType,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: InflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,

        storage_db: DefaultStorageDB,
        session_info_map: StorageMap,
        offline_messages_list: StorageList,
    ) -> Self {
        let state_flags = AtomicU8::empty();
        if session_present {
            state_flags.insert(SESSION_PRESENT);
        }
        if superuser {
            state_flags.insert(SUPERUSER);
        }
        if connected {
            state_flags.insert(CONNECTED);
        }

        let basic = Basic { id: id.clone(), conn_info, created_at, connected_at };
        let password = basic.conn_info.password().cloned();
        let disconnect_info = disconnect_info.unwrap_or_default();

        Self {
            id,
            listen_cfg,
            fitter,
            deliver_queue,
            inflight_win,

            basic: RwLock::new(Some(basic)),
            subscriptions: RwLock::new(Some(subscriptions)),

            state_flags,
            password,
            disconnect_info: RwLock::new(Some(disconnect_info)),

            storage_db,
            session_info_map,
            offline_messages_list,
            last_time: AtomicI64::new(chrono::Local::now().timestamp_millis()),
        }
    }

    #[inline]
    async fn update_last_time(&self, save_enable: bool) {
        let now = chrono::Local::now().timestamp_millis();
        let old = self.last_time.swap(now, Ordering::SeqCst);
        if save_enable || (now - old) > (1000 * 60) {
            if let Err(e) = self.session_info_map.insert(LAST_TIME, &now).await {
                log::warn!("{:?} save last time to db error, {:?}", self.id, e);
            }
            log::debug!("{:?} update last time", self.id);
        }
    }

    #[inline]
    fn is_inactive(&self) -> bool {
        (chrono::Local::now().timestamp_millis() - self.last_time.load(Ordering::SeqCst)) > 1000 * 60 * 3
    }

    #[inline]
    pub(crate) async fn delete_from_db(&self) -> Result<()> {
        if let Err(e) = self.session_info_map.clear().await {
            log::error!("{:?} remove session info error from db, {:?}", self.id, e);
        }
        if let Err(e) = self.offline_messages_list.clear().await {
            log::error!("{:?} remove session offline messages error from db, {:?}", self.id, e);
        }
        Ok(())
    }

    #[inline]
    pub(crate) async fn remove_and_save_to_db(&self) -> Result<()> {
        log::debug!(
            "{:?} remove and save to db ..., {:08b} {:08b} {:08b}",
            self.id,
            self.state_flags.get(),
            !REMOVE_AND_SAVE,
            REMOVE_AND_SAVE
        );
        if self.state_flags.equal_exchange(!REMOVE_AND_SAVE, REMOVE_AND_SAVE, REMOVE_AND_SAVE).is_err() {
            log::debug!("{:?} remove and save to db ..., err: {:08b}", self.id, self.state_flags.get());
            return Ok(());
        }
        log::debug!("{:?} remove and save to db ..., ok: {:08b}", self.id, self.state_flags.get());

        self.save_and_remove_basic_info().await?;
        self.save_and_remove_subscriptions().await?;
        self.save_and_remove_disconnect_info().await?;
        Ok(())
    }

    #[inline]
    pub(crate) async fn save_to_db(&self) -> Result<()> {
        self.update_last_time(true).await;
        self.save_basic_info().await?;
        self.save_subscriptions().await?;
        log::debug!("{:?} save to db ...", self.id);
        Ok(())
    }

    #[inline]
    async fn save_basic_info(&self) -> Result<bool> {
        self.state_flags.remove(REMOVE_AND_SAVE);
        self._save_basic_info().await
    }

    #[inline]
    async fn _save_basic_info(&self) -> Result<bool> {
        let res = if let Some(basic) = self.basic.read().await.as_ref() {
            log::debug!("{:?} save basic info to db", self.id);
            self.session_info_map.insert(BASIC, basic).await?;
            true
        } else {
            false
        };
        Ok(res)
    }

    #[inline]
    async fn save_and_remove_basic_info(&self) -> Result<bool> {
        let res = self._save_basic_info().await?;
        if res {
            self.basic.write().await.take();
        }
        Ok(res)
    }

    #[inline]
    async fn _load_basic_info(&self, tag: &str) -> Result<Option<Basic>> {
        log::debug!("{:?} read basic info from storage ..., {}", self.id, tag);
        if let Some(basic) = self.session_info_map.get::<_, Basic>(BASIC).await? {
            self.basic.write().await.replace(basic.clone());
            Ok(Some(basic))
        } else {
            Ok(None)
        }
    }

    #[inline]
    async fn save_subscriptions(&self) -> Result<bool> {
        self.state_flags.remove(REMOVE_AND_SAVE);
        self._save_subscriptions().await
    }

    #[inline]
    async fn _save_subscriptions(&self) -> Result<bool> {
        let res = if let Some(subscriptions) = self.subscriptions.read().await.as_ref() {
            log::debug!("{:?} save subscriptions to db", self.id);
            let subs = subscriptions.read().await;
            self.session_info_map.insert(SESSION_SUB_MAP, subs.deref()).await?;
            true
        } else {
            false
        };
        Ok(res)
    }

    #[inline]
    async fn save_and_remove_subscriptions(&self) -> Result<bool> {
        let res = self._save_subscriptions().await?;
        if res {
            self.subscriptions.write().await.take();
        }
        Ok(res)
    }

    #[inline]
    async fn _subscriptions_clear(&self) -> Result<()> {
        if let Some(subs) = self.subscriptions.read().await.as_ref() {
            subs._clear().await;
            return Ok(());
        }

        if let Some(subs) = self._load_subscriptions("_subscriptions_clear").await? {
            subs._clear().await;
            return Ok(());
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn _load_subscriptions(&self, tag: &str) -> Result<Option<SessionSubs>> {
        log::debug!("{:?} read subscriptions info from storage ..., {}", self.id, tag);
        if let Some(subs) = self.session_info_map.get::<_, SessionSubMap>(SESSION_SUB_MAP).await? {
            let subs = SessionSubs::from(subs);
            self.subscriptions.write().await.replace(subs.clone());
            Ok(Some(subs))
        } else {
            Ok(None)
        }
    }

    #[inline]
    async fn save_disconnect_info(&self) -> Result<bool> {
        self.state_flags.remove(REMOVE_AND_SAVE);
        self._save_disconnect_info().await
    }

    #[inline]
    async fn _save_disconnect_info(&self) -> Result<bool> {
        let res = if let Some(disconnect_info) = self.disconnect_info.read().await.as_ref() {
            log::debug!("{:?} save disconnect info to db", self.id);
            self.session_info_map.insert(DISCONNECT_INFO, disconnect_info).await?;
            true
        } else {
            false
        };
        Ok(res)
    }

    #[inline]
    async fn save_and_remove_disconnect_info(&self) -> Result<bool> {
        let res = self._save_disconnect_info().await?;
        if res {
            self.disconnect_info.write().await.take();
        }
        Ok(res)
    }

    #[inline]
    async fn _load_disconnect_info(
        &self,
        tag: &str,
    ) -> Result<Option<RwLockWriteGuard<'_, Option<DisconnectInfo>>>> {
        log::debug!("{:?} read disconnect info from storage ..., {}", self.id, tag);
        if let Some(disconnect_info) = self.session_info_map.get::<_, DisconnectInfo>(DISCONNECT_INFO).await?
        {
            let mut disconnect_info_wl = self.disconnect_info.write().await;
            disconnect_info_wl.replace(disconnect_info);
            Ok(Some(disconnect_info_wl))
        } else {
            Ok(None)
        }
    }

    #[inline]
    async fn disconnect_info_wl(&self, tag: &str) -> Result<RwLockWriteGuard<'_, Option<DisconnectInfo>>> {
        {
            let disconnect_info_wl = self.disconnect_info.write().await;
            if disconnect_info_wl.is_some() {
                return Ok(disconnect_info_wl);
            }
            drop(disconnect_info_wl);
        }
        {
            if let Some(wl) = self._load_disconnect_info(tag).await? {
                return Ok(wl);
            }
        }
        let wl = self.disconnect_info.write().await;
        Ok(wl)
    }

    #[inline]
    async fn set_map_stored_key_ttl(&self, session_expiry_interval_millis: i64) {
        match self
            .storage_db
            .clone()
            .expire(make_map_stored_key(self.id.to_string()), session_expiry_interval_millis)
            .await
        {
            Err(e) => {
                log::warn!("{:?} set map ttl to db error, {:?}", self.id, e);
            }
            Ok(res) => {
                log::debug!(
                    "{:?} set map ttl to db ok, {:?}, {}",
                    self.id,
                    Duration::from_millis(session_expiry_interval_millis as u64),
                    res
                );
            }
        }
    }

    #[inline]
    async fn set_list_stored_key_ttl(&self, session_expiry_interval_millis: i64) {
        match self
            .storage_db
            .clone()
            .expire(make_list_stored_key(self.id.to_string()), session_expiry_interval_millis)
            .await
        {
            Err(e) => {
                log::warn!("{:?} set list ttl to db error, {:?}", self.id, e);
            }
            Ok(res) => {
                log::debug!(
                    "{:?} set list ttl to db ok, {:?}, {}",
                    self.id,
                    Duration::from_millis(session_expiry_interval_millis as u64),
                    res
                );
            }
        }
    }

    #[inline]
    fn not_found_error() -> MqttError {
        MqttError::from("not found from storage")
    }
}

#[async_trait]
impl SessionLike for StorageSession {
    fn id(&self) -> &Id {
        &self.id
    }

    #[inline]
    fn listen_cfg(&self) -> &Listener {
        &self.listen_cfg
    }

    #[inline]
    fn deliver_queue(&self) -> &MessageQueueType {
        &self.deliver_queue
    }

    #[inline]
    fn inflight_win(&self) -> &InflightType {
        &self.inflight_win
    }

    #[inline]
    async fn subscriptions(&self) -> Result<SessionSubs> {
        if let Some(subscriptions) = self.subscriptions.read().await.as_ref() {
            return Ok(subscriptions.clone());
        }

        if let Some(subs) = self._load_subscriptions("subscriptions").await? {
            return Ok(subs);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn subscriptions_add(
        &self,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Result<Option<SubscriptionOptions>> {
        log::debug!("{:?} subscriptions_add, topic_filter: {}, opts: {:?}", self.id, topic_filter, opts);
        if let Some(subs) = self.subscriptions.read().await.as_ref() {
            let res = subs._add(topic_filter, opts).await;
            self.save_subscriptions().await?;
            return Ok(res);
        }

        if let Some(subs) = self._load_subscriptions("subscriptions_add").await? {
            let res = subs._add(topic_filter, opts).await;
            self.save_subscriptions().await?;
            return Ok(res);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn subscriptions_remove(
        &self,
        topic_filter: &str,
    ) -> Result<Option<(TopicFilter, SubscriptionOptions)>> {
        log::debug!("{:?} subscriptions_remove, topic_filter: {}", self.id, topic_filter);
        if let Some(subs) = self.subscriptions.read().await.as_ref() {
            let res = subs._remove(topic_filter).await;
            self.save_subscriptions().await?;
            return Ok(res);
        }

        if let Some(subs) = self._load_subscriptions("subscriptions_remove").await? {
            let res = subs._remove(topic_filter).await;
            self.save_subscriptions().await?;
            return Ok(res);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn subscriptions_drain(&self) -> Result<Subscriptions> {
        if let Some(subs) = self.subscriptions.read().await.as_ref() {
            let res = subs._drain().await;
            self.save_subscriptions().await?;
            return Ok(res);
        }

        if let Some(subs) = self._load_subscriptions("subscriptions_drain").await? {
            let res = subs._drain().await;
            self.save_subscriptions().await?;
            return Ok(res);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn subscriptions_extend(&self, other: Subscriptions) -> Result<()> {
        if let Some(subs) = self.subscriptions.read().await.as_ref() {
            subs._extend(other).await;
            self.save_subscriptions().await?;
            return Ok(());
        }

        if let Some(subs) = self._load_subscriptions("subscriptions_extend").await? {
            subs._extend(other).await;
            self.save_subscriptions().await?;
            return Ok(());
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn created_at(&self) -> Result<TimestampMillis> {
        if let Some(basic) = self.basic.read().await.as_ref() {
            return Ok(basic.created_at);
        }

        if let Some(basic) = self._load_basic_info("basic(created_at)").await? {
            return Ok(basic.created_at);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn session_present(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SESSION_PRESENT))
    }

    #[inline]
    async fn connect_info(&self) -> Result<Arc<ConnectInfo>> {
        if let Some(basic) = self.basic.read().await.as_ref() {
            return Ok(basic.conn_info.clone());
        }

        if let Some(basic) = self._load_basic_info("basic(connect_info)").await? {
            return Ok(basic.conn_info);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    fn username(&self) -> Option<&UserName> {
        self.id.username.as_ref()
    }

    #[inline]
    fn password(&self) -> Option<&Password> {
        self.password.as_ref()
    }

    #[inline]
    async fn protocol(&self) -> Result<u8> {
        Ok(self.connect_info().await.ok().map(|c| c.proto_ver()).unwrap_or_default())
    }

    #[inline]
    async fn superuser(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SUPERUSER))
    }

    #[inline]
    async fn connected(&self) -> Result<bool> {
        Ok(self.state_flags.contains(CONNECTED))
    }

    #[inline]
    async fn connected_at(&self) -> Result<TimestampMillis> {
        if let Some(basic) = self.basic.read().await.as_ref() {
            return Ok(basic.connected_at);
        }

        if let Some(basic) = self._load_basic_info("basic(connected_at)").await? {
            return Ok(basic.connected_at);
        }

        Err(Self::not_found_error())
    }

    #[inline]
    async fn disconnected_at(&self) -> Result<TimestampMillis> {
        if let Some(disconnect_info) = self.disconnect_info.read().await.as_ref() {
            return Ok(disconnect_info.disconnected_at);
        }

        if let Some(wl) = self._load_disconnect_info("disconnect_info(disconnected_at)").await? {
            if let Some(disconnect_info) = wl.as_ref() {
                return Ok(disconnect_info.disconnected_at);
            }
        }

        Err(Self::not_found_error())
    }
    #[inline]
    async fn disconnected_reasons(&self) -> Result<Vec<Reason>> {
        if let Some(disconnect_info) = self.disconnect_info.read().await.as_ref() {
            return Ok(disconnect_info.reasons.clone());
        }

        if let Some(wl) = self._load_disconnect_info("disconnect_info(disconnected_reasons)").await? {
            if let Some(disconnect_info) = wl.as_ref() {
                return Ok(disconnect_info.reasons.clone());
            }
        }

        Err(Self::not_found_error())
    }
    #[inline]
    async fn disconnected_reason(&self) -> Result<Reason> {
        Ok(Reason::Reasons(self.disconnected_reasons().await?))
    }
    #[inline]
    async fn disconnected_reason_has(&self) -> bool {
        if let Some(disconnect_info) = self.disconnect_info.read().await.as_ref() {
            return !disconnect_info.reasons.is_empty();
        }

        if let Some(wl) =
            self._load_disconnect_info("disconnect_info(disconnected_reason_has)").await.unwrap_or_default()
        {
            if let Some(disconnect_info) = wl.as_ref() {
                return !disconnect_info.reasons.is_empty();
            }
        }

        false
    }
    #[inline]
    async fn disconnected_reason_add(&self, r: Reason) -> Result<()> {
        {
            let mut disconnect_info_wl =
                self.disconnect_info_wl("disconnect_info(disconnected_reason_add)").await?;
            let disconnect_info = if let Some(disconnect_info) = disconnect_info_wl.deref_mut() {
                disconnect_info
            } else {
                return Err(Self::not_found_error());
            };
            disconnect_info.reasons.push(r);
            drop(disconnect_info_wl);
        }
        self.save_disconnect_info().await?;
        log::debug!("{:?} disconnected_reason_add ... ", self.id);
        Ok(())
    }
    #[inline]
    async fn disconnected_reason_take(&self) -> Result<Reason> {
        let r = {
            let mut disconnect_info_wl =
                self.disconnect_info_wl("disconnect_info(disconnected_reason_take)").await?;
            let disconnect_info = if let Some(disconnect_info) = disconnect_info_wl.deref_mut() {
                disconnect_info
            } else {
                return Err(Self::not_found_error());
            };
            let r = Reason::Reasons(disconnect_info.reasons.drain(..).collect());
            drop(disconnect_info_wl);
            r
        };
        self.save_disconnect_info().await?;
        log::debug!("{:?} disconnected_reason_take ... ", self.id);
        Ok(r)
    }
    #[inline]
    async fn disconnect(&self) -> Result<Option<Disconnect>> {
        if let Some(disconnect_info) = self.disconnect_info.read().await.as_ref() {
            return Ok(disconnect_info.mqtt_disconnect.clone());
        }

        if let Some(wl) = self._load_disconnect_info("disconnect_info(disconnect)").await? {
            if let Some(disconnect_info) = wl.as_ref() {
                return Ok(disconnect_info.mqtt_disconnect.clone());
            }
        }

        Err(Self::not_found_error())
    }
    #[inline]
    async fn disconnected_set(&self, d: Option<Disconnect>, reason: Option<Reason>) -> Result<()> {
        {
            let mut disconnect_info_wl = self.disconnect_info_wl("disconnect_info(disconnected_set)").await?;
            let disconnect_info = if let Some(disconnect_info) = disconnect_info_wl.deref_mut() {
                disconnect_info
            } else {
                return Err(Self::not_found_error());
            };

            if self.state_flags.contains(CONNECTED) {
                disconnect_info.disconnected_at = chrono::Local::now().timestamp_millis();
                self.state_flags.remove(CONNECTED);
            }

            if let Some(d) = d {
                disconnect_info.reasons.push(d.reason());
                disconnect_info.mqtt_disconnect.replace(d);

                let session_expiry_interval =
                    self.fitter.session_expiry_interval(disconnect_info.mqtt_disconnect.as_ref()).as_millis()
                        as i64;
                log::debug!(
                    "{:?} disconnected_set session_expiry_interval: {:?}",
                    self.id,
                    session_expiry_interval
                );
                self.set_map_stored_key_ttl(session_expiry_interval).await;
                match self.offline_messages_list.push::<OfflineMessageOptionType>(&None).await {
                    Ok(()) => {
                        self.set_list_stored_key_ttl(session_expiry_interval).await;
                    }
                    Err(e) => {
                        log::warn!("{:?} save offline messages error, {:?}", self.id, e)
                    }
                }
            }

            if let Some(reason) = reason {
                disconnect_info.reasons.push(reason);
            }
            drop(disconnect_info_wl);
        }
        self.save_disconnect_info().await?;
        log::debug!("{:?} disconnected_set ... ", self.id);
        Ok(())
    }

    #[inline]
    async fn on_drop(&self) -> Result<()> {
        log::debug!("{:?} StorageSession on_drop ...", self.id);
        if let Err(e) = self._subscriptions_clear().await {
            log::error!("{:?} subscriptions clear error, {:?}", self.id, e);
        }
        if let Err(e) = self.delete_from_db().await {
            log::error!("{:?} subscriptions clear error, {:?}", self.id, e);
        }
        Ok(())
    }

    #[inline]
    async fn keepalive(&self, ping: IsPing) {
        log::debug!("ping: {}", ping);
        if ping {
            if self.is_inactive() {
                if let Err(e) = self.remove_and_save_to_db().await {
                    log::error!("{:?} remove and save session info to db error, {:?}", self.id, e);
                }
            }
            self.update_last_time(true).await;
        } else {
            self.update_last_time(false).await;
        }
    }
}

const SESSION_PRESENT: u8 = 0b00000001;
const SUPERUSER: u8 = 0b00000010;
const CONNECTED: u8 = 0b00000100;

const REMOVE_AND_SAVE: u8 = 0b00001000;

//const EMPTY: u8 = u8::MIN;
//const ALL: u8 = u8::MAX;

pub trait AtomicFlags {
    type T;
    fn empty() -> Self;
    fn all() -> Self;
    fn get(&self) -> Self::T;
    fn insert(&self, other: Self::T);
    fn contains(&self, other: Self::T) -> bool;
    fn remove(&self, other: Self::T);
    fn equal_exchange(&self, current: Self::T, new: Self::T, mask: Self::T) -> Result<Self::T, Self::T>;
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
    fn equal_exchange(&self, current: Self::T, new: Self::T, mask: Self::T) -> Result<Self::T, Self::T> {
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
    #[must_use]
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
    pub inflight_messages: Vec<InflightMessage>,
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
