use anyhow::Result;
use ntex_mqtt::v3::{self, codec::SubscribeReturnCode};
use std::convert::From as _;
use std::net::SocketAddr;
use tokio::time::Duration;

use crate::broker::{inflight::MomentStatus, types::*};
use crate::runtime::Runtime;
use crate::settings::listener::Listener;
use crate::{Connection, MqttError, Session, SessionState};

#[inline]
pub async fn handshake<Io>(
    listen_cfg: Listener,
    mut handshake: v3::Handshake<Io>,
    remote_addr: SocketAddr,
    local_addr: SocketAddr,
) -> Result<v3::HandshakeAck<Io, SessionState>, MqttError> {
    log::debug!(
        "new connection: local_addr: {:?}, remote: {:?}, {:?}, listen_cfg: {:?}",
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

    let packet = handshake.packet_mut();

    if listen_cfg.max_clientid_len > 0 && packet.client_id.len() > listen_cfg.max_clientid_len {
        log::error!(
            "{:?} Connection Refused, handshake, identifier_rejected, client_id.len: {}",
            packet.client_id,
            packet.client_id.len()
        );
        return Ok(handshake.identifier_rejected());
    }

    let client_id = packet.client_id.clone();
    let node_id = Runtime::instance()
        .extends
        .router()
        .await
        .get_node_id()
        .await;

    let connected_at = chrono::Local::now().timestamp_millis();
    let id = Id::new(
        node_id,
        local_addr.to_string(),
        remote_addr.to_string(),
        client_id,
    );

    let mut entry =
        match { Runtime::instance().extends.shared().await.entry(id.clone()) }.try_lock() {
            Err(e) => {
                log::error!("{:?} Connection Refused, handshake, {}", id, e);
                return Ok(handshake.service_unavailable());
            }
            Ok(entry) => entry,
        };

    // Kick out the current session, if it exists
    let (session_present, old_session) = match entry.kick(packet.clean_session).await {
        Err(e) => {
            log::error!("{:?} Connection Refused, handshake, {}", id, e);
            return Ok(handshake.service_unavailable());
        }
        Ok(Some((s, _))) => (!packet.clean_session, Some(s)),
        Ok(None) => (false, None),
    };

    log::debug!("{:?} old session: {:?}", id, old_session);
    let (session, session_created) = if let Some(s) = old_session {
        (s, false)
    } else {
        let fitter = Runtime::instance()
            .extends
            .fitter_mgr()
            .await
            .get(id.clone(), listen_cfg.clone());
        (Session::new(listen_cfg, fitter, connected_at), true)
    };

    let keep_alive = match session.fitter.keep_alive(packet.keep_alive) {
        Ok(keep_alive) => keep_alive,
        Err(e) => {
            log::error!("{:?} Connection Refused, {}", id, e);
            return Err(MqttError::from(e));
        }
    };

    let conn = Connection::new(
        id,
        packet.protocol,
        packet.username.take(),
        Duration::from_secs(keep_alive as u64),
        packet.clean_session,
        packet.last_will.take().map(LastWill::V3),
        session_present,
        connected_at,
    );

    let hook = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .hook(&session, &conn);

    if session_created {
        //hook, session created
        if hook.session_created().await {
            log::info!(
                "{:?} Connection Refused, reason: Failed to create session",
                conn.id
            );
            hook.client_connack(ConnectAckReason::V3(
                v3::codec::ConnectAckReason::ServiceUnavailable,
            ))
            .await;
            return Ok(handshake.service_unavailable());
        }
    }

    //hook, client connect
    if hook.client_connect().await {
        log::info!(
            "{:?} Connection Refused, reason: Failed to create connection",
            conn.id
        );
        hook.client_connack(ConnectAckReason::V3(
            v3::codec::ConnectAckReason::ServiceUnavailable,
        ))
        .await;
        return Ok(handshake.service_unavailable());
    }

    //hook, client authenticate
    let ack = hook.client_authenticate(packet.password.take()).await;
    if !ack.success() {
        log::info!(
            "{:?} Connection Refused, reason: Authentication failed, {:?}",
            conn.id,
            ack.reason()
        );
        let h_ack = ack.v3_error_ack(handshake);
        hook.client_connack(ack).await;
        return Ok(h_ack);
    }

    let (state, tx) = SessionState::new(
        conn.id.clone(),
        session,
        conn,
        Sink::V3(handshake.sink()),
        hook,
    )
    .start()
    .await;

    if let Err(e) = entry
        .set(state.session.clone(), tx, state.conn.clone())
        .await
    {
        log::error!("{:?} Connection Refused, handshake, {}", state.id, e);
        state
            .hook
            .client_connack(ConnectAckReason::V3(
                v3::codec::ConnectAckReason::ServiceUnavailable,
            ))
            .await;
        return Ok(handshake.service_unavailable());
    }

    //hook, client connack
    state
        .hook
        .client_connack(ConnectAckReason::V3(
            v3::codec::ConnectAckReason::ConnectionAccepted,
        ))
        .await;

    //hook, client connected
    state.hook.client_connected().await;
    Ok(handshake
        .ack(state, session_present)
        .idle_timeout(keep_alive))
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
                    state
                        .conn
                        .set_disconnected_reason(format!("Subscribe failed, {:?}", e));
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
                state
                    .conn
                    .set_disconnected_reason(format!("Unsubscribe failed, {:?}", e));
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
                .conn
                .take_disconnected_reason()
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
pub async fn publish(
    state: v3::Session<SessionState>,
    pub_msg: v3::PublishMessage,
) -> Result<(), MqttError> {
    log::debug!("{:?} incoming publish message: {:?}", state.id, pub_msg);

    let _ = state.send(Message::Keepalive);

    match pub_msg {
        v3::PublishMessage::Publish(publish) => {
            if let Err(e) = state.publish_v3(publish).await {
                state
                    .conn
                    .set_disconnected_reason(format!("Publish failed, {:?}", e));
                log::error!(
                    "{:?} Publish failed, reason: {:?}",
                    state.id,
                    state.conn.get_disconnected_reason()
                );
                return Err(e);
            }
        }
        v3::PublishMessage::PublishAck(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win.write().remove(&packet_id.get()) {
                //hook, message_ack
                state
                    .hook
                    .message_acked(iflt_msg.from, iflt_msg.publish)
                    .await;
            }
        }
        v3::PublishMessage::PublishReceived(packet_id) => {
            state
                .inflight_win
                .write()
                .update_status(&packet_id.get(), MomentStatus::UnComplete);
        }
        v3::PublishMessage::PublishComplete(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win.write().remove(&packet_id.get()) {
                //hook, message_ack
                state
                    .hook
                    .message_acked(iflt_msg.from, iflt_msg.publish)
                    .await;
            }
        }
    }

    Ok(())
}
