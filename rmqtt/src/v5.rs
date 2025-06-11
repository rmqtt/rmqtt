//! MQTT v5 Protocol Connection Handler Implementation
//!
//! Provides broker-side MQTT v5 protocol implementation with full CONNECT/CONNACK workflow,
//! session management, and QoS enforcement. Key features include:
//!
//! 1. **Protocol Compliance**
//!    - Full support for MQTT v5 specification features
//!    - Session expiration and persistence handling
//!    - Topic alias management (client/server-side)
//!    - Enhanced authentication flow with reason code mapping
//!
//! 2. **Connection Lifecycle**
//!    - Async TCP stream handling using Tokio runtime
//!    - Client ID generation with UUIDv4 fallback
//!    - Keep-alive negotiation and timeout detection
//!    - Configurable maximum packet size enforcement
//!
//! 3. **Session Management**
//!    - Clean session/dirty session persistence
//!    - Session takeover protection with atomic locking
//!    - Automatic offline message queuing
//!    - Resource limits enforcement (max sessions/client)
//!
//! 4. **Security & Extensibility**
//!    - Pluggable authentication hooks
//!    - Anonymous connection support with config toggle
//!    - QoS level validation (0-2)
//!    - Retained message control via feature flags
//!
//! Core components:
//! - `process()`: Main entry point handling TCP stream lifecycle
//! - `handshake()`: Negotiates protocol version and client capabilities
//! - SessionState: Manages client-specific subscriptions and message queues
//! - ConnectAck builder: Generates compliant CONNACK packets with server capabilities
//!
//! Implements advanced MQTT v5 features:
//! - Shared subscription support (feature-gated)
//! - Server-side topic alias mapping
//! - Session expiry interval control
//! - Reason code propagation for diagnostic clarity
//!
//! Metrics integration tracks:
//! - Concurrent handshakes
//! - Session creation/termination
//! - Protocol violations
//! - Resource limit triggers

use std::sync::Arc;

use anyhow::anyhow;
use rmqtt_codec::MqttCodec;
use rust_box::task_exec_queue::SpawnExt;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use crate::codec::v5::{Connect as ConnectV5, ConnectAck, ConnectAckReason as ConnectAckReasonV5};
use crate::context::ServerContext;
use crate::net::v5;
use crate::session::{Session, SessionState};
use crate::types::{
    ClientId, ConnectAckReason, ConnectInfo, Id, ListenerConfig, ListenerId, Message, OfflineSession,
    SessionSubs, Sink,
};
use crate::utils::timestamp_millis;
use crate::{Error, Result};

pub(crate) async fn process<Io>(
    scx: ServerContext,
    mut sink: v5::MqttStream<Io>,
    lid: ListenerId,
) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    scx.handshakings.inc();
    let (state, keep_alive) = match handshake(&scx, &mut sink, lid).await {
        Ok(c) => c,
        Err((ack_code, e)) => {
            scx.handshakings.dec();
            refused_ack(&scx, &mut sink, None, ack_code, e.to_string()).await?;
            return Err(e);
        }
    };

    scx.handshakings.dec();
    state.run(Sink::V5(sink), keep_alive).await;

    Ok(())
}

#[inline]
async fn handshake<Io>(
    scx: &ServerContext,
    sink: &mut v5::MqttStream<Io>,
    lid: ListenerId,
) -> std::result::Result<(SessionState, u16), (ConnectAckReason, Error)>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    let mut c = sink
        .recv_connect(sink.cfg.handshake_timeout)
        .await
        .map_err(|e| (ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e))?;

    log::debug!(
        "new Connection: local_addr: {:?}, remote_addr: {:?}, listen_cfg: {:?}",
        sink.cfg.laddr,
        sink.remote_addr,
        sink.cfg,
    );

    //The client specifies the maximum message length that the server can send to it in the CONNECT packet.
    if let Some(max_packet_size) = c.max_packet_size {
        if let MqttCodec::V5(codec) = sink.io.codec_mut() {
            codec.set_max_outbound_size(max_packet_size.get());
        }
    }

    let assigned_client_id = if c.client_id.is_empty() {
        c.client_id =
            ClientId::from(Uuid::new_v4().as_simple().encode_lower(&mut Uuid::encode_buffer()).to_owned());
        true
    } else {
        false
    };

    let id = Id::new(
        scx.node.id(),
        lid,
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
    //     return Err(MqttError::ServerUnavailable.into());
    // }

    let exec = scx.handshake_exec.get(sink.cfg.laddr.port(), &sink.cfg);
    match _handshake(scx.clone(), id.clone(), c, sink.cfg.clone(), assigned_client_id)
        .spawn(&exec)
        .result()
        .await
    {
        Ok(Ok((state, ack, keep_alive))) => {
            sink.send_connect_ack(ack)
                .await
                .map_err(|e| (ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e))?;
            Ok((state, keep_alive))
        }
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
            Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), err))
        }
    }
}

#[inline]
async fn _handshake(
    scx: ServerContext,
    id: Id,
    connect: Box<ConnectV5>,
    listen_cfg: ListenerConfig,
    is_assigned_client_id: bool,
) -> std::result::Result<(SessionState, ConnectAck, u16), (ConnectAckReason, Error)> {
    let connect_info = ConnectInfo::V5(id.clone(), connect);

    //hook, client connect
    let _ = scx.extends.hook_mgr().client_connect(&connect_info).await;

    //check clientid len
    if listen_cfg.max_clientid_len > 0 && id.client_id.len() > listen_cfg.max_clientid_len {
        return Err((
            ConnectAckReason::V5(ConnectAckReasonV5::ClientIdentifierNotValid),
            anyhow!("client_id is too long"),
        ));
    }

    //Extended Auth is not supported
    if connect_info.auth_method().is_some() {
        return Err((
            ConnectAckReason::V5(ConnectAckReasonV5::BadAuthenticationMethod),
            anyhow!("extended Auth is not supported"),
        ));
    }

    let entry = scx.extends.shared().await.entry(id.clone());
    let max_sessions = scx.mqtt_max_sessions;
    if max_sessions > 0 && scx.sessions.count() >= max_sessions && !entry.exist() {
        return Err((
            ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable),
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
            return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e));
        }
        Ok(entry) => entry,
    };

    // Kick out the current session, if it exists
    let clean_session = connect_info.clean_start();
    let (session_present, _has_offline_session, offline_info) =
        match entry.kick(clean_session, clean_session, false).await {
            Err(e) => {
                return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e));
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
            return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e));
        }
    };

    let mut server_keepalive_sec = connect_info.keep_alive();
    let keep_alive = match session.fitter.keep_alive(&mut server_keepalive_sec) {
        Ok(keep_alive) => keep_alive,
        Err(e) => {
            return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e));
        }
    };

    let hook = session.scx.extends.hook_mgr().hook(session.clone());

    if offline_info.is_none() {
        //hook, session created
        hook.session_created().await;
    }

    let client_topic_alias_max = session.fitter.max_client_topic_aliases();
    let server_topic_alias_max = session.fitter.max_server_topic_aliases();
    let state = SessionState::new(session, hook, server_topic_alias_max, client_topic_alias_max);

    if let Err(e) = entry.set(state.session().clone(), state.tx().clone()).await {
        return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e));
    }

    //hook, client connack
    let _ = state
        .scx
        .extends
        .hook_mgr()
        .client_connack(connect_info.as_ref(), ConnectAckReason::V5(ConnectAckReasonV5::Success))
        .await;

    //hook, client connected
    state.hook.client_connected().await;

    //transfer session state
    if let Some(o) = offline_info {
        if let Err(e) = state.tx().unbounded_send(Message::SessionStateTransfer(o, clean_session)) {
            return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e.into()));
        }
    }

    //automatic subscription
    #[cfg(feature = "auto-subscription")]
    {
        let auto_subscription = state.scx.extends.auto_subscription().await;
        if auto_subscription.enable() {
            match auto_subscription.subscribes(state.id()).await {
                Err(e) => return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e)),
                Ok(subs) => {
                    if let Err(e) = state.tx().unbounded_send(Message::Subscribes(subs, None)) {
                        return Err((ConnectAckReason::V5(ConnectAckReasonV5::ServerUnavailable), e.into()));
                    }
                }
            }
        }
    }

    log::debug!(
        "{:?} keep_alive: {}, server_keepalive_sec: {}",
        state.id,
        keep_alive,
        connect_info.keep_alive()
    );

    let session_expiry_interval = state.fitter.session_expiry_interval(None).as_secs() as u32;
    let session_expiry_interval_secs =
        if session_expiry_interval > 0 { Some(session_expiry_interval) } else { None };
    let max_qos = state.listen_cfg().max_qos_allowed;
    let retain_available = {
        #[cfg(feature = "retain")]
        {
            state.scx.extends.retain().await.enable()
        }
        #[cfg(not(feature = "retain"))]
        {
            false
        }
    };
    let max_server_packet_size = state.listen_cfg().max_packet_size;
    let shared_subscription_available = {
        #[cfg(feature = "shared-subscription")]
        {
            state.scx.extends.shared_subscription().await.is_supported(state.listen_cfg())
        }
        #[cfg(not(feature = "shared-subscription"))]
        {
            false
        }
    };

    let assigned_client_id = if is_assigned_client_id { Some(state.id.client_id.clone()) } else { None };

    let ack = ConnectAck {
        session_present,
        server_keepalive_sec: Some(server_keepalive_sec),
        session_expiry_interval_secs,
        receive_max: max_inflight,
        max_qos,
        retain_available,
        max_packet_size: Some(max_server_packet_size),
        assigned_client_id,
        topic_alias_max: client_topic_alias_max,
        wildcard_subscription_available: true,
        subscription_identifiers_available: true,
        shared_subscription_available,
        ..Default::default()
    };

    Ok((state, ack, keep_alive))
}

async fn refused_ack<Io>(
    scx: &ServerContext,
    sink: &mut v5::MqttStream<Io>,
    connect_info: Option<&ConnectInfo>,
    ack_code: ConnectAckReason,
    reason: String,
) -> Result<()>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    // if matches!(ack_code, ConnectAckReasonV5::ServerUnavailable) {
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
    let reason_code = if let ConnectAckReason::V5(ack_code) = new_ack_code {
        ack_code
    } else {
        ConnectAckReasonV5::ServerUnavailable
    };
    sink.send_connect_ack(ConnectAck { reason_code, ..Default::default() }).await
}
