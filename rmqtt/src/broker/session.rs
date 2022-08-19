use std::convert::AsRef;
use std::convert::From as _f;
use std::convert::TryFrom;
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use futures::StreamExt;
use ntex_mqtt::types::MQTT_LEVEL_5;
use tokio::sync::mpsc;
use tokio::sync::RwLock;
use tokio::time::{Duration, Instant};

use crate::{MqttError, Result, Runtime};
use crate::broker::{fitter::Fitter, hook::Hook};
use crate::broker::inflight::{Inflight, InflightMessage, MomentStatus};
use crate::broker::queue::{Limiter, Policy, Queue, Sender};
use crate::broker::types::*;
use crate::settings::listener::Listener;

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
    pub fitter: Rc<dyn Fitter>,
}

unsafe impl std::marker::Send for SessionState {}

unsafe impl std::marker::Sync for SessionState {}

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
        fitter: Rc<dyn Fitter>,
    ) -> Self {
        Self { tx: None, session, client, sink, hook, deliver_queue_tx: None, fitter }
    }

    #[inline]
    pub(crate) async fn start(mut self, keep_alive: u16) -> (Self, Tx) {
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel();
        self.tx.replace(msg_tx.clone());
        let mut state = self.clone();
        ntex::rt::spawn(async move {
            log::debug!("{:?} there are {} offline messages ...", state.id, state.deliver_queue.len());
            Runtime::instance().stats.sessions.inc();
            Runtime::instance().stats.connections.inc();

            let (burst, replenish_n_per) = state.fitter.mqueue_rate_limit();
            let limiter = Limiter::new(burst, replenish_n_per);
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

            let id = state.id.clone();
            let mut _kicked = false;
            let mut _by_admin_kick = false;
            let mut _disconnect_received = false;
            log::debug!("{:?} start online event loop", id);

            let keep_alive_interval = if keep_alive < 10 {
                Duration::from_secs(10)
            } else {
                Duration::from_secs((keep_alive + 10) as u64)
            };
            log::debug!("{:?} keep_alive_interval is {:?}", id, keep_alive_interval);
            let keep_alive_delay = tokio::time::sleep(keep_alive_interval);
            tokio::pin!(keep_alive_delay);

            let deliver_timeout_delay = tokio::time::sleep(Duration::from_secs(60));
            tokio::pin!(deliver_timeout_delay);

            loop {
                log::debug!("{:?} tokio::select! loop", id);
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
                        log::debug!("{:?} keep alive is timeout, is_elapsed: {:?}", id, keep_alive_delay.is_elapsed());
                        state.client.add_disconnected_reason(Reason::from_static("Timeout(Read/Write)")).await;
                        break
                    },
                    msg = msg_rx.recv() => {
                        log::debug!("{:?} recv msg: {:?}", id, msg);
                        if let Some(msg) = msg{
                            match msg{
                                Message::Forward(from, p) => {
                                    if let Err((from, p)) = deliver_queue_tx.send((from, p)).await{
                                        log::warn!("{:?} deliver_dropped, from: {:?}, {:?}", id, from, p);
                                        //hook, message_dropped
                                        Runtime::instance().extends.hook_mgr().await.message_dropped(Some(state.id.clone()), from, p, Reason::from_static("deliver queue is full")).await;
                                    }
                                },
                                Message::Kick(sender, by_id, is_admin) => {
                                    log::debug!("{:?} Message::Kick, send kick result, to {:?}, is_admin: {}", id, by_id, is_admin);
                                    if !sender.is_closed() {
                                        if let Err(e) = sender.send(()) {
                                            log::warn!("{:?} Message::Kick, send response error, {:?}", id, e);
                                        }
                                        _kicked = true;
                                        _by_admin_kick = is_admin;
                                        state.client.add_disconnected_reason(Reason::from(format!("Kicked by {:?}, is_admin: {}", by_id, is_admin))).await;
                                        break
                                    }else{
                                        log::warn!("{:?} Message::Kick, kick sender is closed, to {:?}, is_admin: {}", id, by_id, is_admin);
                                    }
                                },
                                Message::Disconnect(d) => {
                                    _disconnect_received = true;
                                    state.client.set_mqtt_disconnect(d).await;
                                    state.client.add_disconnected_reason("Disconnect(true) message is received".into()).await;

                                },
                                Message::Closed(reason) => {
                                    log::debug!("{:?} Closed({}) message received, reason: {}", id, _disconnect_received, reason);
                                    if !state.client.has_disconnected_reason().await{
                                        state.client.add_disconnected_reason(reason).await;
                                    }
                                    break
                                },
                                Message::Keepalive => {
                                    keep_alive_delay.as_mut().reset(Instant::now() + keep_alive_interval);
                                },
                                Message::Subscribe(sub, reply_tx) => {
                                    let sub_reply = state.subscribe(sub).await;
                                    if !reply_tx.is_closed(){
                                        if let Err(e) = reply_tx.send(sub_reply) {
                                            log::warn!("{:?} Message::Subscribe, send response error, {:?}", id, e);
                                        }
                                    }else{
                                        log::warn!("{:?} Message::Subscribe, reply sender is closed", id);
                                    }
                                },
                                Message::Unsubscribe(unsub, reply_tx) => {
                                    let unsub_reply = state.unsubscribe(unsub).await;
                                    if !reply_tx.is_closed(){
                                        if let Err(e) = reply_tx.send(unsub_reply) {
                                            log::warn!("{:?} Message::Unsubscribe, send response error, {:?}", id, e);
                                        }
                                    }else{
                                        log::warn!("{:?} Message::Unsubscribe, reply sender is closed", id);
                                    }
                                }
                            }
                        }else{
                            log::warn!("{:?} None is received from the Rx", id);
                            state.client.add_disconnected_reason(Reason::from_static("None is received from the Rx")).await;
                            break;
                        }
                    },

                    _ = &mut deliver_timeout_delay => {
                        while let Some(iflt_msg) = state.inflight_win.write().await.pop_front_timeout(){
                            log::debug!("{:?} has timeout message in inflight: {:?}", id, iflt_msg);
                            if let Err(e) = state.reforward(iflt_msg).await{
                                log::error!("{:?} redeliver message error, {:?}", id, e);
                            }
                        }
                    },

                    deliver_packet = deliver_queue_rx.next(), if state.inflight_win.read().await.has_credit() => {
                        log::debug!("{:?} deliver_packet: {:?}", id, deliver_packet);
                        match deliver_packet{
                            Some(Some((from, p))) => {
                                if let Err(e) = state.deliver(from, p).await{
                                    log::error!("{:?} deliver message error, {:?}", id, e);
                                }
                            },
                            Some(None) => {
                                log::warn!("{:?} None is received from the deliver Queue", id);
                            },
                            None => {
                                log::warn!("{:?} Deliver Queue is closed", id);
                                state.client.add_disconnected_reason("Deliver Queue is closed".into()).await;
                                break;
                            }
                        }
                    }
                }
            }

            log::debug!(
                "{:?} exit online worker, kicked: {}, clean_session: {}",
                id,
                _kicked,
                state.clean_session().await
            );

            Runtime::instance().stats.connections.dec();

            //Setting the disconnected state
            state.client.set_disconnected(None).await;
            if !_disconnect_received {
                if let Err(e) = state.process_last_will().await {
                    log::error!("{:?} process last will error, {:?}", id, e);
                }
            }
            state.sink.close();

            //hook, client_disconnected
            let reason = state
                .client
                .get_disconnected_reason()
                .await
                .unwrap_or(Reason::from_static("Remote close connect"));
            state.hook.client_disconnected(reason).await;

            if !_kicked {
                if state.clean_session().await {
                    state.clean(state.client.get_disconnected_reason().await.unwrap_or_default()).await;
                } else {
                    //Start offline event loop
                    Self::offline_start(
                        state.clone(),
                        &mut msg_rx,
                        &deliver_queue_tx,
                        &mut _kicked,
                        &mut _by_admin_kick,
                    )
                        .await;
                    log::debug!("{:?} offline _kicked: {}", id, _kicked);
                    if !_kicked {
                        state.clean(Reason::from_static("session expired")).await;
                    }
                }
            }

            Runtime::instance().stats.sessions.dec();
        });
        (self, msg_tx)
    }

    #[inline]
    async fn offline_start(
        state: SessionState,
        msg_rx: &mut Rx,
        deliver_queue_tx: &MessageSender,
        kicked: &mut bool,
        by_admin_kick: &mut bool,
    ) {
        log::debug!("{:?} start offline event loop", state.id);
        let id = state.id.clone();

        //state.client.disconnect
        let session_expiry_interval = state.fitter.session_expiry_interval(); //state.listen_cfg.session_expiry_interval;
        let session_expiry_delay = tokio::time::sleep(session_expiry_interval);
        tokio::pin!(session_expiry_delay);

        loop {
            tokio::select! {
                msg = msg_rx.recv() => {
                    log::debug!("{:?} recv offline msg: {:?}", id, msg);
                    if let Some(msg) = msg{
                        match msg{
                            Message::Forward(from, p) => {
                                if let Err((from, p)) = deliver_queue_tx.send((from, p)).await{
                                    log::warn!("{:?} offline deliver_dropped, from: {:?}, {:?}", id, from, p);
                                    //hook, message_dropped
                                    Runtime::instance().extends.hook_mgr().await.message_dropped(Some(state.id.clone()), from, p, Reason::from_static("deliver queue is full")).await;
                                }
                            },
                            Message::Kick(sender, by_id, is_admin) => {
                                log::debug!("{:?} offline Kicked, send kick result, to: {:?}, is_admin: {}", id, by_id, is_admin);
                                if !sender.is_closed() {
                                    if let Err(e) = sender.send(()) {
                                        log::warn!("{:?} offline Kick send response error, to: {:?}, is_admin: {}, {:?}", id, by_id, is_admin, e);
                                    }
                                    *kicked = true;
                                    *by_admin_kick = is_admin;
                                    break
                                }else{
                                    log::warn!("{:?} offline Kick sender is closed, to {:?}, is_admin: {}", id, by_id, is_admin);
                                }
                            },
                            _ => {
                                log::info!("{:?} offline receive message is {:?}", id, msg);
                            }
                        }
                    }else{
                        log::warn!("{:?} offline None is received from the Rx", id);
                        break;
                    }
                },
               _ = &mut session_expiry_delay => { //, if !session_expiry_delay.is_elapsed() => {
                  log::debug!("{:?} session expired", id);
                  break
               },
            }
        }
        log::debug!("{:?} exit offline worker", id);
    }

    #[inline]
    pub(crate) async fn forward(&self, from: From, p: Publish) {
        let res = if let Some(ref tx) = self.tx {
            if let Err(e) = tx.send(Message::Forward(from, p)) {
                if let Message::Forward(from, p) = e.0 {
                    Err((from, p, "Send Publish message error, Tx is closed"))
                } else {
                    Ok(())
                }
            } else {
                Ok(())
            }
        } else {
            log::warn!("{:?} Message Sender is None", self.id);
            Err((from, p, "Message Sender is None"))
        };

        if let Err((from, p, reason)) = res {
            //hook, message_dropped
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(Some(self.id.clone()), from, p, Reason::from_static(reason))
                .await;
        }
    }

    #[inline]
    pub(crate) fn send(&self, msg: Message) -> Result<()> {
        if let Some(ref tx) = self.tx {
            tx.send(msg)?;
            Ok(())
        } else {
            Err(MqttError::from("Message Sender is None"))
        }
    }

    #[inline]
    async fn process_last_will(&self) -> Result<()> {
        if let Some(lw) = self.client.last_will() {
            //@TODO ...
            let p = Publish::try_from(lw)?;
            if let Err(e) = Runtime::instance().extends.shared().await.forwards(self.id.clone(), p).await {
                log::error!("{:?} send last will message fail, {:?}", self.id, e);
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
        let expiry = self.hook.message_expiry_check(from.clone(), &publish).await;

        if expiry {
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(
                    Some(self.id.clone()),
                    from,
                    publish,
                    Reason::from_static("message is expired"),
                )
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
        self.sink.publish(publish.clone())?; //@TODO ... at exception, send hook and or store message

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
                let expiry = self.hook.message_expiry_check(iflt_msg.from.clone(), &iflt_msg.publish).await;

                if expiry {
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
    pub(crate) async fn subscribe(&self, mut sub: Subscribe) -> Result<SubscribeReturn> {
        if self.listen_cfg.max_subscriptions > 0
            && (self.subscriptions_len() >= self.listen_cfg.max_subscriptions)
        {
            return Err(MqttError::TooManySubscriptions);
        }

        sub.qos = sub.qos.less_value(self.listen_cfg.max_qos_allowed);

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
                sub.qos = sub.qos.less_value(qos)
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
                let retain_messages =
                    Runtime::instance().extends.retain().await.get(&sub.topic_filter).await?;
                self.send_retain_messages(retain_messages, qos).await?;
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
    pub async fn publish_v3(&self, publish: &v3::Publish) -> Result<()> {
        if let Err(e) = self.publish(Publish::try_from(publish)?).await {
            self.client.add_disconnected_reason(Reason::from(format!("Publish failed, {:?}", e))).await;
            return Err(e);
        }
        Ok(())
    }

    #[inline]
    pub async fn publish_v5(&self, publish: &v5::Publish) -> Result<()> {
        if let Err(e) = self.publish(Publish::try_from(publish)?).await {
            self.client.add_disconnected_reason(Reason::from(format!("Publish failed, {:?}", e))).await;
            return Err(e);
        }
        Ok(())
    }

    #[inline]
    async fn publish(&self, publish: Publish) -> Result<()> {
        //hook, message_publish
        let publish = self.hook.message_publish(&publish).await.unwrap_or(publish);

        //hook, message_publish_check_acl
        let acl_result = self.hook.message_publish_check_acl(&publish).await;
        log::debug!("{:?} acl_result: {:?}", self.id, acl_result);
        if let PublishAclResult::Rejected(disconnect) = acl_result {
            //Message dropped
            Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_dropped(
                    None,
                    self.id.clone(),
                    publish,
                    Reason::from(format!(
                        "hook::message_publish_check_acl, publish rejected, disconnect:{}",
                        disconnect
                    )),
                )
                .await;
            return if disconnect {
                Err(MqttError::from(
                    "Publish Refused, reason: hook::message_publish_check_acl() -> Rejected(Disconnect)",
                ))
            } else {
                Ok(())
            };
        }

        if self.listen_cfg.retain_available && publish.retain() {
            Runtime::instance()
                .extends
                .retain()
                .await
                .set(publish.topic(), Retain { from: self.id.clone(), publish: publish.clone() })
                .await?;
        }

        if let Err(errs) = Runtime::instance().extends.shared().await.forwards(self.id.clone(), publish).await
        {
            for (to, from, p, reason) in errs {
                //Message dropped
                Runtime::instance().extends.hook_mgr().await.message_dropped(Some(to), from, p, reason).await;
            }
        }

        Ok(())
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

        let id_not_same = if let Some(id) = Runtime::instance().extends.shared().await.id(&self.id.client_id)
        {
            if self.id != id && self.id.node_id == id.node_id {
                log::info!("id not the same, id: {:?}, current id: {:?}, reason: {:?}", self.id, id, reason);
                true
            } else {
                false
            }
        } else {
            false
        };

        if !id_not_same {
            //hook, session terminated
            self.hook.session_terminated(reason).await;

            //clear session, and unsubscribe
            if let Err(e) = Runtime::instance().extends.shared().await.entry(self.id.clone()).remove().await {
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
        if !clear_subscriptions && !offline_info.subscriptions.is_empty()
        {
            for (tf, (qos, shared_group)) in offline_info.subscriptions.iter() {
                let shared_group = shared_group.as_ref().cloned();
                let qos = *qos;
                let id = self.id.clone();
                log::debug!("{:?} transfer_session_state, router.add ... topic_filter: {:?}, shared_group: {:?}, qos: {:?}", id, tf, shared_group, qos);
                if let Err(e) =
                Runtime::instance().extends.router().await.add(tf, id, qos, shared_group).await
                {
                    log::warn!("transfer_session_state, router.add, {:?}", e);
                    return Err(e);
                }
            }
        }

        //Subscription transfer from previous session
        if !clear_subscriptions {
            for (tf, (qos, shared_sub)) in offline_info.subscriptions.drain(..) {
                self.subscriptions_add(tf, qos, shared_sub);
            }
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
    async fn session_expiry_interval(&self) -> Duration {
        if let Some(Disconnect::V5(d)) = self.client.disconnect.read().await.as_ref() {
            if let Some(interval_secs) = d.session_expiry_interval_secs {
                Duration::from_secs(interval_secs as u64)
            } else {
                self.fitter.session_expiry_interval()
            }
        } else {
            self.fitter.session_expiry_interval()
        }
    }

    #[inline]
    async fn clean_session(&self) -> bool {
        if let ConnectInfo::V3(_, conn_info) = &self.client.connect_info {
            conn_info.clean_session
        } else {
            self.session_expiry_interval().await.as_secs() == 0
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

// type SessionSerde =
//     (u16, Vec<(TopicFilter, u8)>, Vec<(From, Publish)>, Vec<InflightMessage>, TimestampMillis);

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
        listen_cfg: Listener,
        max_mqueue_len: usize,
        max_inflight: usize,
        created_at: TimestampMillis,
    ) -> Self {
        //let max_inflight = listen_cfg.max_inflight;
        let message_retry_interval = listen_cfg.message_retry_interval.as_millis() as TimestampMillis;
        let message_expiry_interval = listen_cfg.message_expiry_interval.as_millis() as TimestampMillis;
        Self(Arc::new(_SessionInner {
            id,
            listen_cfg,
            _subscriptions: Arc::new(TopicFilterMap::default()), //Arc::new(RwLock::new(TopicFilterMap::default())),
            deliver_queue: Arc::new(MessageQueue::new(max_mqueue_len)),
            inflight_win: Arc::new(RwLock::new(Inflight::new(
                max_inflight,
                message_retry_interval,
                message_expiry_interval,
            ))),
            created_at,
        }))
    }

    #[inline]
    pub async fn to_offline_info(&self) -> SessionOfflineInfo {
        let id = self.id.clone();
        let subscriptions = self.drain_subscriptions();
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
    pub listen_cfg: Listener,
    _subscriptions: Arc<TopicFilterMap>,
    //Current subscription for this session
    pub deliver_queue: Arc<MessageQueue>,
    pub inflight_win: Arc<RwLock<Inflight>>,
    pub created_at: TimestampMillis,
}

impl Drop for _SessionInner {
    fn drop(&mut self) {
        self.clear_subscriptions();
    }
}

impl _SessionInner {
    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let count = self.subscriptions_len();

        let subs = self
            ._subscriptions
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let (tf, (qos, shared_sub)) = entry.pair();
                if i < 100 {
                    Some(json!({
                        "topic_filter": tf.to_string(),
                        "qos": qos.value(),
                        "shared_group": shared_sub
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

    #[inline]
    pub fn subscriptions_add(&self, topic_filter: TopicFilter, qos: QoS, shared_group: Option<SharedGroup>) {
        let is_shared = shared_group.is_some();
        let prev = self._subscriptions.insert(topic_filter, (qos, shared_group));

        let inc = |shared: bool| {
            Runtime::instance().stats.subscriptions.inc();
            if shared {
                Runtime::instance().stats.subscriptions_shared.inc();
            }
        };

        if let Some((_, prev_group)) = prev {
            match (prev_group.is_some(), is_shared) {
                (true, false) => {
                    Runtime::instance().stats.subscriptions_shared.dec();
                }
                (false, true) => {
                    inc(true);
                }
                (false, false) => {}
                (true, true) => {}
            }
        } else {
            inc(is_shared);
        }
    }

    #[inline]
    pub fn subscriptions_remove(
        &self,
        topic_filter: &str,
    ) -> Option<(TopicFilter, (QoS, Option<SharedGroup>))> {
        let removed = self._subscriptions.remove(topic_filter);
        if let Some((_, (_, group))) = &removed {
            Runtime::instance().stats.subscriptions.dec();
            if group.is_some() {
                Runtime::instance().stats.subscriptions_shared.dec();
            }
        }
        removed
    }

    #[inline]
    pub fn drain_subscriptions(&self) -> Subscriptions {
        let topic_filters = self._subscriptions.iter().map(|entry| entry.key().clone()).collect::<Vec<_>>();
        let subs = topic_filters.iter().filter_map(|tf| self.subscriptions_remove(tf)).collect();
        subs
    }

    #[inline]
    pub fn clear_subscriptions(&self) {
        for entry in self._subscriptions.iter() {
            Runtime::instance().stats.subscriptions.dec();
            let (_, group) = entry.value();
            if group.is_some() {
                Runtime::instance().stats.subscriptions_shared.dec();
            }
        }
        self._subscriptions.clear();
    }

    #[inline]
    pub fn subscriptions(&self) -> &TopicFilterMap {
        self._subscriptions.as_ref()
    }

    #[inline]
    pub fn subscriptions_len(&self) -> usize {
        self._subscriptions.len()
    }

    #[inline]
    pub fn clone_topic_filters(&self) -> TopicFilters {
        self._subscriptions.iter().map(|entry| TopicFilter::from(entry.key().as_ref())).collect()
    }

    // #[inline]
    // pub fn is_shared_subscriptions(&self, topic_filter: &str) -> Option<bool> {
    //     self._subscriptions.get(topic_filter).map(|entry| matches!(entry.value(), (_, Some(_))))
    // }
}

#[derive(Clone)]
pub struct ClientInfo(Arc<_ClientInfo>);

impl ClientInfo {
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn new(
        connect_info: ConnectInfo,
        session_present: bool,
        connected_at: TimestampMillis,
    ) -> ClientInfo {
        let id = connect_info.id().clone();
        Self(Arc::new(_ClientInfo {
            id,
            connect_info,
            session_present,
            connected: AtomicBool::new(true),
            connected_at,
            disconnected_at: AtomicI64::new(0),
            disconnected_reason: RwLock::new(Vec::new()),
            disconnect: RwLock::new(None),
        }))
    }

    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let mut json = self.connect_info.to_json();
        if let Some(json) = json.as_object_mut() {
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

            if let Some(reason) = self.get_disconnected_reason().await {
                json.insert("disconnected_reason".into(), serde_json::Value::String(reason.to_string()));
            } else {
                json.insert("disconnected_reason".into(), serde_json::Value::Null);
            }
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
    pub fn username(&self) -> &UserName {
        &self.id.username
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
        if let Some(r) = d.reason() {
            self.add_disconnected_reason(r.clone()).await;
        }
        self.disconnect.write().await.replace(d);
    }

    #[inline]
    pub async fn get_disconnected_reason(&self) -> Option<Reason> {
        let reason = self.disconnected_reason.read().await;
        if reason.is_empty() {
            None
        } else {
            Some(Reason::from(reason.join(",")))
        }
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
    pub connected: AtomicBool,
    pub connected_at: TimestampMillis,
    pub disconnected_at: AtomicI64,
    pub disconnected_reason: RwLock<Vec<Reason>>,
    pub disconnect: RwLock<Option<Disconnect>>,
}
