use bytestring::ByteString;
use std::convert::AsRef;
use std::convert::From as _f;
use std::convert::TryFrom;
use std::fmt;
use std::num::NonZeroU16;
use std::ops::Deref;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use futures::StreamExt;
use ntex_mqtt::types::MQTT_LEVEL_5;
use ntex_mqtt::v5::codec::RetainHandling;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

use crate::broker::inflight::{Inflight, InflightMessage, MomentStatus};
use crate::broker::queue::{Limiter, Policy, Queue, Sender};
use crate::broker::types::*;
use crate::broker::{fitter::Fitter, hook::Hook};
use crate::metrics::Metrics;
use crate::settings::listener::Listener;
use crate::{MqttError, Result, Runtime};

type MessageSender = Sender<(From, Publish)>;
type MessageQueue = Queue<(From, Publish)>;

#[derive(Clone)]
pub struct SessionState {
    pub tx: Option<Tx>,
    pub session: Session,
    pub client: ClientInfo,
    pub sink: Sink,
    pub hook: Rc<dyn Hook>,
    pub deliver_queue_tx: Option<MessageSender>,
    pub server_topic_aliases: Option<Rc<ServerTopicAliases>>,
    pub client_topic_aliases: Option<Rc<ClientTopicAliases>>,
}

impl fmt::Debug for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SessionState {{ {:?}, {:?}, {:?}, {} }}",
            self.id,
            self.session,
            self.client,
            self.deliver_queue_tx.as_ref().map(|tx| tx.len()).unwrap_or_default()
        )
    }
}

impl SessionState {
    #[inline]
    pub(crate) fn new(
        session: Session,
        client: ClientInfo,
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
            client,
            sink,
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
        self.tx.replace(msg_tx.clone());
        let mut state = self.clone();

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

        log::debug!("{:?} there are {} offline messages ...", state.id, state.deliver_queue.len());

        ntex::rt::spawn(async move {
            Runtime::instance().stats.connections.inc();

            let (deliver_queue_tx, mut deliver_queue_rx) = limiter.channel(state.deliver_queue.clone());
            //When the message queue is full, the message dropping policy is implemented
            let deliver_queue_tx = deliver_queue_tx.policy(|(_, p): &(From, Publish)| -> Policy {
                if let QoS::AtMostOnce = p.qos() {
                    Policy::Current
                } else {
                    Policy::Early
                }
            });
            state.deliver_queue_tx.replace(deliver_queue_tx.clone());

            tokio::pin!(keep_alive_delay);

            tokio::pin!(deliver_timeout_delay);

            loop {
                log::debug!("{:?} tokio::select! loop", state.id);
                deliver_timeout_delay.as_mut().reset(
                    Instant::now()
                        + state
                            .inflight_win
                            .read()
                            .await
                            .get_timeout()
                            .unwrap_or_else(|| Duration::from_secs(120)),
                );

                tokio::select! {
                    _ = &mut keep_alive_delay => {  //, if !keep_alive_delay.is_elapsed()
                        log::debug!("{:?} keep alive is timeout, is_elapsed: {:?}", state.id, keep_alive_delay.is_elapsed());
                        state.client.add_disconnected_reason(Reason::ConnectKeepaliveTimeout).await;
                        break
                    },
                    msg = msg_rx.next() => {
                        log::debug!("{:?} recv msg: {:?}", state.id, msg);
                        if let Some(msg) = msg{
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
                                        state.client.add_disconnected_reason(Reason::ConnectKicked(is_admin)).await;
                                        break
                                    }else{
                                        log::warn!("{:?} Message::Kick, kick sender is closed, to {:?}, is_admin: {}", state.id, by_id, is_admin);
                                    }
                                },
                                Message::Disconnect(d) => {
                                    flags.insert(StateFlags::DisconnectReceived);
                                    state.client.set_mqtt_disconnect(d).await;
                                },
                                Message::Closed(reason) => {
                                    log::debug!("{:?} Closed({}) message received, reason: {}", state.id, flags.contains(StateFlags::DisconnectReceived), reason);
                                    if !state.client.has_disconnected_reason().await{
                                        state.client.add_disconnected_reason(reason).await;
                                    }
                                    break
                                },
                                Message::Keepalive => {
                                    log::debug!("{:?} Message::Keepalive ... ", state.id);
                                    keep_alive_delay.as_mut().reset(Instant::now() + keep_alive_interval);
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
                            state.client.add_disconnected_reason(Reason::from_static("None is received from the Rx")).await;
                            break;
                        }
                    },

                    _ = &mut deliver_timeout_delay => {
                        while let Some(iflt_msg) = state.inflight_win.write().await.pop_front_timeout(){
                            log::debug!("{:?} has timeout message in inflight: {:?}", state.id, iflt_msg);
                            if let Err(e) = state.reforward(iflt_msg).await{
                                log::error!("{:?} redeliver message error, {:?}", state.id, e);
                            }
                        }
                    },

                    deliver_packet = deliver_queue_rx.next(), if state.inflight_win.read().await.has_credit() => {
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
                                state.client.add_disconnected_reason("Deliver Queue is closed".into()).await;
                                break;
                            }
                        }
                    }
                }
            }

            let clean_session = state.clean_session().await;

            log::debug!(
                "{:?} exit online worker, flags: {:?}, clean_session: {} {}",
                state.id,
                flags,
                clean_session,
                flags.contains(StateFlags::CleanStart)
            );

            Runtime::instance().stats.connections.dec();

            //Setting the disconnected state
            state.client.set_disconnected(None).await;

            //Last will message
            let will_delay_interval = if state.last_will_enable(flags, clean_session) {
                let will_delay_interval = state.will_delay_interval();
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

            state.sink.close();

            //hook, client_disconnected
            let reason = if state.client.has_disconnected_reason().await {
                state.client.get_disconnected_reason().await
            } else {
                state.client.add_disconnected_reason(Reason::ConnectRemoteClose).await;
                Reason::ConnectRemoteClose
            };
            state.hook.client_disconnected(reason).await;

            if flags.contains(StateFlags::Kicked) {
                if flags.contains(StateFlags::ByAdminKick) {
                    state.clean(state.client.take_disconnected_reason().await).await;
                }
            } else if clean_session {
                state.clean(state.client.take_disconnected_reason().await).await;
            } else {
                //Start offline event loop
                Self::offline_start(
                    state.clone(),
                    &mut msg_rx,
                    &deliver_queue_tx,
                    &mut flags,
                    will_delay_interval,
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
    ) {
        let session_expiry_interval = state.fitter.session_expiry_interval().await;
        log::debug!(
            "{:?} start offline event loop, session_expiry_interval: {:?}, will_delay_interval: {:?}",
            state.id,
            session_expiry_interval,
            will_delay_interval
        );

        //state.client.disconnect
        let session_expiry_delay = tokio::time::sleep(session_expiry_interval);
        tokio::pin!(session_expiry_delay);

        let will_delay_interval_delay =
            tokio::time::sleep(will_delay_interval.unwrap_or(Duration::MAX));
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
                                log::info!("{:?} offline receive message is {:?}", state.id, msg);
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
    fn will_delay_interval(&self) -> Option<Duration> {
        self.client.last_will().and_then(|lw| lw.will_delay_interval())
    }

    #[inline]
    async fn process_last_will(&self) -> Result<()> {
        if let Some(lw) = self.client.last_will() {
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
            publish.set_packet_id(self.inflight_win.read().await.next_id()?);
        }

        //hook, message_delivered
        let publish = self.hook.message_delivered(from.clone(), &publish).await.unwrap_or(publish);

        //send message
        self.sink
            .publish(&publish, expiry_check_res.message_expiry_interval(), self.server_topic_aliases.as_ref())
            .await?; //@TODO ... at exception, send hook and or store message

        //cache messages to inflight window
        let moment_status = match publish.qos() {
            QoS::AtLeastOnce => Some(MomentStatus::UnAck),
            QoS::ExactlyOnce => Some(MomentStatus::UnReceived),
            _ => None,
        };
        if let Some(moment_status) = moment_status {
            self.inflight_win.write().await.push_back(InflightMessage::new(moment_status, from, publish));
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

                //rerelease
                let release_packet = match &self.sink {
                    Sink::V3(_) => iflt_msg.release_packet_v3(),
                    Sink::V5(_) => iflt_msg.release_packet_v5(),
                };
                if let Some(release_packet) = release_packet {
                    self.sink.send(release_packet)?;
                    self.inflight_win.write().await.push_back(InflightMessage::new(
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
        if self.listen_cfg.max_subscriptions > 0
            && (self.subscriptions.len() >= self.listen_cfg.max_subscriptions)
        {
            return Err(MqttError::TooManySubscriptions);
        }

        if self.listen_cfg.max_topic_levels > 0
            && Topic::from_str(&sub.topic_filter)?.len() > self.listen_cfg.max_topic_levels
        {
            return Err(MqttError::TooManyTopicLevels);
        }

        sub.opts.set_qos(sub.opts.qos().less_value(self.listen_cfg.max_qos_allowed));

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
            if self.listen_cfg.retain_available {
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
                self.client
                    .add_disconnected_reason(Reason::PublishFailed(ByteString::from(e.to_string())))
                    .await;
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
                self.client
                    .add_disconnected_reason(Reason::PublishFailed(ByteString::from(e.to_string())))
                    .await;
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
            p.topic = client_topic_aliases.set_and_get(p.properties.topic_alias, p.topic)?;
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

        if self.listen_cfg.retain_available && publish.retain() {
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
    pub(crate) async fn clean(&self, reason: Reason) {
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
        while let Some(iflt_msg) = self.inflight_win.write().await.pop_front() {
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
            self.subscriptions.extend(offline_info.subscriptions);
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
    async fn clean_session(&self) -> bool {
        if let ConnectInfo::V3(_, conn_info) = &self.client.connect_info {
            conn_info.clean_session
        } else {
            self.fitter.session_expiry_interval().await.is_zero()
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
pub struct Session(Arc<_SessionInner>);

impl Session {
    #[inline]
    pub(crate) fn new(
        id: Id,
        fitter: Box<dyn Fitter>,
        listen_cfg: Listener,
        max_inflight: NonZeroU16,
        created_at: TimestampMillis,
    ) -> Self {
        let max_mqueue_len = fitter.max_mqueue_len();
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
                Runtime::instance().stats.inflights.inc();
            })
            .on_pop(|| {
                Runtime::instance().stats.inflights.dec();
            });

        Runtime::instance().stats.sessions.inc();
        Self(Arc::new(_SessionInner {
            id,
            fitter,
            listen_cfg,
            subscriptions: SessionSubs::new(),
            deliver_queue: Arc::new(deliver_queue),
            inflight_win: Arc::new(RwLock::new(out_inflight)),
            created_at,
        }))
    }

    #[inline]
    pub async fn to_offline_info(&self) -> SessionOfflineInfo {
        let id = self.id.clone();
        let subscriptions = self.subscriptions.drain();
        let mut offline_messages = Vec::new();
        while let Some(item) = self.deliver_queue.pop() {
            //@TODO ..., check message expired
            offline_messages.push(item);
        }
        let mut inflight_win = self.inflight_win.write().await;
        let mut inflight_messages = Vec::new();
        while let Some(msg) = inflight_win.pop_front() {
            //@TODO ..., check message expired
            inflight_messages.push(msg);
        }
        SessionOfflineInfo {
            id,
            subscriptions,
            offline_messages,
            inflight_messages,
            created_at: self.created_at,
        }
    }
}

impl Deref for Session {
    type Target = _SessionInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl std::fmt::Debug for Session {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session {:?}", self.id)
    }
}

pub struct _SessionInner {
    pub id: Id,
    pub fitter: Box<dyn Fitter>,
    pub listen_cfg: Listener,
    //Current subscription for this session
    pub subscriptions: SessionSubs,
    pub deliver_queue: Arc<MessageQueue>,
    pub inflight_win: Arc<RwLock<Inflight>>,
    pub created_at: TimestampMillis,
}

impl Drop for _SessionInner {
    fn drop(&mut self) {
        Runtime::instance().stats.sessions.dec();
        self.subscriptions.clear();
    }
}

impl _SessionInner {
    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let count = self.subscriptions.len();

        let subs = self
            .subscriptions
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let (tf, opts) = entry.pair();
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

        let data = json!({
            "subscriptions": {
                "count": count,
                "topic_filters": subs,
            },
            "queues": self.deliver_queue.len(),
            "inflights": self.inflight_win.read().await.len(),
            "created_at": self.created_at,
        });
        data
    }
}

#[derive(Clone)]
pub struct ClientInfo(Arc<_ClientInfo>);

impl ClientInfo {
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn new(
        connect_info: ConnectInfo,
        session_present: bool,
        superuser: bool,
        connected_at: TimestampMillis,
    ) -> ClientInfo {
        let id = connect_info.id().clone();
        Self(Arc::new(_ClientInfo {
            id,
            connect_info,
            session_present,
            superuser,
            connected: AtomicBool::new(true),
            connected_at,
            disconnected_at: AtomicI64::new(0),
            disconnected_reason: RwLock::new(Vec::new()),
            disconnect: RwLock::new(None),
            extra_attrs: Arc::new(RwLock::new(ExtraAttrs::new())),
        }))
    }

    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let mut json = self.connect_info.to_json();
        if let Some(json) = json.as_object_mut() {
            json.insert("superuser".into(), serde_json::Value::Bool(self.superuser));
            json.insert("session_present".into(), serde_json::Value::Bool(self.session_present));
            json.insert("connected".into(), serde_json::Value::Bool(self.connected.load(Ordering::SeqCst)));
            json.insert(
                "connected_at".into(),
                serde_json::Value::Number(serde_json::Number::from(self.connected_at)),
            );
            json.insert(
                "disconnected_at".into(),
                serde_json::Value::Number(serde_json::Number::from(
                    self.disconnected_at.load(Ordering::SeqCst),
                )),
            );

            json.insert(
                "disconnected_reason".into(),
                serde_json::Value::String(self.get_disconnected_reason().await.to_string()),
            );

            json.insert(
                "extra_attrs".into(),
                serde_json::Value::Number(serde_json::Number::from(self.extra_attrs.read().await.len())),
            );
        }
        json
    }

    #[inline]
    pub fn protocol(&self) -> u8 {
        match &self.connect_info {
            ConnectInfo::V3(_, conn_info) => conn_info.protocol.level(),
            ConnectInfo::V5(_, _conn_info) => MQTT_LEVEL_5,
        }
    }

    #[inline]
    pub fn last_will(&self) -> Option<LastWill> {
        self.connect_info.last_will()
    }

    #[inline]
    pub fn username(&self) -> &str {
        self.id.username_ref()
    }

    #[inline]
    pub fn disconnected_at(&self) -> TimestampMillis {
        self.disconnected_at.load(Ordering::SeqCst)
    }

    #[inline]
    pub async fn set_disconnected(&self, reason: Option<Reason>) {
        self.connected.store(false, Ordering::SeqCst);
        self.disconnected_at.store(chrono::Local::now().timestamp_millis(), Ordering::SeqCst);
        if let Some(reason) = reason {
            self.add_disconnected_reason(reason).await;
        }
        self.extra_attrs.write().await.clear();
    }

    #[inline]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    #[inline]
    pub async fn add_disconnected_reason(&self, r: Reason) {
        self.disconnected_reason.write().await.push(r);
    }

    pub(crate) async fn set_mqtt_disconnect(&self, d: Disconnect) {
        self.add_disconnected_reason(d.reason()).await;
        self.disconnect.write().await.replace(d);
    }

    #[inline]
    pub async fn get_disconnected_reason(&self) -> Reason {
        Reason::Reasons(self.disconnected_reason.read().await.clone())
    }

    #[inline]
    pub async fn take_disconnected_reason(&self) -> Reason {
        Reason::Reasons(self.disconnected_reason.write().await.drain(..).collect())
    }

    #[inline]
    pub async fn has_disconnected_reason(&self) -> bool {
        !self.disconnected_reason.read().await.is_empty()
    }
}

impl std::fmt::Debug for ClientInfo {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientInfo: {:?}", self.id)
    }
}

impl Deref for ClientInfo {
    type Target = _ClientInfo;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

pub struct _ClientInfo {
    pub id: Id,
    pub connect_info: ConnectInfo,
    pub session_present: bool,
    pub superuser: bool,
    pub connected: AtomicBool,
    pub connected_at: TimestampMillis,
    pub disconnected_at: AtomicI64,
    pub disconnected_reason: RwLock<Vec<Reason>>,
    pub disconnect: RwLock<Option<Disconnect>>,
    pub extra_attrs: Arc<RwLock<ExtraAttrs>>,
}
