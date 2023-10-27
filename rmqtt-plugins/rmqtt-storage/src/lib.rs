#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use std::sync::Arc;
use std::time::Duration;

use rmqtt::{
    async_trait::async_trait,
    chrono, futures,
    futures::channel::mpsc,
    futures::channel::oneshot,
    futures::{SinkExt, StreamExt},
    log,
    serde_json::{self, json},
    tokio,
    tokio::sync::RwLock,
    TimestampMillis,
};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::DisconnectInfo,
    plugin::{DynPlugin, DynPluginResult, Plugin},
    ClientId, Result, Runtime, Session, SessionState, SessionSubMap, SessionSubs,
};

use config::PluginConfig;
use rmqtt::anyhow::anyhow;
use rmqtt::broker::fitter::Fitter;
use rmqtt::tokio_cron_scheduler::Job;
use session::{Basic, StorageSessionManager, StoredSessionInfo, StoredSessionInfos};
use store::{init_store_db, storage::Storage as _, Storage, StorageDb};

mod config;
mod session;
mod store;

enum RebuildChanType {
    Session(Session, Duration),
    Done(oneshot::Sender<()>),
}

#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    descr: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(async move {
                StoragePlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct StoragePlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    cfg: Arc<RwLock<PluginConfig>>,
    _storage_db: StorageDb,
    session_basic_kv: Storage,
    session_subs_kv: Storage,
    disconnect_info_kv: Storage,
    session_lasttime_kv: Storage,
    _message_kv: Storage,
    stored_session_infos: StoredSessionInfos,
    register: Box<dyn Register>,
    session_mgr: &'static StorageSessionManager,
    rebuild_tx: mpsc::Sender<RebuildChanType>,
}

impl StoragePlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config_default::<PluginConfig>(&name)?;
        log::info!("{} StoragePlugin cfg: {:?}", name, cfg);

        let storage_db = init_store_db(&cfg)?;
        let session_basic_kv = storage_db.open("session_basics")?;
        log::info!("{} StoragePlugin open session basic storage ok", name);
        let session_subs_kv = storage_db.open("session_subscriptions")?;
        log::info!("{} StoragePlugin open session subscriptions storage ok", name);
        let disconnect_info_kv = storage_db.open("disconnect_info")?;
        log::info!("{} StoragePlugin open session disconnect info storage ok", name);
        let session_lasttime_kv = storage_db.open("session_lasttime")?;
        log::info!("{} StoragePlugin open session last time storage ok", name);

        let message_storage = storage_db.open("message")?;
        log::info!("{} StoragePlugin open message storage ok", name);

        let stored_session_infos = StoredSessionInfos::new();

        let register = runtime.extends.hook_mgr().await.register();
        let session_mgr = StorageSessionManager::get_or_init(
            session_basic_kv.clone(),
            session_subs_kv.clone(),
            disconnect_info_kv.clone(),
            session_lasttime_kv.clone(),
            stored_session_infos.clone(),
        );
        let cfg = Arc::new(RwLock::new(cfg));
        let rebuild_tx = Self::start_local_runtime();
        Ok(Self {
            runtime,
            name,
            descr: descr.into(),
            cfg,
            _storage_db: storage_db,
            session_basic_kv,
            session_subs_kv,
            disconnect_info_kv,
            session_lasttime_kv,
            _message_kv: message_storage,
            stored_session_infos,
            register,
            session_mgr,
            rebuild_tx,
        })
    }

    async fn load_offline_session_infos(&mut self) {
        //Load offline session information from the database
        for item in self.session_basic_kv.iter::<Basic>() {
            match item {
                Ok((meta, basic)) => {
                    let stored_info = match StoredSessionInfo::from(meta, basic) {
                        Err(e) => {
                            log::warn!("{:?}", e);
                            continue;
                        }
                        Ok(store_info) => store_info,
                    };
                    self.stored_session_infos.add(stored_info);
                }
                Err(e) => {
                    log::warn!("Failed to read session basic information from the database, {:?}", e);
                }
            }
        }

        for item in self.session_subs_kv.iter::<(ClientId, SessionSubMap)>() {
            match item {
                Ok((meta, (client_id, subs))) => {
                    if let Err(e) = self.stored_session_infos.add_subs(client_id, meta, subs) {
                        log::warn!("{:?}", e);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to read session subscription information from the database, {:?}", e);
                }
            }
        }

        for item in self.disconnect_info_kv.iter::<(ClientId, DisconnectInfo)>() {
            match item {
                Ok((meta, (client_id, disconnect_info))) => {
                    if let Err(e) =
                        self.stored_session_infos.add_disconnect_info(client_id, meta, disconnect_info)
                    {
                        log::warn!("{:?}", e);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to read session disconnect information from the database, {:?}", e);
                }
            }
        }

        for item in self.session_lasttime_kv.iter::<(ClientId, TimestampMillis)>() {
            match item {
                Ok((meta, (client_id, last_time))) => {
                    if let Err(e) = self.stored_session_infos.set_last_time(client_id, meta, last_time) {
                        log::warn!("{:?}", e);
                    }
                }
                Err(e) => {
                    log::warn!("Failed to read session last time from the database, {:?}", e);
                }
            }
        }

        self.stored_session_infos.retain_latests();
        log::info!("stored_session_infos len: {:?}", self.stored_session_infos.len());

        for item in self._message_kv.iter::<SessionSubMap>() {
            log::info!("_message_kv meta: item: {:?}", item);
        }
    }

    async fn cleanup_offline_session_infos(
        session_basic_kv: &Storage,
        session_subs_kv: &Storage,
        disconnect_info_kv: &Storage,
        session_lasttime_kv: &Storage,
    ) {
        //Clean offline session information
        session_basic_kv
            .retain::<_, _, Basic>(|item| async move {
                match item {
                    Err(e) => {
                        log::warn!(
                            "Failed to retain session basic information from the database, {:?}",
                            e
                        );
                        false
                    },
                    Ok((meta, basic)) => {
                        let id = basic.id.clone();
                        let key = meta.key.to_vec();
                        let disconnect_info = disconnect_info_kv
                            .clone()
                            .get::<_, (ClientId, DisconnectInfo)>(key.as_slice())
                            .ok()
                            .flatten()
                            .map(|(_, d)| d);
                        let lasttime = session_lasttime_kv
                            .get::<_, (ClientId, TimestampMillis)>(key.as_slice())
                            .ok()
                            .flatten()
                            .map(|(_, t)| t)
                            .unwrap_or_else(|| meta.time);
                        if Self::is_session_expiry(basic, disconnect_info, lasttime).await {
                            log::debug!("{:?} session expiry", id);
                            if let Err(e) = session_subs_kv.remove(key.as_slice()) {
                                log::warn!(
                                    "Failed to remove session subscription information from the database, {:?}",
                                    e
                                );
                            }
                            if let Err(e) = disconnect_info_kv.remove(key.as_slice()) {
                                log::warn!(
                                    "Failed to remove session disconnect information from the database, {:?}",
                                    e
                                );
                            }
                            if let Err(e) = session_lasttime_kv.remove(key.as_slice()) {
                                log::warn!(
                                    "Failed to remove session last time from the database, {:?}",
                                    e
                                );
                            }
                            false
                        }else{
                            true
                        }
                    }
                }
            })
            .await;

        //Clean up abnormal data
        async fn cleanup_abnormal_data(kv: &Storage, basic_kv: &Storage, tag: &str) {
            kv.retain_with_meta(|item| async move {
                match item {
                    Err(e) => {
                        log::warn!("Failed to cleanup abnormal data from the database({}), {:?}", tag, e);
                        false
                    }
                    Ok(meta) => {
                        if (chrono::Local::now().timestamp() - (meta.time / 1000)) > 24 * 60 * 60 {
                            if basic_kv.contains_key(meta.key.as_ref()).unwrap_or_default() {
                                true
                            } else {
                                log::info!(
                                    "{:?} cleanup abnormal data from the database({})",
                                    String::from_utf8_lossy(meta.key.as_ref()),
                                    tag
                                );
                                false
                            }
                        } else {
                            true
                        }
                    }
                }
            })
            .await;
        }

        cleanup_abnormal_data(session_subs_kv, session_basic_kv, "subscription").await;
        cleanup_abnormal_data(disconnect_info_kv, session_basic_kv, "disconnect info").await;
        cleanup_abnormal_data(session_lasttime_kv, session_basic_kv, "last time").await;

        let _ = session_subs_kv.flush().await;
        let _ = disconnect_info_kv.flush().await;
        let _ = session_lasttime_kv.flush().await;
        let _ = session_basic_kv.flush().await;
    }

    #[inline]
    async fn is_session_expiry(
        basic: Basic,
        disconnect_info: Option<DisconnectInfo>,
        last_time: TimestampMillis,
    ) -> bool {
        //get listener config
        let listen_cfg = if let Some(listen_cfg) =
            basic.id.local_addr.and_then(|addr| Runtime::instance().settings.listeners.get(addr.port()))
        {
            listen_cfg
        } else {
            log::warn!("tcp listener config is not found, local addr is {:?}", basic.id.local_addr);
            return true;
        };

        //create fitter
        let fitter =
            Runtime::instance().extends.fitter_mgr().await.create(basic.conn_info, basic.id, listen_cfg);

        //check session expiry interval
        let session_expiry_interval =
            session_expiry_interval(fitter.as_ref(), disconnect_info.as_ref(), last_time).await;
        session_expiry_interval <= 0
    }

    fn start_local_runtime() -> mpsc::Sender<RebuildChanType> {
        let (tx, mut rx) = futures::channel::mpsc::channel::<RebuildChanType>(100_000);
        std::thread::spawn(move || {
            let local_rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            let local_set = tokio::task::LocalSet::new();

            local_set.block_on(&local_rt, async {
                while let Some(msg) = rx.next().await {
                    match msg {
                        RebuildChanType::Session(session, session_expiry_interval)  => {

                                let (state, msg_tx) =
                                    SessionState::offline_restart(session.clone(), session_expiry_interval).await;
                                let mut session_entry =
                                    Runtime::instance().extends.shared().await.entry(state.id.clone());

                                let id = session_entry.id().clone();
                                let task_fut = async move {
                                    if let Err(e) = session_entry.set(session, msg_tx).await {
                                        log::warn!("{:?} Rebuild offline session error, {:?}", session_entry.id(), e);
                                    }
                                };

                                let task_exec = &Runtime::instance().exec;
                                if let Err(e) = task_exec.spawn(task_fut).await {
                                    log::warn!("{:?} Rebuild offline session error, {:?}", id, e.to_string());
                                }

                                let completed_count = task_exec.completed_count().await;
                                if completed_count > 0 && completed_count % 5000 == 0 {
                                    log::info!(
                                        "{:?} Rebuild offline session, completed_count: {}, active_count: {}, waiting_count: {}, rate: {:?}",
                                        id,
                                        task_exec.completed_count().await, task_exec.active_count(), task_exec.waiting_count(), task_exec.rate().await
                                    );
                                }
                        },
                        RebuildChanType::Done(done_tx) => {
                            let task_exec = &Runtime::instance().exec;
                            let _ = task_exec.flush().await;
                            let _ = done_tx.send(());
                            log::info!(
                                "Rebuild offline session, completed_count: {}, active_count: {}, waiting_count: {}, rate: {:?}",
                                task_exec.completed_count().await, task_exec.active_count(), task_exec.waiting_count(), task_exec.rate().await
                            );
                        }
                    }
                }
            });
            log::info!("Rebuild offline session ends");
        });
        tx
    }
}

#[async_trait]
impl Plugin for StoragePlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        self.register
            .add(
                Type::BeforeStartup,
                Box::new(StorageHandler::new(
                    self.cfg.clone(),
                    self.stored_session_infos.clone(),
                    self.rebuild_tx.clone(),
                )),
            )
            .await;
        self.load_offline_session_infos().await;

        //Periodic Cleanup Tasks
        let session_basic_kv = self.session_basic_kv.clone();
        let session_subs_kv = self.session_subs_kv.clone();
        let disconnect_info_kv = self.disconnect_info_kv.clone();
        let session_lasttime_kv = self.session_lasttime_kv.clone();
        let async_cleans = Job::new_async(self.cfg.read().await.cleanup_cron.as_str(), move |_uuid, _l| {
            log::info!("Cleanup tasks start ...");
            let session_basic_kv = session_basic_kv.clone();
            let session_subs_kv = session_subs_kv.clone();
            let disconnect_info_kv = disconnect_info_kv.clone();
            let session_lasttime_kv = session_lasttime_kv.clone();
            Box::pin(async move {
                Self::cleanup_offline_session_infos(
                    &session_basic_kv,
                    &session_subs_kv,
                    &disconnect_info_kv,
                    &session_lasttime_kv,
                )
                .await;
            })
        })
        .map_err(|e| anyhow!(e))?;
        self.runtime.sched.add(async_cleans).await.map_err(|e| anyhow!(e))?;

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        *self.runtime.extends.session_mgr_mut().await = Box::new(self.session_mgr);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, if the storage plugin is started, it cannot be stopped", self.name);
        Ok(false)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.0"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        log::info!("lasttime_kv iter start ...");
        for item in self.session_lasttime_kv.iter::<(ClientId, TimestampMillis)>() {
            match item {
                Ok((meta, (client_id, last_time))) => {
                    log::info!(
                        "{} lasttime_kv, client_id: {}, last_time: {}, key: {:?}",
                        self.name,
                        client_id,
                        last_time,
                        String::from_utf8_lossy(meta.key.as_ref())
                    );
                }
                Err(e) => {
                    log::warn!("Failed to read session last time from the database, {:?}", e);
                }
            }
        }

        json!({
            "storage_session_basic_count": self.session_basic_kv.len(),
            "storage_session_subscriptions_count": self.session_subs_kv.len(),
            "storage_session_disconnect_info_count": self.disconnect_info_kv.len(),
            "storage_session_lasttime_count": self.session_lasttime_kv.len(),
            "storage_size_on_disk": self._storage_db.size_on_disk().unwrap_or_default(),
        })
    }
}

struct StorageHandler {
    cfg: Arc<RwLock<PluginConfig>>,
    stored_session_infos: StoredSessionInfos,
    rebuild_tx: mpsc::Sender<RebuildChanType>,
}

impl StorageHandler {
    fn new(
        cfg: Arc<RwLock<PluginConfig>>,
        stored_session_infos: StoredSessionInfos,
        rebuild_tx: mpsc::Sender<RebuildChanType>,
    ) -> Self {
        Self { cfg, stored_session_infos, rebuild_tx }
    }

    //Rebuild offline session.
    async fn rebuild_offline_sessions(&self, rebuild_done_tx: oneshot::Sender<()>) {
        let mut offline_sessions_count = 0;
        for mut entry in self.stored_session_infos.iter_mut() {
            let (_, storeds) = entry.pair_mut();
            if let Some(stored) = storeds.iter_mut().next() {
                let id = stored.basic.id.clone();

                //get listener config
                let listen_cfg = if let Some(listen_cfg) =
                    id.local_addr.and_then(|addr| Runtime::instance().settings.listeners.get(addr.port()))
                {
                    listen_cfg
                } else {
                    log::warn!("tcp listener config is not found, local addr is {:?}", id.local_addr);
                    continue;
                };

                //create fitter
                let fitter = Runtime::instance().extends.fitter_mgr().await.create(
                    stored.basic.conn_info.clone(),
                    id.clone(),
                    listen_cfg.clone(),
                );

                //check session expiry interval
                let session_expiry_interval = session_expiry_interval(
                    fitter.as_ref(),
                    stored.disconnect_info.as_ref(),
                    stored.last_time,
                )
                .await;
                log::debug!("{:?} session_expiry_interval: {:?}", id, session_expiry_interval);
                if session_expiry_interval <= 0 {
                    //session is expiry
                    continue;
                }
                offline_sessions_count += 1;

                if stored.disconnect_info.is_none() {
                    stored.disconnect_info = Some(DisconnectInfo::new(stored.last_time));
                }

                let max_inflight = fitter.max_inflight();
                let max_mqueue_len = fitter.max_mqueue_len();
                let subs = stored.subs.take().map(SessionSubs::from).unwrap_or_else(SessionSubs::new);

                let session = Session::new(
                    id.clone(),
                    max_mqueue_len,
                    listen_cfg,
                    fitter,
                    max_inflight,
                    stored.basic.created_at,
                    stored.basic.conn_info.clone(),
                    false,
                    false,
                    false,
                    stored.basic.connected_at,
                    subs,
                    stored.disconnect_info.take(),
                )
                .await;
                if let Err(e) = self
                    .rebuild_tx
                    .clone()
                    .send(RebuildChanType::Session(
                        session,
                        Duration::from_millis(session_expiry_interval as u64),
                    ))
                    .await
                {
                    log::error!("rebuild offline sessions error, {:?}", e);
                }
            }
        }
        log::info!("offline_sessions_count: {}", offline_sessions_count);
        let _ = self.rebuild_tx.clone().send(RebuildChanType::Done(rebuild_done_tx)).await;
    }
}

#[async_trait]
impl Handler for StorageHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::BeforeStartup => {
                log::info!(
                    "BeforeStartup storage_type: {:?}, stored_session_infos len: {}",
                    self.cfg.read().await.storage_type,
                    self.stored_session_infos.len()
                );
                let (rebuild_done_tx, rebuild_done_rx) = oneshot::channel::<()>();
                self.rebuild_offline_sessions(rebuild_done_tx).await;
                let _ = rebuild_done_rx.await;
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

#[inline]
async fn session_expiry_interval(
    fitter: &dyn Fitter,
    disconnect_info: Option<&DisconnectInfo>,
    last_time: TimestampMillis,
) -> TimestampMillis {
    let disconnected_at = disconnect_info.map(|d| d.disconnected_at).unwrap_or_default();
    let disconnected_at = if disconnected_at <= 0 { last_time } else { disconnected_at };
    fitter.session_expiry_interval(disconnect_info.and_then(|d| d.mqtt_disconnect.as_ref())).await.as_millis()
        as i64
        - (chrono::Local::now().timestamp_millis() - disconnected_at)
}
