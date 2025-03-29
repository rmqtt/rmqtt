use anyhow::anyhow;
use std::convert::From as _f;
use std::fmt;
use std::num::NonZeroU16;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
#[allow(unused_imports)]
use bitflags::Flags;
use bytestring::ByteString;
use futures::StreamExt;

use rmqtt_codec::v5::{Auth, PublishAck2, PublishAck2Reason, RetainHandling, ToReasonCode, UserProperties};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

use crate::acl::AuthInfo;
use crate::codec::{
    v3,
    v5::{self, SubscribeAckReason},
};
use crate::context::ServerContext;
use crate::hook::Hook;
use crate::inflight::{InInflight, MomentStatus, OutInflight, OutInflightMessage};
use crate::net::MqttError;
use crate::queue::{self, Limiter, Policy};
use crate::types::*;
use crate::utils::timestamp_millis;
use crate::Result;

pub struct SessionState {
    inner: Session,
    tx: Tx,
    rx: Rx,
    pub hook: Arc<dyn Hook>,
    // pub deliver_queue_tx: Option<MessageSender>,
    pub server_topic_aliases: Option<Arc<ServerTopicAliases>>,
    pub client_topic_aliases: Option<Arc<ClientTopicAliases>>,
    in_inflight: InInflight,
}

impl fmt::Debug for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionState {{ {:?}, {:?} }}", self.id, self.inner,)
    }
}

impl Deref for SessionState {
    type Target = Session;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl SessionState {
    #[inline]
    pub(crate) fn new(
        session: Session,
        hook: Arc<dyn Hook>,
        server_topic_alias_max: u16,
        client_topic_alias_max: u16,
    ) -> Self {
        let server_topic_aliases = if server_topic_alias_max > 0 {
            Some(Arc::new(ServerTopicAliases::new(server_topic_alias_max as usize)))
        } else {
            None
        };
        let client_topic_aliases = if client_topic_alias_max > 0 {
            Some(Arc::new(ClientTopicAliases::new(client_topic_alias_max as usize)))
        } else {
            None
        };
        log::debug!("server_topic_aliases: {:?}", server_topic_aliases);
        log::debug!("client_topic_aliases: {:?}", client_topic_aliases);

        let (tx, rx) = futures::channel::mpsc::unbounded();
        let tx = SessionTx::new(
            tx,
            #[cfg(feature = "debug")]
            session.scx.clone(),
        );
        let scx = session.scx.clone();
        let max_inflight = session.listen_cfg().max_inflight;
        Self {
            inner: session,
            tx,
            rx,
            hook,
            // deliver_queue_tx: None,
            server_topic_aliases,
            client_topic_aliases,
            in_inflight: InInflight::new(scx, max_inflight.get()),
        }
    }

    #[inline]
    pub fn session(&self) -> &Session {
        &self.inner
    }

    #[inline]
    pub fn tx(&self) -> &Tx {
        &self.tx
    }

    #[inline]
    pub(crate) async fn run<Io>(mut self, mut sink: Sink<Io>, keep_alive: u16) -> Result<()>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        let limiter = {
            let (burst, replenish_n_per) = self.fitter.mqueue_rate_limit();
            Limiter::new(burst, replenish_n_per)?
        };

        let (deliver_queue_tx, mut deliver_queue_rx) = self.deliver_queue_channel(&limiter);
        let mut flags = StateFlags::empty();
        self.scx.stats.connections.inc();
        match self.run_loop(&mut sink, keep_alive, &mut flags, &deliver_queue_tx, &mut deliver_queue_rx).await
        {
            Ok(()) => {
                log::info!("{} exit ...", self.id);
            }
            Err(reason) => {
                // log::info!("{} Reason: {}", self.id, reason);
                self.disconnected_reason_add(reason).await?;
            }
        }
        self.scx.stats.connections.dec();

        let disconnect = self.disconnect().await.unwrap_or(None);
        let clean_session = self.clean_session(disconnect.as_ref()).await;

        log::info!(
            "{:?} exit online worker, flags: {:?}, clean_session: {} {}",
            self.id,
            flags,
            self.connect_info().await?.clean_start(),
            flags.contains(StateFlags::CleanStart)
        );

        //Setting the disconnected state
        if let Err(e) = self.disconnected_set(None, None).await {
            log::warn!("{:?} disconnected set error, {:?}", self.id, e);
        }

        log::info!(
            "{} disconnected_reason({}): {}",
            self.id,
            self.disconnected_reasons().await?.len(),
            self.disconnected_reason().await?
        );

        //Last will message
        let will_delay_interval = if self.last_will_enable(flags, clean_session) {
            let will_delay_interval = self.will_delay_interval().await;
            if clean_session || will_delay_interval.is_none() {
                if let Err(e) = self.process_last_will().await {
                    log::error!("{:?} process last will error, {:?}", self.id, e);
                }
                None
            } else {
                will_delay_interval
            }
        } else {
            None
        };

        //hook, client_disconnected
        let reason = if self.disconnected_reason_has().await {
            self.disconnected_reason().await.unwrap_or_default()
        } else {
            if let Err(e) = self.disconnected_reason_add(Reason::ConnectRemoteClose).await {
                log::warn!("{:?} disconnected reason add error: {:?}", self.id, e);
            }
            Reason::ConnectRemoteClose
        };

        //@TODO ... 需要优化 Reason, 定义，可参考： DisconnectReasonCode
        //向客户端发送‌DISCONNECT 报文，如果是MQTT 5.0
        if let Sink::V5(s) = &mut sink {
            let d = if let Reason::ConnectDisconnect(Some(Disconnect::V5(d))) = &reason {
                d.clone()
            } else {
                v5::Disconnect {
                    reason_code: reason.to_reason_code(),
                    reason_string: Some(reason.to_string().into()),
                    ..Default::default()
                }
            };
            let _ = s.send_disconnect(d).await;
        }

        let _ = sink.close().await;

        self.hook.client_disconnected(reason).await;

        if flags.contains(StateFlags::Kicked) {
            if flags.contains(StateFlags::ByAdminKick) {
                self.clean(&deliver_queue_tx, self.disconnected_reason_take().await.unwrap_or_default())
                    .await;
            }
        } else if clean_session {
            self.clean(&deliver_queue_tx, self.disconnected_reason_take().await.unwrap_or_default()).await;
        } else {
            let session_expiry_interval = self.fitter.session_expiry_interval(disconnect.as_ref());
            //hook, offline_inflight_messages
            let inflight_messages = self.inflight_win().write().await.clone_inflight_messages();
            if !inflight_messages.is_empty() {
                self.hook.offline_inflight_messages(inflight_messages).await;
            }

            //Start offline event loop
            self.offline_run_loop(
                &deliver_queue_tx,
                &mut flags,
                will_delay_interval,
                session_expiry_interval,
            )
            .await;
            log::debug!("{:?} offline flags: {:?}", self.id, flags);
            if !flags.contains(StateFlags::Kicked) {
                self.clean(&deliver_queue_tx, Reason::SessionExpiration).await;
            }
        }

        Ok(())
    }

    #[inline]
    async fn run_loop<Io>(
        &mut self,
        sink: &mut Sink<Io>,
        keep_alive: u16,
        flags: &mut StateFlags,
        deliver_queue_tx: &queue::Sender<(From, Publish)>,
        deliver_queue_rx: &mut queue::Receiver<'_, (From, Publish)>,
    ) -> std::result::Result<(), Reason>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        log::debug!("{:?} start online event loop", self.id);
        let state = self;

        let keep_alive_interval = if keep_alive == 0 {
            Duration::from_secs(u32::MAX as u64)
        } else {
            Duration::from_secs(keep_alive as u64)
        };
        log::debug!("{:?} keep_alive_interval is {:?}", state.id, keep_alive_interval);
        let keep_alive_delay = tokio::time::sleep(keep_alive_interval);

        let deliver_timeout_delay = tokio::time::sleep(Duration::from_secs(60));

        log::debug!("{:?} there are {} offline messages ...", state.id, state.deliver_queue().len());

        tokio::pin!(keep_alive_delay);

        tokio::pin!(deliver_timeout_delay);

        loop {
            log::debug!("{:?} tokio::select! loop", state.id);
            deliver_timeout_delay.as_mut().reset(
                Instant::now()
                    + state
                        .inflight_win()
                        .read()
                        .await
                        .get_timeout()
                        .unwrap_or_else(|| Duration::from_secs(120)),
            );

            tokio::select! {
                 _ = &mut keep_alive_delay => {
                    return Err(Reason::ConnectKeepaliveTimeout)
                },

                _ = &mut deliver_timeout_delay => {
                    while let Some(iflt_msg) = state.inflight_win().write().await.pop_front_timeout(){
                        log::debug!("{:?} has timeout message in inflight: {:?}", state.id, iflt_msg);
                        state.reforward(iflt_msg).await?;
                    }
                },

                deliver_packet = deliver_queue_rx.next(), if state.inflight_win().read().await.has_credit() => {
                    log::debug!("{:?} deliver_packet: {:?}", state.id, deliver_packet);
                    match deliver_packet {
                        Some(Some((from, p))) => {
                            state.deliver(sink, from, p).await?;
                        },
                        Some(None) => {
                            log::warn!("{:?} No messages received from the delivery queue", state.id);
                        },
                        None => {
                            return Err("Message delivery queue is closed".into())
                        }
                    }
                }

                msg = state.rx.next() => {
                    log::debug!("{:?} msg: {:?}", state.id, msg);
                    if let Some(msg) = msg {
                        #[cfg(feature = "debug")]
                        state.scx.stats.debug_session_channels.dec();
                        state.process_message(sink, msg, &deliver_queue_tx, flags).await?;
                    }else{
                        return Err("No message received from the Rx".into());
                    }
                }

                pkt = sink.recv() => {
                    log::debug!("{:?} pkt: {:?}", state.id, pkt);
                    keep_alive_delay.as_mut().reset(Instant::now() + keep_alive_interval);
                    match pkt? {
                        Some(pkt) => {
                            state.process_mqtt_message(sink, pkt, flags).await?;
                        },
                        None => {
                            return Err(Reason::ConnectRemoteClose);
                        }
                    }
                }
            }
        }
    }

    #[inline]
    async fn offline_run_loop(
        &mut self,
        deliver_queue_tx: &MessageSender,
        flags: &mut StateFlags,
        mut will_delay_interval: Option<Duration>,
        session_expiry_interval: Duration,
    ) {
        log::debug!(
            "{:?} start offline event loop, session_expiry_interval: {:?}, will_delay_interval: {:?}",
            self.id,
            session_expiry_interval,
            will_delay_interval
        );

        //state.disconnect
        let session_expiry_delay = tokio::time::sleep(session_expiry_interval);
        tokio::pin!(session_expiry_delay);

        let will_delay_interval_delay = tokio::time::sleep(will_delay_interval.unwrap_or(Duration::MAX));
        tokio::pin!(will_delay_interval_delay);

        loop {
            tokio::select! {
                msg = self.rx.next() => {
                    log::debug!("{:?} recv offline msg: {:?}", self.id, msg);
                    if let Some(msg) = msg {
                        match msg {
                            Message::Forward(from, p) => {

                                //hook, offline_message
                                self.hook.offline_message(from.clone(), &p).await;

                                if let Err((from, p)) = deliver_queue_tx.send((from, p)).await {
                                    log::debug!("{:?} offline deliver_dropped, from: {:?}, {:?}", self.id, from, p);
                                    //hook, message_dropped
                                   self.scx.extends.hook_mgr().message_dropped(Some(self.id.clone()), from, p, Reason::MessageQueueFull).await;
                                }
                            },
                            Message::Kick(sender, by_id, clean_start, is_admin) => {
                                log::debug!("{:?} offline Kicked, send kick result, to: {:?}, clean_start: {}, is_admin: {}", self.id, by_id, clean_start, is_admin);
                                if !sender.is_closed() {
                                    if let Err(e) = sender.send(()) {
                                        log::warn!("{:?} offline Kick send response error, to: {:?}, clean_start: {}, is_admin: {}, {:?}", self.id, by_id, clean_start, is_admin, e);
                                    }
                                    flags.insert(StateFlags::Kicked);
                                    if is_admin {
                                        flags.insert(StateFlags::ByAdminKick);
                                    }
                                    if clean_start {
                                        flags.insert(StateFlags::CleanStart);
                                    }
                                    break
                                }else{
                                    log::warn!("{:?} offline Kick sender is closed, to {:?}, clean_start: {}, is_admin: {}", self.id, by_id, clean_start, is_admin);
                                }
                            },
                            _ => {
                                log::debug!("{:?} offline receive message is {:?}", self.id, msg);
                            }
                        }
                    }else{
                        log::warn!("{:?} offline None is received from the Rx", self.id);
                        break;
                    }
                },
               _ = &mut session_expiry_delay => { //, if !session_expiry_delay.is_elapsed() => {
                  log::debug!("{:?} session expired, will_delay_interval: {:?}", self.id, will_delay_interval);
                  if will_delay_interval.is_some() {
                      if let Err(e) = self.process_last_will().await {
                          log::error!("{:?} process last will error, {:?}", self.id, e);
                      }
                  }
                  break
               },
               _ = &mut will_delay_interval_delay => { //, if !will_delay_interval_delay.is_elapsed() => {
                  log::debug!("{:?} will delay interval, will_delay_interval: {:?}", self.id, will_delay_interval);
                  if will_delay_interval.is_some() {
                      if let Err(e) = self.process_last_will().await {
                          log::error!("{:?} process last will error, {:?}", self.id, e);
                      }
                      will_delay_interval = None;
                  }
                  will_delay_interval_delay.as_mut().reset(
                    Instant::now() + session_expiry_interval,
                  );
               },
            }
        }
        log::debug!("{:?} exit offline worker", self.id);
    }

    #[inline]
    async fn process_message<Io>(
        &mut self,
        sink: &mut Sink<Io>,
        msg: Message,
        deliver_queue_tx: &queue::Sender<(From, Publish)>,
        flags: &mut StateFlags,
    ) -> std::result::Result<(), Reason>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        match msg {
            Message::Forward(from, p) => {
                if let Err((from, p)) = deliver_queue_tx.send((from, p)).await {
                    log::debug!("{:?} deliver_dropped, from: {:?}, {:?}", self.id, from, p);
                    //hook, message_dropped
                    self.scx
                        .extends
                        .hook_mgr()
                        .message_dropped(Some(self.id.clone()), from, p, Reason::MessageQueueFull)
                        .await;
                }
            }
            Message::SendRerelease(iflt_msg) => {
                self.send_rerelease(sink, iflt_msg).await?;
            }
            Message::Kick(sender, by_id, clean_start, is_admin) => {
                log::debug!(
                    "{:?} Message::Kick, send kick result, to {:?}, clean_start: {}, is_admin: {}",
                    self.id,
                    by_id,
                    clean_start,
                    is_admin
                );
                if !sender.is_closed() {
                    if sender.send(()).is_err() {
                        log::warn!("{:?} Message::Kick, send response error, sender is closed", self.id);
                    }
                    flags.insert(StateFlags::Kicked);
                    if is_admin {
                        flags.insert(StateFlags::ByAdminKick);
                    }
                    if clean_start {
                        flags.insert(StateFlags::CleanStart);
                    }
                    return Err(Reason::ConnectKicked(is_admin));
                } else {
                    log::warn!(
                        "{:?} Message::Kick, kick sender is closed, to {:?}, is_admin: {}",
                        self.id,
                        by_id,
                        is_admin
                    );
                }
            }
            Message::Subscribe(sub, reply_tx) => {
                log::debug!("{:?} Message::Subscribe, sub {:?}", self.id, sub,);
                let sub_reply = self.subscribe(sub).await;
                if !reply_tx.is_closed() {
                    if let Err(e) = reply_tx.send(sub_reply) {
                        log::warn!("{:?} Message::Subscribe, send response error, {:?}", self.id, e);
                    }
                } else {
                    log::warn!("{:?} Message::Subscribe, reply sender is closed", self.id);
                }
            }
            Message::Unsubscribe(unsub, reply_tx) => {
                log::debug!("{:?} Message::Unsubscribe, unsub {:?}", self.id, unsub,);
                let unsub_reply = self.unsubscribe(unsub).await;
                if !reply_tx.is_closed() {
                    if let Err(e) = reply_tx.send(unsub_reply) {
                        log::warn!("{:?} Message::Unsubscribe, send response error, {:?}", self.id, e);
                    }
                } else {
                    log::warn!("{:?} Message::Unsubscribe, reply sender is closed", self.id);
                }
            }
            Message::SessionStateTransfer(offline_info, clean_start) => {
                self.transfer_session_state(clean_start, offline_info).await?;
            }
        }
        Ok(())
    }

    #[inline]
    async fn process_mqtt_message<Io>(
        &mut self,
        sink: &mut Sink<Io>,
        pkt: Packet,
        flags: &mut StateFlags,
    ) -> std::result::Result<(), Reason>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        match pkt {
            Packet::V3(v3::Packet::Publish(publish)) => {
                log::debug!("{} publish: {:?}", self.id, publish);
                self.process_publish(sink, publish).await?;
            }
            Packet::V5(v5::Packet::Publish(publish)) => {
                log::debug!("{} publish: {:?}", self.id, publish);
                self.process_publish(sink, publish).await?;
            }

            Packet::V3(v3::Packet::PublishRelease { packet_id }) => {
                self.in_inflight.remove(&packet_id);
                sink.v3_mut().send_publish_complete(packet_id).await?;
            }
            Packet::V5(v5::Packet::PublishRelease(ack2)) => {
                self.in_inflight.remove(&ack2.packet_id);
                sink.v5_mut()
                    .send_publish_complete(PublishAck2 { packet_id: ack2.packet_id, ..Default::default() })
                    .await?;
            }

            Packet::V3(v3::Packet::PublishAck { packet_id }) => {
                if let Some(iflt_msg) = self.inflight_win().write().await.remove(&packet_id.get()) {
                    //hook, message_ack
                    self.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
                }
            }
            Packet::V5(v5::Packet::PublishAck(ack)) => {
                if let Some(iflt_msg) = self.inflight_win().write().await.remove(&ack.packet_id.get()) {
                    //hook, message_ack
                    self.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
                }
            }

            Packet::V3(v3::Packet::PublishReceived { packet_id }) => {
                self.inflight_win().write().await.update_status(&packet_id.get(), MomentStatus::UnComplete);
                sink.v3_mut().send_publish_release(packet_id).await?;
            }
            Packet::V5(v5::Packet::PublishReceived(ack)) => {
                self.inflight_win()
                    .write()
                    .await
                    .update_status(&ack.packet_id.get(), MomentStatus::UnComplete);

                sink.v5_mut()
                    .send_publish_release(PublishAck2 { packet_id: ack.packet_id, ..Default::default() })
                    .await?;
            }

            Packet::V3(v3::Packet::PublishComplete { packet_id }) => {
                if let Some(iflt_msg) = self.inflight_win().write().await.remove(&packet_id.get()) {
                    //hook, message_ack
                    self.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
                }
            }
            Packet::V5(v5::Packet::PublishComplete(ack2)) => {
                if let Some(iflt_msg) = self.inflight_win().write().await.remove(&ack2.packet_id.get()) {
                    //hook, message_ack
                    self.hook.message_acked(iflt_msg.from, &iflt_msg.publish).await;
                }
            }

            Packet::V3(v3::Packet::Subscribe { packet_id, topic_filters }) => {
                let status = match self.subscribes_v3(topic_filters).await {
                    Ok(status) => status,
                    Err(e) => {
                        return Err(Reason::SubscribeFailed(Some(e.to_string().into())));
                    }
                };
                sink.v3_mut().send_subscribe_ack(packet_id, status).await?;
            }
            Packet::V5(v5::Packet::Subscribe(subs)) => {
                let ack = match self.subscribes_v5(subs).await {
                    Err(e) => {
                        return Err(Reason::SubscribeFailed(Some(e.to_string().into())));
                    }
                    Ok(ack) => ack,
                };
                sink.v5_mut().send_subscribe_ack(ack).await?;
            }

            Packet::V3(v3::Packet::Unsubscribe { packet_id, topic_filters }) => {
                match self.unsubscribes_v3(topic_filters).await {
                    Err(e) => {
                        return Err(Reason::UnsubscribeFailed(Some(e.to_string().into())));
                    }
                    Ok(()) => {}
                }
                sink.v3_mut().send_unsubscribe_ack(packet_id).await?;
            }

            Packet::V5(v5::Packet::Unsubscribe(unsubs)) => {
                let ack = match self.unsubscribes_v5(unsubs).await {
                    Err(e) => {
                        return Err(Reason::UnsubscribeFailed(Some(e.to_string().into())));
                    }
                    Ok(ack) => ack,
                };
                sink.v5_mut().send_unsubscribe_ack(ack).await?;
            }

            Packet::V3(v3::Packet::PingRequest) => {
                sink.v3_mut().send_ping_response().await?;
                flags.insert(StateFlags::Ping);
            }
            Packet::V5(v5::Packet::PingRequest) => {
                sink.v5_mut().send_ping_response().await?;
                flags.insert(StateFlags::Ping);
            }

            Packet::V3(v3::Packet::Disconnect) => {
                flags.insert(StateFlags::DisconnectReceived);
                self.disconnected_set(Some(Disconnect::V3), None).await?;
                // return Err(Reason::ConnectDisconnect(Some(Disconnect::V3)));
                return Ok(());
            }
            Packet::V5(v5::Packet::Disconnect(d)) => {
                flags.insert(StateFlags::DisconnectReceived);
                self.disconnected_set(Some(Disconnect::V5(d)), None).await?;
                // return Err(Reason::ConnectDisconnect(Some(Disconnect::V5(d))));
                return Ok(());
            }
            Packet::V5(v5::Packet::Auth(_)) => {
                sink.v5_mut().send_auth(Auth::default()).await?;
                //@TODO 考虑通过hook来实现Auth
            }
            _ => {
                return Err(format!("Received an unimplemented message, {:?}", pkt).into());
            }
        }

        let is_ping = flags.contains(StateFlags::Ping);
        //hook, keepalive
        self.hook.client_keepalive(is_ping).await;
        self.keepalive(is_ping).await;
        if is_ping {
            flags.remove(StateFlags::Ping);
        }
        Ok(())
    }

    #[inline]
    fn last_will_enable(&self, flags: StateFlags, clean_session: bool) -> bool {
        let session_present =
            flags.contains(StateFlags::Kicked) && !flags.contains(StateFlags::CleanStart) && !clean_session;
        !(flags.contains(StateFlags::DisconnectReceived) || session_present)
    }

    #[inline]
    async fn will_delay_interval(&self) -> Option<Duration> {
        self.connect_info().await.ok()?.last_will().and_then(|lw| lw.will_delay_interval())
    }

    #[inline]
    async fn process_last_will(&self) -> Result<()> {
        if let Ok(conn_info) = self.connect_info().await {
            if let Some(lw) = conn_info.last_will() {
                let p = Publish::try_from(lw)?;
                let from = From::from_lastwill(self.id.clone());
                //hook, message_publish
                let p = self.hook.message_publish(from.clone(), &p).await.unwrap_or(p);
                log::debug!("process_last_will, publish: {:?}", p);

                let message_storage_available = self.scx.extends.message_mgr().await.enable();

                let message_expiry_interval =
                    if message_storage_available || (p.retain && self.scx.extends.retain().await.enable()) {
                        Some(self.fitter.message_expiry_interval(&p))
                    } else {
                        None
                    };

                Self::forwards(&self.scx, from, p, message_storage_available, message_expiry_interval)
                    .await?;
            }
        }

        Ok(())
    }

    #[inline]
    async fn clean_session(&self, d: Option<&Disconnect>) -> bool {
        self.connect_info()
            .await
            .map(|c| {
                if let ConnectInfo::V3(_, c) = c.as_ref() {
                    c.clean_session
                } else {
                    self.fitter.session_expiry_interval(d).is_zero()
                }
            })
            .unwrap_or(true)
    }

    #[inline]
    fn packet_id(packet_id: Option<NonZeroU16>) -> std::result::Result<NonZeroU16, Reason> {
        packet_id.ok_or_else(|| Reason::ProtocolError(ByteString::from_static("packet_id is None")))
    }

    #[inline]
    async fn process_publish<Io>(
        &mut self,
        sink: &mut Sink<Io>,
        publish: Publish,
    ) -> std::result::Result<(), Reason>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        let packet_id = publish.packet_id;
        let qos = publish.qos;

        match qos {
            QoS::AtLeastOnce => {
                let packet_id = Self::packet_id(packet_id)?;
                let inflight_res = match self.in_inflight.add(packet_id, qos) {
                    Err(e) => {
                        //hook, Message dropped
                        self.scx
                            .extends
                            .hook_mgr()
                            .message_dropped(None, From::from_custom(self.id.clone()), publish, e.clone())
                            .await;
                        return Err(e);
                    }
                    Ok(res) => res,
                };
                self.publish(publish).await?;
                sink.send_publish_ack(packet_id).await?;
                if inflight_res {
                    self.in_inflight.remove(&packet_id);
                }
            }
            QoS::ExactlyOnce => {
                let packet_id = Self::packet_id(packet_id)?;
                if let Err(e) = self.in_inflight.add(packet_id, qos) {
                    //hook, Message dropped
                    self.scx
                        .extends
                        .hook_mgr()
                        .message_dropped(None, From::from_custom(self.id.clone()), publish, e.clone())
                        .await;
                    return Err(e);
                }
                self.publish(publish).await?;
                sink.send_publish_received(packet_id).await?;
            }
            QoS::AtMostOnce => {
                self.publish(publish).await?;
            }
        }
        Ok(())
    }

    #[inline]
    async fn publish(&self, publish: Publish) -> std::result::Result<bool, Reason> {
        match self._publish(publish).await {
            Err(e) => {
                self.scx.metrics.client_publish_error_inc();
                Err(Reason::PublishFailed(e.to_string().into()))
            }
            Ok(false) => {
                self.scx.metrics.client_publish_error_inc();
                Ok(false)
            }
            Ok(true) => Ok(true),
        }
    }

    #[inline]
    async fn _publish(&self, mut publish: Publish) -> Result<bool> {
        if let Some(client_topic_aliases) = &self.client_topic_aliases {
            publish.topic = client_topic_aliases
                .set_and_get(publish.properties.as_ref().and_then(|p| p.topic_alias), publish.topic)
                .await?;
        }

        let from = From::from_custom(self.id.clone());

        if self.listen_cfg().delayed_publish {
            publish = self.scx.extends.delayed_sender().await.parse(publish)?;
        }

        //hook, message_publish
        let publish = self.hook.message_publish(from.clone(), &publish).await.unwrap_or(publish);

        //hook, message_publish_check_acl
        let acl_result = self.hook.message_publish_check_acl(&publish).await;
        log::debug!("{:?} acl_result: {:?}", self.id, acl_result);
        if let PublishAclResult::Rejected(disconnect) = acl_result {
            self.scx.metrics.client_publish_auth_error_inc();
            //hook, Message dropped
            self.scx.extends.hook_mgr().message_dropped(None, from, publish, Reason::PublishRefused).await;
            return if disconnect {
                Err(anyhow!(
                    "Publish Refused, reason: hook::message_publish_check_acl() -> Rejected(Disconnect)",
                ))
            } else {
                Ok(false)
            };
        }

        let message_storage_available = self.scx.extends.message_mgr().await.enable();

        let message_expiry_interval =
            if message_storage_available || (publish.retain && self.scx.extends.retain().await.enable()) {
                Some(self.fitter.message_expiry_interval(&publish))
            } else {
                None
            };

        //delayed publish
        if publish.delay_interval.is_some() {
            if let Some((f, p)) = self
                .scx
                .extends
                .delayed_sender()
                .await
                .delay_publish(from, publish, message_storage_available, message_expiry_interval)
                .await?
            {
                if self.scx.mqtt_delayed_publish_immediate {
                    Self::forwards(&self.scx, f, p, message_storage_available, message_expiry_interval)
                        .await?;
                } else {
                    //hook, Message dropped
                    self.scx
                        .extends
                        .hook_mgr()
                        .message_dropped(None, f, p, Reason::DelayedPublishRefused)
                        .await;
                    return Ok(false);
                }
            }
            return Ok(true);
        }

        Self::forwards(&self.scx, from, publish, message_storage_available, message_expiry_interval).await?;

        Ok(true)
    }

    #[inline]
    async fn subscribes_v3(
        &mut self,
        topic_filters: Vec<(ByteString, QoS)>,
    ) -> Result<Vec<v3::SubscribeReturnCode>> {
        let listen_cfg = self.listen_cfg();
        let shared_subscription = self.scx.extends.shared_subscription().await.is_supported(listen_cfg);
        let limit_subscription = listen_cfg.limit_subscription;
        let mut acks = Vec::new();
        for (topic_filter, qos) in topic_filters {
            let s = Subscribe::from_v3(&topic_filter, qos, shared_subscription, limit_subscription)?;
            let sub_ret = self.subscribe(s).await?;
            if let Some(qos) = sub_ret.success() {
                acks.push(v3::SubscribeReturnCode::Success(qos))
            } else {
                acks.push(v3::SubscribeReturnCode::Failure)
            }
        }
        Ok(acks)
    }

    #[inline]
    async fn subscribes_v5(&mut self, subs: v5::Subscribe) -> Result<v5::SubscribeAck> {
        let listen_cfg = self.listen_cfg();
        let shared_subscription = self.scx.extends.shared_subscription().await.is_supported(listen_cfg);
        let limit_subscription = listen_cfg.limit_subscription;
        let sub_id = subs.id;

        let mut status: Vec<SubscribeAckReason> = Vec::new();

        for (topic_filter, opts) in &subs.topic_filters {
            let s = Subscribe::from_v5(topic_filter, opts, shared_subscription, limit_subscription, sub_id)?;
            let sub_ret = self.subscribe(s).await?;
            status.push(sub_ret.into_inner());
        }
        Ok(v5::SubscribeAck {
            status,
            packet_id: subs.packet_id,
            properties: v5::UserProperties::default(),
            reason_string: None,
        })
    }

    #[inline]
    async fn unsubscribes_v3(&mut self, topic_filters: Vec<ByteString>) -> Result<()> {
        let listen_cfg = self.listen_cfg();
        let shared_subscription = self.scx.extends.shared_subscription().await.is_supported(listen_cfg);
        let limit_subscription = listen_cfg.limit_subscription;
        for topic_filter in &topic_filters {
            let unsub = Unsubscribe::from(topic_filter, shared_subscription, limit_subscription)?;
            self.unsubscribe(unsub).await?;
        }
        Ok(())
    }

    async fn unsubscribes_v5(&mut self, unsubs: v5::Unsubscribe) -> Result<v5::UnsubscribeAck> {
        let listen_cfg = self.listen_cfg();
        let shared_subscription = self.scx.extends.shared_subscription().await.is_supported(listen_cfg);
        let limit_subscription = listen_cfg.limit_subscription;
        for topic_filter in &unsubs.topic_filters {
            let unsub = Unsubscribe::from(topic_filter, shared_subscription, limit_subscription)?;
            self.unsubscribe(unsub).await?;
        }

        let mut status = Vec::with_capacity(unsubs.topic_filters.len());
        (0..unsubs.topic_filters.len()).for_each(|_| status.push(v5::UnsubscribeAckReason::Success));

        let ack = v5::UnsubscribeAck {
            status,
            packet_id: unsubs.packet_id,
            properties: v5::UserProperties::default(),
            reason_string: None,
        };
        Ok(ack)
    }

    #[inline]
    async fn unsubscribe(&self, mut unsub: Unsubscribe) -> Result<()> {
        log::debug!("{:?} unsubscribe: {:?}", self.id, unsub);
        //hook, client_unsubscribe
        let topic_filter = self.hook.client_unsubscribe(&unsub).await;
        if let Some(topic_filter) = topic_filter {
            unsub.topic_filter = topic_filter;
            log::debug!("{:?} adjust topic_filter: {:?}", self.id, unsub.topic_filter);
        }
        let ok = self.scx.extends.shared().await.entry(self.id.clone()).unsubscribe(&unsub).await?;
        if ok {
            //hook, session_unsubscribed
            self.hook.session_unsubscribed(unsub).await;
        }
        Ok(())
    }

    #[inline]
    #[allow(clippy::type_complexity)]
    fn deliver_queue_channel<'a>(
        &mut self,
        limiter: &'a Limiter,
    ) -> (queue::Sender<(From, Publish)>, queue::Receiver<'a, (From, Publish)>) {
        let (deliver_queue_tx, deliver_queue_rx) = limiter.channel(self.deliver_queue().clone());
        //When the message queue is full, the message dropping policy is implemented
        let deliver_queue_tx = deliver_queue_tx.policy(|(_, p): &(From, Publish)| -> Policy {
            if let QoS::AtMostOnce = p.qos {
                Policy::Current
            } else {
                Policy::Early
            }
        });
        (deliver_queue_tx, deliver_queue_rx)
    }

    #[inline]
    async fn subscribe(&self, sub: Subscribe) -> Result<SubscribeReturn> {
        let ret = self._subscribe(sub).await;
        match &ret {
            Ok(sub_ret) => match sub_ret.ack_reason {
                SubscribeAckReason::NotAuthorized => {
                    self.scx.metrics.client_subscribe_auth_error_inc();
                }
                SubscribeAckReason::GrantedQos0
                | SubscribeAckReason::GrantedQos1
                | SubscribeAckReason::GrantedQos2 => {}
                _ => {
                    self.scx.metrics.client_subscribe_error_inc();
                }
            },
            Err(_) => {
                self.scx.metrics.client_subscribe_error_inc();
            }
        }
        ret
    }

    #[inline]
    async fn _subscribe(&self, mut sub: Subscribe) -> Result<SubscribeReturn> {
        let listen_cfg = self.listen_cfg();

        if listen_cfg.max_subscriptions > 0
            && (self.subscriptions().await?.len().await >= listen_cfg.max_subscriptions)
        {
            return Err(MqttError::TooManySubscriptions.into());
        }

        if listen_cfg.max_topic_levels > 0
            && Topic::from_str(&sub.topic_filter)?.len() > listen_cfg.max_topic_levels
        {
            return Err(MqttError::TooManyTopicLevels.into());
        }

        if let Some(limit) = sub.opts.limit_subs() {
            let (allow, count) = self
                .scx
                .extends
                .router()
                .await
                .relations()
                .get(&sub.topic_filter)
                .map(|rels| {
                    if rels.value().contains_key(&self.id.client_id) {
                        (true, rels.value().len() - 1)
                    } else {
                        let c = rels.value().len();
                        (c < limit, c)
                    }
                })
                .unwrap_or((true, 0));
            if !allow {
                return Err(MqttError::SubscribeLimited(format!(
                    "limited: {}, current count: {}, topic_filter: {}",
                    limit, count, sub.topic_filter
                ))
                .into());
            }
        }

        sub.opts.set_qos(sub.opts.qos().less_value(listen_cfg.max_qos_allowed));

        //hook, client_subscribe
        let topic_filter = self.hook.client_subscribe(&sub).await;
        log::debug!("{:?} topic_filter: {:?}", self.id, topic_filter);

        //adjust topic filter
        if let Some(topic_filter) = topic_filter {
            sub.topic_filter = topic_filter;
        }

        //hook, client_subscribe_check_acl
        let acl_result = self.hook.client_subscribe_check_acl(&sub).await;
        if let Some(acl_result) = acl_result {
            if let Some(qos) = acl_result.success() {
                sub.opts.set_qos(sub.opts.qos().less_value(qos))
            } else {
                return Ok(acl_result);
            }
        }

        //subscribe
        let sub_ret = self.scx.extends.shared().await.entry(self.id.clone()).subscribe(&sub).await?;

        if let Some(qos) = sub_ret.success() {
            //send retain messages
            let excludeds = if self.scx.extends.retain().await.enable() {
                //MQTT V5: Retain Handling
                let send_retain_enable = match sub.opts.retain_handling() {
                    Some(RetainHandling::AtSubscribe) => true,
                    Some(RetainHandling::AtSubscribeNew) => sub_ret.prev_opts.is_none(),
                    Some(RetainHandling::NoAtSubscribe) => false,
                    None => true, //MQTT V3
                };
                log::debug!(
                    "send_retain_enable: {}, sub_ret.prev_opts: {:?}",
                    send_retain_enable,
                    sub_ret.prev_opts
                );
                let excludeds = if send_retain_enable {
                    let retain_messages = self.scx.extends.retain().await.get(&sub.topic_filter).await?;
                    let excludeds = retain_messages
                        .iter()
                        .filter_map(|(_, r)| r.msg_id.map(|msg_id| (r.from.node_id, msg_id)))
                        .collect::<Vec<_>>();
                    self.send_retain_messages(retain_messages, qos).await?;
                    excludeds
                } else {
                    Vec::new()
                };

                log::debug!("{:?} excludeds: {:?}", self.id, excludeds);
                Some(excludeds)
            } else {
                None
            };

            if self.scx.extends.message_mgr().await.enable() {
                //Send messages before they expire
                self.send_storaged_messages(&sub.topic_filter, qos, sub.opts.shared_group(), excludeds)
                    .await?;
            }

            //hook, session_subscribed
            self.hook.session_subscribed(sub).await;
        }

        Ok(sub_ret)
    }

    #[inline]
    pub async fn transfer_session_state(
        &self,
        clear_subscriptions: bool,
        mut offline_info: OfflineInfo,
    ) -> Result<()> {
        log::info!(
                "{:?} transfer session state, form: {:?}, subscriptions: {}, inflight_messages: {}, offline_messages: {}, clear_subscriptions: {}",
                self.id,
                offline_info.id,
                offline_info.subscriptions.len(),
                offline_info.inflight_messages.len(),
                offline_info.offline_messages.len(),
                clear_subscriptions
            );
        if !clear_subscriptions && !offline_info.subscriptions.is_empty() {
            for (tf, opts) in offline_info.subscriptions.iter() {
                let id = self.id.clone();
                log::debug!(
                    "{:?} transfer_session_state, router.add ... topic_filter: {:?}, opts: {:?}",
                    id,
                    tf,
                    opts
                );
                if let Err(e) = self.scx.extends.router().await.add(tf, id, opts.clone()).await {
                    log::warn!("transfer_session_state, router.add, {:?}", e);
                }

                //Send messages before they expire
                if let Err(e) = self.send_storaged_messages(tf, opts.qos(), opts.shared_group(), None).await {
                    log::warn!("transfer_session_state, router.add, {:?}", e);
                }
            }
        }

        //Subscription transfer from previous session
        if !clear_subscriptions {
            self.subscriptions_extend(offline_info.subscriptions).await?;
        }

        //Send previous session unacked messages
        while let Some(msg) = offline_info.inflight_messages.pop() {
            if !matches!(msg.status, MomentStatus::UnComplete) {
                if let Err(e) = self.reforward(msg).await {
                    log::warn!("transfer_session_state, reforward error, {:?}", e);
                }
            }
        }

        //Send offline messages
        while let Some((from, p)) = offline_info.offline_messages.pop() {
            self.forward(from, p).await;
        }
        Ok(())
    }

    #[inline]
    async fn send_retain_messages(&self, retains: Vec<(TopicName, Retain)>, qos: QoS) -> Result<()> {
        for (topic, mut retain) in retains {
            log::debug!("{:?} topic:{:?}, retain:{:?}", self.id, topic, retain);

            retain.publish.dup = false;
            retain.publish.retain = true;
            retain.publish.qos = retain.publish.qos.less_value(qos);
            retain.publish.topic = topic;
            retain.publish.packet_id = None;
            retain.publish.create_time = Some(timestamp_millis());

            log::debug!("{:?} retain.publish: {:?}", self.id, retain.publish);

            if let Err((from, p, reason)) = self
                .scx
                .extends
                .shared()
                .await
                .entry(self.id.clone())
                .publish(retain.from, retain.publish)
                .await
            {
                self.scx.extends.hook_mgr().message_dropped(Some(self.id.clone()), from, p, reason).await;
            }
        }
        Ok(())
    }

    #[inline]
    async fn send_storaged_messages(
        &self,
        topic_filter: &str,
        qos: QoS,
        group: Option<&SharedGroup>,
        excludeds: Option<Vec<(NodeId, MsgID)>>,
    ) -> Result<()> {
        let storaged_messages =
            self.scx.extends.shared().await.message_load(&self.id.client_id, topic_filter, group).await?;
        log::debug!(
            "{:?} storaged_messages: {:?}, topic_filter: {}, group: {:?}, excludeds: {:?}",
            self.id,
            storaged_messages.len(),
            topic_filter,
            group,
            excludeds
        );
        self._send_storaged_messages(storaged_messages, qos, excludeds).await?;
        Ok(())
    }

    #[inline]
    async fn _send_storaged_messages(
        &self,
        storaged_messages: Vec<(MsgID, From, Publish)>,
        qos: QoS,
        excludeds: Option<Vec<(NodeId, MsgID)>>,
    ) -> Result<()> {
        for (msg_id, from, mut publish) in storaged_messages {
            log::debug!(
                "{:?} msg_id: {}, from:{:?}, publish:{:?}, excluded: {}",
                self.id,
                msg_id,
                from,
                publish,
                excludeds
                    .as_ref()
                    .map(|excludeds| excludeds.contains(&(from.node_id, msg_id)))
                    .unwrap_or_default()
            );
            if excludeds
                .as_ref()
                .map(|excludeds| excludeds.contains(&(from.node_id, msg_id)))
                .unwrap_or_default()
            {
                continue;
            }

            publish.dup = false;
            publish.retain = false;
            publish.qos = publish.qos.less_value(qos);
            publish.packet_id = None;

            log::debug!("{:?} persistent.publish: {:?}", self.id, publish);

            if let Err((from, p, reason)) =
                self.scx.extends.shared().await.entry(self.id.clone()).publish(from, publish).await
            {
                self.scx.extends.hook_mgr().message_dropped(Some(self.id.clone()), from, p, reason).await;
            }
        }

        Ok(())
    }

    #[inline]
    pub async fn deliver<Io>(&self, sink: &mut Sink<Io>, from: From, mut publish: Publish) -> Result<()>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        //hook, message_expiry_check
        let expiry_check_res = self.hook.message_expiry_check(from.clone(), &publish).await;
        if expiry_check_res.is_expiry() {
            self.scx
                .extends
                .hook_mgr()
                .message_dropped(Some(self.id.clone()), from, publish, Reason::MessageExpiration)
                .await;
            return Ok(());
        }

        //generate packet_id
        if matches!(publish.qos, QoS::AtLeastOnce | QoS::ExactlyOnce)
            && (!publish.dup || publish.packet_id.is_none())
        {
            publish.packet_id = NonZeroU16::new(self.inflight_win().read().await.next_id()?)
        }

        //hook, message_delivered
        let publish = self.hook.message_delivered(from.clone(), &publish).await.unwrap_or(publish);

        //send message
        sink.publish(
            publish.clone(),
            expiry_check_res.message_expiry_interval(),
            self.server_topic_aliases.as_ref(),
        )
        .await?; //@TODO ... at exception, send hook and or store message

        //cache messages to inflight window
        let moment_status = match publish.qos {
            QoS::AtLeastOnce => Some(MomentStatus::UnAck),
            QoS::ExactlyOnce => Some(MomentStatus::UnReceived),
            _ => None,
        };

        if let Some(moment_status) = moment_status {
            self.inflight_win().write().await.push_back(OutInflightMessage::new(
                moment_status,
                from,
                publish,
            ));
        }

        Ok(())
    }

    #[inline]
    pub async fn reforward(&self, mut iflt_msg: OutInflightMessage) -> Result<()> {
        match iflt_msg.status {
            MomentStatus::UnAck => {
                iflt_msg.publish.dup = true;
                self.forward(iflt_msg.from, iflt_msg.publish).await;
            }
            MomentStatus::UnReceived => {
                iflt_msg.publish.dup = true;
                self.forward(iflt_msg.from, iflt_msg.publish).await;
            }
            MomentStatus::UnComplete => {
                let expiry_check_res =
                    self.hook.message_expiry_check(iflt_msg.from.clone(), &iflt_msg.publish).await;

                if expiry_check_res.is_expiry() {
                    log::warn!(
                        "{:?} MQTT::PublishComplete is not received, from: {:?}, message: {:?}",
                        self.id,
                        iflt_msg.from,
                        iflt_msg.publish
                    );
                    return Ok(());
                }

                //rerelease
                self.tx.unbounded_send(Message::SendRerelease(iflt_msg))?;
            }
        }
        Ok(())
    }

    #[inline]
    async fn send_rerelease<Io>(
        &self,
        sink: &mut Sink<Io>,
        iflt_msg: OutInflightMessage,
    ) -> std::result::Result<(), Reason>
    where
        Io: AsyncRead + AsyncWrite + Unpin,
    {
        let packet_id = Self::packet_id(iflt_msg.publish.packet_id)?;
        let old_packet_id = self.inflight_win().write().await.push_back(OutInflightMessage::new(
            MomentStatus::UnComplete,
            iflt_msg.from,
            iflt_msg.publish,
        ));

        match sink {
            Sink::V3(s) => {
                s.send_publish_release(packet_id).await?;
            }
            Sink::V5(s) => {
                let reason_code = if old_packet_id.is_some() {
                    PublishAck2Reason::Success
                } else {
                    PublishAck2Reason::PacketIdNotFound
                };
                let ack2 = PublishAck2 {
                    packet_id,
                    reason_code,
                    properties: UserProperties::default(),
                    reason_string: None,
                };
                s.send_publish_release(ack2).await?;
            }
        };
        Ok(())
    }

    #[inline]
    pub(crate) async fn forward(&self, from: From, p: Publish) {
        let res = if let Err(e) = self.tx.unbounded_send(Message::Forward(from, p)) {
            if let Message::Forward(from, p) = e.into_inner() {
                Err((from, p, Reason::from("Send Publish message error, Tx is closed")))
            } else {
                Ok(())
            }
        } else {
            Ok(())
        };

        if let Err((from, p, reason)) = res {
            //hook, message_dropped
            self.scx.extends.hook_mgr().message_dropped(Some(self.id.clone()), from, p, reason).await;
        }
    }

    #[inline]
    pub(crate) async fn clean(&self, deliver_queue_tx: &MessageSender, reason: Reason) {
        log::debug!("{:?} clean, reason: {:?}", self.id, reason);

        //Session expired, discarding messages in deliver queue
        while let Some((from, publish)) = deliver_queue_tx.pop() {
            log::debug!("{:?} clean.dropped, from: {:?}, publish: {:?}", self.id, from, publish);
            //hook, message dropped
            self.scx
                .extends
                .hook_mgr()
                .message_dropped(Some(self.id.clone()), from, publish, reason.clone())
                .await;
        }

        //Session expired, discarding messages in the flight window
        while let Some(iflt_msg) = self.inflight_win().write().await.pop_front() {
            log::debug!(
                "{:?} clean.dropped, from: {:?}, publish: {:?}",
                self.id,
                iflt_msg.from,
                iflt_msg.publish
            );

            //hook, message dropped
            self.scx
                .extends
                .hook_mgr()
                .message_dropped(Some(self.id.clone()), iflt_msg.from, iflt_msg.publish, reason.clone())
                .await;
        }

        //hook, session terminated
        self.hook.session_terminated(reason).await;

        //clear session, and unsubscribe
        let mut entry = self.scx.extends.shared().await.entry(self.id.clone());
        if let Some(true) = entry.id_same() {
            if let Err(e) = entry.remove_with(&self.id).await {
                log::warn!("{:?} Failed to remove the session from the broker, {:?}", self.id, e);
            }
        }
    }

    // #[inline]
    // pub(crate) fn send(&self, msg: Message) -> Result<()> {
    //     self.tx.unbounded_send(msg).map_err(anyhow::Error::new)?;
    //     Ok(())
    // }

    #[inline]
    pub async fn forwards(
        scx: &ServerContext,
        from: From,
        publish: Publish,
        message_storage_available: bool,
        message_expiry_interval: Option<Duration>,
    ) -> Result<()> {
        //make message id
        let msg_id = if message_storage_available {
            Some(scx.extends.message_mgr().await.next_msg_id())
        } else {
            None
        };

        let retain = scx.extends.retain().await;
        if retain.enable() && publish.retain {
            retain
                .set(
                    &publish.topic,
                    Retain { msg_id, from: from.clone(), publish: publish.clone() },
                    message_expiry_interval,
                )
                .await?;
        }
        drop(retain);

        let stored_msg =
            if let (Some(msg_id), Some(message_expiry_interval)) = (msg_id, message_expiry_interval) {
                Some((msg_id, from.clone(), publish.clone(), message_expiry_interval))
            } else {
                None
            };

        let sub_cids = match scx.extends.shared().await.forwards(from.clone(), publish).await {
            Ok(None) => {
                //hook, message_nonsubscribed
                scx.extends.hook_mgr().message_nonsubscribed(from).await;
                None
            }
            Ok(Some(sub_cids)) => Some(sub_cids),
            Err(errs) => {
                for (to, from, p, reason) in errs {
                    //hook, Message dropped
                    scx.extends.hook_mgr().message_dropped(Some(to), from, p, reason).await;
                }
                None
            }
        };

        if let Some((msg_id, from, p, expiry_interval)) = stored_msg {
            //Store messages before they expire
            if let Err(e) =
                scx.extends.message_mgr().await.store(msg_id, from, p, expiry_interval, sub_cids).await
            {
                log::warn!("Failed to storage messages, {:?}", e);
            }
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct Session(Arc<_Session>);

impl Deref for Session {
    type Target = _Session;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

pub struct _Session {
    inner: Arc<dyn SessionLike>,
    pub id: Id,
    pub fitter: FitterType,
    pub auth_info: Option<AuthInfo>,
    pub scx: ServerContext,
    // pub extra_attrs: RwLock<ExtraAttrs>,
}

impl Deref for _Session {
    type Target = dyn SessionLike;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl Drop for _Session {
    fn drop(&mut self) {
        self.scx.stats.sessions.dec();
        let id = self.id.clone();
        let s = self.inner.clone();
        tokio::spawn(async move {
            if let Err(e) = s.on_drop().await {
                log::error!("{:?} session clear error, {:?}", id, e);
            }
        });
    }
}

impl fmt::Debug for Session {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session {:?}", self.id)
    }
}

impl Session {
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        id: Id,
        scx: ServerContext,
        max_mqueue_len: usize,
        listen_cfg: ListenerConfig,
        fitter: FitterType,
        auth_info: Option<AuthInfo>,
        max_inflight: NonZeroU16,
        created_at: TimestampMillis,

        conn_info: ConnectInfoType,
        session_present: bool,
        superuser: bool,
        connected: bool,
        connected_at: TimestampMillis,

        subscriptions: SessionSubs,
        disconnect_info: Option<DisconnectInfo>,

        last_id: Option<Id>,
    ) -> Result<Self> {
        let max_inflight = max_inflight.get() as usize;
        let message_retry_interval = listen_cfg.message_retry_interval.as_millis() as TimestampMillis;
        let message_expiry_interval = listen_cfg.message_expiry_interval.as_millis() as TimestampMillis;
        let mut deliver_queue = MessageQueue::new(max_mqueue_len);

        let scx1 = scx.clone();
        deliver_queue.on_push(move || {
            scx1.stats.message_queues.inc();
        });

        let scx1 = scx.clone();
        deliver_queue.on_pop(move || {
            scx1.stats.message_queues.dec();
        });

        let scx1 = scx.clone();
        let scx2 = scx.clone();
        let out_inflight = OutInflight::new(max_inflight, message_retry_interval, message_expiry_interval)
            .on_push(move || {
                scx1.stats.out_inflights.inc();
            })
            .on_pop(move || {
                scx2.stats.out_inflights.dec();
            });

        scx.stats.sessions.inc();
        scx.stats.subscriptions.incs(subscriptions.len().await as isize);
        scx.stats.subscriptions_shared.incs(subscriptions.shared_len().await as isize);

        // let extra_attrs = RwLock::new(ExtraAttrs::new());
        let session_like = scx
            .extends
            .session_mgr()
            .await
            .create(
                id.clone(),
                scx.clone(),
                listen_cfg,
                fitter.clone(),
                subscriptions,
                Arc::new(deliver_queue),
                Arc::new(RwLock::new(out_inflight)),
                conn_info,
                created_at,
                connected_at,
                session_present,
                superuser,
                connected,
                disconnect_info,
                last_id,
            )
            .await?;
        Ok(Self(Arc::new(_Session { inner: session_like, id, fitter, auth_info, scx })))
    }

    #[inline]
    pub async fn to_offline_info(&self) -> Result<OfflineInfo> {
        let id = self.id.clone();
        let created_at = self.created_at().await?;
        let subscriptions = self.subscriptions_drain().await?;

        let mut offline_messages = Vec::new();
        while let Some(item) = self.deliver_queue().pop() {
            //@TODO ..., check message expired
            offline_messages.push(item);
        }
        let inflight_messages = self.inflight_win().write().await.to_inflight_messages();

        Ok(OfflineInfo { id, subscriptions, offline_messages, inflight_messages, created_at })
    }

    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let (count, subs) = if let Ok(subs) = self.subscriptions().await {
            let count = subs.len().await;
            let subs = subs
                .read()
                .await
                .iter()
                .enumerate()
                .filter_map(|(i, (tf, opts))| {
                    if i < 100 {
                        Some(json!({
                            "topic_filter": tf.to_string(),
                            "opts": opts.to_json(),
                        }))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            (count, subs)
        } else {
            (0, Vec::new())
        };

        let data = json!({
            "subscriptions": {
                "count": count,
                "topic_filters": subs,
            },
            "queues": self.deliver_queue().len(),
            "inflights": self.inflight_win().read().await.len(),
            "created_at": self.created_at().await.unwrap_or_default(),
        });
        data
    }
}

#[async_trait]
pub trait SessionManager: Sync + Send {
    #[allow(clippy::too_many_arguments)]
    async fn create(
        &self,
        id: Id,
        scx: ServerContext,
        listen_cfg: ListenerConfig,
        fitter: FitterType,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: OutInflightType,
        conn_info: ConnectInfoType,
        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,

        last_id: Option<Id>,
    ) -> Result<Arc<dyn SessionLike>>;
}

#[async_trait]
pub trait SessionLike: Sync + Send + 'static {
    fn id(&self) -> &Id;
    fn context(&self) -> &ServerContext;
    fn listen_cfg(&self) -> &ListenerConfig;
    fn deliver_queue(&self) -> &MessageQueueType;
    fn inflight_win(&self) -> &OutInflightType;

    async fn subscriptions(&self) -> Result<SessionSubs>;
    async fn subscriptions_add(
        &self,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Result<Option<SubscriptionOptions>>;
    async fn subscriptions_remove(
        &self,
        topic_filter: &str,
    ) -> Result<Option<(TopicFilter, SubscriptionOptions)>>;
    async fn subscriptions_drain(&self) -> Result<Subscriptions>;
    async fn subscriptions_extend(&self, other: Subscriptions) -> Result<()>;

    async fn created_at(&self) -> Result<TimestampMillis>;
    async fn session_present(&self) -> Result<bool>;
    async fn connect_info(&self) -> Result<Arc<ConnectInfo>>;
    fn username(&self) -> Option<&UserName>;
    fn password(&self) -> Option<&Password>;
    async fn protocol(&self) -> Result<u8>;
    async fn superuser(&self) -> Result<bool>;
    async fn connected(&self) -> Result<bool>;
    async fn connected_at(&self) -> Result<TimestampMillis>;

    async fn disconnected_at(&self) -> Result<TimestampMillis>;
    async fn disconnected_reasons(&self) -> Result<Vec<Reason>>;
    async fn disconnected_reason(&self) -> Result<Reason>;
    async fn disconnected_reason_has(&self) -> bool;
    async fn disconnected_reason_add(&self, r: Reason) -> Result<()>;
    async fn disconnected_reason_take(&self) -> Result<Reason>;
    async fn disconnect(&self) -> Result<Option<Disconnect>>;
    async fn disconnected_set(&self, d: Option<Disconnect>, reason: Option<Reason>) -> Result<()>;

    #[inline]
    async fn on_drop(&self) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn keepalive(&self, _ping: IsPing) {}
}

#[derive(Serialize, Deserialize, Clone)]
pub struct OfflineInfo {
    pub id: Id,
    pub subscriptions: Subscriptions,
    pub offline_messages: Vec<(From, Publish)>,
    pub inflight_messages: Vec<OutInflightMessage>,
    pub created_at: TimestampMillis,
}

impl std::fmt::Debug for OfflineInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "subscriptions: {}, offline_messages: {}, inflight_messages: {}, created_at: {}",
            self.subscriptions.len(),
            self.offline_messages.len(),
            self.inflight_messages.len(),
            self.created_at
        )
    }
}

pub struct DefaultSessionManager;

#[async_trait]
impl SessionManager for DefaultSessionManager {
    #[allow(clippy::too_many_arguments)]
    async fn create(
        &self,
        id: Id,
        scx: ServerContext,
        listen_cfg: ListenerConfig,
        _fitter: FitterType,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: OutInflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,

        _last_id: Option<Id>,
    ) -> Result<Arc<dyn SessionLike>> {
        let s = DefaultSession::new(
            id,
            scx,
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
        );
        Ok(Arc::new(s))
    }
}

pub struct DefaultSession {
    id: Id,
    scx: ServerContext,
    listen_cfg: ListenerConfig,
    pub subscriptions: SessionSubs,
    deliver_queue: MessageQueueType,
    inflight_win: OutInflightType,
    conn_info: ConnectInfoType,

    created_at: TimestampMillis,
    state_flags: SessionStateFlags,
    connected_at: TimestampMillis,

    pub disconnect_info: RwLock<DisconnectInfo>,
}

impl DefaultSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: Id,
        scx: ServerContext,
        listen_cfg: ListenerConfig,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: OutInflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,
    ) -> Self {
        let mut state_flags = SessionStateFlags::empty();
        if session_present {
            state_flags.insert(SessionStateFlags::SessionPresent);
        }
        if superuser {
            state_flags.insert(SessionStateFlags::Superuser);
        }
        if connected {
            state_flags.insert(SessionStateFlags::Connected);
        }
        let disconnect_info = disconnect_info.unwrap_or_default();

        Self {
            id,
            scx,
            listen_cfg,
            subscriptions,
            deliver_queue,
            inflight_win,
            conn_info,

            created_at,
            state_flags,
            connected_at,

            disconnect_info: RwLock::new(disconnect_info),
        }
    }
}

#[async_trait]
impl SessionLike for DefaultSession {
    fn id(&self) -> &Id {
        &self.id
    }

    #[inline]
    fn context(&self) -> &ServerContext {
        &self.scx
    }

    #[inline]
    fn listen_cfg(&self) -> &ListenerConfig {
        &self.listen_cfg
    }

    #[inline]
    fn deliver_queue(&self) -> &MessageQueueType {
        &self.deliver_queue
    }

    #[inline]
    fn inflight_win(&self) -> &OutInflightType {
        &self.inflight_win
    }

    #[inline]
    async fn subscriptions(&self) -> Result<SessionSubs> {
        Ok(self.subscriptions.clone())
    }

    #[inline]
    async fn subscriptions_add(
        &self,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Result<Option<SubscriptionOptions>> {
        Ok(self.subscriptions._add(&self.scx, topic_filter, opts).await)
    }

    #[inline]
    async fn subscriptions_remove(
        &self,
        topic_filter: &str,
    ) -> Result<Option<(TopicFilter, SubscriptionOptions)>> {
        Ok(self.subscriptions._remove(&self.scx, topic_filter).await)
    }

    #[inline]
    async fn subscriptions_drain(&self) -> Result<Subscriptions> {
        Ok(self.subscriptions._drain(&self.scx).await)
    }

    #[inline]
    async fn subscriptions_extend(&self, other: Subscriptions) -> Result<()> {
        self.subscriptions._extend(&self.scx, other).await;
        Ok(())
    }

    #[inline]
    async fn created_at(&self) -> Result<TimestampMillis> {
        Ok(self.created_at)
    }

    #[inline]
    async fn session_present(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SessionStateFlags::SessionPresent))
    }

    async fn connect_info(&self) -> Result<Arc<ConnectInfo>> {
        Ok(self.conn_info.clone())
    }
    fn username(&self) -> Option<&UserName> {
        self.id.username.as_ref()
    }
    fn password(&self) -> Option<&Password> {
        self.conn_info.password()
    }
    async fn protocol(&self) -> Result<u8> {
        Ok(self.conn_info.proto_ver())
    }
    async fn superuser(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SessionStateFlags::Superuser))
    }
    async fn connected(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SessionStateFlags::Connected)
            && !self.disconnect_info.read().await.is_disconnected())
    }
    async fn connected_at(&self) -> Result<TimestampMillis> {
        Ok(self.connected_at)
    }
    async fn disconnected_at(&self) -> Result<TimestampMillis> {
        Ok(self.disconnect_info.read().await.disconnected_at)
    }
    async fn disconnected_reasons(&self) -> Result<Vec<Reason>> {
        Ok(self.disconnect_info.read().await.reasons.clone())
    }
    async fn disconnected_reason(&self) -> Result<Reason> {
        Ok(Reason::Reasons(self.disconnect_info.read().await.reasons.clone()))
    }
    async fn disconnected_reason_has(&self) -> bool {
        !self.disconnect_info.read().await.reasons.is_empty()
    }
    async fn disconnected_reason_add(&self, r: Reason) -> Result<()> {
        self.disconnect_info.write().await.reasons.push(r);
        Ok(())
    }
    async fn disconnected_reason_take(&self) -> Result<Reason> {
        Ok(Reason::Reasons(self.disconnect_info.write().await.reasons.drain(..).collect()))
    }
    async fn disconnect(&self) -> Result<Option<Disconnect>> {
        Ok(self.disconnect_info.read().await.mqtt_disconnect.clone())
    }
    async fn disconnected_set(&self, d: Option<Disconnect>, reason: Option<Reason>) -> Result<()> {
        let mut disconnect_info = self.disconnect_info.write().await;

        if !disconnect_info.is_disconnected() {
            disconnect_info.disconnected_at = timestamp_millis();
        }

        if let Some(d) = d {
            disconnect_info.reasons.push(Reason::ConnectDisconnect(Some(d.clone())));
            disconnect_info.mqtt_disconnect.replace(d);
        }

        if let Some(reason) = reason {
            disconnect_info.reasons.push(reason);
        }

        Ok(())
    }

    #[inline]
    async fn on_drop(&self) -> Result<()> {
        self.subscriptions._clear(&self.scx).await;
        Ok(())
    }
}
