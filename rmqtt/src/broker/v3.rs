use bytestring::ByteString;
use std::convert::From as _f;
use std::net::SocketAddr;
use std::sync::Arc;

use rust_box::task_exec_queue::LocalSpawnExt;
use uuid::Uuid;

use crate::broker::executor::get_handshake_exec;
use crate::broker::{inflight::MomentStatus, types::*};
use crate::runtime::Runtime;
use crate::settings::listener::Listener;
use crate::{MqttError, Result, Session, SessionState};

#[inline]
async fn refused_ack<Io>(
    handshake: v3::Handshake<Io>,
    connect_info: &ConnectInfo,
    ack_code: ConnectAckReasonV3,
    reason: String,
) -> v3::HandshakeAck<Io, SessionState> {
    let new_ack_code = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .client_connack(connect_info, ConnectAckReason::V3(ack_code))
        .await;
    log::warn!(
        "{:?} Connection Refused, handshake, ack_code: {:?}, new_ack_code: {:?}, reason: {}",
        connect_info.id(),
        ack_code,
        new_ack_code,
        reason,
    );
    new_ack_code.v3_error_ack(handshake)
}

#[inline]
pub async fn handshake<Io: 'static>(
    listen_cfg: Listener,
    mut handshake: v3::Handshake<Io>,
    remote_addr: SocketAddr,
    local_addr: SocketAddr,
) -> Result<v3::HandshakeAck<Io, SessionState>, MqttError> {
    log::debug!(
        "new Connection: local_addr: {:?}, remote: {:?}, {:?}, listen_cfg: {:?}",
        local_addr,
        remote_addr,
        handshake,
        listen_cfg
    );

    if handshake.packet().client_id.is_empty() {
        if handshake.packet().clean_session {
            handshake.packet_mut().client_id =
                ClientId::from(Uuid::new_v4().as_simple().encode_lower(&mut Uuid::encode_buffer()).to_owned())
        } else {
            log::info!(
                "{:?} Connection Refused, handshake error, reason: invalid client id",
                Id::new(
                    Runtime::instance().node.id(),
                    Some(local_addr),
                    Some(remote_addr),
                    ClientId::default(),
                    handshake.packet().username.clone(),
                )
            );
            return Ok(ConnectAckReason::V3(ConnectAckReasonV3::IdentifierRejected).v3_error_ack(handshake));
        }
    }

    let id = Id::new(
        Runtime::instance().node.id(),
        Some(local_addr),
        Some(remote_addr),
        handshake.packet().client_id.clone(),
        handshake.packet().username.clone(),
    );

    Runtime::instance().stats.handshakings.max_max(handshake.handshakings());

    let exec = get_handshake_exec(local_addr.port(), listen_cfg.clone());
    match _handshake(id.clone(), listen_cfg, handshake).spawn(&exec).result().await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(e)) => {
            log::warn!("{:?} Connection Refused, handshake error, reason: {:?}", id, e.to_string());
            Err(e)
        }
        Err(e) => {
            Runtime::instance().metrics.client_handshaking_timeout_inc();
            let err = MqttError::from("Connection Refused, execute handshake timeout");
            log::warn!("{:?} {:?}, reason: {:?}", id, err, e.to_string());
            Err(err)
        }
    }
}

#[inline]
async fn _handshake<Io: 'static>(
    id: Id,
    listen_cfg: Listener,
    mut handshake: v3::Handshake<Io>,
) -> Result<v3::HandshakeAck<Io, SessionState>, MqttError> {
    let connect_info = Arc::new(ConnectInfo::V3(id.clone(), handshake.packet().clone()));

    //hook, client connect
    let _ = Runtime::instance().extends.hook_mgr().await.client_connect(&connect_info).await;

    if listen_cfg.max_clientid_len > 0 && id.client_id.len() > listen_cfg.max_clientid_len {
        return Ok(refused_ack(
            handshake,
            &connect_info,
            ConnectAckReasonV3::IdentifierRejected,
            "client_id is too long".into(),
        )
        .await);
    }

    //hook, client authenticate
    let (ack, superuser) = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .client_authenticate(&connect_info, listen_cfg.allow_anonymous)
        .await;
    if !ack.success() {
        if let ConnectAckReason::V3(ack) = ack {
            return Ok(refused_ack(handshake, &connect_info, ack, "Authentication failed".into()).await);
        } else {
            unreachable!()
        }
    }

    let sink = handshake.sink();
    let packet = handshake.packet_mut();

    let mut entry = match { Runtime::instance().extends.shared().await.entry(id.clone()) }.try_lock().await {
        Err(e) => {
            return Ok(refused_ack(
                handshake,
                &connect_info,
                ConnectAckReasonV3::ServiceUnavailable,
                format!("{}", e),
            )
            .await);
        }
        Ok(entry) => entry,
    };

    // Kick out the current session, if it exists
    let (session_present, offline_info) =
        match entry.kick(packet.clean_session, packet.clean_session, false).await {
            Err(e) => {
                return Ok(refused_ack(
                    handshake,
                    &connect_info,
                    ConnectAckReasonV3::ServiceUnavailable,
                    format!("{}", e),
                )
                .await);
            }
            Ok(Some(offline_info)) => (!packet.clean_session, Some(offline_info)),
            Ok(None) => (false, None),
        };

    let connected_at = chrono::Local::now().timestamp_millis();

    let fitter = Runtime::instance().extends.fitter_mgr().await.create(
        connect_info.clone(),
        id.clone(),
        listen_cfg.clone(),
    );

    log::debug!("{:?} offline_info: {:?}", id, offline_info);
    let created_at =
        if let Some(ref offline_info) = offline_info { offline_info.created_at } else { connected_at };

    let max_inflight = fitter.max_inflight();
    let max_mqueue_len = fitter.max_mqueue_len();
    let session = match Session::new(
        id,
        max_mqueue_len,
        listen_cfg,
        fitter,
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
            return Ok(refused_ack(
                handshake,
                connect_info.as_ref(),
                ConnectAckReasonV3::ServiceUnavailable,
                format!("{}", e),
            )
            .await);
        }
    };

    let keep_alive = match session.fitter.keep_alive(&mut packet.keep_alive) {
        Ok(keep_alive) => keep_alive,
        Err(e) => {
            return Ok(refused_ack(
                handshake,
                connect_info.as_ref(),
                ConnectAckReasonV3::ServiceUnavailable,
                format!("{:?}", e),
            )
            .await);
        }
    };

    let hook = Runtime::instance().extends.hook_mgr().await.hook(&session);

    if offline_info.is_none() {
        //hook, session created
        hook.session_created().await;
    }

    let (state, tx) = SessionState::new(session, Sink::V3(sink), hook, 0, 0).start(keep_alive).await?;
    if let Err(e) = entry.set(state.session.clone(), tx).await {
        return Ok(refused_ack(
            handshake,
            connect_info.as_ref(),
            ConnectAckReasonV3::ServiceUnavailable,
            format!("{}", e),
        )
        .await);
    }

    //hook, client connack
    let _ = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .client_connack(connect_info.as_ref(), ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted))
        .await;

    //hook, client connected
    state.hook.client_connected().await;

    //transfer session state
    if let Some(o) = offline_info {
        let state1 = state.clone();
        let clean_session = packet.clean_session;
        ntex::rt::spawn(async move {
            if let Err(e) = state1.transfer_session_state(clean_session, o).await {
                log::warn!("{:?} Failed to transfer session state, {}", state1.id, e);
            }
        });
    }

    //automatic subscription
    let auto_subscription = Runtime::instance().extends.auto_subscription().await;
    if auto_subscription.enable() {
        if let Some(tx) = &state.tx {
            auto_subscription.subscribe(state.id(), tx).await?;
        }
    }

    Ok(handshake.ack(state, session_present).idle_timeout(keep_alive))
}

async fn subscribes(
    state: &v3::Session<SessionState>,
    mut subs: v3::control::Subscribe,
) -> Result<v3::ControlResult> {
    let shared_subscription =
        Runtime::instance().extends.shared_subscription().await.is_supported(state.listen_cfg());
    let limit_subscription = state.listen_cfg().limit_subscription;
    for mut sub in subs.iter_mut() {
        let s = Subscribe::from_v3(sub.topic(), sub.qos(), shared_subscription, limit_subscription)?;
        let sub_ret = state.subscribe(s).await?;
        if let Some(qos) = sub_ret.success() {
            sub.confirm(qos)
        } else {
            sub.fail()
        }
    }
    Ok(subs.ack())
}

async fn unsubscribes(
    state: &v3::Session<SessionState>,
    unsubs: v3::control::Unsubscribe,
) -> Result<v3::ControlResult> {
    let shared_subscription =
        Runtime::instance().extends.shared_subscription().await.is_supported(state.listen_cfg());
    let limit_subscription = state.listen_cfg().limit_subscription;
    for topic_filter in unsubs.iter() {
        let unsub = Unsubscribe::from(topic_filter, shared_subscription, limit_subscription)?;
        state.unsubscribe(unsub).await?;
    }
    Ok(unsubs.ack())
}

#[inline]
pub async fn control_message(
    state: v3::Session<SessionState>,
    ctrl_msg: v3::ControlMessage,
) -> Result<v3::ControlResult, MqttError> {
    log::debug!("{:?} incoming control message -> {:?}", state.id, ctrl_msg);

    let crs = match ctrl_msg {
        v3::ControlMessage::Subscribe(subs) => {
            let _ = state.send(Message::Keepalive(false));
            match subscribes(&state, subs).await {
                Err(e) => {
                    log::warn!("{:?} Subscribe failed, reason: {}", state.id, e);
                    state
                        .disconnected_reason_add(Reason::SubscribeFailed(Some(ByteString::from(
                            e.to_string(),
                        ))))
                        .await?;
                    return Err(e);
                }
                Ok(r) => r,
            }
        }
        v3::ControlMessage::Unsubscribe(unsubs) => {
            let _ = state.send(Message::Keepalive(false));
            match unsubscribes(&state, unsubs).await {
                Err(e) => {
                    log::warn!("{:?} Unsubscribe failed, reason: {}", state.id, e);
                    state
                        .disconnected_reason_add(Reason::UnsubscribeFailed(Some(ByteString::from(
                            e.to_string(),
                        ))))
                        .await?;
                    return Err(e);
                }
                Ok(r) => r,
            }
        }
        v3::ControlMessage::Ping(ping) => {
            let _ = state.send(Message::Keepalive(true));
            ping.ack()
        }
        v3::ControlMessage::Disconnect(disc) => {
            //let _ = state.send(Message::Keepalive(false));
            state.send(Message::Disconnect(Disconnect::V3))?;
            disc.ack()
        }
        v3::ControlMessage::Closed(m) => {
            if let Err(e) = state.send(Message::Closed(Reason::ConnectRemoteClose)) {
                log::debug!("{:?} Closed error, reason: {}", state.id, e);
            }
            m.ack()
        }
    };

    Ok(crs)
}

#[inline]
pub async fn publish(state: v3::Session<SessionState>, pub_msg: v3::PublishMessage) -> Result<(), MqttError> {
    log::debug!("{:?} incoming publish message: {:?}", state.id, pub_msg);

    let _ = state.send(Message::Keepalive(false));

    match pub_msg {
        v3::PublishMessage::Publish(publish) => {
            let publish_fut = async move {
                if let Err(e) = state.publish_v3(&publish).await {
                    log::warn!(
                        "{:?} Publish failed, reason: {:?}",
                        state.id,
                        state.disconnected_reason().await
                    );
                    Err(e)
                } else {
                    Ok(())
                }
            };
            if Runtime::instance().is_busy() {
                Runtime::local_exec()
                    .spawn(publish_fut)
                    .result()
                    .await
                    .map_err(|e| MqttError::from(e.to_string()))??;
            } else {
                publish_fut.await?;
            }
        }
        v3::PublishMessage::PublishAck(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win().write().await.remove(&packet_id.get()) {
                //hook, message_ack
                state.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
            }
        }
        v3::PublishMessage::PublishReceived(packet_id) => {
            state.inflight_win().write().await.update_status(&packet_id.get(), MomentStatus::UnComplete);
        }
        v3::PublishMessage::PublishComplete(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win().write().await.remove(&packet_id.get()) {
                //hook, message_ack
                state.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
            }
        }
    }

    Ok(())
}
