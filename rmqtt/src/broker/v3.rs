use bytestring::ByteString;
use std::convert::From as _f;
use std::net::SocketAddr;

use ntex_mqtt::v3::{self};

use crate::broker::executor::get_handshake_exec;
use crate::broker::{inflight::MomentStatus, types::*};
use crate::runtime::Runtime;
use crate::settings::listener::Listener;
use crate::{ClientInfo, MqttError, Result, Session, SessionState};

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
    handshake: v3::Handshake<Io>,
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

    let id = Id::new(
        Runtime::instance().node.id(),
        Some(local_addr),
        Some(remote_addr),
        handshake.packet().client_id.clone(),
        handshake.packet().username.clone(),
    );

    Runtime::instance().stats.handshakings.max_max(handshake.handshakings());

    let exec = get_handshake_exec(local_addr.port(), listen_cfg.clone());
    match exec.spawn(_handshake(id.clone(), listen_cfg, handshake)).await {
        Ok(Ok(res)) => Ok(res),
        Ok(Err(e)) => {
            log::warn!("{:?} Connection Refused, handshake error, reason: {:?}", id, e);
            Err(e)
        }
        Err(e) => {
            log::warn!("{:?} Connection Refused, handshake timeout, reason: {:?}", id, e);
            Err(MqttError::from("Connection Refused, execute handshake timeout"))
        }
    }
}

#[inline]
async fn _handshake<Io: 'static>(
    id: Id,
    listen_cfg: Listener,
    mut handshake: v3::Handshake<Io>,
) -> Result<v3::HandshakeAck<Io, SessionState>, MqttError> {
    let connect_info = ConnectInfo::V3(id.clone(), handshake.packet().clone());

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
                format!("{:?}", e),
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
                    format!("{:?}", e),
                )
                .await);
            }
            Ok(Some(offline_info)) => (!packet.clean_session, Some(offline_info)),
            Ok(None) => (false, None),
        };

    let connected_at = chrono::Local::now().timestamp_millis();
    let client = ClientInfo::new(connect_info, session_present, superuser, connected_at);
    let fitter =
        Runtime::instance().extends.fitter_mgr().await.get(client.clone(), id.clone(), listen_cfg.clone());

    log::debug!("{:?} offline_info: {:?}", id, offline_info);
    let created_at =
        if let Some(ref offline_info) = offline_info { offline_info.created_at } else { connected_at };

    let max_inflight = fitter.max_inflight();
    let session = Session::new(id, fitter, listen_cfg, max_inflight, created_at);

    let keep_alive = match session.fitter.keep_alive(&mut packet.keep_alive) {
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

    if offline_info.is_none() {
        //hook, session created
        hook.session_created().await;
    }

    let (state, tx) = SessionState::new(session, client, Sink::V3(sink), hook).start(keep_alive).await;
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

    //transfer session state
    if let Some(o) = offline_info {
        let state1 = state.clone();
        let clean_session = packet.clean_session;
        ntex::rt::spawn(async move {
            if let Err(e) = state1.transfer_session_state(clean_session, o).await {
                log::warn!("{:?} Failed to transfer session state, {:?}", state1.id, e);
            }
        });
    }

    Ok(handshake.ack(state, session_present).idle_timeout(keep_alive))
}

async fn subscribes(
    state: &v3::Session<SessionState>,
    mut subs: v3::control::Subscribe,
) -> Result<v3::ControlResult> {
    let shared_subscription_supported =
        Runtime::instance().extends.shared_subscription().await.is_supported(&state.listen_cfg);
    for mut sub in subs.iter_mut() {
        let s = Subscribe::from_v3(sub.topic(), sub.qos(), shared_subscription_supported)?;
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
    let shared_subscription_supported =
        Runtime::instance().extends.shared_subscription().await.is_supported(&state.listen_cfg);
    for topic_filter in unsubs.iter() {
        let unsub = Unsubscribe::from(topic_filter, shared_subscription_supported)?;
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

    let _ = state.send(Message::Keepalive);

    let crs = match ctrl_msg {
        v3::ControlMessage::Subscribe(subs) => match subscribes(&state, subs).await {
            Err(e) => {
                state
                    .client
                    .add_disconnected_reason(Reason::SubscribeFailed(Some(ByteString::from(e.to_string()))))
                    .await;
                log::error!("{:?} Subscribe failed, reason: {:?}", state.id, e);
                return Err(e);
            }
            Ok(r) => r,
        },
        v3::ControlMessage::Unsubscribe(unsubs) => match unsubscribes(&state, unsubs).await {
            Err(e) => {
                state
                    .client
                    .add_disconnected_reason(Reason::UnsubscribeFailed(Some(ByteString::from(e.to_string()))))
                    .await;
                log::error!("{:?} Unsubscribe failed, reason: {:?}", state.id, e);
                return Err(e);
            }
            Ok(r) => r,
        },
        v3::ControlMessage::Ping(ping) => ping.ack(),
        v3::ControlMessage::Disconnect(disc) => {
            state.send(Message::Disconnect(Disconnect::V3))?;
            disc.ack()
        }
        v3::ControlMessage::Closed(m) => {
            if let Err(e) = state.send(Message::Closed(Reason::ConnectRemoteClose)) {
                log::debug!("{:?} Closed error, reason: {:?}", state.id, e);
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
            if let Err(e) = state.publish_v3(&publish).await {
                log::error!(
                    "{:?} Publish failed, reason: {:?}",
                    state.id,
                    state.client.get_disconnected_reason().await
                );
                return Err(e);
            }
        }
        v3::PublishMessage::PublishAck(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win.write().await.remove(&packet_id.get()) {
                //hook, message_ack
                state.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
            }
        }
        v3::PublishMessage::PublishReceived(packet_id) => {
            state.inflight_win.write().await.update_status(&packet_id.get(), MomentStatus::UnComplete);
        }
        v3::PublishMessage::PublishComplete(packet_id) => {
            if let Some(iflt_msg) = state.inflight_win.write().await.remove(&packet_id.get()) {
                //hook, message_ack
                state.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
            }
        }
    }

    Ok(())
}
