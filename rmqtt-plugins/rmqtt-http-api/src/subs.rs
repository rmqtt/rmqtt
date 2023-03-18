use rmqtt::{anyhow, futures, tokio::sync::oneshot, HashMap};
use rmqtt::{
    Id, Message as MqttMessage, MqttError, QoSEx, Result, Runtime, Subscribe, TopicFilter, Unsubscribe,
};

use super::types::{SubscribeParams, UnsubscribeParams};

#[inline]
pub(crate) async fn subscribe(params: SubscribeParams) -> Result<HashMap<TopicFilter, Result<bool>>> {
    let topics = params.topics()?;
    let qos = params.qos()?;
    let clientid = params.clientid;
    let id = Id::from(Runtime::instance().node.id(), clientid);
    let entry = Runtime::instance().extends.shared().await.entry(id);
    let s = entry.session().ok_or_else(|| MqttError::from("session does not exist!"))?;
    let shared_sub_supported =
        Runtime::instance().extends.shared_subscription().await.is_supported(&s.listen_cfg);
    let tx = entry.tx().ok_or_else(|| MqttError::from("session message TX is not exist!"))?;
    let qos = qos.less_value(s.listen_cfg.max_qos_allowed);
    let subs = topics
        .iter()
        .map(|t| Subscribe::from_v3(t, qos, shared_sub_supported))
        .collect::<Result<Vec<_>>>()?;

    let mut reply_rxs = Vec::new();
    for sub in subs {
        let topic_filter = sub.topic_filter.clone();
        let (reply_tx, reply_rx) = oneshot::channel();
        let send_reply = tx.clone().send(MqttMessage::Subscribe(sub, reply_tx));

        let reply_fut = async move {
            let reply = if let Err(send_err) = send_reply {
                Err(MqttError::Msg(send_err.to_string()))
            } else {
                match reply_rx.await {
                    Ok(Ok(res)) => Ok(!res.failure()),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(MqttError::Msg(e.to_string())),
                }
            };
            (topic_filter, reply)
        };
        reply_rxs.push(reply_fut);
    }

    Ok(futures::future::join_all(reply_rxs).await.into_iter().collect())
}

#[inline]
pub(crate) async fn unsubscribe(params: UnsubscribeParams) -> Result<()> {
    let topic_filter = params.topic;
    let clientid = params.clientid;
    let id = Id::from(Runtime::instance().node.id(), clientid);
    let entry = Runtime::instance().extends.shared().await.entry(id);
    let s = entry.session().ok_or_else(|| MqttError::from("session does not exist!"))?;
    let shared_sub_supported =
        Runtime::instance().extends.shared_subscription().await.is_supported(&s.listen_cfg);
    let tx = entry.tx().ok_or_else(|| MqttError::from("session message TX is not exist!"))?;

    let unsub = Unsubscribe::from(&topic_filter, shared_sub_supported)?;
    let (reply_tx, reply_rx) = oneshot::channel();
    tx.send(MqttMessage::Unsubscribe(unsub, reply_tx))?;
    reply_rx.await.map_err(anyhow::Error::new)??;
    Ok(())
}
