use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicI64, AtomicU8, Ordering};
use std::sync::Arc;

use rmqtt::{
    async_trait::async_trait,
    bytestring::ByteString,
    chrono, log,
    once_cell::sync::OnceCell,
    tokio,
    tokio::sync::{RwLock, RwLockWriteGuard},
    DashMap,
};
use rmqtt::{
    broker::default::DefaultSessionManager,
    broker::session::{SessionLike, SessionManager},
    broker::types::DisconnectInfo,
    settings::Listener,
    ClientId, ConnectInfo, ConnectInfoType, Disconnect, Id, InflightType, IsPing, MessageQueueType,
    MqttError, Password, Reason, Result, SessionSubMap, SessionSubs, SubscriptionOptions, Subscriptions,
    TimestampMillis, TopicFilter, UserName,
};

use crate::store::storage::{Metadata, Storage as _};
use crate::store::Storage;

pub(crate) struct StorageSessionManager {
    basic_kv: Storage,
    subs_kv: Storage,
    disconnect_info_kv: Storage,
    session_lasttime_kv: Storage,
    _stored_session_infos: StoredSessionInfos,
}

impl StorageSessionManager {
    #[inline]
    pub(crate) fn get_or_init(
        basic_kv: Storage,
        subs_kv: Storage,
        disconnect_info_kv: Storage,
        session_lasttime_kv: Storage,
        _stored_session_infos: StoredSessionInfos,
    ) -> &'static StorageSessionManager {
        static INSTANCE: OnceCell<StorageSessionManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            basic_kv,
            subs_kv,
            disconnect_info_kv,
            session_lasttime_kv,
            _stored_session_infos,
        })
    }
}

impl SessionManager for &'static StorageSessionManager {
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        id: Id,
        listen_cfg: Listener,
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
    ) -> Arc<dyn SessionLike> {
        if conn_info.clean_start() {
            DefaultSessionManager::instance().create(
                id,
                listen_cfg,
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
            )
        } else {
            //Only when 'clean_session' is equal to false or 'clean_start' is equal to false, the
            // session information persistence feature will be initiated.
            let s = Arc::new(StorageSession::new(
                id,
                listen_cfg,
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
                self.basic_kv.clone(),
                self.subs_kv.clone(),
                self.disconnect_info_kv.clone(),
                self.session_lasttime_kv.clone(),
            ));
            if connected {
                let s1 = s.clone();
                tokio::spawn(async move {
                    if let Err(e) = s1.save_to_db().await {
                        log::error!("Save session info error to db, {:?}", e);
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
    deliver_queue: MessageQueueType,
    inflight_win: InflightType,

    basic: RwLock<Option<Basic>>,
    subscriptions: RwLock<Option<SessionSubs>>,

    state_flags: AtomicU8,
    password: Option<Password>,
    disconnect_info: RwLock<Option<DisconnectInfo>>,

    //----------------------------------
    basic_kv: Storage,
    subs_kv: Storage,
    disconnect_info_kv: Storage,
    lasttime_kv: Storage,
    last_time: AtomicI64,
}

impl StorageSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: Id,
        listen_cfg: Listener,
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

        basic_kv: Storage,
        subs_kv: Storage,
        disconnect_info_kv: Storage,
        lasttime_kv: Storage,
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
            deliver_queue,
            inflight_win,

            basic: RwLock::new(Some(basic)),
            subscriptions: RwLock::new(Some(subscriptions)),

            state_flags,
            password,
            disconnect_info: RwLock::new(Some(disconnect_info)),

            basic_kv,
            subs_kv,
            disconnect_info_kv,
            lasttime_kv,
            last_time: AtomicI64::new(chrono::Local::now().timestamp_millis()),
        }
    }

    #[inline]
    fn update_last_time(&self, save_enable: bool) {
        let time = chrono::Local::now().timestamp_millis();
        let old = self.last_time.swap(time, Ordering::SeqCst);
        if save_enable || (time - old) > (1000 * 60) {
            if let Err(e) = self.lasttime_kv.insert(self.id.to_string(), &(&self.id.client_id, time)) {
                log::warn!("save last time to db error, {:?}", e);
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
        let key = self.id.to_string();
        if let Err(e) = self.basic_kv.remove(&key) {
            log::error!("remove session basic info error from db, {:?}", e);
        }
        if let Err(e) = self.subs_kv.remove(&key) {
            log::error!("remove session subscriptions error from db, {:?}", e);
        }
        if let Err(e) = self.disconnect_info_kv.remove(&key) {
            log::error!("remove session disconnect info error from db, {:?}", e);
        }
        if let Err(e) = self.lasttime_kv.remove(&key) {
            log::error!("remove session last time error from db, {:?}", e);
        }

        Ok(())
    }

    #[inline]
    pub(crate) async fn remove_and_save_to_db(&self) -> Result<()> {
        /*
        if self.state_flags.contains(REMOVE_AND_SAVE) {
            return Ok(());
        }
        self.state_flags.insert(REMOVE_AND_SAVE);
        */
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
        self.update_last_time(true);
        self.save_basic_info().await?;
        self.save_subscriptions().await?;
        //self.save_disconnect_info().await?;
        log::debug!("save to db ...");
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
            log::debug!("{:?} save basic info to db", self.id.client_id);
            self.basic_kv.insert(self.id.to_string(), basic)?;
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
        log::debug!("{:?} read basic info from storage ..., {}", self.id.client_id, tag);
        if let Some(basic) = self.basic_kv.get::<_, Basic>(self.id.to_string())? {
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
            log::debug!("{:?} save subscriptions to db", self.id.client_id);
            let subs = subscriptions.read().await;
            self.subs_kv.insert(self.id.to_string(), &(&self.id.client_id, subs.deref()))?;
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
        log::debug!("{:?} read subscriptions info from storage ..., {}", self.id.client_id, tag);
        if let Some((_, subs)) = self.subs_kv.get::<_, (ClientId, SessionSubMap)>(self.id.to_string())? {
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
            log::debug!("{:?} save disconnect info to db", self.id.client_id);
            self.disconnect_info_kv.insert(self.id.to_string(), &(&self.id.client_id, disconnect_info))?;
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
        log::debug!("{:?} read disconnect info from storage ..., {}", self.id.client_id, tag);
        if let Some((_, disconnect_info)) =
            self.disconnect_info_kv.get::<_, (ClientId, DisconnectInfo)>(self.id.to_string())?
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
    fn not_found_error() -> MqttError {
        MqttError::from("not found from storage")
    }
}

#[async_trait]
impl SessionLike for StorageSession {
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
    async fn connected(&self) -> Result<bool> {
        Ok(self.state_flags.contains(CONNECTED))
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
            }

            if let Some(reason) = reason {
                disconnect_info.reasons.push(reason);
            }
            drop(disconnect_info_wl);
        }
        self.save_disconnect_info().await?;
        log::debug!("disconnected_set ... ");
        Ok(())
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
        log::debug!("disconnected_reason_add ... ");
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
        log::debug!("disconnected_reason_take ... ");
        Ok(r)
    }

    #[inline]
    async fn on_drop(&self) -> Result<()> {
        log::debug!("StorageSession on_drop ...");
        if let Err(e) = self._subscriptions_clear().await {
            log::error!("subscriptions clear error, {:?}", e);
        }
        if let Err(e) = self.delete_from_db().await {
            log::error!("subscriptions clear error, {:?}", e);
        }
        Ok(())
    }

    #[inline]
    async fn keepalive(&self, ping: IsPing) {
        log::debug!("ping: {}", ping);
        if ping {
            if self.is_inactive() {
                if let Err(e) = self.remove_and_save_to_db().await {
                    log::error!("remove and save session info to db error, {:?}", e);
                }
            }
            self.update_last_time(true);
        } else {
            self.update_last_time(false);
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

pub(crate) struct StoredSessionInfo {
    pub stored_key: ByteString,
    pub basic: Basic,
    pub subs: Option<SessionSubMap>,
    pub disconnect_info: Option<DisconnectInfo>,
    pub last_time: TimestampMillis,
}

impl StoredSessionInfo {
    #[inline]
    pub fn stored_key(meta: &Metadata) -> Result<ByteString> {
        ByteString::try_from(meta.key.as_ref())
            .map_err(|e| MqttError::from(format!("Metadata data format error, invalid key, {:?}", e)))
    }

    #[inline]
    pub fn from(meta: Metadata, basic: Basic) -> Result<Self> {
        let stored_key = Self::stored_key(&meta)?;
        Ok(Self { stored_key, basic, subs: None, disconnect_info: None, last_time: meta.time })
    }

    #[inline]
    #[allow(clippy::mutable_key_type)]
    pub fn set_subs(&mut self, meta: Metadata, subs: SessionSubMap) {
        self.subs.replace(subs);
        if self.last_time < meta.time {
            self.last_time = meta.time;
        }
    }

    #[inline]
    pub fn set_disconnect_info(&mut self, meta: Metadata, disconnect_info: DisconnectInfo) {
        self.disconnect_info.replace(disconnect_info);
        if self.last_time < meta.time {
            self.last_time = meta.time;
        }
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
    #[allow(clippy::mutable_key_type)]
    pub fn add_subs(&mut self, client_id: ClientId, meta: Metadata, subs: SessionSubMap) -> Result<()> {
        let stored_key = StoredSessionInfo::stored_key(&meta)?;
        if let Some(mut entry) = self.0.get_mut(&client_id) {
            let storeds = entry.value_mut();
            for stored in storeds {
                if stored.stored_key == stored_key {
                    stored.set_subs(meta, subs);
                    break;
                }
            }
        }
        Ok(())
    }

    #[inline]
    pub fn add_disconnect_info(
        &mut self,
        client_id: ClientId,
        meta: Metadata,
        disconnect_info: DisconnectInfo,
    ) -> Result<()> {
        let stored_key = StoredSessionInfo::stored_key(&meta)?;
        if let Some(mut entry) = self.0.get_mut(&client_id) {
            let storeds = entry.value_mut();
            for stored in storeds {
                if stored.stored_key == stored_key {
                    stored.set_disconnect_info(meta, disconnect_info);
                    break;
                }
            }
        }
        Ok(())
    }

    #[inline]
    pub fn set_last_time(
        &mut self,
        client_id: ClientId,
        meta: Metadata,
        last_time: TimestampMillis,
    ) -> Result<()> {
        let stored_key = StoredSessionInfo::stored_key(&meta)?;
        if let Some(mut entry) = self.0.get_mut(&client_id) {
            let storeds = entry.value_mut();
            for stored in storeds {
                if stored.stored_key == stored_key {
                    stored.set_last_time(last_time);
                    break;
                }
            }
        }
        Ok(())
    }

    #[inline]
    pub fn retain_latests(&mut self) {
        for mut entry in self.0.iter_mut() {
            let storeds = entry.value_mut();
            if storeds.len() > 1 {
                if let Some(mut latest) = storeds.pop() {
                    while let Some(stored) = storeds.pop() {
                        if stored.last_time > latest.last_time {
                            latest = stored;
                        }
                    }
                    storeds.push(latest);
                }
            }
        }
    }
}
