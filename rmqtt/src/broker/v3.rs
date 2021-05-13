use anyhow::Result;
use ntex_mqtt::v3::{self, codec::SubscribeReturnCode};
use std::net::SocketAddr;

use crate::broker::{inflight::MomentStatus, types::*};
use crate::runtime::Runtime;
use crate::settings::listener::Listener;
use crate::{ClientInfo, MqttError, Session, SessionState};

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
pub async fn handshake<Io>(
    listen_cfg: Listener,
    mut handshake: v3::Handshake<Io>,
    remote_addr: SocketAddr,
    local_addr: SocketAddr,
) -> Result<v3::HandshakeAck<Io, SessionState>, MqttError> {
    log::trace!(
        "new Connection: local_addr: {:?}, remote: {:?}, {:?}, listen_cfg: {:?}",
        local_addr,
        remote_addr,
        handshake,
        listen_cfg
    );

    let limiter = Runtime::instance()
        .extends
        .limiter_mgr()
        .await
        .get(format!("{}", local_addr.port()), listen_cfg.clone())?;
    limiter.acquire_one().await?;

    let packet = handshake.packet().clone();

    let id = Id::new(
        Runtime::instance().node.id(),
        Some(local_addr),
        Some(remote_addr),
        packet.client_id.clone(),
        handshake.packet_mut().username.take(),
    );

    let connect_info = ConnectInfo::V3(id.clone(), packet);

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

    let sink = handshake.sink();
    let packet = handshake.packet_mut();

    let mut entry = match { Runtime::instance().extends.shared().await.entry(id.clone()) }.try_lock() {
        Err(e) => {
            return Ok(refused_ack(
                handshake,
                &connect_info,
                ConnectAckReasonV3::ServiceUnavailable,
                format!("{:?}", e),
            )
            .await);
        }
        Ok(entry) => entry,
    };

    // Kick out the current session, if it exists
    let (session_present, old_session) = match entry.kick(packet.clean_session).await {
        Err(e) => {
            return Ok(refused_ack(
                handshake,
                &connect_info,
                ConnectAckReasonV3::ServiceUnavailable,
                format!("{:?}", e),
            )
            .await);
        }
        Ok(Some((s, _))) => (!packet.clean_session, Some(s)),
        Ok(None) => (false, None),
    };

    let connected_at = chrono::Local::now().timestamp_millis();
    let client = ClientInfo::new(connect_info, session_present, connected_at);

    log::debug!("{:?} old session: {:?}", id, old_session);
    let (session, session_created) = if let Some(s) = old_session {
        (s, false)
    } else {
        let fitter = Runtime::instance().extends.fitter_mgr().await.get(id.clone(), listen_cfg.clone());
        (Session::new(listen_cfg, fitter, connected_at), true)
    };

    let keep_alive = match session.fitter.keep_alive(packet.keep_alive) {
        Ok(keep_alive) => keep_alive,
        Err(e) => {
            return Ok(refused_ack(
                handshake,
                &client.connect_info,
                ConnectAckReasonV3::ServiceUnavailable,
                format!("{:?}", e),
            )
            .await);
        }
    };

    let hook = Runtime::instance().extends.hook_mgr().await.hook(&session, &client);

    if session_created {
        //hook, session created
        hook.session_created().await;
    }

    //hook, client authenticate
    let ack = hook.client_authenticate(packet.password.take()).await;
    if !ack.success() {
        if let ConnectAckReason::V3(ack) = ack {
            return Ok(
                refused_ack(handshake, &client.connect_info, ack, "Authentication failed".into()).await
            );
        } else {
            unreachable!()
        }
    }

    let (state, tx) =
        SessionState::new(id.clone(), session, client, Sink::V3(sink), hook).start(keep_alive).await;

    if let Err(e) = entry.set(state.session.clone(), tx, state.client.clone()).await {
        return Ok(refused_ack(
            handshake,
            &state.client.connect_info,
            ConnectAckReasonV3::ServiceUnavailable,
            format!("{:?}", e),
        )
        .await);
    }

    //hook, client connack
    let _ = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .client_connack(
            &state.client.connect_info,
            ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
        )
        .await;

    //hook, client connected
    state.hook.client_connected().await;
    Ok(handshake.ack(state, session_present).idle_timeout(keep_alive))
}

#[inline]
pub async fn control_message(
    state: v3::Session<SessionState>,
    ctrl_msg: v3::ControlMessage,
) -> Result<v3::ControlResult, MqttError> {
    log::debug!("{:?} incoming control message -> {:?}", state.id, ctrl_msg);

    let _ = state.send(Message::Keepalive);

    let crs = match ctrl_msg {
        v3::ControlMessage::Subscribe(mut subs) => {
            let subs_ack = match state.subscribe_v3(&mut subs).await {
                Err(e) => {
                    state.client.set_disconnected_reason(format!("Subscribe failed, {:?}", e));
                    log::error!("{:?} Subscribe failed, reason: {:?}", state.id, e);
                    return Err(e);
                }
                Ok(subs_ack) => subs_ack,
            };

            log::debug!("{:?} Subscribe subs_ack: {:?}", state.id, subs_ack);
            let subs_len = subs.iter_mut().count();
            if subs.iter_mut().count() != subs_ack.len() {
                log::error!(
                    "{:?} Subscribe ack is abnormal, subs len is {}, subs_ack len is {}",
                    state.id,
                    subs_len,
                    subs_ack.len()
                );
            }

            for (idx, mut sub) in subs.iter_mut().enumerate() {
                match subs_ack.get(idx) {
                    Some(SubscribeReturnCode::Failure) => sub.fail(),
                    Some(SubscribeReturnCode::Success(qos)) => sub.confirm(*qos),
                    None => sub.fail(),
                }
            }
            subs.ack()
        }
        v3::ControlMessage::Unsubscribe(mut unsubs) => {
            if let Err(e) = state.unsubscribe_v3(&mut unsubs).await {
                state.client.set_disconnected_reason(format!("Unsubscribe failed, {:?}", e));
                log::error!("{:?} Unsubscribe failed, reason: {:?}", state.id, e);
                return Err(e);
            }
            unsubs.ack()
        }
        v3::ControlMessage::Ping(ping) => ping.ack(),
        v3::ControlMessage::Disconnect(disc) => {
            state.send(Message::Disconnect)?;
            disc.ack()
        }
        v3::ControlMessage::Closed(m) => {
            //hook, client_disconnected
            let reason = state
                .client
                .get_disconnected_reason()
                .unwrap_or_else(|| Reason::from_static("unknown error"));
            state.hook.client_disconnected(reason).await;
            if let Err(e) = state.send(Message::Closed) {
                log::error!("{:?} Closed error, reason: {:?}", state.id, e);
            }
            m.ack()
        }
    };

    Ok(crs)
}

#[inline]
pub async fn publish(state: v3::Session<SessionState>, pub_msg: v3::PublishMessage) -> Result<(), MqttError> {
    log::debug!("{:?} incoming publish message: {:?}", state.id, pub_msg);

    let _ = state.send(Message::Keepalive);

    match pub_msg {
        v3::PublishMessage::Publish(publish) => {
            if let Err(e) = state.publish_v3(publish).await {
                state.client.set_disconnected_reason(format!("Publish failed, {:?}", e));
                log::error!(
                    "{:?} Publish failed, reason: {:?}",
                    state.id,
                    state.client.get_disconnected_reason()
                );
                return Err(e);
            }
        }
        v3::PublishMessage::PublishAck(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win.write().remove(&packet_id.get()) {
                //hook, message_ack
                state.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
            }
        }
        v3::PublishMessage::PublishReceived(packet_id) => {
            state.inflight_win.write().update_status(&packet_id.get(), MomentStatus::UnComplete);
        }
        v3::PublishMessage::PublishComplete(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win.write().remove(&packet_id.get()) {
                //hook, message_ack
                state.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
            }
        }
    }

    Ok(())
}
