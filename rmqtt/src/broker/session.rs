use futures::StreamExt;
use parking_lot::RwLock;
use std::convert::AsRef;
use std::convert::From as _;
use std::fmt;
use std::ops::Deref;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

use ntex_mqtt::types::Protocol;

use crate::broker::inflight::{Inflight, InflightMessage, MomentStatus};
use crate::broker::queue::{Limiter, Policy, Queue, Sender};
use crate::broker::types::*;
use crate::broker::{fitter::Fitter, hook::Hook};
use crate::settings::listener::Listener;
use crate::{MqttError, Result, Runtime, Topic};

type MessageSender = Sender<(From, Publish)>;
type MessageQueue = Queue<(From, Publish)>;

#[derive(Clone)]
pub struct SessionState {
    pub id: Id,
    pub tx: Option<Tx>,
    pub session: Session,
    pub conn: Connection,
    pub sink: Sink,
    pub hook: Rc<dyn Hook>,
    pub deliver_queue_tx: Option<MessageSender>,
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
            self.conn,
            self.deliver_queue_tx
                .as_ref()
                .map(|tx| tx.len())
                .unwrap_or_default()
        )
    }
}

impl SessionState {
    #[inline]
    pub(crate) fn new(
        id: Id,
        session: Session,
        conn: Connection,
        sink: Sink,
        hook: Rc<dyn Hook>,
    ) -> Self {
        Self {
            id,
            tx: None,
            session,
            conn,
            sink,
            hook,
            deliver_queue_tx: None,
        }
    }

    #[inline]
    pub(crate) async fn start(mut self) -> (Self, Tx) {
        let (msg_tx, mut msg_rx) = mpsc::unbounded_channel();
        self.tx.replace(msg_tx.clone());
        let mut state = self.clone();
        ntex::rt::spawn(async move {
            log::debug!(
                "{:?} there are {} offline messages ...",
                state.id,
                state.deliver_queue.len()
            );

            let (burst, replenish_n_per) = state.fitter.mqueue_rate_limit();
            let limiter = Limiter::new(burst, replenish_n_per);
            let (deliver_queue_tx, mut deliver_queue_rx) =
                limiter.channel(state.deliver_queue.clone());

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
            let mut _is_disconnect_received = false;
            log::debug!("{:?} start online event loop", id);

            let keep_alive_interval = if state.conn.keep_alive.as_secs() < 10 {
                Duration::from_secs(10)
            } else {
                state.conn.keep_alive + Duration::from_secs(10)
            };
            log::debug!("{:?} keep_alive_interval is {:?}", id, keep_alive_interval);
            let keep_alive_delay = tokio::time::sleep(keep_alive_interval);
            tokio::pin!(keep_alive_delay);

            let deliver_timeout_delay = tokio::time::sleep(Duration::from_secs(60));
            tokio::pin!(deliver_timeout_delay);

            loop {
                log::debug!("{:?} tokio::select! loop", id);
                let start = chrono::Local::now().timestamp_millis();
                deliver_timeout_delay.as_mut().reset(
                    Instant::now()
                        + state
                            .inflight_win
                            .read()
                            .get_timeout()
                            .unwrap_or_else(|| Duration::from_secs(120)),
                );

                tokio::select! {
                    _ = &mut keep_alive_delay => {  //, if !keep_alive_delay.is_elapsed()
                        log::debug!("{:?} session is timeout, {:?}", id, keep_alive_delay);
                        state.conn.set_disconnected_reason("Timeout(Read/Write)".to_owned());
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
                                        state.hook.message_dropped(Some(state.id.clone()), from, p, Reason::from_static("deliver queue is full")).await;
                                    }
                                },
                                Message::Kick(sender, by_id) => {
                                    log::debug!("{:?} Kicked, send kick result, to {:?}", id, by_id);
                                    if let Err(e) = sender.send(()) {
                                        log::warn!("{:?} Kicked response error, {:?}", id, e);
                                    }
                                    _kicked = true;
                                    state.conn.set_disconnected_reason(format!("Kicked by {:?}", by_id));
                                    break
                                },
                                Message::Disconnect => {
                                    _is_disconnect_received = true;
                                    state.conn.set_disconnected_reason(format!("Disconnect({}) message is received", _is_disconnect_received));
                                },
                                Message::Closed => {
                                    log::debug!("{:?} Disconnect({}) message received", id, _is_disconnect_received);
                                    //Disconnect
                                    break
                                },
                                Message::Keepalive => {
                                    keep_alive_delay.as_mut().reset(Instant::now() + keep_alive_interval);
                                },
                            }
                        }else{
                            log::warn!("{:?} None is received from the Rx", id);
                            state.conn.set_disconnected_reason("None is received from the Rx".to_owned());
                            break;
                        }
                    },

                    _ = &mut deliver_timeout_delay => {  //, if state.inflight_win.read().has_timeout() => {
                        while let Some(iflt_msg) = state.inflight_win.write().pop_front_timeout(){
                            log::debug!("{:?} has timeout message in inflight: {:?}", id, iflt_msg);
                            if let Err(e) = state.reforward(iflt_msg).await{
                                log::error!("{:?} redeliver message error, {:?}", id, e);
                            }
                        }
                    },

                    deliver_packet = deliver_queue_rx.next(), if state.inflight_win.read().has_credit() => {
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
                                state.conn.set_disconnected_reason("Deliver Queue is closed".into());
                                break;
                            }
                        }
                    }
                }
                let cost_time = chrono::Local::now().timestamp_millis() - start;
                if cost_time > 100 {
                    log::debug!("tokio::select! cost time: {}MS", cost_time);
                }
            }

            log::debug!(
                "{:?} exit online worker, kicked: {}, clean_session: {}",
                id,
                _kicked,
                state.conn.clean_session
            );

            //Setting the disconnected state
            state.conn.connected.store(false, Ordering::SeqCst);
            state
                .conn
                .disconnected_at
                .store(chrono::Local::now().timestamp_millis(), Ordering::SeqCst);
            if !_is_disconnect_received {
                if let Err(e) = state.process_last_will().await {
                    log::error!("{:?} process last will error, {:?}", id, e);
                }
            }
            state.sink.close();

            if !_kicked {
                if state.conn.clean_session {
                    state
                        .clean(state.conn.get_disconnected_reason().unwrap_or_default())
                        .await;
                } else {
                    //Start offline event loop
                    Self::offline_start(
                        state.clone(),
                        &mut msg_rx,
                        &deliver_queue_tx,
                        &mut _kicked,
                    )
                    .await;
                    log::debug!("{:?} offline _kicked: {}", id, _kicked);
                    if !_kicked {
                        state.clean(Reason::from("session expired")).await;
                    }
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
        kicked: &mut bool,
    ) {
        log::debug!("{:?} start offline event loop", state.id);
        let id = state.id.clone();

        let session_expiry_interval = state.listen_cfg.session_expiry_interval;
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
                                    state.hook.message_dropped(Some(state.id.clone()), from, p, Reason::from_static("deliver queue is full")).await;
                                }
                            },
                            Message::Kick(sender, by_id) => {
                                log::debug!("{:?} offline Kicked, send kick result, to {:?}", id, by_id);
                                if let Err(e) = sender.send(()) {
                                    log::warn!("{:?} offline Kick response error, {:?}", id, e);
                                }
                                *kicked = true;
                                break
                            },
                            _ => {
                                log::warn!("{:?} offline receive message is {:?}", id, msg);
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
            self.hook
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
        match &self.conn.last_will {
            Some(LastWill::V3(lw)) => {
                let p = Publish::V3(PublishV3::from_last_will(lw)?);
                if let Err(e) = Runtime::instance()
                    .extends
                    .shared()
                    .await
                    .forwards(self.id.clone(), p)
                    .await
                {
                    log::error!("{:?} send last will message fail, {:?}", self.id, e);
                }
            }
            Some(LastWill::V5(_lw)) => {
                log::warn!("{:?} [MQTT V5] Not implemented", self.id);
            }
            None => {}
        }
        Ok(())
    }

    #[inline]
    pub async fn send_retain_messages(
        &self,
        retains: Vec<(Topic, Retain)>,
        qos: QoS,
    ) -> Result<()> {
        for (topic, retain) in retains {
            log::debug!("{:?} topic:{:?}, retain:{:?}", self.id, topic, retain);
            let p = match retain.publish {
                Publish::V3(mut p) => {
                    p.packet.dup = false;
                    p.packet.retain = true;
                    p.packet.qos = p.packet.qos.less_value(qos);
                    p.packet.topic = TopicName::from(topic.to_string());
                    p.packet.packet_id = None;
                    p.create_time = chrono::Local::now().timestamp_millis();
                    Publish::V3(p)
                }
                Publish::V5(p) => {
                    log::warn!("{:?} [MQTT 5] Not implemented", self.id);
                    Publish::V5(p)
                }
            };

            if let Err((from, p, reason)) = Runtime::instance()
                .extends
                .shared()
                .await
                .entry(self.id.clone())
                .forward(retain.from, p)
                .await
            {
                self.hook
                    .message_dropped(Some(self.id.clone()), from, p, reason)
                    .await;
            }
        }
        Ok(())
    }

    #[inline]
    async fn deliver(&self, from: From, mut publish: Publish) -> Result<()> {
        //hook, message_expiry_check
        let expiry = self.hook.message_expiry_check(from.clone(), &publish).await;

        if expiry {
            self.hook
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
            publish.set_packet_id(self.inflight_win.read().next_id()?);
        }

        //send message
        self.sink.publish(publish.clone())?;

        //hook, message_deliver
        self.hook.message_deliver(from.clone(), &publish).await;

        //cache messages to inflight window
        let moment_status = match publish.qos() {
            QoS::AtLeastOnce => Some(MomentStatus::UnAck),
            QoS::ExactlyOnce => Some(MomentStatus::UnReceived),
            _ => None,
        };
        if let Some(moment_status) = moment_status {
            self.inflight_win
                .write()
                .push_back(InflightMessage::new(moment_status, from, publish));
        }

        Ok(())
    }

    #[inline]
    async fn reforward(&self, mut iflt_msg: InflightMessage) -> Result<()> {
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
                let expiry = self
                    .hook
                    .message_expiry_check(iflt_msg.from.clone(), &iflt_msg.publish)
                    .await;

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
                if let Some(release_packet) = iflt_msg.release_packet() {
                    self.sink.send(release_packet)?;
                    self.inflight_win.write().push_back(InflightMessage::new(
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
    pub(crate) async fn subscribe_v3(
        &self,
        subs: &mut v3::control::Subscribe,
    ) -> Result<Vec<SubscribeReturnCodeV3>> {
        let subs_v3 = subs
            .iter_mut()
            .map(|ref sub| match Topic::from_str(sub.topic()) {
                Ok(t) => Ok((t, sub.qos())),
                Err(_e) => Err(MqttError::TopicError(sub.topic().to_string())),
            })
            .collect::<Result<Vec<(Topic, QoS)>>>()?;

        if self.listen_cfg.max_subscriptions > 0
            && self.subscriptions.read().len() + subs_v3.len() > self.listen_cfg.max_subscriptions
        {
            log::warn!(
                "{:?} Subscribe Refused, reason: too many subscriptions, max subscriptions limit: {:?}",
                self.conn.id, self.listen_cfg.max_subscriptions
            );

            let ret_codes = [0..subs_v3.len()]
                .iter()
                .map(|_| SubscribeReturnCodeV3::Failure)
                .collect();
            return Ok(ret_codes);
        }

        let mut subs_v3 = Subscribe::V3(subs_v3);

        //hook, client_subscribe
        let (disconnect, hook_subs_ack) = self.hook.client_subscribe(&subs_v3).await;

        if disconnect {
            log::info!(
                "{:?} Subscribe Refused, reason: Subscription failed",
                self.conn.id
            );
            return Err(MqttError::from("Subscribe Refused"));
        }

        log::debug!("{:?} hook_subs_ack: {:?}", self.id, hook_subs_ack);

        let topic_filters = if let Subscribe::V3(subs_v3) = &subs_v3 {
            subs_v3.iter().map(|(tf, _)| tf.clone()).collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        if let Some(SubscribeAck::V3(subs_ack)) = hook_subs_ack {
            for (i, sub_ack) in subs_ack.iter().enumerate() {
                if let Some(tf) = topic_filters.get(i) {
                    match sub_ack {
                        SubscribeReturnCodeV3::Failure => {
                            subs_v3.remove(tf);
                        }
                        SubscribeReturnCodeV3::Success(qos) => {
                            subs_v3.set_qos_if_less(tf, *qos);
                        }
                    }
                }
            }
        }

        //let mut tmp: StdHashMap<&[u8], usize> = StdHashMap::default();
        let mut tmp: StdHashMap<Topic, usize> = StdHashMap::default();
        if let Subscribe::V3(subs_v3) = &subs_v3 {
            for (idx, (tf, _)) in subs_v3.iter().enumerate() {
                //tmp.insert(tf.as_bytes().as_ref(), idx);
                tmp.insert(tf.clone(), idx);
            }
        }

        log::debug!("{:?} topic_filters: {:?}", self.id, topic_filters);
        log::debug!("{:?} tmp: {:?}", self.id, tmp);
        log::debug!("{:?} subs_v3: {:?}", self.id, subs_v3);

        let mut subs_ack = Runtime::instance()
            .extends
            .shared()
            .await
            .entry(self.id.clone())
            .subscribe(subs_v3)
            .await?;

        log::debug!("{:?} Subscribe ack: {:?}", self.id, subs_ack);

        let mut ret_codes = Vec::new();
        for topic_filter in topic_filters {
            //if let Some(idx) = tmp.remove(topic_filter.as_bytes().as_ref()) {
            if let Some(idx) = tmp.remove(&topic_filter) {
                if let SubscribeAck::V3(codes) = &mut subs_ack {
                    match codes.get(idx) {
                        Some(SubscribeReturnCodeV3::Failure) => {
                            ret_codes.push(SubscribeReturnCodeV3::Failure)
                        }
                        Some(SubscribeReturnCodeV3::Success(qos)) => {
                            ret_codes.push(SubscribeReturnCodeV3::Success(*qos));

                            if self.listen_cfg.retain_available {
                                //send retain messages
                                let retain_messages = Runtime::instance()
                                    .extends
                                    .retain()
                                    .await
                                    .get(&topic_filter)
                                    .await?;
                                self.send_retain_messages(retain_messages, *qos).await?;
                            }
                            //hook, session_subscribed
                            self.hook
                                .session_subscribed(Subscribed::V3((topic_filter, *qos)))
                                .await;
                        }
                        None => ret_codes.push(SubscribeReturnCodeV3::Failure),
                    }
                } else {
                    log::warn!("[MQTT 5] Not implemented");
                    ret_codes.push(SubscribeReturnCodeV3::Failure)
                }
            } else {
                ret_codes.push(SubscribeReturnCodeV3::Failure)
            }
        }

        Ok(ret_codes)
    }

    #[inline]
    pub(crate) async fn unsubscribe_v3(&self, unsubs: &mut v3::control::Unsubscribe) -> Result<()> {
        let unsubs_v3 = unsubs
            .iter()
            .map(|ref tf| match Topic::from_str(*tf) {
                Ok(t) => Ok(t),
                Err(_e) => Err(MqttError::TopicError(tf.to_string())),
            })
            .collect::<Result<Vec<Topic>>>()?;

        let unsubs_v3 = Unsubscribe::V3(unsubs_v3);

        //hook, client_unsubscribe
        let disconnect = self.hook.client_unsubscribe(&unsubs_v3).await;
        if disconnect {
            log::info!(
                "{:?} Unsubscribe Refused, reason: Unsubscribe failed",
                self.conn.id
            );
            return Err(MqttError::from("Unsubscribe failed"));
        }

        let _ = Runtime::instance()
            .extends
            .shared()
            .await
            .entry(self.id.clone())
            .unsubscribe(&unsubs_v3)
            .await?;

        //hook, session_unsubscribed
        if let Unsubscribe::V3(mut unsubs) = unsubs_v3 {
            for topic_filter in unsubs.drain(..) {
                self.hook
                    .session_unsubscribed(Unsubscribed::V3(topic_filter))
                    .await;
            }
        }
        Ok(())
    }

    #[inline]
    pub async fn publish_v3(&self, publish: v3::Publish) -> Result<()> {
        let p = Publish::V3(PublishV3::from(&publish)?);

        let (disconnect, deny) = self.hook.message_publish(&p).await;

        if disconnect || deny {
            //Message dropped
            self.hook
                .message_dropped(
                    None,
                    self.id.clone(),
                    p.clone(),
                    Reason::from(format!(
                        "hook::message_publish, disconnect:{}, deny:{}",
                        disconnect, deny
                    )),
                )
                .await;
        }
        if disconnect {
            return Err(MqttError::from(
                "Publish Refused, reason: hook::message_publish(disconnect)",
            ));
        }

        if deny {
            log::info!(
                "{:?} Publish Refused, reason: hook::message_publish(deny)",
                self.id
            );
            return Ok(());
        }

        let topic = Topic::from_str(publish.topic().get_ref())?;

        if self.listen_cfg.retain_available && publish.retain() {
            Runtime::instance()
                .extends
                .retain()
                .await
                .set(
                    &topic,
                    Retain {
                        from: self.id.clone(),
                        publish: p.clone(),
                    },
                )
                .await?;
        }

        if let Err(errs) = Runtime::instance()
            .extends
            .shared()
            .await
            .forwards(self.id.clone(), p)
            .await
        {
            for (to, from, p, reason) in errs {
                //Message dropped
                self.hook.message_dropped(Some(to), from, p, reason).await;
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
                log::debug!(
                    "{:?} clean.dropped, from: {:?}, publish: {:?}",
                    self.id,
                    from,
                    publish
                );

                //hook, message dropped
                self.hook
                    .message_dropped(Some(self.id.clone()), from, publish, reason.clone())
                    .await;
            }
        }

        //Session expired, discarding messages in the flight window
        while let Some(iflt_msg) = self.inflight_win.write().pop_front() {
            log::debug!(
                "{:?} clean.dropped, from: {:?}, publish: {:?}",
                self.id,
                iflt_msg.from,
                iflt_msg.publish
            );

            //hook, message dropped
            self.hook
                .message_dropped(
                    Some(self.id.clone()),
                    iflt_msg.from,
                    iflt_msg.publish,
                    reason.clone(),
                )
                .await;
        }

        //hook, session terminated
        self.hook.session_terminated(reason).await;

        //clear session, and unsubscribe
        if let Err(e) = Runtime::instance()
            .extends
            .shared()
            .await
            .entry(self.id.clone())
            .remove()
            .await
        {
            log::error!("{:?} session remove from broker fail, {:?}", self.id, e);
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

#[derive(Clone)]
pub struct Session(Arc<_SessionInner>);

impl Session {
    #[inline]
    pub(crate) fn new(
        listen_cfg: Listener,
        fitter: Box<dyn Fitter>,
        created_at: TimestampMillis,
    ) -> Self {
        let max_mqueue_len = fitter.max_mqueue_len();
        let max_inflight = listen_cfg.max_inflight;
        let message_retry_interval =
            listen_cfg.message_retry_interval.as_millis() as TimestampMillis;
        let message_expiry_interval =
            listen_cfg.message_expiry_interval.as_millis() as TimestampMillis;
        Self(Arc::new(_SessionInner {
            listen_cfg,
            fitter,
            subscriptions: Arc::new(RwLock::new(TopicFilters::default())),
            deliver_queue: Arc::new(MessageQueue::new(max_mqueue_len)),
            inflight_win: Arc::new(RwLock::new(Inflight::new(
                max_inflight,
                message_retry_interval,
                message_expiry_interval,
            ))),
            created_at,
        }))
    }
}

impl Deref for Session {
    type Target = _SessionInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0.as_ref()
    }
}

impl std::fmt::Debug for Session {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Session {:?}", serde_json::to_string(&self.to_json()))
    }
}

pub struct _SessionInner {
    pub listen_cfg: Listener,
    pub fitter: Box<dyn Fitter>,
    pub subscriptions: Arc<RwLock<TopicFilters>>, //Current subscription for this session
    pub deliver_queue: Arc<MessageQueue>,
    pub inflight_win: Arc<RwLock<Inflight>>,
    pub created_at: TimestampMillis,
}

impl _SessionInner {
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        let data = json!({
            "subscriptions": self.subscriptions.read().len(),
            "queues": self.deliver_queue.len(),
            "inflights": self.inflight_win.read().len(),
            "created_at": self.created_at,
        });
        data
    }

    #[inline]
    pub fn subscriptions_add(&self, topic_filter: TopicFilter, qos: QoS) {
        self.subscriptions.write().insert(topic_filter, qos);
    }

    #[inline]
    pub fn subscriptions_remove(&self, topic_filter: &TopicFilter) -> Option<QoS> {
        self.subscriptions.write().remove(topic_filter)
    }

    #[inline]
    pub fn drain_subscriptions(&self) -> Vec<TopicFilter> {
        self.subscriptions
            .write()
            .drain()
            .map(|(topic_filter, _)| topic_filter)
            .collect()
    }
}

#[derive(Clone)]
pub struct Connection(Arc<_ConnectionInner>);

impl Connection {
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn new(
        id: Id,
        protocol: Protocol,
        username: Option<UserName>,
        keep_alive: Duration,
        clean_session: bool,
        last_will: Option<LastWill>,
        session_present: bool,
        connected_at: TimestampMillis,
    ) -> Connection {
        Self(Arc::new(_ConnectionInner {
            id,
            protocol,
            username,
            keep_alive,
            clean_session,
            last_will,
            session_present,
            connected: AtomicBool::new(true),
            connected_at,
            disconnected_at: AtomicI64::new(0),
            disconnected_reason: RwLock::new(None),
        }))
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        let data = json!({
            "node_id": self.id.node_id,
            "local_addr": self.id.local_addr,
            "remote_addr": self.id.remote_addr,
            "client_id": self.id.client_id,
            "username": self.username,
            "protocol": self.protocol.level(),
            "keep_alive": self.keep_alive.as_secs(),
            "clean_session": self.clean_session,
            "last_will": self.last_will.as_ref().map(|lw|lw.to_json()),
            "session_present": self.session_present,
            "connected": self.connected.load(Ordering::SeqCst),
            "connected_at": self.connected_at,
            "disconnected_at": self.disconnected_at.load(Ordering::SeqCst),
            "disconnected_reason": self.disconnected_reason.read().as_ref(),
        });
        Ok(data)
    }

    #[inline]
    pub fn username(&self) -> UserName {
        self.username
            .as_ref()
            .cloned()
            .unwrap_or_else(|| "undefined".into())
    }

    #[inline]
    pub fn set_disconnected(&self, reason: String) {
        self.connected.store(false, Ordering::SeqCst);
        self.set_disconnected_reason(reason);
    }

    #[inline]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn set_disconnected_reason(&self, r: String) {
        let mut disconnected_reason = self.disconnected_reason.write();
        if disconnected_reason.is_none() {
            disconnected_reason.replace(Reason::from(r));
        }
    }

    #[inline]
    pub fn get_disconnected_reason(&self) -> Option<Reason> {
        self.disconnected_reason.read().clone()
    }

    // #[inline]
    // pub fn take_disconnected_reason(&self) -> Option<Reason> {
    //     self.disconnected_reason.write().take()
    // }

    #[inline]
    pub fn has_disconnected_reason(&self) -> bool {
        self.disconnected_reason.read().is_some()
    }
}

impl std::fmt::Debug for Connection {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Connection {:?}",
            serde_json::to_string(&self.to_json().unwrap_or_default())
        )
    }
}

impl Deref for Connection {
    type Target = _ConnectionInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

pub struct _ConnectionInner {
    pub id: Id,
    pub protocol: Protocol,
    pub username: Option<UserName>,
    pub keep_alive: Duration,
    pub clean_session: bool,
    pub last_will: Option<LastWill>,
    pub session_present: bool,
    pub connected: AtomicBool,
    pub connected_at: TimestampMillis,
    pub disconnected_at: AtomicI64,
    pub disconnected_reason: RwLock<Option<Reason>>,
}
