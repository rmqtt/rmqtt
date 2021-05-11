use serde::de::{self, Deserialize};
use serde::ser::{self, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use rmqtt::broker::hook::Type;
use rmqtt::broker::topic::TopicTree;
use rmqtt::settings::deserialize_duration;
use rmqtt::{Result, Topic};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::worker_threads_default")]
    pub worker_threads: usize,
    #[serde(default = "PluginConfig::async_queue_capacity_default")]
    pub async_queue_capacity: usize,
    #[serde(default)]
    pub http_urls: Vec<String>,
    #[serde(
        default = "PluginConfig::http_timeout_default",
        deserialize_with = "deserialize_duration"
    )]
    pub http_timeout: Duration,
    #[serde(rename = "rule")]
    #[serde(default, deserialize_with = "PluginConfig::deserialize_rules")]
    pub rules: HashMap<Type, Vec<Rule>>,
}

impl PluginConfig {
    fn worker_threads_default() -> usize {
        2
    }
    fn async_queue_capacity_default() -> usize {
        10_000
    }
    fn http_timeout_default() -> Duration {
        Duration::from_secs(5)
    }

    fn deserialize_rules<'de, D>(
        deserializer: D,
    ) -> std::result::Result<HashMap<Type, Vec<Rule>>, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let mut rules_cfg: HashMap<String, Vec<Rule>> = HashMap::deserialize(deserializer)?;
        let mut rules = HashMap::new();
        for (typ, r) in rules_cfg.drain() {
            rules.insert(Type::from(typ.as_str()), r);
        }
        Ok(rules)
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

type TopicsType = Option<(Arc<TopicTree<()>>, Vec<String>)>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(
        default,
        deserialize_with = "Rule::deserialize_topics",
        serialize_with = "Rule::serialize_topics"
    )]
    pub topics: TopicsType,
}

impl Rule {
    fn serialize_topics<S>(topics: &TopicsType, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        if let Some((_, topics_cfg)) = topics {
            topics_cfg.as_slice().serialize(s)
        } else {
            let topics_cfg: Vec<String> = Vec::new();
            topics_cfg.as_slice().serialize(s)
        }
    }

    fn deserialize_topics<'de, D>(deserializer: D) -> std::result::Result<TopicsType, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let topics_cfg: Vec<String> = Vec::deserialize(deserializer)?;

        if topics_cfg.is_empty() {
            Ok(None)
        } else {
            let mut topics = TopicTree::default();
            for topic in topics_cfg.iter() {
                topics.insert(
                    &Topic::from_str(topic).map_err(|e| de::Error::custom(format!("{:?}", e)))?,
                    (),
                );
            }
            Ok(Some((Arc::new(topics), topics_cfg)))
        }
    }
}
