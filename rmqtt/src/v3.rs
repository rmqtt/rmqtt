//! MQTT v3.1.1 Protocol Connection Handler
//!
//! Implements core broker-side connection workflow for MQTT v3.1.1 clients with:
//! 1. Connection handshake and client authentication
//! 2. Session state management with clean/dirty session persistence
//! 3. Resource control and QoS enforcement
//! 4. Asynchronous I/O using Tokio runtime
//!
//! ## Protocol Workflow
//! - Handles CONNECT/CONNACK sequence with timeout control
//! - Validates client IDs and enforces length limits
//! - Manages session takeover through kick-off mechanism
//! - Implements keep-alive negotiation and message queue limits
//!
//! ## Key Components
//! - `process()`: Main entry point handling TCP stream lifecycle
//! - `handshake()`: Performs protocol handshake and authentication
//! - SessionState: Tracks client state with automatic reclamation
//! - Hook system: Extensible authentication/authorization through `hook_mgr()`
//!
//! ## Features
//! - Async/await architecture for high concurrency
//! - Configurable limits:
//!   - Max client ID length
//!   - Concurrent sessions per node
//!   - Inflight window size
//!   - Message queue depth
//! - Automatic session transfer on reconnect
//! - Integrated metrics collection
//!
//! [MQTT v3.1.1 Spec Compliance](https://docs.oasis-open.org/mqtt/mqtt/v3.1.1/os/mqtt-v3.1.1-os.html)

use std::sync::Arc;

use anyhow::anyhow;
use rust_box::task_exec_queue::SpawnExt;
use scopeguard::defer;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::codec::v3::{Connect as ConnectV3, ConnectAckReason as ConnectAckReasonV3};
use crate::context::ServerContext;
use crate::net::v3;
use crate::net::MqttError;
use crate::session::{Session, SessionState};
use crate::types::{
    ClientId, ConnectAckReason, ConnectInfo, Id, ListenerConfig, ListenerId, Message, OfflineSession,
    SessionSubs, Sink,
};
use crate::utils::timestamp_millis;
use crate::{Error, Result};

pub(crate) async fn process<Io>(
    scx: ServerContext,
    mut sink: v3::MqttStream<Io>,
    lid: ListenerId,
) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    let (state, keep_alive) = {
        scx.handshakings.inc();
        defer! {
            scx.handshakings.dec();
        }

        let (state, keep_alive) = match handshake(&scx, &mut sink, lid).await {
            Ok(c) => c,
            Err((ack_code, e)) => {
                refused_ack_v3(&scx, &mut sink, None, ack_code, e.to_string()).await?;
                return Err(e);
            }
        };
        (state, keep_alive)
    };

    state.run(Sink::V3(sink), keep_alive).await;

    Ok(())
}

#[inline]
async fn handshake<Io>(
    scx: &ServerContext,
    sink: &mut v3::MqttStream<Io>,
    lid: ListenerId,
) -> std::result::Result<(SessionState, u16), (ConnectAckReason, Error)>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    let mut c = sink
        .recv_connect(sink.cfg.handshake_timeout)
        .await
        .map_err(|e| (ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e))?;

    log::debug!(
        "new Connection: local_addr: {:?}, remote_addr: {:?}, listen_cfg: {:?}",
        sink.cfg.laddr,
        sink.remote_addr,
        sink.cfg,
    );

    if c.client_id.is_empty() {
        if c.clean_session {
            c.client_id =
                ClientId::from(Uuid::new_v4().as_simple().encode_lower(&mut Uuid::encode_buffer()).to_owned())
        } else {
            log::info!(
                "{:?} Connection Refused, handshake error, reason: invalid client id",
                Id::new(
                    scx.node.id(),
                    lid,
                    Some(sink.cfg.laddr),
                    Some(sink.remote_addr),
                    ClientId::default(),
                    c.username.clone(),
                )
            );
            return Err((
                ConnectAckReason::V3(ConnectAckReasonV3::IdentifierRejected),
                MqttError::IdentifierRejected.into(),
            ));
        }
    }

    let id = Id::new(
        scx.node.id(),
        lid,
        Some(sink.cfg.laddr),
        Some(sink.remote_addr),
        c.client_id.clone(),
        c.username.clone(),
    );

    let now = std::time::Instant::now();
    let exec = scx.handshake_exec.get(sink.cfg.laddr.port(), &sink.cfg);
    match _handshake(scx.clone(), id.clone(), c, sink.cfg.clone(), now).spawn(&exec).result().await {
        Ok(Ok((state, keep_alive))) => {
            let session_present = state
                .session_present()
                .await
                .map_err(|e| (ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e))?;
            sink.send_connect_ack(ConnectAckReasonV3::ConnectionAccepted, session_present)
                .await
                .map_err(|e| (ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e))?;
            Ok((state, keep_alive))
        }
        Ok(Err(e)) => {
            log::info!("{id:?} Connection Refused, handshake error, reason: {e:?}");
            Err(e)
        }
        Err(e) => {
            #[cfg(feature = "metrics")]
            scx.metrics.client_handshaking_timeout_inc();
            let err = anyhow!("Connection Refused, execute handshake timeout");
            log::info!("{:?} {:?}, reason: {:?}", id, err, e.to_string(),);
            Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), err))
        }
    }
}

#[inline]
async fn _handshake(
    scx: ServerContext,
    id: Id,
    connect: Box<ConnectV3>,
    listen_cfg: ListenerConfig,
    hdshk_start: std::time::Instant,
) -> std::result::Result<(SessionState, u16), (ConnectAckReason, Error)> {
    let connect_info = ConnectInfo::V3(id.clone(), connect);

    //hook, client connect
    let _ = scx.extends.hook_mgr().client_connect(&connect_info).await;

    if hdshk_start.elapsed() > listen_cfg.handshake_timeout {
        return Err((
            ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable),
            anyhow!("handshake timeout"),
        ));
    }

    //check clientid len
    if listen_cfg.max_clientid_len > 0 && id.client_id.len() > listen_cfg.max_clientid_len {
        return Err((
            ConnectAckReason::V3(ConnectAckReasonV3::IdentifierRejected),
            anyhow!("client_id is too long"),
        ));
    }

    let entry = scx.extends.shared().await.entry(id.clone());
    let max_sessions = scx.mqtt_max_sessions;
    if max_sessions > 0 && scx.sessions.count() >= max_sessions && !entry.exist() {
        return Err((
            ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable),
            anyhow!(format!("the number of sessions on the current node exceeds the limit, with a maximum of {} sessions allowed", max_sessions)),
        ));
    }

    //hook, client authenticate
    let (ack, superuser, auth_info) =
        scx.extends.hook_mgr().client_authenticate(&connect_info, listen_cfg.allow_anonymous).await;
    if !ack.success() {
        return Err((ack, anyhow!("Authentication failed")));
    }

    let mut entry = match { scx.extends.shared().await.entry(id.clone()) }.try_lock().await {
        Err(e) => {
            return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e));
        }
        Ok(entry) => entry,
    };

    // Kick out the current session, if it exists
    let clean_session = connect_info.clean_start();
    let (session_present, _has_offline_session, offline_info) =
        match entry.kick(clean_session, clean_session, false).await {
            Err(e) => {
                return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e));
            }
            Ok(OfflineSession::NotExist) => (false, false, None),
            Ok(OfflineSession::Exist(Some(offline_info))) => (!clean_session, true, Some(offline_info)),
            Ok(OfflineSession::Exist(None)) => (false, true, None),
        };

    let connected_at = timestamp_millis();

    let connect_info = Arc::new(connect_info);

    log::debug!("{id:?} offline_info: {offline_info:?}");
    let created_at =
        if let Some(ref offline_info) = offline_info { offline_info.created_at } else { connected_at };

    let fitter = scx.extends.fitter_mgr().await.create(connect_info.clone(), id.clone(), listen_cfg.clone());

    let max_inflight = fitter.max_inflight();
    let max_mqueue_len = fitter.max_mqueue_len();

    let session = match Session::new(
        id,
        scx,
        max_mqueue_len,
        listen_cfg,
        fitter,
        auth_info,
        max_inflight,
        created_at,
        connect_info.clone(),
        session_present,
        superuser,
        true,
        connected_at,
        SessionSubs::new(),
        None,
        offline_info.as_ref().map(|o| o.id.clone()),
    )
    .await
    {
        Ok(s) => s,
        Err(e) => {
            return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e));
        }
    };

    let mut keep_alive = connect_info.keep_alive();
    let keep_alive = match session.fitter.keep_alive(&mut keep_alive) {
        Ok(keep_alive) => keep_alive,
        Err(e) => {
            return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e));
        }
    };

    let hook = session.scx.extends.hook_mgr().hook(session.clone());

    if offline_info.is_none() {
        //hook, session created
        hook.session_created().await;
    }

    let state = SessionState::new(session, hook, 0, 0);

    if let Err(e) = entry.set(state.session().clone(), state.tx().clone()).await {
        return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e));
    }

    //hook, client connack
    let _ = state
        .scx
        .extends
        .hook_mgr()
        .client_connack(connect_info.as_ref(), ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted))
        .await;

    //hook, client connected
    state.hook.client_connected().await;

    //transfer session state
    if let Some(o) = offline_info {
        if let Err(e) = state.tx().unbounded_send(Message::SessionStateTransfer(o, clean_session)) {
            return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e.into()));
        }
    }

    //automatic subscription
    #[cfg(feature = "auto-subscription")]
    {
        let auto_subscription = state.scx.extends.auto_subscription().await;
        if auto_subscription.enable() {
            match auto_subscription.subscribes(state.id()).await {
                Err(e) => return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e)),
                Ok(subs) => {
                    if let Err(e) = state.tx().unbounded_send(Message::Subscribes(subs, None)) {
                        return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e.into()));
                    }
                }
            }
        }
    }
    Ok((state, keep_alive))
}

async fn refused_ack_v3<Io>(
    scx: &ServerContext,
    sink: &mut v3::MqttStream<Io>,
    connect_info: Option<&ConnectInfo>,
    ack_code: ConnectAckReason,
    reason: String,
) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    let new_ack_code = if let Some(connect_info) = connect_info {
        scx.extends.hook_mgr().client_connack(connect_info, ack_code).await
    } else {
        ack_code
    };
    log::info!(
        "{:?} Connection Refused, handshake, ack_code: {:?}, new_ack_code: {:?}, reason: {}",
        connect_info.map(|c| c.id()),
        ack_code,
        new_ack_code,
        reason
    );
    let new_ack_code = if let ConnectAckReason::V3(ack_code) = new_ack_code {
        ack_code
    } else {
        ConnectAckReasonV3::ServiceUnavailable
    };
    sink.send_connect_ack(new_ack_code, false).await
}
