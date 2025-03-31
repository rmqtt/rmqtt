use anyhow::anyhow;
use std::sync::Arc;

use rust_box::task_exec_queue::SpawnExt;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::codec::v3::{Connect as ConnectV3, ConnectAckReason as ConnectAckReasonV3};
use crate::context::ServerContext;
use crate::net::v3;
use crate::net::MqttError;
use crate::session::{Session, SessionState};
use crate::utils::timestamp_millis;
use crate::{
    ClientId, ConnectAckReason, ConnectInfo, Id, ListenerConfig, Message, OfflineSession, SessionSubs, Sink,
};
use crate::{Error, Result};

pub(crate) async fn process<Io>(scx: ServerContext, mut sink: v3::MqttStream<Io>) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    let state = match handshake(&scx, &mut sink).await {
        Ok(c) => c,
        Err((ack_code, e)) => {
            refused_ack_v3(&scx, &mut sink, None, ack_code, e.to_string()).await?;
            return Err(e);
        }
    };

    sink.send_connect_ack(ConnectAckReasonV3::ConnectionAccepted, state.session_present().await?).await?;

    let connect_info = state.connect_info().await?;
    let mut keep_alive = state.connect_info().await?.keep_alive();
    let keep_alive = match state.session().fitter.keep_alive(&mut keep_alive) {
        Ok(keep_alive) => keep_alive,
        Err(e) => {
            refused_ack_v3(
                &state.scx,
                &mut sink,
                Some(connect_info.as_ref()),
                ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable),
                e.to_string(),
            )
            .await?;
            return Err(e);
        }
    };

    state.run(Sink::V3(sink), keep_alive).await?;

    Ok(())
}

#[inline]
async fn handshake<Io>(
    scx: &ServerContext,
    sink: &mut v3::MqttStream<Io>,
) -> std::result::Result<SessionState, (ConnectAckReason, Error)>
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
        Some(sink.cfg.laddr),
        Some(sink.remote_addr),
        c.client_id.clone(),
        c.username.clone(),
    );

    // //RReject the service if the connection handshake too many unavailable.
    // if is_too_many_unavailable().await {
    //     log::warn!(
    //         "{:?} Connection Refused, handshake fail, reason: too busy, fails rate: {}",
    //         id,
    //         unavailable_stats().rate()
    //     );
    //     return Err(MqttError::ServiceUnavailable.into());
    // }

    // Runtime::instance().stats.handshakings.max_max(handshake.handshakings());

    let exec = scx.handshake_exec.get(sink.cfg.laddr.port(), &sink.cfg);
    match _handshake(scx.clone(), id.clone(), c, sink.cfg.clone()).spawn(&exec).result().await {
        Ok(Ok(state)) => Ok(state),
        Ok(Err(e)) => {
            // unavailable_stats().inc();
            // log::warn!(
            //     "{:?} Connection Refused, handshake error, reason: {:?}, fails rate: {}",
            //     id,
            //     e.to_string(),
            //     unavailable_stats().rate()
            // );
            log::warn!("{:?} Connection Refused, handshake error, reason: {:?}", id, e);
            Err(e)
        }
        Err(e) => {
            #[cfg(feature = "metrics")]
            scx.metrics.client_handshaking_timeout_inc();
            // unavailable_stats().inc();
            let err = anyhow!("Connection Refused, execute handshake timeout");
            log::warn!("{:?} {:?}, reason: {:?}", id, err, e.to_string(),);
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
) -> std::result::Result<SessionState, (ConnectAckReason, Error)> {
    let connect_info = ConnectInfo::V3(id.clone(), connect);

    //hook, client connect
    let _ = scx.extends.hook_mgr().client_connect(&connect_info).await;

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

    log::debug!("{:?} offline_info: {:?}", id, offline_info);
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
            if let Err(e) = auto_subscription.subscribe(state.id(), state.tx()).await {
                return Err((ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable), e));
            }
        }
    }
    Ok(state)
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
    // if matches!(ack_code, ConnectAckReasonV3::ServiceUnavailable) {
    //     // unavailable_stats().inc();
    // }
    let new_ack_code = if let Some(connect_info) = connect_info {
        scx.extends.hook_mgr().client_connack(connect_info, ack_code).await
    } else {
        ack_code
    };
    log::warn!(
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
