use std::convert::AsRef;
use std::convert::From as _f;
use std::convert::TryFrom;
use std::fmt;
use std::num::NonZeroU16;
use std::ops::Deref;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use bytestring::ByteString;
use futures::StreamExt;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

use ntex_mqtt::v5::codec::RetainHandling;

use crate::broker::hook::Hook;
use crate::broker::inflight::{Inflight, InflightMessage, MomentStatus};
use crate::broker::queue::{self, Limiter, Policy};
use crate::broker::types::*;
use crate::metrics::Metrics;
use crate::settings::listener::Listener;
use crate::{MqttError, Result, Runtime};

#[derive(Clone)]
pub struct SessionState {
    pub tx: Option<Tx>,
    pub session: Session,
    pub sink: Option<Sink>,
    pub hook: Rc<dyn Hook>,
    pub deliver_queue_tx: Option<MessageSender>,
    pub server_topic_aliases: Option<Rc<ServerTopicAliases>>,
    pub client_topic_aliases: Option<Rc<ClientTopicAliases>>,
}

impl fmt::Debug for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SessionState {{ {:?}, {:?}, {} }}",
            self.id,
            self.session,
            self.deliver_queue_tx.as_ref().map(|tx| tx.len()).unwrap_or_default()
        )
    }
}

impl SessionState {
    #[inline]
    pub(crate) fn new(
        session: Session,
        sink: Sink,
        hook: Rc<dyn Hook>,
        server_topic_alias_max: u16,
        client_topic_alias_max: u16,
    ) -> Self {
        let server_topic_aliases = if server_topic_alias_max > 0 {
            Some(Rc::new(ServerTopicAliases::new(server_topic_alias_max as usize)))
        } else {
            None
        };
        let client_topic_aliases = if client_topic_alias_max > 0 {
            Some(Rc::new(ClientTopicAliases::new(client_topic_alias_max as usize)))
        } else {
            None
        };
        log::debug!("server_topic_aliases: {:?}", server_topic_aliases);
        log::debug!("client_topic_aliases: {:?}", client_topic_aliases);
        Self {
            tx: None,
            session,
            sink: Some(sink),
            hook,
            deliver_queue_tx: None,
            server_topic_aliases,
            client_topic_aliases,
        }
    }

    #[inline]
    pub(crate) async fn start(mut self, keep_alive: u16) -> (Self, Tx) {
        log::debug!("{:?} start online event loop", self.id);
        let (msg_tx, mut msg_rx) = futures::channel::mpsc::unbounded();
        let msg_tx = SessionTx::new(msg_tx);
        self.tx.replace(msg_tx.clone());
        let state = self.clone();

        let keep_alive_interval = if keep_alive == 0 {
            Duration::from_secs(u32::MAX as u64)
        } else {
            Duration::from_secs(keep_alive as u64)
        };
        log::debug!("{:?} keep_alive_interval is {:?}", state.id, keep_alive_interval);
        let keep_alive_delay = tokio::time::sleep(keep_alive_interval);

        let deliver_timeout_delay = tokio::time::sleep(Duration::from_secs(60));

        let limiter = {
            let (burst, replenish_n_per) = state.fitter.mqueue_rate_limit();
            Limiter::new(burst, replenish_n_per)
        };

        let mut flags = StateFlags::empty();

        log::debug!("{:?} there are {} offline messages ...", state.id, state.deliver_queue().len());

        ntex::rt::spawn(async move {
            Runtime::instance().stats.connections.inc();

            let (state, deliver_queue_tx, mut deliver_queue_rx) = state.deliver_queue_channel(&limiter);

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
                    _ = &mut keep_alive_delay => {  //, if !keep_alive_delay.is_elapsed()
                        log::debug!("{:?} keep alive is timeout, is_elapsed: {:?}", state.id, keep_alive_delay.is_elapsed());
                        let _ = state.disconnected_reason_add(Reason::ConnectKeepaliveTimeout).await;
                        break
                    },

                    msg = msg_rx.next() => {
                        log::debug!("{:?} recv msg: {:?}", state.id, msg);
                        if let Some(msg) = msg{
                            #[cfg(feature = "debug")]
                            Runtime::instance().stats.debug_session_channels.dec();
                            match msg{
                                Message::Forward(from, p) => {
                                    if let Err((from, p)) = deliver_queue_tx.send((from, p)).await{
                                        log::warn!("{:?} deliver_dropped, from: {:?}, {:?}", state.id, from, p);
                                        //hook, message_dropped
                                        Runtime::instance().extends.hook_mgr().await.message_dropped(Some(state.id.clone()), from, p, Reason::MessageQueueFull).await;
                                    }
                                },
                                Message::Kick(sender, by_id, clean_start, is_admin) => {
                                    log::debug!("{:?} Message::Kick, send kick result, to {:?}, clean_start: {}, is_admin: {}", state.id, by_id, clean_start, is_admin);
                                    if !sender.is_closed() {
                                        if sender.send(()).is_err() {
                                            log::warn!("{:?} Message::Kick, send response error, sender is closed", state.id);
                                        }
                                        flags.insert(StateFlags::Kicked);
                                        if is_admin {
                                            flags.insert(StateFlags::ByAdminKick);
                                        }
                                        if clean_start {
                                            flags.insert(StateFlags::CleanStart);
                                        }
                                        let _ = state.disconnected_reason_add(Reason::ConnectKicked(is_admin)).await;
                                        break
                                    }else{
                                        log::warn!("{:?} Message::Kick, kick sender is closed, to {:?}, is_admin: {}", state.id, by_id, is_admin);
                                    }
                                },
                                Message::Disconnect(d) => {
                                    flags.insert(StateFlags::DisconnectReceived);
                                    //state.set_mqtt_disconnect(d).await;
                                    let _ = state.disconnected_set(Some(d), None).await;
                                },
                                Message::Closed(reason) => {
                                    log::debug!("{:?} Closed({}) message received, reason: {}", state.id, flags.contains(StateFlags::DisconnectReceived), reason);
                                    if !state.disconnected_reason_has().await{
                                        let _ = state.disconnected_reason_add(reason).await;
                                    }
                                    break
                                },
                                Message::Keepalive(ping) => {
                                    log::debug!("{:?} Message::Keepalive ... ", state.id);
                                    keep_alive_delay.as_mut().reset(Instant::now() + keep_alive_interval);
                                    if ping {
                                        flags.insert(StateFlags::Ping);
                                    }
                                },
                                Message::Subscribe(sub, reply_tx) => {
                                    let sub_reply = state.subscribe(sub).await;
                                    if !reply_tx.is_closed(){
                                        if let Err(e) = reply_tx.send(sub_reply) {
                                            log::warn!("{:?} Message::Subscribe, send response error, {:?}", state.id, e);
                                        }
                                    }else{
                                        log::warn!("{:?} Message::Subscribe, reply sender is closed", state.id);
                                    }
                                },
                                Message::Unsubscribe(unsub, reply_tx) => {
                                    let unsub_reply = state.unsubscribe(unsub).await;
                                    if !reply_tx.is_closed(){
                                        if let Err(e) = reply_tx.send(unsub_reply) {
                                            log::warn!("{:?} Message::Unsubscribe, send response error, {:?}", state.id, e);
                                        }
                                    }else{
                                        log::warn!("{:?} Message::Unsubscribe, reply sender is closed", state.id);
                                    }
                                }
                            }
                        }else{
                            log::warn!("{:?} None is received from the Rx", state.id);
                            let _ = state.disconnected_reason_add(Reason::from_static("None is received from the Rx")).await;
                            break;
                        }
                    },

                    _ = &mut deliver_timeout_delay => {
                        while let Some(iflt_msg) = state.inflight_win().write().await.pop_front_timeout(){
                            log::debug!("{:?} has timeout message in inflight: {:?}", state.id, iflt_msg);
                            if let Err(e) = state.reforward(iflt_msg).await{
                                log::error!("{:?} redeliver message error, {:?}", state.id, e);
                            }
                        }
                    },

                    deliver_packet = deliver_queue_rx.next(), if state.inflight_win().read().await.has_credit() => {
                        log::debug!("{:?} deliver_packet: {:?}", state.id, deliver_packet);
                        match deliver_packet{
                            Some(Some((from, p))) => {
                                if let Err(e) = state.deliver(from, p).await{
                                    log::error!("{:?} deliver message error, {:?}", state.id, e);
                                }
                            },
                            Some(None) => {
                                log::warn!("{:?} None is received from the deliver Queue", state.id);
                            },
                            None => {
                                log::warn!("{:?} Deliver Queue is closed", state.id);
                                let _ = state.disconnected_reason_add("Deliver Queue is closed".into()).await;
                                break;
                            }
                        }
                    }
                }

                let is_ping = flags.contains(StateFlags::Ping);
                state.keepalive(is_ping).await;
                if is_ping {
                    flags.remove(StateFlags::Ping);
                }
            }

            let disconnect = state.disconnect().await.unwrap_or(None);
            let clean_session = state.clean_session(disconnect.as_ref()).await;

            log::debug!(
                "{:?} exit online worker, flags: {:?}, clean_session: {} {}",
                state.id,
                flags,
                clean_session,
                flags.contains(StateFlags::CleanStart)
            );

            Runtime::instance().stats.connections.dec();

            //Setting the disconnected state
            let _ = state.disconnected_set(None, None).await;

            //Last will message
            let will_delay_interval = if state.last_will_enable(flags, clean_session) {
                let will_delay_interval = state.will_delay_interval().await;
                if clean_session || will_delay_interval.is_none() {
                    if let Err(e) = state.process_last_will().await {
                        log::error!("{:?} process last will error, {:?}", state.id, e);
                    }
                    None
                } else {
                    will_delay_interval
                }
            } else {
                None
            };

            if let Some(sink) = state.sink.as_ref() {
                sink.close()
            }

            //hook, client_disconnected
            let reason = if state.disconnected_reason_has().await {
                state.disconnected_reason().await.unwrap_or_default()
            } else {
                let _ = state.disconnected_reason_add(Reason::ConnectRemoteClose).await;
                Reason::ConnectRemoteClose
            };
            state.hook.client_disconnected(reason).await;

            if flags.contains(StateFlags::Kicked) {
                if flags.contains(StateFlags::ByAdminKick) {
                    state.clean(state.disconnected_reason_take().await.unwrap_or_default()).await;
                }
            } else if clean_session {
                state.clean(state.disconnected_reason_take().await.unwrap_or_default()).await;
            } else {
                let session_expiry_interval = state.fitter.session_expiry_interval(disconnect.as_ref()).await;
                //Start offline event loop
                Self::offline_start(
                    state.clone(),
                    &mut msg_rx,
                    &deliver_queue_tx,
                    &mut flags,
                    will_delay_interval,
                    session_expiry_interval,
                )
                .await;
                log::debug!("{:?} offline flags: {:?}", state.id, flags);
                if !flags.contains(StateFlags::Kicked) {
                    state.clean(Reason::SessionExpiration).await;
                }
            }
        });
        (self, msg_tx)
    }

    #[inline]
    async fn offline_start(
        state: SessionState,
        msg_rx: &mut Rx,
        deliver_queue_tx: &MessageSender,
        flags: &mut StateFlags,
        mut will_delay_interval: Option<Duration>,
        //disconnect: Option<&Disconnect>,
        session_expiry_interval: Duration,
    ) {
        //let session_expiry_interval = state.fitter.session_expiry_interval(disconnect).await;
        log::debug!(
            "{:?} start offline event loop, session_expiry_interval: {:?}, will_delay_interval: {:?}",
            state.id,
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
                msg = msg_rx.next() => {
                    log::debug!("{:?} recv offline msg: {:?}", state.id, msg);
                    if let Some(msg) = msg{
                        match msg{
                            Message::Forward(from, p) => {
                                if let Err((from, p)) = deliver_queue_tx.send((from, p)).await{
                                    log::warn!("{:?} offline deliver_dropped, from: {:?}, {:?}", state.id, from, p);
                                    //hook, message_dropped
                                    Runtime::instance().extends.hook_mgr().await.message_dropped(Some(state.id.clone()), from, p, Reason::MessageQueueFull).await;
                                }
                            },
                            Message::Kick(sender, by_id, clean_start, is_admin) => {
                                log::debug!("{:?} offline Kicked, send kick result, to: {:?}, clean_start: {}, is_admin: {}", state.id, by_id, clean_start, is_admin);
                                if !sender.is_closed() {
                                    if let Err(e) = sender.send(()) {
                                        log::warn!("{:?} offline Kick send response error, to: {:?}, clean_start: {}, is_admin: {}, {:?}", state.id, by_id, clean_start, is_admin, e);
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
                                    log::warn!("{:?} offline Kick sender is closed, to {:?}, clean_start: {}, is_admin: {}", state.id, by_id, clean_start, is_admin);
                                }
                            },
                            _ => {
                                log::debug!("{:?} offline receive message is {:?}", state.id, msg);
                            }
                        }
                    }else{
                        log::warn!("{:?} offline None is received from the Rx", state.id);
                        break;
                    }
                },
               _ = &mut session_expiry_delay => { //, if !session_expiry_delay.is_elapsed() => {
                  log::debug!("{:?} session expired, will_delay_interval: {:?}", state.id, will_delay_interval);
                  if will_delay_interval.is_some() {
                      if let Err(e) = state.process_last_will().await {
                          log::error!("{:?} process last will error, {:?}", state.id, e);
                      }
                  }
                  break
               },
               _ = &mut will_delay_interval_delay => { //, if !will_delay_interval_delay.is_elapsed() => {
                  log::debug!("{:?} will delay interval, will_delay_interval: {:?}", state.id, will_delay_interval);
                  if will_delay_interval.is_some() {
                      if let Err(e) = state.process_last_will().await {
                          log::error!("{:?} process last will error, {:?}", state.id, e);
                      }
                      will_delay_interval = None;
                  }
                  will_delay_interval_delay.as_mut().reset(
                    Instant::now() + session_expiry_interval,
                  );
               },
            }
        }
        log::debug!("{:?} exit offline worker", state.id);
    }

    #[inline]
    pub async fn offline_restart(session: Session, session_expiry_interval: Duration) -> (SessionState, Tx) {
        let hook = Runtime::instance().extends.hook_mgr().await.hook(&session);

        let (msg_tx, mut msg_rx) = futures::channel::mpsc::unbounded();
        let msg_tx = SessionTx::new(msg_tx);

        let state = SessionState {
            tx: Some(msg_tx.clone()),
            session,
            sink: None,
            hook,
            deliver_queue_tx: None,
            server_topic_aliases: None,
            client_topic_aliases: None,
        };

        let limiter = {
            let (burst, replenish_n_per) = state.fitter.mqueue_rate_limit();
            Limiter::new(burst, replenish_n_per)
        };

        let state1 = state.clone();
        ntex::rt::spawn(async move {
            let (state, deliver_queue_tx, _deliver_queue_rx) = state.deliver_queue_channel(&limiter);
            let mut flags = StateFlags::empty();

            let disconnect = state.disconnect().await.unwrap_or(None);
            let clean_session = state.clean_session(disconnect.as_ref()).await;

            //Last will message
            let will_delay_interval = if state.last_will_enable(flags, clean_session) {
                let will_delay_interval = state.will_delay_interval().await;
                if clean_session || will_delay_interval.is_none() {
                    if let Err(e) = state.process_last_will().await {
                        log::error!("{:?} process last will error, {:?}", state.id, e);
                    }
                    None
                } else {
                    will_delay_interval
                }
            } else {
                None
            };

            Self::offline_start(
                state.clone(),
                &mut msg_rx,
                &deliver_queue_tx,
                &mut flags,
                will_delay_interval,
                session_expiry_interval,
            )
            .await;

            if !flags.contains(StateFlags::Kicked) {
                state.clean(Reason::SessionExpiration).await;
            }
        });

        (state1, msg_tx)
    }

    #[inline]
    #[allow(clippy::type_complexity)]
    fn deliver_queue_channel(
        mut self,
        limiter: &Limiter,
    ) -> (Self, queue::Sender<(From, Publish)>, queue::Receiver<'_, (From, Publish)>) {
        let (deliver_queue_tx, deliver_queue_rx) = limiter.channel(self.deliver_queue().clone());
        //When the message queue is full, the message dropping policy is implemented
        let deliver_queue_tx = deliver_queue_tx.policy(|(_, p): &(From, Publish)| -> Policy {
            if let QoS::AtMostOnce = p.qos() {
                Policy::Current
            } else {
                Policy::Early
            }
        });
        self.deliver_queue_tx.replace(deliver_queue_tx.clone());
        (self, deliver_queue_tx, deliver_queue_rx)
    }

    #[inline]
    pub(crate) async fn forward(&self, from: From, p: Publish) {
        let res = if let Some(ref tx) = self.tx {
            if let Err(e) = tx.unbounded_send(Message::Forward(from, p)) {
                if let Message::Forward(from, p) = e.into_inner() {
                    Err((from, p, Reason::from("Send Publish message error, Tx is closed")))
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            }
        } else {
            log::warn!("{:?} Message Sender is None", self.id);
            Err((from, p, Reason::from("Send Publish message error, Tx is None")))
        };

        if let Err((from, p, reason)) = res {
            //hook, message_dropped
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(Some(self.id.clone()), from, p, reason)
                .await;
        }
    }

    #[inline]
    pub(crate) fn send(&self, msg: Message) -> Result<()> {
        if let Some(ref tx) = self.tx {
            tx.unbounded_send(msg).map_err(anyhow::Error::new)?;
            Ok(())
        } else {
            Err(MqttError::from("Message Sender is None"))
        }
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
        if let Some(lw) = self.connect_info().await?.last_will() {
            let p = Publish::try_from(lw)?;
            let from = From::from_lastwill(self.id.clone());
            //hook, message_publish
            let p = self.hook.message_publish(from.clone(), &p).await.unwrap_or(p);

            match Runtime::instance().extends.shared().await.forwards(from.clone(), p).await {
                Ok(0) => {
                    //hook, message_nonsubscribed
                    Runtime::instance().extends.hook_mgr().await.message_nonsubscribed(from).await;
                }
                Ok(_) => {}
                Err(droppeds) => {
                    for (to, from, p, r) in droppeds {
                        //hook, message_dropped
                        Runtime::instance()
                            .extends
                            .hook_mgr()
                            .await
                            .message_dropped(Some(to), from, p, r)
                            .await;
                    }
                }
            }
        }
        Ok(())
    }

    #[inline]
    pub async fn send_retain_messages(&self, retains: Vec<(TopicName, Retain)>, qos: QoS) -> Result<()> {
        for (topic, mut retain) in retains {
            log::debug!("{:?} topic:{:?}, retain:{:?}", self.id, topic, retain);

            retain.publish.dup = false;
            retain.publish.retain = true;
            retain.publish.qos = retain.publish.qos.less_value(qos);
            retain.publish.topic = topic;
            retain.publish.packet_id = None;
            retain.publish.create_time = chrono::Local::now().timestamp_millis();

            log::debug!("{:?} retain.publish: {:?}", self.id, retain.publish);

            if let Err((from, p, reason)) = Runtime::instance()
                .extends
                .shared()
                .await
                .entry(self.id.clone())
                .publish(retain.from, retain.publish)
                .await
            {
                Runtime::instance()
                    .extends
                    .hook_mgr()
                    .await
                    .message_dropped(Some(self.id.clone()), from, p, reason)
                    .await;
            }
        }
        Ok(())
    }

    #[inline]
    pub async fn deliver(&self, from: From, mut publish: Publish) -> Result<()> {
        let sink = if let Some(sink) = self.sink.as_ref() {
            sink
        } else {
            let err = MqttError::from("MQTT connection lost, unreachable!()");
            log::warn!("{:?} {:?}, from: {:?}, message: {:?}", self.id, err, from, publish);
            return Err(err);
        };

        //hook, message_expiry_check
        let expiry_check_res = self.hook.message_expiry_check(from.clone(), &publish).await;
        if expiry_check_res.is_expiry() {
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(Some(self.id.clone()), from, publish, Reason::MessageExpiration)
                .await;
            return Ok(());
        }

        //generate packet_id
        if matches!(publish.qos(), QoS::AtLeastOnce | QoS::ExactlyOnce)
            && (!publish.dup() || publish.packet_id_is_none())
        {
            publish.set_packet_id(self.inflight_win().read().await.next_id()?);
        }

        //hook, message_delivered
        let publish = self.hook.message_delivered(from.clone(), &publish).await.unwrap_or(publish);

        //send message
        sink.publish(
            &publish,
            expiry_check_res.message_expiry_interval(),
            self.server_topic_aliases.as_ref(),
        )
        .await?; //@TODO ... at exception, send hook and or store message

        //cache messages to inflight window
        let moment_status = match publish.qos() {
            QoS::AtLeastOnce => Some(MomentStatus::UnAck),
            QoS::ExactlyOnce => Some(MomentStatus::UnReceived),
            _ => None,
        };
        if let Some(moment_status) = moment_status {
            self.inflight_win().write().await.push_back(InflightMessage::new(moment_status, from, publish));
        }

        Ok(())
    }

    #[inline]
    pub async fn reforward(&self, mut iflt_msg: InflightMessage) -> Result<()> {
        match iflt_msg.status {
            MomentStatus::UnAck => {
                iflt_msg.publish.set_dup(true);
                self.forward(iflt_msg.from, iflt_msg.publish).await;
            }
            MomentStatus::UnReceived => {
                iflt_msg.publish.set_dup(true);
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

                let sink = if let Some(sink) = self.sink.as_ref() {
                    sink
                } else {
                    let err = MqttError::from("MQTT connection lost, unreachable!()");
                    log::warn!(
                        "{:?} {:?}, from: {:?}, message: {:?}",
                        self.id,
                        err,
                        iflt_msg.from,
                        iflt_msg.publish
                    );
                    return Err(err);
                };

                //rerelease
                let release_packet = match sink {
                    Sink::V3(_) => iflt_msg.release_packet_v3(),
                    Sink::V5(_) => iflt_msg.release_packet_v5(),
                };
                if let Some(release_packet) = release_packet {
                    sink.send(release_packet)?;
                    self.inflight_win().write().await.push_back(InflightMessage::new(
                        MomentStatus::UnComplete,
                        iflt_msg.from,
                        iflt_msg.publish,
                    ));
                } else {
                    log::error!("packet_id is None, {:?}", iflt_msg.publish);
                }
            }
        }
        Ok(())
    }

    #[inline]
    pub(crate) async fn subscribe(&self, sub: Subscribe) -> Result<SubscribeReturn> {
        let ret = self._subscribe(sub).await;
        match &ret {
            Ok(sub_ret) => match sub_ret.ack_reason {
                SubscribeAckReason::NotAuthorized => {
                    Metrics::instance().client_subscribe_auth_error_inc();
                }
                SubscribeAckReason::GrantedQos0
                | SubscribeAckReason::GrantedQos1
                | SubscribeAckReason::GrantedQos2 => {}
                _ => {
                    Metrics::instance().client_subscribe_error_inc();
                }
            },
            Err(_) => {
                Metrics::instance().client_subscribe_error_inc();
            }
        }
        ret
    }

    #[inline]
    async fn _subscribe(&self, mut sub: Subscribe) -> Result<SubscribeReturn> {
        if self.listen_cfg().max_subscriptions > 0
            && (self.subscriptions().await?.len().await >= self.listen_cfg().max_subscriptions)
        {
            return Err(MqttError::TooManySubscriptions);
        }

        if self.listen_cfg().max_topic_levels > 0
            && Topic::from_str(&sub.topic_filter)?.len() > self.listen_cfg().max_topic_levels
        {
            return Err(MqttError::TooManyTopicLevels);
        }

        sub.opts.set_qos(sub.opts.qos().less_value(self.listen_cfg().max_qos_allowed));

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
        let sub_ret =
            Runtime::instance().extends.shared().await.entry(self.id.clone()).subscribe(&sub).await?;

        if let Some(qos) = sub_ret.success() {
            //send retain messages
            if self.listen_cfg().retain_available {
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
                if send_retain_enable {
                    let retain_messages =
                        Runtime::instance().extends.retain().await.get(&sub.topic_filter).await?;
                    self.send_retain_messages(retain_messages, qos).await?;
                }
            };
            //hook, session_subscribed
            self.hook.session_subscribed(sub).await;
        }

        Ok(sub_ret)
    }

    #[inline]
    pub(crate) async fn unsubscribe(&self, mut unsub: Unsubscribe) -> Result<()> {
        log::debug!("{:?} unsubscribe: {:?}", self.id, unsub);
        //hook, client_unsubscribe
        let topic_filter = self.hook.client_unsubscribe(&unsub).await;
        if let Some(topic_filter) = topic_filter {
            unsub.topic_filter = topic_filter;
            log::debug!("{:?} adjust topic_filter: {:?}", self.id, unsub.topic_filter);
        }
        let ok =
            Runtime::instance().extends.shared().await.entry(self.id.clone()).unsubscribe(&unsub).await?;
        if ok {
            //hook, session_unsubscribed
            self.hook.session_unsubscribed(unsub).await;
        }
        Ok(())
    }

    #[inline]
    pub async fn publish_v3(&self, publish: &v3::Publish) -> Result<bool> {
        match self.publish(Publish::from(publish)).await {
            Err(e) => {
                Metrics::instance().client_publish_error_inc();
                self.disconnected_reason_add(Reason::PublishFailed(ByteString::from(e.to_string()))).await?;
                Err(e)
            }
            Ok(false) => {
                Metrics::instance().client_publish_error_inc();
                Ok(false)
            }
            Ok(true) => Ok(true),
        }
    }

    #[inline]
    pub async fn publish_v5(&self, publish: &v5::Publish) -> Result<bool> {
        match self._publish_v5(publish).await {
            Err(e) => {
                Metrics::instance().client_publish_error_inc();
                self.disconnected_reason_add(Reason::PublishFailed(ByteString::from(e.to_string()))).await?;
                Err(e)
            }
            Ok(false) => {
                Metrics::instance().client_publish_error_inc();
                Ok(false)
            }
            Ok(true) => Ok(true),
        }
    }

    #[inline]
    async fn _publish_v5(&self, publish: &v5::Publish) -> Result<bool> {
        log::debug!("{:?} publish: {:?}", self.id, publish);
        let mut p = Publish::from(publish);
        if let Some(client_topic_aliases) = &self.client_topic_aliases {
            p.topic = client_topic_aliases.set_and_get(p.properties.topic_alias, p.topic).await?;
        }
        self.publish(p).await
    }

    #[inline]
    async fn publish(&self, publish: Publish) -> Result<bool> {
        let from = From::from_custom(self.id.clone());

        //hook, message_publish
        let publish = self.hook.message_publish(from.clone(), &publish).await.unwrap_or(publish);

        //hook, message_publish_check_acl
        let acl_result = self.hook.message_publish_check_acl(&publish).await;
        log::debug!("{:?} acl_result: {:?}", self.id, acl_result);
        if let PublishAclResult::Rejected(disconnect) = acl_result {
            Metrics::instance().client_publish_auth_error_inc();
            //hook, Message dropped
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(None, from, publish, Reason::PublishRefused)
                .await;
            return if disconnect {
                Err(MqttError::from(
                    "Publish Refused, reason: hook::message_publish_check_acl() -> Rejected(Disconnect)",
                ))
            } else {
                Ok(false)
            };
        }

        if self.listen_cfg().retain_available && publish.retain() {
            Runtime::instance()
                .extends
                .retain()
                .await
                .set(publish.topic(), Retain { from: from.clone(), publish: publish.clone() })
                .await?;
        }

        match Runtime::instance().extends.shared().await.forwards(from.clone(), publish).await {
            Ok(0) => {
                //hook, message_nonsubscribed
                Runtime::instance().extends.hook_mgr().await.message_nonsubscribed(from).await;
            }
            Ok(_) => {}
            Err(errs) => {
                for (to, from, p, reason) in errs {
                    //Message dropped
                    Runtime::instance()
                        .extends
                        .hook_mgr()
                        .await
                        .message_dropped(Some(to), from, p, reason)
                        .await;
                }
            }
        }
        Ok(true)
    }

    #[inline]
    pub async fn clean(&self, reason: Reason) {
        log::debug!("{:?} clean, reason: {:?}", self.id, reason);

        //Session expired, discarding messages in deliver queue
        if let Some(queue) = self.deliver_queue_tx.as_ref() {
            while let Some((from, publish)) = queue.pop() {
                log::debug!("{:?} clean.dropped, from: {:?}, publish: {:?}", self.id, from, publish);

                //hook, message dropped
                Runtime::instance()
                    .extends
                    .hook_mgr()
                    .await
                    .message_dropped(Some(self.id.clone()), from, publish, reason.clone())
                    .await;
            }
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
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(Some(self.id.clone()), iflt_msg.from, iflt_msg.publish, reason.clone())
                .await;
        }

        //hook, session terminated
        self.hook.session_terminated(reason).await;

        //clear session, and unsubscribe
        let mut entry = Runtime::instance().extends.shared().await.entry(self.id.clone());
        if let Some(true) = entry.id_same() {
            if let Err(e) = entry.remove_with(&self.id).await {
                log::warn!("{:?} failed to remove the session from the broker, {:?}", self.id, e);
            }
        }
    }

    #[inline]
    pub async fn transfer_session_state(
        &self,
        clear_subscriptions: bool,
        mut offline_info: SessionOfflineInfo,
    ) -> Result<()> {
        log::debug!(
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
                if let Err(e) = Runtime::instance().extends.router().await.add(tf, id, opts.clone()).await {
                    log::warn!("transfer_session_state, router.add, {:?}", e);
                    return Err(e);
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
    async fn clean_session(&self, d: Option<&Disconnect>) -> bool {
        let connect_info = self.connect_info().await;
        if let Ok(connect_info) = connect_info.as_ref() {
            if let ConnectInfo::V3(_, conn_info) = connect_info.as_ref() {
                conn_info.clean_session
            } else {
                self.fitter.session_expiry_interval(d).await.is_zero()
            }
        } else {
            true
        }
    }
}

impl Deref for SessionState {
    type Target = Session;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionOfflineInfo {
    pub id: Id,
    pub subscriptions: Subscriptions,
    pub offline_messages: Vec<(From, Publish)>,
    pub inflight_messages: Vec<InflightMessage>,
    pub created_at: TimestampMillis,
}

impl std::fmt::Debug for SessionOfflineInfo {
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
    pub extra_attrs: RwLock<ExtraAttrs>,
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
        Runtime::instance().stats.sessions.dec();
        let id = self.id.clone();
        let s = self.inner.clone();
        tokio::spawn(async move {
            if let Err(e) = s.on_drop().await {
                log::error!("{:?} session clear error, {:?}", id, e);
            }
        });
    }
}

impl std::fmt::Debug for Session {
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
        max_mqueue_len: usize,
        listen_cfg: Listener,
        fitter: FitterType,
        max_inflight: NonZeroU16,
        created_at: TimestampMillis,

        conn_info: ConnectInfoType,
        session_present: bool,
        superuser: bool,
        connected: bool,
        connected_at: TimestampMillis,

        subscriptions: SessionSubs,
        disconnect_info: Option<DisconnectInfo>,
    ) -> Self {
        let max_inflight = max_inflight.get() as usize;
        let message_retry_interval = listen_cfg.message_retry_interval.as_millis() as TimestampMillis;
        let message_expiry_interval = listen_cfg.message_expiry_interval.as_millis() as TimestampMillis;
        let mut deliver_queue = MessageQueue::new(max_mqueue_len);
        deliver_queue.on_push(|| {
            Runtime::instance().stats.message_queues.inc();
        });
        deliver_queue.on_pop(|| {
            Runtime::instance().stats.message_queues.dec();
        });
        let out_inflight = Inflight::new(max_inflight, message_retry_interval, message_expiry_interval)
            .on_push(|| {
                Runtime::instance().stats.out_inflights.inc();
            })
            .on_pop(|| {
                Runtime::instance().stats.out_inflights.dec();
            });

        Runtime::instance().stats.sessions.inc();
        Runtime::instance().stats.subscriptions.incs(subscriptions.len().await as isize);
        Runtime::instance().stats.subscriptions_shared.incs(subscriptions.shared_len().await as isize);

        let extra_attrs = RwLock::new(ExtraAttrs::new());
        let session_like = Runtime::instance().extends.session_mgr().await.create(
            id.clone(),
            listen_cfg,
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
        );
        Self(Arc::new(_Session { inner: session_like, id, fitter, extra_attrs }))
    }

    #[inline]
    pub async fn to_offline_info(&self) -> Result<SessionOfflineInfo> {
        let id = self.id.clone();
        let created_at = self.created_at().await?;
        let subscriptions = self.subscriptions_drain().await?;

        let mut offline_messages = Vec::new();
        while let Some(item) = self.deliver_queue().pop() {
            //@TODO ..., check message expired
            offline_messages.push(item);
        }
        let mut inflight_win = self.inflight_win().write().await;
        let mut inflight_messages = Vec::new();
        while let Some(msg) = inflight_win.pop_front() {
            //@TODO ..., check message expired
            inflight_messages.push(msg);
        }
        Ok(SessionOfflineInfo { id, subscriptions, offline_messages, inflight_messages, created_at })
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

pub trait SessionManager: Sync + Send {
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        id: Id,
        listen_cfg: Listener,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: InflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,
    ) -> Arc<dyn SessionLike>;
}

#[async_trait]
pub trait SessionLike: Sync + Send {
    fn listen_cfg(&self) -> &Listener;
    fn deliver_queue(&self) -> &MessageQueueType;
    fn inflight_win(&self) -> &InflightType;

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
