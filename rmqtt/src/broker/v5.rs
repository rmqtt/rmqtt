use std::net::SocketAddr;

use ntex_mqtt::v5;
use ntex_mqtt::v5::codec::{Auth, DisconnectReasonCode};

use crate::broker::types::*;
use crate::settings::listener::Listener;
use crate::{ClientInfo, MqttError, Runtime, Session, SessionState};

#[inline]
pub async fn handshake<Io>(
    listen_cfg: Listener,
    mut handshake: v5::Handshake<Io>,
    remote_addr: SocketAddr,
    local_addr: SocketAddr,
) -> Result<v5::HandshakeAck<Io, SessionState>, MqttError> {
    log::debug!("new connection: {:?}", handshake);

    let packet = handshake.packet_mut();
    // let keep_alive = (if packet.keep_alive == 0 {
    //     60
    // } else {
    //     packet.keep_alive
    // } as f32
    //     * 1.5)
    //     .round() as u16;

    let id =
        Id::new(1, Some(local_addr), Some(remote_addr), packet.client_id.clone(), packet.username.clone());
    let fitter = Runtime::instance().extends.fitter_mgr().await.get(id.clone(), listen_cfg.clone());

    let session = Session::new(listen_cfg, fitter, 0);
    let session_present = false;
    let connected_at = chrono::Local::now().timestamp_millis();

    let conn = ClientInfo::new(
        ConnectInfo::V5(id, Box::new(handshake.packet().clone())),
        session_present,
        connected_at,
    );

    let hook = Runtime::instance().extends.hook_mgr().await.hook(&session, &conn);

    let state = SessionState::new(conn.id.clone(), session, conn, Sink::V5(handshake.sink()), hook);

    Ok(handshake.ack(state))
}

#[inline]
pub async fn publish(
    session: v5::Session<SessionState>,
    publish: v5::Publish,
) -> Result<v5::PublishAck, MqttError> {
    log::info!(
        "incoming publish ({:?}) : {:?} -> {:?}",
        session.state().client.id,
        publish.id(),
        publish.topic()
    );

    Ok(publish.ack())
}

pub async fn control_message<E>(
    _session: v5::Session<SessionState>,
    ctrl_msg: v5::ControlMessage<E>,
) -> Result<v5::ControlResult, MqttError> {
    let crs = match ctrl_msg {
        v5::ControlMessage::Auth(auth) => auth.ack(Auth::default()),
        v5::ControlMessage::Ping(ping) => ping.ack(),
        v5::ControlMessage::Disconnect(disconnect) => disconnect.ack(),
        v5::ControlMessage::Subscribe(subscribe) => subscribe.ack(),
        v5::ControlMessage::Unsubscribe(unsubscribe) => unsubscribe.ack(),
        v5::ControlMessage::Closed(closed) => closed.ack(),
        v5::ControlMessage::Error(e) => e.ack(DisconnectReasonCode::ServerBusy),
        v5::ControlMessage::ProtocolError(protocol_error) => protocol_error.ack(),
    };

    Ok(crs)
}
