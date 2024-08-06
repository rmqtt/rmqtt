use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::Arc;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::Serializer;
use serde::ser::{self, Serialize};

use rmqtt::{
    anyhow::anyhow,
    log,
    serde_json::{self},
    tokio::sync::RwLock,
};
use rmqtt::{broker::topic::TopicTree, MqttError, Result, Session, Topic, TopicFilter, TopicName};

type Rules = Arc<RwLock<TopicTree<Rule>>>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(
        default,
        serialize_with = "PluginConfig::serialize_rules",
        deserialize_with = "PluginConfig::deserialize_rules"
    )]
    rules: (Rules, serde_json::Value),
}

impl PluginConfig {
    #[inline]
    pub fn rules(&self) -> &Rules {
        let (_rules, _) = &self.rules;
        _rules
    }

    #[inline]
    pub async fn rewrite_topic(
        &self,
        action: Action,
        s: Option<&Session>,
        topic: &str,
    ) -> Result<Option<TopicName>> {
        let t = Topic::from_str(topic)?;
        //Take only the best match.
        if let Some(r) = self
            .rules()
            .read()
            .await
            .matches(&t)
            .iter()
            .flat_map(|(_, rs)| rs)
            .filter(|r| match (r.action, action) {
                (Action::All, _) => true,
                (Action::Subscribe, Action::Subscribe) => true,
                (Action::Publish, Action::Publish) => true,
                (_, _) => false,
            })
            .collect::<Vec<_>>()
            .last()
        {
            log::debug!("rule: {:?}", r);
            Ok(self.make_new_topic(r, s, topic))
        } else {
            Ok(None)
        }
    }

    #[inline]
    fn make_new_topic(&self, r: &Rule, s: Option<&Session>, topic: &str) -> Option<TopicName> {
        let mut matcheds = Vec::new();
        if let Some(re) = r.re.get() {
            if let Some(caps) = re.captures(topic) {
                for i in 1..caps.len() {
                    if let Some(matched) = caps.get(i) {
                        matcheds.push((i, matched.as_str()));
                    }
                }
            }
        }
        log::debug!("matcheds: {:?}", matcheds);
        let mut new_topic = String::new();
        for item in &r.dest_topic_items {
            match item {
                DestTopicItem::Normal(normal) => new_topic.push_str(normal),
                DestTopicItem::Clientid => {
                    if let Some(s) = s {
                        new_topic.push_str(s.id.client_id.as_ref())
                    } else {
                        log::info!(
                            "session is not exist, source_topic_filter: {}, dest_topic: {}",
                            r.source_topic_filter,
                            r.dest_topic
                        );
                        return None;
                    }
                }
                DestTopicItem::Username => {
                    if let Some(s) = s {
                        if let Some(name) = s.username() {
                            new_topic.push_str(name)
                        } else {
                            log::warn!(
                                "{} username is not exist, source_topic_filter: {}, dest_topic: {}",
                                s.id,
                                r.source_topic_filter,
                                r.dest_topic
                            );
                            return None;
                        }
                    } else {
                        log::info!(
                            "session is not exist, source_topic_filter: {}, dest_topic: {}",
                            r.source_topic_filter,
                            r.dest_topic
                        );
                        return None;
                    }
                }
                DestTopicItem::Place(idx) => {
                    if let Some((_, matched)) = matcheds.iter().find(|(p, _)| *p == *idx) {
                        new_topic.push_str(matched)
                    } else {
                        log::warn!(
                            "{:?} placeholders(${}) is not exist, source_topic_filter: {}, dest_topic: {}",
                            s.map(|s| &s.id),
                            idx,
                            r.source_topic_filter,
                            r.dest_topic
                        );
                        return None;
                    }
                }
            }
        }
        Some(TopicName::from(new_topic))
    }

    #[inline]
    fn serialize_rules<S>(rules: &(Rules, serde_json::Value), s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        let (_, rules) = rules;
        rules.serialize(s)
    }

    #[inline]
    pub fn deserialize_rules<'de, D>(
        deserializer: D,
    ) -> std::result::Result<(Rules, serde_json::Value), D::Error>
    where
        D: Deserializer<'de>,
    {
        let json_rules = serde_json::Value::deserialize(deserializer)?;
        let mut rules = TopicTree::default();
        if let Some(rules_cfg) = json_rules.as_array() {
            let mut source_topic_filters = Vec::new();
            for rule_cfg in rules_cfg {
                let r = Rule::try_from(rule_cfg).map_err(de::Error::custom)?;
                let tf = Topic::from_str(r.source_topic_filter.as_ref())
                    .map_err(|e| de::Error::custom(format!("{:?}", e)))?;

                if source_topic_filters.contains(&tf) {
                    return Err(de::Error::custom(format!(
                        "There is a conflict due to a duplicate topic filter, {}",
                        r.source_topic_filter
                    )));
                }

                rules.insert(&tf, r);
                source_topic_filters.push(tf);
            }
        }
        let rules = Arc::new(RwLock::new(rules));
        Ok((rules, json_rules))
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum DestTopicItem {
    Normal(String),
    //Sequential placeholder numbers, like $1 = 1, $2 = 2, and $10 = 10, are used to represent specific positions or items.
    Place(usize),
    Clientid,
    Username,
}

impl DestTopicItem {
    fn normal(normal: &str) -> Self {
        DestTopicItem::Normal(normal.into())
    }

    fn place(place: &str) -> Result<Self> {
        if place.len() >= 2 {
            Ok(DestTopicItem::Place(place[1..].parse()?))
        } else {
            Err(MqttError::from(format!("placeholder format error, {}", place)))
        }
    }
}

impl Default for DestTopicItem {
    fn default() -> Self {
        DestTopicItem::Normal(String::new())
    }
}

#[derive(Default, Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct Rule {
    pub action: Action,
    pub source_topic_filter: TopicFilter,
    pub dest_topic: TopicName,
    pub dest_topic_items: Vec<DestTopicItem>,
    pub re: Regex,
}

fn to_dest_topic_items(dest_topic: &str) -> Result<Vec<DestTopicItem>> {
    let re = regex::Regex::new(r"\$\d+|\$\{clientid\}|\$\{username\}").map_err(|e| anyhow!(e))?;
    let mut items = Vec::new();
    let mut idx = 0;
    for mat in re.find_iter(dest_topic) {
        if mat.start() > idx {
            items.push(DestTopicItem::normal(&dest_topic[idx..mat.start()]));
        }
        let item = &dest_topic[mat.start()..mat.end()];
        match &dest_topic[mat.start()..mat.end()] {
            "${clientid}" => {
                items.push(DestTopicItem::Clientid);
            }
            "${username}" => {
                items.push(DestTopicItem::Username);
            }
            _ => {
                items.push(DestTopicItem::place(item)?);
            }
        }

        idx = mat.end();
    }
    if idx < dest_topic.len() {
        items.push(DestTopicItem::normal(&dest_topic[idx..]));
    }
    log::debug!("items: {:?}", items);
    Ok(items)
}

impl std::convert::TryFrom<&serde_json::Value> for Rule {
    type Error = MqttError;
    #[inline]
    fn try_from(rule_cfg: &serde_json::Value) -> Result<Self, Self::Error> {
        let err_msg = format!("Topic-Rewrite Rule config error, rule config is {:?}", rule_cfg);
        if let Some(cfg_objs) = rule_cfg.as_object() {
            let action_cfg = cfg_objs.get("action").ok_or_else(|| MqttError::from(err_msg.as_str()))?;
            let source_topic_filter_cfg = cfg_objs
                .get("source_topic_filter")
                .and_then(|tf| tf.as_str())
                .ok_or_else(|| MqttError::from(err_msg.as_str()))?;
            let dest_topic_cfg = cfg_objs
                .get("dest_topic")
                .and_then(|tf| tf.as_str())
                .ok_or_else(|| MqttError::from(err_msg.as_str()))?;
            let regex_cfg = cfg_objs.get("regex").and_then(|tf| tf.as_str());

            let action = Action::try_from(action_cfg)?;
            let source_topic_filter = TopicFilter::from(source_topic_filter_cfg);
            let dest_topic = TopicFilter::from(dest_topic_cfg);
            let re = Regex::new(regex_cfg)?;
            let dest_topic_items = to_dest_topic_items(&dest_topic)?;

            Ok(Rule { action, source_topic_filter, dest_topic, dest_topic_items, re })
        } else {
            Err(MqttError::from(err_msg))
        }
    }
}

#[derive(Default, Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Action {
    #[default]
    All,
    Publish,
    Subscribe,
}

impl std::convert::TryFrom<&serde_json::Value> for Action {
    type Error = MqttError;
    #[inline]
    fn try_from(action_cfg: &serde_json::Value) -> Result<Self, Self::Error> {
        let err_msg = format!("Topic-Rewrite Rule config error, action config is {:?}", action_cfg);
        match action_cfg.as_str().ok_or_else(|| MqttError::from(err_msg.as_str()))?.to_lowercase().as_str() {
            "all" => Ok(Action::All),
            "publish" => Ok(Action::Publish),
            "subscribe" => Ok(Action::Subscribe),
            _ => Err(MqttError::from(err_msg)),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct Regex(Option<regex::Regex>);

impl Regex {
    fn new(re: Option<&str>) -> Result<Self> {
        let re = if let Some(re) = re { Some(regex::Regex::new(re).map_err(|e| anyhow!(e))?) } else { None };
        Ok(Self(re))
    }

    #[inline]
    fn get(&self) -> Option<&regex::Regex> {
        self.0.as_ref()
    }
}

impl Serialize for Regex {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(re) = self.0.as_ref() {
            re.as_str().serialize(serializer)
        } else {
            "".serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for Regex {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let re_cfg = String::deserialize(deserializer)?;
        let re = if re_cfg.is_empty() {
            None
        } else {
            Some(regex::Regex::new(&re_cfg).map_err(de::Error::custom)?)
        };
        Ok(Regex(re))
    }
}

impl Hash for Regex {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if let Some(re) = self.0.as_ref() {
            re.as_str().hash(state);
        }
    }
}

impl PartialEq for Regex {
    fn eq(&self, other: &Regex) -> bool {
        match (&self.0, &other.0) {
            (Some(re1), Some(re2)) => re1.as_str().eq(re2.as_str()),
            (Some(_), None) => false,
            (None, Some(_)) => false,
            (None, None) => true,
        }
    }
}

impl Eq for Regex {}

impl PartialOrd for Regex {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Regex {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (Some(re1), Some(re2)) => re1.as_str().cmp(re2.as_str()),
            (Some(_), None) => Ordering::Greater,
            (None, Some(_)) => Ordering::Less,
            (None, None) => Ordering::Equal,
        }
    }
}
