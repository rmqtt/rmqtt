use anyhow::anyhow;
use serde::{
    de::{self, Deserializer},
    ser, Deserialize, Serialize,
};
use serde_json::{self, json};

use rmqtt::{
    codec::v5::{RetainHandling, SubscriptionOptions},
    types::{QoS, Subscribe, TopicFilter},
    Error, Result,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(
        default,
        serialize_with = "PluginConfig::serialize_subscribes",
        deserialize_with = "PluginConfig::deserialize_subscribes"
    )]
    pub subscribes: Vec<SubscribeItem>,
}

impl PluginConfig {
    #[inline]
    fn serialize_subscribes<S>(subscribes: &[SubscribeItem], s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        //topic_filter = "x/+/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0
        subscribes
            .iter()
            .map(|item| {
                let sub = &item.sub;
                let topic_filter = if let Some(shared_group) = sub.opts.shared_group() {
                    format!("$share/{}/{}", shared_group, sub.topic_filter)
                } else if let Some(limit) = sub.opts.limit_subs() {
                    if limit == 1 {
                        format!("$exclusive/{}", sub.topic_filter)
                    } else {
                        format!("$limit/{}/{}", limit, sub.topic_filter)
                    }
                } else {
                    sub.topic_filter.to_string()
                };
                let qos = sub.opts.qos().value();
                let no_local = sub.opts.no_local().unwrap_or_default();
                let retain_as_published = sub.opts.retain_as_published().unwrap_or_default();
                let retain_handling = sub
                    .opts
                    .retain_handling()
                    .map(|h| match h {
                        RetainHandling::AtSubscribe => 0,
                        RetainHandling::AtSubscribeNew => 1,
                        RetainHandling::NoAtSubscribe => 3,
                    })
                    .unwrap_or(0);
                json!({
                    "topic_filter": topic_filter,
                    "qos": qos,
                    "no_local": no_local,
                    "retain_as_published": retain_as_published,
                    "retain_handling": retain_handling,
                })
            })
            .collect::<Vec<_>>()
            .serialize(s)
    }

    #[inline]
    pub fn deserialize_subscribes<'de, D>(
        deserializer: D,
    ) -> std::result::Result<Vec<SubscribeItem>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut subscribes = Vec::new();
        let json_subscribes: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        if let Some(subs) = json_subscribes.as_array() {
            for sub in subs {
                if let Some(objs) = sub.as_object() {
                    let topic_filter = objs
                        .get("topic_filter")
                        .and_then(|tf| tf.as_str())
                        .ok_or_else(|| de::Error::custom("topic_filter is required"))?;
                    let qos = QoS::try_from(
                        objs.get("qos")
                            .and_then(|qos| qos.as_u64().map(|q| q as u8))
                            .ok_or_else(|| de::Error::custom("qos is required"))?,
                    )
                    .map_err(de::Error::custom)?;
                    let no_local =
                        objs.get("no_local").and_then(|no_local| no_local.as_bool()).unwrap_or(false);
                    let retain_as_published = objs
                        .get("retain_as_published")
                        .and_then(|retain_as_published| retain_as_published.as_bool())
                        .unwrap_or(false);
                    let retain_handling = objs
                        .get("retain_handling")
                        .and_then(|retain_handling| retain_handling.as_u64())
                        .map(|retain_handling| match retain_handling {
                            0 => Ok::<_, Error>(RetainHandling::AtSubscribe),
                            1 => Ok::<_, Error>(RetainHandling::AtSubscribeNew),
                            2 => Ok::<_, Error>(RetainHandling::NoAtSubscribe),
                            _ => Err(anyhow!("illegal retain_handling value, only 0, 1, 2 are allowed",)),
                        })
                        .unwrap_or_else(|| Ok(RetainHandling::AtSubscribe))
                        .map_err(de::Error::custom)?;

                    let opts = SubscriptionOptions { qos, no_local, retain_as_published, retain_handling };
                    let sub = Subscribe::from_v5(&TopicFilter::from(topic_filter), &opts, true, true, None)
                        .map_err(de::Error::custom)?;
                    let has_clientid_placeholder = topic_filter.contains("${clientid}");
                    let has_username_placeholder = topic_filter.contains("${username}");
                    subscribes.push(SubscribeItem {
                        sub,
                        has_clientid_placeholder,
                        has_username_placeholder,
                    });
                }
            }
        }
        Ok(subscribes)
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Clone, Debug)]
pub struct SubscribeItem {
    pub sub: Subscribe,
    pub has_clientid_placeholder: bool,
    pub has_username_placeholder: bool,
}
